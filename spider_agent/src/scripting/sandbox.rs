//! Host helpers shared by both interpreters: HTTP fetch, sandboxed fs, output capture.
//!
//! Everything in this module runs on a **worker OS thread**, not on a tokio runtime
//! thread. HTTP calls bridge to the existing reqwest client via the runtime handle
//! captured at job-submit time — `Handle::block_on` from a non-runtime thread parks
//! the OS thread on a futex until the async future completes on the runtime, with
//! no risk of starving runtime workers.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Default timeout for `agent.fetch` when the script doesn't override.
const DEFAULT_FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Hard cap on a single `agent.fetch` response body (1 MiB).
const FETCH_BODY_MAX_BYTES: usize = 1024 * 1024;
/// Hard cap on a single sandboxed file read or write (1 MiB).
pub(crate) const SANDBOX_FILE_MAX_BYTES: usize = 1024 * 1024;

thread_local! {
    /// Per-worker-thread output buffer. Each script runs synchronously on exactly
    /// one worker thread, so a thread-local `RefCell` is enough — no Mutex, no
    /// atomics, zero contention. The buffer is reset by `clear` at the start of
    /// each script and drained by `drain_to_string` at the end.
    static SCRIPT_OUTPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Worker-thread-local output buffer. Cheap to clone (zero-sized).
///
/// **No locks.** All operations route through a thread-local `RefCell`. The
/// scripting engine's worker pool spawns one OS thread per worker; each
/// thread runs scripts sequentially, so single-threaded access is the model.
#[derive(Clone, Copy, Default)]
pub(crate) struct OutputBuffer;

impl OutputBuffer {
    pub(crate) fn new() -> Self {
        // Reset the per-thread buffer at the start of each script so leftover
        // bytes from a prior job on the same worker can't leak into a new run.
        SCRIPT_OUTPUT.with(|cell| cell.borrow_mut().clear());
        Self
    }

    pub(crate) fn write_str(&self, s: &str) {
        SCRIPT_OUTPUT.with(|cell| {
            // `borrow_mut` panics only on overlapping borrows; this is the sole
            // borrow site (no nested calls), so it cannot conflict.
            if let Ok(mut buf) = cell.try_borrow_mut() {
                buf.extend_from_slice(s.as_bytes());
            }
        });
    }

    pub(crate) fn drain_to_string(&self) -> String {
        SCRIPT_OUTPUT.with(|cell| {
            let bytes = match cell.try_borrow_mut() {
                Ok(mut buf) => std::mem::take(&mut *buf),
                Err(_) => return String::new(),
            };
            match String::from_utf8(bytes) {
                Ok(s) => s,
                Err(e) => String::from_utf8_lossy(&e.into_bytes()).into_owned(),
            }
        })
    }
}

/// Shape of the request object accepted by `agent.fetch`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct FetchRequest {
    pub method: Option<String>,
    pub headers: Option<std::collections::HashMap<String, String>>,
    pub body: Option<String>,
    pub timeout_ms: Option<u64>,
}

/// Shape of the response returned to the script.
#[derive(Debug, Serialize)]
pub(crate) struct FetchResponse {
    pub status: u16,
    pub ok: bool,
    pub url: String,
    pub headers: std::collections::HashMap<String, String>,
    pub body: String,
    pub truncated: bool,
}

/// Process-wide HTTP client used by `agent.fetch`. Built on the agent's existing
/// reqwest dep so we get one connection pool + TLS config across spider_agent.
///
/// Returns `Err` (not a panic) if the client cannot be initialized — `reqwest`'s
/// own `Client::new` panics on TLS-init failure, so we cache an `Option` and
/// surface a string error to the script.
fn http_client() -> Result<&'static reqwest::Client, &'static str> {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    let cached = CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("spider_agent_script/", env!("CARGO_PKG_VERSION")))
            .build()
            .ok()
    });
    cached
        .as_ref()
        .ok_or("failed to initialize HTTP client (TLS backend)")
}

