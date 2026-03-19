//! mmap-backed bloom filter with hugepage support for URL deduplication.
//!
//! The bloom filter is used as a **fast negative cache** layered in front of
//! the authoritative `HashSet` in `ListBucket`.  It never replaces the HashSet,
//! so there are **zero false positives** at the `ListBucket` API level:
//!
//! - Bloom says "definitely not present" → skip HashSet (fast path)
//! - Bloom says "maybe present" → fall through to HashSet (always correct)
//!
//! Memory is obtained via `mmap` (with `MAP_HUGETLB` on Linux for 2 MiB huge
//! pages) and released on `Drop`.  ~1.2 MB for 1 M URLs at default settings.

use std::hash::{Hash, Hasher};

/// Default expected number of elements.
const DEFAULT_CAPACITY: usize = 1_000_000;

/// Target false-positive probability for the bloom filter itself.
/// Note: false positives in the bloom only cause a HashSet lookup — they never
/// cause incorrect behavior at the `ListBucket` level.
const TARGET_FP: f64 = 0.01;

/// Number of hash functions (k) for the target FP rate.
/// k = -ln(p) / ln(2) ≈ 6.64 → 7
const NUM_HASHES: u32 = 7;

/// Compute optimal bit count for `n` elements at `fp` false-positive rate,
/// rounded up to the next **power of two** so modulo can be replaced with
/// a bitmask (`& mask`).
///
/// m = -n * ln(p) / (ln2)^2, then → next_power_of_two
fn optimal_bits(n: usize, fp: f64) -> usize {
    let m = -(n as f64) * fp.ln() / (core::f64::consts::LN_2.powi(2));
    let m = (m.ceil() as usize).max(64);
    // Round to next power of two for bitmask addressing.
    m.next_power_of_two()
}

/// Tracks how the backing memory was allocated so `Drop` can free it correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AllocKind {
    /// Allocated via `mmap` (possibly with hugepages). Stores the actual mapped
    /// size which may be larger than `len_bytes` due to hugepage alignment.
    Mmap { mapped_bytes: usize },
    /// Heap fallback via `Vec<u8>`.
    Heap,
}

/// An mmap-backed bloom filter.
///
/// Memory is allocated via `mmap` (with `MAP_HUGETLB` on Linux) and released
/// on `Drop`.  All bit operations are byte-granularity atomic-free — the
/// filter is designed for single-threaded insert/query, matching `ListBucket`.
pub struct MmapBloom {
    /// Pointer to the allocated region.
    ptr: *mut u8,
    /// Usable length in bytes (= num_bits / 8).
    len_bytes: usize,
    /// Bitmask for fast modulo: `num_bits - 1` (num_bits is always power of 2).
    mask: u64,
    /// Number of elements inserted (approximate — counts every insert call).
    count: usize,
    /// How the memory was allocated, for correct deallocation.
    alloc_kind: AllocKind,
}

// SAFETY: The mmap region is exclusively owned; no shared references escape.
unsafe impl Send for MmapBloom {}
unsafe impl Sync for MmapBloom {}

impl MmapBloom {
    /// Create a new bloom filter sized for `capacity` elements.
    ///
    /// Falls back gracefully: hugepages → regular mmap → heap `Vec`.
    pub fn new(capacity: usize) -> Self {
        let cap = if capacity == 0 {
            DEFAULT_CAPACITY
        } else {
            capacity
        };
        let bits = optimal_bits(cap, TARGET_FP);
        let len_bytes = bits / 8;

        let (ptr, alloc_kind) = Self::alloc(len_bytes);

        Self {
            ptr,
            len_bytes,
            mask: (bits as u64) - 1,
            count: 0,
            alloc_kind,
        }
    }

