//! #7442: an uncaught `error("a\u0000b")` writes its message up to the NUL byte, as jq 1.7.1 and 1.8.2
//! do (`fprintf(stderr, "jq: error (at %s): %s\n", ...)` over a C string): the text after the NUL is
//! never written, and the line still ends with its newline. The error value itself stays whole, so a
//! `catch` sees all of it, and a value that is not a string is dumped as JSON, which escapes the NUL.
//!
//! Every row was captured from jq 1.7.1 and 1.8.2 (`--stream`'s position on its last event differs
//! between the two, so those rows pin the text after the position only).

use anyhow::Result;

use crate::run_jq_full;

/// A program that fails with a message holding NULs, and what jq writes of the message.
const MESSAGES: [(&str, &str); 8] = [
    (r#"error("a\u0000b")"#, "a"),
    (r#"error("\u0000")"#, ""),
    (r#"error("\u0000x")"#, ""),
    (r#"error("x\u0000")"#, "x"),
    (r#"error("a\u0000b\u0000c")"#, "a"),
    (r#"error("a\u0000b\nc")"#, "a"),
    (r#""a\u0000b"|error"#, "a"),
    (r#""é\u0000ü"|error"#, "é"),
];

#[test]
fn an_uncaught_error_message_stops_at_its_nul_7442() -> Result<()> {
    for (program, text) in MESSAGES {
        // `-n`: no input, so no position; a record on one line; three records; `-s`.
        let rows: [(&[&str], Option<&str>, String); 5] = [
            (&["-n"], None, format!("jq: error (at <unknown>): {text}\n")),
            (
                &["-r"],
                Some("[1,2]"),
                format!("jq: error (at <stdin>:0): {text}\n"),
            ),
            (
                &["-c"],
                Some("[1,2] 3 \"z\""),
                format!("jq: error (at <stdin>:0): {text}\n").repeat(3),
            ),
            (
                &["-n", "-e"],
                None,
                format!("jq: error (at <unknown>): {text}\n"),
            ),
            (
                &["-s"],
                Some("[1,2]"),
                format!("jq: error (at <stdin>:0): {text}\n"),
            ),
        ];
        for (flags, input, stderr) in rows {
            let mut args = flags.to_vec();
            args.push(program);
            let (out, err, status) = run_jq_full(&args, input)?;
            assert_eq!(
                (out.as_str(), err.as_str(), status),
                ("", stderr.as_str(), 5),
                "{args:?} over {input:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_stream_run_stops_the_message_at_its_nul_7442() -> Result<()> {
    for (program, text) in MESSAGES {
        let (out, err, status) = run_jq_full(&["--stream", program], Some("[1,2]"))?;
        assert_eq!((out.as_str(), status), ("", 5), "{program}");
        let lines: Vec<&str> = err.split_inclusive('\n').collect();
        assert_eq!(lines.len(), 3, "{program}: {err:?}");
        for line in lines {
            let (marker, message) = line.split_once("): ").expect("a position marker");
            assert!(marker.starts_with("jq: error (at "), "{program}: {line:?}");
            assert_eq!(message, format!("{text}\n"), "{program}");
        }
    }
    Ok(())
}

#[test]
fn the_error_value_keeps_its_nul_for_a_catch_7442() -> Result<()> {
    let (out, err, status) =
        run_jq_full(&["-n", r#"try error("a\u0000b") catch (., length)"#], None)?;
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        ("\"a\\u0000b\"\n3\n", "", 0)
    );
    let (out, err, status) = run_jq_full(&["-n", "-r", r#"try error("a\u0000b") catch ."#], None)?;
    assert_eq!(
        (out.as_bytes(), err.as_str(), status),
        (&b"a\0b\n"[..], "", 0)
    );
    // Not a string: dumped as JSON, with the NUL escaped, whole.
    let (out, err, status) = run_jq_full(&["-n", r#"error({a:"x\u0000y"})"#], None)?;
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        (
            "",
            "jq: error (at <unknown>) (not a string): {\"a\":\"x\\u0000y\"}\n",
            5
        )
    );
    Ok(())
}
