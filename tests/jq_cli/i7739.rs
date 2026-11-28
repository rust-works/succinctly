//! #7739: jq's parser compiles operators and syntactic forms into calls by name, resolved by
//! ordinary lexical scope, so a `def` of that name and arity in scope replaces the operator:
//! `_plus` ... `_greatereq` (every binary operator jq does not constant-fold), `_negate` (unary
//! minus), `format/1` (`@name` and string interpolation), `recurse/0` (`..`), `_modify/2` (`|=` and
//! the update-assignments) and `_assign/2` (`=`). Every row was captured from jq 1.7.1
//! (`/usr/bin/jq`); jq 1.8.2 agrees on each.

use std::process::Command;

use anyhow::Result;

use crate::{run_jq_full, spawn_with_signal_retry, succinctly_bin};

/// `(args, stdout, stderr, exit)`, as jq 1.7.1 answers.
type Row = (&'static [&'static str], &'static str, &'static str, i32);

fn check(rows: &[Row]) -> Result<()> {
    for &(args, stdout, stderr, code) in rows {
        let (out, err, status) = run_jq_full(args, None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (stdout, stderr, code),
            "{args:?}"
        );
    }
    Ok(())
}

/// A binary operator that is not constant-folded is a call to `_plus`, `_minus`, ..., `_greatereq`: a `def` of that name and arity in scope replaces it. Two constant operands fold, so no call is made.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object constructions read like format arguments
fn test_operator_calls_binop_7739() -> Result<()> {
    check(&[
        (
            &[r"-nc", r#"def _plus(a;b): "P"; 1 | . + 2"#],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; 1 + 2"#],
            r"3
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"def _plus(a;b): "P"; reduce (1,2) as $x (0; . + $x)"#,
            ],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; [1,2] | add"#],
            r"3
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"def _plus(a;b): [a,b]; 1 | . + 2"],
            r"[1,2]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; 1 as $x | $x + 1"#],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; [1] + [2]"#],
            r"[1,2]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; {} + {}"#],
            r"{}
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; null + 1"#],
            r"1
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; 1 + 2 + ."#],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; . + 1 + 2"#],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; "x" + "y""#],
            r#""xy"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; (1,2) + 3"#],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _minus(a;b): "M"; 1 | . - 1"#],
            r#""M"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _minus(a;b): "M"; 3 - 1"#],
            r"2
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _multiply(a;b): "X"; 2 | . * 3"#],
            r#""X"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _divide(a;b): "D"; 2 | . / 1"#],
            r#""D"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _divide(a;b): "D"; 1 / 0"#],
            r"",
            r"jq: error (at <unknown>): number (1) and number (0) cannot be divided because the divisor is zero
",
            5,
        ),
        (
            &[r"-nc", r#"def _divide(a;b): "D"; 1 | . / 0"#],
            r#""D"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _mod(a;b): "%"; 5 | . % 2"#],
            r#""%"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _equal(a;b): "E"; 1 | . == 1"#],
            r#""E"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _equal(a;b): "E"; 1 == 1"#],
            r"true
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _notequal(a;b): "NE"; 1 | . != 1"#],
            r#""NE"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _less(a;b): "L"; 1 | . < 2"#],
            r#""L"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _greater(a;b): "G"; 1 | . > 2"#],
            r#""G"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _lesseq(a;b): "LE"; 1 | . <= 2"#],
            r#""LE"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _greatereq(a;b): "GE"; 1 | . >= 2"#],
            r#""GE"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _less(a;b): "L"; [3,1] | sort"#],
            r"[1,3]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _less(a;b): "L"; 1 | [select(. < 2)]"#],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"def _equal(a;b): true; [label $f | 1, break $f, 2]",
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _equal(a;b): "E"; [1,2] | index(1)"#],
            r"0
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _equal(a;b): "E"; 1 | IN(1)"#],
            r"true
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _equal(a;b): "E"; [1,1] | unique"#],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; [limit(2; 1,2,3)]"#],
            r"[1,2]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _minus(a;b): "M"; [limit(2; 1,2,3)]"#],
            r"[1,2]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; [range(3)]"#],
            r"[0,1,2]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; [range(0;3;1)]"#],
            r"[0,1,2]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _greater(a;b): "G"; [limit(2; 1,2,3)]"#],
            r"[1,2]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"def _lesseq(a;b): false; [limit(2; 1,2,3)]"],
            r"[1,2]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"def _plus(a;b): "P"; 1 | if . + 1 == 2 then "y" else "n" end"#,
            ],
            r#""n"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def f: def _plus(a;b): "P"; . + 1; 1 | f, . + 1"#],
            r#""P"
2
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"def _plus(a;b): [a,b] | add; 1 | . + 2"],
            r"3
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _equal(a; b): "E"; [1,2] | contains([1])"#],
            r"true
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; $__loc__"#],
            r#"{"file":"<top-level>","line":1}
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; {a: 1} | .a + .a"#],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; [1,2][0] + 1"#],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; 1 | . + 1 as $x | $x"#],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; 1 | -(. + 1)"#],
            r"",
            r#"jq: error (at <unknown>): string ("P") cannot be negated
"#,
            5,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; "a""#],
            r#""a"
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"def _plus(a;b): "P"; try error("x") catch . + "y""#,
            ],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; [.[]?] | length"#],
            r"0
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"def _plus(a;b): "P"; 1 as $x | 2 as $y | $x + $y"#,
            ],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; [1,2] | .[0] + .[1]"#],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"def _less(a;b): true; [3,1,2] | sort_by(.)"],
            r"[1,2,3]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _greater(a;b): "G"; [1,2] | max"#],
            r"2
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; -0 + 1"#],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; [-1] + [2]"#],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; [1, -1] + [2]"#],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; $__loc__ + $__loc__"#],
            r#"{"file":"<top-level>","line":1}
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _equal(a;b): "E"; null | . == null"#],
            r#""E"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _equal(a;b): "E"; null == null"#],
            r"true
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _equal(a;b): "E"; [1] == [1]"#],
            r"true
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _equal(a;b): "E"; {"a":1} == {"a":1}"#],
            r"true
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _equal(a;b): "E"; {a:1} == {"a":1}"#],
            r"true
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _equal(a;b): "E"; (1) == (1)"#],
            r"true
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _equal(a;b): "E"; [.[]?] == []"#],
            r#""E"
