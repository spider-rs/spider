//! Pure-Rust embedded scripting for the agent.
//!
//! Exposes two LLM-callable actions, `RunPython` and `RunJavaScript`, that evaluate
//! arbitrary code in embedded interpreters (`rustpython-vm` and `boa_engine`).
//!
//! ## Topology
//!
//! ```text
//! async caller task                       dedicated OS threads (worker pool)
//! ─────────────────                       ─────────────────────────────────
//!                       flume::bounded
//!    send_async(job) ───────────────────► worker.recv()  (blocking on futex)
//!                                                │
//!                                                ▼
//!                                         RustPython / Boa runs synchronously.
//!                                         Host fns use Handle::block_on for HTTP.
//!                                                │
//!                       tokio::oneshot           ▼
//!    reply_rx.await ◄─────────────────── reply_tx.send(result)
//! ```
//!
//! ## Why this shape
//!
//! * Workers are plain `std::thread`s, **not** on tokio's blocking pool — they cannot
//!   starve reqwest, file I/O, or any other `spawn_blocking` user.
//! * Async caller only `.await`s lock-free primitives (flume async sender,
//!   tokio oneshot, semaphore). Cannot block a runtime worker, cannot deadlock.
//! * Bounded worker count (`num_workers`) hard-caps blast radius if a pathological
//!   script refuses to honor the cooperative interrupt.
//! * Fresh VM per call — zero cross-call state leakage.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, Semaphore};

pub mod js;
pub mod python;
pub mod sandbox;

/// Which interpreter to run a script in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptLanguage {
    /// Python (`rustpython-vm`).
    Python,
    /// JavaScript (`boa_engine`).
    JavaScript,
}

impl ScriptLanguage {
    /// Short label for logging / Display.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::JavaScript => "javascript",
        }
    }
}

/// Runtime knobs for the scripting engine.
///
/// Defaults are conservative for a cloud agent: opt-in network, sandboxed fs only,
/// bounded workers, bounded output. Tune via the engine config.
#[derive(Debug, Clone)]
pub struct ScriptConfig {
    /// Feature switch — when false, all script actions return an error without spawning workers.
    pub enabled: bool,
    /// Number of dedicated OS threads to pre-spawn for script execution.
    pub num_workers: usize,
    /// Maximum jobs waiting in the queue before back-pressure kicks in.
    pub queue_capacity: usize,
    /// Maximum concurrent in-flight script calls (independent of queue depth).
    pub max_concurrent: usize,
    /// Default per-call timeout when the action doesn't specify one.
    pub default_timeout: Duration,
    /// Truncate combined stdout/stderr to this many bytes before returning.
    pub max_output_bytes: usize,
    /// Expose `agent.fetch(url, opts)` to scripts.
    pub allow_network: bool,
    /// Expose `agent.read_file`/`agent.write_file` (sandboxed to per-call tmpdir).
    pub allow_filesystem: bool,
    /// Inject the current page HTML as `agent.html` (truncated to `html_max_bytes`).
    pub inject_page_html: bool,
    /// Cap on `agent.html` length when injected.
    pub html_max_bytes: usize,
}

impl Default for ScriptConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            num_workers: 4,
            queue_capacity: 64,
            max_concurrent: 4,
            default_timeout: Duration::from_secs(5),
            max_output_bytes: 64 * 1024,
            allow_network: false,
            allow_filesystem: true,
            inject_page_html: true,
            html_max_bytes: 32 * 1024,
        }
    }
}

/// Read-only context injected as `agent.*` globals before each script runs.
#[derive(Debug, Clone, Default)]
pub struct ScriptContext {
    /// Current page URL (becomes `agent.url`).
    pub url: Option<String>,
    /// Current page title (becomes `agent.title`).
    pub title: Option<String>,
    /// Current page HTML (becomes `agent.html`, capped by `ScriptConfig::html_max_bytes`).
    pub html: Option<String>,
    /// Free-form agent memory serialized as JSON (becomes `agent.memory`).
    pub memory_json: Option<String>,
}

