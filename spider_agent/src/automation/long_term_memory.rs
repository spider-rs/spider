//! Long-term experience memory for agent learning.
//!
//! Stores successful automation experiences in an append-only `.jsonl` log
//! and uses `memvid-rs` for semantic search over past strategies.
//! The memvid video+index is rebuilt lazily when recall is needed after
//! new experiences have been added.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Outcome of an automation experience.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExperienceOutcome {
    /// Automation completed successfully.
    Success,
    /// Automation failed.
    Failure,
}

impl std::fmt::Display for ExperienceOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Success => write!(f, "success"),
            Self::Failure => write!(f, "failure"),
        }
    }
}

/// A single recorded automation experience.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceRecord {
    /// Type of challenge encountered (e.g. "image-grid-selection").
    pub challenge_type: String,
    /// Domain/path pattern of the URL.
    pub url_pattern: String,
    /// Keywords extracted from the page title.
    pub title_keywords: Vec<String>,
    /// CSS classes/elements that identify the challenge.
    pub html_signals: Vec<String>,
    /// Human-readable summary of what strategy worked.
    pub strategy_summary: String,
    /// Serialized action steps as JSON.
    pub steps_json: String,
    /// Whether the automation succeeded or failed.
    pub outcome: ExperienceOutcome,
    /// Number of rounds the automation took.
    pub rounds_taken: u32,
    /// Unix timestamp of when the experience was recorded.
    pub timestamp: u64,
}

impl ExperienceRecord {
    /// Build an experience record from a completed automation session.
    ///
    /// Extracts URL pattern, title keywords, and constructs a strategy summary
    /// from the session data.
    pub fn from_session(
        url: &str,
        label: &str,
        memory: &super::AutomationMemory,
        steps_executed: usize,
        rounds: u32,
    ) -> Self {
        // Extract domain pattern from URL
        let url_pattern = url
            .split('/')
            .take(3)
            .collect::<Vec<_>>()
            .join("/")
            + "/*";

        // Extract title keywords (non-trivial words)
        let title_keywords: Vec<String> = label
            .split_whitespace()
            .filter(|w| w.len() > 2)
            .map(|w| w.to_lowercase())
            .collect();

        // Build strategy summary from memory actions
        let actions: Vec<String> = memory
            .action_history
            .iter()
            .rev()
            .take(10)
            .rev()
            .cloned()
            .collect();
        let strategy_summary = if actions.is_empty() {
            format!("Completed '{}' in {} steps", label, steps_executed)
        } else {
            let action_summary = actions.join(" → ");
            if action_summary.len() > 300 {
                format!("{}...", &action_summary[..297])
            } else {
                action_summary
            }
        };

        let steps_json = serde_json::to_string(&actions).unwrap_or_default();

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Self {
            challenge_type: label.to_string(),
            url_pattern,
            title_keywords,
            html_signals: Vec::new(),
            strategy_summary,
            steps_json,
            outcome: ExperienceOutcome::Success,
            rounds_taken: rounds,
            timestamp,
        }
    }

    /// Convert this record to a searchable text representation for memvid indexing.
    fn to_search_text(&self) -> String {
        format!(
            "challenge: {} url: {} keywords: {} strategy: {} outcome: {} rounds: {}",
            self.challenge_type,
            self.url_pattern,
            self.title_keywords.join(", "),
            self.strategy_summary,
            self.outcome,
            self.rounds_taken,
        )
    }
}

/// A recalled experience with its relevance score.
#[derive(Debug, Clone)]
pub struct RecalledExperience {
    /// The original experience record.
    pub record: ExperienceRecord,
    /// Relevance score from semantic search (0.0 - 1.0).
    pub relevance: f32,
}

/// Configuration for the experience memory system.
#[derive(Debug, Clone)]
pub struct ExperienceMemoryConfig {
    /// Maximum number of experiences to retrieve per query.
    pub max_recall: usize,
    /// Maximum total characters of context to inject into prompts.
    pub max_context_chars: usize,
    /// Minimum relevance score threshold for recall results.
    pub min_relevance_score: f32,
    /// Text chunk size for memvid indexing.
    pub chunk_size: usize,
    /// Chunk overlap for memvid indexing.
    pub overlap: usize,
}

impl Default for ExperienceMemoryConfig {
    fn default() -> Self {
        Self {
            max_recall: 3,
            max_context_chars: 2000,
            min_relevance_score: 0.15,
            chunk_size: 512,
            overlap: 32,
        }
    }
}

