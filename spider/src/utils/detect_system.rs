use std::sync::atomic::{AtomicI8, Ordering};
use sysinfo::System;
use tokio::sync::OnceCell;
use tokio::time::sleep;

/// The CPU state for the crawl.
static CPU_STATE: AtomicI8 = AtomicI8::new(0);

/// The System Memory state for the crawl.
#[cfg(feature = "disk")]
static MEMORY_STATE: AtomicI8 = AtomicI8::new(0);

/// `OnceCell` CPU tracking.
static INIT: OnceCell<()> = OnceCell::const_new();

/// Get the total avg CPU being used.
fn get_cpu_usage(sys: &System) -> f32 {
    sys.cpus()
        .iter()
        .map(|cpu| cpu.cpu_usage() / sys.cpus().len() as f32)
        .sum::<f32>()
}

/// The total memory used.
#[cfg(feature = "disk")]
fn get_memory_limits(sys: &System) -> u64 {
    let total_memory = sys.total_memory();
    let used_memory = sys.used_memory();
    (used_memory / total_memory) * 100
}

/// The CPU state to determine how to use concurrency and delays.
/// 0 = Full Concurrency.
/// 1 = Shared Concurrency.
/// 2 = Shared Concurrency with delays.
fn determine_cpu_state(usage: f32) -> i8 {
    if usage >= 70.0 {
        1
    } else if usage >= 95.0 {
        2
    } else {
        0
    }
}

/// The Memory state to determine how to use concurrency and delays.
/// 0 = Full Memory.
/// 1 = Hybrid Memory/Disk.
/// 2 = Full Disk.
#[cfg(feature = "disk")]
fn determine_memory_state(usage: u64) -> i8 {
    if usage >= 50 {
        1
    } else if usage >= 80 {
        2
    } else {
        0
    }
}

/// Update the memory used.
#[cfg(feature = "disk")]
fn update_memory(sys: &mut System) {
    sys.refresh_memory();
    MEMORY_STATE.store(
        determine_memory_state(get_memory_limits(&sys)),
        Ordering::Relaxed,
    );
}

/// Update the memory used.
#[cfg(not(feature = "disk"))]
fn update_memory(_sys: &mut System) {}

/// Update the cpu used.
fn update_cpu(sys: &mut System) {
    sys.refresh_cpu_usage();
    CPU_STATE.store(determine_cpu_state(get_cpu_usage(&sys)), Ordering::Relaxed);
}

/// Update the cpu usage being used.
async fn update_cpu_usage() {
    if sysinfo::IS_SUPPORTED_SYSTEM {
        let mut sys = System::new();

        loop {
            update_cpu(&mut sys);
            update_memory(&mut sys);
            sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL).await;
        }
    }
}

/// Setup the cpu tracker.
async fn init_once() {
    INIT.get_or_init(|| async {
        tokio::spawn(update_cpu_usage());
    })
    .await;
}

/// Get the cpu usage being used state utility.
pub async fn get_global_cpu_state() -> i8 {
    init_once().await;
    CPU_STATE.load(Ordering::Relaxed)
}

/// Get the memory usage being used state utility.
#[cfg(feature = "disk")]
pub async fn get_global_memory_state() -> i8 {
    init_once().await;
    MEMORY_STATE.load(Ordering::Relaxed)
}

/// Get the memory usage being used state utility.
#[cfg(not(feature = "disk"))]
pub async fn get_global_memory_state() -> i8 {
    0
}
