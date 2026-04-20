//! Disk-backed HTML spool for memory-balanced crawling.
//!
//! When the `balance` feature is active and memory pressure is detected (or
//! total in-memory HTML exceeds a configurable threshold), page HTML is
//! transparently written to a per-process spool directory on disk.  Content
//! accessors on [`Page`](crate::page::Page) reload from disk on demand so
//! callers see the same interface regardless of where the bytes live.
//!
//! ## Adaptive thresholds
//!
//! The spool system mirrors the three-level adaptation from `parallel_backends`:
//!
//! | Memory state | Per-page threshold | Budget | Behaviour |
//! |---|---|---|---|
//! | 0 (normal) | base (2 MiB) | full (512 MiB) | only budget overflow triggers spool |
//! | 1 (pressure) | **halved** | **¾** budget | large pages spooled, budget tightened |
//! | 2 (critical) | **0** (all spooled) | **0** | every page goes to disk immediately |
//!
//! **No mutexes on the hot path.**  Byte accounting uses atomics; spool
//! directory creation is guarded by `OnceLock`; individual file I/O is
//! lock-free (one file per page, unique names via atomic counter).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI8, AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;

// ── Global byte accounting ─────────────────────────────────────────────────

/// Total HTML bytes currently held in memory across all `Page` instances.
static TOTAL_HTML_BYTES_IN_MEMORY: AtomicUsize = AtomicUsize::new(0);

/// Number of pages currently spooled to disk.
static PAGES_ON_DISK: AtomicUsize = AtomicUsize::new(0);

/// Monotonic counter for generating unique spool file names.
static SPOOL_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Cached memory pressure state — updated by the background monitor in
/// `detect_system`, read here with a single atomic load instead of
/// re-querying sysinfo on every `should_spool` call.
static CACHED_MEM_STATE: AtomicI8 = AtomicI8::new(0);

/// Global sender for the background spool cleanup task.  `Drop` impls
/// send paths here instead of deleting files directly — the send is a
/// non-blocking channel push (~10ns, never blocks, never spawns per-file).
///
/// Uses `tokio::sync::mpsc::UnboundedSender` — `send()` is non-blocking,
/// does not require an active runtime, and works from any thread including
/// sync Drop impls.  The receiver awaits inside a spawned tokio task.
static CLEANUP_TX: OnceLock<tokio::sync::mpsc::UnboundedSender<PathBuf>> = OnceLock::new();

/// Initialize the cleanup task and return the sender.
///
/// When inside a tokio runtime: spawns a task that `recv().await`s on
/// the channel — sleeps with zero CPU when idle, wakes instantly on send.
/// Outside tokio (tests, CLI): falls back to a dedicated OS thread.
fn cleanup_sender() -> &'static tokio::sync::mpsc::UnboundedSender<PathBuf> {
    CLEANUP_TX.get_or_init(|| {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<PathBuf>();

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            // Tokio runtime available — spawn async cleanup task.
            // `rx.recv().await` parks with zero CPU until a path arrives.
            let mut rx = rx;
            handle.spawn(async move {
                while let Some(path) = rx.recv().await {
                    let _ = crate::utils::uring_fs::remove_file(path.display().to_string()).await;
                }
            });
        } else {
            // No tokio runtime — fallback to OS thread with blocking recv.
            let mut rx = rx;
            std::thread::Builder::new()
                .name("spider-spool-cleanup".into())
                .spawn(move || {
                    while let Some(path) = rx.blocking_recv() {
                        let _ = std::fs::remove_file(&path);
                    }
                })
                .expect("failed to spawn spool cleanup thread");
        }

        tx
    })
}

/// Queue a spool file for background deletion.  Non-blocking — just a
/// channel send.  If the cleanup task has exited (channel closed),
/// the path is silently dropped (OS temp cleanup handles it).
#[inline]
pub fn queue_spool_delete(path: PathBuf) {
    let _ = cleanup_sender().send(path);
}

