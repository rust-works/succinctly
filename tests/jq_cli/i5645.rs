//! #5645: a number or literal where an object key belongs (`{1.5e3`, `{true`) is no fault to jq's
//! lexer, which takes the token and meets the fault only at the token after it. The read jq
//! resumes at is therefore the one holding that next token, which may be a later line, where this
//! build resumed at the line after the value itself and read the rest as documents. Rows are
//! `(stdin, output)` captured from jq 1.7.1 (`/usr/bin/jq`) with
//! `-nc '[limit(6; repeat(try input catch "E"))]'`.

use anyhow::Result;

use crate::run_jq_full;

const CATCH_INPUTS: &str = "[limit(6; repeat(try input catch \"E\"))]";

const READS: &[(&str, &str)] = &[
    // The next token is on the next line, so that line is the one the fault is met in.
    ("{1.5e3\n1\"k\":false", r#"["E","E","E","E","E","E"]"#),
    ("{1\n\"k\":false", r#"["E","E","E","E","E","E"]"#),
    ("{1\n2\n3", r#"["E",3,"E","E","E","E"]"#),
    // A structural byte is met at itself, an invalid token at its own end.
    ("{true\n,2\n3", r#"["E",3,"E","E","E","E"]"#),
    ("{null\n:2\n3", r#"["E",3,"E","E","E","E"]"#),
    ("{1\n}\n3", r#"["E",3,"E","E","E","E"]"#),
    ("{1\nx\n3", r#"["E",3,"E","E","E","E"]"#),
    // The same after a comma, and across any run of whitespace.
    ("{\"a\":1,2\n3\n4", r#"["E",4,"E","E","E","E"]"#),
    ("{\"a\":1,2 \n\"k\"\n4", r#"["E",4,"E","E","E","E"]"#),
    ("{1 \t \n\n 2 3\n4", r#"["E",4,"E","E","E","E"]"#),
    ("{01\n2\n3", r#"["E",3,"E","E","E","E"]"#),
    // No next token: the fault is at the end of input.
    ("{1", r#"["E","E","E","E","E","E"]"#),
    ("{1\n\n", r#"["E","E","E","E","E","E"]"#),
    // Controls that never changed: the token itself is the fault, or a structural byte follows.
    ("{x\n1\"k\":false", r#"["E",1,"k","E","E","E"]"#),
    ("{1.5e3,\n1\"k\":false", r#"["E",1,"k","E","E","E"]"#),
    ("{\"a\":1,}\n2\n3", r#"["E",2,3,"E","E","E"]"#),
];

#[test]
fn test_a_number_in_key_position_is_met_at_the_next_token_5645() -> Result<()> {
    for (stdin, expected) in READS {
        let (out, err, code) = run_jq_full(&["-nc", CATCH_INPUTS], Some(stdin))?;
        assert_eq!(
            (out.trim_end(), err.as_str(), code),
            (*expected, "", 0),
            "{stdin:?}"
        );
    }
    Ok(())
}

/// `--stream` reads the same bytes, so it resumes at the same read (the events a fault cuts off
/// are #5635's).
#[test]
fn test_the_stream_reader_resumes_at_the_same_read_5645() -> Result<()> {
    for (stdin, expected) in [
        ("{1\n\"k\"\n4", r#"["E",[[],4],"E","E","E","E"]"#),
        ("{1.5e3\n1\"k\":false", r#"["E","E","E","E","E","E"]"#),
        ("{true\n:2\n3", r#"["E",[[],3],"E","E","E","E"]"#),
    ] {
        let (out, err, code) = run_jq_full(&["-nc", "--stream", CATCH_INPUTS], Some(stdin))?;
        assert_eq!(
            (out.trim_end(), err.as_str(), code),
            (expected, "", 0),
            "{stdin:?}"
        );
    }
    Ok(())
}