/// Result of a single script execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScriptResult {
    /// Language that ran the script.
    pub language: String,
    /// Whether the script ran to completion without an interpreter-level error.
    pub success: bool,
    /// Captured stdout from `print`/`agent.log`/`console.log`.
    pub stdout: String,
    /// Captured stderr (interpreter errors, tracebacks, fetch failures).
    pub stderr: String,
    /// JSON-serialized final expression value (best effort), when extractable.
    pub value: Option<serde_json::Value>,
    /// Wall-clock duration in milliseconds.
    pub elapsed_ms: u64,
    /// True if the script was interrupted by the cooperative-cancel hook.
    pub timed_out: bool,
}

impl ScriptResult {
    pub(crate) fn error(language: ScriptLanguage, msg: impl Into<String>, elapsed_ms: u64) -> Self {
        Self {
            language: language.as_str().to_string(),
            success: false,
            stdout: String::new(),
            stderr: msg.into(),
            value: None,
            elapsed_ms,
            timed_out: false,
        }
    }

    pub(crate) fn timeout(language: ScriptLanguage, elapsed_ms: u64) -> Self {
        Self {
            language: language.as_str().to_string(),
            success: false,
            stdout: String::new(),
            stderr: "script timed out".into(),
            value: None,
            elapsed_ms,
            timed_out: true,
        }
    }

    /// Truncate stdout/stderr to `max_output_bytes` (UTF-8 safe).
    pub(crate) fn truncate_output(&mut self, max_output_bytes: usize) {
        truncate_utf8(&mut self.stdout, max_output_bytes);
        truncate_utf8(&mut self.stderr, max_output_bytes);
    }
}

fn truncate_utf8(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut cut = max;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s.truncate(cut);
    s.push_str("\n…[output truncated]");
}

/// Internal job submitted to a worker thread.
pub(crate) struct Job {
    pub language: ScriptLanguage,
    pub code: String,
    pub context: ScriptContext,
    pub config: Arc<ScriptConfig>,
    pub interrupt: Arc<AtomicBool>,
    pub started_at: std::time::Instant,
    pub runtime: tokio::runtime::Handle,
    pub reply: oneshot::Sender<ScriptResult>,
}

/// Public scripting engine — clone-safe handle around the worker pool.
#[derive(Clone)]
pub struct ScriptEngine {
    config: Arc<ScriptConfig>,
    tx: flume::Sender<Job>,
    permits: Arc<Semaphore>,
}

impl std::fmt::Debug for ScriptEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptEngine")
            .field("enabled", &self.config.enabled)
            .field("num_workers", &self.config.num_workers)
            .field("queue_capacity", &self.config.queue_capacity)
            .field("max_concurrent", &self.config.max_concurrent)
            .finish()
    }
}

impl ScriptEngine {
    /// Spawn the worker pool and return a handle. Must be called from inside a tokio runtime.
    ///
    /// `num_workers` dedicated OS threads are pre-spawned and parked on `flume::Receiver::recv()`
    /// until jobs arrive. They exit cleanly when the engine is dropped (channel closes).
    pub fn new(config: ScriptConfig) -> Self {
        let config = Arc::new(config);
        let (tx, rx) = flume::bounded::<Job>(config.queue_capacity.max(1));
        let permits = Arc::new(Semaphore::new(config.max_concurrent.max(1)));

        for i in 0..config.num_workers.max(1) {
            let rx = rx.clone();
            let name = format!("spider-agent-script-{i}");
            // Workers are plain std::thread — NOT on tokio's blocking pool.
            // Stack size matches the chrome-side default (2 MiB) to give the
            // interpreters comfortable headroom.
            let spawn_result = std::thread::Builder::new()
                .name(name.clone())
                .stack_size(2 * 1024 * 1024)
                .spawn(move || worker_loop(rx));
            if let Err(e) = spawn_result {
                log::error!("failed to spawn script worker {name}: {e}");
            }
        }

        Self {
            config,
            tx,
            permits,
        }
    }

    /// Whether the engine is enabled (feature-flag + config switch).
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Engine config snapshot.
    pub fn config(&self) -> &ScriptConfig {
        &self.config
    }

    /// Run a Python script. Async — never blocks the calling runtime worker.
    pub async fn run_python(
        &self,
        code: String,
        context: ScriptContext,
        timeout_override: Option<Duration>,
    ) -> ScriptResult {
        self.run(ScriptLanguage::Python, code, context, timeout_override)
            .await
    }