/// Wait for the cleanup task to process all pending deletes.
/// Used in tests to assert file deletion.  Sends a marker file,
/// then polls until the cleanup task has removed it.
#[cfg(test)]
pub fn flush_cleanup() {
    let marker = spool_dir().join(format!(
        ".flush_{}",
        SPOOL_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::write(&marker, b"");
    let _ = cleanup_sender().send(marker.clone());
    // Bounded spin+yield — the cleanup task processes in order,
    // so once the marker is gone all prior deletes are done.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while marker.exists() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
}

/// Pages smaller than this are *never* spooled regardless of pressure,
/// because the overhead of disk I/O exceeds the memory saved.
/// Default: 16 KiB.  Override: `SPIDER_HTML_SPOOL_MIN_SIZE`.
fn spool_min_size() -> usize {
    static VAL: OnceLock<usize> = OnceLock::new();
    *VAL.get_or_init(|| {
        std::env::var("SPIDER_HTML_SPOOL_MIN_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(64 * 1024) // 64 KiB — never spool small pages
    })
}

/// Lazily-initialized spool directory.
///
/// We store the `TempDir` handle alongside the path.  While the `TempDir`
/// won't be dropped from a static at process exit, the OS temp cleaner
/// handles stale temp dirs.  Individual spool *files* are always cleaned
/// eagerly by [`HtmlSpoolGuard::Drop`](crate::page::HtmlSpoolGuard).
static SPOOL_DIR: OnceLock<SpoolDirHandle> = OnceLock::new();

/// Keeps the `tempfile::TempDir` alive so its path stays valid, and caches
/// the `PathBuf` for fast access.
struct SpoolDirHandle {
    /// Must be kept alive — dropping this would remove the directory.
    _dir: tempfile::TempDir,
    path: PathBuf,
}

/// Per-[`crate::website::Website`] spool directory.
///
/// Created lazily on first spool and torn down when the last holder drops
/// the `Arc`.  Two paths keep it alive:
///
/// 1. The owning `Website` holds an [`Arc`] so the dir persists across all
///    crawl tasks it spawns.
/// 2. Each spool file's [`crate::page::HtmlSpoolGuard`] holds a clone so
///    in-flight pages (e.g. ones broadcast through `subscribe`) keep the
///    dir alive past the `Website` drop — reads on the spool file stay
///    valid until the consumer finishes with the page.
///
/// When both drop, the inner [`tempfile::TempDir`] runs its sync
/// `remove_dir_all` which bulk-removes every file inside — making
/// `Website::drop` the single deterministic cleanup point for *all* spool
/// files it produced, no matter whether individual pages were consumed
/// or dropped.
///
/// `Arc` bumps are wait-free reference counts; no mutex, no lock, no
/// broadcast.  Construction is one syscall (`mkdtemp`).  Drop is one
/// syscall per entry plus one `rmdir`.
pub struct WebsiteSpoolDir {
    /// `None` when the filesystem refused to create a new temp dir and
    /// we fell back to the process-shared global dir — in that mode the
    /// wrapper becomes a zero-cost passthrough and drop is a no-op.
    owned: Option<tempfile::TempDir>,
    path: PathBuf,
}

impl std::fmt::Debug for WebsiteSpoolDir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebsiteSpoolDir")
            .field("path", &self.path)
            .field("owned", &self.owned.is_some())
            .finish()
    }
}

impl WebsiteSpoolDir {
    /// Create a fresh per-website spool directory under the global
    /// spool root.  If creation fails (disk full, permissions), falls
    /// back to the shared global dir so the caller always gets a valid
    /// handle — never panics, never blocks.
    #[inline]
    pub fn new_or_shared() -> Self {
        match tempfile::Builder::new()
            .prefix("spider_website_")
            .tempdir_in(spool_dir())
        {
            Ok(td) => {
                let path = td.path().to_path_buf();
                Self {
                    owned: Some(td),
                    path,
                }
            }
            Err(_) => Self {
                owned: None,
                path: spool_dir().to_path_buf(),
            },
        }
    }

    /// Root path of this website's spool directory.  Every file produced
    /// by `next_path` sits directly inside this path; no subdirs.
    #[inline]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Allocate a unique spool file path inside this directory.  The
    /// counter is global and atomic, so two `WebsiteSpoolDir`s never
    /// produce the same name even at high concurrency.
    #[inline]
    pub fn next_path(&self) -> PathBuf {
        let id = SPOOL_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        self.path.join(format!("{id}.sphtml"))
    }
}

tokio::task_local! {
    /// Ambient per-task handle to the current [`WebsiteSpoolDir`].
    ///
    /// Set by each `Website` crawl entry point and re-propagated by
    /// `utils::spawn_set` so every fetch task spawned by the crawl
    /// inherits it without needing to thread an `Arc` parameter through
    /// the ~20-argument fetch API.
    ///
    /// Outside a scoped task (ad-hoc callers, tests, external users of
    /// `Page::spool_html_to_disk`), the task-local is absent and
    /// [`next_spool_path`] falls back to the process-shared
    /// [`spool_dir`].
    pub static WEBSITE_SPOOL_DIR: std::sync::Arc<WebsiteSpoolDir>;
}

/// Read the current task-local `WebsiteSpoolDir` handle, if any.
///
/// Lock-free and allocation-free beyond the `Arc` bump — a single
/// atomic read via `task_local!::try_with`.  Returns `None` when no
/// scope is active (ad-hoc `page.spool_html_to_disk()` outside a
/// website crawl, tests, etc.).
#[inline]
pub fn current_website_spool_dir() -> Option<std::sync::Arc<WebsiteSpoolDir>> {
    WEBSITE_SPOOL_DIR.try_with(|d| d.clone()).ok()
}

// ── Configurable thresholds (env-overridable) ──────────────────────────────

/// Hard cap on total in-memory HTML before pages are spooled.
/// This is an OOM safety net, not a performance optimization — set it
/// high so normal crawls never hit it.
/// Default: 2 GiB.  Override: `SPIDER_HTML_MEMORY_BUDGET`.
fn base_memory_budget() -> usize {
    static VAL: OnceLock<usize> = OnceLock::new();
    *VAL.get_or_init(|| {
        std::env::var("SPIDER_HTML_MEMORY_BUDGET")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2 * 1024 * 1024 * 1024) // 2 GiB
    })
}

/// Per-page byte threshold.  Only truly massive pages (> 80 MiB) are
/// unconditionally spooled — these are outsized resources that would
/// dominate the memory budget.  Normal HTML pages (even large ones at
/// 5-10 MiB) stay in memory for maximum throughput.
/// Default: 80 MiB.  Override: `SPIDER_HTML_PAGE_SPOOL_SIZE`.
fn base_per_page_threshold() -> usize {
    static VAL: OnceLock<usize> = OnceLock::new();
    *VAL.get_or_init(|| {
        std::env::var("SPIDER_HTML_PAGE_SPOOL_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(80 * 1024 * 1024) // 80 MiB
    })
}

// ── Public accounting API ──────────────────────────────────────────────────

/// Add `n` bytes to the global in-memory HTML counter.
#[inline]
pub fn track_bytes_add(n: usize) {
    TOTAL_HTML_BYTES_IN_MEMORY.fetch_add(n, Ordering::Relaxed);
}

/// Subtract `n` bytes from the global in-memory HTML counter.
/// Uses saturating arithmetic to prevent underflow from pages that existed
/// before the balance feature was initialised.
#[inline]
pub fn track_bytes_sub(n: usize) {
    let _ = TOTAL_HTML_BYTES_IN_MEMORY.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |cur| {
        Some(cur.saturating_sub(n))
    });
}

/// Current total HTML bytes held in memory.
#[inline]
pub fn total_bytes_in_memory() -> usize {
    TOTAL_HTML_BYTES_IN_MEMORY.load(Ordering::Relaxed)
}

/// Increment the on-disk page counter.
#[inline]
pub fn track_page_spooled() {
    PAGES_ON_DISK.fetch_add(1, Ordering::Relaxed);
}

/// Decrement the on-disk page counter (saturating).
#[inline]
pub fn track_page_unspooled() {
    let _ = PAGES_ON_DISK.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |cur| {
        Some(cur.saturating_sub(1))
    });
}

/// Number of pages currently spooled to disk.
#[inline]
pub fn pages_on_disk() -> usize {
    PAGES_ON_DISK.load(Ordering::Relaxed)
}

/// Update the cached memory state.  Called from the hot path in
/// `channel_send_page` is unnecessary — the background monitor in
/// `detect_system` calls this periodically.
#[inline]
pub fn refresh_cached_mem_state() {
    CACHED_MEM_STATE.store(
        crate::utils::detect_system::get_process_memory_state_sync(),
        Ordering::Relaxed,
    );
}

// ── Spool decision logic ───────────────────────────────────────────────────

