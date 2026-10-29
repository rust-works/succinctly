//! #5474: `jq --stream-errors` turns a parse error into an input of the form `[message, path]`
//! and keeps reading, and `-b` is accepted and ignored. Every row's expected text was captured
//! from jq 1.7.1 (`/usr/bin/jq`); `scripts/jq-stream-errors-oracle-sweep.py` ran many more random
//! malformed inputs through both binaries.

use anyhow::Result;

use crate::{run_jq_stdin, run_jq_stdin_streams};

fn stream_errors(input: &str, extra_args: &[&str]) -> Result<(String, i32)> {
    let mut args = vec!["--stream-errors", "-c"];
    args.extend_from_slice(extra_args);
    run_jq_stdin(".", input, &args)
}

#[test]
fn test_stream_errors_reports_the_events_before_the_fault_and_the_path_5474() -> Result<()> {
    for (input, want) in [
        (
            "[1,",
            "[[0],1]\n[\"Unfinished JSON term at EOF at line 1, column 3\",[1]]\n",
        ),
        (
            r#"{"a":[1,{"b":2"#,
            "[[\"a\",0],1]\n[\"Unfinished JSON term at EOF at line 1, column 14\",[\"a\",1,\"b\"]]\n",
        ),
        (
            "\"abc",
            "[\"Unfinished string at EOF at line 1, column 4\",[]]\n",
        ),
        // `null` stands for an object key that has not been read yet.
        (
            "{\"a\":1}\n{\"a\" 2}\n{\"b\":3}\n",
            "[[\"a\"],1]\n[[\"a\"]]\n[\"Expected separator between values at line 2, column 7\",[null]]\n[[\"b\"],3]\n[[\"b\"]]\n",
        ),
    ] {
        assert_eq!(stream_errors(input, &[])?, (want.to_string(), 0), "{input:?}");
    }
    Ok(())
}

#[test]
fn test_stream_errors_drops_the_rest_of_the_line_and_goes_on_5474() -> Result<()> {
    let want = "[[0],1]\n[[1],2]\n[\"Expected another array element at line 1, column 6\",[2]]\n[[0],3]\n[[0]]\n";
    assert_eq!(stream_errors("[1,2,]\n[3]\n", &[])?, (want.to_string(), 0));
    Ok(())
}

#[test]
fn test_stream_errors_loses_a_value_completed_by_the_byte_that_fails_5474() -> Result<()> {
    // `1` completes on the `}` that is then rejected, and jq returns the error instead of it.
    assert_eq!(
        stream_errors("1}", &[])?,
        (
            "[\"Unmatched '}' at the top-level at line 1, column 2\",[]]\n".to_string(),
            0
        )
    );
    Ok(())
}

#[test]
fn test_stream_errors_without_a_fault_matches_stream_5474() -> Result<()> {
    let input = "{\"a\":[1,{\"b\":null}],\"c\":[]}\n1 \"x\" [] {}\n";
    let (events, code) = run_jq_stdin(".", input, &["--stream", "-c"])?;
    assert_eq!(code, 0);
    assert_eq!(stream_errors(input, &[])?, (events.clone(), 0));
    // Together they are one stream: the events are not expanded a second time.
    assert_eq!(
        run_jq_stdin(".", input, &["--stream", "--stream-errors", "-c"])?,
        (events, 0)
    );
    Ok(())
}

#[test]
fn test_stream_errors_slurp_leaves_what_the_last_call_left_pending_outside_5474() -> Result<()> {
    // The input has no final newline, so jq's last read ends on the `]` that yields `[[1],2]`; the
    // closing event the same byte leaves behind is handed out after the slurped array.
    assert_eq!(
        stream_errors("[1,2]", &["-s"])?,
        ("[[[0],1],[[1],2]]\n[[1]]\n".to_string(), 0)
    );
    assert_eq!(
        stream_errors("[1,2]\n", &["-s"])?,
        ("[[[0],1],[[1],2],[[1]]]\n".to_string(), 0)
    );
    // An error at the end of the input is raised in the call that reached it, inside the array.
    assert_eq!(
        stream_errors("[1,2", &["-s"])?,
        (
            "[[[0],1],[\"Unfinished JSON term at EOF at line 1, column 4\",[1]]]\n".to_string(),
            0
        )
    );
    Ok(())
}

#[test]
fn test_stream_errors_inputs_read_the_errors_too_5474() -> Result<()> {
    let (stdout, code) =
        run_jq_stdin("[inputs]", "[1,2]\n[3,\n", &["--stream-errors", "-c", "-n"])?;
    assert_eq!(
        (stdout.as_str(), code),
        (
            "[[[0],1],[[1],2],[[1]],[[0],3],[\"Unfinished JSON term at EOF at line 3, column 0\",[1]]]\n",
            0
        )
    );
    Ok(())
}

#[test]
fn test_stream_errors_is_ignored_for_raw_input_5474() -> Result<()> {
    assert_eq!(
        stream_errors("[1,2", &["-R"])?,
        ("\"[1,2\"\n".to_string(), 0)
    );
    Ok(())
}

#[test]
fn test_stream_errors_names_the_input_position_the_way_jq_does_5474() -> Result<()> {
    // jq has closed its input when the last read ended the file, and only then.
    for (input, at) in [("[1,2]", "<unknown>"), ("[1,2]\n", "<stdin>:1")] {
        let (stdout, stderr, code) = run_jq_stdin_streams(
            "[inputs] | error(\"y\")",
            input,
            &["--stream-errors", "-c", "-n"],
        )?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            ("", format!("jq: error (at {at}): y\n").as_str(), 5),
            "{input:?}"
        );
    }
    Ok(())
}

#[test]
fn test_binary_short_flag_is_accepted_and_ignored_5474() -> Result<()> {
    for args in [&["-b"][..], &["-cb"], &["-bc"], &["-b", "-c"]] {
        let (stdout, code) = run_jq_stdin(".", "{\"a\": 1}", args)?;
        assert_eq!(code, 0, "{args:?}");
        assert!(stdout.contains("\"a\""), "{args:?}: {stdout}");
    }
    Ok(())
}

#[test]
fn test_binary_long_flag_is_still_rejected_5474() -> Result<()> {
    // jq 1.7.1 outside Windows: `Unknown option --binary`, exit 2.
    let (stdout, _, code) = run_jq_stdin_streams(".", "1", &["--binary"])?;
    assert_eq!((stdout.as_str(), code), ("", 2));
    Ok(())
}