/// Statistics about the experience memory.
#[derive(Debug, Clone, Default)]
pub struct MemoryStats {
    /// Total number of stored experiences.
    pub experience_count: usize,
    /// Size of the .jsonl log file in bytes.
    pub log_size_bytes: u64,
    /// Size of the memvid video file in bytes.
    pub video_size_bytes: u64,
    /// Size of the memvid index file in bytes.
    pub index_size_bytes: u64,
    /// Whether the index needs rebuilding.
    pub dirty: bool,
}

/// Manages long-term experience storage and semantic retrieval.
///
/// Experiences are stored in an append-only `.jsonl` file (source of truth).
/// A memvid video+index is rebuilt lazily when recall is needed after new
/// experiences have been added. This avoids the cost of rebuilding on every
/// store operation.
///
/// Not `Clone` because it owns the memvid retriever handle.
pub struct ExperienceMemory {
    /// Path to the .jsonl append-only experience log.
    log_path: PathBuf,
    /// Path to the memvid .mp4 video file.
    video_path: PathBuf,
    /// Path to the memvid .db index file.
    index_path: PathBuf,
    /// Lazy-loaded memvid retriever.
    retriever: Option<memvid_rs::MemvidRetriever>,
    /// True when .jsonl has new entries since last index build.
    dirty: bool,
    /// Cache of query hash → recalled experiences.
    recall_cache: DashMap<u64, Vec<RecalledExperience>>,
    /// Configuration for this memory instance.
    pub config: ExperienceMemoryConfig,
}

// SAFETY: ExperienceMemory is always accessed through Arc<tokio::sync::RwLock<>>,
// which provides proper synchronization. The non-Send/Sync interiors (rusqlite's
// RefCell and ffmpeg raw pointers inside MemvidRetriever) are never accessed
// concurrently — the RwLock guards all access.
unsafe impl Send for ExperienceMemory {}
unsafe impl Sync for ExperienceMemory {}

impl std::fmt::Debug for ExperienceMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExperienceMemory")
            .field("log_path", &self.log_path)
            .field("video_path", &self.video_path)
            .field("index_path", &self.index_path)
            .field("has_retriever", &self.retriever.is_some())
            .field("dirty", &self.dirty)
            .field("cache_entries", &self.recall_cache.len())
            .field("config", &self.config)
            .finish()
    }
}

impl ExperienceMemory {
    /// Create or load an experience memory from a directory.
    ///
    /// If the directory doesn't exist, it will be created.
    /// If existing `.jsonl` / `.mp4` / `.db` files are found, they are loaded.
    pub async fn new(dir: impl Into<PathBuf>, config: ExperienceMemoryConfig) -> std::io::Result<Self> {
        let dir = dir.into();
        tokio::fs::create_dir_all(&dir).await?;

        let log_path = dir.join("experiences.jsonl");
        let video_path = dir.join("experiences.mp4");
        let index_path = dir.join("experiences.db");

        // Determine if we have unindexed experiences
        let log_exists = log_path.exists();
        let index_exists = video_path.exists() && index_path.exists();
        let dirty = log_exists && !index_exists;

        Ok(Self {
            log_path,
            video_path,
            index_path,
            retriever: None,
            dirty,
            recall_cache: DashMap::new(),
            config,
        })
    }

