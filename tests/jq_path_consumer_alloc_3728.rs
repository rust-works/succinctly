//! The consumers of a path expression that swallow a `.[]` over a scalar build
//! nothing for it (#3728).
//!
//! #3689 and #3722 stopped the value boundaries and `path(f)`'s two generic
//! walkers building the `Cannot iterate over ...` message a `?` drops. Four
//! other consumers still did, because each ran the owned evaluator or the path
//! resolver (`eval.rs`) rather than a cursor walk:
//!
//! - `catch empty` in the path resolver seeded its handler with the caught
//!   payload, flattened it, and ran it over a re-indexed copy: about 28
//!   allocator calls per scalar for a handler that delivers nothing;
//! - the resolver's `Expr::Try` and `Expr::Optional` arms stepped `.[]` over a
//!   scalar to format the message they then pruned;
//! - `eval_each_owned`, the bridge `paths(f)`, `any`, `all` and `IN` evaluate
//!   their filter through, re-indexed the scalar before reaching #3689's
//!   shortcut: about 12 per node, twice per node for `paths(f)`.
//!
//! The assertion is on proportion, as in #3689's test: each swallowing query
//! against a twin that visits the same members and does the legitimate
//! per-member work (`path(.)` for a bare `path(f)`, `paths(.)` for `paths(f)`,
//! `. // .` for the alternative), in the same process, with a bound of a stated
//! number of allocator calls per member. The `catch empty` handler is the one
//! exception: its twin is the same body with a handler the resolver has to run
//! (`catch (empty | empty)`), which makes the saving the whole difference. Each
//! shortcut has a row that fails with only that shortcut deleted.
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

