//! A thread-gated counting allocator and the measuring helpers the jq
//! allocation tests share (#4161).
//!
//! Included by `tests/jq_recurse_alloc_3023.rs`, `tests/jq_swallowed_iterate_alloc_3689.rs`,
//! `tests/jq_caught_iterate_alloc_3704.rs` and `tests/jq_path_consumer_alloc_3728.rs`
//! via `#[path = "common/counting_alloc.rs"] mod counting_alloc;`. Lives under
//! `tests/common/` because cargo auto-discovers `tests/*.rs` as test binaries
//! but not files in subdirectories.
//!
//! A `#[global_allocator]` can be declared once per test binary, so the
//! allocator *type* lives here and each file keeps its own
//! `#[global_allocator] static ALLOCATOR: Counting = Counting;`. Without that
//! static the counter never moves and every measurement reads 0.
//!
//! Counting is gated to the calling thread, so the harness's other threads, and
//! other tests in the same file, cannot add to a window.

#![allow(dead_code)] // Each consumer uses a different subset.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use succinctly::jq::eval_generic::{eval_each_with_cursor_using, GenericResult};
use succinctly::jq::{eval_full, parse, JqSemantics, OwnedValue, QueryResult};
use succinctly::json::JsonIndex;

thread_local! {
    // `None` while this thread is not counting. Const-initialized with no
    // destructor, so reading it from inside the allocator never allocates.
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

pub struct Counting;

fn note_allocation() {
    // `try_with` so the allocator itself can never panic.
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(n) = count.get() {
            count.set(Some(n + 1));
        }
    });
}

// A counting global allocator is the whole point of these tests, and
// `GlobalAlloc` is an unsafe trait, so the unsafe code is opted back in here
// and only here.
#[allow(unsafe_code)] // STYLE-0004: a test-only counting allocator, forwarding to `System` unchanged.
                      // SAFETY: every method forwards its arguments unchanged to `System`, which upholds
                      // `GlobalAlloc`'s contract; the only addition is a counter bump that neither allocates
                      // nor touches the pointer.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_allocation();
        // SAFETY: `layout` is the caller's, passed straight to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note_allocation();
        // SAFETY: `layout` is the caller's, passed straight to `System`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note_allocation();
        // SAFETY: `ptr`, `layout` and `new_size` are the caller's, passed straight to
        // `System`, and `ptr` came from `System` through this allocator's `alloc`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` and `layout` are the caller's, passed straight to `System`, and
        // `ptr` came from `System` through this allocator's `alloc`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

/// Allocator calls `run` makes on this thread, and what it returned.
///
/// A realloc counts as one call; `dealloc` is not counted, since a term that
/// allocates per node frees per node too. Not re-entrant: a window opened
/// inside another resets it.
pub fn count_allocations<R>(run: impl FnOnce() -> R) -> (usize, R) {
    ALLOCATIONS.with(|count| count.set(Some(0)));
    let result = run();
    let counted = ALLOCATIONS
        .with(|count| count.replace(None))
        .expect("the window was opened above");
    (counted, result)
}

/// Allocator calls one run of `run` makes, after one warm-up run outside the
/// window, and what `answer` made of its result.
///
/// The warm-up keeps lazy statics and one-time table builds out of the term
/// measured; `answer` is applied to both runs, which must agree. `run` is a
/// closure rather than a query so the caller names the entry point and the
/// result's element type (the crate-private `DocumentValue`) is inferred at
/// the call site instead of being named here.
pub fn measure<R, T: PartialEq + std::fmt::Debug>(
    run: impl Fn() -> R,
    answer: impl Fn(R) -> T,
) -> (usize, T) {
    let expected = answer(run());
    let (counted, result) = count_allocations(&run);
    let answered = answer(result);
    assert_eq!(answered, expected, "a repeat evaluation must agree");
    (counted, expected)
}

/// Allocator calls one evaluation of `query` makes over `json` through the
/// streaming entry (`eval_each_with_cursor_using`, what `succinctly jq`
/// drives), while every output is handed to a sink that only counts it, and
/// the number of outputs.
pub fn allocations_streaming(query: &str, json: &str) -> (usize, usize) {
    let bytes = json.as_bytes();
    let index = JsonIndex::build(bytes);
    let root = index.root(bytes);
    let expr = parse(query).expect("parse");

    measure(
        || {
            let mut outputs = 0usize;
            let escaped =
                eval_each_with_cursor_using::<JqSemantics, _>(&expr, root, &mut |result| {
                    match result {
                        GenericResult::Many(v) => outputs += v.len(),
                        GenericResult::ManyCursor(v) => outputs += v.len(),
                        GenericResult::ManyOwned(v) => outputs += v.len(),
                        GenericResult::None => {}
                        GenericResult::Error(e) => panic!("`{query}` failed: {e:?}"),
                        _ => outputs += 1,
                    }
                    true
                });
            assert!(escaped.is_none(), "`{query}` ended early: {escaped:?}");
            outputs
        },
        |outputs| outputs,
    )
}

/// Allocator calls one evaluation of `query` makes through `eval_full`, the
/// value evaluator, and the number it answered. The only entry that reaches
/// `eval.rs`'s `eval_try`/`each_try` for a document.
pub fn allocations_value_route(query: &str, json: &str) -> (usize, i64) {
    let bytes = json.as_bytes();
    let index = JsonIndex::build(bytes);
    let expr = parse(query).expect("parse");

    measure(
        || eval_full::<Vec<u64>, JqSemantics>(&expr, index.root(bytes)),
        |result: QueryResult<'_, Vec<u64>>| match result {
            QueryResult::Owned(OwnedValue::Int(n)) => n,
            _ => panic!("`{query}` did not settle to one number"),
        },
    )
}
