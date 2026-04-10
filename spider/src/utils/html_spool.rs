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
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;

// ── Global byte accounting ─────────────────────────────────────────────────

/// Total HTML bytes currently held in memory across all `Page` instances.
static TOTAL_HTML_BYTES_IN_MEMORY: AtomicUsize = AtomicUsize::new(0);

/// Number of pages currently spooled to disk.
static PAGES_ON_DISK: AtomicUsize = AtomicUsize::new(0);

/// Monotonic counter for generating unique spool file names.
static SPOOL_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

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

/// Compute the *effective* per-page spool threshold for the current memory
/// state.  Under pressure the threshold halves so smaller pages start
/// spilling earlier.  Under critical pressure the threshold drops to 0
/// (spool everything).
#[inline]
fn effective_per_page_threshold(mem_state: i8) -> usize {
    match mem_state {
        s if s >= 2 => 0,
        1 => base_per_page_threshold() / 2,
        _ => base_per_page_threshold(),
    }
}

/// Compute the *effective* global budget for the current memory state.
/// Under pressure the budget tightens to ¾; under critical it drops to 0.
#[inline]
fn effective_budget(mem_state: i8) -> usize {
    match mem_state {
        s if s >= 2 => 0,
        1 => base_memory_budget() * 3 / 4,
        _ => base_memory_budget(),
    }
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

// ── Spool decision logic ───────────────────────────────────────────────────

/// Decide whether a page with `html_len` bytes should be spooled to disk.
///
/// The decision tree adapts to the current memory pressure level:
///
/// 1. **Critical** (state 2) → always spool (effective budget & threshold = 0).
/// 2. Total in-memory HTML exceeds *effective* budget → spool.
/// 3. **Pressure** (state 1) AND page > *effective* per-page threshold → spool.
/// 4. **Normal** (state 0) AND budget exceeded → spool.
/// 5. Otherwise → keep in memory.
pub fn should_spool(html_len: usize) -> bool {
    let mem_state = crate::utils::detect_system::get_process_memory_state_sync();

    // Critical pressure — spool everything.
    if mem_state >= 2 {
        return true;
    }

    let budget = effective_budget(mem_state);
    let current = total_bytes_in_memory();

    // Global byte budget exceeded (budget adapts to pressure level).
    if current.saturating_add(html_len) > budget {
        return true;
    }

    // Pressure + page exceeds the (halved) per-page threshold.
    if mem_state >= 1 && html_len > effective_per_page_threshold(mem_state) {
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
    pub fn base_per_page_helper() -> usize {
        base_per_page_threshold()
    }

    /// Expose base_memory_budget for cross-module tests.
    pub fn base_budget_helper() -> usize {
        base_memory_budget()
    }

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
    fn test_adaptive_thresholds() {
        let base_pp = base_per_page_threshold();
        let base_budget = base_memory_budget();

        // State 0 — full thresholds.
        assert_eq!(effective_per_page_threshold(0), base_pp);
        assert_eq!(effective_budget(0), base_budget);

        // State 1 — halved per-page, ¾ budget.
        assert_eq!(effective_per_page_threshold(1), base_pp / 2);
        assert_eq!(effective_budget(1), base_budget * 3 / 4);

        // State 2 — zero thresholds (everything spools).
        assert_eq!(effective_per_page_threshold(2), 0);
        assert_eq!(effective_budget(2), 0);

        // State 3+ — same as critical.
        assert_eq!(effective_per_page_threshold(3), 0);
        assert_eq!(effective_budget(3), 0);
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
