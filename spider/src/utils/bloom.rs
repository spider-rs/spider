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

/// Compute optimal bit count for `n` elements at `fp` false-positive rate.
/// m = -n * ln(p) / (ln2)^2
fn optimal_bits(n: usize, fp: f64) -> usize {
    let m = -(n as f64) * fp.ln() / (core::f64::consts::LN_2.powi(2));
    // Round up to next multiple of 8 so we address whole bytes.
    let m = m.ceil() as usize;
    (m + 7) & !7
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
    /// Total number of usable bits (= len_bytes * 8).
    num_bits: u64,
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
            num_bits: bits as u64,
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

    /// Compute the k bit positions for a given item using double hashing.
    /// h_i = h1 + i * h2  (mod num_bits)
    #[inline(always)]
    fn bit_positions<T: Hash>(&self, item: &T) -> [u64; NUM_HASHES as usize] {
        let mut h1_state = ahash::AHasher::default();
        item.hash(&mut h1_state);
        let h1 = h1_state.finish();

        // Second hash: fold and mix.
        let h2 = h1
            .wrapping_mul(0x517cc1b727220a95)
            .wrapping_add(0x6c62272e07bb0142);

        let mut positions = [0u64; NUM_HASHES as usize];
        for i in 0..NUM_HASHES as u64 {
            positions[i as usize] = (h1.wrapping_add(i.wrapping_mul(h2))) % self.num_bits;
        }
        positions
    }

    /// Set bit at position `pos`.
    #[inline(always)]
    fn set_bit(&mut self, pos: u64) {
        let byte_idx = (pos / 8) as usize;
        let bit_idx = (pos % 8) as u8;
        debug_assert!(byte_idx < self.len_bytes);
        // SAFETY: `pos < num_bits` (enforced by modulo in `bit_positions`),
        // and `num_bits == len_bytes * 8`, so `byte_idx < len_bytes` always holds.
        unsafe {
            let byte = &mut *self.ptr.add(byte_idx);
            *byte |= 1 << bit_idx;
        }
    }

    /// Test bit at position `pos`.
    #[inline(always)]
    fn test_bit(&self, pos: u64) -> bool {
        let byte_idx = (pos / 8) as usize;
        let bit_idx = (pos % 8) as u8;
        debug_assert!(byte_idx < self.len_bytes);
        // SAFETY: same invariant as `set_bit`.
        unsafe {
            let byte = *self.ptr.add(byte_idx);
            byte & (1 << bit_idx) != 0
        }
    }

    /// Insert an item into the bloom filter.
    #[inline]
    pub fn insert<T: Hash>(&mut self, item: &T) {
        let positions = self.bit_positions(item);
        for &pos in &positions {
            self.set_bit(pos);
        }
        self.count += 1;
    }

    /// Check if an item is probably in the set.
    /// Returns `false` only when the item is *definitely* absent.
    #[inline]
    pub fn contains<T: Hash>(&self, item: &T) -> bool {
        let positions = self.bit_positions(item);
        positions.iter().all(|&pos| self.test_bit(pos))
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
            .field("num_bits", &self.num_bits)
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
            num_bits: self.num_bits,
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
        // For 1M items at 1% FP: ~1.2 MB
        assert!(bloom.size_bytes() > 1_000_000);
        assert!(bloom.size_bytes() < 2_000_000);
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
        // Should be ~9.58M bits ≈ 1.2 MB
        assert!(bits > 9_000_000);
        assert!(bits < 10_000_000);
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