/// Documents of `N` scalar members, one of each kind a message quotes.
fn fixtures() -> Vec<(&'static str, String)> {
    let join = |parts: Vec<String>| parts.join(",");
    vec![
        (
            "array of integers",
            format!("[{}]", join((0..N).map(|i| i.to_string()).collect())),
        ),
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

/// `swallowed` must cost what `twin` does, within `allowance_halves` half
/// allocator calls per member: a message built per member would add at least
/// two more, and a handler or a re-index many more.
#[track_caller]
fn assert_costs_like_twin(
    name: &str,
    swallowed: (&str, usize),
    twin: (&str, usize),
    allowance_halves: usize,
) {
    assert!(
        swallowed.1 <= twin.1 + allowance_halves * N / 2,
        "{name}: `{}` made {} allocator calls against `{}`'s {} over the same members \
         (allowing {} per member)",
        swallowed.0,
        swallowed.1,
        twin.0,
        twin.1,
        allowance_halves as f64 / 2.0
    );
}

/// Query pairs through the collecting entry, then the streaming one: the
/// swallowing query must yield `answer` (0 for a boundary that reaches nothing)
/// and cost what the twin does.
#[track_caller]
fn assert_rows_like_twin(rows: &[(&str, &str, i64, usize)]) {
    for (name, json) in fixtures() {
        for &(swallowed, twin, answer, allowance_halves) in rows {
            let (cost, answered) = allocations_collecting(swallowed, &json);
            assert_eq!(answered, answer, "{name}: `{swallowed}`'s answer");
            let (twin_cost, _) = allocations_collecting(twin, &json);
            // The counter counts: the twin makes an allocation for every member.
            assert!(twin_cost >= N, "{name}: `{twin}` made {twin_cost}");
            assert_costs_like_twin(name, (swallowed, cost), (twin, twin_cost), allowance_halves);
        }
    }
}

/// `catch empty` delivers nothing for any payload, so the resolver neither
/// seeds nor runs it. The body is `error("x")`, not `.[]`, so no other
/// shortcut is in play: the twin differs only in a handler (`empty | empty`)
/// that is not the bare `empty` the shortcut matches, and so is run.
#[test]
fn a_catch_empty_handler_in_path_f_is_not_run_3728() {
    for (name, json) in fixtures() {
        let (empty, answered) =
            allocations_collecting("[.[] | path(try error(\"x\") catch empty)] | length", &json);
        assert_eq!(answered, 0, "{name}: the handler delivers nothing");
        let (run, answered) = allocations_collecting(
            "[.[] | path(try error(\"x\") catch (empty | empty))] | length",
            &json,
        );
        assert_eq!(answered, 0, "{name}: the twin's handler delivers nothing");
        assert!(
            empty + 10 * N <= run,
            "{name}: `catch empty` made {empty} allocator calls and the handler the resolver \
             has to run made {run}; not running it should save at least 10 per member"
        );
        let (streamed, outputs) =
            allocations_streaming(".[] | path(try error(\"x\") catch empty)", &json);
        assert_eq!(outputs, 0, "{name}: streamed");
        let (streamed_run, _) =
            allocations_streaming(".[] | path(try error(\"x\") catch (empty | empty))", &json);
        assert!(
            streamed + 10 * N <= streamed_run,
            "{name}: streamed, {streamed} against {streamed_run}"
        );
    }
}

/// The resolver's `Expr::Try` arm answers a swallowed `.[]` over a trackable
/// scalar without formatting the message: it costs what `path(.)` does.
#[test]
fn the_resolvers_try_arm_builds_no_message_for_a_swallowed_iteration_3728() {
    assert_rows_like_twin(&[
        // `path(.)`'s own walker emits one path per member; this reaches none,
        // so it is allowed a few small allocations the walker avoids (the
        // resolver's per-call state), but not a message per member.
        (
            "[.[] | path(try .[] catch empty)] | length",
            "[.[] | path(.)] | length",
            0,
            6,
        ),
        (
            "[.[] | path(try .[])] | length",
            "[.[] | path(.)] | length",
            0,
            6,
        ),
    ]);
}

/// The resolver's `Expr::Optional` arm, reached through `//` and `first`, which
/// `path(f)`'s cursor walkers decline.
#[test]
fn the_resolvers_optional_arm_builds_no_message_for_a_swallowed_iteration_3728() {
    assert_rows_like_twin(&[
        (
            "[.[] | path(first(.[]?))] | length",
            "[.[] | path(.)] | length",
            0,
            6,
        ),
        // The alternative falls back to `.`, so it also emits a path: its twin
        // does the same with a left side that is `.`.
        (
            "[.[] | path(.[]? // .)] | length",
            "[.[] | path(. // .)] | length",
            N as i64,
            3,
        ),
    ]);
}

/// `eval_each_owned` answers a swallowed `.[]` over an owned scalar before the
/// re-index bridge: `paths(f)` evaluates its filter twice per node through it,
/// so it costs what `paths(.)` does.
#[test]
fn eval_each_owned_does_not_reindex_a_scalar_for_a_swallowed_iteration_3728() {
    assert_rows_like_twin(&[
        (
            "[.[] | paths(.[]?)] | length",
            "[.[] | paths(.)] | length",
            0,
            3,
        ),
        (
            "[.[] | paths(try .[] catch empty)] | length",
            "[.[] | paths(.)] | length",
            0,
            6,
        ),
    ]);
}

/// The streaming entry (`succinctly jq` drives it) reaches the same arms.
#[test]
fn the_streaming_entry_reaches_the_same_shortcuts_3728() {
    for (name, json) in fixtures() {
        for (swallowed, twin, allowance_halves) in [
            (".[] | path(try .[] catch empty)", ".[] | path(.)", 6),
            (".[] | path(first(.[]?))", ".[] | path(.)", 6),
            (".[] | paths(.[]?)", ".[] | paths(.)", 3),
        ] {
            let (cost, outputs) = allocations_streaming(swallowed, &json);
            assert_eq!(outputs, 0, "{name}: `{swallowed}` reaches nothing");
            let (twin_cost, _) = allocations_streaming(twin, &json);
            assert_costs_like_twin(name, (swallowed, cost), (twin, twin_cost), allowance_halves);
        }
    }
}
