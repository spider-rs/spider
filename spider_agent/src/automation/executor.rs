//! High-performance automation executor.
//!
//! Provides optimized execution patterns for automation workflows:
//! - Parallel step execution for independent operations
//! - Batched processing for multiple items
//! - Smart caching for LLM responses
//! - Prefetching for predictable navigation

use super::{ChainCondition, ChainContext, ChainResult, ChainStep, ChainStepResult};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, Semaphore};

/// High-performance executor for automation chains.
///
/// Features:
/// - Parallel execution of independent steps
/// - Response caching with TTL
/// - Configurable concurrency limits
/// - Progress tracking and cancellation
#[derive(Debug)]
pub struct ChainExecutor {
    /// Maximum concurrent operations.
    max_concurrency: usize,
    /// Semaphore for concurrency control.
    semaphore: Arc<Semaphore>,
    /// Response cache.
    cache: Arc<RwLock<ResponseCache>>,
    /// Whether caching is enabled.
    cache_enabled: bool,
    /// Default timeout per step.
    step_timeout: Duration,
}

impl Default for ChainExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl ChainExecutor {
    /// Create a new executor with default settings.
    pub fn new() -> Self {
        let max_concurrency = 5;
        Self {
            max_concurrency,
            semaphore: Arc::new(Semaphore::new(max_concurrency)),
            cache: Arc::new(RwLock::new(ResponseCache::new())),
            cache_enabled: true,
            step_timeout: Duration::from_secs(30),
        }
    }

    /// Create with custom concurrency limit.
    pub fn with_concurrency(max_concurrency: usize) -> Self {
        Self {
            max_concurrency,
            semaphore: Arc::new(Semaphore::new(max_concurrency)),
            cache: Arc::new(RwLock::new(ResponseCache::new())),
            cache_enabled: true,
            step_timeout: Duration::from_secs(30),
        }
    }