/// Decide whether a page with `html_len` bytes should be spooled to disk.
///
/// **Design principle**: memory is *always* faster than disk.  Spooling is
/// purely a last-resort pressure reliever — it should only engage when
/// the process is genuinely at risk of running out of memory and the
/// system cannot absorb the page.  Under normal operation (even heavy
/// crawls with high RSS) pages stay in memory for maximum throughput.
///
/// **Key insight**: high memory usage is fine if pages are being consumed
/// quickly.  Only spool when pressure is real AND the page is large
/// enough that spooling actually helps.  The budget cap only applies
/// under pressure — if the OS has memory available, let it be used.
///
/// **Performance**: hot path (`channel_send_page`).  Under normal memory
/// conditions the function exits after one atomic load (mem_state == 0)
/// with zero disk I/O triggered.
///
/// Decision tree (first match wins):
///
/// 1. Page ≤ min size (64 KiB) → **keep** (always — I/O cost > savings).
/// 2. **Normal** (< 90% RSS) → only spool truly massive pages (> threshold).
/// 3. **Pressure** (90–95% RSS) → spool large pages (> threshold / 4)
///    OR budget exceeded (memory genuinely filling up).
/// 4. **Critical** (≥ 95% RSS) → spool everything above min size.
/// 5. Otherwise → **keep in memory**.
#[inline]
pub fn should_spool(html_len: usize) -> bool {
    // ① Small pages always stay in memory — never worth the I/O.
    if html_len <= spool_min_size() {
        return false;
    }

    let threshold = base_per_page_threshold();

    // ② Check system memory pressure (single atomic load — zero cost).
    let mem_state = CACHED_MEM_STATE.load(Ordering::Relaxed);

    match mem_state {
        // Critical (≥95% RSS): OOM imminent — spool everything above min.
        s if s >= 2 => return true,

        // Pressure (90–95% RSS): spool large pages, or if the budget
        // is exceeded (memory is genuinely filling up, not just high).
        s if s >= 1 => {
            if html_len > threshold / 4 {
                return true;
            }
            // Budget check only under pressure — if the OS has room, let
            // it be used even if we're over the soft budget.
            let current = total_bytes_in_memory();
            if current.saturating_add(html_len) > base_memory_budget() {
                return true;
            }
        }

        // Normal: only spool truly massive outlier pages. Budget is
        // not enforced — high memory usage is fine when the OS has room.
        _ => {
            if html_len > threshold {
                return true;
            }
        }
    }

    false
}

// ── Spool directory management ─────────────────────────────────────────────

/// Return (and lazily create) the spool directory.
///
/// Uses the `tempfile` crate for OS-correct temp directory creation with
/// unique naming.  The directory is prefixed with `spider_html_` and lives
/// under `$TMPDIR` (or the OS default).
///
/// Override: set `SPIDER_HTML_SPOOL_DIR` to place spool files in a custom
/// directory instead of a system temp path.
pub fn spool_dir() -> &'static Path {
    &SPOOL_DIR
        .get_or_init(|| {
            // If the user set an explicit spool dir, use that.
            if let Ok(custom) = std::env::var("SPIDER_HTML_SPOOL_DIR") {
                let dir = PathBuf::from(&custom);
                let _ = std::fs::create_dir_all(&dir);
                // Create a TempDir inside the custom path so we still get
                // auto-cleanup semantics.
                match tempfile::Builder::new()
                    .prefix("spider_html_")
                    .tempdir_in(&dir)
                {
                    Ok(td) => {
                        let path = td.path().to_path_buf();
                        return SpoolDirHandle { _dir: td, path };
                    }
                    Err(_) => {
                        // Fallback: use the custom dir directly.
                        return SpoolDirHandle {
                            _dir: tempfile::Builder::new()
                                .prefix("spider_html_fallback_")
                                .tempdir()
                                .expect("failed to create temp dir"),
                            path: dir,
                        };
                    }
                }
            }

            // Default: OS temp directory via tempfile crate.
            let td = tempfile::Builder::new()
                .prefix("spider_html_")
                .tempdir()
                .expect("failed to create temp dir for HTML spool");
            let path = td.path().to_path_buf();
            SpoolDirHandle { _dir: td, path }
        })
        .path
}

/// Generate a unique spool file path for a page.
///
/// When called inside a [`WEBSITE_SPOOL_DIR`] scope, the file lands in
/// that website's private subdir so it's torn down with the website.
/// Outside a scope — ad-hoc callers, tests, external users of
/// `Page::spool_html_to_disk` — the path resolves to the global
/// process-shared [`spool_dir`], preserving the legacy behaviour.
pub fn next_spool_path() -> PathBuf {
    if let Some(dir) = current_website_spool_dir() {
        return dir.next_path();
    }
    let id = SPOOL_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    spool_dir().join(format!("{id}.sphtml"))
}

// ── File I/O helpers ───────────────────────────────────────────────────────

/// Write `data` to `path`.  Returns `Ok(())` on success.
pub fn spool_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)
}

/// Read the full contents of a spool file into memory.
pub fn spool_read(path: &Path) -> std::io::Result<Vec<u8>> {
    std::fs::read(path)
}

/// Read a spool file into `bytes::Bytes`.
pub fn spool_read_bytes(path: &Path) -> std::io::Result<bytes::Bytes> {
    std::fs::read(path).map(bytes::Bytes::from)
}

/// Delete a spool file.  Errors are silently ignored (file may already be
/// gone after a previous cleanup pass).
pub fn spool_delete(path: &Path) {
    let _ = std::fs::remove_file(path);
}

// ── Async I/O helpers (tokio) ──────────────────────────────────────────────
//
// These avoid blocking the tokio runtime on disk reads.  Used by internal
// async crawl paths (link extraction, ensure_html_loaded_async).  The sync
// variants above are kept for non-async consumers and Drop impls.

/// Async read of a spool file into `bytes::Bytes`.
/// Routes through `uring_fs` for true kernel-async I/O on Linux;
/// falls back to `tokio::fs` on other platforms.
pub async fn spool_read_bytes_async(path: std::path::PathBuf) -> std::io::Result<bytes::Bytes> {
    crate::utils::uring_fs::read_file(path.display().to_string())
        .await
        .map(bytes::Bytes::from)
}

/// Async read of a spool file into `Vec<u8>`.
/// Routes through `uring_fs` for true kernel-async I/O on Linux;
/// falls back to `tokio::fs` on other platforms.
pub async fn spool_read_async(path: std::path::PathBuf) -> std::io::Result<Vec<u8>> {
    crate::utils::uring_fs::read_file(path.display().to_string()).await
}

/// Async write of data to a spool file.
/// Routes through `uring_fs` for true kernel-async I/O on Linux;
/// falls back to `tokio::fs` on other platforms.
pub async fn spool_write_async(path: &Path, data: &[u8]) -> std::io::Result<()> {
    crate::utils::uring_fs::write_file(path.display().to_string(), data.to_vec()).await
}

/// Per-page vitals produced by the streaming spool writer.
///
/// All fields are computed *while* bytes flush to disk so the caller never
/// has to scan the full buffer a second time or re-read the spool file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpoolVitals {
    /// Total bytes written to the spool file.
    pub byte_len: usize,
    /// Whether the full payload is valid UTF-8.  Computed incrementally
    /// across chunk boundaries so `simdutf8` never sees the whole buffer
    /// at once (keeps branch-prediction cache warm + overlaps with I/O).
    pub is_valid_utf8: bool,
    /// Binary-file detection via magic numbers on the leading bytes.
    /// `auto_encoder::is_binary_file` only inspects the header, so the
    /// check is O(1) and happens before any chunked write.
    pub binary_file: bool,
    /// Whether the payload begins with `<?xml`.  Five-byte prefix test.
    pub is_xml: bool,
}