/// Perform a fetch from a worker thread. `runtime` is the handle captured at job
/// submission time. `interrupt` lets a timeout short-circuit the call.
pub(crate) fn agent_fetch_blocking(
    runtime: &tokio::runtime::Handle,
    interrupt: &Arc<AtomicBool>,
    url: &str,
    req: FetchRequest,
) -> Result<FetchResponse, String> {
    if interrupt.load(Ordering::Relaxed) {
        return Err("interrupted".into());
    }

    let method = req.method.as_deref().unwrap_or("GET").to_ascii_uppercase();
    let method = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|e| format!("invalid method: {e}"))?;

    let timeout = req
        .timeout_ms
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_FETCH_TIMEOUT);

    let client = http_client().map_err(|e| e.to_string())?.clone();
    let url_owned = url.to_string();

    // Handle::block_on from a non-runtime thread is the canonical sync→async bridge.
    // The OS thread parks on a futex; no tokio worker is consumed.
    // The per-request `timeout(...)` plus the outer script timeout bound total wall time;
    // we don't need to interrupt mid-fetch — reqwest's own timeout handles that.
    runtime.block_on(async move {
        let mut builder = client.request(method, &url_owned).timeout(timeout);
        if let Some(headers) = req.headers {
            for (k, v) in headers {
                builder = builder.header(k, v);
            }
        }
        if let Some(body) = req.body {
            builder = builder.body(body);
        }

        let response = builder
            .send()
            .await
            .map_err(|e| format!("fetch error: {e}"))?;

        let status = response.status().as_u16();
        let ok = response.status().is_success();
        let final_url = response.url().to_string();
        let mut headers = std::collections::HashMap::new();
        for (k, v) in response.headers().iter() {
            if let Ok(value) = v.to_str() {
                headers.insert(k.as_str().to_string(), value.to_string());
            }
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| format!("read body: {e}"))?;
        let (slice, truncated) = if bytes.len() > FETCH_BODY_MAX_BYTES {
            (&bytes[..FETCH_BODY_MAX_BYTES], true)
        } else {
            (&bytes[..], false)
        };
        let body = String::from_utf8_lossy(slice).into_owned();

        Ok(FetchResponse {
            status,
            ok,
            url: final_url,
            headers,
            body,
            truncated,
        })
    })
}

/// Capability-style filesystem handle rooted at a per-call tmpdir.
///
/// `cap_std::fs::Dir` structurally rejects `..` and absolute paths — the handle
/// is rooted, so a script cannot escape the tmpdir regardless of clever input.
pub(crate) struct SandboxedDir {
    dir: cap_std::fs::Dir,
    root: PathBuf,
    _tempdir: tempfile::TempDir,
}

impl SandboxedDir {
    pub(crate) fn new() -> std::io::Result<Self> {
        let tempdir = tempfile::tempdir()?;
        let root = tempdir.path().to_path_buf();
        // cap_std::ambient_authority is the explicit opt-in for opening a Dir handle
        // from an absolute path. Once obtained, every subsequent op goes through this
        // handle and cannot escape it.
        let authority = cap_std::ambient_authority();
        let dir = cap_std::fs::Dir::open_ambient_dir(&root, authority)?;
        Ok(Self {
            dir,
            root,
            _tempdir: tempdir,
        })
    }

    pub(crate) fn root_path(&self) -> &std::path::Path {
        &self.root
    }

    pub(crate) fn read_file(&self, relative_path: &str) -> Result<String, String> {
        use std::io::Read;
        let mut f = self
            .dir
            .open(relative_path)
            .map_err(|e| format!("open {relative_path}: {e}"))?;
        let meta = f
            .metadata()
            .map_err(|e| format!("metadata {relative_path}: {e}"))?;
        if meta.len() as usize > SANDBOX_FILE_MAX_BYTES {
            return Err(format!(
                "file too large: {} bytes (max {})",
                meta.len(),
                SANDBOX_FILE_MAX_BYTES
            ));
        }
        let mut buf = String::new();
        f.read_to_string(&mut buf)
            .map_err(|e| format!("read {relative_path}: {e}"))?;
        Ok(buf)
    }

    pub(crate) fn write_file(&self, relative_path: &str, content: &str) -> Result<(), String> {
        use std::io::Write;
        if content.len() > SANDBOX_FILE_MAX_BYTES {
            return Err(format!(
                "content too large: {} bytes (max {})",
                content.len(),
                SANDBOX_FILE_MAX_BYTES
            ));
        }
        let mut f = self
            .dir
            .create(relative_path)
            .map_err(|e| format!("create {relative_path}: {e}"))?;
        f.write_all(content.as_bytes())
            .map_err(|e| format!("write {relative_path}: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_round_trip() {
        let sb = SandboxedDir::new().unwrap();
        sb.write_file("hello.txt", "world").unwrap();
        assert_eq!(sb.read_file("hello.txt").unwrap(), "world");
    }

    #[test]
    fn sandbox_rejects_escape() {
        let sb = SandboxedDir::new().unwrap();
        // cap-std rejects path traversal at the filesystem-handle layer.
        let bad = sb.read_file("../../../etc/passwd");
        assert!(bad.is_err(), "path traversal must be rejected");
    }

    #[test]
    fn sandbox_rejects_absolute() {
        let sb = SandboxedDir::new().unwrap();
        let bad = sb.read_file("/etc/passwd");
        assert!(bad.is_err(), "absolute paths must be rejected");
    }

    #[test]
    fn output_buffer_drains() {
        let buf = OutputBuffer::new();
        buf.write_str("hello ");
        buf.write_str("world");
        assert_eq!(buf.drain_to_string(), "hello world");
        // After draining, the buffer is empty.
        assert_eq!(buf.drain_to_string(), "");
    }

    #[test]
    fn fetch_request_default_method_is_get() {
        let req: FetchRequest = serde_json::from_str("{}").unwrap();
        assert!(req.method.is_none());
    }
}