    /// Set step timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.step_timeout = timeout;
        self
    }

    /// Enable or disable caching.
    pub fn with_cache(mut self, enabled: bool) -> Self {
        self.cache_enabled = enabled;
        self
    }

    /// Get the maximum concurrency limit.
    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    /// Execute a chain of steps.
    ///
    /// Steps are analyzed for dependencies and independent steps
    /// are executed in parallel where possible.
    pub async fn execute<F, Fut>(
        &self,
        steps: Vec<ChainStep>,
        mut context: ChainContext,
        step_fn: F,
    ) -> ChainResult
    where
        F: Fn(ChainStep, ChainContext) -> Fut + Clone + Send + Sync + 'static,
        Fut: std::future::Future<Output = ChainStepResult> + Send,
    {
        let start = Instant::now();
        let mut result = ChainResult::new();
        let _total_steps = steps.len();

        // Group steps by dependency
        let groups = self.analyze_dependencies(&steps, &context);

        for group in groups {
            if group.len() == 1 {
                // Single step - execute directly
                let step = group.into_iter().next().unwrap();
                let step_result = self.execute_step(step, context.clone(), &step_fn).await;

                context.set_previous_result(step_result.success);
                context.advance();
                result.add_step(step_result);
            } else {
                // Multiple independent steps - execute in parallel
                let step_results = self
                    .execute_parallel(group, context.clone(), &step_fn)
                    .await;

                let all_succeeded = step_results.iter().all(|r| r.success || !r.executed);
                context.set_previous_result(all_succeeded);

                for step_result in step_results {
                    context.advance();
                    result.add_step(step_result);
                }
            }

            // Check for abort conditions
            if result.steps_failed > 0 && !self.should_continue(&result) {
                break;
            }
        }

        result.duration_ms = start.elapsed().as_millis() as u64;
        result.success = result.steps_failed == 0;
        result
    }

    /// Execute a single step with timeout and caching.
    async fn execute_step<F, Fut>(
        &self,
        step: ChainStep,
        context: ChainContext,
        step_fn: &F,
    ) -> ChainStepResult
    where
        F: Fn(ChainStep, ChainContext) -> Fut + Clone + Send + Sync,
        Fut: std::future::Future<Output = ChainStepResult> + Send,
    {
        let index = context.step_index;

        // Check condition
        if !step.should_execute(&context) {
            return ChainStepResult::skipped(index, &step.instruction);
        }

        // Check cache if enabled
        if self.cache_enabled {
            let cache_key = self.cache_key(&step, &context);
            if let Some(cached) = self.get_cached(&cache_key).await {
                return cached;
            }
        }

        // Acquire semaphore permit
        let _permit = self.semaphore.acquire().await.ok();

        // Execute with timeout
        let timeout = step
            .timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(self.step_timeout);
        let step_clone = step.clone();

        let result = tokio::time::timeout(timeout, step_fn(step_clone, context)).await;

        let step_result = match result {
            Ok(r) => r,
            Err(_) => ChainStepResult::executed(index, &step.instruction, false)
                .with_error("Step timed out"),
        };

        // Cache successful results
        if self.cache_enabled && step_result.success {
            let cache_key = self.cache_key(&step, &ChainContext::default());
            self.set_cached(&cache_key, step_result.clone()).await;
        }

        step_result
    }

    /// Execute multiple steps in parallel.
    async fn execute_parallel<F, Fut>(
        &self,
        steps: Vec<ChainStep>,
        context: ChainContext,
        step_fn: &F,
    ) -> Vec<ChainStepResult>
    where
        F: Fn(ChainStep, ChainContext) -> Fut + Clone + Send + Sync + 'static,
        Fut: std::future::Future<Output = ChainStepResult> + Send,
    {
        let mut handles = Vec::with_capacity(steps.len());

        for (i, step) in steps.into_iter().enumerate() {
            let ctx = ChainContext {
                step_index: context.step_index + i,
                ..context.clone()
            };
            let semaphore = self.semaphore.clone();
            let timeout = step
                .timeout_ms
                .map(Duration::from_millis)
                .unwrap_or(self.step_timeout);
            let step_fn = step_fn.clone();

            handles.push(tokio::spawn(async move {
                let _permit = semaphore.acquire().await.ok();

                if !step.should_execute(&ctx) {
                    return ChainStepResult::skipped(ctx.step_index, &step.instruction);
                }

                let step_clone = step.clone();
                let result = tokio::time::timeout(timeout, step_fn(step_clone, ctx.clone())).await;

                match result {
                    Ok(r) => r,
                    Err(_) => ChainStepResult::executed(ctx.step_index, &step.instruction, false)
                        .with_error("Step timed out"),
                }
            }));
        }

        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            if let Ok(result) = handle.await {
                results.push(result);
            }
        }

        // Sort by index to maintain order
        results.sort_by_key(|r| r.index);
        results
    }

    /// Analyze steps for dependencies and group independent ones.
    fn analyze_dependencies(
        &self,
        steps: &[ChainStep],
        _context: &ChainContext,
    ) -> Vec<Vec<ChainStep>> {
        let mut groups: Vec<Vec<ChainStep>> = Vec::new();
        let mut current_group: Vec<ChainStep> = Vec::new();

        for step in steps {
            let depends_on_previous = matches!(
                &step.condition,
                Some(ChainCondition::PreviousSucceeded) | Some(ChainCondition::PreviousFailed)
            );

            if depends_on_previous && !current_group.is_empty() {
                // This step depends on previous - start new group
                groups.push(std::mem::take(&mut current_group));
            }

            current_group.push(step.clone());

            // If step modifies state significantly, flush group
            if step.extract.is_some() || !step.continue_on_failure {
                groups.push(std::mem::take(&mut current_group));
            }
        }

        if !current_group.is_empty() {
            groups.push(current_group);
        }

        groups
    }

    /// Generate cache key for a step.
    fn cache_key(&self, step: &ChainStep, context: &ChainContext) -> String {
        format!("{}:{}", step.instruction, context.current_url)
    }

    /// Get cached result.
    async fn get_cached(&self, key: &str) -> Option<ChainStepResult> {
        let cache = self.cache.read().await;
        cache.get(key)
    }

    /// Set cached result.
    async fn set_cached(&self, key: &str, result: ChainStepResult) {
        let mut cache = self.cache.write().await;
        cache.set(key, result);
    }

    /// Check if execution should continue after failure.
    fn should_continue(&self, result: &ChainResult) -> bool {
        // Continue if recent steps allowed failure
        result
            .step_results
            .last()
            .map(|r| !r.executed || r.success)
            .unwrap_or(true)
    }

    /// Clear the response cache.
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }
}

/// LRU cache for responses with TTL.
#[derive(Debug)]
pub struct ResponseCache {
    entries: HashMap<String, CacheEntry>,
    max_entries: usize,
    ttl: Duration,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    result: ChainStepResult,
    created_at: Instant,
}

