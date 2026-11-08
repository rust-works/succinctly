//! #6620: the records of a `--seq` stream with a diagnostic are read in one pass where they are
//! plain text, and by the checked materializer where they are not. Either way the run is jq 1.7.1's:
//! every expected string was captured from it (`/usr/bin/jq`). The stream is a truncated record
//! (first, in the middle, last) among records that stress the one-pass reader: number spellings, a
//! repeated key, non-ASCII text, a number too large for a double and nesting on both sides of its
//! 128-level limit.

use anyhow::Result;

use crate::run_jq_full;

const RS: char = '\u{1e}';

/// The records, in the order the streams below repeat them.
fn records() -> Vec<String> {
    vec![
        "1.50".to_string(),
        "100000000000000000000".to_string(),
        "{\"a\":1,\"a\":2}".to_string(),
        "{\"b\":1.5,\"a\":\"café 😀\",\"c\":[true,null,{\"d\":-0}]}".to_string(),
        "[]".to_string(),
        "\"s\"".to_string(),
        "[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]".to_string(),
        "[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]".to_string(),
        "{\"id\":7,\"name\":\"n\"}".to_string(),
        "1e400".to_string(),
        "\"\\u00e9\\n\"".to_string(),
    ]
}

/// A stream of `records()`, each as `RS value LF`, with one truncated record (`RS {"id":-1,"name":"cut LF`)
/// at `position`: `first`, `last`, or after the fifth record.
fn stream(position: &str) -> String {
    let record = |r: &String| format!("{RS}{r}\n");
    let records = records();
    let bad = format!("{RS}{{\"id\":-1,\"name\":\"cut\n");
    match position {
        "first" => format!("{bad}{}", records.iter().map(record).collect::<String>()),
        "last" => format!("{}{bad}", records.iter().map(record).collect::<String>()),
        _ => format!(
            "{}{bad}{}",
            records[..5].iter().map(record).collect::<String>(),
            records[5..].iter().map(record).collect::<String>()
        ),
    }
}

