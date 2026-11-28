//! The heap a run of the call machine holds (#8064).
//!
//! The machine's byte budget bounds what a run holds by bytes: its control state, and every value
//! it keeps while it runs, a `[...]` accumulator's, a `reduce`'s or `foreach`'s state, a leaf's
//! buffered outputs and a binding's included. Sizing those values by walking them would be
//! quadratic wherever a collected value is collected again (a `reduce` whose state nests the
//! last one, `[f]` inside `f`), and would count a value shared with one already held twice. So
//! the bytes are counted where they are allocated: [`MeteredAlloc`], installed as the global
//! allocator by the `succinctly` binary (and by the library's own tests), adds every allocation
//! made on a thread while a run is under way to that thread's count and takes every free off it.
//! A value is counted once however many places share it, and a count costs the same whatever the
//! value holds.
//!
//! While no run is under way anywhere, the allocator only reads one atomic per call. Without it
//! installed, [`Meter::held`] stays at zero, and the machine bounds its control state alone.

#![allow(unsafe_code)] // `GlobalAlloc` is an unsafe trait: each method forwards its own arguments to `System`

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::alloc::System;

/// How many runs are under way, on any thread. While none is, [`MeteredAlloc`] counts nothing.
static RUNS: AtomicUsize = AtomicUsize::new(0);

std::thread_local! {
    /// The bytes this thread allocated, less those it freed, while a run was under way.
    static LIVE: Cell<isize> = const { Cell::new(0) };
    /// The [`LIVE`] at which this thread's outermost run started, and how many runs deep the
    /// thread is: a run nested in another is charged from the outer one's start, so the two
    /// share one budget.
    static OUTER: Cell<(isize, u32)> = const { Cell::new((0, 0)) };
}

/// Whether a run is under way on any thread, so allocations are counted.
#[inline(always)]
fn counting() -> bool {
    RUNS.load(Ordering::Relaxed) != 0
}

/// Count `delta` bytes for this thread.
#[inline(always)]
fn add(delta: isize) {
    // A thread being torn down has no count to keep; nothing it frees then is a run's.
    let _ = LIVE.try_with(|live| live.set(live.get().wrapping_add(delta)));
}

/// The system allocator, counting what a thread holds while a run of the call machine is under
/// way ([`Meter`]). The `succinctly` binary installs it with `#[global_allocator]`.
///
/// Each method is out of line, so an allocation site holds one call, as with the default
/// allocator, and asks whether to count before it allocates: when not, it is the system
/// allocator's call in tail position, one load and a branch more than the default; when so, the
/// counting arm, out of line again.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MeteredAlloc;

impl MeteredAlloc {
    #[cold]
    #[inline(never)]
    unsafe fn counted_alloc(layout: Layout, zeroed: bool) -> *mut u8 {
        // SAFETY: the caller's `layout`, as `GlobalAlloc::alloc` requires of it.
        let ptr = unsafe {
            if zeroed {
                System.alloc_zeroed(layout)
            } else {
                System.alloc(layout)
            }
        };
        if !ptr.is_null() {
            add(layout.size() as isize);
        }
        ptr
    }

    #[cold]
    #[inline(never)]
    unsafe fn counted_dealloc(ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from this allocator, which is `System`'s, with `layout`.
        unsafe { System.dealloc(ptr, layout) };
        add(-(layout.size() as isize));
    }

    #[cold]
    #[inline(never)]
    unsafe fn counted_realloc(ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: as `GlobalAlloc::realloc` requires of its caller.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            add(new_size as isize - layout.size() as isize);
        }
        new
    }
}

// SAFETY: every method forwards to `System` with the arguments it was given and returns what
// `System` returns, so `System`'s contract is this one's; the count is a thread-local `Cell` that
// needs no allocation and has no destructor.
unsafe impl GlobalAlloc for MeteredAlloc {
    #[inline(never)]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if counting() {
            // SAFETY: the caller's `layout`, as `GlobalAlloc::alloc` requires of it.
            return unsafe { Self::counted_alloc(layout, false) };
        }
        // SAFETY: as above.
        unsafe { System.alloc(layout) }
    }

    #[inline(never)]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if counting() {
            // SAFETY: as `alloc`.
            return unsafe { Self::counted_alloc(layout, true) };
        }
        // SAFETY: as `alloc`.
        unsafe { System.alloc_zeroed(layout) }
    }

    #[inline(never)]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if counting() {
            // SAFETY: `ptr` came from this allocator, which is `System`'s, with `layout`.
            return unsafe { Self::counted_dealloc(ptr, layout) };
        }
        // SAFETY: as above.
        unsafe { System.dealloc(ptr, layout) }
    }

    #[inline(never)]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if counting() {
            // SAFETY: as `GlobalAlloc::realloc` requires of its caller.
            return unsafe { Self::counted_realloc(ptr, layout, new_size) };
        }
        // SAFETY: as above.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// A run under way on this thread, from [`Meter::begin`] until it is dropped.
///
/// It keeps the address of this thread's [`LIVE`], so that reading what the run holds is one load
/// rather than a thread-local lookup (on macOS a call per lookup), and the machine can afford to
/// read it at every call, append and store. The raw pointer makes it neither `Send` nor `Sync`,
/// so it stays on the thread whose count it reads, which outlives it.
pub(super) struct Meter {
    /// This thread's [`LIVE`].
    live: *const Cell<isize>,
    /// [`LIVE`] when the thread's outermost run started.
    #[cfg(test)]
    base: isize,
    /// The [`LIVE`] past which the run is over its budget: `base` plus the budget.
    top: isize,
}

impl Meter {
    /// Start counting this thread's allocations for a run whose budget is `budget` bytes.
    pub(super) fn begin(budget: usize) -> Self {
        RUNS.fetch_add(1, Ordering::Relaxed);
        let live: *const Cell<isize> = LIVE.with(|live| live as *const Cell<isize>);
        // SAFETY: `live` is this thread's `LIVE`, which lives as long as the thread.
        let now = unsafe { (*live).get() };
        let base = OUTER.with(|outer| {
            let (base, depth) = outer.get();
            let base = if depth == 0 { now } else { base };
            outer.set((base, depth + 1));
            base
        });
        let top = base.saturating_add(isize::try_from(budget).unwrap_or(isize::MAX));
        Self {
            live,
            #[cfg(test)]
            base,
            top,
        }
    }

    /// This thread's count now.
    #[inline(always)]
    fn live(&self) -> isize {
        // SAFETY: `live` is the `LIVE` of the thread that made this `Meter`, which is the thread
        // reading it (`Meter` is not `Send`) and which outlives it; a `Cell` read while no
        // reference into it is live (the allocator's writes are not reentrant with this one).
        unsafe { (*self.live).get() }
    }

    /// The bytes the budget has left, below zero past it: one load and a subtraction, so the
    /// machine can ask at every call, append and store.
    #[inline(always)]
    pub(super) fn room(&self) -> isize {
        self.top.wrapping_sub(self.live())
    }

    /// The bytes this thread has allocated and not freed since its outermost run started: what
    /// the run holds. Below zero when the run freed more than it allocated (its input, say).
    #[cfg(test)]
    pub(super) fn held(&self) -> isize {
        self.live().wrapping_sub(self.base)
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        OUTER.with(|outer| {
            let (base, depth) = outer.get();
            outer.set((base, depth.saturating_sub(1)));
        });
        RUNS.fetch_sub(1, Ordering::Relaxed);
    }
}