    /// Run a JavaScript script. Async — never blocks the calling runtime worker.
    pub async fn run_javascript(
        &self,
        code: String,
        context: ScriptContext,
        timeout_override: Option<Duration>,
    ) -> ScriptResult {
        self.run(ScriptLanguage::JavaScript, code, context, timeout_override)
            .await
    }

    async fn run(
        &self,
        language: ScriptLanguage,
        code: String,
        context: ScriptContext,
        timeout_override: Option<Duration>,
    ) -> ScriptResult {
        let start = std::time::Instant::now();

        if !self.config.enabled {
            return ScriptResult::error(language, "scripting engine is disabled", 0);
        }

        // Acquire a concurrency permit — bounded in-flight independent of queue depth.
        let _permit = match self.permits.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => return ScriptResult::error(language, "permit acquire failed", 0),
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        let runtime = tokio::runtime::Handle::current();

        let job = Job {
            language,
            code,
            context,
            config: self.config.clone(),
            interrupt: interrupt.clone(),
            started_at: start,
            runtime,
            reply: reply_tx,
        };

        if self.tx.send_async(job).await.is_err() {
            return ScriptResult::error(
                language,
                "script worker pool is shut down",
                elapsed(start),
            );
        }

        let deadline = timeout_override.unwrap_or(self.config.default_timeout);
        match tokio::time::timeout(deadline, reply_rx).await {
            Ok(Ok(mut result)) => {
                result.truncate_output(self.config.max_output_bytes);
                result
            }
            Ok(Err(_)) => {
                // Worker dropped the reply channel without sending — should not happen,
                // but recover instead of panicking.
                ScriptResult::error(
                    language,
                    "script worker dropped reply channel",
                    elapsed(start),
                )
            }
            Err(_) => {
                // Signal the worker to bail via cooperative-cancel hook.
                interrupt.store(true, Ordering::Relaxed);
                ScriptResult::timeout(language, elapsed(start))
            }
        }
    }
}

fn elapsed(start: std::time::Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

fn worker_loop(rx: flume::Receiver<Job>) {
    log::debug!(
        "script worker started on thread {:?}",
        std::thread::current().name()
    );
    while let Ok(job) = rx.recv() {
        let started_at = job.started_at;
        let language = job.language;

        // `catch_unwind` guards against interpreter-level panics so a single bad
        // script can never tear down a worker thread (and starve the queue).
        // UnwindSafe is asserted via AssertUnwindSafe — the job is owned, used
        // once, and dropped at the end of this iteration; nothing crosses the
        // catch_unwind boundary that could observe a poisoned state.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match language {
            ScriptLanguage::Python => python::run(&job),
            ScriptLanguage::JavaScript => js::run(&job),
        }));

        let mut result = match outcome {
            Ok(Ok(r)) => r,
            Ok(Err(err)) => ScriptResult::error(
                language,
                format!("internal error: {err}"),
                elapsed(started_at),
            ),
            Err(panic_payload) => {
                let msg = panic_message(panic_payload);
                log::error!("script worker caught panic: {msg}");
                ScriptResult::error(
                    language,
                    format!("interpreter panic: {msg}"),
                    elapsed(started_at),
                )
            }
        };
        if result.elapsed_ms == 0 {
            result.elapsed_ms = elapsed(started_at);
        }
        // `oneshot::Sender::send` returns Err only if the receiver was dropped
        // (caller timed out). That's an expected race; we just discard the result.
        let _ = job.reply.send(result);
    }
    log::debug!(
        "script worker stopped on thread {:?}",
        std::thread::current().name()
    );
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_utf8_safe() {
        let mut s = "héllo, wörld".to_string();
        truncate_utf8(&mut s, 5);
        // Must remain valid UTF-8 even when the byte boundary falls inside a multi-byte char.
        assert!(s.is_char_boundary(s.len() - "\n…[output truncated]".len()));
        assert!(s.starts_with("h"));
    }

    #[test]
    fn engine_disabled_by_default() {
        let cfg = ScriptConfig::default();
        assert!(!cfg.enabled);
    }
}