#[test]
fn a_seq_stream_with_a_diagnostic_answers_as_jq_6620() -> Result<()> {
    for (position, flags, filter, want, want_err, want_code) in [
        ("first", "-c", ".", "\u{1e}1.50\n\u{1e}100000000000000000000\n\u{1e}{\"a\":2}\n\u{1e}{\"b\":1.5,\"a\":\"café 😀\",\"c\":[true,null,{\"d\":-0}]}\n\u{1e}[]\n\u{1e}\"s\"\n\u{1e}[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\u{1e}[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\u{1e}{\"id\":7,\"name\":\"n\"}\n\u{1e}1E+400\n\u{1e}\"é\\n\"\n", "jq: ignoring parse error: Truncated value at line 2, column 1\n", 0),
        ("first", "-c", "[.]|length", "\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n", "jq: ignoring parse error: Truncated value at line 2, column 1\n", 0),
        ("first", "-r", ".name? // .id? // \"x\"", "x\nx\nx\nx\nx\nx\nx\nx\nn\nx\nx\n", "jq: ignoring parse error: Truncated value at line 2, column 1\n", 0),
        ("first", "-c", "type", "\u{1e}\"number\"\n\u{1e}\"number\"\n\u{1e}\"object\"\n\u{1e}\"object\"\n\u{1e}\"array\"\n\u{1e}\"string\"\n\u{1e}\"array\"\n\u{1e}\"array\"\n\u{1e}\"object\"\n\u{1e}\"number\"\n\u{1e}\"string\"\n", "jq: ignoring parse error: Truncated value at line 2, column 1\n", 0),
        ("first", "-s -c", "length", "\u{1e}11\n", "jq: ignoring parse error: Truncated value at line 2, column 1\n", 0),
        ("first", "-c", "tojson|length", "\u{1e}4\n\u{1e}21\n\u{1e}7\n\u{1e}47\n\u{1e}2\n\u{1e}3\n\u{1e}201\n\u{1e}261\n\u{1e}19\n\u{1e}6\n\u{1e}5\n", "jq: ignoring parse error: Truncated value at line 2, column 1\n", 0),
        ("middle", "-c", ".", "\u{1e}1.50\n\u{1e}100000000000000000000\n\u{1e}{\"a\":2}\n\u{1e}{\"b\":1.5,\"a\":\"café 😀\",\"c\":[true,null,{\"d\":-0}]}\n\u{1e}[]\n\u{1e}\"s\"\n\u{1e}[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\u{1e}[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\u{1e}{\"id\":7,\"name\":\"n\"}\n\u{1e}1E+400\n\u{1e}\"é\\n\"\n", "jq: ignoring parse error: Truncated value at line 7, column 1\n", 0),
        ("middle", "-c", "[.]|length", "\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n", "jq: ignoring parse error: Truncated value at line 7, column 1\n", 0),
        ("middle", "-r", ".name? // .id? // \"x\"", "x\nx\nx\nx\nx\nx\nx\nx\nn\nx\nx\n", "jq: ignoring parse error: Truncated value at line 7, column 1\n", 0),
        ("middle", "-c", "type", "\u{1e}\"number\"\n\u{1e}\"number\"\n\u{1e}\"object\"\n\u{1e}\"object\"\n\u{1e}\"array\"\n\u{1e}\"string\"\n\u{1e}\"array\"\n\u{1e}\"array\"\n\u{1e}\"object\"\n\u{1e}\"number\"\n\u{1e}\"string\"\n", "jq: ignoring parse error: Truncated value at line 7, column 1\n", 0),
        ("middle", "-s -c", "length", "\u{1e}11\n", "jq: ignoring parse error: Truncated value at line 7, column 1\n", 0),
        ("middle", "-c", "tojson|length", "\u{1e}4\n\u{1e}21\n\u{1e}7\n\u{1e}47\n\u{1e}2\n\u{1e}3\n\u{1e}201\n\u{1e}261\n\u{1e}19\n\u{1e}6\n\u{1e}5\n", "jq: ignoring parse error: Truncated value at line 7, column 1\n", 0),
        ("last", "-c", ".", "\u{1e}1.50\n\u{1e}100000000000000000000\n\u{1e}{\"a\":2}\n\u{1e}{\"b\":1.5,\"a\":\"café 😀\",\"c\":[true,null,{\"d\":-0}]}\n\u{1e}[]\n\u{1e}\"s\"\n\u{1e}[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\u{1e}[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\u{1e}{\"id\":7,\"name\":\"n\"}\n\u{1e}1E+400\n\u{1e}\"é\\n\"\n", "jq: ignoring parse error: Unfinished string at EOF at line 13, column 0\n", 0),
        ("last", "-c", "[.]|length", "\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n\u{1e}1\n", "jq: ignoring parse error: Unfinished string at EOF at line 13, column 0\n", 0),
        ("last", "-r", ".name? // .id? // \"x\"", "x\nx\nx\nx\nx\nx\nx\nx\nn\nx\nx\n", "jq: ignoring parse error: Unfinished string at EOF at line 13, column 0\n", 0),
        ("last", "-c", "type", "\u{1e}\"number\"\n\u{1e}\"number\"\n\u{1e}\"object\"\n\u{1e}\"object\"\n\u{1e}\"array\"\n\u{1e}\"string\"\n\u{1e}\"array\"\n\u{1e}\"array\"\n\u{1e}\"object\"\n\u{1e}\"number\"\n\u{1e}\"string\"\n", "jq: ignoring parse error: Unfinished string at EOF at line 13, column 0\n", 0),
        ("last", "-s -c", "length", "\u{1e}11\n", "jq: ignoring parse error: Unfinished string at EOF at line 13, column 0\n", 0),
        ("last", "-c", "tojson|length", "\u{1e}4\n\u{1e}21\n\u{1e}7\n\u{1e}47\n\u{1e}2\n\u{1e}3\n\u{1e}201\n\u{1e}261\n\u{1e}19\n\u{1e}6\n\u{1e}5\n", "jq: ignoring parse error: Unfinished string at EOF at line 13, column 0\n", 0),
    ] {
        let mut args = vec!["--seq"];
        args.extend(flags.split_whitespace());
        args.push(filter);
        let (stdout, stderr, code) = run_jq_full(&args, Some(&stream(position)))?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (want, want_err, want_code),
            "{position} {flags} {filter}"
        );
    }
    Ok(())
}