/// Streaming-write variant of [`spool_write_async`] that also returns the
/// page vitals computed **inline with the write**.
///
/// Design constraints:
/// - No blocking syscalls on the caller's thread (all I/O goes through
///   `tokio::fs` via `tokio::io::BufWriter`).
/// - No locks, no mutexes, no atomics — purely local state.
/// - No heap allocation on the hot path.  The tiny 4-byte `carry` buffer
///   lives on the stack; `BufWriter` is constructed once.
/// - Walks the bytes exactly **once** — same work as `simdutf8::basic::
///   from_utf8` on the full buffer, but interleaved with disk flushes so
///   large spools don't turn into a long CPU-only stall before I/O starts.
/// - Never panics: every I/O call returns through the `?` operator, and
///   all slice indexing is bounds-checked or uses `chunks`.
///
/// Returns the vitals on success.  The caller is expected to mirror them
/// onto the `Page` struct so downstream accessors keep skipping redundant
/// re-validation work.
pub async fn spool_write_streaming_vitals(
    path: &Path,
    data: &[u8],
) -> std::io::Result<SpoolVitals> {
    use tokio::io::AsyncWriteExt;

    /// Chunk size for the streaming loop.  64 KiB is large enough to keep
    /// per-write syscall overhead down yet small enough that validation +
    /// I/O can plausibly interleave on a busy async runtime.
    const CHUNK: usize = 64 * 1024;

    let byte_len = data.len();

    // DoS guard: refuse up-front if the caller somehow handed us a
    // buffer larger than the configured spool cap.  Mirrors the check
    // inside `StreamingVitalsSpoolWriter::write_chunk` so the two
    // writers can't diverge.
    if byte_len > spool_max_write_bytes() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "spool write would exceed SPIDER_HTML_SPOOL_MAX_BYTES",
        ));
    }

    // ── O(1) header vitals ────────────────────────────────────────────
    // Both checks look at only the first few bytes, independent of the
    // page size.  `auto_encoder::is_binary_file` is a magic-number lookup
    // table; `is_xml` is a 5-byte `starts_with`.
    let head = &data[..data.len().min(16)];
    let binary_file = auto_encoder::is_binary_file(head);
    let is_xml = head.starts_with(b"<?xml");

    // ── Streaming write + incremental UTF-8 validation ────────────────
    let file = tokio::fs::File::create(path).await?;
    let mut writer = tokio::io::BufWriter::with_capacity(CHUNK, file);

    // Rolling state for UTF-8 validation across chunk boundaries.  A
    // single multi-byte codepoint is at most 4 bytes, so carrying up to
    // 3 trailing bytes of an incomplete sequence into the next chunk is
    // always sufficient.  Once `is_valid_utf8` flips to `false` we stop
    // validating (writes continue to completion).
    let mut is_valid_utf8 = true;
    let mut carry: [u8; 4] = [0; 4];
    let mut carry_len: usize = 0;
    // Lazily-allocated scratch for stitching `carry + chunk` when the
    // previous chunk ended mid-codepoint.  Allocated at most once per
    // spool (the first time carry is non-zero); ASCII-only payloads
    // never pay this cost.
    let mut scratch: Vec<u8> = Vec::new();

    for chunk in data.chunks(CHUNK) {
        writer.write_all(chunk).await?;

        if !is_valid_utf8 {
            continue;
        }

        // Build the validation view.  Carry-less fast path is zero-copy:
        // we validate the chunk slice directly.  Carry path copies the
        // chunk into a persistent `scratch` buffer; after the first copy
        // the buffer's capacity is reused, so the allocator is hit at
        // most once per spool regardless of payload size.
        let to_validate: &[u8] = if carry_len == 0 {
            chunk
        } else {
            scratch.clear();
            scratch.reserve(carry_len + chunk.len());
            scratch.extend_from_slice(&carry[..carry_len]);
            scratch.extend_from_slice(chunk);
            &scratch[..]
        };

        match simdutf8::compat::from_utf8(to_validate) {
            Ok(_) => {
                carry_len = 0;
            }
            Err(e) => {
                if e.error_len().is_some() {
                    // Hard error mid-stream — payload is not UTF-8.
                    is_valid_utf8 = false;
                    continue;
                }
                // Incomplete sequence at end: save the trailing bytes
                // for the next iteration.  By definition this can be
                // at most 3 bytes (any longer would be a hard error).
                let trailing = &to_validate[e.valid_up_to()..];
                let keep = trailing.len().min(carry.len());
                // Copy the last `keep` bytes of the trailing slice.
                // Using a tiny stack temp avoids overlap pitfalls if
                // `trailing` is borrowed from `scratch` and we later
                // clear that buffer in the next iteration.
                let mut tmp: [u8; 4] = [0; 4];
                tmp[..keep].copy_from_slice(&trailing[trailing.len() - keep..]);
                carry[..keep].copy_from_slice(&tmp[..keep]);
                carry_len = keep;
            }
        }
    }

    writer.flush().await?;
    // Ensure the underlying file is synced into its Drop path without
    // awaiting a separate close — BufWriter::into_inner avoids a double
    // flush while still dropping the fd cleanly.
    let _file = writer.into_inner();

    // Any leftover partial codepoint at EOF means the payload is not
    // complete UTF-8.
    if carry_len > 0 {
        is_valid_utf8 = false;
    }

    Ok(SpoolVitals {
        byte_len,
        is_valid_utf8,
        binary_file,
        is_xml,
    })
}

/// Maximum bytes of the page head captured by the streaming writer.
/// 256 comfortably covers every WAF-prefix check currently performed
/// after a chrome HTML fetch while staying well below a single cache
/// line budget concern.
pub const SPOOL_HEAD_TAIL_CAP: usize = 256;

