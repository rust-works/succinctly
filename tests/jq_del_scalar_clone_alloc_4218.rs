//! `del(.[]?)` and `del(.)` over a scalar member copy no expression tree per
//! member (#4218).
//!
//! Each scalar crossing the owned evaluator's bridge paid ten allocator calls
//! for `del(.[]?)` and eight for `del(.)`. Four of the ten were `Expr` boxes
//! the query never needed a second copy of:
//!
//! - `eval_generic::eval_builtin` rebuilt `Expr::Builtin(builtin.clone())` for
//!   the bridge from a `Builtin` whose enclosing node its dispatcher held
//!   (`builtin_expr` now borrows it);
//! - `builtin_del` copied the path it was handed into a one-element `Vec` for
//!   `delete_at_path`, which only reads it (it now lends the path unless an
//!   alias table is installed).
//!
//! What remains per member is the reindex bridge (text, interest bits, their
//! rank, the two-bit sequence: #4154) and the two decodes of the scalar.
//!
//! The bounds are absolute because the term is the bridge's own: there is no
//! twin query doing the same legitimate work without crossing it. Each is the
//! measured cost, so either fix alone, put back, exceeds it: the integer
//! `del(.[]?)` row reads 8 per member with only one of the two in place, and 10
//! with neither (`del(.)`: 7 and 8).
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

fn fixtures() -> Vec<(&'static str, String)> {
    let join = |parts: Vec<String>| parts.join(",");
    vec![
        (
            "array of integers",
            format!("[{}]", join((0..N).map(|i| i.to_string()).collect())),
        ),
        (
            "array of strings",
            format!(
                "[{}]",
                join((0..N).map(|i| format!("\"value number {i}\"")).collect())
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

/// Allocator calls per member a row may make, on each fixture in `fixtures`
/// order: the bridge's four, the scalar's two decodes (an owned string is one
/// more call each) and nothing else. The booleans and nulls need no decode
/// allocation.
const PER_MEMBER: [usize; 3] = [6, 8, 4];

/// Calls independent of the member count (the result array and the one-time
/// tables a first evaluation builds): 22 measured on the integer and boolean
/// rows, 12 on the strings.
const FIXED: usize = 32;

const ROWS: [&str; 2] = ["[.[] | del(.[]?)] | length", "[.[] | del(.)] | length"];

#[test]
fn del_over_scalar_members_copies_no_expression_tree_4218() {
    for (fixture, (name, json)) in fixtures().into_iter().enumerate() {
        for query in ROWS {
            let (cost, answered) = allocations_collecting(query, &json);
            assert_eq!(answered, N as i64, "{name}: `{query}`'s answer");
            let bound = PER_MEMBER[fixture];
            assert!(
                cost <= bound * N + FIXED,
                "{name}: `{query}` made {cost} allocator calls over {N} members \
                 (allowing {bound} per member and {FIXED} in all)"
            );
        }
    }
}
