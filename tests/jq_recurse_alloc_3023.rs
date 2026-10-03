//! `..` over a cursor allocates no more per child than `.[]` does (#3023).
//!
//! `each_recurse_cursor_generic` used to reuse the `path()` step for its child
//! enumeration. For every child that built an `Rc` trail link and, for an
//! object, an owned key `String`, then threw the trail away unread; and for
//! every scalar leaf -- most nodes -- it formatted a `Cannot iterate over ...`
//! error that `..`'s own `.[]?` swallowed. A counting allocator over
//! `[..] | length` sees all of that as a term proportional to the document's
//! node count: about ten allocator calls per member of a flat object, against
//! one for the `[.[]] | length` that visits the same members.
//!
//! The assertion is on that proportion -- `..` against its `.[]` twin, on the
//! same input, in the same process -- not on an exact count, so it survives
//! allocator and toolchain changes that move the constant.
//!
//! This file holds one `#[test]` so the counted window shares the process with
//! nothing but the test harness, and counting is gated to the calling thread.

// A counting global allocator is the whole point of this test, and
// `GlobalAlloc` is an unsafe trait. The crate's own `unsafe_code = "deny"`
// policy (Cargo.toml) asks for a localized, reasoned opt-in; nothing here
// ships in the library.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use succinctly::jq::eval_generic::{eval_with_cursor_using, GenericResult};
use succinctly::jq::{parse, JqSemantics, OwnedValue};
use succinctly::json::JsonIndex;

static COUNT: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    // Const-initialized with no destructor, so reading it from inside the
    // allocator never allocates.
    static COUNTING: Cell<bool> = const { Cell::new(false) };
}

struct Counting;

fn note_allocation() {
    // `try_with`: a thread being torn down has no slot left to read.
    if COUNTING.try_with(Cell::get).unwrap_or(false) {
        COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note_allocation();
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Allocator calls one evaluation of `query` makes over `json`, after one
/// warm-up evaluation outside the window, and the length it answered.
fn allocations_of(query: &str, json: &str) -> (usize, i64) {
    let bytes = json.as_bytes();
    let index = JsonIndex::build(bytes);
    let root = index.root(bytes);
    let expr = parse(query).expect("parse");

    // A macro rather than a closure: `DocumentValue` is crate-private, so the
    // result's element type cannot be named from outside.
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

    COUNT.store(0, Ordering::SeqCst);
    COUNTING.with(|c| c.set(true));
    let result = eval_with_cursor_using::<JqSemantics, _>(&expr, root);
    COUNTING.with(|c| c.set(false));
    let counted = COUNT.load(Ordering::SeqCst);

    assert_eq!(answer!(result), expected, "a repeat evaluation must agree");
    (counted, expected)
}

#[test]
fn recurse_allocates_no_more_per_child_than_iterate_3023() {
    const N: usize = 2_000;

    let join = |parts: Vec<String>| parts.join(",");
    let fixtures = [
        // Every member a keyed child: the owned-key `String` and the trail link
        // were each paid N times, and every integer leaf built an error.
        (
            "object of integers",
            format!(
                "{{{}}}",
                join((0..N).map(|i| format!("\"key{i}\":{i}")).collect())
            ),
        ),
        // The array twin: no key `String`, but the link and the leaf error
        // were per child just the same.
        (
            "array of integers",
            format!("[{}]", join((0..N).map(|i| i.to_string()).collect())),
        ),
        // A string leaf is decoded to be validated; that must not allocate
        // either, since the walk throws the result away.
        (
            "object of strings",
            format!(
                "{{{}}}",
                join(
                    (0..N)
                        .map(|i| format!("\"key{i}\":\"value number {i}\""))
                        .collect()
                )
            ),
        ),
    ];

    for (name, json) in &fixtures {
        let (iterate, iterated) = allocations_of("[.[]] | length", json);
        let (recurse, recursed) = allocations_of("[..] | length", json);
        assert_eq!(iterated, N as i64, "{name}: the members");
        assert_eq!(recursed, N as i64 + 1, "{name}: the members and the root");
        assert!(
            recurse < iterate + N / 4,
            "{name}: `[..] | length` made {recurse} allocator calls against \
             `[.[]] | length`'s {iterate} over the same {N} members; a per-child \
             allocation in `..` would add at least {N}"
        );
    }
}
