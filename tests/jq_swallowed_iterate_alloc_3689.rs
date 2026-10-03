//! A `.[]` over a scalar that a `?` or a `try` swallows builds no error (#3689).
//!
//! `.[]` on a number, string, boolean or `null` raises `Cannot iterate over ...`,
//! and the error's message quotes the scalar: formatting it costs a preview
//! string and an owned copy of the value. Wherever the boundary swallows the
//! error that work was thrown away -- 6 allocator calls per scalar for `.[]?`,
//! 21 for `try .[] catch empty`, 6 more per node for `.. | path` (whose `.[]?`
//! step is the same swallow) -- and most nodes of a document are scalars.
//!
//! The assertion is on proportion, as in #3023's `..` test: each swallowing
//! query against a twin that visits the same members and does the legitimate
//! per-member work, on the same input, in the same process -- not an exact
//! count, so it survives allocator and toolchain changes that move the
//! constant. A lower bound on the twin guards the other direction: a counter
//! that stopped counting would otherwise pass every comparison with
//! `0 < 0 + N/4`.
//!
//! Both entry points are measured. The collecting one
//! (`eval_with_cursor_using`) answers a `[...] | length`; the streaming one
//! (`eval_each_with_cursor_using`, what `succinctly jq` drives) reaches the
//! same boundary through `each_try_generic`, a different function.
//!
//! Counting is gated to the calling thread, so the harness's other threads, and
//! other tests in this file, cannot add to the window.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use succinctly::jq::eval_generic::{
    eval_each_with_cursor_using, eval_with_cursor_using, GenericResult,
};
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

