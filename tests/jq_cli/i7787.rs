//! #7787: a tree recursion over a leaf or a combination the settle question of #7460 and #7713
//! did not read (`first`, `last`, `limit(1; ...)`, `foreach`, `until`, a local def with
//! parameters, `env`, `input_line_number`, `IN`, an object, `getpath`, `//`, `label`, `and`,
//! `or`, a string interpolation, an `if` condition) was refused with `exceeded maximum recursion
//! depth` at about 8,192 leaves, and #7744 settled `first(...)` around a `?//` that a later failure
//! has to retry. The expected outputs are jq 1.7.1's (`/usr/bin/jq`), and jq 1.8.2's are the same.

use anyhow::Result;

use crate::run_jq_full;

/// Leaves of `t($n - 1) + t($n - 1)`, 8,192 of them, depth 13.
const LEAVES: &[&str] = &[
    "limit(1; 1, 2)",
    "foreach (1) as $x (0; $x)",
    "(0|until(. >= 1; . + 1))",
    "[1] | first",
    "[1] | last",
    "input_line_number + 1",
    "(label $f | 1, break $f)",
    "def k($a): $a; k(1)",
    "def k(f): f; k(1)",
    "[1] | IN(.[]; 1) | if . then 1 else 0 end",
    "tojson | fromjson | 1",
    "1 | until(true; .)",
    "env | 1",
];

/// Combinations of two calls (`{A}`, `{B}`) over the leaf `1`, depth 13.
const COMBINATIONS: &[&str] = &[
    "first({A}) + first({B})",
    "{a: {A}, b: {B}} | .a + .b",
    "{a: {A}} | getpath([\"a\"]) + {B}",
    "{A} + (label $q | {B})",
    "{A} + first({B}, 7)",
    "{A} + ({B} // 0)",
    "def g: {A}; g + {B}",
];

/// A recursion through each place a construction runs the rest of its work from a part's output:
/// `and`, `or`, a string interpolation, an object's value and key, an `if` condition, `//`, and an
/// operator whose other operand calls nothing; depth 14.
const SITES: &[&str] = &[
    "def t($n): if $n <= 0 then true else t($n-1) and t($n-1) end; [0 | t(14)] | length",
    "def t($n): if $n <= 0 then true else t($n-1) or t($n-1) end; [0 | t(14)] | length",
    "def t($n): if $n <= 0 then 1 else \"\\(t($n-1))\\(t($n-1))\" | length end; [0 | t(14)] | length",
    "def t($n): if $n <= 0 then 1 else {a: t($n-1), b: t($n-1)} | .a + .b end; [0 | t(14)] | length",
    "def t($n): if $n <= 0 then 1 else {(t($n-1) | tostring): t($n-1)} | length end; [0 | t(14)] | length",
    "def t($n): if $n <= 0 then 1 else (if t($n-1) > 0 then t($n-1) else 0 end) + 0 end; [0 | t(14)] | length",
    "def t($n): if $n <= 0 then 1 else t($n-1) + (t($n-1) // 0) end; [0 | t(14)] | length",
    "def t($n): if $n <= 0 then 1 else t($n-1) + 1 + t($n-1) - 1 end; [0 | t(14)] | length",
];

#[test]
fn a_tree_recursion_over_more_leaves_and_combinations_answers_7787() -> Result<()> {
    let programs = LEAVES
        .iter()
        .map(|leaf| {
            format!("def t($n): if $n <= 0 then {leaf} else t($n-1) + t($n-1) end; [0 | t(13)] | length")
        })
        .chain(COMBINATIONS.iter().map(|comb| {
            let body = comb.replace("{A}", "t($n-1)").replace("{B}", "t($n-1)");
            format!("def t($n): if $n <= 0 then 1 else {body} end; [0 | t(13)] | length")
        }))
        .chain(SITES.iter().map(|p| (*p).to_string()));
    for program in programs {
        let (out, err, code) = run_jq_full(&["-nc", &program], None)?;
        assert_eq!((out.as_str(), code), ("1\n", 0), "{program}: {err}");
    }
    Ok(())
}

