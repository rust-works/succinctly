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
//! `try path(.[]) catch LITERAL` raises the same error over a scalar and takes
//! the same shortcut (#4280); it is measured beside `.[]` on every route.
//!
//! A handler that reads its input (`catch .`) still builds the payload; it is
//! measured as the lower bound of what the shortcut saves.
//!
//! Counting is gated to the calling thread, so the harness's other threads, and
//! other tests in this file, cannot add to the window.

use succinctly::jq::eval_generic::eval_with_cursor_using;
use succinctly::jq::{parse, JqSemantics, OwnedValue};
use succinctly::json::JsonIndex;

#[path = "common/counting_alloc.rs"]
mod counting_alloc;
use counting_alloc::{allocations_streaming, allocations_value_route, measure, Counting};

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

    // `measure` infers the result's element type (the
    // crate-private `DocumentValue`) from the call, so it is never named here.
    measure(
        || eval_with_cursor_using::<JqSemantics, _>(&expr, root),
        |result| match result.into_owned::<JqSemantics>() {
            Ok(Some(OwnedValue::Int(n))) => n,
            other => panic!("`{query}` did not settle to one number: {other:?}"),
        },
    )
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
                // `path(.[])` raises the same error over a scalar (#4280).
                format!("[.[] | try path(.[]) catch {literal}] | length"),
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
                format!(".[] | try path(.[]) catch {literal}"),
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

            for query in [
                format!("[.[] | try .[] catch {literal}] | length"),
                format!("[.[] | try path(.[]) catch {literal}] | length"),
            ] {
                let (caught, answered) = allocations_value_route(&query, &json);
                assert_eq!(answered, N as i64, "{name}: `{query}` answers the literal");
                assert_costs_like_twin(name, (&query, caught), (&twin_query, twin), allowance);
            }

            // A consumer that pulls the boundary one output at a time reaches
            // `each_try` rather than `eval_try`; its twin is the same consumer
            // over the bare literal.
            let twin_query = format!("[.[] | first({literal})] | length");
            let (twin, answered) = allocations_value_route(&twin_query, &json);
            assert_eq!(answered, N as i64, "{name}: the twin's answer");
            for query in [
                format!("[.[] | first(try .[] catch {literal})] | length"),
                format!("[.[] | first(try path(.[]) catch {literal})] | length"),
            ] {
                let (caught, answered) = allocations_value_route(&query, &json);
                assert_eq!(answered, N as i64, "{name}: `{query}` answers the literal");
                assert_costs_like_twin(name, (&query, caught), (&twin_query, twin), allowance);
            }
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