impl Default for ResponseCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ResponseCache {
    /// Create a new cache.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            max_entries: 1000,
            ttl: Duration::from_secs(300), // 5 minutes
        }
    }

    /// Create with custom settings.
    pub fn with_settings(max_entries: usize, ttl: Duration) -> Self {
        Self {
            entries: HashMap::with_capacity(max_entries),
            max_entries,
            ttl,
        }
    }

    /// Get a cached entry if valid.
    pub fn get(&self, key: &str) -> Option<ChainStepResult> {
        self.entries.get(key).and_then(|entry| {
            if entry.created_at.elapsed() < self.ttl {
                Some(entry.result.clone())
            } else {
                None
            }
        })
    }

    /// Set a cache entry.
    pub fn set(&mut self, key: &str, result: ChainStepResult) {
        // Evict old entries if at capacity
        if self.entries.len() >= self.max_entries {
            self.evict_expired();
            if self.entries.len() >= self.max_entries {
                self.evict_oldest();
            }
        }

        self.entries.insert(
            key.to_string(),
            CacheEntry {
                result,
                created_at: Instant::now(),
            },
        );
    }

    /// Clear the cache.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Evict expired entries.
    fn evict_expired(&mut self) {
        self.entries
            .retain(|_, entry| entry.created_at.elapsed() < self.ttl);
    }

    /// Evict oldest entry.
    fn evict_oldest(&mut self) {
        if let Some(oldest_key) = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.created_at)
            .map(|(k, _)| k.clone())
        {
            self.entries.remove(&oldest_key);
        }
    }
}

/// Batch executor for processing multiple items efficiently.
#[derive(Debug)]
pub struct BatchExecutor {
    /// Maximum batch size.
    pub max_batch_size: usize,
    /// Maximum concurrent batches.
    pub max_concurrent: usize,
    /// Semaphore for concurrency control.
    semaphore: Arc<Semaphore>,
}

impl Default for BatchExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl BatchExecutor {
    /// Create a new batch executor.
    pub fn new() -> Self {
        Self {
            max_batch_size: 10,
            max_concurrent: 3,
            semaphore: Arc::new(Semaphore::new(3)),
        }
    }

    /// Create with custom settings.
    pub fn with_settings(max_batch_size: usize, max_concurrent: usize) -> Self {
        Self {
            max_batch_size,
            max_concurrent,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
        }
    }

    /// Process items in batches.
    pub async fn process<T, R, F, Fut>(&self, items: Vec<T>, processor: F) -> Vec<R>
    where
        T: Clone + Send + 'static,
        R: Send + 'static,
        F: Fn(T) -> Fut + Clone + Send + Sync + 'static,
        Fut: std::future::Future<Output = R> + Send,
    {
        let mut results = Vec::with_capacity(items.len());
        let chunks: Vec<Vec<T>> = items
            .into_iter()
            .collect::<Vec<_>>()
            .chunks(self.max_batch_size)
            .map(|c| c.to_vec())
            .collect();

        for chunk in chunks {
            let mut handles = Vec::with_capacity(chunk.len());

            for item in chunk {
                let semaphore = self.semaphore.clone();
                let processor = processor.clone();

                handles.push(tokio::spawn(async move {
                    let _permit = semaphore.acquire().await.ok();
                    processor(item).await
                }));
            }

            for handle in handles {
                if let Ok(result) = handle.await {
                    results.push(result);
                }
            }
        }

        results
    }

    /// Process items in parallel with index tracking.
    pub async fn process_indexed<T, R, F, Fut>(
        &self,
        items: Vec<T>,
        processor: F,
    ) -> Vec<(usize, R)>
    where
        T: Clone + Send + 'static,
        R: Send + 'static,
        F: Fn(usize, T) -> Fut + Clone + Send + Sync + 'static,
        Fut: std::future::Future<Output = R> + Send,
    {
        let mut results = Vec::with_capacity(items.len());
        let indexed: Vec<(usize, T)> = items.into_iter().enumerate().collect();

        let chunks: Vec<Vec<(usize, T)>> = indexed
            .into_iter()
            .collect::<Vec<_>>()
            .chunks(self.max_batch_size)
            .map(|c| c.to_vec())
            .collect();

        for chunk in chunks {
            let mut handles = Vec::with_capacity(chunk.len());

            for (idx, item) in chunk {
                let semaphore = self.semaphore.clone();
                let processor = processor.clone();

                handles.push(tokio::spawn(async move {
                    let _permit = semaphore.acquire().await.ok();
                    (idx, processor(idx, item).await)
                }));
            }

            for handle in handles {
                if let Ok(result) = handle.await {
                    results.push(result);
                }
            }
        }

        // Sort by index
        results.sort_by_key(|(idx, _)| *idx);
        results
    }
}

/// Prefetch manager for predictive page loading.
#[derive(Debug)]
pub struct PrefetchManager {
    /// URLs currently being prefetched.
    in_progress: Arc<RwLock<HashMap<String, tokio::task::JoinHandle<Option<String>>>>>,
    /// Prefetched content cache.
    cache: Arc<RwLock<HashMap<String, PrefetchedContent>>>,
    /// Maximum prefetch cache size.
    max_cache_size: usize,
    /// Maximum concurrent prefetches.
    max_concurrent: usize,
    /// Semaphore for concurrency.
    semaphore: Arc<Semaphore>,
}