"#,
            r"",
            0,
        ),
    ])
}

/// Unary minus is `_negate`, a negative number literal included; its operand is the call's input.
#[test]
fn test_operator_calls_negate_7739() -> Result<()> {
    check(&[
        (
            &[r"-nc", r#"def _negate: "N"; 1 | -."#],
            r#""N"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; [-1]"#],
            r#"["N"]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; -1"#],
            r#""N"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; -(1)"#],
            r#""N"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; 1 as $x | -$x"#],
            r#""N"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; {"a":1} | -.a"#],
            r#""N"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; [1,2] | map(-.)"#],
            r#"["N","N"]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; - "a""#],
            r#""N"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; 1 | abs"#],
            r"1
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; -1 | abs"#],
            r#""N"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; 1 | - ."#],
            r#""N"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; 3 - 1"#],
            r"2
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: [., "N"]; 2 | -."#],
            r#"[2,"N"]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"def _negate: 0 - .; 1 | -."],
            r"-1
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; 1 | -(. + 1)"#],
            r#""N"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; [1] | .[-1]"#],
            r"",
            r#"jq: error (at <unknown>): Cannot index array with string "N"
"#,
            5,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; [1] | .[-1:]"#],
            r"",
            r"jq: error (at <unknown>): Array/string slice indices must be integers
",
            5,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; 1 | - 2"#],
            r#""N"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; -(-1)"#],
            r#""N"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; {"a": -1, "b": (- 2)}"#],
            r#"{"a":"N","b":"N"}
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; [-1, -2.5, -0]"#],
            r#"["N","N","N"]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; -0"#],
            r#""N"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _negate: "N"; - $__loc__"#],
            r#""N"
"#,
            r"",
            0,
        ),
    ])
}

/// `@name` is `format("name")`, and a string with slots is `"" + (E | format(F))` joined with `_plus`.
#[test]
fn test_operator_calls_format_7739() -> Result<()> {
    check(&[
        (
            &[r"-nc", r#"def format(f): "<" + f + ">"; "id=\(1)""#],
            r#""id=<text>"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def format(f): "<" + f + ">"; @csv"#],
            r#""<csv>"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def format(f): "<" + f + ">"; [1,2] | @csv"#],
            r#""<csv>"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def format(f): "<" + f + ">"; @base64 "x\(1)y""#],
            r#""x<base64>y"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def format(f): f; "a\(1)b""#],
            r#""atextb"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def format(f): f; @json "a\(1)b""#],
            r#""ajsonb"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def format(f): "F"; @text"#],
            r#""F"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def format(f): "F"; [1] | @json"#],
            r#""F"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def format(f): "F"; "abc""#],
            r#""abc"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def format(f): "F"; @base64 "abc""#],
            r#""abc"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def format(f): "F"; "a" | format("csv")"#],
            r#""F"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def format: "F"; "a\(1)""#],
            r#""a1"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; "a\(1)""#],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; @text "a\(1)""#],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; @json "\(1)""#],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; "\(1)""#],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; "\(1)\(2)""#],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def format(f): "F"; "\(1)\(2)""#],
            r#""FF"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; {"a\(1)": 2}"#],
            r#"{"P":2}
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; .a as {"k\(1)": $v} | $v"#],
            r"null
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; {"a\(1)": 2} | keys"#],
            r#"["P"]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; {"a":1} | ."a\(1)""#],
            r"null
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; {"a":1} | .["a\(1)"]"#],
            r"null
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def format(f): "F"; {(@base64 "k\(1)"): 2}"#],
            r#"{"kF":2}
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def format(f): "F"; {@base64 "k\(1)": 2}"#],
            r#"{"kF":2}
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def format(f): f; "\(1,2)\(3,4)""#],
            r#""texttext"
"texttext"
"texttext"
"texttext"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#""\(1,2)\(3,4)""#],
            r#""13"
"23"
"14"
"24"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): [a,b]; "\(1,2)-\(3,4)""#],
            r#"[[["","1","2"],"-"],"3","4"]
"#,
            r"",
            0,
        ),
    ])
}

/// `..` is `recurse/0`, which a parameter of that name replaces too.
#[test]
fn test_operator_calls_recurse_7739() -> Result<()> {
    check(&[
        (
            &[r"-nc", r#"def recurse: "R"; {"a":[1]} | [..]"#],
            r#"["R"]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def recurse: "R"; {"a":[1]} | [recurse]"#],
            r#"["R"]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def recurse(f): "R1"; {"a":[1]} | [..]"#],
            r#"[{"a":[1]},[1],1]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def recurse: "R"; {"a":[1]} | [..|numbers]"#],
            r"[]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def recurse: "R"; [paths]"#],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"def recurse: def r: ., (.[]? | r); r; {"a":[1]} | [..]"#,
            ],
            r#"[{"a":[1]},[1],1]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def recurse: .a; {"a":1} | path(..)"#],
            r#"["a"]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def recurse: .a; {"a":{"a":1}} | [..]"#],
            r#"[{"a":1}]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def recurse: .a; {"a":1} | (..) |= 5"#],
            r#"{"a":5}
"#,
            r"",
            0,
        ),
    ])
}

/// `=` is `_assign(paths; value)`, `|=` is `_modify(paths; update)`, and an update-assignment is `value as $tmp | _modify(paths; . op $tmp)`.
#[test]
fn test_operator_calls_assign_7739() -> Result<()> {
    check(&[
        (
            &[r"-nc", r#"def _modify(p;f): "M"; {} | .a |= 1"#],
            r#""M"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _modify(p;f): "M"; {} | .a += 1"#],
            r#""M"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _modify(p;f): "M"; {} | .a //= 1"#],
            r#""M"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _modify(p;f): "M"; {} | .a -= 1"#],
            r#""M"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _modify(p;f): "M"; [1] | map_values(.+1)"#],
            r"[2]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _modify(p;f): "M"; {} | .a = 1"#],
            r#"{"a":1}
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _modify(p;f): [p, f]; {"a":2} | .a |= 1"#],
            r"[2,1]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _modify(p;f): [path(p)]; {"a":2} | .a |= 1"#],
            r#"[["a"]]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _assign(p;f): "A"; {} | .a = 1"#],
            r#""A"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"def _assign(p;$v): $v; {} | .a = 1"],
            r"1
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _assign(p;f): "A"; {} | .a |= 1"#],
            r#"{"a":1}
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"def _assign(p;f): [path(p), f]; {"a":2} | .a = (3,4)"#,
            ],
            r#"[["a"],3,4]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; {} | .a += 1"#],
            r#"{"a":"P"}
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"def _modify(paths; f): reduce path(paths) as $p (.; setpath($p; getpath($p) | f)); {"a":1} | .a |= . + 1"#,
            ],
            r#"{"a":2}
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _modify(p;f): "M"; [1] | .[0] |= 2"#],
            r#""M"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _modify(p;f): "M"; [1] | .[0] += 2"#],
            r#""M"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _assign(p;f): "A"; [1] | .[0] = 2"#],
            r#""A"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"def _modify(p;f): f; 3 | . += 1"],
            r"4
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"def _modify(p;f): f; 3 | . += (1,2)"],
            r"4
5
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _modify(p;f): [p]; {"a":3} | .a //= 7"#],
            r"[3]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _modify(p;f): f; {"a":3} | .a //= 7"#],
            r#"{"a":3}
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"def _modify(p;f): f; null | . //= 7"],
            r"7
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a;b): "P"; {"a":[1,2]} | .a[] += 1"#],
            r#"{"a":["P","P"]}
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _multiply(a;b): "X"; {"a":2} | .a *= 3"#],
            r#"{"a":"X"}
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _divide(a;b): "D"; {"a":2} | .a /= 3"#],
            r#"{"a":"D"}
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _mod(a;b): "%"; {"a":2} | .a %= 3"#],
            r#"{"a":"%"}
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _minus(a;b): "M"; {"a":2} | .a -= 3"#],
            r#"{"a":"M"}
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"def _assign(p; $v): [$v, path(p)]; {} | .a.b = 1"],
            r#"[1,["a","b"]]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"def _assign(p; $v): $v; {} | .a = (1,2)"],
            r"1
