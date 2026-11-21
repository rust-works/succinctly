//! #7453: `succinctly jq` negates a number literal as jq 1.8.2 does: the result is still a literal.
//!
//! jq 1.7.1's `_negate` turned its operand into a double, so `-1.500` printed `-1.5`, `-1e1000`
//! printed `-1.7976931348623157e+308`, and `-1` compared with an input `-1.00000000000000000001` as
//! the double `-1` (equal). Upstream called that a bug (jqlang/jq#2704, labelled `bug`) and jq 1.8
//! fixed it ("Preserve numerical precision on unary negation", jqlang/jq#3242): `jv_number_negate`
//! negates a literal's decimal, so the result keeps every digit, prints as written, and compares
//! exactly. `decNumberMinus` computes `0 - x`, so a zero negates to positive zero: `-0` is `0`,
//! `-0.0` is `0.0`. The owner chose 1.8.2's behaviour for #7453, `-0` included. It applies to a
//! literal written after `-`, to `-(...)`, `-$x` and `-.` on a literal (an input number,
//! `--argjson`), and not to a computed number (`-(1 + 0)` is the double `-1`). A negated literal is
//! a fresh `jv`, so `path(.[0] | -1)` on `[-1]` is refused. Every row captured from jq 1.8.2
//! (`/opt/homebrew/bin/jq`); jq 1.7.1's answer is noted where it differs.

use anyhow::Result;

use crate::run_jq_stdin_streams;

/// `(input, args, filter, stdout)`, each run with `-c`.
const ROWS: &[(&str, &[&str], &str, &str)] = &[
    // The issue's rows. jq 1.7.1: `[true,true,0,[-1.00000000000000000001],"hit"]`, `[true,[-0]]`.
    (
        r#"{"n":-1.00000000000000000001,"z":1e-400}"#,
        &[],
        r#"[.n == -1, .z == -0, ([.n] | index(-1)), ([.n, -1] | unique), (.n | select(. == -1) | "hit")]"#,
        "[false,false,null,[-1.00000000000000000001,-1]]\n",
    ),
    (
        "null",
        &[],
        "[-0 == 1e-400, ([-0, 1e-400] | unique)]",
        "[false,[0,1E-400]]\n",
    ),
    // The literal carried through a container or arithmetic, the other comparison consumers, and
    // the spellings jq 1.7.1 already computed (`-1.0`, `-(1)`). jq 1.7.1: all `true` but `<`.
    (
        "-1.00000000000000000001",
        &[],
        "[[-1][0] == ., {a: -1}.a == ., (null + -1) == ., ([-1] | add) == ., (. < -1), \
         (. | IN(-1)), ([.] - [-1]), ({a: .} == {a: -1}), ([., -1] | sort), ([., -1] | group_by(.)), \
         (. == -1.0), (. == -(1))]",
        "[false,false,false,false,true,false,[-1.00000000000000000001],false,\
         [-1.00000000000000000001,-1],[[-1.00000000000000000001],[-1]],false,false]\n",
    ),
    // jq 1.7.1: `[true,true,false,false,[1E-400]]`.
    (
        "1e-400",
        &[],
        "[. == -0, . == -00, . < -0, . > -0, ([., -0] | unique)]",
        "[false,false,false,true,[0,1E-400]]\n",
    ),
    // jq 1.7.1: `[true,[-9007199254740992.0000001]]`.
    (
        "-9007199254740992.0000001",
        &[],
        "[. == -9007199254740992, ([., -9007199254740992] | unique)]",
        "[false,[-9007199254740992.0000001,-9007199254740992]]\n",
    ),
    // `--argjson` binds a literal, and `-$x` keeps it one. jq 1.7.1: `[true,[..],1]`.
    (
        "null",
        &["--argjson", "x", "-1.00000000000000000001"],
        "[$x == -1, ([$x, -1] | unique), -$x]",
        "[false,[-1.00000000000000000001,-1],1.00000000000000000001]\n",
    ),
    // Printing. jq 1.7.1: `[-1,-0,-0,-1,-9007199254740992,-9007199254740992,-1e+20,-1.5,-100,-0,
    // -1.7976931348623157e+308,-0,-1.2345678901234568e+39]`.
    (
        "null",
        &[],
        "[-1, -0, -00, -01, -9007199254740992, -9007199254740993, -100000000000000000000, -1.500, \
         -1e2, -0.0, -1e1000, -1e-400, -1234567890123456789012345678901234567890]",
        "[-1,0,0,-1,-9007199254740992,-9007199254740993,-100000000000000000000,-1.500,-1E+2,0.0,\
         -1E+1000,-1E-400,-1234567890123456789012345678901234567890]\n",
    ),
    // jq 1.7.1: `["-0","-1.5","-0","[-0]",{"a":-1.5},[-0,1],-1.5,1.5,0,0,1.5,{"-1.5":-0}]`.
    (
        "null",
        &[],
        "[(-0 | tojson), (-1.500 | tostring), (-0 | @text), ([-0.0] | tojson), {a: -1.500}, \
         [range(-0; 2)], (-1.500 as $x | $x), -(-1.500), -(-0), (-0 + 0), (-1.500 | -.), \
         {(-1.500 | tostring): -0}]",
        "[\"0\",\"-1.500\",\"0\",\"[0.0]\",{\"a\":-1.500},[0,1],-1.500,1.500,0,0,1.500,\
         {\"-1.500\":0}]\n",
    ),
    // `-.`, `-$x` and `-(...)` on an input literal keep it; a computed number is negated as a
    // double. jq 1.7.1: `[[-1.5,-0,-0,0,-1e+20,2.5],-1.5,-1.5,-1.5,-1,1.5]`.
    (
        "[1.500, 1e-400, 0.0, -0, 100000000000000000001, -2.50]",
        &[],
        "[map(-.), (.[0] as $x | -$x), -(.[0]), -(1.500 + 0), -(1 + 0), (.[0] | -. | -.)]",
        "[[-1.500,-1E-400,0.0,0,-100000000000000000001,2.50],-1.500,-1.500,-1.5,-1,1.500]\n",
    ),
    // A negative index or slice bound names the same position; its path component is the
    // negated literal. jq 1.7.1: `[3,1,[-0],[-1],[-1],[2,3],[{"start":-1,"end":null}]]`.
    (
        "[1,2,3]",
        &[],
        "[.[-1], .[-0], path(.[-0]), path(.[-1]), path(.[-1.0]), .[-2:], path(.[-1.0:])]",
        "[3,1,[0],[-1],[-1.0],[2,3],[{\"start\":-1.0,\"end\":null}]]\n",
    ),
    // A negated literal is a fresh `jv`, identical to nothing else. jq 1.7.1: `[0]`, `[-0]`.
    (
        "null",
        &[],
        "([-1] | try path(.[0] | -1) catch .), [-0 | try path(error) catch .]",
        "\"Invalid path expression with result -1\"\n[0]\n",
    ),
];

#[test]
fn test_7453_negated_literal_stays_a_literal_as_in_jq_1_8() -> Result<()> {
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
