//! #7337: a `?//` bind that opens the rest of a pipe over a `group_by` group, or any other lazy
//! sequence, retries its next alternative when a later stage raises, as jq's does.
//!
//! The bind used to run to completion on the group before the next stage saw its first output, so
//! the error of `.b + 1` over the first alternative's `{"a":1,"b":"y"}` was reported as it was
//! (`string ("y") and number (1) cannot be added`) where jq retries the second alternative, `$x`
//! bound to the whole group, and reports `Cannot index array with string "b"`.
//!
//! Every expected line is jq 1.7.1's (`/usr/bin/jq -c`, input on stdin).

use anyhow::Result;

use crate::run_jq_full;

const GROUPS: &str = r#"[{"a":2,"b":"x"},{"a":1,"b":"y"},{"a":2,"b":"z"}]"#;
const ARRAYS: &str = r#"[[{"a":2,"b":"x"}],[{"a":1,"b":"y"}],[{"a":3,"b":"z"}]]"#;

/// The bind, then a stage that fails on the first alternative's value and on the second's alike
/// (an object has a `.b` that is no number; an array has no `.b`).
const BIND: &str = r"(. as [$x] ?// $x | $x) | .b + 1";

const RETRIED: &str = "jq: error (at <stdin>:0): Cannot index array with string \"b\"\n";

fn run(input: &str, filter: &str) -> Result<(String, String, i32)> {
    run_jq_full(&["-c", filter], Some(input))
}

/// Every spelling that hands the group on, or a group's slice, to the bind answers jq's retried
/// error: the sink route, the collecting route, a slice then `[]`, and a lone group by index.
#[test]
fn a_bind_over_a_group_retries_its_alternative_on_a_later_error_7337() -> Result<()> {
    for filter in [
        format!("[group_by(.a)[] | {BIND}]"),
        format!("group_by(.a)[] | {BIND}"),
        format!("group_by(.a) | .[0:2][] | {BIND}"),
        format!("group_by(.a) | .[1:][] | {BIND}"),
        format!("[group_by(.a)[0:2][] | {BIND}]"),
        format!("group_by(.a) | first | {BIND}"),
        format!("group_by(.a) | .[-1] | {BIND}"),
        format!("first(group_by(.a)[] | {BIND})"),
    ] {
        let (stdout, stderr, code) = run(GROUPS, &filter)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            ("", RETRIED, 5),
            "{filter}"
        );
    }
    Ok(())
}

/// The same shapes over a `map` sequence and over a plain array, which the first of these shared
/// the divergence with (a slice's `[]` left a multi-value the bind stage was then applied to one
/// stage at a time) and the second never had.
#[test]
fn a_bind_after_a_slice_and_iterate_retries_over_any_lazy_sequence_7337() -> Result<()> {
    for filter in [
        format!("map(.) | .[0:2][] | {BIND}"),
        format!(".[0:2][] | {BIND}"),
    ] {
        let (stdout, stderr, code) = run(ARRAYS, &filter)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            ("", RETRIED, 5),
            "{filter}"
        );
    }
    Ok(())
}

/// When the retried alternative succeeds, its output is what comes out: the groups' arrays have a
/// length, where their first element does not (`error("e")`).
#[test]
fn the_retried_alternative_supplies_the_output_over_a_group_7337() -> Result<()> {
    let filter = r#"[group_by(.a)[] | (. as [$x] ?// $x | $x) | if type == "array" then length else error("e") end]"#;
    let (stdout, stderr, code) = run(GROUPS, filter)?;
    assert_eq!((stdout.as_str(), stderr.as_str(), code), ("[1,2]\n", "", 0));
    Ok(())
}

/// A bind that does not retry still binds the first alternative, and a `try` around the whole
/// pipe catches the error the retry ends in.
#[test]
fn a_bind_over_a_group_that_needs_no_retry_and_one_under_try_are_unchanged_7337() -> Result<()> {
    for (filter, expected) in [
        (
            "[group_by(.a)[] | (. as [$x] ?// $x | $x) | .a]".to_string(),
            "[1,2]\n",
        ),
        (
            r#"[group_by(.a)[] | (. as [$x] ?// $x | $x) | select(.b == "z")]"#.to_string(),
            "[]\n",
        ),
        (
            format!(r#"[group_by(.a)[] | try ({BIND}) catch "caught"]"#),
            "[\"caught\",\"caught\"]\n",
        ),
    ] {
        let (stdout, stderr, code) = run(GROUPS, &filter)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (expected, "", 0),
            "{filter}"
        );
    }
    Ok(())
}

/// The retry is per value, and what an earlier value already answered stays: the group of `a == 1`
/// answers `"y"` on its first alternative, the next group's first alternative raises `late`, and
/// its second (the group as an array) raises `arr`, which is the error jq reports after the `"y"`.
#[test]
fn an_earlier_groups_output_survives_a_later_groups_retried_error_7337() -> Result<()> {
    let filter = r#"group_by(.a) | .[0:2][] | (. as [$x] ?// $x | $x) | if type == "object" then (if .a == 1 then .b else error("late") end) else error("arr") end"#;
    let (stdout, stderr, code) = run(GROUPS, filter)?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        ("\"y\"\n", "jq: error (at <stdin>:0): arr\n", 5)
    );
    // An error raised by a generator after the groups (`error("boom")`) is not a bind's to retry.
    let filter = r#"[(group_by(.a)[0:2][], error("boom")) | (. as [$x] ?// $x | $x) | .a]"#;
    let (stdout, stderr, code) = run(GROUPS, filter)?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        ("", "jq: error (at <stdin>:0): boom\n", 5)
    );
    Ok(())
}

/// A consumer that stops after the first output is an error the bind retries on, in jq: `limit(1; ..)`
/// reaches the second alternative, whose `.a` over the group's array raises. The slice-then-iterate
/// route used to answer `[1]`, as it ran the bind over every value before any stop could reach it.
#[test]
fn a_stop_after_the_first_output_retries_the_bind_over_a_slice_of_groups_7337() -> Result<()> {
    let bound = r"(. as [$x] ?// $x | $x)";
    let retried = "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n";
    let filter = format!("[limit(1; group_by(.a) | .[0:3][] | {bound} | .a)]");
    let (stdout, stderr, code) = run(GROUPS, &filter)?;
    assert_eq!((stdout.as_str(), stderr.as_str(), code), ("", retried, 5));
    // `first` prints its one output before the stop is retried into the same error.
    let filter = format!("first(group_by(.a) | .[0:3][] | {bound} | .a)");
    let (stdout, stderr, code) = run(GROUPS, &filter)?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        ("1\n", retried, 5)
    );
    Ok(())
}

/// The bind need not open its stage: parenthesised twice, or inside a sub-pipe, it still has to be
/// live when the next stage raises. Over a plain array these always retried; a group did not.
#[test]
fn a_bind_inside_parentheses_or_a_sub_pipe_retries_over_a_group_7337() -> Result<()> {
    let bound = r"(. as [$x] ?// $x | $x)";
    for filter in [
        format!("group_by(.a)[] | (({bound})) | .b + 1"),
        format!("group_by(.a) | .[0:2][] | (({bound})) | .b + 1"),
        format!("[group_by(.a)[] | ({bound} | .b) | . + 1]"),
        format!("group_by(.a)[] | (.[0:1] | {bound}) | .b + 1"),
    ] {
        let (stdout, stderr, code) = run(GROUPS, &filter)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            ("", RETRIED, 5),
            "{filter}"
        );
    }
    Ok(())
}