    /// Append an experience record to the .jsonl log.
    ///
    /// Marks the index as dirty so it will be rebuilt on next recall.
    pub async fn store_experience(&mut self, record: &ExperienceRecord) -> std::io::Result<()> {
        use tokio::io::AsyncWriteExt;

        let mut line = serde_json::to_string(record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        line.push('\n');

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.flush().await?;

        self.dirty = true;
        self.recall_cache.clear();
        self.retriever = None;

        log::debug!(
            "Stored experience: challenge={} outcome={} rounds={}",
            record.challenge_type,
            record.outcome,
            record.rounds_taken,
        );

        Ok(())
    }

    /// Recall relevant past experiences for a given query.
    ///
    /// If the index is dirty (new experiences added since last build),
    /// it will be rebuilt first. Results are cached by query hash.
    pub async fn recall(
        &mut self,
        query: &str,
        k: usize,
    ) -> Result<Vec<RecalledExperience>, Box<dyn std::error::Error + Send + Sync>> {
        let query_hash = super::fnv1a64(query.as_bytes());

        // Check cache first
        if let Some(cached) = self.recall_cache.get(&query_hash) {
            return Ok(cached.clone());
        }

        // Rebuild index if dirty
        if self.dirty {
            self.rebuild_index().await?;
        }

        // Load retriever if needed
        if self.retriever.is_none() {
            if !self.video_path.exists() || !self.index_path.exists() {
                return Ok(Vec::new());
            }
            self.retriever = Some(memvid_rs::MemvidRetriever::new(
                self.video_path.to_string_lossy().as_ref(),
                self.index_path.to_string_lossy().as_ref(),
            ).await?);
        }

        let retriever = self.retriever.as_mut().expect("retriever loaded");
        let search_results = retriever.search(query, k).await?;

        // Load all records for matching
        let all_records = self.load_all_records().await?;

        let mut experiences = Vec::new();
        for (score, text) in &search_results {
            // Match search result text back to an experience record
            let relevance = *score;
            if relevance < self.config.min_relevance_score {
                continue;
            }

            // Find the best matching record for this search result
            if let Some(record) = Self::match_record_to_search_result(&all_records, text) {
                experiences.push(RecalledExperience {
                    record,
                    relevance,
                });
            }
        }

        // Cache results
        self.recall_cache.insert(query_hash, experiences.clone());

        Ok(experiences)
    }

    /// Format recalled experiences as context suitable for prompt injection.
    ///
    /// Output is capped at `max_context_chars` to keep prompts manageable.
    pub fn recall_to_context(
        experiences: &[RecalledExperience],
        max_context_chars: usize,
    ) -> String {
        if experiences.is_empty() {
            return String::new();
        }

        let mut ctx = String::from("## Learned Strategies (from past experience)\n");
        let mut remaining = max_context_chars.saturating_sub(ctx.len());

        for exp in experiences {
            let mut entry = format!(
                "### {} (relevance: {:.2})\n",
                exp.record.challenge_type, exp.relevance,
            );
            entry.push_str(&format!("URL pattern: {}\n", exp.record.url_pattern));

            // Truncate strategy summary if needed
            let summary = if exp.record.strategy_summary.len() > 300 {
                format!("{}...", &exp.record.strategy_summary[..297])
            } else {
                exp.record.strategy_summary.clone()
            };
            entry.push_str(&format!("Strategy: {}\n", summary));

            if exp.record.rounds_taken > 0 {
                entry.push_str(&format!(
                    "Outcome: {} in {} rounds\n",
                    exp.record.outcome, exp.record.rounds_taken
                ));
            }
            entry.push_str("---\n");

            if entry.len() > remaining {
                break;
            }
            remaining -= entry.len();
            ctx.push_str(&entry);
        }

        ctx
    }

    /// Remove experiences matching a semantic query from the log.
    ///
    /// Searches for the top-k matches and removes them from the `.jsonl` file.
    /// Marks the index as dirty.
    pub async fn release_by_query(
        &mut self,
        query: &str,
        k: usize,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let matches = self.recall(query, k).await?;
        if matches.is_empty() {
            return Ok(0);
        }

        // Collect timestamps of records to remove
        let remove_timestamps: std::collections::HashSet<u64> =
            matches.iter().map(|m| m.record.timestamp).collect();

        let all_records = self.load_all_records().await?;
        let remaining: Vec<&ExperienceRecord> = all_records
            .iter()
            .filter(|r| !remove_timestamps.contains(&r.timestamp))
            .collect();

        let removed_count = all_records.len() - remaining.len();

        // Rewrite the log with remaining records
        self.rewrite_log(&remaining).await?;

        self.dirty = true;
        self.recall_cache.clear();
        self.retriever = None;

        Ok(removed_count)
    }

    /// Delete all experience data and reset state.
    pub async fn release_all(&mut self) -> std::io::Result<()> {
        for path in [&self.log_path, &self.video_path, &self.index_path] {
            if path.exists() {
                let _ = tokio::fs::remove_file(path).await;
            }
        }
        self.dirty = false;
        self.recall_cache.clear();
        self.retriever = None;
        Ok(())
    }

    /// Get statistics about the current memory state.
    pub fn stats(&self) -> MemoryStats {
        let log_size = std::fs::metadata(&self.log_path)
            .map(|m| m.len())
            .unwrap_or(0);
        let video_size = std::fs::metadata(&self.video_path)
            .map(|m| m.len())
            .unwrap_or(0);
        let index_size = std::fs::metadata(&self.index_path)
            .map(|m| m.len())
            .unwrap_or(0);

        // Count lines in .jsonl for experience count
        let count = std::fs::read_to_string(&self.log_path)
            .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0);

        MemoryStats {
            experience_count: count,
            log_size_bytes: log_size,
            video_size_bytes: video_size,
            index_size_bytes: index_size,
            dirty: self.dirty,
        }
    }

    /// Force rebuild the memvid index from the .jsonl log.
    pub async fn flush(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.rebuild_index().await?;
        Ok(())
    }