    /// Create with the default 1 M element capacity.
    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }

    /// Allocate `len` bytes.  Tries mmap (with hugepages on Linux), then heap.
    fn alloc(len: usize) -> (*mut u8, AllocKind) {
        #[cfg(unix)]
        {
            Self::alloc_unix(len)
        }
        #[cfg(not(unix))]
        {
            Self::alloc_heap(len)
        }
    }

    /// Unix mmap path — tries MAP_HUGETLB (Linux) then regular mmap, then heap.
    #[cfg(unix)]
    fn alloc_unix(len: usize) -> (*mut u8, AllocKind) {
        use libc::{mmap, MAP_ANONYMOUS, MAP_FAILED, MAP_PRIVATE, PROT_READ, PROT_WRITE};
        use std::ptr;

        // On Linux, first try with MAP_HUGETLB for 2 MiB huge pages.
        #[cfg(target_os = "linux")]
        {
            const MAP_HUGETLB: libc::c_int = 0x40000;
            const HUGE_PAGE_SIZE: usize = 2 << 20; // 2 MiB
                                                   // Round up to 2 MiB boundary for hugepages.
            let aligned = (len + HUGE_PAGE_SIZE - 1) & !(HUGE_PAGE_SIZE - 1);
            let p = unsafe {
                mmap(
                    ptr::null_mut(),
                    aligned,
                    PROT_READ | PROT_WRITE,
                    MAP_PRIVATE | MAP_ANONYMOUS | MAP_HUGETLB,
                    -1,
                    0,
                )
            };
            if p != MAP_FAILED {
                return (
                    p as *mut u8,
                    AllocKind::Mmap {
                        mapped_bytes: aligned,
                    },
                );
            }
            // Hugepage allocation failed (not configured / no free pages) — fall through.
        }

        // Regular mmap.
        let p = unsafe {
            mmap(
                ptr::null_mut(),
                len,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if p != MAP_FAILED {
            return (p as *mut u8, AllocKind::Mmap { mapped_bytes: len });
        }

        // Last resort: heap.
        Self::alloc_heap(len)
    }

    /// Heap fallback.
    fn alloc_heap(len: usize) -> (*mut u8, AllocKind) {
        let mut v: Vec<u8> = vec![0u8; len];
        let ptr = v.as_mut_ptr();
        std::mem::forget(v);
        (ptr, AllocKind::Heap)
    }

    /// Compute double-hash seeds for an item.
    ///
    /// h2 is derived via a MurmurHash3-style finalizer to decorrelate it from
    /// h1 — critical for low false-positive rates with power-of-2 masking.
    /// The `| 1` forces h2 odd (coprime with any power of 2) so all bit
    /// positions are reachable.
    #[inline(always)]
    fn hash_seeds<T: Hash + ?Sized>(item: &T) -> (u64, u64) {
        let mut state = ahash::AHasher::default();
        item.hash(&mut state);
        let h1 = state.finish();
        // MurmurHash3 64-bit finalizer — avalanches all bits.
        let mut x = h1;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51afd7ed558ccd);
        x ^= x >> 33;
        x = x.wrapping_mul(0xc4ceb9fe1a85ec53);
        x ^= x >> 33;
        (h1, x | 1)
    }

    /// Insert an item into the bloom filter.
    ///
    /// Computes each bit position inline — no intermediate array.
    /// Uses enhanced double hashing: h_i = h1 + i*h2 + i*(i-1)/2 to
    /// eliminate correlation artefacts with power-of-2 sizing.
    #[inline]
    pub fn insert<T: Hash + ?Sized>(&mut self, item: &T) {
        let (h1, h2) = Self::hash_seeds(item);
        let mask = self.mask;
        let mut composite = h1;
        for i in 0..NUM_HASHES as u64 {
            let pos = composite & mask;
            let byte_idx = (pos >> 3) as usize;
            let bit_idx = (pos & 7) as u8;
            // SAFETY: pos < num_bits (mask guarantees), num_bits == len_bytes * 8.
            unsafe {
                let byte = &mut *self.ptr.add(byte_idx);
                *byte |= 1 << bit_idx;
            }
            // Enhanced double hashing: next = h1 + (i+1)*h2 + (i+1)*i/2
            //   = composite + h2 + i
            composite = composite.wrapping_add(h2).wrapping_add(i);
        }
        self.count += 1;
    }

    /// Check if an item is probably in the set.
    ///
    /// Returns `false` as soon as *any* bit is unset — on the common "absent"
    /// path this exits after testing only 1-2 bits instead of all 7.
    #[inline]
    pub fn contains<T: Hash + ?Sized>(&self, item: &T) -> bool {
        let (h1, h2) = Self::hash_seeds(item);
        let mask = self.mask;
        let mut composite = h1;
        for i in 0..NUM_HASHES as u64 {
            let pos = composite & mask;
            let byte_idx = (pos >> 3) as usize;
            let bit_idx = (pos & 7) as u8;
            // SAFETY: same invariant as `insert`.
            let set = unsafe {
                let byte = *self.ptr.add(byte_idx);
                byte & (1 << bit_idx) != 0
            };
            if !set {
                return false;
            }
            composite = composite.wrapping_add(h2).wrapping_add(i);
        }
        true
    }

    /// Approximate number of insertions performed.
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether any insertions have been performed.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Reset the filter — zero all bits and reset the counter.
    pub fn clear(&mut self) {
        // SAFETY: `ptr` is valid for `len_bytes` bytes (our allocation
        // invariant). write_bytes is equivalent to memset(0).
        unsafe {
            std::ptr::write_bytes(self.ptr, 0, self.len_bytes);
        }
        self.count = 0;
    }

    /// Size of the usable allocation in bytes.
    #[inline]
    pub fn size_bytes(&self) -> usize {
        self.len_bytes
    }
}