/// What settling must keep: the order of `debug`/`stderr` output around the calls it runs first,
/// an error raised after an output, a consumer that stops early (`first`, `limit`, `label`), a
/// `break` to an outer label, `halt` after an output, and a `?//` that a failure after the call
/// retries through `first`, `limit`, `try`, `label`, an object, `as`, `if`, an operator or a pipe
/// (and not through `last`, `[...]` or `reduce`). `(program, stdout, stderr, exit status)`.
const SEMANTICS: &[(&str, &str, &str, i32)] = &[
    (
        "def d($x): $x | debug; d(1) + d(2)",
        "3\n",
        "[\"DEBUG:\",2]\n[\"DEBUG:\",1]\n",
        0,
    ),
    (
        "def s($x): $x | stderr; s(1) + s(2)",
        "3\n",
        "21",
        0,
    ),
    (
        "def d($x): $x | debug; (d(true) and d(false)), (d(false) or d(true))",
        "false\ntrue\n",
        "[\"DEBUG:\",true]\n[\"DEBUG:\",false]\n[\"DEBUG:\",false]\n[\"DEBUG:\",true]\n",
        0,
    ),
    (
        "def d($x): $x | debug; d(1) | d(. + 1) | d(. + 1)",
        "3\n",
        "[\"DEBUG:\",1]\n[\"DEBUG:\",2]\n[\"DEBUG:\",3]\n",
        0,
    ),
    (
        "def d($x): $x | debug; d(1) as $a | d(2) as $b | $a + $b",
        "3\n",
        "[\"DEBUG:\",1]\n[\"DEBUG:\",2]\n",
        0,
    ),
    (
        "def d($x): $x | debug; if d(true) then d(1) else d(2) end",
        "1\n",
        "[\"DEBUG:\",true]\n[\"DEBUG:\",1]\n",
        0,
    ),
    (
        "def d($x): $x | debug; {a: d(1), (d(\"k\")): d(2)}",
        "{\"a\":1,\"k\":2}\n",
        "[\"DEBUG:\",1]\n[\"DEBUG:\",\"k\"]\n[\"DEBUG:\",2]\n",
        0,
    ),
    (
        "def d($x): $x | debug; \"\\(d(1))-\\(d(2))\"",
        "\"1-2\"\n",
        "[\"DEBUG:\",2]\n[\"DEBUG:\",1]\n",
        0,
    ),
    (
        "def d($x): $x | debug; (d(null) // d(1)) + d(2)",
        "3\n",
        "[\"DEBUG:\",2]\n[\"DEBUG:\",null]\n[\"DEBUG:\",1]\n",
        0,
    ),
    (
        "def d($x): $x | debug; first(d(1), d(2)) + d(3)",
        "4\n",
        "[\"DEBUG:\",3]\n[\"DEBUG:\",1]\n",
        0,
    ),
    (
        "def d($x): $x | debug; [limit(1; d(1), d(2))] + [d(3)]",
        "[1,3]\n",
        "[\"DEBUG:\",3]\n[\"DEBUG:\",1]\n",
        0,
    ),
    (
        "def d($x): $x | debug; (label $l | d(1), break $l) + d(2)",
        "3\n",
        "[\"DEBUG:\",2]\n[\"DEBUG:\",1]\n",
        0,
    ),
    (
        "def d($x): $x | debug; (try d(1)) + (try error(\"x\") catch d(2))",
        "3\n",
        "[\"DEBUG:\",2]\n[\"DEBUG:\",1]\n",
        0,
    ),
    (
        "def d($x): $x | debug; reduce (d(1), d(2)) as $x (d(0); . + $x) | d(.)",
        "3\n",
        "[\"DEBUG:\",0]\n[\"DEBUG:\",1]\n[\"DEBUG:\",2]\n[\"DEBUG:\",3]\n",
        0,
    ),
    (
        "def d($x): $x | debug; foreach d(1) as $x (d(0); . + $x) | d(.)",
        "1\n",
        "[\"DEBUG:\",0]\n[\"DEBUG:\",1]\n[\"DEBUG:\",1]\n",
        0,
    ),
    (
        "def d($x): $x | debug; (0 | until(d(. >= 2); d(. + 1))) + d(10)",
        "12\n",
        "[\"DEBUG:\",10]\n[\"DEBUG:\",false]\n[\"DEBUG:\",1]\n[\"DEBUG:\",false]\n[\"DEBUG:\",2]\n[\"DEBUG:\",true]\n",
        0,
    ),
    (
        "def d($x): $x | debug; first(d(1), d(2) | . + 0)",
        "1\n",
        "[\"DEBUG:\",1]\n",
        0,
    ),
    (
        "def g: (1 | debug), (2 | debug); first(g + 0)",
        "1\n",
        "[\"DEBUG:\",1]\n",
        0,
    ),
    (
        "def g: (1 | debug), (2 | debug); [limit(1; g | . + 0)]",
        "[1]\n",
        "[\"DEBUG:\",1]\n",
        0,
    ),
    (
        "def g: (1 | debug), (2 | debug); label $o | (g + 0) | ., break $o",
        "1\n",
        "[\"DEBUG:\",1]\n",
        0,
    ),
    (
        "def g: 1, (2 | debug | empty); first(g + 0)",
        "1\n",
        "",
        0,
    ),
    (
        "def g: 1, halt; [g + 0]",
        "",
        "",
        0,
    ),
    (
        "def g: 1, (\"bye\" | halt_error); g + 0",
        "1\n",
        "bye",
        5,
    ),
    (
        "def g: 1, error(\"x\"); g + 0",
        "1\n",
        "jq: error (at <unknown>): x\n",
        5,
    ),
    (
        "def g: 1, error(\"x\"); first(g + 0)",
        "1\n",
        "",
        0,
    ),
    (
        "def g: 1, error(\"x\"); [try (g + 0)]",
        "[1]\n",
        "",
        0,
    ),
    (
        "def g: 1, 2; (g + 0) | if . == 1 then error(\"c\") else . end",
        "",
        "jq: error (at <unknown>): c\n",
        5,
    ),
    (
        "def g: (1, empty); (g + 0) | error(\"c\")",
        "",
        "jq: error (at <unknown>): c\n",
        5,
    ),
    (
        "label $out | (def g: 1, break $out; g + 0), 9",
        "1\n",
        "",
        0,
    ),
    (
        "[label $out | (def g: (1, break $out); g + g), 9]",
        "[2]\n",
        "",
        0,
    ),
    (
        "def f: $__loc__.line; f + f",
        "2\n",
        "",
        0,
    ),
    (
        "def f: env | type; f + f",
        "\"objectobject\"\n",
        "",
        0,
    ),
    (
        "def f: first([1] as [$a] ?// $a | $a); f | if . == 1 then error(\"e\") else . end",
        "[1]\n",
        "",
        0,
    ),
    (
        "def f: first([1] as [$a] ?// $a | $a); (f | if . == 1 then error(\"e\") else . end) + 0",
        "",
        "jq: error (at <unknown>): array ([1]) and number (0) cannot be added\n",
        5,
    ),
    (
        "def f: limit(1; [1] as [$a] ?// $a | $a); f | if . == 1 then error(\"e\") else . end",
        "[1]\n",
        "",
        0,
    ),
    (
        "def f: limit(1; [1] as [$a] ?// $a | $a); (f | if . == 1 then error(\"e\") else . end) + 0",
        "",
        "jq: error (at <unknown>): array ([1]) and number (0) cannot be added\n",
        5,
    ),
    (
        "def f: try ([1] as [$a] ?// $a | $a); f | if . == 1 then error(\"e\") else . end",
        "[1]\n",
        "",
        0,
    ),
    (
        "def f: try ([1] as [$a] ?// $a | $a); (f | if . == 1 then error(\"e\") else . end) + 0",
        "",
        "jq: error (at <unknown>): array ([1]) and number (0) cannot be added\n",
        5,
    ),
    (
        "def f: label $l | ([1] as [$a] ?// $a | $a); f | if . == 1 then error(\"e\") else . end",
        "[1]\n",
        "",
        0,
    ),
    (
        "def f: label $l | ([1] as [$a] ?// $a | $a); (f | if . == 1 then error(\"e\") else . end) + 0",
        "",
        "jq: error (at <unknown>): array ([1]) and number (0) cannot be added\n",
        5,
    ),
    (
        "def f: {k: ([1] as [$a] ?// $a | $a)} | .k; f | if . == 1 then error(\"e\") else . end",
        "[1]\n",
        "",
        0,
    ),
    (
        "def f: {k: ([1] as [$a] ?// $a | $a)} | .k; (f | if . == 1 then error(\"e\") else . end) + 0",
        "",
        "jq: error (at <unknown>): array ([1]) and number (0) cannot be added\n",
        5,
    ),
    (
        "def f: ([1] as [$a] ?// $a | $a) as $v | $v; f | if . == 1 then error(\"e\") else . end",
        "[1]\n",
        "",
        0,
    ),
    (
        "def f: ([1] as [$a] ?// $a | $a) as $v | $v; (f | if . == 1 then error(\"e\") else . end) + 0",
        "",
        "jq: error (at <unknown>): array ([1]) and number (0) cannot be added\n",
        5,
    ),
    (
        "def f: if true then ([1] as [$a] ?// $a | $a) else 0 end; f | if . == 1 then error(\"e\") else . end",
        "[1]\n",
        "",
        0,
    ),
    (
        "def f: if true then ([1] as [$a] ?// $a | $a) else 0 end; (f | if . == 1 then error(\"e\") else . end) + 0",
        "",
        "jq: error (at <unknown>): array ([1]) and number (0) cannot be added\n",
        5,
    ),
    (
        "def f: ([1] as [$a] ?// $a | $a) + 0; f | if . == 1 then error(\"e\") else . end",
        "",
        "jq: error (at <unknown>): array ([1]) and number (0) cannot be added\n",
        5,
    ),
    (
        "def f: ([1] as [$a] ?// $a | $a) + 0; (f | if . == 1 then error(\"e\") else . end) + 0",
        "",
        "jq: error (at <unknown>): array ([1]) and number (0) cannot be added\n",
        5,
    ),
    (
        "def f: [isempty([1] as [$a] ?// $a | $a)] | .[0] as $e | ([1] as [$a] ?// $a | $a); f | if . == 1 then error(\"e\") else . end",
        "[1]\n",
        "",
        0,
    ),
    (
        "def f: [isempty([1] as [$a] ?// $a | $a)] | .[0] as $e | ([1] as [$a] ?// $a | $a); (f | if . == 1 then error(\"e\") else . end) + 0",
        "",
        "jq: error (at <unknown>): array ([1]) and number (0) cannot be added\n",
        5,
    ),
    (
        "def f: last([1] as [$a] ?// $a | $a); f | if . == 1 then error(\"e\") else . end",
        "",
        "jq: error (at <unknown>): e\n",
        5,
    ),
    (
        "def f: last([1] as [$a] ?// $a | $a); (f | if . == 1 then error(\"e\") else . end) + 0",
        "",
        "jq: error (at <unknown>): e\n",
        5,
    ),
    (
        "def f: [[1] as [$a] ?// $a | $a] | .[0]; f | if . == 1 then error(\"e\") else . end",
        "",
        "jq: error (at <unknown>): e\n",
        5,
    ),
    (
        "def f: [[1] as [$a] ?// $a | $a] | .[0]; (f | if . == 1 then error(\"e\") else . end) + 0",
        "",
        "jq: error (at <unknown>): e\n",
        5,
    ),
    (
        "def f: reduce ([1] as [$a] ?// $a | $a) as $x (0; $x); f | if . == 1 then error(\"e\") else . end",
        "",
        "jq: error (at <unknown>): e\n",
        5,
    ),
    (
        "def f: reduce ([1] as [$a] ?// $a | $a) as $x (0; $x); (f | if . == 1 then error(\"e\") else . end) + 0",
        "",
        "jq: error (at <unknown>): e\n",
        5,
    ),
    (
        "def f: \"\\([1] as [$a] ?// $a | $a)\"; f | if . == 1 then error(\"e\") else . end",
        "\"1\"\n",
        "",
        0,
    ),
    (
        "def f: \"\\([1] as [$a] ?// $a | $a)\"; (f | if . == 1 then error(\"e\") else . end) + 0",
        "",
        "jq: error (at <unknown>): string (\"[1]\") and number (0) cannot be added\n",
        5,
    ),
    (
        "def f: ([1] as [$a] ?// $a | $a) | .; f | if . == 1 then error(\"e\") else . end",
        "[1]\n",
        "",
        0,
    ),
    (
        "def f: ([1] as [$a] ?// $a | $a) | .; (f | if . == 1 then error(\"e\") else . end) + 0",
        "",
        "jq: error (at <unknown>): array ([1]) and number (0) cannot be added\n",
        5,
    ),
];

#[test]
fn settling_keeps_jqs_order_of_effects_and_its_retries_7787() -> Result<()> {
    for (program, stdout, stderr, status) in SEMANTICS {
        let (out, err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*stdout, *stderr, *status),
            "{program}"
        );
    }
    Ok(())
}

/// `input` reads, in order, around settled calls. `(program, stdout, stderr, exit status)`.
const INPUTS: &[(&str, &str, &str, i32)] = &[
    ("def f: input; [f + f], [f]", "[3]\n[3]\n", "", 0),
    ("def f: [input, input]; f + [input]", "[2,3,1]\n", "", 0),
    ("def f: input | input_line_number; [f + f]", "[3]\n", "", 0),
    (
        "def f: try input catch \"none\"; [f, f, f, f, f]",
        "[1,2,3,\"none\",\"none\"]\n",
        "",
        0,
    ),
];

#[test]
fn settled_calls_read_input_in_jqs_order_7787() -> Result<()> {
    for (program, stdout, stderr, status) in INPUTS {
        let (out, err, code) = run_jq_full(&["-nc", program], Some("1\n2\n3\n"))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*stdout, *stderr, *status),
            "{program}"
        );
    }
    Ok(())
}
