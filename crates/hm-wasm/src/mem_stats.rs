//! Allocator counters for memory measurement (feature `mem-stats`, off in
//! the shipped build): the bytes the engine holds, beside the linear memory
//! the allocator has grown to.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use wasm_bindgen::prelude::*;

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

fn grow(n: usize) {
    let v = LIVE.fetch_add(n, Relaxed) + n;
    PEAK.fetch_max(v, Relaxed);
}

// SAFETY: every call forwards to the system allocator with the caller's
// arguments; the counters only observe sizes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        // SAFETY: forwarded as is.
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
            grow(l.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        // SAFETY: forwarded as is.
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
            grow(l.size());
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Relaxed);
        // SAFETY: forwarded as is.
        unsafe { System.dealloc(p, l) }
    }

    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        // SAFETY: forwarded as is.
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
            if n >= l.size() {
                grow(n - l.size());
            } else {
                LIVE.fetch_sub(l.size() - n, Relaxed);
            }
        }
        q
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// `[live bytes, peak live bytes since the last reset, allocations]`.
#[wasm_bindgen(js_name = memStats)]
pub fn mem_stats() -> Vec<f64> {
    vec![
        LIVE.load(Relaxed) as f64,
        PEAK.load(Relaxed) as f64,
        ALLOCS.load(Relaxed) as f64,
    ]
}

/// Starts a new peak at the live bytes now.
#[wasm_bindgen(js_name = memResetPeak)]
pub fn mem_reset_peak() {
    PEAK.store(LIVE.load(Relaxed), Relaxed);
}
