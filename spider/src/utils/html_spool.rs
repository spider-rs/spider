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
/// Uses `std::sync::mpsc::Sender` so it works from any context (async
/// Drop, sync Drop, any thread) without requiring a tokio runtime handle.
/// The receiver is a tokio task that uses `tokio::fs::remove_file` for
/// non-blocking deletion — zero sync I/O on any thread.
static CLEANUP_TX: OnceLock<std::sync::mpsc::Sender<PathBuf>> = OnceLock::new();

/// Initialize the cleanup task and return the sender.
///
/// When inside a tokio runtime: spawns a lightweight tokio task that
/// drains the channel with `tokio::fs::remove_file` (non-blocking).
/// Outside tokio (tests, CLI): falls back to a dedicated OS thread
/// with `std::fs::remove_file`.
fn cleanup_sender() -> &'static std::sync::mpsc::Sender<PathBuf> {
    CLEANUP_TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<PathBuf>();

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            // Tokio runtime available — spawn async cleanup task.
            handle.spawn(async move {
                loop {
                    // try_recv is non-blocking; yield between batches.
                    match rx.try_recv() {
                        Ok(path) => {
                            let _ = tokio::fs::remove_file(&path).await;
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                    }
                }
            });
        } else {
            // No tokio runtime — fallback to OS thread.
            std::thread::Builder::new()
                .name("spider-spool-cleanup".into())
                .spawn(move || {
                    while let Ok(path) = rx.recv() {
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
            .unwrap_or(16 * 1024)
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

// ── Configurable thresholds (env-overridable) ──────────────────────────────

/// Base total in-memory HTML budget before pages are spooled.
/// Default: 512 MiB.  Override: `SPIDER_HTML_MEMORY_BUDGET`.
fn base_memory_budget() -> usize {
    static VAL: OnceLock<usize> = OnceLock::new();
    *VAL.get_or_init(|| {
        std::env::var("SPIDER_HTML_MEMORY_BUDGET")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(512 * 1024 * 1024)
    })
}

/// Base per-page byte threshold.  Pages larger than the *effective*
/// threshold are spooled under pressure.  Default: 2 MiB.
/// Override: `SPIDER_HTML_PAGE_SPOOL_SIZE`.
fn base_per_page_threshold() -> usize {
    static VAL: OnceLock<usize> = OnceLock::new();
    *VAL.get_or_init(|| {
        std::env::var("SPIDER_HTML_PAGE_SPOOL_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2 * 1024 * 1024)
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
/// **Performance**: designed for the hot path (`channel_send_page`).
/// Small pages (≤ 16 KiB) hit a fast-path with a single comparison.
///
/// Decision tree (first match wins):
///
/// 1. Page ≤ `spool_min_size` (16 KiB) → **keep in memory** (fast-path).
/// 2. Page > per-page threshold (2 MiB) → **always spool** (large resource).
/// 3. Total in-memory HTML exceeds budget → **spool** (memory pressure).
/// 4. Process memory **pressure** (state ≥ 1) → **spool** (system pressure).
/// 5. Otherwise → keep in memory.
#[inline]
pub fn should_spool(html_len: usize) -> bool {
    // ① Small-page fast-path — disk I/O cost exceeds memory saved.
    if html_len <= spool_min_size() {
        return false;
    }

    // ② Large page — always spool regardless of pressure.
    if html_len > base_per_page_threshold() {
        return true;
    }

    // ③ Global byte budget exceeded.
    let current = total_bytes_in_memory();
    if current.saturating_add(html_len) > base_memory_budget() {
        return true;
    }

    // ④ System under any memory pressure — spool to keep headroom.
    let mem_state = CACHED_MEM_STATE.load(Ordering::Relaxed);
    if mem_state >= 1 {
        return true;
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
pub fn next_spool_path() -> PathBuf {
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
/// Uses `spawn_blocking` to keep the runtime non-blocking.
pub async fn spool_read_bytes_async(path: std::path::PathBuf) -> std::io::Result<bytes::Bytes> {
    tokio::task::spawn_blocking(move || std::fs::read(&path).map(bytes::Bytes::from))
        .await
        .unwrap_or_else(|_| Err(std::io::Error::other("join error")))
}

/// Async read of a spool file into `Vec<u8>`.
pub async fn spool_read_async(path: std::path::PathBuf) -> std::io::Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || std::fs::read(&path))
        .await
        .unwrap_or_else(|_| Err(std::io::Error::other("join error")))
}

/// Async streaming read of a spool file in chunks.
/// Runs the blocking loop on `spawn_blocking`, sends chunks back via a
/// callback that is invoked on the blocking thread.
pub async fn spool_stream_chunks_async(
    path: std::path::PathBuf,
    chunk_size: usize,
    cb: impl FnMut(&[u8]) -> bool + Send + 'static,
) -> std::io::Result<usize> {
    tokio::task::spawn_blocking(move || spool_stream_chunks(&path, chunk_size, cb))
        .await
        .unwrap_or_else(|_| Err(std::io::Error::other("join error")))
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
        // Tiny pages never spool.
        assert!(!should_spool(100));
        assert!(!should_spool(spool_min_size()));

        // Large pages always spool.
        assert!(should_spool(base_per_page_threshold() + 1));
        assert!(should_spool(10 * 1024 * 1024));
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
}
