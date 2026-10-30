//! #5497: jq's nesting limits. Its parser refuses a `[` or `{` once 256 stack entries are open
//! (an array holds one, an object two once its key is read), on every input route, and stops
//! reading there; its writer prints anything more than 256 levels down as `<skipped: too deep>`.
//! Every expected output was captured from jq 1.7.1 (`/usr/bin/jq`).

use std::fs;

use anyhow::Result;

use crate::run_jq_full;

const SKIPPED: &str = "<skipped: too deep>";

fn nest(open: &str, inner: &str, close: &str, levels: usize) -> String {
    format!("{}{inner}{}", open.repeat(levels), close.repeat(levels))
}

fn refused(line: usize, column: usize) -> String {
    format!("jq: parse error: Exceeds depth limit for parsing at line {line}, column {column}\n")
}

#[test]
fn test_input_past_jq_parse_depth_is_refused_like_jq_5497() -> Result<()> {
    for (input, stdout, stderr, code) in [
        (nest("[", "", "]", 256), "1\n", String::new(), 0),
        (nest("[", "", "]", 257), "", refused(1, 257), 5),
        (nest("[", "", "]", 5000), "", refused(1, 257), 5),
        (nest("{\"a\":", "1", "}", 128), "1\n", String::new(), 0),
        (nest("{\"a\":", "1", "}", 129), "", refused(1, 641), 5),
        (nest("{\"a\":", "{}", "}", 128), "", refused(1, 641), 5),
        (nest("[{\"a\":", "1", "}]", 85), "1\n", String::new(), 0),
        (nest("[{\"a\":", "1", "}]", 86), "", refused(1, 512), 5),
        // The key itself is never refused, only a container opened under it.
        (nest("[", "{\"a\":1}", "]", 255), "1\n", String::new(), 0),
        (nest("[", "{\"a\":[1]}", "]", 254), "", refused(1, 260), 5),
        // Brackets inside a string are not containers.
        (nest("[", "\"[[[[{{{{\"", "]", 200), "1\n", String::new(), 0),
        // Everything before the refused document is printed, nothing after it.
        (
            format!("1\n{}\n2\n", nest("[", "", "]", 257)),
            "1\n",
            refused(2, 257),
            5,
        ),
    ] {
        for program in ["length", ".", "[..] | length"] {
            let (out, err, exit) = run_jq_full(&["-c", program], Some(&input))?;
            if program == "length" {
                assert_eq!(out, stdout, "{}", &input[..input.len().min(40)]);
            }
            assert_eq!(err, stderr, "{program}: {}", &input[..input.len().min(40)]);
            assert_eq!(exit, code, "{program}: {}", &input[..input.len().min(40)]);
        }
    }
    Ok(())
}

/// jq reads every file as one stream: the parser's line count runs on from the files before,
/// while `input`'s `(at <file>:<line>)` names the file's own (0 for a file with no newline).
#[test]
fn test_depth_refusal_counts_lines_across_files_5497() -> Result<()> {
    let dir = tempfile::TempDir::new()?;
    let three = dir.path().join("three.json");
    let deep = dir.path().join("deep.json");
    fs::write(&three, "1\n2\n3\n")?;
    fs::write(&deep, nest("[", "", "]", 300))?;
    let (three, deep) = (three.to_str().unwrap(), deep.to_str().unwrap());

    let (out, err, exit) = run_jq_full(&["-c", "length", three, deep, three], None)?;
    assert_eq!((out.as_str(), exit), ("1\n2\n3\n", 5));
    assert_eq!(err, refused(4, 257));

    let (out, err, exit) = run_jq_full(&["-n", "-c", "[inputs] | length", three, deep], None)?;
    assert_eq!((out.as_str(), exit), ("", 5));
    assert_eq!(
        err,
        format!("jq: error (at {deep}:0): Exceeds depth limit for parsing at line 4, column 257\n")
    );
    Ok(())
}

/// `input` raises jq's own message, position included, so a program that catches it sees what
/// jq's would.
#[test]
fn test_input_raises_jq_depth_message_5497() -> Result<()> {
    let input = format!("1 {}", nest("{\"a\":", "1", "}", 129));
    let (out, err, exit) = run_jq_full(&["-n", "-c", "input, (try input catch .)"], Some(&input))?;
    assert_eq!(
        out,
        "1\n\"Exceeds depth limit for parsing at line 1, column 643\"\n"
    );
    assert_eq!((err.as_str(), exit), ("", 0));
    Ok(())
}

/// `input`'s `(at <stdin>:<line>)` is the line jq has read to when its parser refuses the
/// bracket, not where the document starts.
#[test]
fn test_input_names_the_line_of_the_refused_bracket_5497() -> Result<()> {
    let input = format!("1\n{}{}\n", "[\n".repeat(300), "]".repeat(300));
    let (out, err, exit) = run_jq_full(&["-n", "-c", "[inputs] | length"], Some(&input))?;
    assert_eq!((out.as_str(), exit), ("", 5));
    assert_eq!(
        err,
        "jq: error (at <stdin>:258): Exceeds depth limit for parsing at line 258, column 1\n"
    );
    Ok(())
}