    /// Clear the recall cache. Called at the start of each new automation run.
    pub fn clear_cache(&self) {
        self.recall_cache.clear();
    }

    // ---- Internal helpers ----

    /// Rebuild the memvid video+index from the .jsonl log.
    async fn rebuild_index(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let records = self.load_all_records().await?;
        if records.is_empty() {
            self.dirty = false;
            return Ok(());
        }

        // Build combined text from all records
        let combined_text: String = records
            .iter()
            .map(|r| r.to_search_text())
            .collect::<Vec<_>>()
            .join("\n\n");

        let video_path_str = self.video_path.to_string_lossy().to_string();
        let index_path_str = self.index_path.to_string_lossy().to_string();
        let chunk_size = self.config.chunk_size;
        let overlap = self.config.overlap;

        // Encode and build index
        let mut encoder = memvid_rs::MemvidEncoder::new(None).await?;
        encoder.add_text(&combined_text, chunk_size, overlap).await?;
        encoder.build_video(&video_path_str, &index_path_str).await?;

        self.dirty = false;
        self.retriever = None; // will be reloaded on next recall
        self.recall_cache.clear();

        log::debug!(
            "Rebuilt experience index: {} records, video={}, index={}",
            records.len(),
            self.video_path.display(),
            self.index_path.display(),
        );

        Ok(())
    }

    /// Load all experience records from the .jsonl log.
    async fn load_all_records(&self) -> std::io::Result<Vec<ExperienceRecord>> {
        if !self.log_path.exists() {
            return Ok(Vec::new());
        }

        let content = tokio::fs::read_to_string(&self.log_path).await?;
        let records: Vec<ExperienceRecord> = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();

        Ok(records)
    }

    /// Rewrite the .jsonl log with only the given records.
    async fn rewrite_log(&self, records: &[&ExperienceRecord]) -> std::io::Result<()> {
        use tokio::io::AsyncWriteExt;

        let mut content = String::new();
        for record in records {
            let line = serde_json::to_string(record)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            content.push_str(&line);
            content.push('\n');
        }

        let mut file = tokio::fs::File::create(&self.log_path).await?;
        file.write_all(content.as_bytes()).await?;
        file.flush().await?;

        Ok(())
    }

    /// Match a search result text back to the most similar experience record.
    fn match_record_to_search_result(
        records: &[ExperienceRecord],
        search_text: &str,
    ) -> Option<ExperienceRecord> {
        let search_lower = search_text.to_lowercase();
        records
            .iter()
            .max_by_key(|r| {
                let record_text = r.to_search_text().to_lowercase();
                // Simple overlap score: count shared words
                let record_words: std::collections::HashSet<&str> =
                    record_text.split_whitespace().collect();
                let search_words: std::collections::HashSet<&str> =
                    search_lower.split_whitespace().collect();
                record_words.intersection(&search_words).count()
            })
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_experience_outcome_display() {
        assert_eq!(ExperienceOutcome::Success.to_string(), "success");
        assert_eq!(ExperienceOutcome::Failure.to_string(), "failure");
    }

    #[test]
    fn test_experience_record_serialization() {
        let record = ExperienceRecord {
            challenge_type: "image-grid-selection".to_string(),
            url_pattern: "https://example.com/*".to_string(),
            title_keywords: vec!["select".to_string(), "images".to_string()],
            html_signals: vec!["grid-item".to_string()],
            strategy_summary: "Click matching grid tiles".to_string(),
            steps_json: "[]".to_string(),
            outcome: ExperienceOutcome::Success,
            rounds_taken: 3,
            timestamp: 1700000000,
        };

        let json = serde_json::to_string(&record).unwrap();
        let deserialized: ExperienceRecord = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.challenge_type, "image-grid-selection");
        assert_eq!(deserialized.outcome, ExperienceOutcome::Success);
        assert_eq!(deserialized.rounds_taken, 3);
        assert_eq!(deserialized.title_keywords.len(), 2);
    }

    #[test]
    fn test_config_defaults() {
        let config = ExperienceMemoryConfig::default();
        assert_eq!(config.max_recall, 3);
        assert_eq!(config.max_context_chars, 2000);
        assert_eq!(config.min_relevance_score, 0.15);
        assert_eq!(config.chunk_size, 512);
        assert_eq!(config.overlap, 32);
    }

    #[test]
    fn test_recall_to_context_empty() {
        let ctx = ExperienceMemory::recall_to_context(&[], 2000);
        assert!(ctx.is_empty());
    }