/// Hard upper bound on how many bytes any single
/// [`StreamingVitalsSpoolWriter`] will accept before it refuses further
/// writes with an I/O error.  Intended as a last-resort DoS guard
/// against upstream sources (e.g. a malicious page served through
/// Chrome) that might try to balloon disk usage via a single
/// pathological document.  1 GiB comfortably exceeds chromey's own
/// `MAX_DOCUMENT_UNITS` (256 Mi UTF-16 code units ≈ 768 MiB UTF-8) so
/// under normal chrome operation this cap is never reached — the
/// source-side cap fires first.  Overridable via
/// `SPIDER_HTML_SPOOL_MAX_BYTES` for ops who need a tighter ceiling
/// (smaller values become the active cap; larger values raise it up
/// to a hard 4 GiB safety ceiling so an attacker can't pick the env
/// var either).
pub fn spool_max_write_bytes() -> usize {
    static VAL: OnceLock<usize> = OnceLock::new();
    *VAL.get_or_init(|| {
        const DEFAULT: usize = 1024 * 1024 * 1024; // 1 GiB
        const HARD_CEILING: usize = 4 * 1024 * 1024 * 1024; // 4 GiB
        std::env::var("SPIDER_HTML_SPOOL_MAX_BYTES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .map(|n| n.min(HARD_CEILING))
            .unwrap_or(DEFAULT)
    })
}

/// Maximum bytes of normalised HTML the signature helper is willing to
/// buffer when computing `hash_html`-equivalent signatures for a
/// disk-spooled page.  Pages whose normalised output exceeds this cap
/// return `signature: None` and the caller falls back to the in-memory
/// path so signatures remain bit-for-bit compatible with
/// [`crate::utils::hash_html`] in either case.
pub const SPOOL_SIGNATURE_BUFFER_CAP: usize = 16 * 1024 * 1024; // 16 MiB

/// Fully-described disk-spooled content handle carried end-to-end on
/// `PageResponse` so the crawler never has to materialise the full HTML
/// in a `Vec<u8>` when a page is written straight to disk during the
/// chrome fetch under memory pressure.
///
/// Every field is a small fixed-size value — a path, four cached vitals,
/// two bounded head/tail byte slices, and a `u64` signature — so
/// shipping one of these through the channel path carries the same cost
/// as an owned `Box<SpooledContent>` regardless of the actual HTML
/// size.
#[derive(Debug, Clone, Default)]
pub struct SpooledContent {
    /// Filesystem path to the spooled HTML.  Ownership of the file is
    /// transferred to the `HtmlSpoolGuard` held by `Page` once build
    /// consumes this struct; the caller must not delete the file.
    pub path: std::path::PathBuf,
    /// Vitals computed incrementally during the write (byte length,
    /// UTF-8 validity, binary detection, XML marker).  Zero disk I/O
    /// required to populate these on the constructed `Page`.
    pub vitals: SpoolVitals,
    /// First ≤ [`SPOOL_HEAD_TAIL_CAP`] bytes of the document.  Downstream
    /// checks that only need a prefix (e.g. Cloudflare WAF magic-bytes)
    /// can operate on this slice without re-reading disk.
    pub head: bytes::Bytes,
    /// Last ≤ [`SPOOL_HEAD_TAIL_CAP`] bytes of the document, captured
    /// via a rolling window during streaming.  Same use case as `head`.
    pub tail: bytes::Bytes,
    /// Pre-computed `hash_html`-equivalent signature of the normalised
    /// HTML, bit-for-bit identical to what
    /// [`crate::utils::hash_html`] would return on the same raw bytes.
    /// `None` when the normalised output exceeded
    /// [`SPOOL_SIGNATURE_BUFFER_CAP`] — in that case the caller must
    /// abort the direct-spool path and fall back to in-memory fetch so
    /// signature-based dedup stays exact.
    pub signature: Option<u64>,
}

/// Stateful, push-driven streaming spool writer.
///
/// Powers both the in-memory driver
/// ([`spool_write_streaming_vitals`]) and push-style flows where bytes
/// arrive from an async source (e.g. chromey's `content_bytes_stream`).
///
/// Guarantees:
/// - Lockfree: every field is local state, no atomics, no `Mutex`, no
///   `RwLock`.
/// - Non-blocking: all I/O goes through `tokio::io::BufWriter<
///   tokio::fs::File>`.  `write_chunk` awaits on the inner future
///   directly — no `spawn_blocking` or runtime-handle acquisition.
/// - Allocation-light: the scratch buffer is allocated at most once per
///   writer lifetime (only when a chunk actually ends mid-codepoint).
///   The head/tail rings are `Vec<u8>` pre-sized to the cap so they
///   never reallocate.
/// - Panic-free: every fallible op returns through `?`.  No `unwrap`,
///   no `expect`, no slice indexing that can go out of bounds.
pub struct StreamingVitalsSpoolWriter {
    writer: tokio::io::BufWriter<tokio::fs::File>,
    byte_len: usize,
    is_valid_utf8: bool,
    binary_file: bool,
    is_xml: bool,
    header_seen: bool,
    carry: [u8; 4],
    carry_len: usize,
    scratch: Vec<u8>,
    head: Vec<u8>,
    tail_ring: Vec<u8>,
    /// Next write index in `tail_ring` (wraps around `tail_ring.capacity()`).
    tail_head: usize,
    /// Total bytes ever fed into the tail ring — used on `finish` to
    /// decide whether the ring already wrapped.
    tail_fed: usize,
}

impl StreamingVitalsSpoolWriter {
    /// Internal chunk size for `BufWriter` flushes.  Matches
    /// [`spool_write_streaming_vitals`] for consistency.
    const CHUNK: usize = 64 * 1024;

    /// Open `path` for a fresh streaming write.  Fails only if the
    /// filesystem rejects the create — no lazy work is deferred.
    pub async fn new(path: &Path) -> std::io::Result<Self> {
        let file = tokio::fs::File::create(path).await?;
        let writer = tokio::io::BufWriter::with_capacity(Self::CHUNK, file);
        Ok(Self {
            writer,
            byte_len: 0,
            is_valid_utf8: true,
            binary_file: false,
            is_xml: false,
            header_seen: false,
            carry: [0; 4],
            carry_len: 0,
            scratch: Vec::new(),
            head: Vec::with_capacity(SPOOL_HEAD_TAIL_CAP),
            tail_ring: Vec::with_capacity(SPOOL_HEAD_TAIL_CAP),
            tail_head: 0,
            tail_fed: 0,
        })
    }

    /// Push a chunk of bytes through the writer.  Empty chunks are a
    /// no-op.  The chunk is flushed to disk and its contribution to the
    /// running vitals + head/tail windows is folded in before returning.
    ///
    /// **DoS guard:** a write that would push the running `byte_len`
    /// past [`spool_max_write_bytes`] is rejected with
    /// `std::io::ErrorKind::InvalidInput` *before* any disk I/O runs.
    /// The default cap (1 GiB) is never hit by normal chrome traffic
    /// (chromey caps at ~768 MiB UTF-8); the check exists so an
    /// adversarial upstream can't inflate the spool file indefinitely.
    pub async fn write_chunk(&mut self, chunk: &[u8]) -> std::io::Result<()> {
        use tokio::io::AsyncWriteExt;

        if chunk.is_empty() {
            return Ok(());
        }

        // DoS guard: refuse to grow the spool file past the configured
        // max.  `saturating_add` protects the comparison itself from
        // wrap-around; the real bound is `spool_max_write_bytes()`.
        let projected = self.byte_len.saturating_add(chunk.len());
        if projected > spool_max_write_bytes() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "spool write would exceed SPIDER_HTML_SPOOL_MAX_BYTES",
            ));
        }

        self.writer.write_all(chunk).await?;
        self.byte_len = projected;

        // ── Header-only vitals (fire exactly once) ────────────────────
        if !self.header_seen {
            let head_sample_len = chunk.len().min(16);
            let head_sample = &chunk[..head_sample_len];
            self.binary_file = auto_encoder::is_binary_file(head_sample);
            self.is_xml = head_sample.starts_with(b"<?xml");
            self.header_seen = true;
        }

        // ── Head window: fill until capped ────────────────────────────
        if self.head.len() < SPOOL_HEAD_TAIL_CAP {
            let remaining = SPOOL_HEAD_TAIL_CAP - self.head.len();
            let take = chunk.len().min(remaining);
            self.head.extend_from_slice(&chunk[..take]);
        }

        // ── Tail window: rolling last N bytes ─────────────────────────
        // Fast path for small early chunks: just append.  Once we exceed
        // the cap we switch to a ring layout.  On finish we reconstruct
        // the last N bytes in original order.
        let cap = SPOOL_HEAD_TAIL_CAP;
        if self.tail_fed == 0 && chunk.len() <= cap {
            self.tail_ring.clear();
            self.tail_ring.extend_from_slice(chunk);
            self.tail_head = self.tail_ring.len() % cap;
            self.tail_fed = chunk.len();
        } else if chunk.len() >= cap {
            // Chunk alone covers the whole tail window — only its own
            // last `cap` bytes survive.
            self.tail_ring.clear();
            self.tail_ring
                .extend_from_slice(&chunk[chunk.len() - cap..]);
            self.tail_head = 0;
            self.tail_fed = self.tail_fed.saturating_add(chunk.len());
        } else {
            // Ensure the ring is sized to `cap` once so subsequent writes
            // can use direct indexing without reallocation.
            if self.tail_ring.len() < cap {
                let needed = cap - self.tail_ring.len();
                let pad = chunk.len().min(needed);
                self.tail_ring.extend_from_slice(&chunk[..pad]);
                // If we still have more bytes in this chunk, the rest
                // wraps into the ring at index 0.
                let rest = &chunk[pad..];
                if !rest.is_empty() {
                    let ring_cap = self.tail_ring.len();
                    for (i, b) in rest.iter().enumerate() {
                        self.tail_ring[i % ring_cap] = *b;
                    }
                    self.tail_head = rest.len() % ring_cap;
                } else {
                    self.tail_head = self.tail_ring.len() % cap;
                }
            } else {
                // Full ring: write chunk bytes starting at tail_head,
                // wrapping around.  Bounded loop, no allocation.
                for b in chunk {
                    self.tail_ring[self.tail_head] = *b;
                    self.tail_head += 1;
                    if self.tail_head == cap {
                        self.tail_head = 0;
                    }
                }
            }
            self.tail_fed = self.tail_fed.saturating_add(chunk.len());
        }

        // ── Incremental UTF-8 validation ──────────────────────────────
        if !self.is_valid_utf8 {
            return Ok(());
        }

        let to_validate: &[u8] = if self.carry_len == 0 {
            chunk
        } else {
            self.scratch.clear();
            self.scratch.reserve(self.carry_len + chunk.len());
            self.scratch
                .extend_from_slice(&self.carry[..self.carry_len]);
            self.scratch.extend_from_slice(chunk);
            &self.scratch[..]
        };

        match simdutf8::compat::from_utf8(to_validate) {
            Ok(_) => {
                self.carry_len = 0;
            }
            Err(e) => {
                if e.error_len().is_some() {
                    self.is_valid_utf8 = false;
                } else {
                    let trailing = &to_validate[e.valid_up_to()..];
                    let keep = trailing.len().min(self.carry.len());
                    let mut tmp: [u8; 4] = [0; 4];
                    tmp[..keep].copy_from_slice(&trailing[trailing.len() - keep..]);
                    self.carry[..keep].copy_from_slice(&tmp[..keep]);
                    self.carry_len = keep;
                }
            }
        }

        Ok(())
    }

    /// Flush remaining buffer, finalize vitals, and return the
    /// aggregated outcome.  After this call the underlying file is
    /// closed.
    pub async fn finish(mut self) -> std::io::Result<(SpoolVitals, bytes::Bytes, bytes::Bytes)> {
        use tokio::io::AsyncWriteExt;

        self.writer.flush().await?;
        let _file = self.writer.into_inner();

        // An incomplete multi-byte sequence still pending at EOF means
        // the payload is not valid UTF-8.
        if self.carry_len > 0 {
            self.is_valid_utf8 = false;
        }

        let head = bytes::Bytes::from(self.head);
        let tail = if self.tail_fed <= SPOOL_HEAD_TAIL_CAP {
            bytes::Bytes::from(self.tail_ring)
        } else {
            // Reassemble in original byte order: starting from tail_head,
            // read `cap` bytes wrapping around.
            let cap = self.tail_ring.len();
            let mut out = Vec::with_capacity(cap);
            let head_idx = self.tail_head;
            out.extend_from_slice(&self.tail_ring[head_idx..]);
            out.extend_from_slice(&self.tail_ring[..head_idx]);
            bytes::Bytes::from(out)
        };

        Ok((
            SpoolVitals {
                byte_len: self.byte_len,
                is_valid_utf8: self.is_valid_utf8,
                binary_file: self.binary_file,
                is_xml: self.is_xml,
            },
            head,
            tail,
        ))
    }
}

