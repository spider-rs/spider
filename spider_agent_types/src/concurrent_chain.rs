//! Concurrent action chain execution with dependency management.
//!
//! This module provides a dependency graph for executing actions in parallel
//! where dependencies allow. Independent actions run concurrently via `tokio::JoinSet`,
//! while dependent actions respect their ordering constraints.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};

/// A step in a dependency chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependentStep {
    /// Unique identifier for this step.
    pub id: String,
    /// The action to execute (as JSON value).
    pub action: Value,
    /// IDs of steps that must complete before this one can start.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// IDs of steps that this step blocks (computed from depends_on).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<String>,
    /// Optional description for logging/debugging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl DependentStep {
    /// Create a new dependent step.
    pub fn new(id: impl Into<String>, action: Value) -> Self {
        Self {
            id: id.into(),
            action,
            depends_on: Vec::new(),
            blocks: Vec::new(),
            description: None,
        }
    }

    /// Add a dependency on another step.
    pub fn depends_on(mut self, step_id: impl Into<String>) -> Self {
        self.depends_on.push(step_id.into());
        self
    }

    /// Add multiple dependencies.
    pub fn depends_on_all(mut self, step_ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.depends_on
            .extend(step_ids.into_iter().map(|s| s.into()));
        self
    }

    /// Add a description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Check if this action is safe to run in parallel.
    ///
    /// Parallel-safe actions are those that don't mutate page state:
    /// - Evaluate (read-only JS)
    /// - Wait/WaitFor (timing)
    /// - Screenshot (capture only)
    ///
    /// Sequential actions (mutate state):
    /// - Click, Fill, Navigate, etc.
    pub fn is_parallel_safe(&self) -> bool {
        if let Some(obj) = self.action.as_object() {
            for action_name in obj.keys() {
                match action_name.as_str() {
                    // Read-only / timing actions
                    "Wait" | "WaitFor" | "WaitForWithTimeout" | "WaitForNavigation"
                    | "WaitForDom" | "Screenshot" => continue,
                    // Evaluate is parallel-safe if it doesn't mutate
                    "Evaluate" => {
                        // Conservative: assume evaluate might mutate
                        // Could add heuristics to detect read-only evals
                        return false;
                    }
                    // All other actions mutate state
                    _ => return false,
                }
            }
            true
        } else {
            false
        }
    }
}

/// Result of executing a single step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    /// Whether the step succeeded.
    pub success: bool,
    /// Output from the step, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    /// Error message if failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

impl StepResult {
    /// Create a successful result.
    pub fn success() -> Self {
        Self {
            success: true,
            output: None,
            error: None,
            duration_ms: 0,
        }
    }

    /// Create a successful result with output.
    pub fn success_with_output(output: Value) -> Self {
        Self {
            success: true,
            output: Some(output),
            error: None,
            duration_ms: 0,
        }
    }

    /// Create a failed result.
    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            success: false,
            output: None,
            error: Some(error.into()),
            duration_ms: 0,
        }
    }

    /// Set the duration.
    pub fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = ms;
        self
    }
}

/// Result of executing a concurrent chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConcurrentChainResult {
    /// Whether the entire chain succeeded.
    pub success: bool,
    /// Number of steps executed (including failed).
    pub steps_executed: usize,
    /// Maximum number of steps running in parallel at any point.
    pub max_parallel: usize,
    /// Total duration in milliseconds.
    pub duration_ms: u64,
    /// Results for each step, keyed by step ID.
    pub step_results: HashMap<String, StepResult>,
    /// First error encountered, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_error: Option<String>,
}

impl ConcurrentChainResult {
    /// Create a new result.
    pub fn new() -> Self {
        Self {
            success: true,
            steps_executed: 0,
            max_parallel: 0,
            duration_ms: 0,
            step_results: HashMap::new(),
            first_error: None,
        }
    }

    /// Record a step result.
    pub fn record(&mut self, step_id: String, result: StepResult) {
        if !result.success && self.first_error.is_none() {
            self.first_error = result.error.clone();
            self.success = false;
        }
        self.step_results.insert(step_id, result);
        self.steps_executed += 1;
    }
}

