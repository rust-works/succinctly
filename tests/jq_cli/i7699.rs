//! #7699: a call to a `def` whose body reads none of its parameters by the bare name, with no
//! literal argument, binds its `$` parameters with their `as` wrappers alone, without the
//! substitution walk that only copied the body. These are the shapes that route takes, and their
//! neighbours on the route it leaves (a bare parameter read, a literal argument); the expected
//! outputs are jq 1.7.1's (`/usr/bin/jq`, input `0`), and jq 1.8.2's are the same.

use anyhow::Result;

use crate::run_jq_full;

/// `(program, jq -c stdout on input 0)`.
const ROWS: &[(&str, &str)] = &[
    (
        "def f($n): if $n == 0 then . + 1 else f($n - 1) | f($n - 1) end; 0 | f(10)",
        "1024\n",
    ),
    (
        "def f($a; $b): [$a, $b]; [f(1, 2; 3, 4)]",
        "[[1,3],[1,4],[2,3],[2,4]]\n",
    ),
    (
        "def f($n): def g($n): $n * 10; [g($n + 1), $n]; f(. + 1)",
        "[20,1]\n",
    ),
    (
        "def f($n): reduce range($n) as $n (0; . + $n) + $n; f(. + 4)",
        "10\n",
    ),
    ("def f($n): [$n, $__loc__.line]; f(. + 1)", "[1,1]\n"),
    ("def f($n): . as $n | [$n]; 5 | f(. + 1)", "[5]\n"),
    (
        "def f($n): if $n > 2 then [$n] else [$n] + f($n + 1) end; f(. * 0)",
        "[0,1,2,3]\n",
    ),
    (
        "def f($n): path(.[$n]); [3, 4] | [f(. | length - 1)]",
        "[[1]]\n",
    ),
    ("def f($a): 1; try f(error(\"boom\")) catch .", "\"boom\"\n"),
    ("def f($a): $a; [f(empty)]", "[]\n"),
    ("def f($x): .[$x] |= . + 1; [1, 2] | f(0 + 1)", "[1,3]\n"),
    (
        "def f($n; g): if $n == 0 then g else f($n - 1; g + 1) end; 0 | f(. + 5; .)",
        "5\n",
    ),
    ("def f($n): [$n, n]; f(. + 2)", "[2,2]\n"),
    (
        "def f($n): label $out | if $n > 3 then break $out else $n end; [range(6) | f(. + 0)]",
        "[0,1,2,3]\n",
    ),
    (
        "def fib($n): if $n < 2 then $n else fib($n - 1) + fib($n - 2) end; fib(. + 15)",
        "610\n",
    ),
    (
        "def f($n): if $n == 0 then 1 else [f($n - 1), f($n - 1)] | add end; f(. + 8)",
        "256\n",
    ),
    // A definition already bound into the body (`g`): its argument is searched for a bare read,
    // its body is not. And a body holding an outer parameter's argument (`a`, a `Shared` node).
    ("def g(x): x; def f($n): [g(n), $n]; f(. + 1)", "[1,1]\n"),
    ("def g(x): x; def f($n): [g($n), $n]; f(. + 1)", "[1,1]\n"),
    ("def o(a): def f($n): [a, $n]; f(. + 1); o(5)", "[5,1]\n"),
];

#[test]
fn a_def_with_nothing_to_substitute_answers_as_jq_does_7699() -> Result<()> {
    for (program, stdout) in ROWS {
        let (out, err, code) = run_jq_full(&["-c", program], Some("0"))?;
        assert_eq!((out.as_str(), code), (*stdout, 0), "{program}: {err}");
    }
    Ok(())
}
