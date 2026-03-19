//! NUMA-aware thread pinning for multi-socket servers.
//!
//! On multi-socket servers, memory access latency doubles when a thread
//! crosses NUMA node boundaries.  This module detects the NUMA topology
//! at runtime and pins worker threads to the node they belong to, keeping
//! memory accesses local.
//!
//! # Linux
//! Uses `sched_setaffinity(2)` via `libc` FFI to bind the calling thread
//! to CPUs within a single NUMA node.  Topology is read from
//! `/sys/devices/system/node/`.
//!
//! # Non-Linux
//! Compiles to no-ops — NUMA pinning is a Linux-specific optimisation.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether NUMA topology was successfully detected.
static NUMA_DETECTED: AtomicBool = AtomicBool::new(false);

/// Whether NUMA pinning is enabled and available.
#[inline]
pub fn is_numa_available() -> bool {
    NUMA_DETECTED.load(Ordering::Relaxed)
}

// ─── Linux implementation ───────────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod inner {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// A NUMA node with its associated CPU set.
    #[derive(Debug, Clone)]
    pub struct NumaNode {
        /// Node id (0, 1, …).
        pub id: u32,
        /// CPU ids belonging to this node.
        pub cpus: Vec<u32>,
    }

    /// Cached NUMA topology.
    static TOPOLOGY: std::sync::OnceLock<Vec<NumaNode>> = std::sync::OnceLock::new();

    /// Parse a Linux CPU list string like "0-3,8-11" into individual CPU ids.
    fn parse_cpu_list(s: &str) -> Vec<u32> {
        let mut cpus = Vec::new();
        for part in s.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if let Some((start, end)) = part.split_once('-') {
                if let (Ok(s), Ok(e)) = (start.trim().parse::<u32>(), end.trim().parse::<u32>()) {
                    cpus.extend(s..=e);
                }
            } else if let Ok(cpu) = part.parse::<u32>() {
                cpus.push(cpu);
            }
        }
        cpus
    }

    /// Detect NUMA topology from sysfs.
    ///
    /// Returns `None` on single-node systems (NUMA pinning would be pointless).
    fn detect_topology() -> Option<Vec<NumaNode>> {
        let node_dir = Path::new("/sys/devices/system/node");
        if !node_dir.exists() {
            return None;
        }

        let mut nodes = Vec::new();

        let entries = fs::read_dir(node_dir).ok()?;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.starts_with("node") {
                continue;
            }
            let id: u32 = name_str[4..].parse().ok()?;

            let cpulist_path = entry.path().join("cpulist");
            let cpulist = fs::read_to_string(cpulist_path).ok()?;
            let cpus = parse_cpu_list(&cpulist);
            if !cpus.is_empty() {
                nodes.push(NumaNode { id, cpus });
            }
        }

        // Sort by node id for deterministic assignment.
        nodes.sort_by_key(|n| n.id);

        // Only useful on multi-node systems.
        if nodes.len() >= 2 {
            Some(nodes)
        } else {
            None
        }
    }

    /// Initialise NUMA detection.  Safe to call multiple times (idempotent).
    pub fn init_numa() {
        TOPOLOGY.get_or_init(|| match detect_topology() {
            Some(topo) => {
                log::info!(
                    "NUMA topology detected: {} nodes, {} total CPUs",
                    topo.len(),
                    topo.iter().map(|n| n.cpus.len()).sum::<usize>()
                );
                for node in &topo {
                    log::debug!("  node {}: cpus {:?}", node.id, node.cpus);
                }
                NUMA_DETECTED.store(true, Ordering::Relaxed);
                topo
            }
            None => {
                log::debug!("Single NUMA node or sysfs unavailable — pinning disabled");
                Vec::new()
            }
        });
    }

    /// Get the detected topology (empty if single-node or not initialised).
    pub fn topology() -> &'static [NumaNode] {
        TOPOLOGY.get().map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Returns the number of NUMA nodes detected.
    pub fn node_count() -> usize {
        topology().len()
    }

    /// Pin the **calling** thread to CPUs belonging to `node_id`.
    ///
    /// Returns `Ok(())` on success, `Err` with an OS error on failure.
    /// Silently succeeds if NUMA is not available or the node id is invalid.
    pub fn pin_thread_to_node(node_id: usize) -> Result<(), std::io::Error> {
        let topo = topology();
        if topo.is_empty() || node_id >= topo.len() {
            return Ok(());
        }

        let node = &topo[node_id];
        pin_thread_to_cpus(&node.cpus)
    }

    /// Pin the **calling** thread to a specific set of CPU ids.
    pub fn pin_thread_to_cpus(cpus: &[u32]) -> Result<(), std::io::Error> {
        if cpus.is_empty() {
            return Ok(());
        }

        unsafe {
            let mut cpuset: libc::cpu_set_t = std::mem::zeroed();

            for &cpu in cpus {
                libc::CPU_SET(cpu as usize, &mut cpuset);
            }

            let ret = libc::sched_setaffinity(
                0, // 0 = calling thread
                std::mem::size_of::<libc::cpu_set_t>(),
                &cpuset,
            );

            if ret == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        }
    }

    /// Pin the calling thread to the NUMA node selected by round-robin
    /// based on `worker_index`.
    ///
    /// This is the primary API for use in `on_thread_start` callbacks.
    pub fn pin_worker(worker_index: usize) {
        let topo = topology();
        if topo.is_empty() {
            return;
        }
        let node_id = worker_index % topo.len();
        if let Err(e) = pin_thread_to_node(node_id) {
            log::warn!(
                "Failed to pin worker {} to NUMA node {}: {}",
                worker_index,
                node_id,
                e
            );
        } else {
            log::trace!("Worker {} pinned to NUMA node {}", worker_index, node_id);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_parse_cpu_list_range() {
            assert_eq!(parse_cpu_list("0-3"), vec![0, 1, 2, 3]);
        }

        #[test]
        fn test_parse_cpu_list_mixed() {
            assert_eq!(parse_cpu_list("0-2,5,8-9"), vec![0, 1, 2, 5, 8, 9]);
        }

        #[test]
        fn test_parse_cpu_list_single() {
            assert_eq!(parse_cpu_list("7"), vec![7]);
        }

        #[test]
        fn test_parse_cpu_list_empty() {
            assert!(parse_cpu_list("").is_empty());
        }

        #[test]
        fn test_parse_cpu_list_whitespace() {
            assert_eq!(parse_cpu_list(" 1 - 3 , 5 "), vec![1, 2, 3, 5]);
        }

        #[test]
        fn test_init_numa_idempotent() {
            init_numa();
            init_numa(); // second call should be no-op
        }

        #[test]
        fn test_topology_returns_slice() {
            init_numa();
            let topo = topology();
            // On single-socket dev machines this is empty; on multi-socket it's >1.
            // Either way it shouldn't panic.
            let _ = topo.len();
        }

        #[test]
        fn test_pin_worker_no_panic() {
            init_numa();
            // Should not panic even on single-node systems.
            pin_worker(0);
            pin_worker(999);
        }

        #[test]
        fn test_pin_thread_to_node_invalid() {
            init_numa();
            // Invalid node id should silently succeed.
            assert!(pin_thread_to_node(9999).is_ok());
        }

        #[test]
        fn test_pin_thread_to_cpus_empty() {
            assert!(pin_thread_to_cpus(&[]).is_ok());
        }

        #[test]
        fn test_node_count() {
            init_numa();
            // Just check it doesn't panic.
            let _ = node_count();
        }
    }
}

// ─── Non-Linux stub ─────────────────────────────────────────────────────────

#[cfg(not(target_os = "linux"))]
mod inner {
    /// Initialise NUMA detection (no-op on non-Linux).
    #[inline]
    pub fn init_numa() {}

    /// Returns 0 on non-Linux platforms.
    #[inline]
    pub fn node_count() -> usize {
        0
    }

    /// No-op on non-Linux.
    #[inline]
    pub fn pin_worker(_worker_index: usize) {}

    /// No-op on non-Linux.
    #[inline]
    pub fn pin_thread_to_node(_node_id: usize) -> Result<(), std::io::Error> {
        Ok(())
    }

    /// No-op on non-Linux.
    #[inline]
    pub fn pin_thread_to_cpus(_cpus: &[u32]) -> Result<(), std::io::Error> {
        Ok(())
    }
}

// Re-export the platform-specific implementation.
pub use inner::*;