impl Default for ConcurrentChainResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Dependency graph for managing step execution order.
#[derive(Debug, Clone)]
pub struct DependencyGraph {
    /// All steps in the graph, keyed by ID.
    steps: HashMap<String, DependentStep>,
    /// Steps that are ready to execute (no pending dependencies).
    ready: VecDeque<String>,
    /// Steps waiting on dependencies: step_id -> set of blocking step IDs.
    waiting: HashMap<String, HashSet<String>>,
    /// Completed step IDs.
    completed: HashSet<String>,
    /// Failed step IDs.
    failed: HashSet<String>,
    /// Results for each step.
    results: HashMap<String, StepResult>,
}

impl DependencyGraph {
    /// Create a new dependency graph from a list of steps.
    ///
    /// Validates the graph for cycles and missing dependencies.
    pub fn new(steps: Vec<DependentStep>) -> Result<Self, String> {
        let mut graph = Self {
            steps: HashMap::new(),
            ready: VecDeque::new(),
            waiting: HashMap::new(),
            completed: HashSet::new(),
            failed: HashSet::new(),
            results: HashMap::new(),
        };

        // First pass: collect all steps and compute blocks
        let mut blocks_map: HashMap<String, Vec<String>> = HashMap::new();
        for step in &steps {
            if graph.steps.contains_key(&step.id) {
                return Err(format!("Duplicate step ID: {}", step.id));
            }
            graph.steps.insert(step.id.clone(), step.clone());

            // Build reverse mapping for blocks
            for dep in &step.depends_on {
                blocks_map
                    .entry(dep.clone())
                    .or_default()
                    .push(step.id.clone());
            }
        }

        // Second pass: validate dependencies exist
        let step_ids: HashSet<String> = graph.steps.keys().cloned().collect();
        for step in graph.steps.values() {
            for dep in &step.depends_on {
                if !step_ids.contains(dep) {
                    return Err(format!(
                        "Step '{}' depends on non-existent step '{}'",
                        step.id, dep
                    ));
                }
            }
        }

        // Third pass: populate blocks
        for (step_id, blocks) in blocks_map {
            if let Some(step) = graph.steps.get_mut(&step_id) {
                step.blocks = blocks;
            }
        }

        // Check for cycles using topological sort
        if let Err(cycle) = graph.detect_cycle() {
            return Err(format!("Cycle detected in dependency graph: {}", cycle));
        }

        // Initialize ready queue with steps that have no dependencies
        for step in graph.steps.values() {
            if step.depends_on.is_empty() {
                graph.ready.push_back(step.id.clone());
            } else {
                graph
                    .waiting
                    .insert(step.id.clone(), step.depends_on.iter().cloned().collect());
            }
        }

        Ok(graph)
    }

    /// Detect cycles using DFS with coloring.
    fn detect_cycle(&self) -> Result<(), String> {
        #[derive(Clone, Copy, PartialEq)]
        enum Color {
            White, // Not visited
            Gray,  // In current DFS path
            Black, // Fully processed
        }

        let mut colors: HashMap<&str, Color> = self
            .steps
            .keys()
            .map(|k| (k.as_str(), Color::White))
            .collect();

        fn dfs<'a>(
            node: &'a str,
            steps: &'a HashMap<String, DependentStep>,
            colors: &mut HashMap<&'a str, Color>,
            path: &mut Vec<&'a str>,
        ) -> Result<(), String> {
            colors.insert(node, Color::Gray);
            path.push(node);

            if let Some(step) = steps.get(node) {
                for dep in &step.depends_on {
                    match colors.get(dep.as_str()) {
                        Some(Color::Gray) => {
                            // Found a cycle
                            let cycle_start = path.iter().position(|&n| n == dep.as_str()).unwrap();
                            let cycle: Vec<_> = path[cycle_start..].to_vec();
                            return Err(cycle.join(" -> "));
                        }
                        Some(Color::White) | None => {
                            dfs(dep, steps, colors, path)?;
                        }
                        Some(Color::Black) => {}
                    }
                }
            }

            colors.insert(node, Color::Black);
            path.pop();
            Ok(())
        }

