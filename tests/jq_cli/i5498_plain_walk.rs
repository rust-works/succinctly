//! #5498: a plain `--stream` walk (no `-n`, no `-s`) pops its events from the stream reader, so it
//! holds neither the document nor its event list, and answers as jq's streaming parser does: the
//! events of each document with the line jq has read when it yields them, a duplicate key as two
//! events, a document nested past 256 levels, and a fault as the events before it and then jq's own
//! `parse error` text with no location, at exit 5.
//!
//! The rows are `tests/data/jq-stream-walk-oracle-5498.tsv` (over stdin) and
//! `tests/data/jq-stream-walk-file-oracle-5498.tsv` (over a file, whose path is `{FILE}`): stream,
//! program, flags, stdout, stderr and exit status, captured from `/usr/bin/jq` 1.7.1 and kept only
//! where `succinctly jq` answered the same. Fields escape `\`, tab, newline, carriage return and
//! any other non-ASCII or control character as `\u{..}`.

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

type Row<'a> = [&'a str; 6];

fn rows(table: &str) -> impl Iterator<Item = Row<'_>> {
    table.lines().map(|line| {
        line.split('\t')
            .collect::<Vec<_>>()
            .try_into()
            .unwrap_or_else(|_| panic!("malformed row {line:?}"))
    })
}

#[test]
fn a_plain_stream_walk_over_stdin_answers_as_jq_5498() -> Result<()> {
    let table = include_str!("../data/jq-stream-walk-oracle-5498.tsv");
    let mut count = 0;
    for [stream, program, flags, stdout, stderr, code] in rows(table) {
        let (stream, program) = (unescape(stream), unescape(program));
        let mut args: Vec<&str> = flags.split(' ').collect();
        args.push(&program);
        let (got_out, got_err, got_code) = run_jq_full(&args, Some(&stream))?;
        let context = format!("{args:?} over {stream:?}");
        assert_eq!(got_out, unescape(stdout), "stdout of {context}");
        assert_eq!(got_err, unescape(stderr), "stderr of {context}");
        assert_eq!(got_code, code.parse::<i32>()?, "status of {context}");
        count += 1;
    }
    assert!(count > 600, "the table was read: {count} rows");
    Ok(())
}

#[test]
fn a_plain_stream_walk_over_a_file_answers_as_jq_5498() -> Result<()> {
    let table = include_str!("../data/jq-stream-walk-file-oracle-5498.tsv");
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("stream.json");
    let file = path.to_string_lossy().into_owned();
    let mut count = 0;
    for [stream, program, flags, stdout, stderr, code] in rows(table) {
        let (stream, program) = (unescape(stream), unescape(program));
        std::fs::write(&path, &stream)?;
        let mut args: Vec<&str> = flags.split(' ').collect();
        args.push(&program);
        args.push(&file);
        let (got_out, got_err, got_code) = run_jq_full(&args, None)?;
        let context = format!("{args:?} over {stream:?}");
        assert_eq!(
            got_out,
            unescape(stdout).replace("{FILE}", &file),
            "stdout of {context}"
        );
        assert_eq!(
            got_err,
            unescape(stderr).replace("{FILE}", &file),
            "stderr of {context}"
        );
        assert_eq!(got_code, code.parse::<i32>()?, "status of {context}");
        count += 1;
    }
    assert!(count > 600, "the table was read: {count} rows");
    Ok(())
}

/// The documents jq streams past the depth the value parse refuses: 257 and 1000 levels give
/// jq's events at exit 0, one per level as jq's own count (the limitations page recorded the
/// refusal, #5497).
#[test]
fn a_stream_deeper_than_the_value_parse_allows_is_walked_event_by_event_5498() -> Result<()> {
    for depth in [257usize, 1000] {
        let stream = format!("{}{}\n", "[".repeat(depth), "]".repeat(depth));
        let (stdout, stderr, code) = run_jq_full(&["-c", "--stream", "length"], Some(&stream))?;
        assert_eq!((stderr.as_str(), code), ("", 0), "{depth} levels");
        assert_eq!(stdout.lines().count(), depth, "{depth} levels");
    }
    Ok(())
}
