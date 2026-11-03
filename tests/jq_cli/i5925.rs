//! #5925: after a parse error that `input` raises, jq drops the rest of the `fgets` read the fault
//! was in and resumes at the next one. When the number the fault is in is ended by a quote
//! (`1e"str`, `-"x`, `1."x`), jq meets the fault at that quote: the string it opens is part of the
//! dropped read, not a token to complete. `succinctly jq` took it for one, and a string that never
//! closes put the fault at the end of the input, so the stream ended there. Every expected stream was
//! captured from jq 1.7.1 (`/usr/bin/jq`). The filter strips each message's `at line N, column M`:
//! the position a second fault gets after the first is #5923's, not this issue's.

use anyhow::Result;

use crate::run_jq_stdin;

const READS: &str =
    r#"[limit(14; repeat(try input catch sub(" at line [0-9]+, column [0-9]+"; "")))]"#;

fn stream(input: &str) -> Result<String> {
    let (out, code) = run_jq_stdin(READS, input, &["-nc"])?;
    assert_eq!(code, 0, "{input:?}");
    Ok(out.trim_end().to_string())
}

#[test]
fn a_number_ended_by_a_quote_resumes_at_the_next_read_5925() -> Result<()> {
    for (input, want) in [
        // the issue's first stream
        ("{\"a\"}\n\n1e\"str\n\ntrue\n\n]\n[}12 34\t", "[\"Objects must consist of key:value pairs\",\"Invalid numeric literal\",true,\"Unmatched ']'\",\"Unmatched '}'\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        // the issue's second stream
        ("true1e\n:\n[1,\t1e\"str\n{\"a\"}  ", "[\"Invalid literal\",\"Expected string key before ':'\",\"Invalid numeric literal\",\"Objects must consist of key:value pairs\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        // a bad exponent, the unclosed string on its own line
        ("1e\"str\n\ntrue\n", "[\"Invalid numeric literal\",true,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        // a bad exponent and a string closed on the line
        ("1e\"str\"\n\ntrue\n", "[\"Invalid numeric literal\",true,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        // a lone minus before the quote
        ("-\"x\n7\n8\n", "[\"Invalid numeric literal\",7,8,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        // a bare fraction point before the quote
        ("1.\"x\n7\n", "[1,\"Unfinished string at EOF\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        // a signed bad exponent before the quote
        ("1e+\"x\n7\n8\n", "[\"Invalid numeric literal\",7,8,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        // inside an array
        ("[1,2,-\"abc\n[3]\n", "[\"Invalid numeric literal\",[3],\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        // as an object value
        ("{\"a\":1e\"x\n\ntrue\n", "[\"Invalid numeric literal\",true,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        // inside a nested object value
        ("{\"a\":[1e\"q\n7\n8\n", "[\"Invalid numeric literal\",7,8,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        // as an object key
        ("{1e\"k\n7\n8\n", "[\"Invalid numeric literal\",7,8,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        // a bad literal the quote ends (already read as jq does)
        ("1x\"s\ntrue\n2\n", "[\"Invalid numeric literal\",true,2,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
    ] {
        assert_eq!(stream(input)?, want, "{input:?}");
    }
    Ok(())
}

/// The read the fault is in runs to its newline or to 4,095 bytes, whichever comes first; the quote
/// and the string it opens can sit before, at or past that edge.
#[test]
fn the_unclosed_string_may_run_past_the_4095_byte_read_5925() -> Result<()> {
    for (padding, want) in [
        (4090, "[\"Invalid numeric literal\",true,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        (4093, "[\"Invalid numeric literal\",\"Invalid numeric literal\",true,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        (4094, "[\"Invalid numeric literal\",\"Invalid numeric literal\",true,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        (4095, "[\"Invalid numeric literal\",\"Invalid numeric literal\",true,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        (4096, "[\"Invalid numeric literal\",\"Invalid numeric literal\",true,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        (8190, "[\"Invalid numeric literal\",\"Invalid numeric literal\",true,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
    ] {
        let input = format!("1e\"{}\ntrue\n", "a".repeat(padding));
        assert_eq!(stream(&input)?, want, "{padding} bytes after the quote");
    }
    Ok(())
}

/// The quote can also sit at the edge of a read: the fault is met there, and the read it drops runs
/// to the next newline or the 4,095-byte limit.
#[test]
fn the_quote_may_sit_at_the_edge_of_a_read_5925() -> Result<()> {
    for (leading, want) in [
        (4090, "[\"Invalid numeric literal\",7,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        (4093, "[\"Invalid numeric literal\",7,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        (4094, "[\"Invalid numeric literal\",7,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        (4095, "[\"Invalid numeric literal\",7,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
        (4096, "[\"Invalid numeric literal\",7,\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]"),
    ] {
        let input = format!("{}1e\"s\n7\n", " ".repeat(leading));
        assert_eq!(stream(&input)?, want, "{leading} bytes before the quote");
    }
    Ok(())
}
