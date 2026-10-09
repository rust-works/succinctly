//! `..` over a cursor allocates no more per node than `.[]` does (#3023).
//!
//! `each_recurse_cursor_generic` used to reuse the `path()` step for its child
//! enumeration. For every child that built an `Rc` trail link and, for an
//! object, an owned key `String`, then threw the trail away unread; it kept a
//! `Vec` of those per container; and for every scalar leaf -- most nodes -- it
//! formatted a `Cannot iterate over ...` error that `..`'s own `.[]?`
//! swallowed. A counting allocator over `[..] | length` sees all of that as a
//! term proportional to the document's node count. Measured on a debug build
//! when this was written: 18,055 allocator calls for a 2,000-member flat
//! object, against 23 for the `[.[]] | length` that visits the same members.
//!
//! The assertion is on that proportion -- `..` against an `.[]` twin that
//! visits the same nodes, on the same input, in the same process -- not on an
//! exact count, so it survives allocator and toolchain changes that move the
//! constant. A lower bound on the twin guards the other direction: a counter
//! that stopped counting would otherwise pass every comparison with `0 < 0 + N/4`.
//!
//! Counting is gated to the calling thread, so the harness's other threads, and
//! other tests in this file, cannot add to the window.

#[path = "common/counting_alloc.rs"]
mod counting_alloc;
use counting_alloc::{allocations_collecting, Counting};

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[test]
fn recurse_allocates_no_more_per_node_than_iterate_3023() {
    const N: usize = 2_000;

    let join = |parts: Vec<String>| parts.join(",");
    // (name, document, `..` query and its answer, the `.[]` twin and its answer)
    let fixtures = [
        // Every member a keyed child: the owned-key `String` and the trail link
        // were each paid N times, and every integer leaf built an error.
        (
            "object of integers",
            format!(
                "{{{}}}",
                join((0..N).map(|i| format!("\"key{i}\":{i}")).collect())
            ),
            ("[..] | length", N + 1),
            ("[.[]] | length", N),
        ),
        // The array twin: no key `String`, but the link and the leaf error
        // were per child just the same.
        (
            "array of integers",
            format!("[{}]", join((0..N).map(|i| i.to_string()).collect())),
            ("[..] | length", N + 1),
            ("[.[]] | length", N),
        ),
        // A string leaf is read to be validated; that must not allocate either,
        // since the walk throws the result away.
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
            ("[..] | length", N + 1),
            ("[.[]] | length", N),
        ),
        // N containers: the per-container `Vec` of children the walk used to
        // build is the only term the flat fixtures above cannot see. The twin
        // visits the same 2N nodes below the root through `.[]`.
        (
            "array of single-element arrays",
            format!("[{}]", join((0..N).map(|_| "[1]".to_string()).collect())),
            ("[..] | length", 2 * N + 1),
            ("[.[], (.[] | .[])] | length", 2 * N),
        ),
    ];

    for (name, json, (recurse_query, recursed), (iterate_query, iterated)) in &fixtures {
        let (iterate, answered) = allocations_collecting(iterate_query, json);
        assert_eq!(answered, *iterated as i64, "{name}: the twin's answer");
        let (recurse, answered) = allocations_collecting(recurse_query, json);
        assert_eq!(answered, *recursed as i64, "{name}: the nodes `..` visits");

        assert!(
            iterate > 0,
            "{name}: the `.[]` twin counted no allocations, so the counter is not counting"
        );
        assert!(
            recurse < iterate + N / 4,
            "{name}: `{recurse_query}` made {recurse} allocator calls against \
             `{iterate_query}`'s {iterate} over the same members; a per-node \
             allocation in `..` would add at least {N}"
        );
    }
}
