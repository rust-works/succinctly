//! A `.[]` over a scalar that a `try` catches with a constant handler builds no
//! error (#3704).
//!
//! `try .[] catch "x"` over a scalar raised `Cannot iterate over ...`, whose
//! message quotes the scalar (a preview string and an owned copy of the value),
//! only to hand it to a handler that never reads its input: 8 allocator calls
//! per scalar over what the handler's own answer costs. A literal handler
//! (`"x"`, `1`, `null`, ...) answers itself, so the boundary now costs what
//! emitting that literal does.
//!
//! The assertion is on proportion, as in #3689's test of the swallowing
//! boundary: each query against a twin that visits the same members and emits
//! the same literal without a `try`, in the same process, with a lower bound on
//! a query that allocates for every member so a counter that stopped counting
//! cannot pass every comparison as `0 < 0 + N/4`.
//!
//! The same three routes are measured, since each reaches a different copy of
//! the boundary: the collecting entry (`try_single_generic`), the streaming one
//! (`each_try_generic`), and `eval_full` (`eval_try`/`each_try` in `eval.rs`,
//! which the other two never reach for a document cursor, so only a row here
//! fails if those shortcuts are deleted).
//!
//! A handler that reads its input (`catch .`) still builds the payload; it is
//! measured as the lower bound of what the shortcut saves.
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
            match $result.into_owned::<JqSemantics>() {
                Ok(Some(OwnedValue::Int(n))) => n,
                other => panic!("`{query}` did not settle to one number: {other:?}"),
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
    /// Allocations per member the boundary may keep: 0, or 1 for a string that
    /// has to be decoded (an escape).
    allowance: usize,
}

fn fixtures() -> Vec<Fixture> {
    let join = |parts: Vec<String>| parts.join(",");
    vec![
        Fixture {
            name: "object of integers",
            json: format!(
                "{{{}}}",
                join((0..N).map(|i| format!("\"key{i}\":{i}")).collect())
            ),
            allowance: 0,
        },
        Fixture {
            name: "array of integers",
            json: format!("[{}]", join((0..N).map(|i| i.to_string()).collect())),
            allowance: 0,
        },
        Fixture {
            name: "object of strings",
            json: format!(
                "{{{}}}",
                join(
                    (0..N)
                        .map(|i| format!("\"key{i}\":\"value number {i}\""))
                        .collect()
                )
            ),
            allowance: 0,
        },
        Fixture {
            name: "array of booleans and nulls",
            json: format!(
                "[{}]",
                join(
                    (0..N)
                        .map(|i| ["true", "false", "null"][i % 3].to_string())
                        .collect()
                )
            ),
            allowance: 0,
        },
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
            allowance: 1,
        },
    ]
}

/// The literal handlers, each with the twin that emits the same literal for
/// every member: a string, a number, `null`, and a constant that is neither.
const LITERALS: [&str; 4] = ["\"x\"", "1", "null", "false"];

