//! #5576: a malformed member the semi-index accepts is found when the filter reads it, and is
//! worded as jq's parser words the stream's first fault (`jq: parse error: ...`), after which
//! the run reads nothing more, as jq does. Every expected message was captured from jq 1.7.1
//! (`/usr/bin/jq`).
//!
//! A filter that never reads the member keeps answering ("Every M2 filter streams"), so every
//! row here reads it.

use anyhow::Result;

use crate::{run_jq_full, run_jq_over_files};

const A: &str = "1\n2\n";
const B: &str = "3\n{\"a\":1,}\n";
const C: &str = "4\n";

#[test]
fn test_a_malformed_member_is_worded_as_jqs_parser_words_it_5576() -> Result<()> {
    let message = "jq: parse error: Expected another key-value pair at line 4, column 8\n";
    // The values before the fault are answered and nothing after it is read, so `4` never prints.
    for (args, out) in [
        (&["-c", "."][..], "1\n2\n3\n"),
        (&["-c", "-s", "."], ""),
        (&["-S", "-c", "."], "1\n2\n3\n"),
        (&["-r", "."], "1\n2\n3\n"),
        (&["-e", "."], "1\n2\n3\n"),
        (&["-c", "tojson"], "\"1\"\n\"2\"\n\"3\"\n"),
    ] {
        let (stdout, stderr, code, _) = run_jq_over_files(args, &[A, B, C])?;
        assert_eq!((stdout.as_str(), code), (out, 5), "{args:?}");
        assert_eq!(stderr, message, "{args:?}");
    }
    Ok(())
}

#[test]
fn test_each_member_fault_keeps_jqs_own_wording_5576() -> Result<()> {
    // Rows captured from jq 1.7.1, one fault kind each, a member the splitter delimits.
    for (doc, message) in [
        (
            "[1.5,-2\ne3]\n",
            "Invalid numeric literal at line 2, column 3",
        ),
        ("[1,,2]\n", "Expected value before ',' at line 1, column 4"),
        (
            "{\"a\" 1}\n",
            "Expected separator between values at line 1, column 7",
        ),
        ("{\"a\":tru}\n", "Invalid literal at line 1, column 9"),
        (
            "[1 2]\n",
            "Expected separator between values at line 1, column 5",
        ),
    ] {
        let (stdout, stderr, code, _) = run_jq_over_files(&["-c", "."], &[doc])?;
        assert_eq!((stdout.as_str(), code), ("", 5), "{doc:?}");
        assert_eq!(stderr, format!("jq: parse error: {message}\n"), "{doc:?}");
    }
    Ok(())
}

#[test]
fn test_the_first_fault_of_the_stream_is_the_one_named_5576() -> Result<()> {
    // Two malformed documents: jq stops at the first, so the second is never worded.
    let (stdout, stderr, code, _) =
        run_jq_over_files(&["-c", "."], &["{\"a\":1,}\n{\"b\":[1,,]}\n"])?;
    assert_eq!((stdout.as_str(), code), ("", 5));
    assert_eq!(
        stderr,
        "jq: parse error: Expected another key-value pair at line 1, column 8\n"
    );
    // A fault in a later file is counted in lines across the files as one stream.
    let (stdout, stderr, code, _) = run_jq_over_files(&["-c", "."], &[A, "[1,2,\n3,]\n", C])?;
    assert_eq!((stdout.as_str(), code), ("1\n2\n", 5));
    assert_eq!(
        stderr,
        "jq: parse error: Expected another array element at line 4, column 3\n"
    );
    Ok(())
}

#[test]
fn test_stdin_names_the_fault_the_same_way_5576() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "."], Some("{\"a\":1}\n{\"a\":2,}\n{\"a\":3}\n"))?;
    assert_eq!((stdout.as_str(), code), ("{\"a\":1}\n", 5));
    assert_eq!(
        stderr,
        "jq: parse error: Expected another key-value pair at line 2, column 8\n"
    );
    Ok(())
}

#[test]
fn test_slurp_from_stdin_keeps_the_evaluators_wording_5576() -> Result<()> {
    // A slurped stdin has no file to read again for jq's position, so it stays the
    // evaluator's own message (recorded in the limitations page).
    let (stdout, stderr, code) = run_jq_full(&["-sc", "."], Some("{\"a\":1}\n{\"a\":2,}\n"))?;
    assert_eq!((stdout.as_str(), code), ("", 5));
    assert!(stderr.starts_with("jq: error (at <stdin>:"), "{stderr}");
    assert!(stderr.contains("Invalid JSON text:"), "{stderr}");
    Ok(())
}

#[test]
fn test_on_error_continue_still_stops_at_jqs_parse_error_5576() -> Result<()> {
    // jq's parser cannot read on past a fault, so the flag cannot keep the stream running: the
    // splitter's own faults already stop it the same way.
    let (stdout, stderr, code, _) =
        run_jq_over_files(&["-c", "--on-error=continue", "."], &[A, B, C])?;
    assert_eq!((stdout.as_str(), code), ("1\n2\n3\n", 5));
    assert_eq!(
        stderr,
        "jq: parse error: Expected another key-value pair at line 4, column 8\n"
    );
    Ok(())
}
