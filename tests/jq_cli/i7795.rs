//! #7795: #7744 settled a call whose definition's body is `first(f)` without looking inside `f`, so
//! an error raised after the call no longer retried a `?//` in `f` with its next pattern. The
//! expected outputs are jq 1.7.1's (`/usr/bin/jq`), and jq 1.8.2's are the same.

use anyhow::Result;

use crate::run_jq_full;

/// `(program, stdout, stderr, exit status)`.
const ROWS: &[(&str, &str, &str, i32)] = &[
    (
        "def f: first([1] as [$a] ?// $a | $a); f | if . == 1 then error(\"e\") else . end",
        "[1]\n",
        "",
        0,
    ),
    (
        "def f: first([1] as [$a] ?// $a | $a); [f | if . == 1 then error(\"e\") else . end]",
        "[[1]]\n",
        "",
        0,
    ),
    // `last` runs the bind to its end, and jq does not retry through it.
    (
        "def f: last([1] as [$a] ?// $a | $a); f | if . == 1 then error(\"e\") else . end",
        "",
        "jq: error (at <unknown>): e\n",
        5,
    ),
];

#[test]
fn a_def_of_first_around_a_retrying_bind_is_retried_7795() -> Result<()> {
    for (program, stdout, stderr, status) in ROWS {
        let (out, err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*stdout, *stderr, *status),
            "{program}"
        );
    }
    Ok(())
}
