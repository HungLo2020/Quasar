//! GPU memory bookkeeping without GPU types.
//!
//! [`RangeAllocator`] sub-allocates large buffers with best-fit, O(log n)
//! allocation and coalescing frees. Frees are *deferred*: a range released
//! while a submission may still read it is parked until the caller reports
//! that submission complete. [`PagedArena`] spreads allocations over fixed
//! pages so buffers never need to move. Callers with any GPU abstraction
//! perform the actual uploads at the offsets this layer returns.
use std::collections::{BTreeMap, BTreeSet};

/// An allocated byte range.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Allocation {
    pub offset: u64,
    pub size: u64,
}

/// Best-fit range allocator with deferred, coalescing frees.
#[derive(Clone, Debug)]
pub struct RangeAllocator {
    capacity: u64,
    free_by_offset: BTreeMap<u64, u64>,
    free_by_size: BTreeSet<(u64, u64)>,
    pending: Vec<(u64, Allocation)>,
    used: u64,
}

fn align_up(value: u64, align: u64) -> u64 {
    value.div_ceil(align) * align
}

impl RangeAllocator {
    pub fn new(capacity: u64) -> Self {
        let mut a = Self {
            capacity,
            free_by_offset: BTreeMap::new(),
            free_by_size: BTreeSet::new(),
            pending: Vec::new(),
            used: 0,
        };
        if capacity > 0 {
            a.insert_free(0, capacity);
        }
        a
    }
    pub fn capacity(&self) -> u64 {
        self.capacity
    }
    pub fn used(&self) -> u64 {
        self.used
    }
    /// Bytes waiting for their submission to complete.
    pub fn pending_bytes(&self) -> u64 {
        self.pending.iter().map(|(_, a)| a.size).sum()
    }
    pub fn largest_free(&self) -> u64 {
        self.free_by_size
            .iter()
            .next_back()
            .map_or(0, |(size, _)| *size)
    }
    fn insert_free(&mut self, offset: u64, size: u64) {
        self.free_by_offset.insert(offset, size);
        self.free_by_size.insert((size, offset));
    }
    fn remove_free(&mut self, offset: u64, size: u64) {
        self.free_by_offset.remove(&offset);
        self.free_by_size.remove(&(size, offset));
    }
    /// Allocates `size` bytes at an offset that is a multiple of `align`
    /// (any non-zero value, e.g. a 12-byte element size).
    pub fn allocate(&mut self, size: u64, align: u64) -> Option<Allocation> {
        if size == 0 {
            return Some(Allocation { offset: 0, size: 0 });
        }
        let align = align.max(1);
        // Smallest block that fits even after alignment padding.
        let candidates: Vec<(u64, u64)> = self
            .free_by_size
            .range((size, 0)..)
            .take(8)
            .copied()
            .collect();
        for (block_size, offset) in candidates {
            let start = align_up(offset, align);
            let padding = start - offset;
            if padding + size > block_size {
                continue;
            }
            self.remove_free(offset, block_size);
            if padding > 0 {
                self.insert_free(offset, padding);
            }
            let tail = block_size - padding - size;
            if tail > 0 {
                self.insert_free(start + size, tail);
            }
            self.used += size;
            return Some(Allocation {
                offset: start,
                size,
            });
        }
        // Fall back to a scan for heavily aligned requests.
        let blocks: Vec<(u64, u64)> = self.free_by_offset.iter().map(|(o, s)| (*o, *s)).collect();
        for (offset, block_size) in blocks {
            let start = align_up(offset, align);
            let padding = start - offset;
            if block_size >= padding + size {
                self.remove_free(offset, block_size);
                if padding > 0 {
                    self.insert_free(offset, padding);
                }
                let tail = block_size - padding - size;
                if tail > 0 {
                    self.insert_free(start + size, tail);
                }
                self.used += size;
                return Some(Allocation {
                    offset: start,
                    size,
                });
            }
        }
        None
    }
    /// Frees immediately (the range is not in use by the GPU).
    pub fn free(&mut self, allocation: Allocation) {
        if allocation.size == 0 {
            return;
        }
        self.used -= allocation.size;
        let mut offset = allocation.offset;
        let mut size = allocation.size;
        if let Some((&prev, &prev_size)) = self.free_by_offset.range(..offset).next_back()
            && prev + prev_size == offset
        {
            self.remove_free(prev, prev_size);
            offset = prev;
            size += prev_size;
        }
        if let Some(&next_size) = self.free_by_offset.get(&(offset + size)) {
            self.remove_free(offset + size, next_size);
            size += next_size;
        }
        self.insert_free(offset, size);
    }
    /// Frees once `submission` is reported complete via [`Self::retire`].
    pub fn free_after(&mut self, allocation: Allocation, submission: u64) {
        self.pending.push((submission, allocation));
    }
    /// Releases every deferred range whose submission is `<= completed`.
    pub fn retire(&mut self, completed: u64) -> usize {
        let (done, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|(s, _)| *s <= completed);
        self.pending = keep;
        let count = done.len();
        for (_, allocation) in done {
            self.free(allocation);
        }
        count
    }
    /// Extends the managed range (e.g. after the buffer grew).
    pub fn grow(&mut self, capacity: u64) {
        if capacity <= self.capacity {
            return;
        }
        let extra = Allocation {
            offset: self.capacity,
            size: capacity - self.capacity,
        };
        self.capacity = capacity;
        self.used += extra.size;
        self.free(extra);
    }
}

/// An allocation inside a [`PagedArena`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PageAllocation {
    pub page: u32,
    pub range: Allocation,
}

/// Fixed-size pages, each a [`RangeAllocator`]; new pages are added on
/// demand (the caller creates a GPU buffer per page).
#[derive(Clone, Debug)]
pub struct PagedArena {
    page_size: u64,
    pages: Vec<RangeAllocator>,
}

impl PagedArena {
    pub fn new(page_size: u64) -> Self {
        Self {
            page_size,
            pages: Vec::new(),
        }
    }
    pub fn page_size(&self) -> u64 {
        self.page_size
    }
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }
    /// Allocates, adding a page if needed. Returns the allocation and whether
    /// a new page was created (the caller must create its buffer).
    pub fn allocate(&mut self, size: u64, align: u64) -> Option<(PageAllocation, bool)> {
        if size > self.page_size {
            return None;
        }
        for (i, page) in self.pages.iter_mut().enumerate() {
            if let Some(range) = page.allocate(size, align) {
                return Some((
                    PageAllocation {
                        page: i as u32,
                        range,
                    },
                    false,
                ));
            }
        }
        let mut page = RangeAllocator::new(self.page_size);
        let range = page.allocate(size, align)?;
        self.pages.push(page);
        Some((
            PageAllocation {
                page: (self.pages.len() - 1) as u32,
                range,
            },
            true,
        ))
    }
    pub fn free(&mut self, allocation: PageAllocation) {
        self.pages[allocation.page as usize].free(allocation.range);
    }
    pub fn free_after(&mut self, allocation: PageAllocation, submission: u64) {
        self.pages[allocation.page as usize].free_after(allocation.range, submission);
    }
    pub fn retire(&mut self, completed: u64) -> usize {
        self.pages.iter_mut().map(|p| p.retire(completed)).sum()
    }
    pub fn used(&self) -> u64 {
        self.pages.iter().map(RangeAllocator::used).sum()
    }
}
