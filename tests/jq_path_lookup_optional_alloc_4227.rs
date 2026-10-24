//! `.a?` / `.[N]?` over a scalar inside `path(f)` builds no `Cannot index ...`
//! message the `?` drops (#4227).
//!
//! #3728 settled a bare `.[]?` over a scalar without stepping it; a `Field` or
//! `Index` lookup was still evaluated first, so the `Cannot index number with
//! "a"` message (a preview and an owned copy of the scalar) was formatted for
//! the `?` to discard, one allocator call per member. The owned path resolver now
//! settles a non-null scalar under a `?`/`try` over a bare lookup the way it
//! settles `.[]?`.
//!
//! The assertion is on proportion, as in #3728's and #4157's tests: each
//! swallowing query against a twin that visits the same members and does the same
//! legitimate per-member work (`path(.[]?)`, whose shortcut is #3728's), in the
//! same process. A built message adds at least two allocator calls per member, so
//! half a call of slack catches it coming back.
//!
//! Counting is gated to the calling thread, so the harness's other threads, and
//! other tests in this file, cannot add to the window.

#[path = "common/counting_alloc.rs"]
mod counting_alloc;
use counting_alloc::{allocations_collecting, allocations_streaming, Counting};

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const N: usize = 2_000;

/// Documents of `N` non-null scalar members, one of each kind a message quotes.
/// `null` is left out on purpose: `null | .a` is `null`, not an error, so a
/// lookup over it has a path to deliver and no twin to compare against.
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
            "array of booleans",
            format!(
                "[{}]",
                join(
                    (0..N)
                        .map(|i| ["true", "false"][i % 2].to_string())
                        .collect()
                )
            ),
        ),
    ]
}

/// `swallowed` must cost what `twin` does, within half an allocator call per
/// member.
#[track_caller]
fn assert_costs_like_twin(name: &str, swallowed: (&str, usize), twin: (&str, usize)) {
    assert!(
        swallowed.1 <= twin.1 + N / 2,
        "{name}: `{}` made {} allocator calls against `{}`'s {} over the same members \
         (allowing half a call per member)",
        swallowed.0,
        swallowed.1,
        twin.0,
        twin.1,
    );
}

/// The collecting entry: every spelling of the boundary reaches nothing and
/// costs what the settled `path(.[]?)` does.
#[test]
fn a_swallowed_scalar_lookup_builds_no_message_4227() {
    for (name, json) in fixtures() {
        let twin = "[.[] | path(.[]?)] | length";
        let (twin_cost, _) = allocations_collecting(twin, &json);
        // The counter counts: the twin allocates for every member.
        assert!(twin_cost >= N, "{name}: the twin made {twin_cost}");
        for (swallowed, twin) in [
            ("[.[] | path(.a?)] | length", twin),
            ("[.[] | path(.[0]?)] | length", twin),
            ("[.[] | path((.a)?)] | length", twin),
            ("[.[] | path(try .a)] | length", twin),
            ("[.[] | path(try .a catch empty)] | length", twin),
            ("[.[] | path(try (.[0]) catch empty)] | length", twin),
            // (A closure boundary, `def opt(f): f?; path(opt(.a))`, runs on the
            // path-mode interpreter since #5223, which still formats the message
            // the `?` drops.)
        ] {
            let (twin_cost, _) = allocations_collecting(twin, &json);
            let (cost, answered) = allocations_collecting(swallowed, &json);
            assert_eq!(answered, 0, "{name}: `{swallowed}` reaches nothing");
            assert_costs_like_twin(name, (swallowed, cost), (twin, twin_cost));
        }
    }
}

/// The `//` alternative is where the row appears in real queries: the swallowed
/// lookup falls through to `.`, and the row costs what the settled twin does.
#[test]
fn an_alternative_over_a_swallowed_lookup_costs_like_its_twin_4227() {
    for (name, json) in fixtures() {
        let twin = "[.[] | path(.[]? // .)] | length";
        let (twin_cost, twin_answered) = allocations_collecting(twin, &json);
        assert_eq!(twin_answered, N as i64, "{name}: the twin reaches `.`");
        for swallowed in [
            "[.[] | path(.a? // .)] | length",
            "[.[] | path(.[0]? // .)] | length",
            "[.[] | path((try .a catch empty) // .)] | length",
        ] {
            let (cost, answered) = allocations_collecting(swallowed, &json);
            assert_eq!(answered, N as i64, "{name}: `{swallowed}` falls through");
            assert_costs_like_twin(name, (swallowed, cost), (twin, twin_cost));
        }
    }
}

/// The streaming entry (`succinctly jq` drives it) reaches the same shortcut. A `try` over a
/// bare lookup or `.[]`, with no handler or `catch empty`, is `E?` and takes the same cursor walk
/// (#4412), so it costs what `path(.[]?)` does; before, it took the materializing route and cost
/// two calls per member (a copy of the member and the `Rc` around it, neither of them a message).
#[test]
fn the_streaming_entry_reaches_the_same_shortcut_4227() {
    for (name, json) in fixtures() {
        for (swallowed, twin) in [
            (".[] | path(.a?)", ".[] | path(.[]?)"),
            (".[] | path(.[0]?)", ".[] | path(.[]?)"),
            (".[] | path((.a)?)", ".[] | path(.[]?)"),
            (".[] | path(try .a catch empty)", ".[] | path(.[]?)"),
            (".[] | path(try .a)", ".[] | path(.[]?)"),
            (".[] | path(try .[0])", ".[] | path(.[]?)"),
            (".[] | path(try .[])", ".[] | path(.[]?)"),
            (".[] | path(try .[] catch empty)", ".[] | path(.[]?)"),
            // (The closure boundary is left out here too, as above.)
        ] {
            let (twin_cost, _) = allocations_streaming(twin, &json);
            let (cost, outputs) = allocations_streaming(swallowed, &json);
            assert_eq!(outputs, 0, "{name}: `{swallowed}` reaches nothing");
            assert_costs_like_twin(name, (swallowed, cost), (twin, twin_cost));
        }
    }
}

/// What the shortcut must not touch, with jq 1.7.1's answers over
/// `[1, null, "s", {"a": 1}, [2], true]`: `null` indexes to `null` (a path), a
/// slice of a string is valid, a container is looked up as ever, and a `catch`
/// that runs still runs.
#[test]
fn the_shortcut_leaves_everything_else_to_the_step_4227() {
    let json = r#"[1, null, "s", {"a": 1}, [2], true]"#;
    let answer = |query: &str| allocations_collecting(query, json).1;
    // `null`, and an object that has the key.
    assert_eq!(answer("[.[] | path(.a?)] | length"), 2);
    // `null`, and the array.
    assert_eq!(answer("[.[] | path(.[0]?)] | length"), 2);
    assert_eq!(answer("[.[] | path(try .a)] | length"), 2);
    // A slice is not a lookup: `null`, the string and the array take it.
    assert_eq!(answer("[.[] | path(.[0:1]?)] | length"), 3);
    // `null` falls through to `.` (a falsy `null.a`), as does every scalar.
    assert_eq!(answer("[.[] | path(.a? // .)] | length"), 6);
}
