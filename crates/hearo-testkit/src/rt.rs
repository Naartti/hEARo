//! Real-time safety harness: a global allocator that counts allocations made while a
//! "no allocation" region is active on the current thread.
//!
//! Unlike `assert_no_alloc` (which aborts the process), counting lets tests report exactly
//! how many allocations happened and lets us test the harness itself. See ADR 0007.
//!
//! ```ignore
//! #[global_allocator]
//! static ALLOC: hearo_testkit::rt::CountingAllocator = hearo_testkit::rt::CountingAllocator;
//!
//! let (_, allocs) = hearo_testkit::rt::count_allocs(|| chain.process(&mut block));
//! assert_eq!(allocs, 0);
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static GUARDED: Cell<bool> = const { Cell::new(false) };
    static COUNT: Cell<usize> = const { Cell::new(0) };
}

/// Install as `#[global_allocator]` in a test binary to enable [`count_allocs`].
#[derive(Debug)]
pub struct CountingAllocator;

#[inline]
fn note() {
    // `try_with` so allocations during thread teardown never panic.
    let _ = GUARDED.try_with(|g| {
        if g.get() {
            let _ = COUNT.try_with(|c| c.set(c.get() + 1));
        }
    });
}

// SAFETY: forwards every call to the system allocator unchanged; only counts.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note();
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note();
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Run `f` and return its result plus the number of allocations, deallocations and
/// reallocations it made on this thread. Only meaningful when [`CountingAllocator`] is the
/// global allocator.
pub fn count_allocs<R>(f: impl FnOnce() -> R) -> (R, usize) {
    COUNT.with(|c| c.set(0));
    GUARDED.with(|g| g.set(true));
    let r = f();
    GUARDED.with(|g| g.set(false));
    (r, COUNT.with(Cell::get))
}
