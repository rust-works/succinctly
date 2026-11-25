//! #7928: a `try` whose body yields and then raises (`try (g + (1, error("x"))) catch 0`) yields
//! twice in jq: the body's output, then the handler's on the error the `try` catches. #7787 taught
//! the settle question that a `,` with an error after its output answers once, and the `try` arm
//! still took a body that answers once and a handler that answers once to answer once. A closure
//! argument of that shape passed down a doubly recursive def made every call look single-valued,
//! so `first(f(null; 4) | select(. > 30))` ran every call to all of its outputs first: 14,929,920
//! values at depth 4, about 16 s, where jq 1.7.1 answers `32` at once. A `try` with a handler that
//! yields now settles only over a body that cannot raise after its output.
//!
//! The expected outputs are jq 1.7.1's (`/usr/bin/jq`); jq 1.8.2 answers every row the same.

use anyhow::Result;

use crate::run_jq_full;

/// `(program, stdout)`, each with exit status 0 and nothing on stderr.
const CASES: &[(&str, &str)] = &[
    // The issue's program, and deeper: each ran every output of every call first.
    (
        "def f(g; $n): if $n > 0 then f(try (g + (1, error(\"x\"))) catch 0; $n - 1) + f(g; $n - 1) else g end; first(f(null; 4) | select(. > 30))",
        "32\n",
    ),
    (
        "def f(g; $n): if $n > 0 then f(try (g + (1, error(\"x\"))) catch 0; $n - 1) + f(g; $n - 1) else g end; first(f(null; 6) | select(. > 30))",
        "192\n",
    ),
    (
        "def f(g; $n): if $n > 0 then f(try (g + (1, error(\"x\"))) catch 0; $n - 1) + f(g; $n - 1) else g end; [limit(6; f(null; 3))]",
        "[12,11,10,9,11,10]\n",
    ),
    (
        "def f(g; $n): if $n > 0 then f(try (g + (1, error(\"x\"))) catch 0; $n - 1) + f(g; $n - 1) else g end; [f(null; 3)] | length",
        "864\n",
    ),
    // The handler runs after the output, for an error and for a `break` alike.
    ("def f: try (1, error(\"x\")) catch 0; [f]", "[1,0]\n"),
    ("def f: label $l | try (1, break $l) catch 0; [f]", "[1,0]\n"),
    ("def f: try (1, error(\"x\")) catch empty; [f]", "[1]\n"),
    // Through a helper, a recursion and a closure argument.
    ("def g: 1, error(\"x\"); def f: try g catch 0; [f]", "[1,0]\n"),
    (
        "def f($n): if $n == 0 then (1, error(\"x\")) else try f($n - 1) catch 0 end; [f(3)]",
        "[1,0]\n",
    ),
    (
        "def f($n): if $n == 0 then 1 else try f($n - 1) catch 0 end; [f(3)]",
        "[1]\n",
    ),
    (
        "def f(g; $n): if $n > 0 then f(try g catch 0; $n - 1) else g end; [f((1, error(\"x\")); 3)]",
        "[1,0]\n",
    ),
];

#[test]
fn a_try_whose_body_raises_after_its_output_is_not_run_first_7928() -> Result<()> {
    for (program, stdout) in CASES {
        let (out, err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*stdout, "", 0),
            "{program}"
        );
    }
    Ok(())
}

/// A tree recursion over `try ... catch` still holds only its depth on the native stack: a `try`
/// whose body cannot raise after its output is still run first (depth 13, 8,192 leaves).
#[test]
fn a_tree_recursion_through_a_try_with_a_handler_still_answers_7928() -> Result<()> {
    for program in [
        "def t($n): if $n <= 0 then 1 else (try t($n-1) catch 0) + (try t($n-1) catch 0) end; [0 | t(13)] | length",
        "def t($n): if $n <= 0 then (try error(\"x\") catch 1) else t($n-1) + t($n-1) end; [0 | t(13)] | length",
        "def t($n): if $n <= 0 then (try 1 catch 2) else t($n-1) + t($n-1) end; [0 | t(13)] | length",
    ] {
        let (out, err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((out.as_str(), code), ("1\n", 0), "{program}: {err}");
    }
    Ok(())
}
