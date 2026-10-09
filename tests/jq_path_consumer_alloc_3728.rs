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

#[path = "common/counting_alloc.rs"]
mod counting_alloc;
use counting_alloc::{allocations_collecting, allocations_streaming, Counting};

#[global_allocator]
static ALLOCATOR: Counting = Counting;

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
        // 7 per member on the cheapest fixture since #4217 recycled a scalar
        // bridge document's three index buffers (it was 10 when each crossing
        // allocated them). `allocations_collecting` warms the thread's pool.
        assert!(
            empty + 7 * N <= run,
            "{name}: `catch empty` made {empty} allocator calls and the handler the resolver \
             has to run made {run}; not running it should save at least 7 per member"
        );
        let (streamed, outputs) =
            allocations_streaming(".[] | path(try error(\"x\") catch empty)", &json);
        assert_eq!(outputs, 0, "{name}: streamed");
        let (streamed_run, _) =
            allocations_streaming(".[] | path(try error(\"x\") catch (empty | empty))", &json);
        assert!(
            streamed + 7 * N <= streamed_run,
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

/// A lone leaf in `path(f)` (`//`, `first`, `try`) is resolved by the path
/// resolver, which the cursor walkers decline to (#4155). It cloned the whole
/// expression to flatten it, evaluated a `.` over a scalar to clone it back out,
/// and walked the empty path it resolved to: about 12 allocator calls per scalar
/// for `. // .` against `path(.)`'s 3. Each row is bounded against that twin, and
/// each shortcut has a row that fails with only that shortcut deleted:
///
/// - the lazy flatten: `first(.a?)` and `try .a catch empty` would clone the
///   expression, and `. // .` its two boxes;
/// - the scalar `.`: `. // .` would evaluate and clone the member;
/// - the empty-path short circuit: `. // .` would build a trail and a walk list;
/// - the deferred component clone: `.a?` over a scalar would clone `a` for a
///   lookup that finds nothing.
#[test]
fn a_lone_leaf_in_path_f_is_resolved_without_cloning_or_walking_4155() {
    // Emits the empty path for every member: the twin's walker emits it too, so
    // the allowance is the resolver's own per-call state (a branch list, a prefix
    // root) and the copy of the member it resolves against.
    assert_rows_like_twin(&[(
        "[.[] | path(. // .)] | length",
        "[.[] | path(.)] | length",
        N as i64,
        5,
    )]);

    // `.a` over an integer or a string reaches nothing, so these emit no path and
    // only the lookup's own error is allowed on top. A `null` member would
    // answer `["a"]`, so the booleans-and-nulls document is left out.
    for (name, json) in fixtures().into_iter().take(2) {
        for swallowed in [
            "[.[] | path(first(.a?))] | length",
            "[.[] | path(try .a catch empty)] | length",
            "[.[] | path(.a? // .)] | length",
        ] {
            let (cost, answered) = allocations_collecting(swallowed, &json);
            let reaches = if swallowed.contains("//") {
                N as i64
            } else {
                0
            };
            assert_eq!(answered, reaches, "{name}: `{swallowed}`'s answer");
            let (twin_cost, _) = allocations_collecting("[.[] | path(.)] | length", &json);
            assert_costs_like_twin(
                name,
                (swallowed, cost),
                ("[.[] | path(.)] | length", twin_cost),
                7,
            );
        }
    }
}
