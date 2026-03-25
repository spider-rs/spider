use dashmap::DashMap;
use serde::Serialize;
use std::time::Instant;

/// Shared server state for crawl sessions.
pub struct SharedState {
    pub sessions: DashMap<String, CrawlSession>,
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
        }
    }
}

/// A stored crawl session with accumulated results.
#[derive(Serialize)]
pub struct CrawlSession {
    pub status: CrawlSessionStatus,
    pub pages: Vec<CrawlPageResult>,
    #[serde(skip)]
    pub started_at: Instant,
}

#[derive(Serialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CrawlSessionStatus {
    Running,
    Complete,
    Failed,
}

#[derive(Serialize, Clone)]
pub struct CrawlPageResult {
    pub url: String,
    pub content: String,
    pub status_code: u16,
    pub links: Vec<String>,
}