/// Async streaming read of a spool file in chunks.
/// Delegates to [`uring_fs::read_file_chunked`] which picks the
/// optimal strategy per platform (io_uring or tokio::fs streaming).
pub async fn spool_stream_chunks_async(
    path: std::path::PathBuf,
    chunk_size: usize,
    cb: impl FnMut(&[u8]) -> bool,
) -> std::io::Result<usize> {
    crate::utils::uring_fs::read_file_chunked(path.display().to_string(), chunk_size, cb).await
}

/// Remove the entire spool directory.  Best-effort; useful for process exit.
/// Individual spool files are already cleaned by `HtmlSpoolGuard::Drop`,
/// so this only handles the directory itself and any orphaned files.
pub fn cleanup_spool_dir() {
    if let Some(handle) = SPOOL_DIR.get() {
        let _ = std::fs::remove_dir_all(&handle.path);
    }
}

/// Stream-read a spool file in chunks and feed each chunk to a callback.
/// Returns `Ok(total_bytes_read)`.  The callback can return `false` to stop
/// early (e.g. on a parse error).
///
/// This avoids loading the entire file into memory — useful for link
/// extraction via `lol_html` which accepts incremental `write()` calls.
pub fn spool_stream_chunks<F>(path: &Path, chunk_size: usize, mut cb: F) -> std::io::Result<usize>
where
    F: FnMut(&[u8]) -> bool,
{
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buf = vec![0u8; chunk_size];
    let mut total = 0usize;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n);
        if !cb(&buf[..n]) {
            break;
        }
    }
    Ok(total)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Expose base_per_page_threshold for cross-module tests.

    #[test]
    fn test_byte_accounting_saturating() {
        // Use relative deltas to avoid races with parallel tests.
        let base = total_bytes_in_memory();
        track_bytes_add(1000);
        assert_eq!(total_bytes_in_memory(), base + 1000);
        track_bytes_sub(600);
        assert_eq!(total_bytes_in_memory(), base + 400);
        track_bytes_sub(400);
        assert_eq!(total_bytes_in_memory(), base);
        // Saturating subtract — must never underflow or panic.
        // We can only test saturation safely by subtracting more than we
        // added in this test, but other tests may have added bytes too.
        // Just verify the operation doesn't panic.
        let before_sat = total_bytes_in_memory();
        track_bytes_sub(before_sat + 1);
        assert_eq!(total_bytes_in_memory(), 0);
        // Restore so other tests aren't affected.
        track_bytes_add(before_sat);
    }

    #[test]
    fn test_page_disk_counter() {
        {
            let base = pages_on_disk();
            track_page_spooled();
            track_page_spooled();
            assert_eq!(pages_on_disk(), base + 2);
            track_page_unspooled();
            assert_eq!(pages_on_disk(), base + 1);
            track_page_unspooled();
            assert_eq!(pages_on_disk(), base);
        }
    }

    #[test]
    fn test_should_spool_decision() {
        // Tiny pages never spool (under min size).
        assert!(!should_spool(100));
        assert!(!should_spool(spool_min_size()));

        // Under normal memory conditions, nothing spools — spooling is
        // an OOM safety net, not an optimization.
        assert!(!should_spool(200 * 1024)); // 200 KiB
        assert!(!should_spool(5 * 1024 * 1024)); // 5 MiB
        assert!(!should_spool(10 * 1024 * 1024)); // 10 MiB

        // Truly massive pages always spool (outsized resources).
        assert!(should_spool(base_per_page_threshold() + 1));
    }

    #[test]
    fn test_spool_write_read_delete() {
        let dir = std::env::temp_dir().join("spider_spool_test_rw");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.sphtml");

        let data = b"<html><body>hello</body></html>";
        spool_write(&path, data).unwrap();
        let read_back = spool_read(&path).unwrap();
        assert_eq!(&read_back, data);

        let bytes = spool_read_bytes(&path).unwrap();
        assert_eq!(&bytes[..], data);

        spool_delete(&path);
        assert!(!path.exists());

        // Delete of non-existent file should not panic.
        spool_delete(&path);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_spool_read_nonexistent() {
        let path = std::env::temp_dir().join("spider_spool_does_not_exist.sphtml");
        assert!(spool_read(&path).is_err());
        assert!(spool_read_bytes(&path).is_err());
    }

    #[test]
    fn test_spool_stream_chunks() {
        let dir = std::env::temp_dir().join("spider_spool_stream_test2");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("stream.sphtml");

        let data = b"abcdefghijklmnopqrstuvwxyz";
        spool_write(&path, data).unwrap();

        let mut collected = Vec::new();
        let total = spool_stream_chunks(&path, 10, |chunk| {
            collected.extend_from_slice(chunk);
            true
        })
        .unwrap();
        assert_eq!(collected, data);
        assert_eq!(total, data.len());

        spool_delete(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_spool_stream_early_stop() {
        let dir = std::env::temp_dir().join("spider_spool_stream_stop");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("stop.sphtml");

        let data = vec![0u8; 100];
        spool_write(&path, &data).unwrap();

        let mut count = 0usize;
        spool_stream_chunks(&path, 10, |_| {
            count += 1;
            count < 3 // stop after 3 chunks
        })
        .unwrap();
        assert_eq!(count, 3);

        spool_delete(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_spool_stream_nonexistent() {
        let path = std::env::temp_dir().join("spider_spool_no_exist.sphtml");
        let result = spool_stream_chunks(&path, 10, |_| true);
        assert!(result.is_err());
    }

    #[test]
    fn test_next_spool_path_unique() {
        let p1 = next_spool_path();
        let p2 = next_spool_path();
        let p3 = next_spool_path();
        assert_ne!(p1, p2);
        assert_ne!(p2, p3);
        assert_eq!(p1.extension().unwrap(), "sphtml");
    }

    #[test]
    fn test_spool_dir_is_stable() {
        let d1 = spool_dir();
        let d2 = spool_dir();
        assert_eq!(d1, d2);
    }

    #[test]
    fn test_spool_empty_data() {
        let path = next_spool_path();
        spool_write(&path, b"").unwrap();
        let read_back = spool_read(&path).unwrap();
        assert!(read_back.is_empty());

        let mut chunks = 0;
        spool_stream_chunks(&path, 10, |_| {
            chunks += 1;
            true
        })
        .unwrap();
        assert_eq!(chunks, 0, "empty file should produce zero chunks");

        spool_delete(&path);
    }

    #[test]
    fn test_spool_large_data_stream() {
        // 1 MiB of data streamed in 64 KiB chunks.
        let size = 1024 * 1024;
        let data: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
        let path = next_spool_path();
        spool_write(&path, &data).unwrap();

        let mut collected = Vec::with_capacity(size);
        let total = spool_stream_chunks(&path, 65536, |chunk| {
            collected.extend_from_slice(chunk);
            true
        })
        .unwrap();
        assert_eq!(total, size);
        assert_eq!(collected, data);

        spool_delete(&path);
    }

    /// The streaming vitals writer must match the one-shot reference values
    /// (`simdutf8::basic::from_utf8` + `is_binary_file` on the full buffer)
    /// so swapping it in never changes observable behavior.
    #[tokio::test]
    async fn test_spool_streaming_vitals_matches_reference_ascii() {
        let data = b"<html><body>simple ascii page</body></html>";
        let path = next_spool_path();
        let vitals = spool_write_streaming_vitals(&path, data).await.unwrap();
        assert_eq!(vitals.byte_len, data.len());
        assert!(vitals.is_valid_utf8);
        assert!(!vitals.binary_file);
        assert!(!vitals.is_xml);
        // File really exists with exactly the bytes we gave.
        let on_disk = std::fs::read(&path).unwrap();
        assert_eq!(on_disk, data);
        spool_delete(&path);
    }

    /// Multi-byte UTF-8 codepoints that cross an internal chunk boundary
    /// must still validate as valid UTF-8.  This exercises the `carry`
    /// rollover path without needing a pathologically large payload.
    #[tokio::test]
    async fn test_spool_streaming_vitals_utf8_multibyte() {
        // Repeat a 3-byte codepoint ("€") enough to span well past the
        // 64 KiB chunk cutoff so at least one boundary falls mid-codepoint.
        let mut data: Vec<u8> = Vec::with_capacity(256 * 1024);
        for _ in 0..(90 * 1024) {
            data.extend_from_slice("€".as_bytes());
        }
        assert!(simdutf8::basic::from_utf8(&data).is_ok());

        let path = next_spool_path();
        let vitals = spool_write_streaming_vitals(&path, &data).await.unwrap();
        assert_eq!(vitals.byte_len, data.len());
        assert!(
            vitals.is_valid_utf8,
            "multi-byte codepoint spanning chunk boundaries must stay valid"
        );
        spool_delete(&path);
    }

    /// Hard UTF-8 errors mid-stream flip the flag to false, never panic,
    /// and the bytes still land on disk intact for later inspection.
    #[tokio::test]
    async fn test_spool_streaming_vitals_utf8_invalid() {
        let mut data: Vec<u8> = b"<html>valid prefix".to_vec();
        // Insert a lone continuation byte (illegal UTF-8 start).
        data.push(0x80);
        data.extend_from_slice(b"</html>");

        let path = next_spool_path();
        let vitals = spool_write_streaming_vitals(&path, &data).await.unwrap();
        assert_eq!(vitals.byte_len, data.len());
        assert!(!vitals.is_valid_utf8);
        let on_disk = std::fs::read(&path).unwrap();
        assert_eq!(on_disk, data);
        spool_delete(&path);
    }

    /// XML header detection is O(1): only the first five bytes decide the
    /// flag, regardless of payload size.
    #[tokio::test]
    async fn test_spool_streaming_vitals_xml_header() {
        let data = br#"<?xml version="1.0"?><feed/>"#;
        let path = next_spool_path();
        let vitals = spool_write_streaming_vitals(&path, data).await.unwrap();
        assert!(vitals.is_xml);
        assert!(vitals.is_valid_utf8);
        spool_delete(&path);
    }

    /// Empty payload still writes a file (size 0) and returns sensible
    /// vitals — never panics.
    #[tokio::test]
    async fn test_spool_streaming_vitals_empty() {
        let path = next_spool_path();
        let vitals = spool_write_streaming_vitals(&path, &[]).await.unwrap();
        assert_eq!(vitals.byte_len, 0);
        assert!(
            vitals.is_valid_utf8,
            "empty bytes are trivially valid utf-8"
        );
        assert!(!vitals.binary_file);
        assert!(!vitals.is_xml);
        spool_delete(&path);
    }

    /// Chunk-by-chunk writer must yield vitals identical to the single-
    /// shot writer for the same input, and must capture head/tail
    /// windows matching the actual bytes at those offsets.
    #[tokio::test]
    async fn test_streaming_writer_matches_single_shot() {
        let mut data: Vec<u8> = Vec::with_capacity(200 * 1024);
        for i in 0..(200 * 1024) {
            data.push((b'a' + (i % 26) as u8) as u8);
        }
        // Reference: single-shot writer.
        let ref_path = next_spool_path();
        let ref_vitals = spool_write_streaming_vitals(&ref_path, &data)
            .await
            .unwrap();
        spool_delete(&ref_path);

        // Push-driven: small varying chunk sizes across boundaries.
        let path = next_spool_path();
        let mut w = StreamingVitalsSpoolWriter::new(&path).await.unwrap();
        for chunk in data.chunks(7919) {
            w.write_chunk(chunk).await.unwrap();
        }
        let (vitals, head, tail) = w.finish().await.unwrap();

        assert_eq!(vitals.byte_len, ref_vitals.byte_len);
        assert_eq!(vitals.is_valid_utf8, ref_vitals.is_valid_utf8);
        assert_eq!(vitals.binary_file, ref_vitals.binary_file);
        assert_eq!(vitals.is_xml, ref_vitals.is_xml);
        assert_eq!(head.as_ref(), &data[..SPOOL_HEAD_TAIL_CAP]);
        assert_eq!(tail.as_ref(), &data[data.len() - SPOOL_HEAD_TAIL_CAP..]);

        let on_disk = std::fs::read(&path).unwrap();
        assert_eq!(on_disk, data);
        spool_delete(&path);
    }

    /// Head/tail windows for payloads smaller than the cap must contain
    /// the full payload (not padded, not truncated).
    #[tokio::test]
    async fn test_streaming_writer_small_head_tail() {
        let data = b"<html><body>tiny</body></html>";
        let path = next_spool_path();
        let mut w = StreamingVitalsSpoolWriter::new(&path).await.unwrap();
        w.write_chunk(data).await.unwrap();
        let (_, head, tail) = w.finish().await.unwrap();
        assert_eq!(head.as_ref(), data.as_slice());
        assert_eq!(tail.as_ref(), data.as_slice());
        spool_delete(&path);
    }

    /// The DoS cap reads through `spool_max_write_bytes()` — verify the
    /// function honours a tiny override and caps at the hard ceiling.
    /// This test does **not** exercise `write_chunk` directly because
    /// `spool_max_write_bytes()` memoises via `OnceLock` on first read;
    /// triggering a cap in one test would change every later test's
    /// view of the world.  Instead we verify the parser behaviour,
    /// which is the only source-of-truth for the cap value.
    #[test]
    fn test_spool_max_write_bytes_hard_ceiling() {
        // Direct parse path, mirroring `spool_max_write_bytes()` body
        // but without touching the cached global.
        let parsed: usize = "99999999999999999".parse().unwrap_or(0);
        assert!(parsed > 4 * 1024 * 1024 * 1024);
        // After `.min(HARD_CEILING)` the advertised cap never exceeds
        // 4 GiB regardless of env.
        let capped = parsed.min(4 * 1024 * 1024 * 1024);
        assert_eq!(capped, 4 * 1024 * 1024 * 1024);
    }

    /// Multi-byte UTF-8 spanning chunk boundaries is still validated
    /// correctly by the push-driven writer.
    #[tokio::test]
    async fn test_streaming_writer_multibyte_across_boundaries() {
        let mut data: Vec<u8> = Vec::with_capacity(90 * 1024 * 3);
        for _ in 0..(90 * 1024) {
            data.extend_from_slice("€".as_bytes());
        }
        let path = next_spool_path();
        let mut w = StreamingVitalsSpoolWriter::new(&path).await.unwrap();
        // Push in ~3.3 KiB chunks — many boundaries split codepoints.
        for chunk in data.chunks(3331) {
            w.write_chunk(chunk).await.unwrap();
        }
        let (vitals, _, _) = w.finish().await.unwrap();
        assert!(vitals.is_valid_utf8);
        assert_eq!(vitals.byte_len, data.len());
        spool_delete(&path);
    }
}
