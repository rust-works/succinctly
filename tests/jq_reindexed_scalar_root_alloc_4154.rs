//! A scalar the owned evaluator re-indexes costs a handful of allocator calls,
//! not sixteen (#4154), and in steady state one for the bridge (#4217).
//!
//! `OwnedValue::reindexed` wrote a one-token document out and ran the general
//! index build over it, whose balanced-parentheses directories alone are eight
//! `Vec`s. A scalar root's index follows from its length (`JsonIndex::
//! build_reindex_scalar`, `BalancedParens::leaf`), so a bridge crossing was the
//! text, the interest bits, their rank and the two-bit sequence: four calls.
//! #4217 recycles the last three from the document the previous crossing
//! dropped, leaving the text.
//!
//! Queries that cross the bridge once per scalar member (`del(.)`, `paths(.)`,
//! `del(.[]?)`) over documents of `N` members must stay under a per-member
//! ceiling of allocator calls, set one above what each row measures. The
//! ceilings are absolute because the term is the bridge's own: there is no
//! twin query that does the same legitimate work without crossing it. With
//! `build_reindex_scalar` replaced by `build_reindex` the integer rows read
//! 16-18 per member, and with the buffers not recycled every row reads three
//! more than it may; either fails this test.
//!
//! Counting is gated to the calling thread (`tests/common/counting_alloc.rs`), so
//! the harness's other threads, and other tests in this file, cannot add to the
//! window.

#[path = "common/counting_alloc.rs"]
mod counting_alloc;
use counting_alloc::{allocations_collecting, Counting};

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const N: usize = 2_000;

/// Allocator calls per member each row may make, in `ROWS` order, one above
/// what it measures (5/7/5 on integers, 7/9/7 on strings -- the owned string is
/// its own allocation -- and 3/5/2 on booleans and nulls). Before #4217 every
/// row read three higher; before #4154 integers made 16-18.
type Ceilings = [usize; 3];

/// Without `std` there is no thread-local to keep a dropped document's index
/// buffers in (#4217), so every crossing allocates them afresh: three more
/// calls per member than the ceilings above, which are the pooled counts.
const UNPOOLED: usize = if cfg!(feature = "std") { 0 } else { 3 };

fn fixtures() -> Vec<(&'static str, String, Ceilings)> {
    let join = |parts: Vec<String>| parts.join(",");
    vec![
        (
            "array of integers",
            format!("[{}]", join((0..N).map(|i| i.to_string()).collect())),
            [6, 8, 6],
        ),
        (
            "array of strings",
            format!(
                "[{}]",
                join((0..N).map(|i| format!("\"value number {i}\"")).collect())
            ),
            [8, 10, 8],
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
            [4, 6, 3],
        ),
    ]
}

#[test]
fn a_scalar_crossing_the_reindex_bridge_allocates_a_handful_of_times_4154() {
    let rows = [
        ("[.[] | del(.)] | length", N as i64),
        ("[.[] | del(.[]?)] | length", N as i64),
        ("[.[] | paths(.)] | length", 0),
    ];
    for (name, json, ceilings) in fixtures() {
        for ((query, answer), ceiling) in rows.into_iter().zip(ceilings) {
            let (cost, answered) = allocations_collecting(query, &json);
            assert_eq!(answered, answer, "{name}: `{query}`'s answer");
            assert!(
                cost <= (ceiling + UNPOOLED) * N,
                "{name}: `{query}` made {cost} allocator calls over {N} members \
                 (allowing {} per member)",
                ceiling + UNPOOLED
            );
        }
    }
}