/// The same through the streaming entry: allocator calls one evaluation makes
/// while every output is handed to a sink that only counts it, and the number
/// of outputs.
fn allocations_streaming(query: &str, json: &str) -> (usize, usize) {
    let bytes = json.as_bytes();
    let index = JsonIndex::build(bytes);
    let root = index.root(bytes);
    let expr = parse(query).expect("parse");

    let outputs_of = || {
        let mut outputs = 0usize;
        let escaped = eval_each_with_cursor_using::<JqSemantics, _>(&expr, root, &mut |result| {
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
    };

    // Warm-up, as above.
    let expected = outputs_of();

    ALLOCATIONS.with(|count| count.set(Some(0)));
    let outputs = outputs_of();
    let counted = ALLOCATIONS
        .with(|count| count.replace(None))
        .expect("the window was opened above");

    assert_eq!(outputs, expected, "a repeat evaluation must agree");
    (counted, outputs)
}

const N: usize = 2_000;

/// (name, document): every member a scalar the swallowing step must leave
/// alone.
fn fixtures() -> Vec<(&'static str, String)> {
    let join = |parts: Vec<String>| parts.join(",");
    vec![
        // The keyed twin: an owned key `String` per member beside the scalar.
        (
            "object of integers",
            format!(
                "{{{}}}",
                join((0..N).map(|i| format!("\"key{i}\":{i}")).collect())
            ),
        ),
        (
            "array of integers",
            format!("[{}]", join((0..N).map(|i| i.to_string()).collect())),
        ),
        // A string scalar is read to be validated; that must not allocate
        // either, since the boundary throws the result away.
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
        // Booleans and `null` are the other scalar kinds a message quotes.
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

/// `swallowed` must cost what `twin` does, within a quarter of one allocation
/// per member -- a per-member allocation in the swallowing step would add at
/// least `N`.
///
/// The twin may legitimately make none (a streamed `.[]` over an array walks
/// it without allocating), so this is only an upper bound; that the counter
/// counts at all is [`assert_counting`]'s job.
#[track_caller]
fn assert_costs_like_twin(name: &str, swallowed: (&str, usize), twin: (&str, usize)) {
    assert!(
        swallowed.1 < twin.1 + N / 4,
        "{name}: `{}` made {} allocator calls against `{}`'s {} over the same \
         members; a per-member allocation in the swallowed `.[]` would add at \
         least {N}",
        swallowed.0,
        swallowed.1,
        twin.0,
        twin.1
    );
}

/// A lower bound on a query that allocates for every member (`path` builds an
/// array per output), so every comparison above cannot be passing as
/// `0 < 0 + N/4` because the counter stopped counting.
#[track_caller]
fn assert_counting(name: &str, (query, allocations): (&str, usize)) {
    assert!(
        allocations >= N,
        "{name}: `{query}` made {allocations} allocator calls over {N} members, so the \
         counter is not counting"
    );
}

#[test]
fn a_swallowed_iteration_over_a_scalar_allocates_no_more_than_its_twin_3689() {
    for (name, json) in fixtures() {
        let (twin, answered) = allocations_collecting("[.[]] | length", &json);
        assert_eq!(answered, N as i64, "{name}: the twin's answer");

        // `.[]?`, `try .[]` and `try .[] catch empty` all drop the error.
        for query in [
            "[.[] | .[]?] | length",
            "[.[] | try .[]] | length",
            "[.[] | try .[] catch empty] | length",
            // Parenthesised: the same boundary under a `Paren`.
            "[.[] | (.[])?] | length",
        ] {
            let (swallowed, answered) = allocations_collecting(query, &json);
            assert_eq!(
                answered, 0,
                "{name}: `{query}` iterates scalars, so yields nothing"
            );
            assert_costs_like_twin(name, (query, swallowed), ("[.[]] | length", twin));
        }

        // `.. | path` steps every node through that same `.[]?`; its twin does
        // the per-member work `path` itself needs.
        let (path_twin, answered) = allocations_collecting("[.[] | path] | length", &json);
        assert_eq!(answered, N as i64, "{name}: the path twin's answer");
        assert_counting(name, ("[.[] | path] | length", path_twin));
        let (walked, answered) = allocations_collecting("[.. | path] | length", &json);
        assert_eq!(answered, N as i64 + 1, "{name}: the nodes `..` visits");
        assert_costs_like_twin(
            name,
            ("[.. | path] | length", walked),
            ("[.[] | path] | length", path_twin),
        );
    }
}

#[test]
fn a_swallowed_iteration_over_a_scalar_allocates_no_more_than_its_twin_when_streamed_3689() {
    for (name, json) in fixtures() {
        let (twin, outputs) = allocations_streaming(".[]", &json);
        assert_eq!(outputs, N, "{name}: the twin's outputs");

        for query in [".[] | .[]?", ".[] | try .[]", ".[] | try .[] catch empty"] {
            let (swallowed, outputs) = allocations_streaming(query, &json);
            assert_eq!(
                outputs, 0,
                "{name}: `{query}` iterates scalars, so yields nothing"
            );
            assert_costs_like_twin(name, (query, swallowed), (".[]", twin));
        }

        let (path_twin, outputs) = allocations_streaming(".[] | path", &json);
        assert_eq!(outputs, N, "{name}: the path twin's outputs");
        assert_counting(name, (".[] | path", path_twin));
        let (walked, outputs) = allocations_streaming(".. | path", &json);
        assert_eq!(outputs, N + 1, "{name}: the nodes `..` visits");
        assert_costs_like_twin(name, (".. | path", walked), (".[] | path", path_twin));
    }
}

/// The counter can fail: a handler that reads the payload still has to see the
/// `Cannot iterate` message, so `catch .` must keep paying for it. If this
/// stopped holding, the proportion above could be passing because nothing was
/// being counted rather than because nothing was being built.
#[test]
fn a_handler_that_reads_the_error_still_builds_it_3689() {
    let (name, json) = &fixtures()[0];
    let (twin, _) = allocations_collecting("[.[]] | length", json);
    let (observed, answered) = allocations_collecting("[.[] | try .[] catch .] | length", json);
    assert_eq!(
        answered, N as i64,
        "{name}: `catch .` yields one message per scalar"
    );
    assert!(
        observed >= twin + N,
        "{name}: `try .[] catch .` made {observed} allocator calls against the twin's {twin}; \
         formatting each message must cost at least one allocation per member"
    );
}
