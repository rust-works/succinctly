//! #5855: a program that reads `input`/`inputs` and runs without `-n` takes the lazy input route
//! of #5661 over a clean plain-JSON stream, so the driver loop and the program's own `input` pop
//! from one queue and convert a document when it is read. What a program sees, writes and exits
//! with is unchanged, and is what jq 1.7.1 answers.
//!
//! The rows are `tests/data/jq-lazy-input-without-n-oracle-5855.tsv`: stream, program, flags,
//! stdout, stderr and exit status, captured from `/usr/bin/jq` 1.7.1 over stdin (a row is kept
//! only where `succinctly jq` answered the same). Fields escape `\`, tab, newline, carriage
//! return and any other non-ASCII or control character as `\u{..}`.

use anyhow::Result;

use crate::run_jq_full;

fn unescape(field: &str) -> String {
    let mut out = String::new();
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some('u') => {
                assert_eq!(chars.next(), Some('{'));
                let hex: String = chars.by_ref().take_while(|&c| c != '}').collect();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
            }
            other => panic!("bad escape {other:?} in {field:?}"),
        }
    }
    out
}

#[test]
fn a_program_reading_inputs_without_null_input_answers_as_jq_5855() -> Result<()> {
    let table = include_str!("../data/jq-lazy-input-without-n-oracle-5855.tsv");
    let mut rows = 0;
    for line in table.lines() {
        let [stream, program, flags, stdout, stderr, code] =
            line.split('\t').collect::<Vec<_>>()[..]
        else {
            panic!("malformed row {line:?}");
        };
        let (stream, program) = (unescape(stream), unescape(program));
        let mut args: Vec<&str> = flags.split(' ').collect();
        args.push(&program);
        let (got_out, got_err, got_code) = run_jq_full(&args, Some(&stream))?;
        let context = format!("{args:?} over {stream:?}");
        assert_eq!(got_out, unescape(stdout), "stdout of {context}");
        assert_eq!(got_err, unescape(stderr), "stderr of {context}");
        assert_eq!(got_code, code.parse::<i32>()?, "status of {context}");
        rows += 1;
    }
    assert!(rows > 600, "the table was read: {rows} rows");
    Ok(())
}

/// `halt` after the first document: the rest of the stream is never converted. The answer is
/// the same as when every document is read first (an empty stdout, status 0).
#[test]
fn a_program_that_halts_on_the_first_document_leaves_the_rest_unread_5855() -> Result<()> {
    let stream = "1\n2\n3\n";
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "[., input_line_number] | halt"], Some(stream))?;
    assert_eq!((stdout.as_str(), stderr.as_str(), code), ("", "", 0));
    let (stdout, _, code) = run_jq_full(&["-c", ". , halt"], Some(stream))?;
    assert_eq!((stdout.as_str(), code), ("1\n", 0));
    Ok(())
}
