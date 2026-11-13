//! #7025: `--header-preprocess=false` reads a YAML input as an ordinary stream. yq's default slurps the
//! blank lines, comments, `%YAML` and `---` lines before the first content line as a header and prints
//! them back verbatim; with the flag off there is no header, so the leading comments are the first
//! node's head comment, a leading `---` is a marker that is read and not echoed, and the break after a
//! leading `--- text` line is kept (`--- x` / `y` is `"x y"`, not the default's `"xy"`). It is the
//! reading yq gives a byte-order-marked file, which has no header either (#5357).
//!
//! Every row's expected output was written by yq v4.53.3 with the flag; a row that yq refuses is
//! checked for the refusal alone. What still differs, and is listed in `limitations.md`: a comment or
//! text on a `---` line (yq keeps `--- # c` as the head comment), `%YAML` with no `---` after it (an
//! error in yq), and the `line` of a document with no content.

use anyhow::Result;
use std::io::Write;
use tempfile::NamedTempFile;

use crate::{run_yq_stdin, run_yq_stdin_bytes_with_stderr};

/// `(input, flags and filter, expected output)`; the flag itself is added by the test.
type Row = (&'static str, &'static [&'static str], &'static str);

const ROWS: &[Row] = &[
    ("# lead\n\na: 1\n", &["."], "# lead\na: 1\n"),
    ("# lead\n\na: 1\n", &["-P", "."], "# lead\na: 1\n"),
    ("# lead\n\na: 1\n", &[".a"], "1\n"),
    ("# lead\n\na: 1\n", &[".a = 5"], "# lead\na: 5\n"),
    ("# lead\n\na: 1\n", &["-N", "."], "# lead\na: 1\n"),
    ("# lead\n\na: 1\n", &["-o=json", "-I0", "."], "{\"a\":1}\n"),
    (
        "# lead\n\na: 1\n",
        &["[.. | head_comment]", "-o=json", "-I0"],
        "[\"lead\",\"\"]\n",
    ),
    (
        "# lead\n# more\na: 1\nb: 2\n",
        &["[.. | head_comment]", "-o=json", "-I0"],
        "[\"\",\"\",\"\"]\n",
    ),
    ("\n\na: 1\n", &["."], "a: 1\n"),
    ("\n\na: 1\n", &["-P", "."], "a: 1\n"),
    ("\n# c\n\n- x\n- y\n", &["."], "# c\n- x\n- y\n"),
    ("\n# c\n\n- x\n- y\n", &["-P", "."], "# c\n- x\n- y\n"),
    ("---\na: 1\n", &["."], "a: 1\n"),
    ("---\na: 1\n", &["-P", "."], "a: 1\n"),
    ("---\na: 1\n", &["-N", "."], "a: 1\n"),
    ("# c\n---\na: 1\n", &["."], "# c\na: 1\n"),
    ("# c\n\n---\na: 1\n", &["."], "# c\na: 1\n"),
    ("---\n# c\na: 1\n", &["."], "# c\na: 1\n"),
    ("%YAML 1.1\n---\na: 1\n", &["."], "a: 1\n"),
    ("%YAML 1.1\n---\na: 1\n", &["-P", "."], "a: 1\n"),
    (
        "# lead\na: 1\n---\nb: 2\n",
        &["."],
        "# lead\na: 1\n---\nb: 2\n",
    ),
    (
        "# lead\na: 1\n---\nb: 2\n",
        &["-P", "."],
        "# lead\na: 1\n---\nb: 2\n",
    ),
    ("---\na: 1\n---\nb: 2\n", &["."], "a: 1\n---\nb: 2\n"),
    (
        "---\n---\na: 1\n",
        &["-o=json", "-I0", "."],
        "null\n{\"a\":1}\n",
    ),
    (
        "# lead\n\na: 1 # t\nb: 2\n# foot\n",
        &["."],
        "# lead\na: 1 # t\nb: 2\n# foot\n",
    ),
    (
        "# lead\n\na: 1 # t\nb: 2\n# foot\n",
        &["-P", "."],
        "# lead\na: 1 # t\nb: 2\n# foot\n",
    ),
    ("--- x\ny\n", &["-o=json", "-I0", "."], "\"x y\"\n"),
    ("--- x\ny\n", &["."], "x y\n"),
    ("--- x\n", &["."], "x\n"),
    ("# lead\n", &["."], "# lead\n"),
    ("# lead\n\n", &["."], "# lead\n\n"),
    ("# lead\n\n", &["-P", "."], "# lead\n\n"),
    ("\n\n\n", &["."], "\n\n\n"),
    ("#c2\n# c\n", &["."], "#c2\n# c\n"),
    ("---\n", &["."], "\n"),
    ("---\n# c\n", &["."], "\n"),
    ("# c\n---\n", &["."], "\n"),
    ("%YAML 1.1\n---\n", &["."], "\n"),
    ("", &["."], "\n"),
    ("# lead\n\na: 1\n", &["-o=props", "."], "a = 1\n"),
    (
        "# lead\n\na: 1\n",
        &["-o=json", "-I0", "[head_comment, .a]"],
        "[\"lead\",1]\n",
    ),
    (
        "# lead\n\na: &x 1\nb: *x\n",
        &["."],
        "# lead\na: &x 1\nb: *x\n",
    ),
    (
        "# lead\n\n- &x {a: 1}\n- *x\n",
        &["."],
        "# lead\n- &x {a: 1}\n- *x\n",
    ),
];

fn run_row(input: &str, args: &[&str]) -> Result<(String, i32)> {
    let flags: Vec<&str> = args
        .iter()
        .copied()
        .filter(|a| a.starts_with('-') && *a != ".")
        .collect();
    let filter = args
        .iter()
        .copied()
        .find(|a| !flags.contains(a))
        .unwrap_or(".");
    let mut all = vec!["--header-preprocess=false"];
    all.extend(flags);
    run_yq_stdin(filter, input, &all)
}

/// `succinctly yq ARGS`, with the filter before the files, as the arguments are given.
fn run_files(args: &[&str]) -> Result<(String, i32)> {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(args)
        .output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code().unwrap_or(-1),
    ))
}

