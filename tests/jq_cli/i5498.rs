//! #5498: under `-n`, `--stream-errors` hands `input`/`inputs` their events as the program asks
//! for them, not from a list built before it runs. Every row's expected text was captured from
//! jq 1.7.1 (`/usr/bin/jq`), with the input a program sees on stdin.

use anyhow::Result;

use crate::run_jq_stdin;

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
