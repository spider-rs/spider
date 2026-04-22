#[cfg(feature = "balance")]
use std::sync::atomic::{AtomicI8, Ordering};
#[cfg(feature = "balance")]
use sysinfo::System;
#[cfg(feature = "balance")]
use tokio::sync::OnceCell;
#[cfg(feature = "balance")]
use tokio::time::sleep;

/// The CPU state for the crawl.
#[cfg(feature = "balance")]
static CPU_STATE: AtomicI8 = AtomicI8::new(0);

/// The System Memory state for the crawl.
#[cfg(all(feature = "disk", feature = "balance"))]
static MEMORY_STATE: AtomicI8 = AtomicI8::new(0);

/// The process RSS memory state for the crawl.
/// 0 = Normal, 1 = Pressure, 2 = Critical.
#[cfg(feature = "balance")]
static PROCESS_MEMORY_STATE: AtomicI8 = AtomicI8::new(0);

/// `OnceCell` CPU tracking.
#[cfg(feature = "balance")]
static INIT: OnceCell<()> = OnceCell::const_new();

/// Get the total avg CPU being used.
#[cfg(feature = "balance")]
fn get_cpu_usage(sys: &System) -> f32 {
    sys.cpus()
        .iter()
        .map(|cpu| cpu.cpu_usage() / sys.cpus().len() as f32)
        .sum::<f32>()
}

/// The total memory used as a percentage (0–100).
#[cfg(all(feature = "disk", feature = "balance"))]
fn get_memory_limits(sys: &System) -> u64 {
    let total_memory = sys.total_memory();
    if total_memory == 0 {
        return 0;
    }
    let used_memory = sys.used_memory();
    (used_memory * 100) / total_memory
}

/// The CPU state to determine how to use concurrency and delays.
/// 0 = Full Concurrency.
/// 1 = Shared Concurrency.
/// 2 = Shared Concurrency with delays.
#[cfg(feature = "balance")]
fn determine_cpu_state(usage: f32) -> i8 {
    if usage >= 95.0 {
        2
    } else if usage >= 70.0 {
        1
    } else {
        0
    }
}

/// The Memory state to determine how to use concurrency and delays.
/// 0 = Full Memory.
/// 1 = Hybrid Memory/Disk.
/// 2 = Full Disk.
#[cfg(all(feature = "disk", feature = "balance"))]
fn determine_memory_state(usage: u64) -> i8 {
    if usage >= 80 {
        2
    } else if usage >= 50 {
        1
    } else {
        0
    }
}