/// Records the one-pass reader must hand to the materializer, beside a truncated record: invalid
/// UTF-8 (substituted before either reads it), jq's `nan` and leading zero, a repeated key, and nesting
/// on both sides of the reader's 128-level limit and at jq's own 256. Raw bytes in, raw bytes out,
/// captured from jq 1.7.1. Printing a value exactly 256 deep (`.`) is left out: succinctly refuses it
/// with or without the one-pass reader (#6618, #6631).
#[test]
fn records_the_one_pass_reader_declines_answer_as_jq_6620() -> Result<()> {
    for (name, position, flags, filter, stream, want, want_err, want_code) in [
        ("invalid UTF-8 in a string", "first", "-c", ".", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e{\"a\":\"x\xffy\",\"b\":1}\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e{\"a\":\"x\xef\xbf\xbdy\",\"b\":1}\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("invalid UTF-8 in a string", "first", "-c", "type", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e{\"a\":\"x\xffy\",\"b\":1}\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e\"object\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("invalid UTF-8 in a string", "last", "-c", ".", b"\x1e{\"a\":\"x\xffy\",\"b\":1}\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e{\"a\":\"x\xef\xbf\xbdy\",\"b\":1}\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("invalid UTF-8 in a string", "last", "-c", "type", b"\x1e{\"a\":\"x\xffy\",\"b\":1}\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e\"object\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("invalid UTF-8 as a key", "first", "-c", ".", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e{\"k\xfe\":1}\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e{\"k\xef\xbf\xbd\":1}\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("invalid UTF-8 as a key", "first", "-c", "type", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e{\"k\xfe\":1}\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e\"object\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("invalid UTF-8 as a key", "last", "-c", ".", b"\x1e{\"k\xfe\":1}\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e{\"k\xef\xbf\xbd\":1}\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("invalid UTF-8 as a key", "last", "-c", "type", b"\x1e{\"k\xfe\":1}\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e\"object\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("nan", "first", "-c", ".", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1enan\n\x1e{\"ok\":true}\n" as &[u8], b"\x1enull\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("nan", "first", "-c", "type", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1enan\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e\"number\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("nan", "last", "-c", ".", b"\x1enan\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1enull\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("nan", "last", "-c", "type", b"\x1enan\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e\"number\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("nan in an array", "first", "-c", ".", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e[1,nan,3]\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e[1,null,3]\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("nan in an array", "first", "-c", "type", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e[1,nan,3]\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e\"array\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("nan in an array", "last", "-c", ".", b"\x1e[1,nan,3]\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e[1,null,3]\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("nan in an array", "last", "-c", "type", b"\x1e[1,nan,3]\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e\"array\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("leading zero", "first", "-c", ".", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e01\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e1\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("leading zero", "first", "-c", "type", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e01\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e\"number\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("leading zero", "last", "-c", ".", b"\x1e01\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e1\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("leading zero", "last", "-c", "type", b"\x1e01\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e\"number\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("leading zero in an array", "first", "-c", ".", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e[01,2]\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e[1,2]\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("leading zero in an array", "first", "-c", "type", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e[01,2]\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e\"array\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("leading zero in an array", "last", "-c", ".", b"\x1e[01,2]\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e[1,2]\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("leading zero in an array", "last", "-c", "type", b"\x1e[01,2]\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e\"array\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("depth 127", "first", "-c", ".", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("depth 127", "first", "-c", "type", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e\"array\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("depth 127", "last", "-c", ".", b"\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("depth 127", "last", "-c", "type", b"\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e\"array\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("depth 128", "first", "-c", ".", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("depth 128", "first", "-c", "type", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e\"array\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("depth 128", "last", "-c", ".", b"\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("depth 128", "last", "-c", "type", b"\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e\"array\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("depth 129", "first", "-c", ".", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("depth 129", "first", "-c", "type", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e\"array\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("depth 129", "last", "-c", ".", b"\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("depth 129", "last", "-c", "type", b"\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e\"array\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("depth 256", "first", "-c", "type", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e\"array\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("depth 256", "last", "-c", "type", b"\x1e[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e\"array\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("repeated key", "first", "-c", ".", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e{\"a\":1,\"a\":2}\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e{\"a\":2}\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("repeated key", "first", "-c", "type", b"\x1e{\"id\":-1,\"name\":\"cut\n\x1e{\"a\":1,\"a\":2}\n\x1e{\"ok\":true}\n" as &[u8], b"\x1e\"object\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Truncated value at line 2, column 1\n" as &[u8], 0),
        ("repeated key", "last", "-c", ".", b"\x1e{\"a\":1,\"a\":2}\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e{\"a\":2}\n\x1e{\"ok\":true}\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
        ("repeated key", "last", "-c", "type", b"\x1e{\"a\":1,\"a\":2}\n\x1e{\"ok\":true}\n\x1e{\"id\":-1,\"name\":\"cut\n" as &[u8], b"\x1e\"object\"\n\x1e\"object\"\n" as &[u8], b"jq: ignoring parse error: Unfinished string at EOF at line 4, column 0\n" as &[u8], 0),
    ] {
        let mut args = vec!["--seq"];
        args.extend(flags.split_whitespace());
        args.push(filter);
        let (output, code) = crate::spawn_jq(&args, Some(stream))?;
        assert_eq!(
            (output.stdout.as_slice(), output.stderr.as_slice(), code),
            (want, want_err, want_code),
            "{name}, fault {position}: {flags} {filter}"
        );
    }
    Ok(())
}

/// The records a program the owned door answers (`.name`, `{id,name}`, `select(...)`) and one it
/// declines (`[.id,.name]`, `keys`, a program that reads `input`) both read, with one truncated
/// record at each position: the door is taken per record (#6620), and a run of declines closes it
/// for the rest of the stream. Every expected string was captured from jq 1.7.1.
fn door_stream(position: &str) -> String {
    let records = [
        "{\"id\":1,\"name\":\"Alice\",\"age\":31,\"tags\":[\"a\",\"b\"]}",
        "{\"name\":\"b\",\"id\":2.50}",
        "[3,{\"name\":\"c\"}]",
        "1.50",
        "{\"id\":4,\"age\":50,\"name\":null}",
        "\"s\"",
        "{\"a\":{\"b\":1},\"name\":\"deep\",\"id\":5}",
    ];
    let record = |r: &&str| format!("{RS}{r}\n");
    let bad = format!("{RS}{{\"id\":-1,\"name\":\"cut\n");
    match position {
        "first" => format!("{bad}{}", records.iter().map(record).collect::<String>()),
        "last" => format!("{}{bad}", records.iter().map(record).collect::<String>()),
        _ => format!(
            "{}{bad}{}",
            records[..3].iter().map(record).collect::<String>(),
            records[3..].iter().map(record).collect::<String>()
        ),
    }
}

#[test]
fn programs_over_a_seq_stream_with_a_diagnostic_answer_as_jq_6620() -> Result<()> {
    for (position, flags, filter, want, want_err, want_code) in [
        ("first", "-c", ".name", "\u{1e}\"Alice\"\n\u{1e}\"b\"\n\u{1e}null\n\u{1e}\"deep\"\n", "jq: ignoring parse error: Truncated value at line 2, column 1\njq: error (at <stdin>:4): Cannot index array with string \"name\"\njq: error (at <stdin>:5): Cannot index number with string \"name\"\njq: error (at <stdin>:7): Cannot index string with string \"name\"\n", 0),
        ("first", "-c", ".id", "\u{1e}1\n\u{1e}2.50\n\u{1e}4\n\u{1e}5\n", "jq: ignoring parse error: Truncated value at line 2, column 1\njq: error (at <stdin>:4): Cannot index array with string \"id\"\njq: error (at <stdin>:5): Cannot index number with string \"id\"\njq: error (at <stdin>:7): Cannot index string with string \"id\"\n", 0),
        ("first", "-c", "{id,name}", "\u{1e}{\"id\":1,\"name\":\"Alice\"}\n\u{1e}{\"id\":2.50,\"name\":\"b\"}\n\u{1e}{\"id\":4,\"name\":null}\n\u{1e}{\"id\":5,\"name\":\"deep\"}\n", "jq: ignoring parse error: Truncated value at line 2, column 1\njq: error (at <stdin>:4): Cannot index array with string \"id\"\njq: error (at <stdin>:5): Cannot index number with string \"id\"\njq: error (at <stdin>:7): Cannot index string with string \"id\"\n", 0),
        ("first", "-c", "select(.age>30)|.name", "\u{1e}\"Alice\"\n\u{1e}null\n", "jq: ignoring parse error: Truncated value at line 2, column 1\njq: error (at <stdin>:4): Cannot index array with string \"age\"\njq: error (at <stdin>:5): Cannot index number with string \"age\"\njq: error (at <stdin>:7): Cannot index string with string \"age\"\n", 0),
        ("first", "-c", "[.id,.name]", "\u{1e}[1,\"Alice\"]\n\u{1e}[2.50,\"b\"]\n\u{1e}[4,null]\n\u{1e}[5,\"deep\"]\n", "jq: ignoring parse error: Truncated value at line 2, column 1\njq: error (at <stdin>:4): Cannot index array with string \"id\"\njq: error (at <stdin>:5): Cannot index number with string \"id\"\njq: error (at <stdin>:7): Cannot index string with string \"id\"\n", 0),
        ("first", "-c", "keys", "\u{1e}[\"age\",\"id\",\"name\",\"tags\"]\n\u{1e}[\"id\",\"name\"]\n\u{1e}[0,1]\n\u{1e}[\"age\",\"id\",\"name\"]\n\u{1e}[\"a\",\"id\",\"name\"]\n", "jq: ignoring parse error: Truncated value at line 2, column 1\njq: error (at <stdin>:5): number (1.50) has no keys\njq: error (at <stdin>:7): string (\"s\") has no keys\n", 0),
        ("first", "-c", ".name, (try (input|.name) catch \"x\")", "\u{1e}\"Alice\"\n\u{1e}\"b\"\n\u{1e}null\n\u{1e}\"x\"\n\u{1e}\"deep\"\n\u{1e}\"x\"\n", "jq: ignoring parse error: Truncated value at line 2, column 1\njq: error (at <stdin>:4): Cannot index array with string \"name\"\njq: error (at <stdin>:5): Cannot index number with string \"name\"\n", 0),
        ("first", "-c", "[.,input]", "\u{1e}[{\"id\":1,\"name\":\"Alice\",\"age\":31,\"tags\":[\"a\",\"b\"]},{\"name\":\"b\",\"id\":2.50}]\n\u{1e}[[3,{\"name\":\"c\"}],1.50]\n\u{1e}[{\"id\":4,\"age\":50,\"name\":null},\"s\"]\n", "jq: ignoring parse error: Truncated value at line 2, column 1\njq: error (at <stdin>:8): break\n", 5),
        ("first", "-c", "debug|.name?", "\u{1e}\"Alice\"\n\u{1e}\"b\"\n\u{1e}null\n\u{1e}\"deep\"\n", "jq: ignoring parse error: Truncated value at line 2, column 1\n[\"DEBUG:\",{\"id\":1,\"name\":\"Alice\",\"age\":31,\"tags\":[\"a\",\"b\"]}]\n[\"DEBUG:\",{\"name\":\"b\",\"id\":2.50}]\n[\"DEBUG:\",[3,{\"name\":\"c\"}]]\n[\"DEBUG:\",1.50]\n[\"DEBUG:\",{\"id\":4,\"age\":50,\"name\":null}]\n[\"DEBUG:\",\"s\"]\n[\"DEBUG:\",{\"a\":{\"b\":1},\"name\":\"deep\",\"id\":5}]\n", 0),
        ("first", "-r", ".name?", "Alice\nb\n\u{1e}null\ndeep\n", "jq: ignoring parse error: Truncated value at line 2, column 1\n", 0),
        ("first", "-c", "del(.name)?", "\u{1e}{\"id\":1,\"age\":31,\"tags\":[\"a\",\"b\"]}\n\u{1e}{\"id\":2.50}\n\u{1e}{\"id\":4,\"age\":50}\n\u{1e}{\"a\":{\"b\":1},\"id\":5}\n", "jq: ignoring parse error: Truncated value at line 2, column 1\n", 0),
        ("first", "-c", ".x=1", "\u{1e}{\"id\":1,\"name\":\"Alice\",\"age\":31,\"tags\":[\"a\",\"b\"],\"x\":1}\n\u{1e}{\"name\":\"b\",\"id\":2.50,\"x\":1}\n\u{1e}{\"id\":4,\"age\":50,\"name\":null,\"x\":1}\n\u{1e}{\"a\":{\"b\":1},\"name\":\"deep\",\"id\":5,\"x\":1}\n", "jq: ignoring parse error: Truncated value at line 2, column 1\njq: error (at <stdin>:4): Cannot index array with string \"x\"\njq: error (at <stdin>:5): Cannot index number with string \"x\"\njq: error (at <stdin>:7): Cannot index string with string \"x\"\n", 0),
        ("first", "-c", "to_entries|length", "\u{1e}4\n\u{1e}2\n\u{1e}2\n\u{1e}3\n\u{1e}3\n", "jq: ignoring parse error: Truncated value at line 2, column 1\njq: error (at <stdin>:5): number (1.50) has no keys\njq: error (at <stdin>:7): string (\"s\") has no keys\n", 0),
        ("first", "-c", "input_line_number", "\u{1e}2\n\u{1e}3\n\u{1e}4\n\u{1e}5\n\u{1e}6\n\u{1e}7\n\u{1e}8\n", "jq: ignoring parse error: Truncated value at line 2, column 1\n", 0),
        ("first", "-c", "try error catch .", "\u{1e}{\"id\":1,\"name\":\"Alice\",\"age\":31,\"tags\":[\"a\",\"b\"]}\n\u{1e}{\"name\":\"b\",\"id\":2.50}\n\u{1e}[3,{\"name\":\"c\"}]\n\u{1e}1.50\n\u{1e}{\"id\":4,\"age\":50,\"name\":null}\n\u{1e}\"s\"\n\u{1e}{\"a\":{\"b\":1},\"name\":\"deep\",\"id\":5}\n", "jq: ignoring parse error: Truncated value at line 2, column 1\n", 0),
        ("middle", "-c", ".name", "\u{1e}\"Alice\"\n\u{1e}\"b\"\n\u{1e}null\n\u{1e}\"deep\"\n", "jq: error (at <stdin>:3): Cannot index array with string \"name\"\njq: ignoring parse error: Truncated value at line 5, column 1\njq: error (at <stdin>:5): Cannot index number with string \"name\"\njq: error (at <stdin>:7): Cannot index string with string \"name\"\n", 0),
        ("middle", "-c", ".id", "\u{1e}1\n\u{1e}2.50\n\u{1e}4\n\u{1e}5\n", "jq: error (at <stdin>:3): Cannot index array with string \"id\"\njq: ignoring parse error: Truncated value at line 5, column 1\njq: error (at <stdin>:5): Cannot index number with string \"id\"\njq: error (at <stdin>:7): Cannot index string with string \"id\"\n", 0),
        ("middle", "-c", "{id,name}", "\u{1e}{\"id\":1,\"name\":\"Alice\"}\n\u{1e}{\"id\":2.50,\"name\":\"b\"}\n\u{1e}{\"id\":4,\"name\":null}\n\u{1e}{\"id\":5,\"name\":\"deep\"}\n", "jq: error (at <stdin>:3): Cannot index array with string \"id\"\njq: ignoring parse error: Truncated value at line 5, column 1\njq: error (at <stdin>:5): Cannot index number with string \"id\"\njq: error (at <stdin>:7): Cannot index string with string \"id\"\n", 0),
        ("middle", "-c", "select(.age>30)|.name", "\u{1e}\"Alice\"\n\u{1e}null\n", "jq: error (at <stdin>:3): Cannot index array with string \"age\"\njq: ignoring parse error: Truncated value at line 5, column 1\njq: error (at <stdin>:5): Cannot index number with string \"age\"\njq: error (at <stdin>:7): Cannot index string with string \"age\"\n", 0),
        ("middle", "-c", "[.id,.name]", "\u{1e}[1,\"Alice\"]\n\u{1e}[2.50,\"b\"]\n\u{1e}[4,null]\n\u{1e}[5,\"deep\"]\n", "jq: error (at <stdin>:3): Cannot index array with string \"id\"\njq: ignoring parse error: Truncated value at line 5, column 1\njq: error (at <stdin>:5): Cannot index number with string \"id\"\njq: error (at <stdin>:7): Cannot index string with string \"id\"\n", 0),
        ("middle", "-c", "keys", "\u{1e}[\"age\",\"id\",\"name\",\"tags\"]\n\u{1e}[\"id\",\"name\"]\n\u{1e}[0,1]\n\u{1e}[\"age\",\"id\",\"name\"]\n\u{1e}[\"a\",\"id\",\"name\"]\n", "jq: ignoring parse error: Truncated value at line 5, column 1\njq: error (at <stdin>:5): number (1.50) has no keys\njq: error (at <stdin>:7): string (\"s\") has no keys\n", 0),
        ("middle", "-c", ".name, (try (input|.name) catch \"x\")", "\u{1e}\"Alice\"\n\u{1e}\"b\"\n\u{1e}null\n\u{1e}\"x\"\n\u{1e}\"deep\"\n\u{1e}\"x\"\n", "jq: error (at <stdin>:3): Cannot index array with string \"name\"\njq: ignoring parse error: Truncated value at line 5, column 1\njq: error (at <stdin>:5): Cannot index number with string \"name\"\n", 0),
        ("middle", "-c", "[.,input]", "\u{1e}[{\"id\":1,\"name\":\"Alice\",\"age\":31,\"tags\":[\"a\",\"b\"]},{\"name\":\"b\",\"id\":2.50}]\n\u{1e}[1.50,{\"id\":4,\"age\":50,\"name\":null}]\n\u{1e}[\"s\",{\"a\":{\"b\":1},\"name\":\"deep\",\"id\":5}]\n", "jq: error (at <stdin>:5): Truncated value at line 5, column 1\n", 0),
        ("middle", "-c", "debug|.name?", "\u{1e}\"Alice\"\n\u{1e}\"b\"\n\u{1e}null\n\u{1e}\"deep\"\n", "[\"DEBUG:\",{\"id\":1,\"name\":\"Alice\",\"age\":31,\"tags\":[\"a\",\"b\"]}]\n[\"DEBUG:\",{\"name\":\"b\",\"id\":2.50}]\n[\"DEBUG:\",[3,{\"name\":\"c\"}]]\njq: ignoring parse error: Truncated value at line 5, column 1\n[\"DEBUG:\",1.50]\n[\"DEBUG:\",{\"id\":4,\"age\":50,\"name\":null}]\n[\"DEBUG:\",\"s\"]\n[\"DEBUG:\",{\"a\":{\"b\":1},\"name\":\"deep\",\"id\":5}]\n", 0),
        ("middle", "-r", ".name?", "Alice\nb\n\u{1e}null\ndeep\n", "jq: ignoring parse error: Truncated value at line 5, column 1\n", 0),
        ("middle", "-c", "del(.name)?", "\u{1e}{\"id\":1,\"age\":31,\"tags\":[\"a\",\"b\"]}\n\u{1e}{\"id\":2.50}\n\u{1e}{\"id\":4,\"age\":50}\n\u{1e}{\"a\":{\"b\":1},\"id\":5}\n", "jq: ignoring parse error: Truncated value at line 5, column 1\n", 0),
        ("middle", "-c", ".x=1", "\u{1e}{\"id\":1,\"name\":\"Alice\",\"age\":31,\"tags\":[\"a\",\"b\"],\"x\":1}\n\u{1e}{\"name\":\"b\",\"id\":2.50,\"x\":1}\n\u{1e}{\"id\":4,\"age\":50,\"name\":null,\"x\":1}\n\u{1e}{\"a\":{\"b\":1},\"name\":\"deep\",\"id\":5,\"x\":1}\n", "jq: error (at <stdin>:3): Cannot index array with string \"x\"\njq: ignoring parse error: Truncated value at line 5, column 1\njq: error (at <stdin>:5): Cannot index number with string \"x\"\njq: error (at <stdin>:7): Cannot index string with string \"x\"\n", 0),
        ("middle", "-c", "to_entries|length", "\u{1e}4\n\u{1e}2\n\u{1e}2\n\u{1e}3\n\u{1e}3\n", "jq: ignoring parse error: Truncated value at line 5, column 1\njq: error (at <stdin>:5): number (1.50) has no keys\njq: error (at <stdin>:7): string (\"s\") has no keys\n", 0),
        ("middle", "-c", "input_line_number", "\u{1e}1\n\u{1e}2\n\u{1e}3\n\u{1e}5\n\u{1e}6\n\u{1e}7\n\u{1e}8\n", "jq: ignoring parse error: Truncated value at line 5, column 1\n", 0),
        ("middle", "-c", "try error catch .", "\u{1e}{\"id\":1,\"name\":\"Alice\",\"age\":31,\"tags\":[\"a\",\"b\"]}\n\u{1e}{\"name\":\"b\",\"id\":2.50}\n\u{1e}[3,{\"name\":\"c\"}]\n\u{1e}1.50\n\u{1e}{\"id\":4,\"age\":50,\"name\":null}\n\u{1e}\"s\"\n\u{1e}{\"a\":{\"b\":1},\"name\":\"deep\",\"id\":5}\n", "jq: ignoring parse error: Truncated value at line 5, column 1\n", 0),
        ("last", "-c", ".name", "\u{1e}\"Alice\"\n\u{1e}\"b\"\n\u{1e}null\n\u{1e}\"deep\"\n", "jq: error (at <stdin>:3): Cannot index array with string \"name\"\njq: error (at <stdin>:4): Cannot index number with string \"name\"\njq: error (at <stdin>:6): Cannot index string with string \"name\"\njq: ignoring parse error: Unfinished string at EOF at line 9, column 0\n", 0),
        ("last", "-c", ".id", "\u{1e}1\n\u{1e}2.50\n\u{1e}4\n\u{1e}5\n", "jq: error (at <stdin>:3): Cannot index array with string \"id\"\njq: error (at <stdin>:4): Cannot index number with string \"id\"\njq: error (at <stdin>:6): Cannot index string with string \"id\"\njq: ignoring parse error: Unfinished string at EOF at line 9, column 0\n", 0),
        ("last", "-c", "{id,name}", "\u{1e}{\"id\":1,\"name\":\"Alice\"}\n\u{1e}{\"id\":2.50,\"name\":\"b\"}\n\u{1e}{\"id\":4,\"name\":null}\n\u{1e}{\"id\":5,\"name\":\"deep\"}\n", "jq: error (at <stdin>:3): Cannot index array with string \"id\"\njq: error (at <stdin>:4): Cannot index number with string \"id\"\njq: error (at <stdin>:6): Cannot index string with string \"id\"\njq: ignoring parse error: Unfinished string at EOF at line 9, column 0\n", 0),
        ("last", "-c", "select(.age>30)|.name", "\u{1e}\"Alice\"\n\u{1e}null\n", "jq: error (at <stdin>:3): Cannot index array with string \"age\"\njq: error (at <stdin>:4): Cannot index number with string \"age\"\njq: error (at <stdin>:6): Cannot index string with string \"age\"\njq: ignoring parse error: Unfinished string at EOF at line 9, column 0\n", 0),
        ("last", "-c", "[.id,.name]", "\u{1e}[1,\"Alice\"]\n\u{1e}[2.50,\"b\"]\n\u{1e}[4,null]\n\u{1e}[5,\"deep\"]\n", "jq: error (at <stdin>:3): Cannot index array with string \"id\"\njq: error (at <stdin>:4): Cannot index number with string \"id\"\njq: error (at <stdin>:6): Cannot index string with string \"id\"\njq: ignoring parse error: Unfinished string at EOF at line 9, column 0\n", 0),
        ("last", "-c", "keys", "\u{1e}[\"age\",\"id\",\"name\",\"tags\"]\n\u{1e}[\"id\",\"name\"]\n\u{1e}[0,1]\n\u{1e}[\"age\",\"id\",\"name\"]\n\u{1e}[\"a\",\"id\",\"name\"]\n", "jq: error (at <stdin>:4): number (1.50) has no keys\njq: error (at <stdin>:6): string (\"s\") has no keys\njq: ignoring parse error: Unfinished string at EOF at line 9, column 0\n", 0),
        ("last", "-c", ".name, (try (input|.name) catch \"x\")", "\u{1e}\"Alice\"\n\u{1e}\"b\"\n\u{1e}null\n\u{1e}\"x\"\n\u{1e}\"deep\"\n\u{1e}\"x\"\n", "jq: error (at <stdin>:3): Cannot index array with string \"name\"\njq: error (at <stdin>:4): Cannot index number with string \"name\"\n", 0),
        ("last", "-c", "[.,input]", "\u{1e}[{\"id\":1,\"name\":\"Alice\",\"age\":31,\"tags\":[\"a\",\"b\"]},{\"name\":\"b\",\"id\":2.50}]\n\u{1e}[[3,{\"name\":\"c\"}],1.50]\n\u{1e}[{\"id\":4,\"age\":50,\"name\":null},\"s\"]\n", "jq: error (at <stdin>:8): Unfinished string at EOF at line 9, column 0\n", 5),
        ("last", "-c", "debug|.name?", "\u{1e}\"Alice\"\n\u{1e}\"b\"\n\u{1e}null\n\u{1e}\"deep\"\n", "[\"DEBUG:\",{\"id\":1,\"name\":\"Alice\",\"age\":31,\"tags\":[\"a\",\"b\"]}]\n[\"DEBUG:\",{\"name\":\"b\",\"id\":2.50}]\n[\"DEBUG:\",[3,{\"name\":\"c\"}]]\n[\"DEBUG:\",1.50]\n[\"DEBUG:\",{\"id\":4,\"age\":50,\"name\":null}]\n[\"DEBUG:\",\"s\"]\n[\"DEBUG:\",{\"a\":{\"b\":1},\"name\":\"deep\",\"id\":5}]\njq: ignoring parse error: Unfinished string at EOF at line 9, column 0\n", 0),
        ("last", "-r", ".name?", "Alice\nb\n\u{1e}null\ndeep\n", "jq: ignoring parse error: Unfinished string at EOF at line 9, column 0\n", 0),
        ("last", "-c", "del(.name)?", "\u{1e}{\"id\":1,\"age\":31,\"tags\":[\"a\",\"b\"]}\n\u{1e}{\"id\":2.50}\n\u{1e}{\"id\":4,\"age\":50}\n\u{1e}{\"a\":{\"b\":1},\"id\":5}\n", "jq: ignoring parse error: Unfinished string at EOF at line 9, column 0\n", 0),
        ("last", "-c", ".x=1", "\u{1e}{\"id\":1,\"name\":\"Alice\",\"age\":31,\"tags\":[\"a\",\"b\"],\"x\":1}\n\u{1e}{\"name\":\"b\",\"id\":2.50,\"x\":1}\n\u{1e}{\"id\":4,\"age\":50,\"name\":null,\"x\":1}\n\u{1e}{\"a\":{\"b\":1},\"name\":\"deep\",\"id\":5,\"x\":1}\n", "jq: error (at <stdin>:3): Cannot index array with string \"x\"\njq: error (at <stdin>:4): Cannot index number with string \"x\"\njq: error (at <stdin>:6): Cannot index string with string \"x\"\njq: ignoring parse error: Unfinished string at EOF at line 9, column 0\n", 0),
        ("last", "-c", "to_entries|length", "\u{1e}4\n\u{1e}2\n\u{1e}2\n\u{1e}3\n\u{1e}3\n", "jq: error (at <stdin>:4): number (1.50) has no keys\njq: error (at <stdin>:6): string (\"s\") has no keys\njq: ignoring parse error: Unfinished string at EOF at line 9, column 0\n", 0),
        ("last", "-c", "input_line_number", "\u{1e}1\n\u{1e}2\n\u{1e}3\n\u{1e}4\n\u{1e}5\n\u{1e}6\n\u{1e}7\n", "jq: ignoring parse error: Unfinished string at EOF at line 9, column 0\n", 0),
        ("last", "-c", "try error catch .", "\u{1e}{\"id\":1,\"name\":\"Alice\",\"age\":31,\"tags\":[\"a\",\"b\"]}\n\u{1e}{\"name\":\"b\",\"id\":2.50}\n\u{1e}[3,{\"name\":\"c\"}]\n\u{1e}1.50\n\u{1e}{\"id\":4,\"age\":50,\"name\":null}\n\u{1e}\"s\"\n\u{1e}{\"a\":{\"b\":1},\"name\":\"deep\",\"id\":5}\n", "jq: ignoring parse error: Unfinished string at EOF at line 9, column 0\n", 0),
    ] {
        let mut args = vec!["--seq"];
        args.extend(flags.split_whitespace());
        args.push(filter);
        let (stdout, stderr, code) = run_jq_full(&args, Some(&door_stream(position)))?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (want, want_err, want_code),
            "{position} {flags} {filter}"
        );
    }
    Ok(())
}