/// `caught` must cost what `twin` does, within a quarter of one allocation
/// per member plus `allowance` per member -- a per-member allocation in the
/// swallowing step would add at least `N`.
///
/// The twin may legitimately make none (a streamed `.[]` over an array walks
/// it without allocating), so this is only an upper bound; that the counter
/// counts at all is [`assert_counting`]'s job.
#[track_caller]
fn assert_costs_like_twin(
    name: &str,
    caught: (&str, usize),
    twin: (&str, usize),
    allowance: usize,
) {
    assert!(
        caught.1 < twin.1 + allowance * N + N / 4,
        "{name}: `{}` made {} allocator calls against `{}`'s {} over the same \
         members (allowing {allowance} per member); a per-member allocation in the \
         caught `.[]` would add at least {N} more",
        caught.0,
        caught.1,
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
fn a_caught_iteration_over_a_scalar_allocates_no_more_than_its_twin_3704() {
    for Fixture {
        name,
        json,
        allowance,
    } in fixtures()
    {
        let (path_twin, _) = allocations_collecting("[.[] | path] | length", &json);
        assert_counting(name, ("[.[] | path] | length", path_twin));
        for literal in LITERALS {
            let twin_query = format!("[.[] | {literal}] | length");
            let (twin, answered) = allocations_collecting(&twin_query, &json);
            assert_eq!(answered, N as i64, "{name}: the twin's answer");

            for query in [
                format!("[.[] | try .[] catch {literal}] | length"),
                // Parenthesised, and a closure argument (`Expr::Shared`).
                format!("[.[] | (try (.[]) catch {literal})] | length"),
            ] {
                let (caught, answered) = allocations_collecting(&query, &json);
                assert_eq!(answered, N as i64, "{name}: `{query}` answers the literal");
                assert_costs_like_twin(name, (&query, caught), (&twin_query, twin), allowance);
            }
        }

        // A handler that reads its input still builds the payload: the cost the
        // shortcut removes, so this is the discriminating direction.
        let (reads_input, answered) =
            allocations_collecting("[.[] | try .[] catch .] | length", &json);
        assert_eq!(answered, N as i64);
        let (twin, _) = allocations_collecting("[.[] | \"x\"] | length", &json);
        assert!(
            reads_input >= twin + N,
            "{name}: `catch .` made {reads_input} allocator calls against the twin's {twin}; \
             the payload it is handed should cost at least one allocation per member"
        );
    }
}

#[test]
fn a_caught_iteration_over_a_scalar_allocates_no_more_than_its_twin_when_streamed_3704() {
    for Fixture {
        name,
        json,
        allowance,
    } in fixtures()
    {
        let (path_twin, _) = allocations_streaming(".[] | path", &json);
        assert_counting(name, (".[] | path", path_twin));
        for literal in LITERALS {
            let twin_query = format!(".[] | {literal}");
            let (twin, outputs) = allocations_streaming(&twin_query, &json);
            assert_eq!(outputs, N, "{name}: the twin's outputs");

            for query in [
                format!(".[] | try .[] catch {literal}"),
                format!(".[] | (try (.[]) catch {literal})"),
            ] {
                let (caught, outputs) = allocations_streaming(&query, &json);
                assert_eq!(outputs, N, "{name}: `{query}` answers the literal");
                assert_costs_like_twin(name, (&query, caught), (&twin_query, twin), allowance);
            }
        }
    }
}

#[test]
fn a_caught_iteration_over_a_scalar_allocates_no_more_than_its_twin_on_the_value_route_3704() {
    for Fixture {
        name,
        json,
        allowance,
    } in fixtures()
    {
        let (path_twin, _) = allocations_value_route("[.[] | path] | length", &json);
        assert_counting(name, ("[.[] | path] | length", path_twin));
        for literal in LITERALS {
            let twin_query = format!("[.[] | {literal}] | length");
            let (twin, answered) = allocations_value_route(&twin_query, &json);
            assert_eq!(answered, N as i64, "{name}: the twin's answer");

            let query = format!("[.[] | try .[] catch {literal}] | length");
            let (caught, answered) = allocations_value_route(&query, &json);
            assert_eq!(answered, N as i64, "{name}: `{query}` answers the literal");
            assert_costs_like_twin(name, (&query, caught), (&twin_query, twin), allowance);

            // A consumer that pulls the boundary one output at a time reaches
            // `each_try` rather than `eval_try`; its twin is the same consumer
            // over the bare literal.
            let twin_query = format!("[.[] | first({literal})] | length");
            let query = format!("[.[] | first(try .[] catch {literal})] | length");
            let (twin, answered) = allocations_value_route(&twin_query, &json);
            assert_eq!(answered, N as i64, "{name}: the twin's answer");
            let (caught, answered) = allocations_value_route(&query, &json);
            assert_eq!(answered, N as i64, "{name}: `{query}` answers the literal");
            assert_costs_like_twin(name, (&query, caught), (&twin_query, twin), allowance);
        }
    }
}

/// A closure argument reaches the boundary as `Expr::Shared`; the twin is the
/// same definition returning the literal without evaluating its argument, so
/// both pay the call's own allocations and only the caught `.[]` differs.
#[test]
fn a_caught_iteration_through_a_closure_argument_allocates_no_more_than_its_twin_3704() {
    for Fixture {
        name,
        json,
        allowance,
    } in fixtures()
    {
        for literal in LITERALS {
            let twin_query = format!("def konst(f): {literal}; [.[] | konst(.[])] | length");
            let query = format!("def safe(f): try f catch {literal}; [.[] | safe(.[])] | length");
            let (twin, answered) = allocations_collecting(&twin_query, &json);
            assert_eq!(answered, N as i64, "{name}: the twin's answer");
            let (caught, answered) = allocations_collecting(&query, &json);
            assert_eq!(answered, N as i64, "{name}: `{query}` answers the literal");
            assert_costs_like_twin(name, (&query, caught), (&twin_query, twin), allowance);

            let (twin, answered) = allocations_value_route(&twin_query, &json);
            assert_eq!(answered, N as i64, "{name}: the twin's answer");
            let (caught, answered) = allocations_value_route(&query, &json);
            assert_eq!(answered, N as i64, "{name}: `{query}` answers the literal");
            assert_costs_like_twin(name, (&query, caught), (&twin_query, twin), allowance);

            let twin_query = format!("def konst(f): {literal}; .[] | konst(.[])");
            let query = format!("def safe(f): try f catch {literal}; .[] | safe(.[])");
            let (twin, outputs) = allocations_streaming(&twin_query, &json);
            assert_eq!(outputs, N, "{name}: the twin's outputs");
            let (caught, outputs) = allocations_streaming(&query, &json);
            assert_eq!(outputs, N, "{name}: `{query}` answers the literal");
            assert_costs_like_twin(name, (&query, caught), (&twin_query, twin), allowance);
        }
    }
}
