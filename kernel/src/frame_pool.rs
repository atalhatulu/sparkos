//! Physical frame bookkeeping, independent of page tables and CPU instructions.
use alloc::{collections::BTreeSet, vec::Vec};

pub const PAGE_BYTES: u64 = 4096;

pub struct FramePool {
    regions: Vec<(u64, u64)>,
    recycled: Vec<u64>,
    allocated: BTreeSet<u64>,
    total_bytes: u64,
}

impl FramePool {
    pub fn new(regions: Vec<(u64, u64)>) -> Self {
        let total_bytes = regions.iter().map(|(start, end)| end - start).sum();
        Self { regions, recycled: Vec::new(), allocated: BTreeSet::new(), total_bytes }
    }

    pub fn allocate(&mut self, count: usize) -> Option<u64> {
        if count == 0 { return None; }
        let bytes = (count as u64).checked_mul(PAGE_BYTES)?;

        // Returned surface frames must be reusable as a contiguous extent.
        // Keep this sorted so a failed large request never consumes small runs.
        if count <= self.recycled.len() {
            let mut run_start = 0;
            for end in 0..self.recycled.len() {
                if end > 0 && self.recycled[end - 1].checked_add(PAGE_BYTES) != Some(self.recycled[end]) {
                    run_start = end;
                }
                if end - run_start + 1 == count {
                    let first = self.recycled[run_start];
                    for addr in self.recycled.drain(run_start..=end) {
                        self.allocated.insert(addr);
                    }
                    return Some(first);
                }
            }
        }

        for (start, end) in &mut self.regions {
            let Some(next) = start.checked_add(bytes) else { continue; };
            if next <= *end {
                let first = *start;
                *start = next;
                for i in 0..count {
                    self.allocated.insert(first + i as u64 * PAGE_BYTES);
                }
                return Some(first);
            }
        }
        None
    }

    pub fn release(&mut self, addr: u64) {
        if self.allocated.remove(&addr) {
            let index = self.recycled.binary_search(&addr).unwrap_err();
            self.recycled.insert(index, addr);
        }
    }

    pub fn stats(&self) -> (u64, u64) {
        (self.allocated.len() as u64 * PAGE_BYTES, self.total_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surfaces_reuse_returned_contiguous_frames() {
        let mut pool = FramePool::new(alloc::vec![(0x1000, 0x5000)]);
        assert_eq!(pool.allocate(4), Some(0x1000));
        for addr in [0x3000, 0x1000, 0x4000, 0x2000] { pool.release(addr); }
        assert_eq!(pool.stats(), (0, 0x4000));
        assert_eq!(pool.allocate(4), Some(0x1000));
        assert_eq!(pool.allocate(1), None);
    }

    #[test]
    fn failed_large_request_preserves_smaller_regions() {
        let mut pool = FramePool::new(alloc::vec![(0x1000, 0x3000), (0x9000, 0xd000)]);
        assert_eq!(pool.allocate(5), None);
        assert_eq!(pool.allocate(4), Some(0x9000));
        assert_eq!(pool.allocate(2), Some(0x1000));
    }

    #[test]
    fn fragmented_recycled_frames_are_not_a_contiguous_allocation() {
        let mut pool = FramePool::new(alloc::vec![(0x1000, 0x5000)]);
        pool.allocate(4).unwrap();
        pool.release(0x1000);
        pool.release(0x3000);
        assert_eq!(pool.allocate(2), None);
        assert_eq!(pool.allocate(1), Some(0x1000));
        assert_eq!(pool.allocate(1), Some(0x3000));
    }

    #[test]
    fn invalid_and_duplicate_frees_do_not_create_frames() {
        let mut pool = FramePool::new(alloc::vec![(0x1000, 0x2000)]);
        assert_eq!(pool.allocate(0), None);
        assert_eq!(pool.allocate(usize::MAX), None);
        pool.allocate(1).unwrap();
        for addr in [0x1000, 0x1000, 0x1001, 0x9000] { pool.release(addr); }
        assert_eq!(pool.allocate(1), Some(0x1000));
        assert_eq!(pool.allocate(1), None);
    }
}