2
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"def _modify(p; f): reduce path(p) as $q (.; setpath($q; getpath($q) | f)); [1,2] | .[] |= . * 10",
            ],
            r"[10,20]
",
            r"",
            0,
        ),
    ])
}

/// Scope: only a `def` (or parameter) of the same name and arity in scope at the operator replaces it; later and inner definitions win as for any call, and jq's own builtins keep theirs.
#[test]
fn test_operator_calls_scope_7739() -> Result<()> {
    check(&[
        (
            &[r"-nc", r"def _plus(a;b): a; def f: . + 1; 5 | f"],
            r"5
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def f: . + 1; def _plus(a;b): "P"; 5 | f"#],
            r"6
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"(. + 1), (def _plus(a;b): "P"; . + 1)"#],
            r#"1
"P"
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"def _plus(a;b): "P1"; def _plus(a;b): "P2"; 1 | . + 1"#,
            ],
            r#""P2"
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"def _plus(a;b): "P"; def _plus(a): "Q"; 1 | . + 1"#,
            ],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus(a): "Q"; 1 | . + 1"#],
            r"2
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def _plus: "Q"; 1 | . + 1"#],
            r"2
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def f(_plus): 1 | . + 1; f("x")"#],
            r"2
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def f(recurse): [..]; {"a":1} | f("R")"#],
            r#"["R"]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def f(_negate): -.; 1 | f("N")"#],
            r#""N"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def f($_negate): -.; 1 | f("N")"#],
            r#""N"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def f(format): "\(1)"; f("x")"#],
            r#""1"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def f(_modify): .a |= 1; {} | f("x")"#],
            r#"{"a":1}
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def g: def _plus(a;b): "P"; 1; g | . + 1"#],
            r"2
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"def _plus(a;b): "outer"; def g: def _plus(a;b): "inner"; . + 1; (g, . + 1)"#,
            ],
            r#""inner"
"outer"
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"def _plus(a;b): "P"; def _plus(a;b;c): "Q"; 1 | . + 1"#,
            ],
            r#""P"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"def f(_plus): . + 1; 1 | f(5)"],
            r"2
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def f(format): "\(1)"; f("x")"#],
            r#""1"
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def f(recurse): [..]; {"a":1} | f(empty)"#],
            r"[]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def f(recurse; x): [..]; {"a":1} | f(1; 2)"#],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def f($recurse): [..]; {"a":1} | f(9)"#],
            r"[9]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"def f($format): "\(1)"; f(9)"#],
            r#""1"
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"def recurse(f): "R1"; def recurse: "R0"; {"a":1} | [..]"#,
            ],
            r#"["R0"]
