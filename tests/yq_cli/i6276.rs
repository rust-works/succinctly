//! #6276: yq's `--header-preprocess` pushes the text after a leading `--- ` back onto its reader
//! without the marker line's own line break, so the next line continues it: `--- x` / `y` is the
//! scalar `xy`, not `x y`. Every row was captured from yq v4.53.3 (`yq -o=json -I0 .`).

use anyhow::Result;
use std::io::Write;

use crate::{run_yq_files, run_yq_stdin, run_yq_stdin_with_stderr};

const ARGS: &[&str] = &["-o=json", "-I=0"];

fn json_of(doc: &str) -> Result<String> {
    let (stdout, code) = run_yq_stdin(".", doc, ARGS)?;
    assert_eq!(code, 0, "{doc:?} should read, got {stdout:?}");
    Ok(stdout.trim_end().to_owned())
}

#[test]
fn test_the_break_after_a_marker_line_is_lost_so_its_scalar_runs_on_6276() -> Result<()> {
    for (doc, expected) in [
        ("--- x\ny\n", "\"xy\""),
        ("--- x\n\ty\n", "\"x\\ty\""),
        ("--- x\n  y\n", "\"x  y\""),
        ("--- x \n y\n", "\"x  y\""),
        ("--- x\n\ny\n", "\"x y\""),
        ("--- x\n\n\ny\n", "\"x\\ny\""),
        ("--- x\r\ny\r\n", "\"xy\""),
        ("--- x\ny\nz\n", "\"xy z\""),
        ("--- x\ny\n\nz\n", "\"xy\\nz\""),
        ("--- [a, x\ny]\n", "[\"a\",\"xy\"]"),
        ("--- {a: x\ny}\n", "{\"a\":\"xy\"}"),
        ("--- a: x\n  y\n", "{\"a\":\"x  y\"}"),
        ("--- \"x\ny\"\n", "\"xy\""),
        ("--- \"x\n\ty\"\n", "\"x\\ty\""),
        ("--- 'x\n\ny'\n", "\"x y\""),
        ("--- &a x\ny\n", "\"xy\""),
        ("--- !!str x\ny\n", "\"xy\""),
        ("--- x\ny: 1\n", "{\"xy\":1}"),
        ("--- ? x\ny\n: 1\n", "{\"xy\":1}"),
        ("--- - x\n  y\n", "[\"x  y\"]"),
        ("--- x\ny\n...\n", "\"xy\""),
        // The blanks before the marker are not in yq's stream, and a second marker is stripped.
        ("  --- x\ny\n", "\"xy\""),
        ("  --- x\n  y\n", "\"x  y\""),
        ("\t--- x\ny\n", "\"xy\""),
        ("--- --- x\ny\n", "\"xy\""),
        ("--- --- x\ny\nz\n", "\"xyz\""),
    ] {
        assert_eq!(json_of(doc)?, expected, "{doc:?}");
    }
    Ok(())
}

#[test]
fn test_a_following_marker_is_no_longer_at_a_line_start_so_it_is_scalar_text_6276() -> Result<()> {
    for (doc, expected) in [
        ("--- x\n--- y\nz\n", "\"x--- y z\""),
        ("--- x\n--- y\n", "\"x--- y\""),
        ("--- x\n...\n--- y\nz\n", "\"x...\"\n\"y z\""),
    ] {
        assert_eq!(json_of(doc)?, expected, "{doc:?}");
    }
    Ok(())
}

#[test]
fn test_a_leading_header_before_the_marker_line_does_not_stop_the_glue_6276() -> Result<()> {
    for (doc, expected) in [
        ("# c\n--- x\ny\n", "\"xy\""),
        ("\n--- x\ny\n", "\"xy\""),
        ("%YAML 1.2\n--- x\ny\n", "\"xy\""),
    ] {
        assert_eq!(json_of(doc)?, expected, "{doc:?}");
    }
    Ok(())
}

#[test]
fn test_a_comment_after_the_marker_swallows_the_next_line_into_the_comment_6276() -> Result<()> {
    for (doc, expected) in [
        ("--- # c\nx\ny\n", "\"y\""),
        ("--- # c\na: 1\nb: 2\n", "{\"b\":2}"),
        ("--- &a\nx\ny\n", "\"y\""),
    ] {
        assert_eq!(json_of(doc)?, expected, "{doc:?}");
    }
    Ok(())
}

#[test]
fn test_what_yq_does_not_glue_is_read_as_before_6276() -> Result<()> {
    for (doc, expected) in [
        // The marker on a line of its own keeps its break.
        ("---\nx\ny\n", "\"x y\""),
        ("--- \nx\ny\n", "\"x y\""),
        // The loop has stopped at the first content line: a later marker is a real marker.
        ("a: 1\n--- x\ny\n", "{\"a\":1}\n\"x y\""),
        ("--- x\ny\n--- z\nw\n", "\"xy\"\n\"z w\""),
        ("--- x\ny\n---\nz\nw\n", "\"xy\"\n\"z w\""),
        // yq splits its lines on `\n` only: a lone CR is an ordinary break.
        ("--- x\ry\r", "\"x y\""),
        // A line break with nothing after it glues nothing.
        ("--- x\n", "\"x\""),
        // Between flow tokens a break is only white space whether or not it is lost.
        ("--- [a\n, x\ny]\n", "[\"a\",\"x y\"]"),
    ] {
        assert_eq!(json_of(doc)?, expected, "{doc:?}");
    }
    Ok(())
}

#[test]
fn test_a_line_that_the_glue_makes_invalid_is_refused_as_yq_refuses_it_6276() -> Result<()> {
    // `--- a: 1` / `b: 2` is `a: 1b: 2`, a second `:` on the one line; the same for an indented
    // continuation and for a document whose first marker line is a `a: b` mapping entry.
    for doc in [
        "--- a: 1\nb: 2\n",
        "--- a: 1\n  b: 2\n",
        "--- a: b\n--- c: d\n",
    ] {
        let (stdout, _stderr, code) = run_yq_stdin_with_stderr(".", doc, ARGS)?;
        assert_ne!(code, 0, "{doc:?} should be refused, got {stdout:?}");
    }
    Ok(())
}

#[test]
fn test_the_glue_applies_to_every_file_and_to_the_first_under_eval_all_6276() -> Result<()> {
    let mut first = tempfile::NamedTempFile::new()?;
    write!(first, "--- x\ny\n")?;
    let mut second = tempfile::NamedTempFile::new()?;
    write!(second, "--- x\ny\n")?;
    let files = [first.path(), second.path()];

    // Each file is a stream of its own, so each one's marker line is glued.
    let (stdout, _stderr, code) = run_yq_files(".", &files, ARGS)?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "\"xy\"\n\"xy\"\n");

    // yq slurps the header of the first file only when it evaluates all of them together.
    let (stdout, _stderr, code) = run_yq_files(".", &files, &["--eval-all", "-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "\"xy\"\n\"x y\"\n");
    Ok(())
}

#[test]
fn test_the_glue_is_what_in_place_editing_writes_back_6276() -> Result<()> {
    let mut file = tempfile::NamedTempFile::new()?;
    write!(file, "--- x\ny\n")?;
    let (_stdout, _stderr, code) = run_yq_files(".", &[file.path()], &["-i"])?;
    assert_eq!(code, 0);
    assert_eq!(std::fs::read_to_string(file.path())?, "---\nxy\n");
    Ok(())
}