/// The pressure threshold percentage for process RSS (default 70%).
#[cfg(feature = "balance")]
fn process_memory_pressure_pct() -> u64 {
    static VAL: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *VAL.get_or_init(|| {
        std::env::var("SPIDER_MEMORY_PRESSURE_PCT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(90) // 90% — high but tolerable, start being selective
    })
}

/// The critical threshold percentage for process RSS (default 95%).
/// At this level OOM is imminent — spool aggressively to survive.
#[cfg(feature = "balance")]
fn process_memory_critical_pct() -> u64 {
    static VAL: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *VAL.get_or_init(|| {
        std::env::var("SPIDER_MEMORY_CRITICAL_PCT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(95) // 95% — OOM imminent, spool everything
    })
}

/// The process memory state based on RSS as a percentage of system total.
/// 0 = Normal, 1 = Pressure, 2 = Critical.
#[cfg(feature = "balance")]
fn determine_process_memory_state(pct: u64) -> i8 {
    let critical = process_memory_critical_pct();
    let pressure = process_memory_pressure_pct();
    if pct >= critical {
        2
    } else if pct >= pressure {
        1
    } else {
        0
    }
}

/// Update the process RSS memory state.
#[cfg(feature = "balance")]
fn update_process_memory(sys: &mut System) {
    if let Ok(pid) = sysinfo::get_current_pid() {
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        if let Some(process) = sys.process(pid) {
            let rss = process.memory();
            let total = sys.total_memory();
            if let Some(pct) = (rss * 100).checked_div(total) {
                PROCESS_MEMORY_STATE.store(determine_process_memory_state(pct), Ordering::Relaxed);
            }
        }
    }
}

/// Update the memory used.
#[cfg(all(feature = "disk", feature = "balance"))]
fn update_memory(sys: &mut System) {
    sys.refresh_memory();
    MEMORY_STATE.store(
        determine_memory_state(get_memory_limits(sys)),
        Ordering::Relaxed,
    );
}

/// Update the memory used.
#[cfg(not(all(feature = "disk", feature = "balance")))]
#[cfg(feature = "balance")]
fn update_memory(_sys: &mut System) {}

/// Update the cpu used.
#[cfg(feature = "balance")]
fn update_cpu(sys: &mut System) {
    sys.refresh_cpu_usage();
    CPU_STATE.store(determine_cpu_state(get_cpu_usage(sys)), Ordering::Relaxed);
}

/// Update the cpu usage being used.
#[cfg(feature = "balance")]
async fn update_cpu_usage() {
    if sysinfo::IS_SUPPORTED_SYSTEM {
        let mut sys = System::new();

        loop {
            update_cpu(&mut sys);
            update_memory(&mut sys);
            update_process_memory(&mut sys);
            // Push the latest memory state into the html_spool cache so
            // should_spool() reads a single atomic instead of re-querying.
            crate::utils::html_spool::refresh_cached_mem_state();
            sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL).await;
        }
    }
}

/// Setup the cpu tracker.
#[cfg(feature = "balance")]
async fn init_once() {
    INIT.get_or_init(|| async {
        tokio::spawn(update_cpu_usage());
    })
    .await;
}

/// Get the cpu usage being used state utility.
#[cfg(feature = "balance")]
pub async fn get_global_cpu_state() -> i8 {
    init_once().await;
    CPU_STATE.load(Ordering::Relaxed)
}

/// Get CPU state without async overhead.
#[cfg(feature = "balance")]
pub fn get_global_cpu_state_sync() -> i8 {
    if INIT.initialized() {
        CPU_STATE.load(Ordering::Relaxed)
    } else {
        0
    }
}

/// Get the cpu usage being used state utility.
#[cfg(not(feature = "balance"))]
pub async fn get_global_cpu_state() -> i8 {
    0
}

/// Get CPU state without async overhead (no-op without balance).
#[cfg(not(feature = "balance"))]
pub fn get_global_cpu_state_sync() -> i8 {
    0
}

/// Get the memory usage being used state utility.
#[cfg(all(feature = "disk", feature = "balance"))]
pub async fn get_global_memory_state() -> i8 {
    init_once().await;
    MEMORY_STATE.load(Ordering::Relaxed)
}

/// Get the memory state without async overhead.
///
/// Returns the cached atomic value if the background monitor is already
/// initialized, otherwise returns 0 (no pressure). This avoids the
/// `OnceCell` poll + waker machinery on every call in hot paths like
/// `insert_link` / `insert_signature`.
#[cfg(all(feature = "disk", feature = "balance"))]
pub fn get_global_memory_state_sync() -> i8 {
    if INIT.initialized() {
        MEMORY_STATE.load(Ordering::Relaxed)
    } else {
        0
    }
}

/// Get the memory usage being used state utility.
#[cfg(all(feature = "disk", not(feature = "balance")))]
pub async fn get_global_memory_state() -> i8 {
    0
}

/// Get the memory state without async overhead (no-op without balance).
#[cfg(all(feature = "disk", not(feature = "balance")))]
pub fn get_global_memory_state_sync() -> i8 {
    0
}

/// Get the memory usage being used state utility.
#[cfg(not(feature = "disk"))]
pub async fn get_global_memory_state() -> i8 {
    0
}

/// Get the memory state without async overhead (no-op without disk).
#[cfg(not(feature = "disk"))]
pub fn get_global_memory_state_sync() -> i8 {
    0
}

/// Get the process RSS memory pressure state.
/// 0 = Normal, 1 = Pressure, 2 = Critical.
#[cfg(feature = "balance")]
pub async fn get_process_memory_state() -> i8 {
    init_once().await;
    PROCESS_MEMORY_STATE.load(Ordering::Relaxed)
}

/// Get process RSS memory state without async overhead.
#[cfg(feature = "balance")]
pub fn get_process_memory_state_sync() -> i8 {
    if INIT.initialized() {
        PROCESS_MEMORY_STATE.load(Ordering::Relaxed)
    } else {
        0
    }
}

/// Get the process RSS memory pressure state (no-op without balance).
#[cfg(not(feature = "balance"))]
pub async fn get_process_memory_state() -> i8 {
    0
}

/// Get process RSS memory state without async overhead (no-op without balance).
#[cfg(not(feature = "balance"))]
pub fn get_process_memory_state_sync() -> i8 {
    0
}

#[cfg(all(test, feature = "balance"))]
mod tests {
    use super::*;

    #[test]
    fn test_determine_cpu_state_all_states() {
        assert_eq!(determine_cpu_state(0.0), 0);
        assert_eq!(determine_cpu_state(50.0), 0);
        assert_eq!(determine_cpu_state(69.9), 0);
        assert_eq!(determine_cpu_state(70.0), 1);
        assert_eq!(determine_cpu_state(94.9), 1);
        assert_eq!(determine_cpu_state(95.0), 2);
        assert_eq!(determine_cpu_state(100.0), 2);
    }

    #[test]
    fn test_determine_process_memory_state_all_states() {
        // Default thresholds: pressure=90, critical=95
        assert_eq!(determine_process_memory_state(0), 0);
        assert_eq!(determine_process_memory_state(89), 0);
        assert_eq!(determine_process_memory_state(90), 1);
        assert_eq!(determine_process_memory_state(94), 1);
        assert_eq!(determine_process_memory_state(95), 2);
        assert_eq!(determine_process_memory_state(100), 2);
    }

    #[cfg(feature = "disk")]
    #[test]
    fn test_determine_memory_state_all_states() {
        assert_eq!(determine_memory_state(0), 0);
        assert_eq!(determine_memory_state(49), 0);
        assert_eq!(determine_memory_state(50), 1);
        assert_eq!(determine_memory_state(79), 1);
        assert_eq!(determine_memory_state(80), 2);
        assert_eq!(determine_memory_state(100), 2);
    }

    #[cfg(feature = "disk")]
    #[test]
    fn test_get_memory_limits_correct_percentage() {
        let mut sys = System::new();
        sys.refresh_memory();
        let total = sys.total_memory();
        if total > 0 {
            let pct = get_memory_limits(&sys);
            // Should be a reasonable percentage, not 0 from truncation
            assert!(pct <= 100, "memory percentage should be <= 100, got {pct}");
        }
    }
}