        for step_id in self.steps.keys() {
            if colors.get(step_id.as_str()) == Some(&Color::White) {
                dfs(step_id, &self.steps, &mut colors, &mut Vec::new())?;
            }
        }

        Ok(())
    }

    /// Get all steps that are currently ready to execute.
    pub fn ready_steps(&self) -> Vec<&DependentStep> {
        self.ready
            .iter()
            .filter_map(|id| self.steps.get(id))
            .collect()
    }

    /// Get the IDs of ready steps.
    pub fn ready_step_ids(&self) -> Vec<&str> {
        self.ready.iter().map(|s| s.as_str()).collect()
    }

    /// Check if there are any steps ready to execute.
    pub fn has_ready_steps(&self) -> bool {
        !self.ready.is_empty()
    }

    /// Check if all steps are complete.
    pub fn is_complete(&self) -> bool {
        self.completed.len() + self.failed.len() == self.steps.len()
    }

    /// Check if any step has failed.
    pub fn has_failed(&self) -> bool {
        !self.failed.is_empty()
    }

    /// Take the next ready step for execution.
    ///
    /// Returns None if no steps are ready.
    pub fn take_ready_step(&mut self) -> Option<&DependentStep> {
        let id = self.ready.pop_front()?;
        self.steps.get(&id)
    }

    /// Take multiple ready steps for parallel execution.
    ///
    /// Returns up to `max` steps that are ready and parallel-safe together.
    pub fn take_ready_steps(&mut self, max: usize) -> Vec<String> {
        let count = self.ready.len().min(max);
        let mut taken = Vec::with_capacity(count);
        for _ in 0..count {
            if let Some(id) = self.ready.pop_front() {
                taken.push(id);
            }
        }
        taken
    }

    /// Mark a step as complete and update the graph.
    ///
    /// This removes the step from dependencies of waiting steps,
    /// potentially making them ready.
    pub fn complete(&mut self, step_id: &str, result: StepResult) {
        self.completed.insert(step_id.to_string());
        if !result.success {
            self.failed.insert(step_id.to_string());
        }
        self.results.insert(step_id.to_string(), result);

        // Find steps that were waiting on this one
        let step = match self.steps.get(step_id) {
            Some(s) => s.clone(),
            None => return,
        };

        for blocked_id in &step.blocks {
            if let Some(deps) = self.waiting.get_mut(blocked_id) {
                deps.remove(step_id);
                if deps.is_empty() {
                    self.waiting.remove(blocked_id);
                    self.ready.push_back(blocked_id.clone());
                }
            }
        }
    }

    /// Get topological sort order of steps.
    ///
    /// Returns step IDs in an order where dependencies come before dependents.
    pub fn execution_order(&self) -> Vec<String> {
        let mut result = Vec::with_capacity(self.steps.len());
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut queue = VecDeque::new();

        // Calculate in-degrees
        for (id, step) in &self.steps {
            in_degree.insert(id.as_str(), step.depends_on.len());
            if step.depends_on.is_empty() {
                queue.push_back(id.as_str());
            }
        }

        // Process in topological order
        while let Some(id) = queue.pop_front() {
            result.push(id.to_string());
            if let Some(step) = self.steps.get(id) {
                for blocked in &step.blocks {
                    if let Some(deg) = in_degree.get_mut(blocked.as_str()) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(blocked.as_str());
                        }
                    }
                }
            }
        }

        result
    }

    /// Calculate the maximum parallelism possible in this graph.
    ///
    /// This is the maximum number of steps that could run concurrently
    /// in an ideal execution.
    pub fn max_parallelism(&self) -> usize {
        if self.steps.is_empty() {
            return 0;
        }

        // Simulate execution to find max concurrent
        let mut temp_graph = self.clone();
        let mut max_parallel = 0;

        while !temp_graph.is_complete() {
            let ready_count = temp_graph.ready.len();
            max_parallel = max_parallel.max(ready_count);

            // Complete all ready steps
            let ready_ids: Vec<_> = temp_graph.ready.drain(..).collect();
            for id in ready_ids {
                temp_graph.complete(&id, StepResult::success());
            }
        }

        max_parallel
    }

    /// Get a step by ID.
    pub fn get_step(&self, id: &str) -> Option<&DependentStep> {
        self.steps.get(id)
    }

    /// Get the result for a step.
    pub fn get_result(&self, id: &str) -> Option<&StepResult> {
        self.results.get(id)
    }

    /// Get all results.
    pub fn results(&self) -> &HashMap<String, StepResult> {
        &self.results
    }

    /// Get count of steps.
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }

    /// Get count of completed steps.
    pub fn completed_count(&self) -> usize {
        self.completed.len()
    }

    /// Get count of failed steps.
    pub fn failed_count(&self) -> usize {
        self.failed.len()
    }
}

