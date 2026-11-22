//! #7460: a tree recursion that pipes one call of itself into the next (`f($n - 1) | f($n - 1)`)
//! ran the second call inside the first call's output sink, so every call made stayed on the native
//! stack until the outermost returned, and 8,192 calls at depth 13 were refused with `f/1 exceeded
//! maximum recursion depth`. The expected outputs are jq 1.7.1's (`/usr/bin/jq`), and jq 1.8.2's
//! are the same.

use anyhow::Result;

use crate::run_jq_full;

/// `(program, jq -nc stdout)`.
const ROWS: &[(&str, &str)] = &[
    (
        "def f($n): if $n == 0 then . + 1 else f($n - 1) | f($n - 1) end; 0 | f(13)",
        "8192\n",
    ),
    (
        "def f($n): if $n == 0 then . + 1 else f($n - 1) | f($n - 1) end; 0 | f(16)",
        "65536\n",
    ),
    // Bare parameters: the argument is read lazily, a chain of closures one link a level.
    (
        "def f(n): if n == 0 then . + 1 else f(n - 1) | f(n - 1) end; 0 | f(14)",
        "16384\n",
    ),
    // A depth-first walk that threads its count through the value, 10,000 nodes deep 13.
    (
        "def dfs($m; $i): (if $i <= $m then .c += 1 else . end) \
         | if $i > $m then . else dfs($m; $i * 2) | dfs($m; $i * 2 + 1) end; \
         {c: 0} | dfs(10000; 1) | .c",
        "10000\n",
    ),
    // Each level binds what it computes with `as`, and a local def recurses (Rosetta Code's
    // Peano curve, three ways where it has nine).
    (
        "def peano($lg): def p($x; $lg): if $lg == 1 then . + [$x] \
         else (($lg / 3) | floor) as $lg | p($x; $lg) | p($x + $lg; $lg) | p($x + 2 * $lg; $lg) \
         end; p(0; $lg); [] | peano(19683) | length",
        "19683\n",
    ),
    // A memoised recursion whose calls are `as` sources and whose answer is stored with
    // `setpath` (Rosetta Code's Hofstadter Q sequence; a release build stopped at `Q(1000)`).
    (
        "def Q: def Q(n): n as $n | (if . == null then [1, 1, 1] else . end) as $q \
         | if $q[$n] != null then $q else $q | Q($n - 1) as $q1 | $q1 | Q($n - 2) as $q2 \
         | $q2 | Q($n - $q2[$n - 1]) as $q3 | $q3 | Q($n - $q3[$n - 2]) as $q4 \
         | ($q4[$n - $q4[$n - 1]] + $q4[$n - $q4[$n - 2]]) as $ans | $q4 | setpath([$n]; $ans) end; \
         . as $n | null | Q($n) | .[$n]; 1000 | Q",
        "502\n",
    ),
    // Three calls in a row, and a call in parentheses.
    (
        "def f($n): if $n == 0 then . + 1 else f($n - 1) | (f($n - 1)) | f($n - 1) end; 0 | f(9)",
        "19683\n",
    ),
];

#[test]
fn a_recursion_piping_one_call_into_the_next_holds_its_depth_7460() -> Result<()> {
    for (program, stdout) in ROWS {
        let (out, err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((out.as_str(), code), (*stdout, 0), "{program}: {err}");
    }
    Ok(())
}

/// The protection stays: a recursion a million calls deep is refused with the resource limit's
/// words (jq 1.7.1 answers 1000000 there, on a stack of its own on the heap), not an overflow.
#[test]
fn a_million_deep_recursion_is_still_refused_7460() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &[
            "-nc",
            "def f($n): if $n == 0 then 0 else 1 + f($n - 1) end; f(1000000)",
        ],
        None,
    )?;
    assert_eq!(
        (out.as_str(), code, err.trim_end()),
        (
            "",
            5,
            "jq: error (at <unknown>): f/1 exceeded maximum recursion depth"
        )
    );
    Ok(())
}
