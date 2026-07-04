use dashmap::DashMap;
use serde::Serialize;
use std::time::{Duration, Instant};

/// Hard cap on retained sessions. Once reached, the oldest *terminal*
/// (complete/failed) sessions are evicted to make room. In-flight (`Running`)
/// sessions are never evicted, so an active crawl's results are always
/// readable until it finishes.
const MAX_SESSIONS: usize = 256;

/// Age after which a terminal session is dropped even if under the cap. Bounds
/// memory for a long-lived server: completed results stay readable for a
/// window, then the session is reclaimed.
const SESSION_TTL: Duration = Duration::from_secs(60 * 60);

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

    /// Reclaim finished crawl sessions so a long-running server's memory stays
    /// bounded. Without this, every completed crawl's pages/links would be
    /// retained for the process lifetime.
    ///
    /// Two-stage, both bounded by the session count:
    /// 1. Drop terminal sessions older than [`SESSION_TTL`].
    /// 2. If still at/over [`MAX_SESSIONS`], evict the oldest terminal sessions
    ///    until back under the cap.
    ///
    /// Concurrency: keys to remove are collected during a read-only `iter()`,
    /// which is dropped *before* any `remove()` — so no shard guard is ever
    /// held across a mutation (no DashMap self-deadlock). No mutexes, no
    /// blocking, no `.await`. `Running` sessions are left untouched.
    pub fn evict_stale_sessions(&self) {
        let now = Instant::now();

        // Pass 1: classify terminal sessions. TTL-expired ones are queued for
        // removal; the rest are recorded (key, start time) as cap candidates.
        let running_ttl = running_session_ttl();
        let mut expired: Vec<String> = Vec::new();
        let mut terminal: Vec<(String, Instant)> = Vec::new();
        for entry in self.sessions.iter() {
            let session = entry.value();
            if matches!(
                session.status,
                CrawlSessionStatus::Complete | CrawlSessionStatus::Failed
            ) {
                if now.duration_since(session.started_at) >= SESSION_TTL {
                    expired.push(entry.key().clone());
                } else {
                    terminal.push((entry.key().clone(), session.started_at));
                }
            } else if let Some(ttl) = running_ttl {
                // Gated (default: None => skipped => byte-identical). When set,
                // reclaim an in-flight session that has been `Running` past the
                // TTL; its crawl is aborted on removal (see `abort_and_remove`),
                // releasing any browser a stuck/unpolled crawl still holds.
                if now.duration_since(session.started_at) >= ttl {
                    expired.push(entry.key().clone());
                }
            }
        }
        // `iter()` fully consumed and dropped above — safe to mutate now.

        for key in &expired {
            self.abort_and_remove(key);
        }

        // Pass 2: enforce the hard cap by dropping the oldest terminal sessions.
        let len = self.sessions.len();
        if len >= MAX_SESSIONS {
            let overflow = len + 1 - MAX_SESSIONS;
            terminal.sort_by_key(|(_, started_at)| *started_at); // oldest first
            for (key, _) in terminal.into_iter().take(overflow) {
                self.abort_and_remove(&key);
            }
        }
    }

    /// Remove a session and abort its background crawl (releasing any browser it
    /// holds). For terminal sessions the crawl has already finished, so the abort
    /// is a no-op; the cleanup matters for gated `Running` eviction.
    fn abort_and_remove(&self, key: &str) {
        if let Some((_, session)) = self.sessions.remove(key) {
            if let Some(handle) = session.abort {
                handle.abort();
            }
        }
    }
}

/// A stored crawl session with accumulated results.
#[derive(Serialize)]
pub struct CrawlSession {
    pub status: CrawlSessionStatus,
    pub pages: Vec<CrawlPageResult>,
    /// When the session was created; drives age-based eviction in
    /// [`SharedState::evict_stale_sessions`]. Not serialized (`Instant` has no
    /// stable representation).
    #[serde(skip)]
    pub started_at: Instant,
    /// Abort handle for the background crawl task. Aborting it on removal stops
    /// the crawl and releases any browser it holds. Not serialized.
    #[serde(skip)]
    pub abort: Option<tokio::task::AbortHandle>,
}

