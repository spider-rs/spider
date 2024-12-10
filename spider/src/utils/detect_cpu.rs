use std::sync::atomic::{AtomicI8, Ordering};
use sysinfo::System;
use tokio::sync::OnceCell;
use tokio::time::sleep;

/// The CPU state for the crawl.
///
static CPU_STATE: AtomicI8 = AtomicI8::new(0);

/// `OnceCell` CPU tracking.
static INIT: OnceCell<()> = OnceCell::const_new();

/// Get the total avg CPU being used.
fn get_cpu_usage(sys: &System) -> f32 {
    sys.cpus()
        .iter()
        .map(|cpu| cpu.cpu_usage() / sys.cpus().len() as f32)
        .sum::<f32>()
}

/// Update the cpu usage being used.
async fn update_cpu_usage() {
    if sysinfo::IS_SUPPORTED_SYSTEM {
        let mut sys = System::new();

        loop {
            sys.refresh_cpu_usage();
            let usage = get_cpu_usage(&sys);
            let state = if usage >= 70.0 {
                1
            } else if usage >= 95.0 {
                2
            } else {
                0
            };
            CPU_STATE.store(state, Ordering::Relaxed);
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

/// Get the cpu usage being used utility.
pub async fn get_global_cpu_usage() -> i8 {
    init_once().await;
    CPU_STATE.load(Ordering::Relaxed)
}