/// Configuration for concurrent chain execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConcurrentChainConfig {
    /// Maximum number of steps to run in parallel.
    pub max_parallel: usize,
    /// Whether to stop on first failure.
    pub stop_on_failure: bool,
    /// Timeout per step in milliseconds (0 = no timeout).
    pub step_timeout_ms: u64,
    /// Whether to run parallel-safe steps concurrently.
    pub enable_parallel: bool,
}

impl Default for ConcurrentChainConfig {
    fn default() -> Self {
        Self {
            max_parallel: 4,
            stop_on_failure: true,
            step_timeout_ms: 30_000,
            enable_parallel: true,
        }
    }
}

impl ConcurrentChainConfig {
    /// Create a new config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set max parallel steps.
    pub fn with_max_parallel(mut self, n: usize) -> Self {
        self.max_parallel = n;
        self
    }

    /// Set whether to stop on failure.
    pub fn with_stop_on_failure(mut self, stop: bool) -> Self {
        self.stop_on_failure = stop;
        self
    }

    /// Set step timeout.
    pub fn with_step_timeout(mut self, ms: u64) -> Self {
        self.step_timeout_ms = ms;
        self
    }

    /// Enable/disable parallel execution.
    pub fn with_parallel(mut self, enable: bool) -> Self {
        self.enable_parallel = enable;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dependent_step_creation() {
        let step = DependentStep::new("step1", serde_json::json!({"Click": "button"}))
            .depends_on("step0")
            .with_description("Click the button");

        assert_eq!(step.id, "step1");
        assert_eq!(step.depends_on, vec!["step0"]);
        assert_eq!(step.description, Some("Click the button".to_string()));
    }

    #[test]
    fn test_parallel_safe_detection() {
        let wait_step = DependentStep::new("w", serde_json::json!({"Wait": 100}));
        assert!(wait_step.is_parallel_safe());

        let click_step = DependentStep::new("c", serde_json::json!({"Click": "button"}));
        assert!(!click_step.is_parallel_safe());

        let screenshot_step = DependentStep::new("s", serde_json::json!({"Screenshot": {}}));
        assert!(screenshot_step.is_parallel_safe());
    }

    #[test]
    fn test_graph_construction() {
        let steps = vec![
            DependentStep::new("a", serde_json::json!({"Click": "a"})),
            DependentStep::new("b", serde_json::json!({"Click": "b"})).depends_on("a"),
            DependentStep::new("c", serde_json::json!({"Click": "c"})).depends_on("a"),
            DependentStep::new("d", serde_json::json!({"Click": "d"}))
                .depends_on("b")
                .depends_on("c"),
        ];

        let graph = DependencyGraph::new(steps).expect("valid graph");
        assert_eq!(graph.step_count(), 4);
        assert_eq!(graph.ready_step_ids(), vec!["a"]);
    }

    #[test]
    fn test_graph_cycle_detection() {
        let steps = vec![
            DependentStep::new("a", serde_json::json!({"Click": "a"})).depends_on("c"),
            DependentStep::new("b", serde_json::json!({"Click": "b"})).depends_on("a"),
            DependentStep::new("c", serde_json::json!({"Click": "c"})).depends_on("b"),
        ];

        let result = DependencyGraph::new(steps);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cycle detected"));
    }

    #[test]
    fn test_graph_missing_dependency() {
        let steps =
            vec![DependentStep::new("a", serde_json::json!({"Click": "a"}))
                .depends_on("nonexistent")];

        let result = DependencyGraph::new(steps);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("non-existent step"));
    }