/// Under `--slurp` the limit applies to each document, not to the array they are joined into;
/// the position is not recovered there (recorded in `limitations.md`).
#[test]
fn test_slurp_applies_the_limit_per_document_5497() -> Result<()> {
    let (out, err, exit) = run_jq_full(&["-s", "-c", "length"], Some(&nest("[", "", "]", 256)))?;
    assert_eq!((out.as_str(), err.as_str(), exit), ("1\n", "", 0));
    let (out, err, exit) = run_jq_full(
        &["-s", "-c", "length"],
        Some(&format!("1 {}", nest("{\"a\":", "1", "}", 129))),
    )?;
    assert_eq!(
        (out.as_str(), err.as_str(), exit),
        ("", "jq: parse error: Exceeds depth limit for parsing\n", 5)
    );
    Ok(())
}

/// jq's streaming parser has no depth limit, so `--stream` reads a document its ordinary
/// parser refuses.
#[test]
fn test_stream_reads_past_the_parse_limit_5497() -> Result<()> {
    let input = nest("{\"a\":", "1", "}", 129);
    let (out, err, exit) = run_jq_full(
        &["--stream", "-c", "select(length == 2) | .[0] | length"],
        Some(&input),
    )?;
    assert_eq!((out.as_str(), err.as_str(), exit), ("129\n", "", 0));
    Ok(())
}

/// A value the program built prints with jq's marker past 256 levels, in every writer jq
/// routes through its one dumper.
#[test]
fn test_built_value_past_print_depth_prints_the_marker_5497() -> Result<()> {
    let arrays = |levels| format!("reduce range({levels}) as $i (0; [.])");
    let full = |levels| nest("[", "0", "]", levels);
    let marked = nest("[", SKIPPED, "]", 257);
    for (args, stdout) in [
        (vec!["-n", "-c", &arrays(256) as &str], full(256) + "\n"),
        (vec!["-n", "-c", &arrays(257)], marked.clone() + "\n"),
        (vec!["-n", "-c", &arrays(300)], marked.clone() + "\n"),
        (
            vec!["-n", "-c", &format!("{} | tojson | length", arrays(300))],
            "533\n".into(),
        ),
        (
            vec![
                "-n",
                "-r",
                &format!("{} | \"<\\(.)>\" | length", arrays(257)),
            ],
            "535\n".into(),
        ),
        (
            vec!["-n", "-c", "reduce range(257) as $i ({\"k\":true}; {a:.})"],
            nest("{\"a\":", SKIPPED, "}", 257) + "\n",
        ),
        // Sorting happens level by level, so it stops at the marker too.
        (
            vec!["-n", "-c", "-S", "reduce range(257) as $i (1; {z:., y:0})"],
            nest(
                "{\"y\":0,\"z\":",
                &format!("{{\"y\":{SKIPPED},\"z\":{SKIPPED}}}"),
                "}",
                256,
            ) + "\n",
        ),
    ] {
        let (out, err, exit) = run_jq_full(&args, None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), exit),
            (stdout.as_str(), "", 0),
            "{args:?}"
        );
    }
    Ok(())
}

/// `debug` prints `["DEBUG:", v]` as one value, so `v` reaches the marker a level sooner; an
/// uncaught `error(v)` renders `v` through the same dumper.
#[test]
fn test_debug_and_error_render_the_marker_5497() -> Result<()> {
    let (out, err, exit) = run_jq_full(
        &[
            "-n",
            "-c",
            "reduce range(256) as $i (0; [.]) | debug | empty",
        ],
        None,
    )?;
    assert_eq!((out.as_str(), exit), ("", 0));
    assert_eq!(
        err,
        format!("[\"DEBUG:\",{}]\n", nest("[", SKIPPED, "]", 256))
    );

    let (out, err, exit) = run_jq_full(
        &["-n", "-c", "reduce range(257) as $i (0; [.]) | error"],
        None,
    )?;
    assert_eq!((out.as_str(), exit), ("", 5));
    assert_eq!(
        err,
        format!(
            "jq: error (at <unknown>) (not a string): {}\n",
            nest("[", SKIPPED, "]", 257)
        )
    );
    Ok(())
}

/// Under `-C` the marker takes the color of the value it stands for, then a reset.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq's `{a:.}`, not a format argument
fn test_colored_marker_takes_the_skipped_values_color_5497() -> Result<()> {
    let (out, _, exit) = run_jq_full(
        &["-n", "-c", "-C", "reduce range(257) as $i (\"s\"; [.])"],
        None,
    )?;
    assert_eq!(exit, 0);
    assert!(
        out.contains("\x1b[1;39m[\x1b[0;32m<skipped: too deep>\x1b[0m\x1b[1;39m\x1b[1;39m]"),
        "{}",
        &out[out.len().saturating_sub(200)..]
    );
    let (out, _, _) = run_jq_full(
        &["-n", "-c", "-C", "reduce range(257) as $i (0; {a:.})"],
        None,
    )?;
    assert!(
        out.contains("\x1b[1;39m:\x1b[0m\x1b[0;39m<skipped: too deep>\x1b[0m"),
        "{}",
        &out[out.len().saturating_sub(200)..]
    );
    Ok(())
}