/// Aborts the wrapped task when dropped. Used to cancel an inline MCP crawl if
/// the request future is dropped (client disconnect) so the crawl doesn't keep
/// running — and holding a browser — with no consumer. It is a no-op on success
/// because the crawl has already finished by the time the inline drain returns.
pub struct AbortOnDrop(pub tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Optional TTL after which an in-flight (`Running`) session is aborted and
/// reclaimed, from `SPIDER_MCP_RUNNING_SESSION_TTL_SECS`. Unset or `0` = never
/// (the historical behaviour: a Running session is retained until it completes),
/// so this is byte-identical by default.
fn running_session_ttl() -> Option<Duration> {
    std::env::var("SPIDER_MCP_RUNNING_SESSION_TTL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&s| s > 0)
        .map(Duration::from_secs)
}

#[derive(Serialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CrawlSessionStatus {
    Running,
    Complete,
    #[allow(dead_code)]
    Failed,
}

#[derive(Serialize, Clone)]
pub struct CrawlPageResult {
    pub url: String,
    pub content: String,
    pub status_code: u16,
    pub links: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(status: CrawlSessionStatus, started_at: Instant) -> CrawlSession {
        CrawlSession {
            status,
            pages: Vec::new(),
            started_at,
            abort: None,
        }
    }

    #[test]
    fn running_sessions_are_never_evicted() {
        let state = SharedState::new();
        let old = Instant::now()
            .checked_sub(SESSION_TTL * 2)
            .unwrap_or_else(Instant::now);
        // A running session well past the TTL must survive — its results may
        // still be streaming in.
        state
            .sessions
            .insert("running".into(), session(CrawlSessionStatus::Running, old));
        state.evict_stale_sessions();
        assert!(state.sessions.contains_key("running"));
    }

    #[test]
    fn ttl_expired_terminal_sessions_are_dropped() {
        let state = SharedState::new();
        let old = Instant::now()
            .checked_sub(SESSION_TTL + Duration::from_secs(1))
            .unwrap_or_else(Instant::now);
        state
            .sessions
            .insert("old".into(), session(CrawlSessionStatus::Complete, old));
        state.sessions.insert(
            "fresh".into(),
            session(CrawlSessionStatus::Complete, Instant::now()),
        );
        state.evict_stale_sessions();
        assert!(!state.sessions.contains_key("old"));
        assert!(state.sessions.contains_key("fresh"));
    }

    #[test]
    fn hard_cap_evicts_oldest_terminal_first() {
        let state = SharedState::new();
        let base = Instant::now()
            .checked_sub(Duration::from_secs(MAX_SESSIONS as u64 + 10))
            .unwrap_or_else(Instant::now);
        // Fill to the cap with terminal sessions of strictly increasing age-anchor.
        for i in 0..MAX_SESSIONS {
            let started = base + Duration::from_secs(i as u64);
            state.sessions.insert(
                format!("s{i}"),
                session(CrawlSessionStatus::Complete, started),
            );
        }
        assert_eq!(state.sessions.len(), MAX_SESSIONS);

        // Inserting one more triggers eviction of exactly the oldest entry.
        state.evict_stale_sessions();
        assert!(state.sessions.len() < MAX_SESSIONS);
        assert!(!state.sessions.contains_key("s0"), "oldest evicted");
        assert!(
            state
                .sessions
                .contains_key(&format!("s{}", MAX_SESSIONS - 1)),
            "newest retained"
        );
    }

    #[test]
    fn running_sessions_survive_cap_pressure() {
        let state = SharedState::new();
        // All running, over the cap: none are terminal, so none can be evicted.
        // Bounded growth is acceptable here — running sessions are transient.
        for i in 0..(MAX_SESSIONS + 5) {
            state.sessions.insert(
                format!("r{i}"),
                session(CrawlSessionStatus::Running, Instant::now()),
            );
        }
        state.evict_stale_sessions();
        assert_eq!(state.sessions.len(), MAX_SESSIONS + 5);
    }
}