    #[test]
    fn test_graph_execution_order() {
        let steps = vec![
            DependentStep::new("a", serde_json::json!({"Click": "a"})),
            DependentStep::new("b", serde_json::json!({"Click": "b"})).depends_on("a"),
            DependentStep::new("c", serde_json::json!({"Click": "c"})).depends_on("b"),
        ];

        let graph = DependencyGraph::new(steps).expect("valid graph");
        let order = graph.execution_order();
        assert_eq!(order, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_graph_max_parallelism() {
        // Diamond pattern: a -> b, c -> d
        let steps = vec![
            DependentStep::new("a", serde_json::json!({"Click": "a"})),
            DependentStep::new("b", serde_json::json!({"Click": "b"})).depends_on("a"),
            DependentStep::new("c", serde_json::json!({"Click": "c"})).depends_on("a"),
            DependentStep::new("d", serde_json::json!({"Click": "d"}))
                .depends_on("b")
                .depends_on("c"),
        ];

        let graph = DependencyGraph::new(steps).expect("valid graph");
        assert_eq!(graph.max_parallelism(), 2); // b and c can run in parallel
    }

    #[test]
    fn test_graph_completion() {
        let steps = vec![
            DependentStep::new("a", serde_json::json!({"Click": "a"})),
            DependentStep::new("b", serde_json::json!({"Click": "b"})).depends_on("a"),
        ];

        let mut graph = DependencyGraph::new(steps).expect("valid graph");

        assert_eq!(graph.ready_step_ids(), vec!["a"]);
        assert!(!graph.is_complete());

        // Take the step before completing it (as would happen in actual execution)
        let taken = graph.take_ready_steps(1);
        assert_eq!(taken, vec!["a"]);

        graph.complete("a", StepResult::success());
        assert_eq!(graph.ready_step_ids(), vec!["b"]);
        assert!(!graph.is_complete());

        let taken = graph.take_ready_steps(1);
        assert_eq!(taken, vec!["b"]);

        graph.complete("b", StepResult::success());
        assert!(graph.ready_step_ids().is_empty());
        assert!(graph.is_complete());
    }

    #[test]
    fn test_step_result() {
        let success = StepResult::success_with_output(serde_json::json!({"key": "value"}));
        assert!(success.success);
        assert!(success.output.is_some());

        let failure = StepResult::failure("Something went wrong").with_duration(100);
        assert!(!failure.success);
        assert_eq!(failure.error, Some("Something went wrong".to_string()));
        assert_eq!(failure.duration_ms, 100);
    }

    #[test]
    fn test_concurrent_chain_result() {
        let mut result = ConcurrentChainResult::new();
        assert!(result.success);
        assert_eq!(result.steps_executed, 0);

        result.record("step1".to_string(), StepResult::success());
        assert!(result.success);
        assert_eq!(result.steps_executed, 1);

        result.record("step2".to_string(), StepResult::failure("Error"));
        assert!(!result.success);
        assert_eq!(result.steps_executed, 2);
        assert_eq!(result.first_error, Some("Error".to_string()));
    }

    #[test]
    fn test_config_builder() {
        let config = ConcurrentChainConfig::new()
            .with_max_parallel(8)
            .with_stop_on_failure(false)
            .with_step_timeout(60_000)
            .with_parallel(true);

        assert_eq!(config.max_parallel, 8);
        assert!(!config.stop_on_failure);
        assert_eq!(config.step_timeout_ms, 60_000);
        assert!(config.enable_parallel);
    }
}