impl Drop for MmapBloom {
    fn drop(&mut self) {
        if self.len_bytes == 0 || self.ptr.is_null() {
            return;
        }
        match self.alloc_kind {
            #[cfg(unix)]
            AllocKind::Mmap { mapped_bytes } => {
                // SAFETY: `ptr` was returned by mmap with size `mapped_bytes`.
                unsafe {
                    libc::munmap(self.ptr as *mut libc::c_void, mapped_bytes);
                }
            }
            #[cfg(not(unix))]
            AllocKind::Mmap { .. } => {
                // Unreachable on non-unix, but handle gracefully.
                unsafe {
                    let _ = Vec::from_raw_parts(self.ptr, self.len_bytes, self.len_bytes);
                }
            }
            AllocKind::Heap => {
                // SAFETY: `ptr` was obtained from a `Vec<u8>` of exactly
                // `len_bytes` length and capacity, then `mem::forget`'d.
                unsafe {
                    let _ = Vec::from_raw_parts(self.ptr, self.len_bytes, self.len_bytes);
                }
            }
        }
    }
}

impl std::fmt::Debug for MmapBloom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MmapBloom")
            .field("num_bits", &(self.mask + 1))
            .field("count", &self.count)
            .field("size_bytes", &self.len_bytes)
            .field("alloc_kind", &self.alloc_kind)
            .finish()
    }
}

impl Clone for MmapBloom {
    fn clone(&self) -> Self {
        let (ptr, alloc_kind) = Self::alloc(self.len_bytes);
        // SAFETY: both `self.ptr` and `ptr` are valid for `self.len_bytes`
        // bytes. The regions don't overlap (fresh allocation).
        unsafe {
            std::ptr::copy_nonoverlapping(self.ptr, ptr, self.len_bytes);
        }
        Self {
            ptr,
            len_bytes: self.len_bytes,
            mask: self.mask,
            count: self.count,
            alloc_kind,
        }
    }
}

impl PartialEq for MmapBloom {
    fn eq(&self, other: &Self) -> bool {
        if self.len_bytes != other.len_bytes || self.count != other.count {
            return false;
        }
        // SAFETY: both pointers are valid for `len_bytes` bytes.
        unsafe {
            std::slice::from_raw_parts(self.ptr, self.len_bytes)
                == std::slice::from_raw_parts(other.ptr, other.len_bytes)
        }
    }
}

