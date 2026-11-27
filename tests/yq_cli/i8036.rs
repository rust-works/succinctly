//! #8036: a fold used as a path (`del(...)`'s argument, or the left side of `=` or `|=`) reads its
//! source before INIT, as #8022 made the fold as a value do. yq's `reduceOperator`
//! (`pkg/yqlib/operator_reduce.go`, v4.53.3) evaluates SRC in full, then INIT, then UPDATE for each
//! node, so an error in SRC outranks INIT's and INIT's outranks UPDATE's; `succinctly yq` resolved
//! the fold as a path INIT first, so `del(error("s") as $x ireduce (error("i"); .))` was
//! `Error: i`. Every row is captured from yq v4.53.3, as `yq -o=json -I0 FILTER` on `[1, 2]`.

use anyhow::Result;

use crate::{run_jq_stdin_with_stderr, run_yq_stdin_with_stderr};

const INPUT: &str = "[1, 2]\n";
const JSON: &[&str] = &["-o", "json", "-I0"];

/// Filters yq v4.53.3 fails with exactly this error and no output.
const ERROR_ROWS: &[(&str, &str)] = &[
    // An error in SRC wins over INIT's...
    (
        "del(error(\"s\") as $x ireduce (error(\"i\"); .))",
        "Error: s",
    ),
    (
        "(error(\"s\") as $x ireduce (error(\"i\"); .)) = 9",
        "Error: s",
    ),
    (
        "(error(\"s\") as $x ireduce (error(\"i\"); .)) |= 9",
        "Error: s",
    ),
    (
        "del((.[], error(\"s\")) as $x ireduce ((error(\"i\"), .); error(\"u\")))",
        "Error: s",
    ),
    // ...is read when INIT yields nothing...
    (
        "del(error(\"s\") as $x ireduce (select(false); .))",
        "Error: s",
    ),
    // ...and after UPDATE failed or dropped every accumulator.
    (
        "del((1, error(\"s\")) as $x ireduce (.; error(\"u\")))",
        "Error: s",
    ),
    (
        "((1, error(\"s\")) as $x ireduce (.; error(\"u\"))) |= 9",
        "Error: s",
    ),
    (
        "del((.[], error(\"s\")) as $x ireduce (.; select(false)))",
        "Error: s",
    ),
    // With no error in SRC, INIT's comes next, even after an output INIT made first.
    (
        "del(.[] as $x ireduce (error(\"i\"); error(\"u\")))",
        "Error: i",
    ),
    (
        "del(.[] as $x ireduce ((., error(\"i\")); error(\"u\")))",
        "Error: i",
    ),
    ("del([] as $x ireduce (error(\"i\"); .))", "Error: i"),
    // Then UPDATE's, over an empty accumulator list too (#8035).
    ("del(.[] as $x ireduce (.; error(\"u\")))", "Error: u"),
    (
        "del(.[] as $x ireduce (.; select($x == 2) | error(\"u\")))",
        "Error: u",
    ),
    (
        "del(.[] as $x ireduce (select(false); error(\"u\")))",
        "Error: u",
    ),
    (
        "del(.[] as $x ireduce (select(false); error($x)))",
        "Error: 1",
    ),
];

#[test]
fn a_fold_as_a_path_reads_its_source_before_init_8036() -> Result<()> {
    for (filter, error) in ERROR_ROWS {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, INPUT, JSON)?;
        assert_eq!(
            (stdout.as_str(), stderr.trim_end(), code),
            ("", *error, 1),
            "{filter}"
        );
    }
    Ok(())
}

/// A fold as a path with no error answers as before, in both tools: `.` folded over the source
/// is the root, which `del` deletes; an empty source leaves INIT's `.`, whose UPDATE never runs;
/// and a collect over an empty accumulator list is a fresh node, which deletes nothing.
#[test]
fn a_fold_as_a_path_without_errors_is_unchanged_8036() -> Result<()> {
    for (filter, expected) in [
        ("del(.[] as $x ireduce (.; .))", ""),
        ("del(select(false) as $x ireduce (.; error(\"u\")))", ""),
        ("del(.[] as $x ireduce (select(false); [.]))", "[1,2]\n"),
        ("(.[] as $x ireduce (0; .)) = 9", "[1,2]\n"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, INPUT, JSON)?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}: {stderr}");
    }
    Ok(())
}

