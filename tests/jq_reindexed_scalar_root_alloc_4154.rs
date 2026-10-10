//! A scalar the owned evaluator re-indexes costs a handful of allocator calls,
//! not sixteen (#4154), and in steady state one for the bridge (#4217).
//!
//! `OwnedValue::reindexed` wrote a one-token document out and ran the general
//! index build over it, whose balanced-parentheses directories alone are eight
//! `Vec`s. A scalar root's index follows from its length (`JsonIndex::
//! build_reindex_scalar`, `BalancedParens::leaf`), so a bridge crossing was the
//! text, the interest bits, their rank and the two-bit sequence: four calls.
//! #4217 recycles the last three from the document the previous crossing
//! dropped, leaving the text.
//!
//! Queries that cross the bridge once per scalar member (`del(.)`, `paths(.)`,
//! `del(.[]?)`) over documents of `N` members must stay under a per-member
//! ceiling of allocator calls, set one above what each row measures. The
//! ceilings are absolute because the term is the bridge's own: there is no
//! twin query that does the same legitimate work without crossing it. With
//! `build_reindex_scalar` replaced by `build_reindex` the integer rows read
//! 16-18 per member, and with the buffers not recycled every row reads three
//! more than it may; either fails this test.
//!
//! Counting is gated to the calling thread, so the harness's other threads, and
//! other tests in this file, cannot add to the window.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use succinctly::jq::eval_generic::{eval_with_cursor_using, GenericResult};
use succinctly::jq::{parse, JqSemantics, OwnedValue};
use succinctly::json::JsonIndex;

thread_local! {
    // `None` while this thread is not counting. Const-initialized with no
    // destructor, so reading it from inside the allocator never allocates.
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

struct Counting;

fn note_allocation() {
    // `try_with` so the allocator itself can never panic.
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(n) = count.get() {
            count.set(Some(n + 1));
        }
    });
}

// A counting global allocator is the whole point of this test, and
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

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Allocator calls one evaluation of `query` makes over `json` through the
/// collecting entry, after one warm-up evaluation outside the window, and the
/// number it answered.
///
/// A realloc counts as one call; `dealloc` is not counted, since a term that
/// allocates per node frees per node too.
fn allocations_collecting(query: &str, json: &str) -> (usize, i64) {
    let bytes = json.as_bytes();
    let index = JsonIndex::build(bytes);
    let root = index.root(bytes);
    let expr = parse(query).expect("parse");

    // A macro, so the result's element type (the crate-private `DocumentValue`)
    // is inferred at each use instead of being named here.
    macro_rules! answer {
        ($result:expr) => {
            match $result {
                GenericResult::Owned(OwnedValue::Int(n)) => n,
                _ => panic!("`{query}` did not settle to one number"),
            }
        };
    }

    // Warm-up: lazy statics and one-time table builds are not the term measured.
    let expected = answer!(eval_with_cursor_using::<JqSemantics, _>(&expr, root));

    ALLOCATIONS.with(|count| count.set(Some(0)));
    let result = eval_with_cursor_using::<JqSemantics, _>(&expr, root);
    let counted = ALLOCATIONS
        .with(|count| count.replace(None))
        .expect("the window was opened above");

    assert_eq!(answer!(result), expected, "a repeat evaluation must agree");
    (counted, expected)
}

const N: usize = 2_000;

/// Allocator calls per member each row may make, in `ROWS` order, one above
/// what it measures (5/7/5 on integers, 7/9/7 on strings -- the owned string is
/// its own allocation -- and 3/5/2 on booleans and nulls). Before #4217 every
/// row read three higher; before #4154 integers made 16-18.
type Ceilings = [usize; 3];

fn fixtures() -> Vec<(&'static str, String, Ceilings)> {
    let join = |parts: Vec<String>| parts.join(",");
    vec![
        (
            "array of integers",
            format!("[{}]", join((0..N).map(|i| i.to_string()).collect())),
            [6, 8, 6],
        ),
        (
            "array of strings",
            format!(
                "[{}]",
                join((0..N).map(|i| format!("\"value number {i}\"")).collect())
            ),
            [8, 10, 8],
        ),
        (
            "array of booleans and nulls",
            format!(
                "[{}]",
                join(
                    (0..N)
                        .map(|i| ["true", "false", "null"][i % 3].to_string())
                        .collect()
                )
            ),
            [4, 6, 3],
        ),
    ]
}

#[test]
fn a_scalar_crossing_the_reindex_bridge_allocates_a_handful_of_times_4154() {
    let rows = [
        ("[.[] | del(.)] | length", N as i64),
        ("[.[] | del(.[]?)] | length", N as i64),
        ("[.[] | paths(.)] | length", 0),
    ];
    for (name, json, ceilings) in fixtures() {
        for ((query, answer), ceiling) in rows.into_iter().zip(ceilings) {
            let (cost, answered) = allocations_collecting(query, &json);
            assert_eq!(answered, answer, "{name}: `{query}`'s answer");
            assert!(
                cost <= ceiling * N,
                "{name}: `{query}` made {cost} allocator calls over {N} members \
                 (allowing {ceiling} per member)"
            );
        }
    }
}
