use std::sync::atomic::{AtomicUsize, Ordering};
use sysinfo::System;
use tokio::sync::OnceCell;
use tokio::time::sleep;

/// Atomic value to store CPU usage.
static CPU_USAGE: AtomicUsize = AtomicUsize::new(0);

/// `OnceCell` CPU tracking.
static INIT: OnceCell<()> = OnceCell::const_new();

/// Get the total avg CPU being used.
fn get_cpu_usage(sys: &System) -> usize {
    let total: f32 = sys.cpus().iter().map(|cpu| cpu.cpu_usage()).sum();
    (total / sys.cpus().len() as f32) as usize
}

/// Update the cpu usage being used.
async fn update_cpu_usage() {
    if sysinfo::IS_SUPPORTED_SYSTEM {
        let mut sys = System::new();

        loop {
            sys.refresh_cpu_usage();
            let usage = get_cpu_usage(&sys);
            CPU_USAGE.store(usage, Ordering::Relaxed);
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

/// Get the cpu usage being used.
pub async fn get_global_cpu_usage() -> usize {
    init_once().await;
    CPU_USAGE.load(Ordering::Relaxed)
}