"#,
            r"",
            0,
        ),
    ])
}

/// `succinctly <args>` with `-L <dir>` holding `m.jq` (`def _plus(a;b): "P"; def inc: . + 1;`)
/// and `n.jq` (`def _negate: "N"; def neg: -.;`), on no input.
fn run_with_modules(args: &[&str]) -> Result<(String, i32)> {
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("m.jq"),
        "def _plus(a;b): \"P\";\ndef inc: . + 1;\n",
    )?;
    std::fs::write(
        dir.path().join("n.jq"),
        "def _negate: \"N\";\ndef neg: -.;\n",
    )?;
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(succinctly_bin());
            command
                .args(&args[..1])
                .arg("-L")
                .arg(dir.path())
                .args(&args[1..]);
            command
        },
        None,
    )?;
    Ok((String::from_utf8(output.stdout)?, code))
}

/// A module's definitions reach the operators where jq's scope takes them: an `include`d one
/// everywhere after it, an `import`ed one only inside the module's own definitions.
#[test]
fn test_operator_calls_module_scope_7739() -> Result<()> {
    for (program, want) in [
        (r#"include "m"; 1 | . + 1"#, "\"P\"\n"),
        (r#"import "m" as m; 1 | . + 1"#, "2\n"),
        (r#"import "m" as m; 1 | m::inc"#, "\"P\"\n"),
        (r#"include "m"; 1 | inc"#, "\"P\"\n"),
        (r#"include "m"; "a\(1)""#, "\"P\"\n"),
        (r#"import "n" as n; 1 | n::neg"#, "\"N\"\n"),
        (r#"include "n"; 1 | -."#, "\"N\"\n"),
        (r#"import "n" as n; 1 | -."#, "-1\n"),
    ] {
        let (out, code) = run_with_modules(&["jq", "-nc", program])?;
        assert_eq!((out.as_str(), code), (want, 0), "{program}");
    }
    Ok(())
}

/// yq mode has no such calls: with `--jq-extensions` a `def _plus` is an ordinary definition and
/// the operators stay operators.
#[test]
fn test_operator_calls_leave_yq_mode_alone_7739() -> Result<()> {
    for (program, input, want) in [
        (r#"def _plus(a;b): "P"; . + 1"#, "1\n", "2\n"),
        (r#"def _negate: "N"; -1"#, "1\n", "-1\n"),
        (r#"def recurse: "R"; [..] | length"#, "a: 1\n", "2\n"),
    ] {
        let (output, code) = spawn_with_signal_retry(
            || {
                let mut command = Command::new(succinctly_bin());
                command.args(["yq", "--jq-extensions", program]);
                command
            },
            Some(input.as_bytes()),
        )?;
        assert_eq!(
            (String::from_utf8(output.stdout)?.as_str(), code),
            (want, 0),
            "{program}"
        );
    }
    Ok(())
}

/// Recorded in docs/compliance/jq/limitations.md: `label` compares with the native `==`, not a
/// shadowing `_equal` (jq raises `{"__jq":0}`, exit 5), and a definition that calls the operator it
/// replaces stops at the recursion limit where jq runs until it aborts.
#[test]
fn test_operator_calls_recorded_divergences_7739() -> Result<()> {
    let (out, _err, code) = run_jq_full(
        &["-nc", "def _equal(a;b): false; [label $f | 1, break $f]"],
        None,
    )?;
    assert_eq!((out.as_str(), code), ("[1]\n", 0));
    for (program, name) in [
        ("def _negate: -.; 1 | -.", "_negate/0"),
        ("def _plus(a;b): a + b; 1 | . + 1", "_plus/2"),
        (r#"def recurse: ..; {"a":1} | [..]"#, "recurse/0"),
        (r#"def format(f): "<\(f)>"; "a\(1)""#, "format/1"),
    ] {
        let (out, err, code) = run_jq_full(&["-nc", program], None)?;
        // With `SUCCINCTLY_CALL_MACHINE=1` a recursion that builds a value a level (`format`'s
        // strings) can meet the machine's memory limit before its control-state budget (#8064),
        // and is refused as that, with that wording; the shipped route never gives it.
        let memory = format!("jq: error (at <unknown>): {name}: memory limit exceeded\n");
        let err = if err == memory {
            format!("jq: error (at <unknown>): {name} exceeded maximum recursion depth\n")
        } else {
            err
        };
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (
                "",
                format!("jq: error (at <unknown>): {name} exceeded maximum recursion depth\n")
                    .as_str(),
                5
            ),
            "{program}"
        );
    }
    Ok(())
}

/// A program that defines one of the names but not `_negate` negates its literals through
/// `arith_negate` rather than folding them, which answers as the fold does: as jq 1.8.2, which
/// `succinctly jq` follows for a negated literal since #7453 (jq 1.7.1 converts to a double).
#[test]
fn test_operator_calls_keep_negated_literals_7739() -> Result<()> {
    for (program, want) in [
        (
            "def format(f): f; [-1.500, -0, -1e1000, -(1.50), -0.0]",
            "[-1.500,0,-1E+1000,-1.50,0.0]\n",
        ),
        (r#"def _plus(a;b): "P"; -1.500"#, "-1.500\n"),
        (
            r#"def _plus(a;b): "P"; [1,2,3] | [.[-1], .[-2:]]"#,
            "[3,[2,3]]\n",
        ),
        (
            r#"def recurse: "R"; [-9007199254740993, -1E2]"#,
            "[-9007199254740993,-1E+2]\n",
        ),
        (r#"def _plus(a;b): "P"; [-1 as $x | $x]"#, "[-1]\n"),
    ] {
        let (out, err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((out.as_str(), code), (want, 0), "{program}: {err}");
    }
    Ok(())
}
