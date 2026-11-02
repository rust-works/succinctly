//! #5498: under `-n`, `--stream` and `--stream-errors` hand `input`/`inputs` their events as the
//! program asks for them, not from a list built before it runs. Every row's expected text was captured from
//! jq 1.7.1 (`/usr/bin/jq`), with the input a program sees on stdin.

use anyhow::Result;

use crate::{run_jq_full, run_jq_stdin};

fn stream_errors_null(filter: &str, input: &str) -> Result<(String, i32)> {
    run_jq_stdin(filter, input, &["--stream-errors", "-nc"])
}

#[test]
fn a_lazily_read_stream_gives_inputs_the_events_and_lines_jq_gives_5498() -> Result<()> {
    let fault = "[1,2]\n{\"a\" 2}\n[3";
    let blank = "{\"a\":1}\n\n[2,3]\n";
    let tokens = "1 2\n\"x\"";
    for (input, filter, want) in [
        (
            fault,
            "[inputs]",
            "[[[0],1],[[1],2],[[1]],[\"Expected separator between values at line 2, column 7\",[null]],[\"Unfinished JSON term at EOF at line 2, column 9\",[0]]]\n",
        ),
        (fault, "[inputs|input_line_number]", "[1,1,1,2,2]\n"),
        (fault, "first(inputs)", "[[0],1]\n"),
        (
            fault,
            "[., input_line_number, (try input catch \"E\"), input_line_number]",
            "[null,0,[[0],1],1]\n",
        ),
        (
            blank,
            "[inputs]",
            "[[[\"a\"],1],[[\"a\"]],[[0],2],[[1],3],[[1]]]\n",
        ),
        (blank, "[inputs|input_line_number]", "[1,1,3,3,3]\n"),
        (tokens, "[inputs]", "[[[],1],[[],2],[[],\"x\"]]\n"),
        (tokens, "[inputs|input_line_number]", "[1,1,1]\n"),
        (tokens, "input|input_line_number", "1\n"),
    ] {
        assert_eq!(
            stream_errors_null(filter, input)?,
            (want.to_string(), 0),
            "{filter} over {input:?}"
        );
    }
    Ok(())
}

/// Reading the stream to its end, and then past it, moves jq's reader to its end: the second read
/// raises `break` (the filter's `input` has none) and `input_line_number` is 0 once jq has closed
/// the file.
#[test]
fn a_read_past_the_end_leaves_the_reader_where_jq_leaves_it_5498() -> Result<()> {
    let (out, code) = stream_errors_null(
        "[inputs] | (try input catch .), input_line_number",
        "[1]\n[2]\n",
    )?;
    assert_eq!((out.as_str(), code), ("\"break\"\n0\n", 0));
    Ok(())
}

/// Plain `--stream` raises the fault as the read that reaches it, with jq's text and position,
/// where `--stream-errors` hands it over as an input; the events before it are read first.
#[test]
fn a_stream_fault_is_raised_by_the_read_that_reaches_it_5498() -> Result<()> {
    let fault = "[1,2]\n{\"a\" 2}\n[3\n";
    let message = "Expected separator between values at line 2, column 7";
    // Uncaught, the read that meets it ends the program at exit 5.
    for filter in ["[inputs]", "[inputs|input_line_number]"] {
        let (out, err, code) = run_jq_full(&["--stream", "-nc", filter], Some(fault))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (
                "",
                format!("jq: error (at <stdin>:2): {message}\n").as_str(),
                5
            ),
            "{filter}"
        );
    }
    // Caught, it is the string, the events before it are read first and the stream goes on.
    let (out, _, code) = run_jq_full(
        &["--stream", "-nc", "[limit(6; repeat(try input catch .))]"],
        Some(fault),
    )?;
    assert_eq!(code, 0);
    assert_eq!(
        out,
        format!(
            "[[[0],1],[[1],2],[[1]],\"{message}\",\"Unfinished JSON term at EOF at line 3, column 0\",\"break\"]\n"
        )
    );
    let (out, _, code) = run_jq_full(&["--stream", "-nc", "first(inputs)"], Some(fault))?;
    assert_eq!((out.as_str(), code), ("[[0],1]\n", 0));
    Ok(())
}

/// A value that spans two files is read as one by the parser, which names the file it completes
/// in (`input_filename`) and counts each file's lines on its own.
#[test]
fn a_stream_over_two_files_carries_a_token_across_them_5498() -> Result<()> {
    let dir = tempfile::TempDir::new()?;
    let first = dir.path().join("m1.json");
    let second = dir.path().join("m2.json");
    std::fs::write(&first, "{\"a\":1,")?;
    std::fs::write(&second, "\"b\":2}\n[3]\n")?;
    let (first, second) = (first.to_string_lossy(), second.to_string_lossy());
    let files = [first.as_ref(), second.as_ref()];
    for (filter, want) in [
        ("[inputs|input_line_number]", "[0,1,1,2,2]\n".to_string()),
        (
            "[inputs]",
            "[[[\"a\"],1],[[\"b\"],2],[[\"b\"]],[[0],3],[[0]]]\n".to_string(),
        ),
        (
            "[inputs|input_filename]",
            format!("[{first:?},{second:?},{second:?},{second:?},{second:?}]\n"),
        ),
    ] {
        for flag in ["--stream", "--stream-errors"] {
            let mut args = vec![flag, "-nc", filter];
            args.extend(files);
            let (out, err, code) = run_jq_full(&args, None)?;
            assert_eq!((out, code), (want.clone(), 0), "{flag} {filter}: {err}");
        }
    }
    Ok(())
}