    #[test]
    fn test_recall_to_context_formatting() {
        let experiences = vec![
            RecalledExperience {
                record: ExperienceRecord {
                    challenge_type: "grid-selection".to_string(),
                    url_pattern: "https://example.com/*".to_string(),
                    title_keywords: vec!["select".to_string()],
                    html_signals: vec![],
                    strategy_summary: "Click grid tiles matching description".to_string(),
                    steps_json: "[]".to_string(),
                    outcome: ExperienceOutcome::Success,
                    rounds_taken: 2,
                    timestamp: 1700000000,
                },
                relevance: 0.82,
            },
        ];

        let ctx = ExperienceMemory::recall_to_context(&experiences, 2000);
        assert!(ctx.contains("Learned Strategies"));
        assert!(ctx.contains("grid-selection"));
        assert!(ctx.contains("0.82"));
        assert!(ctx.contains("Click grid tiles matching description"));
        assert!(ctx.contains("example.com"));
    }

    #[test]
    fn test_recall_to_context_respects_max_chars() {
        let experiences: Vec<RecalledExperience> = (0..100)
            .map(|i| RecalledExperience {
                record: ExperienceRecord {
                    challenge_type: format!("challenge-type-{}", i),
                    url_pattern: format!("https://example{}.com/*", i),
                    title_keywords: vec![],
                    html_signals: vec![],
                    strategy_summary: "A".repeat(200),
                    steps_json: "[]".to_string(),
                    outcome: ExperienceOutcome::Success,
                    rounds_taken: 1,
                    timestamp: 1700000000 + i as u64,
                },
                relevance: 0.9,
            })
            .collect();

        let ctx = ExperienceMemory::recall_to_context(&experiences, 500);
        assert!(ctx.len() <= 600); // some slack for the last entry that fits
    }

    #[test]
    fn test_memory_stats_default() {
        let stats = MemoryStats::default();
        assert_eq!(stats.experience_count, 0);
        assert_eq!(stats.log_size_bytes, 0);
        assert!(!stats.dirty);
    }

    #[test]
    fn test_experience_record_to_search_text() {
        let record = ExperienceRecord {
            challenge_type: "image-grid".to_string(),
            url_pattern: "https://example.com/*".to_string(),
            title_keywords: vec!["select".to_string(), "all".to_string()],
            html_signals: vec![],
            strategy_summary: "Click matching tiles".to_string(),
            steps_json: "[]".to_string(),
            outcome: ExperienceOutcome::Success,
            rounds_taken: 3,
            timestamp: 0,
        };

        let text = record.to_search_text();
        assert!(text.contains("image-grid"));
        assert!(text.contains("example.com"));
        assert!(text.contains("select, all"));
        assert!(text.contains("Click matching tiles"));
        assert!(text.contains("success"));
    }

    #[tokio::test]
    async fn test_store_and_load() {
        let dir = std::env::temp_dir().join(format!("spider_exp_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);

        let mut mem = ExperienceMemory::new(&dir, ExperienceMemoryConfig::default())
            .await
            .unwrap();

        let record = ExperienceRecord {
            challenge_type: "test".to_string(),
            url_pattern: "https://test.com/*".to_string(),
            title_keywords: vec!["test".to_string()],
            html_signals: vec![],
            strategy_summary: "Test strategy".to_string(),
            steps_json: "[]".to_string(),
            outcome: ExperienceOutcome::Success,
            rounds_taken: 1,
            timestamp: 1700000000,
        };

        mem.store_experience(&record).await.unwrap();

        let stats = mem.stats();
        assert_eq!(stats.experience_count, 1);
        assert!(stats.dirty);
        assert!(stats.log_size_bytes > 0);

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_release_all() {
        let dir = std::env::temp_dir().join(format!("spider_exp_release_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);

        let mut mem = ExperienceMemory::new(&dir, ExperienceMemoryConfig::default())
            .await
            .unwrap();

        let record = ExperienceRecord {
            challenge_type: "test".to_string(),
            url_pattern: "https://test.com/*".to_string(),
            title_keywords: vec![],
            html_signals: vec![],
            strategy_summary: "Test".to_string(),
            steps_json: "[]".to_string(),
            outcome: ExperienceOutcome::Success,
            rounds_taken: 1,
            timestamp: 1700000000,
        };

        mem.store_experience(&record).await.unwrap();
        assert!(mem.log_path.exists());

        mem.release_all().await.unwrap();
        assert!(!mem.log_path.exists());

        let stats = mem.stats();
        assert_eq!(stats.experience_count, 0);
        assert!(!stats.dirty);

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }
}
