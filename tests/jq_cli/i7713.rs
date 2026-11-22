//! #7713: a tree recursion whose definition's body (or a helper it calls) uses a shape the settle
//! test of #7460 did not read -- `try`, `?`, `label`, `reduce`, an object, `first`, `//`, a builtin
//! that answers once, a def declared in the body, a call with arguments, a module's forwarding
//! stub -- or that recurses through a pipe operand (`(.[0] | count) + ...`), ran each call inside
//! the other's output sink and was refused with `exceeded maximum recursion depth` at about 8,192
//! leaves. The expected outputs are jq 1.7.1's (`/usr/bin/jq`), and jq 1.8.2's are the same.

use anyhow::Result;

use crate::run_jq_full;

/// The leaves `own` is defined with in [`a_tree_recursion_over_any_leaf_answers_7713`].
const LEAVES: &[&str] = &[
    "1",
    "try 1",
    "def h: 1; h",
    "reduce (1) as $x (0; $x)",
    "label $l | 1",
    "{a: 1}.a",
    "error(\"x\")? // 1",
    "[1] | .[0]",
    "first(1)",
    "1 // 2",
    "tostring",
    "length",
];

/// `t($n - 1) + t($n - 1)` and `t($n - 1) | t($n - 1)` over each leaf, 8,192 leaves deep 13.
#[test]
fn a_tree_recursion_over_any_leaf_answers_7713() -> Result<()> {
    for leaf in LEAVES {
        for (comb, want) in [
            ("t($n - 1) + t($n - 1)", plus_answer(leaf)),
            ("t($n - 1) | t($n - 1)", pipe_answer(leaf).to_string()),
        ] {
            let program = format!(
                "def own: {leaf}; def t($n): if $n == 0 then own else {comb} end; 0 | t(13)"
            );
            let (out, err, code) = run_jq_full(&["-nc", &program], None)?;
            assert_eq!((out.as_str(), code), (want.as_str(), 0), "{program}: {err}");
        }
    }
    Ok(())
}

/// What jq 1.7.1 answers for the `+` combination: 8,192 leaves of `own`'s value on `0`.
fn plus_answer(leaf: &str) -> String {
    match leaf {
        "tostring" => format!("\"{}\"\n", "0".repeat(8192)),
        "length" => "0\n".to_string(),
        _ => "8192\n".to_string(),
    }
}

/// What jq 1.7.1 answers for the `|` combination: `own` on `0`, piped through itself.
fn pipe_answer(leaf: &str) -> &'static str {
    match leaf {
        "tostring" => "\"0\"\n",
        "length" => "0\n",
        _ => "1\n",
    }
}

/// The hunt's repro: counting the leaves of a binary tree 13 deep, through pipe operands.
#[test]
fn counting_a_binary_tree_through_pipe_operands_answers_7713() -> Result<()> {
    let mut tree = "1".to_string();
    for _ in 0..13 {
        tree = format!("[{tree},{tree}]");
    }
    for (program, want) in [
        (
            "def count: if type == \"array\" then (.[0] | count) + (.[1] | count) else 1 end; count",
            "8192\n",
        ),
        (
            "def sum: if type == \"array\" then (.[0] | sum) + (.[1] | sum) else . end; sum",
            "8192\n",
        ),
    ] {
        let (out, err, code) = run_jq_full(&["-c", program], Some(&tree))?;
        assert_eq!((out.as_str(), code), (want, 0), "{program}: {err}");
    }
    Ok(())
}

/// The shapes now settled keep jq's order and its errors: an error after the call is not caught
/// by a `try` inside it, a generator still yields every output, an error in an argument still
/// raises. `(program, stdout, exit status, stderr)`.
const SEMANTICS: &[(&str, &str, i32, &str)] = &[
    (
        "def f: try 1; f | error(\"x\")",
        "",
        5,
        "jq: error (at <unknown>): x\n",
    ),
    (
        "def f: 1?; f | error(\"y\")",
        "",
        5,
        "jq: error (at <unknown>): y\n",
    ),
    (
        "def f: label $l | 1; f | error(\"z\")",
        "",
        5,
        "jq: error (at <unknown>): z\n",
    ),
    (
        "def f: try 1; try (f | error(\"x\")) catch \"outer\"",
        "\"outer\"\n",
        0,
        "",
    ),
    ("def f: try (1, 2); [f, f]", "[1,2,1,2]\n", 0, ""),
    ("def f: first(1, error(\"q\")); f + 1", "2\n", 0, ""),
    (
        "def f: {a: (1, 2), b: (3, 4)}; [f]",
        "[{\"a\":1,\"b\":3},{\"a\":1,\"b\":4},{\"a\":2,\"b\":3},{\"a\":2,\"b\":4}]\n",
        0,
        "",
    ),
    (
        "def f: reduce (1, 2) as $x (0, 100; . + $x); [f + f]",
        "[6,106,106,206]\n",
        0,
        "",
    ),
    (
        "def f: label $l | (1, 2, break $l, 3); [f + f]",
        "[2,3,3,4]\n",
        0,
        "",
    ),
    (
        "def g: def h: 1; def h: (2, 3); h; [g + g]",
        "[4,5,5,6]\n",
        0,
        "",
    ),
    (
        "def f($x): try $x catch \"c\"; [f(1, error(\"e\")) + 0]",
        "",
        5,
        "jq: error (at <unknown>): e\n",
    ),
    ("def f: try error({a: 1}) catch .a; f + f", "2\n", 0, ""),
];

#[test]
fn the_settled_shapes_keep_jqs_order_and_errors_7713() -> Result<()> {
    for (program, stdout, status, stderr) in SEMANTICS {
        let (out, err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!(
            (out.as_str(), code, err.as_str()),
            (*stdout, *status, *stderr),
            "{program}"
        );
    }
    Ok(())
}

/// A leaf that calls into a module reaching a second one (`a.jq` includes `b.jq`), and a tail
/// loop whose stage does (`w.jq` includes `s.jq`): the module's forwarding stub is a def declared
/// in the caller's body.
#[test]
fn a_recursion_through_two_modules_answers_7713() -> Result<()> {
    let dir = tempfile::TempDir::new()?;
    for (name, text) in [
        ("b.jq", "def g: 1;"),
        ("a.jq", "include \"b\"; def f: g;"),
        ("s.jq", "def step: . + 1;"),
        ("w.jq", "include \"s\"; def step2: step;"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    let lib = dir.path().to_string_lossy().into_owned();
    for (program, want) in [
        (
            "include \"a\"; def t($n): if $n == 0 then f else t($n - 1) + t($n - 1) end; 0 | t(13)",
            "8192\n",
        ),
        (
            "include \"w\"; def loop: if . < 300000 then step2 | loop else . end; 0 | loop",
            "300000\n",
        ),
    ] {
        let (out, err, code) = run_jq_full(&["-nc", "-L", &lib, program], None)?;
        assert_eq!((out.as_str(), code), (want, 0), "{program}: {err}");
    }
    Ok(())
}