#[test]
fn every_row_matches_yq_with_the_header_off_7025() -> Result<()> {
    for (input, args, expected) in ROWS {
        let (out, code) = run_row(input, args)?;
        assert_eq!(
            (out.as_str(), code),
            (*expected, 0),
            "input {input:?} args {args:?}"
        );
    }
    Ok(())
}

#[test]
fn text_after_a_leading_marker_with_a_mapping_below_is_refused_as_in_yq_7025() -> Result<()> {
    // `--- x` / `y: 1`: with no header slurp the line break after `x` stays, so `x` is a plain
    // scalar and `y: 1` follows it, which YAML refuses; the default's glue read it as `{"xy": 1}`.
    let (out, code) = run_row("--- x\ny: 1\n", &["."])?;
    assert_eq!((out.as_str(), code), ("", 1));
    // The default's glue joins them: `{"xy": 1}`.
    let (out, code) = run_yq_stdin(".", "--- x\ny: 1\n", &["-o=json", "-I0"])?;
    assert_eq!((out.as_str(), code), ("{\"xy\":1}\n", 0));
    Ok(())
}

#[test]
fn the_default_still_slurps_the_header_7025() -> Result<()> {
    let input = "# lead\n\n---\na: 1\n";
    assert_eq!(
        run_yq_stdin(".", input, &[])?,
        ("# lead\n\n---\na: 1\n".to_string(), 0)
    );
    assert_eq!(
        run_yq_stdin(".", input, &["--header-preprocess=false"])?,
        ("# lead\na: 1\n".to_string(), 0)
    );
    Ok(())
}

#[test]
fn the_flag_takes_go_bool_values_after_an_equals_sign_7025() -> Result<()> {
    let input = "# lead\n\na: 1\n";
    for on in [
        "--header-preprocess",
        "--header-preprocess=true",
        "--header-preprocess=1",
        "--header-preprocess=T",
    ] {
        assert_eq!(
            run_yq_stdin(".", input, &[on])?,
            (input.to_string(), 0),
            "{on}"
        );
    }
    for off in [
        "--header-preprocess=false",
        "--header-preprocess=0",
        "--header-preprocess=F",
    ] {
        assert_eq!(
            run_yq_stdin(".", input, &[off])?,
            ("# lead\na: 1\n".to_string(), 0),
            "{off}"
        );
    }
    // The last one wins; a value after a space is the filter, as for every boolean flag of yq's.
    assert_eq!(
        run_yq_stdin(
            ".",
            input,
            &["--header-preprocess=false", "--header-preprocess=true"]
        )?,
        (input.to_string(), 0)
    );
    assert_eq!(
        run_yq_stdin(
            ".",
            input,
            &["--header-preprocess=true", "--header-preprocess=false"]
        )?,
        ("# lead\na: 1\n".to_string(), 0)
    );
    Ok(())
}

#[test]
fn a_comment_only_input_is_printed_back_and_one_with_a_marker_is_an_empty_document_7025(
) -> Result<()> {
    for input in ["# c\n", "# c\n\n", "\n\n\n", "#c2\n# c\n"] {
        assert_eq!(
            run_yq_stdin(".", input, &["--header-preprocess=false"])?,
            (input.to_string(), 0),
            "{input:?}"
        );
    }
    for input in ["---\n", "---\n# c\n", "# c\n---\n", "%YAML 1.1\n---\n"] {
        assert_eq!(
            run_yq_stdin(".", input, &["--header-preprocess=false"])?,
            ("\n".to_string(), 0),
            "{input:?}"
        );
    }
    Ok(())
}

#[test]
fn files_are_read_the_same_way_and_inplace_keeps_the_comment_7025() -> Result<()> {
    let mut a = NamedTempFile::new()?;
    a.write_all(b"# lead\n\na: 1\n")?;
    let mut b = NamedTempFile::new()?;
    b.write_all(b"---\n# two\nb: 2\n")?;
    let (pa, pb) = (a.path().to_str().unwrap(), b.path().to_str().unwrap());
    let (out, code) = run_files(&["--header-preprocess=false", ".", pa, pb])?;
    assert_eq!(
        (out.as_str(), code),
        ("# lead\na: 1\n---\n# two\nb: 2\n", 0)
    );
    let (_, code) = run_files(&["--header-preprocess=false", "-i", ".a = 5", pa])?;
    assert_eq!(code, 0);
    assert_eq!(std::fs::read_to_string(pa)?, "# lead\na: 5\n");
    Ok(())
}

#[test]
fn a_byte_order_mark_leaves_no_header_so_no_break_is_lost_after_a_marker_line_7025() -> Result<()> {
    // The default's `--- text` glue belongs to the header slurp, which a mark switches off:
    // yq reads `--- x` / `y` after one as `"x y"`, and `--- # c` / `a: 1` keeps its mapping.
    let (out, _, code) =
        run_yq_stdin_bytes_with_stderr(".", "\u{feff}--- x\ny\n".as_bytes(), &["-o=json", "-I0"])?;
    assert_eq!((out.as_str(), code), ("\"x y\"\n", 0));
    let (out, _, code) =
        run_yq_stdin_bytes_with_stderr(".a", "\u{feff}--- # c\na: 1\n".as_bytes(), &[])?;
    assert_eq!((out.as_str(), code), ("1\n", 0));
    Ok(())
}