#[derive(Debug, Clone)]
struct PrefetchedContent {
    html: String,
    fetched_at: Instant,
}

impl Default for PrefetchManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PrefetchManager {
    /// Create a new prefetch manager.
    pub fn new() -> Self {
        Self {
            in_progress: Arc::new(RwLock::new(HashMap::new())),
            cache: Arc::new(RwLock::new(HashMap::new())),
            max_cache_size: 50,
            max_concurrent: 3,
            semaphore: Arc::new(Semaphore::new(3)),
        }
    }

    /// Start prefetching a URL.
    pub async fn prefetch<F, Fut>(&self, url: String, fetcher: F)
    where
        F: FnOnce(String) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Option<String>> + Send,
    {
        // Check if already cached or in progress
        {
            let cache = self.cache.read().await;
            if cache.contains_key(&url) {
                return;
            }
        }
        {
            let in_progress = self.in_progress.read().await;
            if in_progress.contains_key(&url) {
                return;
            }
        }

        let semaphore = self.semaphore.clone();
        let cache = self.cache.clone();
        let url_clone = url.clone();
        let max_cache_size = self.max_cache_size;

        let handle = tokio::spawn(async move {
            let _permit = semaphore.acquire().await.ok();
            let result = fetcher(url_clone.clone()).await;

            if let Some(ref html) = result {
                let mut cache = cache.write().await;

                // Evict if at capacity
                if cache.len() >= max_cache_size {
                    if let Some(oldest_key) = cache
                        .iter()
                        .min_by_key(|(_, v)| v.fetched_at)
                        .map(|(k, _)| k.clone())
                    {
                        cache.remove(&oldest_key);
                    }
                }

                cache.insert(
                    url_clone,
                    PrefetchedContent {
                        html: html.clone(),
                        fetched_at: Instant::now(),
                    },
                );
            }

            result
        });

        let mut in_progress = self.in_progress.write().await;
        in_progress.insert(url, handle);
    }

    /// Get prefetched content if available.
    pub async fn get(&self, url: &str) -> Option<String> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(content) = cache.get(url) {
                // Consider content fresh for 60 seconds
                if content.fetched_at.elapsed() < Duration::from_secs(60) {
                    return Some(content.html.clone());
                }
            }
        }

        // Check if prefetch is in progress
        let handle = {
            let mut in_progress = self.in_progress.write().await;
            in_progress.remove(url)
        };

        if let Some(handle) = handle {
            // Wait for prefetch to complete
            if let Ok(result) = handle.await {
                return result;
            }
        }

        None
    }

    /// Prefetch multiple URLs.
    pub async fn prefetch_many<F, Fut>(&self, urls: Vec<String>, fetcher: F)
    where
        F: Fn(String) -> Fut + Clone + Send + 'static,
        Fut: std::future::Future<Output = Option<String>> + Send,
    {
        for url in urls {
            let fetcher = fetcher.clone();
            self.prefetch(url, fetcher).await;
        }
    }

    /// Clear the prefetch cache.
    pub async fn clear(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();

        let mut in_progress = self.in_progress.write().await;
        for (_, handle) in in_progress.drain() {
            handle.abort();
        }
    }

    /// Get the maximum concurrent prefetches.
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_batch_executor() {
        let executor = BatchExecutor::with_settings(3, 2);

        let items: Vec<i32> = (0..10).collect();
        let results = executor.process(items, |x| async move { x * 2 }).await;

        assert_eq!(results.len(), 10);
    }

    #[tokio::test]
    async fn test_batch_executor_indexed() {
        let executor = BatchExecutor::new();

        let items = vec!["a", "b", "c"];
        let results = executor
            .process_indexed(items, |idx, s| async move { format!("{}:{}", idx, s) })
            .await;

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (0, "0:a".to_string()));
        assert_eq!(results[1], (1, "1:b".to_string()));
        assert_eq!(results[2], (2, "2:c".to_string()));
    }

    #[tokio::test]
    async fn test_response_cache() {
        let mut cache = ResponseCache::new();

        let result = ChainStepResult::executed(0, "test", true);
        cache.set("key1", result.clone());

        assert!(cache.get("key1").is_some());
        assert!(cache.get("key2").is_none());
    }

    #[test]
    fn test_chain_executor_dependency_analysis() {
        let executor = ChainExecutor::new();
        let context = ChainContext::new("https://example.com");

        let steps = vec![
            ChainStep::new("Step 1"),
            ChainStep::new("Step 2"),
            ChainStep::new("Step 3").when(ChainCondition::PreviousSucceeded),
            ChainStep::new("Step 4"),
        ];

        let groups = executor.analyze_dependencies(&steps, &context);

        // Should create groups based on dependencies
        assert!(!groups.is_empty());
    }
}
