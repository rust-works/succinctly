//! #7052: under `-n` yq's input is an empty node, the one an input holding no document reads as, so
//! `yq -n .` prints a blank line (not `null`) and every read of it answers as it does for
//! `printf '' | yq`. Each row is captured from yq v4.53.3 (stdout and exit status); stdin carries a
//! document that `-n` must not read.

use anyhow::Result;

use crate::run_yq_stdin;

/// `(filter, extra args, stdout)` of `yq -n [args] FILTER`, all of exit status 0.
const ROWS: &[(&str, &[&str], &str)] = &[
    (".", &[], "\n"),
    ("(.)", &[], "\n"),
    (". as $x | $x", &[], "\n"),
    (". | select(. == null)", &[], "\n"),
    ("with(.; .)", &[], "\n"),
    ("[.]", &[], "-\n"),
    ("[., 1]", &[], "-\n- 1\n"),
    (r#"{"a": .}"#, &[], "a:\n"),
    (".a", &[], "null\n"),
    (".a.b", &[], "null\n"),
    ("tag", &[], "!!null\n"),
    ("type", &[], "!!null\n"),
    ("length", &[], "0\n"),
    ("line", &[], "0\n"),
    ("column", &[], "0\n"),
    ("file_index", &[], "0\n"),
    (". == null", &[], "true\n"),
    ("select(.)", &[], ""),
    (". // 1", &[], "1\n"),
    (". + 1", &[], "1\n"),
    ("1", &[], "1\n"),
    (".", &["-p=csv"], "\n"),
    (".", &["-p=json"], "\n"),
    (".", &["-p=json", "-o=json"], "null\n"),
    (".", &["-p=toml"], "\n"),
    (".", &["-o=json"], "null\n"),
    ("[.]", &["-o=json", "-I=0"], "[null]\n"),
    (".", &["-o=lua"], "return nil;\n"),
    ("[.]", &["-o=lua"], "return {\n\tnil,\n};\n"),
];

#[test]
fn null_input_is_the_empty_node_7052() -> Result<()> {
    for &(filter, args, expected) in ROWS {
        let mut all = vec!["-n"];
        all.extend_from_slice(args);
        let (stdout, code) = run_yq_stdin(filter, "a: 1\n", &all)?;
        assert_eq!(
            (stdout.as_str(), code),
            (expected, 0),
            "yq -n {args:?} {filter:?}"
        );
    }
    Ok(())
}

/// Without a filter `-n` evaluates `.`, which prints the blank line as well.
#[test]
fn null_input_without_a_filter_prints_the_empty_line_7052() -> Result<()> {
    let (stdout, code) = run_yq_stdin("", "a: 1\n", &["-n"])?;
    assert_eq!((stdout.as_str(), code), ("\n", 0));
    Ok(())
}

/// `-n` and an input holding no document are the same run, as far as the output goes.
#[test]
fn null_input_prints_what_an_empty_input_prints_7052() -> Result<()> {
    for filter in [
        ".",
        "[.]",
        r#"{"a": .}"#,
        "tag",
        ".a",
        "select(.)",
        "[., 1]",
    ] {
        let (null_input, null_code) = run_yq_stdin(filter, "a: 1\n", &["-n"])?;
        let (empty, empty_code) = run_yq_stdin(filter, "", &[])?;
        assert_eq!(
            (null_input, null_code),
            (empty, empty_code),
            "yq -n {filter:?} against an empty input"
        );
    }
    Ok(())
}

/// `-e` reads the empty node as no match, as in yq: the blank line is printed and the exit status is 1.
#[test]
fn null_input_under_exit_status_is_no_match_7052() -> Result<()> {
    let (stdout, code) = run_yq_stdin(".", "a: 1\n", &["-n", "-e"])?;
    assert_eq!((stdout.as_str(), code), ("\n", 1));
    Ok(())
}
