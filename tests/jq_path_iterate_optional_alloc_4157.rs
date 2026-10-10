//! `path(.[])?` over a scalar builds no `Cannot iterate over ...` message the
//! `?` drops (#4157).
//!
//! #3689/#3722/#3728 shortcut a bare `.[]` boundary, but `path(.[])?` wraps the
//! `path(...)`, so the boundary's body is `Builtin::Path`, not `Expr::Iterate`:
//! `path_iterate_step_generic` formatted the message (a preview string and an
//! owned copy of the scalar, seven allocator calls per member) for the `?` to
//! discard. The value boundaries now recognise `path(.[])` too; the path walkers
//! do not, since `path(...)` is not a path expression there.
//!
//! The assertion is on proportion, as in #3728's test: each swallowing query
//! against a twin that visits the same members and does the legitimate
//! per-member work (`path(.)`), in the same process. Each entry (collecting,
//! streaming, the owned bridge) has a row that fails with only its shortcut
//! deleted.
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
/// two more (it made seven before the shortcut).
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

/// The collecting entry: every spelling of the boundary reaches nothing and
/// costs what `path(.)` does.
#[test]
fn a_swallowed_path_iteration_builds_no_message_4157() {
    for (name, json) in fixtures() {
        let (twin_cost, _) = allocations_collecting("[.[] | path(.)] | length", &json);
        // The counter counts: the twin makes an allocation for every member.
        assert!(twin_cost >= N, "{name}: the twin made {twin_cost}");
        for swallowed in [
            "[.[] | path(.[])?] | length",
            "[.[] | (path(.[]))?] | length",
            "[.[] | path((.[]))?] | length",
            "[.[] | try path(.[])] | length",
            "[.[] | try path(.[]) catch empty] | length",
            "def opt(f): f?; [.[] | opt(path(.[]))] | length",
        ] {
            let (cost, answered) = allocations_collecting(swallowed, &json);
            assert_eq!(answered, 0, "{name}: `{swallowed}` reaches nothing");
            assert_costs_like_twin(
                name,
                (swallowed, cost),
                ("[.[] | path(.)] | length", twin_cost),
                6,
            );
        }
    }
}

/// The streaming entry (`succinctly jq` drives it) reaches the same shortcut.
#[test]
fn the_streaming_entry_reaches_the_same_shortcut_4157() {
    for (name, json) in fixtures() {
        let (twin_cost, _) = allocations_streaming(".[] | path(.)", &json);
        for swallowed in [".[] | path(.[])?", ".[] | try path(.[]) catch empty"] {
            let (cost, outputs) = allocations_streaming(swallowed, &json);
            assert_eq!(outputs, 0, "{name}: `{swallowed}` reaches nothing");
            assert_costs_like_twin(name, (swallowed, cost), (".[] | path(.)", twin_cost), 6);
        }
    }
}

/// The owned bridge `paths(f)` evaluates its filter through answers it before
/// re-indexing the scalar: it costs what `paths(.)` does.
#[test]
fn the_owned_bridge_answers_a_swallowed_path_iteration_4157() {
    for (name, json) in fixtures() {
        let (twin_cost, _) = allocations_collecting("[.[] | paths(.)] | length", &json);
        let (cost, answered) = allocations_collecting("[.[] | paths(path(.[])?)] | length", &json);
        assert_eq!(answered, 0, "{name}: reaches nothing");
        assert_costs_like_twin(
            name,
            ("[.[] | paths(path(.[])?)] | length", cost),
            ("[.[] | paths(.)] | length", twin_cost),
            // `paths(f)` evaluates the filter itself: one small allocation per
            // member more than `.[]?` costs (`paths(.[]?)` is within 1 of its
            // twin), against the fifteen a built message made.
            5,
        );
    }
}

/// A container still lists its members: the shortcut is for scalars.
#[test]
fn a_container_still_yields_its_paths_4157() {
    let (_, answered) = allocations_collecting("[[[1,2],{\"k\":3}][] | path(.[])?] | length", "0");
    assert_eq!(answered, 3);
}
