//! `del(.[]?)` and `del(.)` over a scalar member copy no expression tree per
//! member (#4218).
//!
//! Each scalar crossing the owned evaluator's bridge paid ten allocator calls
//! for `del(.[]?)` and eight for `del(.)`. Four of the ten were `Expr` boxes
//! the query never needed a second copy of:
//!
//! - `eval_generic::eval_builtin` rebuilt `Expr::Builtin(builtin.clone())` for
//!   the bridge from a `Builtin` whose enclosing node its dispatcher held
//!   (`builtin_expr` now borrows it);
//! - `builtin_del` copied the path it was handed into a one-element `Vec` for
//!   `delete_at_path`, which only reads it (it now lends the path unless an
//!   alias table is installed).
//!
//! What remains per member is the reindex bridge (text, interest bits, their
//! rank, the two-bit sequence: #4154) and the two decodes of the scalar.
//!
//! The bounds are absolute because the term is the bridge's own: there is no
//! twin query doing the same legitimate work without crossing it. Each is the
//! measured cost, so either fix alone, put back, exceeds it: the integer
//! `del(.[]?)` row reads 8 per member with only one of the two in place, and 10
//! with neither (`del(.)`: 7 and 8).
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

fn fixtures() -> Vec<(&'static str, String)> {
    let join = |parts: Vec<String>| parts.join(",");
    vec![
        (
            "array of integers",
            format!("[{}]", join((0..N).map(|i| i.to_string()).collect())),
        ),
        (
            "array of strings",
            format!(
                "[{}]",
                join((0..N).map(|i| format!("\"value number {i}\"")).collect())
            ),
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
        ),
    ]
}

/// Allocator calls per member a row may make, on each fixture in `fixtures`
/// order: the bridge's four, the scalar's two decodes (an owned string is one
/// more call each) and nothing else. The booleans and nulls need no decode
/// allocation.
const PER_MEMBER: [usize; 3] = [6, 8, 4];

/// Calls independent of the member count (the result array's growth, the
/// warm-up-free lazy tables): 22 measured on the integer rows.
const FIXED: usize = 64;

const ROWS: [&str; 2] = ["[.[] | del(.[]?)] | length", "[.[] | del(.)] | length"];

#[test]
fn del_over_scalar_members_copies_no_expression_tree_4218() {
    for (fixture, (name, json)) in fixtures().into_iter().enumerate() {
        for query in ROWS {
            let (cost, answered) = allocations_collecting(query, &json);
            assert_eq!(answered, N as i64, "{name}: `{query}`'s answer");
            let bound = PER_MEMBER[fixture];
            assert!(
                cost <= bound * N + FIXED,
                "{name}: `{query}` made {cost} allocator calls over {N} members \
                 (allowing {bound} per member and {FIXED} in all)"
            );
        }
    }
}
