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
//! Three routes are measured, each reaching a different copy of the boundary.
//! The collecting entry (`eval_with_cursor_using`) answers a `[...] | length`
//! through `try_single_generic`; the streaming one
//! (`eval_each_with_cursor_using`, what `succinctly jq` drives) through
//! `each_try_generic`; and `eval_full`, the value evaluator, through `eval_try`
//! and `each_try` in `eval.rs`, which the other two never reach for a document
//! cursor, so only a row here fails if those shortcuts are deleted.
//!
//! A string with a backslash in it is the one scalar that costs something: it
//! has to be decoded to be validated, which allocates once per member (twice in
//! `.. | path`, whose leaf check decodes it for `scalar_iteration_precheck`
//! and again for `validate_cursor`). Those fixtures carry that allowance; the
//! rest must cost nothing per member.
//!
//! Counting is gated to the calling thread, so the harness's other threads, and
//! other tests in this file, cannot add to the window.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use succinctly::jq::eval_generic::{
    eval_each_with_cursor_using, eval_with_cursor_using, GenericResult,
};
use succinctly::jq::{eval_full, parse, JqSemantics, OwnedValue, QueryResult};
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

/// A document of `N` scalar members, and what its scalars cost to validate.
struct Fixture {
    name: &'static str,
    json: String,
    /// Allocations per member the swallowing boundary may keep: 0, or 1 for a
    /// string that has to be decoded (an escape).
    boundary_allowance: usize,
    /// The same for `.. | path`, whose leaf check decodes a string twice.
    walk_allowance: usize,
}

