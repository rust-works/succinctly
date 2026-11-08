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
