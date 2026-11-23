//! #7846: since #7787 a string interpolation's part (and an object's key or value) that holds a
//! call to a recursive def asked whether it answers once in a frame of its own, around the part's
//! evaluation. A part that streams (a leaf with two outputs) then held that frame under every
//! suspended level of the recursion, and a tree recursion that jq answers at depth 12 was refused
//! with `f/1 exceeded maximum recursion depth`, where the build before #7787 answered. The
//! question is now answered before the part runs, out of line. The expected outputs are jq
//! 1.7.1's (`/usr/bin/jq`), and jq 1.8.2's are the same.
//!
//! These rows pin the issue's answers; they sit at the release build's margin, and a debug build's
//! frames leave more room, so they pass there with the extra frame too. What the frame was is
//! pinned by `settling_part_tests_7846` in `src/jq/eval.rs` and `src/jq/eval_generic.rs`: a part
//! that streams reaches its consumer at the stack depth the plain evaluation does.

use anyhow::Result;

use crate::run_jq_full;

/// The issue's leaf, with two outputs (`1`, `3`), in parentheses, and its `extract` variant.
const LEAF: &str = "(foreach (1,2) as $x (0; . + $x))";
const LEAF_EXTRACT: &str = "(foreach (1,2) as $x (0; . + $x; [$x]))";

fn tree(leaf: &str, tail: &str) -> String {
    format!("def f($n): if $n <= 0 then {leaf} else \"\\(f($n - 1))\\(f($n - 1))\" end; {tail}")
}

#[test]
fn a_tree_recursion_through_an_interpolation_over_a_two_output_leaf_answers_7846() -> Result<()> {
    let rows = [
        (tree(LEAF, "first(f(12)) | length"), "4096\n"),
        (tree(LEAF, "[limit(1; 0 | f(12))] | length"), "1\n"),
        (tree(LEAF, "first(0 | f(12)) | tojson | length"), "4098\n"),
        (
            tree(LEAF_EXTRACT, "first(0 | f(12)) | tojson | length"),
            "12290\n",
        ),
        (
            tree(LEAF, "first(f(12)) | with_entries(.value |= tostring)?"),
            "",
        ),
    ];
    for (program, want) in &rows {
        let (out, err, code) = run_jq_full(&["-c", program], Some("null"))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*want, "", 0),
            "{program}"
        );
    }
    Ok(())
}