fn fixtures() -> Vec<Fixture> {
    let join = |parts: Vec<String>| parts.join(",");
    let plain = |name, json| Fixture {
        name,
        json,
        boundary_allowance: 0,
        walk_allowance: 0,
    };
    vec![
        // The keyed twin: an owned key `String` per member beside the scalar.
        plain(
            "object of integers",
            format!(
                "{{{}}}",
                join((0..N).map(|i| format!("\"key{i}\":{i}")).collect())
            ),
        ),
        plain(
            "array of integers",
            format!("[{}]", join((0..N).map(|i| i.to_string()).collect())),
        ),
        // A string scalar is read to be validated; without an escape that must
        // not allocate either, since the boundary throws the result away.
        plain(
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
        plain(
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
        // An escape makes the string decode to be validated: one allocation per
        // member, not the eight the message cost.
        Fixture {
            name: "object of escaped strings",
            json: format!(
                "{{{}}}",
                join(
                    (0..N)
                        .map(|i| format!("\"key{i}\":\"line\\nbreak {i}\""))
                        .collect()
                )
            ),
            boundary_allowance: 1,
            walk_allowance: 2,
        },
    ]
}

/// `swallowed` must cost what `twin` does, within a quarter of one allocation
/// per member plus `allowance` per member -- a per-member allocation in the
/// swallowing step would add at least `N`.
///
/// The twin may legitimately make none (a streamed `.[]` over an array walks
/// it without allocating), so this is only an upper bound; that the counter
/// counts at all is [`assert_counting`]'s job.
#[track_caller]
fn assert_costs_like_twin(
    name: &str,
    swallowed: (&str, usize),
    twin: (&str, usize),
    allowance: usize,
) {
    assert!(
        swallowed.1 < twin.1 + allowance * N + N / 4,
        "{name}: `{}` made {} allocator calls against `{}`'s {} over the same \
         members (allowing {allowance} per member); a per-member allocation in the \
         swallowed `.[]` would add at least {N} more",
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

/// Allocator calls one evaluation of `query` makes through `eval_full`, the
/// value evaluator, and the number it answered. The only entry that reaches
/// `eval.rs`'s `eval_try`/`each_try` for a document.
fn allocations_value_route(query: &str, json: &str) -> (usize, i64) {
    let bytes = json.as_bytes();
    let index = JsonIndex::build(bytes);
    let expr = parse(query).expect("parse");

    let answer = |result: QueryResult<'_, Vec<u64>>| match result {
        QueryResult::Owned(OwnedValue::Int(n)) => n,
        _ => panic!("`{query}` did not settle to one number"),
    };

    // Warm-up, as above.
    let expected = answer(eval_full::<Vec<u64>, JqSemantics>(&expr, index.root(bytes)));

    ALLOCATIONS.with(|count| count.set(Some(0)));
    let result = eval_full::<Vec<u64>, JqSemantics>(&expr, index.root(bytes));
    let counted = ALLOCATIONS
        .with(|count| count.replace(None))
        .expect("the window was opened above");

    assert_eq!(answer(result), expected, "a repeat evaluation must agree");
    (counted, expected)
}

#[test]
fn a_swallowed_iteration_over_a_scalar_allocates_no_more_than_its_twin_3689() {
    for Fixture {
        name,
        json,
        boundary_allowance,
        walk_allowance,
    } in fixtures()
    {
        let (twin, answered) = allocations_collecting("[.[]] | length", &json);
        assert_eq!(answered, N as i64, "{name}: the twin's answer");

        // `.[]?`, `try .[]` and `try .[] catch empty` all drop the error.
        for query in [
            "[.[] | .[]?] | length",
            "[.[] | try .[]] | length",
            "[.[] | try .[] catch empty] | length",
            // Parenthesised: the same boundary under a `Paren`.
            "[.[] | (.[])?] | length",
            // A closure argument reaches the boundary as `Expr::Shared`.
            "def opt(f): f?; [.[] | opt(.[])] | length",
            "def safe(f): try f catch empty; [.[] | safe(.[])] | length",
        ] {
            let (swallowed, answered) = allocations_collecting(query, &json);
            assert_eq!(
                answered, 0,
                "{name}: `{query}` iterates scalars, so yields nothing"
            );
            assert_costs_like_twin(
                name,
                (query, swallowed),
                ("[.[]] | length", twin),
                boundary_allowance,
            );
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
            walk_allowance,
        );

        // The same `.[]?` written out in front of a path-context read: nothing
        // flows past it, so it costs what visiting the members' positions does
        // (the path twin), and no more.
        for query in [
            "[.[] | .[]? | key] | length",
            "[.[] | try .[] catch empty | path] | length",
        ] {
            let (swallowed, answered) = allocations_collecting(query, &json);
            assert_eq!(answered, 0, "{name}: `{query}` yields nothing");
            assert_costs_like_twin(
                name,
                (query, swallowed),
                ("[.[] | path] | length", path_twin),
                boundary_allowance,
            );
        }
    }
}

#[test]
fn a_swallowed_iteration_over_a_scalar_allocates_no_more_than_its_twin_when_streamed_3689() {
    for Fixture {
        name,
        json,
        boundary_allowance,
        walk_allowance,
    } in fixtures()
    {
        let (twin, outputs) = allocations_streaming(".[]", &json);
        assert_eq!(outputs, N, "{name}: the twin's outputs");

        for query in [
            ".[] | .[]?",
            ".[] | try .[]",
            ".[] | try .[] catch empty",
            // The parenthesised boundary reaches `each_try_generic` as a
            // `Paren`, a different dispatch from the collecting entry's.
            ".[] | (.[])?",
            "def opt(f): f?; .[] | opt(.[])",
        ] {
            let (swallowed, outputs) = allocations_streaming(query, &json);
            assert_eq!(
                outputs, 0,
                "{name}: `{query}` iterates scalars, so yields nothing"
            );
            assert_costs_like_twin(name, (query, swallowed), (".[]", twin), boundary_allowance);
        }

        let (path_twin, outputs) = allocations_streaming(".[] | path", &json);
        assert_eq!(outputs, N, "{name}: the path twin's outputs");
        assert_counting(name, (".[] | path", path_twin));
        let (walked, outputs) = allocations_streaming(".. | path", &json);
        assert_eq!(outputs, N + 1, "{name}: the nodes `..` visits");
        assert_costs_like_twin(
            name,
            (".. | path", walked),
            (".[] | path", path_twin),
            walk_allowance,
        );
    }
}

/// `eval_full` is the value evaluator, whose `eval_try`/`each_try` carry their
/// own copy of the shortcut. Nothing else in this file reaches them for a
/// document cursor, so deleting either shortcut would leave every other row
/// green and the output unchanged; this is the row that fails.
#[test]
fn a_swallowed_iteration_over_a_scalar_allocates_no_more_than_its_twin_on_the_value_route_3689() {
    for Fixture {
        name,
        json,
        boundary_allowance,
        ..
    } in fixtures()
    {
        let (twin, answered) = allocations_value_route("[.[]] | length", &json);
        assert_eq!(answered, N as i64, "{name}: the twin's answer");
        // The twin may make next to nothing here; `tostring` builds a string per
        // member, so the counter is shown to count on this route.
        let (live, _) = allocations_value_route("[.[] | tostring] | length", &json);
        assert_counting(name, ("[.[] | tostring] | length", live));

        for query in [
            "[.[] | .[]?] | length",
            "[.[] | try .[] catch empty] | length",
            "[.[] | (.[])?] | length",
        ] {
            let (swallowed, answered) = allocations_value_route(query, &json);
            assert_eq!(answered, 0, "{name}: `{query}` iterates scalars");
            assert_costs_like_twin(
                name,
                (query, swallowed),
                ("[.[]] | length", twin),
                boundary_allowance,
            );
        }
    }
}

/// `each_try` in `eval.rs` is the push-model twin of `eval_try`, and the only
/// document-free way to reach it from here is a pipeline over an owned literal
/// (`[range(N)] | .[] | .[]?`): the generic evaluator hands that to the value
/// evaluator whole. The remaining cost there is the reindex bridge, which has no
/// twin that does not also swallow, so this is a differential instead: the same
/// pipeline with a boundary the shortcut does not match (`(.[] | empty)?` is a
/// pipe, not a bare `.[]`) formats the message for every scalar, and must cost
/// at least the message term more than the one it does. With the shortcut
/// disabled the two cost the same and this fails.
#[test]
fn the_owned_streaming_route_saves_the_message_3689() {
    for (matched, unmatched) in [
        (
            format!("[range({N})] | .[] | .[]?"),
            format!("[range({N})] | .[] | (.[] | empty)?"),
        ),
        (
            format!("[range({N})] | .[] | try .[] catch empty"),
            format!("[range({N})] | .[] | try (.[] | empty) catch empty"),
        ),
    ] {
        let (with_shortcut, outputs) = allocations_streaming(&matched, "[]");
        assert_eq!(
            outputs, 0,
            "`{matched}` iterates scalars, so yields nothing"
        );
        let (without, outputs) = allocations_streaming(&unmatched, "[]");
        assert_eq!(outputs, 0, "`{unmatched}` yields nothing");
        assert!(
            without >= with_shortcut + 4 * N,
            "`{matched}` made {with_shortcut} allocator calls against `{unmatched}`'s {without}: \
             the message the boundary drops cost at least 4 more per scalar before the shortcut"
        );
    }
}

/// The counter can fail: a handler that reads the payload still has to see the
/// `Cannot iterate` message, so `catch .` must keep paying for it. If this
/// stopped holding, the proportion above could be passing because nothing was
/// being counted rather than because nothing was being built.
#[test]
fn a_handler_that_reads_the_error_still_builds_it_3689() {
    let fixture = &fixtures()[0];
    let (twin, _) = allocations_collecting("[.[]] | length", &fixture.json);
    let (observed, answered) =
        allocations_collecting("[.[] | try .[] catch .] | length", &fixture.json);
    assert_eq!(
        answered, N as i64,
        "{}: `catch .` yields one message per scalar",
        fixture.name
    );
    assert!(
        observed >= twin + N,
        "{}: `try .[] catch .` made {observed} allocator calls against the twin's {twin}; \
         formatting each message must cost at least one allocation per member",
        fixture.name
    );
}
