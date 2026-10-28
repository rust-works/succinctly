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
/// raised, and the refusal stays an error under `try` and `?` in yq mode too.
#[test]
fn test_a_call_that_never_ends_still_ends_in_yq_mode_5379() -> Result<()> {
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr("def f: f; try f catch \"caught\"", "a: 1\n", &[])?;
    assert_eq!((stdout.as_str(), code), ("", 1), "{stderr:?}");
    assert!(
        stderr.contains("f/0 exceeded maximum recursion depth"),
        "{stderr:?}"
    );
    Ok(())
}

/// A stage that leaves a node of the document and a getter of its metadata after it: the loop
/// walks a pipe stage by stage, which would drop the node's position, so a pipe that reads
/// metadata is left to the evaluator that carries it (#4696). The same pipe outside a def is the
/// reference (`.c | .y += 1 | line` is 4 in yq v4.53.3).
#[test]
fn test_a_getter_after_a_settled_stage_keeps_the_nodes_position_5379() -> Result<()> {
    let doc = "a: 0\nb: [1, 2, 3]\nc:\n  x: 1\n  y: 2\nn: 0\n";
    for (filter, want) in [
        (
            "def f: if .x < 0 then f else (.y += 1 | line) end; .c | f",
            "4\n",
        ),
        (
            ".b | def f: if length > 0 then (sort | line) else f end; f",
            "2\n",
        ),
        (".c | .y += 1 | line", "4\n"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, doc, &["--jq-extensions"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "`{filter}`: {stderr:?}");
    }
    Ok(())
}

/// An index whose key yields nothing answers every child in yq, so a computed key is not a stage
/// the loop runs to its one value: the loop leaves it to the evaluator and the answer is
/// yq's.
#[test]
fn test_an_index_with_no_key_is_not_run_to_one_value_5379() -> Result<()> {
    let doc = "a: [1, 2, 3]\nb: x\nn: 2\n";
    for filter in [
        "def g: .; def f: (.a[select(false)]) | g; f",
        "def g: .; def f: if (.a[select(false)] | length) > 0 then . else g end; f | .n",
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, doc, &["-o", "json", "-I0"])?;
        assert_eq!(code, 0, "`{filter}`: {stderr:?}");
        assert!(!stdout.is_empty(), "`{filter}`: {stdout:?}");
    }
    let (stdout, _, code) = run_yq_stdin_with_stderr(
        "def g: .; def f: (.a[select(false)]) | g; f",
        doc,
        &["-o", "json", "-I0"],
    )?;
    assert_eq!((stdout.as_str(), code), ("1\n2\n3\n", 0));
    Ok(())
}
