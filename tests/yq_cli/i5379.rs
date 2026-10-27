//! #5379: a call with no arguments, last in its body, loops in place in `succinctly yq` too.
//! `def` is a succinctly extension in yq mode (yq's lexer rejects it), and the extension follows
//! jq, so these rows have no yq capture to be checked against: the jq answers are jq 1.7.1's.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

#[test]
fn test_a_tail_recursive_def_runs_past_the_frame_cap_in_yq_mode_5379() -> Result<()> {
    for (filter, doc, want) in [
        (
            "def f: if . < 200000 then . + 1 | f else . end; .a | f",
            "a: 0\n",
            "200000\n",
        ),
        (
            "def f: if . < 200000 then . + 1 | f else . end; def g: f; .a.b | g",
            "a:\n  b: 0\n",
            "200000\n",
        ),
        (
            "def f: if .n < 200000 then .n += 1 | f else .n end; f",
            "n: 0\n",
            "200000\n",
        ),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, doc, &["--jq-extensions"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "`{filter}`: {stderr:?}");
    }
    Ok(())
}

/// A tail call that never ends is stopped by the round budget with the refusal the nested form
/// raised, and the program stays an error under `try`.
#[test]
fn test_a_call_that_never_ends_still_ends_in_yq_mode_5379() -> Result<()> {
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr("def f: [f]; try f catch \"caught\"", "a: 1\n", &[])?;
    assert_eq!((stdout.as_str(), code), ("", 1), "{stderr:?}");
    Ok(())
}
