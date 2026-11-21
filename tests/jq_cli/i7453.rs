//! #7453: a negative integer written in the program (`-1`, `-0`) compares as a literal, as in jq
//! 1.8.2, and not as the computed double jq 1.7.1 makes of it.
//!
//! jq 1.7.1 reads `-1` as `_negate` applied to the literal `1`, which drops the literal, so
//! `-1.00000000000000000001 == -1` from the input is `true` there (a computed number and a literal
//! compare as doubles, #6275). Upstream called that a bug (jqlang/jq#2704, labelled `bug`) and jq 1.8
//! fixed it ("Preserve numerical precision on unary negation", jqlang/jq#3242): the negated literal
//! stays a literal, and two literals compare exactly, so the same comparison is `false`. Where jq 1.8
//! fixed a 1.7.1 bug, `succinctly jq` adopts the fix (the limitations page records it as newer than
//! the 1.7.1 pin). `succinctly jq` already answers 1.8.2's way for every row here, so these rows pin
//! it against a regression to 1.7.1's answer.
//!
//! Only the comparison rows are pinned. How a negative literal *prints* still follows 1.7.1
//! (`-0`, `-1.500` as `-1.5`): see the limitations page. Every row captured from jq 1.8.2
//! (`/opt/homebrew/bin/jq`); the comment on each states jq 1.7.1's answer where it differs.

use anyhow::Result;

use crate::run_jq_stdin_streams;

/// `(input, args, filter, stdout)`, each run with `-c`.
const ROWS: &[(&str, &[&str], &str, &str)] = &[
    // The issue's rows. jq 1.7.1: `[true,true,0,[-1.00000000000000000001],"hit"]`.
    (
        r#"{"n":-1.00000000000000000001,"z":1e-400}"#,
        &[],
        r#"[.n == -1, .z == -0, ([.n] | index(-1)), ([.n, -1] | unique), (.n | select(. == -1) | "hit")]"#,
        "[false,false,null,[-1.00000000000000000001,-1]]\n",
    ),
    // jq 1.7.1: `[true,1]`.
    (
        "null",
        &[],
        "[-0 == 1e-400, ([-0, 1e-400] | unique | length)]",
        "[false,2]\n",
    ),
    // The literal carried through a container or arithmetic, and the other comparison consumers.
    // jq 1.7.1: `[true,true,true,true,false,true,[],true,[..],[[-1.00000000000000000001,-1]]]`.
    (
        "-1.00000000000000000001",
        &[],
        "[[-1][0] == ., {a: -1}.a == ., (null + -1) == ., ([-1] | add) == ., (. < -1), \
         (. | IN(-1)), ([.] - [-1]), ({a: .} == {a: -1}), ([., -1] | sort), ([., -1] | group_by(.))]",
        "[false,false,false,false,true,false,[-1.00000000000000000001],false,\
         [-1.00000000000000000001,-1],[[-1.00000000000000000001],[-1]]]\n",
    ),
    // `-0` and `-00` against an input whose double is zero. jq 1.7.1: `[true,true,false,false,1]`.
    (
        "1e-400",
        &[],
        "[. == -0, . == -00, . < -0, . > -0, ([., -0] | unique | length)]",
        "[false,false,false,true,2]\n",
    ),
    // The largest in-range magnitude. jq 1.7.1: `[true,[-9007199254740992.0000001]]`.
    (
        "-9007199254740992.0000001",
        &[],
        "[. == -9007199254740992, ([., -9007199254740992] | unique)]",
        "[false,[-9007199254740992.0000001,-9007199254740992]]\n",
    ),
    // The value bound with `--argjson` rather than read as the input. jq 1.7.1: `[true,[..]]`.
    (
        "null",
        &["--argjson", "x", "-1.00000000000000000001"],
        "[$x == -1, ([$x, -1] | unique)]",
        "[false,[-1.00000000000000000001,-1]]\n",
    ),
    // Controls every version agrees on: a negative index or slice bound, and how a negative
    // integer prints.
    (
        "[1,2,3]",
        &[],
        "[.[-1], .[-2:], .[:-1], path(.[-1]), path(.[-1:])]",
        "[3,[2,3],[1,2],[-1],[{\"start\":-1,\"end\":null}]]\n",
    ),
    (
        "null",
        &[],
        "[-1, -01, -9007199254740992, (-1 | tojson), (-1 | tostring), {a: -1}, (-1 as $x | $x), \
         [range(-2; 0)]]",
        "[-1,-1,-9007199254740992,\"-1\",\"-1\",{\"a\":-1},-1,[-2,-1]]\n",
    ),
];

#[test]
fn test_7453_negative_integer_literal_compares_as_a_literal_as_in_jq_1_8() -> Result<()> {
    for (input, extra, filter, want) in ROWS {
        let mut args = vec!["-c"];
        args.extend_from_slice(extra);
        let (out, err, code) = run_jq_stdin_streams(filter, input, &args)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*want, "", 0),
            "{filter} on {input}"
        );
    }
    Ok(())
}
