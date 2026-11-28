//! Profiling builds only (`trace-chrome`): the system allocator with two counters, so the trace
//! can show heap allocations and bytes per frame. A global allocator is a static by design; this
//! one exists only in profiling builds and holds nothing but the counters.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicU64, Ordering::Relaxed},
};

pub struct Counting {
    allocations: AtomicU64,
    bytes: AtomicU64,
}

impl Counting {
    fn count(&self, size: usize) {
        self.allocations.fetch_add(1, Relaxed);
        self.bytes.fetch_add(size as u64, Relaxed);
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting {
    allocations: AtomicU64::new(0),
    bytes: AtomicU64::new(0),
};

// SAFETY: every call forwards to the system allocator with the caller's arguments unchanged;
// the counters do not touch the memory.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.count(layout.size());
        // SAFETY: the caller's layout, forwarded unchanged.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        self.count(layout.size());
        // SAFETY: the caller's layout, forwarded unchanged.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        self.count(new_size);
        // SAFETY: the caller's pointer, layout and size, forwarded unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller's pointer and layout, forwarded unchanged.
        unsafe { System.dealloc(ptr, layout) }
    }
}

/// Allocations (including reallocations) and their requested bytes since the start.
pub fn totals() -> (u64, u64) {
    (
        ALLOCATOR.allocations.load(Relaxed),
        ALLOCATOR.bytes.load(Relaxed),
    )
}