/// An INIT that yields nothing as a path here only because it reads its position (`parent`,
/// `key`, `path`) is not taken for an empty one: yq's INIT yields a node there, so UPDATE never
/// runs over an empty list (`[.] | .[0] | keys` over it would raise that `null` has no keys).
/// Such a fold answers as before: on `{"a": {"b": 1}}` yq v4.53.3 leaves the first two unchanged,
/// exit 0. `$__loc__` is no builtin in yq, so an INIT of it is empty in both tools and UPDATE runs
/// over the empty list (`Error: u` in yq).
#[test]
fn an_init_that_reads_its_position_is_not_taken_for_empty_8036() -> Result<()> {
    let input = "{\"a\": {\"b\": 1}}\n";
    for filter in [
        "del(.a as $x ireduce ((.a.b | parent); [.] | .[0] | keys))",
        "(.a as $x ireduce ((.a.b | parent); [.] | .[0] | keys)) = 9",
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, input, JSON)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            ("{\"a\":{\"b\":1}}\n", "", 0),
            "{filter}"
        );
    }
    for filter in [
        "del(.a as $x ireduce ($__loc__; error(\"u\")))",
        "(.a as $x ireduce ($__loc__; error(\"u\"))) = 9",
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, input, JSON)?;
        assert_eq!(
            (stdout.as_str(), stderr.trim_end(), code),
            ("", "Error: u", 1),
            "{filter}"
        );
    }
    Ok(())
}

/// Only an ordinary error waits for the rest of the source, as on the value route (#8022): a
/// `break` and a `halt` (behind `--jq-extensions`; yq has neither) end the fold at once, so the
/// source's later error never outranks them.
#[test]
fn break_and_halt_end_a_fold_as_a_path_at_once_8036() -> Result<()> {
    let ext = &["--jq-extensions", "-o", "json", "-I0"];
    for (filter, expected_stderr, expected_code) in [
        (
            "label $f | del((1, 2, error(\"s\")) as $x ireduce (.; break $f))",
            "",
            0,
        ),
        (
            "del((1, error(\"s\")) as $x ireduce (.; \"bye\" | halt_error))",
            "bye",
            1,
        ),
        (
            "del((1, error(\"s\")) as $x ireduce ((\"bye\" | halt_error); .))",
            "bye",
            1,
        ),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, INPUT, ext)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            ("", expected_stderr, expected_code),
            "{filter}"
        );
    }
    Ok(())
}

/// jq's `reduce` as a path still evaluates INIT first (jq 1.7.1: `i`, `u`, and nothing with
/// exit 0 for an empty INIT, whose source is never read).
#[test]
fn jq_reduce_as_a_path_keeps_init_first_8036() -> Result<()> {
    for (filter, expected_stderr, expected_code) in [
        (
            "path(reduce error(\"s\") as $x (error(\"i\"); .))",
            Some("i"),
            5,
        ),
        (
            "path(reduce (1, error(\"s\")) as $x (.; error(\"u\")))",
            Some("u"),
            5,
        ),
        ("path(reduce error(\"s\") as $x (empty; .))", None, 0),
    ] {
        let (stdout, stderr, code) = run_jq_stdin_with_stderr(filter, INPUT, &["-c"])?;
        assert_eq!((stdout.as_str(), code), ("", expected_code), "{filter}");
        match expected_stderr {
            Some(message) => assert!(
                stderr.starts_with("jq: error")
                    && stderr.trim_end().ends_with(&format!(": {message}")),
                "{filter}: {stderr}"
            ),
            None => assert_eq!(stderr, "", "{filter}"),
        }
    }
    Ok(())
}