impl Eq for MmapBloom {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_contains() {
        let mut bloom = MmapBloom::new(1000);
        bloom.insert(&"https://example.com");
        assert!(bloom.contains(&"https://example.com"));
        assert!(!bloom.contains(&"https://other.com"));
    }

    #[test]
    fn test_empty() {
        let bloom = MmapBloom::new(1000);
        assert!(bloom.is_empty());
        assert_eq!(bloom.len(), 0);
        assert!(!bloom.contains(&"anything"));
    }

    #[test]
    fn test_clear() {
        let mut bloom = MmapBloom::new(1000);
        bloom.insert(&"https://example.com");
        assert!(bloom.contains(&"https://example.com"));
        bloom.clear();
        assert!(bloom.is_empty());
        assert!(!bloom.contains(&"https://example.com"));
    }

    #[test]
    fn test_false_positive_rate() {
        let n = 10_000;
        let mut bloom = MmapBloom::new(n);

        // Insert n items.
        for i in 0..n {
            bloom.insert(&format!("url-{}", i));
        }
        assert_eq!(bloom.len(), n);

        // Check items that were NOT inserted.
        let test_count = 10_000;
        let mut false_positives = 0;
        for i in n..(n + test_count) {
            if bloom.contains(&format!("url-{}", i)) {
                false_positives += 1;
            }
        }

        let fp_rate = false_positives as f64 / test_count as f64;
        // Should be well under 5% (target is 1%).
        assert!(
            fp_rate < 0.05,
            "False positive rate too high: {:.2}%",
            fp_rate * 100.0
        );
    }

    #[test]
    fn test_no_false_negatives() {
        let mut bloom = MmapBloom::new(5000);
        let urls: Vec<String> = (0..5000)
            .map(|i| format!("https://site.com/{}", i))
            .collect();

        for url in &urls {
            bloom.insert(url);
        }

        // Every inserted item MUST be found — bloom filters guarantee zero false negatives.
        for url in &urls {
            assert!(bloom.contains(url), "False negative for {}", url);
        }
    }

    #[test]
    fn test_clone() {
        let mut bloom = MmapBloom::new(100);
        bloom.insert(&"https://a.com");
        bloom.insert(&"https://b.com");

        let bloom2 = bloom.clone();
        assert!(bloom2.contains(&"https://a.com"));
        assert!(bloom2.contains(&"https://b.com"));
        assert_eq!(bloom2.len(), 2);
    }

    #[test]
    fn test_size_reasonable() {
        let bloom = MmapBloom::new(1_000_000);
        // For 1M items at 1% FP: ~1.2 MB optimal, rounded to next power of 2 → 2 MB.
        assert!(bloom.size_bytes() > 1_000_000);
        assert!(bloom.size_bytes() <= 2_097_152); // 2 MiB (16 Mbit)
    }

    #[test]
    fn test_default_capacity() {
        let bloom = MmapBloom::with_default_capacity();
        assert!(bloom.size_bytes() > 0);
        assert!(bloom.is_empty());
    }

    #[test]
    fn test_optimal_bits() {
        let bits = optimal_bits(1_000_000, 0.01);
        // ~9.58M optimal → next power of 2 = 16_777_216 (2^24)
        assert!(bits.is_power_of_two());
        assert_eq!(bits, 16_777_216);
    }

    #[test]
    fn test_drop_safety() {
        // Ensure no panics on drop for various sizes.
        for size in [0, 1, 100, 10_000, 1_000_000] {
            let bloom = MmapBloom::new(size);
            drop(bloom);
        }
    }

    #[test]
    fn test_clone_independence() {
        let mut bloom = MmapBloom::new(100);
        bloom.insert(&"url-a");
        let mut bloom2 = bloom.clone();

        // Mutating clone doesn't affect original.
        bloom2.insert(&"url-b");
        assert!(!bloom.contains(&"url-b"));
        assert!(bloom2.contains(&"url-b"));
    }
}
