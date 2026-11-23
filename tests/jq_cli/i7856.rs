//! #7856: a recursion's outputs climb back through every level's consumer on top of the
//! descent's frames, and nothing on the climb checked the native-stack floor (ADR-0025).
//! `{k: f($n - 1), v: (foreach (1,2) as $x (0; . + $x))}` runs the object's value for each
//! output of the call from inside that call, and the climb spent more stack per level than the
//! descent did: at depth 9,000 a release build overflowed the stack and aborted (exit 134), where
//! jq 1.7.1 and 1.8.2 print `2`. An output of a call to a def that may recurse now checks the stack
//! on its way out, a quarter of the stack below the floor, so the climb is refused with the
//! floor's error instead.
//!
//! The depth of the refusal follows the build (a frame's size keys on the optimisation level and
//! the stack's on `debug_assertions`), so it is found, not fixed: the depth doubles until the
//! program is refused, and the depths short of that, 4% apart from half the refused depth, must
//! each answer `2` (jq's answer, captured from `/usr/bin/jq` and jq 1.8.2) or be refused, never
//! end on a signal. The issue's leaf is the case of the release build; a leaf of four streamed
//! `foreach` stages climbs with more stack per level than the descent takes in any build, the
//! debug build these tests run in included.

use anyhow::Result;

use crate::run_jq_full;

/// The issue's leaf, and four streamed `foreach` stages.
const FOREACH: &str = "(foreach (1,2) as $x (0; . + $x))";
const HEAVY: &str = "(0 | foreach (1,2) as $x (.; . + $x) | foreach (1,2) as $y (.; . + $y) \
                     | foreach (1,2) as $z (.; . + $z) | foreach (1,2) as $w (.; . + $w))";

fn program(leaf: &str, depth: u32) -> String {
    format!(
        "def f($n): if $n <= 0 then {leaf} else {{k: f($n - 1), v: {leaf}}} end; \
         first(f({depth})) | length"
    )
}

/// Whether `stderr` is the floor's refusal: at a call (`f/1`), or at the read of its argument.
fn is_refusal(stderr: &str) -> bool {
    [
        "jq: error (at <unknown>): f/1 exceeded maximum recursion depth\n",
        "jq: error (at <unknown>): exceeded maximum recursion depth\n",
    ]
    .contains(&stderr)
}

/// What a run of `program` ended in: `true` when it was refused (exit 5), `false` when it
/// answered jq's `2`; anything else fails.
fn refused(program: &str) -> Result<bool> {
    let (out, err, code) = run_jq_full(&["-nc", program], None)?;
    if out.is_empty() && code == 5 && is_refusal(&err) {
        return Ok(true);
    }
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        ("2\n", "", 0),
        "neither jq's answer nor the floor's refusal: {program}"
    );
    Ok(false)
}

#[test]
fn a_recursion_climbing_through_an_object_refuses_instead_of_aborting_7856() -> Result<()> {
    for leaf in [FOREACH, HEAVY] {
        let mut depth = 1_000;
        while !refused(&program(leaf, depth))? {
            assert!(depth < 1_000_000, "no refusal up to {depth} levels: {leaf}");
            depth *= 2;
        }
        let mut below = depth / 2;
        while below < depth {
            refused(&program(leaf, below))?;
            below += (depth / 25).max(1);
        }
        // Uncatchable, like every refusal of the floor's.
        let (out, err, code) = run_jq_full(
            &[
                "-nc",
                &format!(r#"try ({}) catch "x""#, program(leaf, depth)),
            ],
            None,
        )?;
        assert!(
            out.is_empty() && code == 5 && is_refusal(&err),
            "{leaf}: {out:?} {err:?} {code}"
        );
    }
    Ok(())
}

/// The issue's two neighbours, which climbed with less stack per level and answered: they still
/// do, at a depth the release build reaches with room to spare.
#[test]
fn the_issues_neighbours_still_answer_7856() -> Result<()> {
    for leaf in ["(1,3)", "1"] {
        assert!(!refused(&program(leaf, 2_000))?, "{leaf}");
    }
    Ok(())
}
