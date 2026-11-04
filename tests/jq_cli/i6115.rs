//! #6115: `--seq` with `--stream` or `--stream-errors` reads with jq's streaming parser in its
//! record-sequence mode, so the program reads the events of each record (a truncated record gives
//! the events before the cut), and a fault in a record is the warning `jq: ignoring parse error`,
//! or under `--stream-errors` an event `[message, path]`. Before this the flags were ignored under
//! `--seq`, and the program read the records as documents. A program that reads inputs gets a fault
//! as the error `input` raises, and `-s` closes its array where jq does. Every expectation was
//! captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::{run_jq_full, run_jq_over_byte_files};

/// `(stdin, flags, program, stdout, stderr, exit code)`.
type Row = (
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
    &'static str,
    i32,
);

const ROWS: &[Row] = &[
    ("\u{1e}[1]\n\u{1e}{\"a\":2}\n", &["-c", "--seq", "--stream"], ".", "\u{1e}[[0],1]\n\u{1e}[[0]]\n\u{1e}[[\"a\"],2]\n\u{1e}[[\"a\"]]\n", "", 0),
    ("\u{1e}[1]\n\u{1e}{\"a\":2}\n", &["-c", "--seq", "--stream"], "length", "\u{1e}2\n\u{1e}1\n\u{1e}2\n\u{1e}1\n", "", 0),
    ("\u{1e}[1]\n\u{1e}{\"a\":2}\n", &["-c", "--stream", "--seq"], ".", "\u{1e}[[0],1]\n\u{1e}[[0]]\n\u{1e}[[\"a\"],2]\n\u{1e}[[\"a\"]]\n", "", 0),
    ("\u{1e}[1]\n\u{1e}{\"a\":2}\n", &["-c", "--seq", "--stream", "-s"], ".", "\u{1e}[[[0],1],[[0]],[[\"a\"],2],[[\"a\"]]]\n", "", 0),
    ("\u{1e}[1]\n\u{1e}{\"a\":2}\n", &["-c", "--seq", "--stream", "-n"], "[inputs]", "\u{1e}[[[0],1],[[0]],[[\"a\"],2],[[\"a\"]]]\n", "", 0),
    ("\u{1e}3\n\u{1e}\"x\"\n", &["-c", "--seq", "--stream"], ".", "\u{1e}[[],3]\n\u{1e}[[],\"x\"]\n", "", 0),
    ("\u{1e}3\u{1e}\"x\"\n", &["-c", "--seq", "--stream"], ".", "\u{1e}[[],3]\n\u{1e}[[],\"x\"]\n", "", 0),
    ("\u{1e}[]\u{1e}{}\n", &["-c", "--seq", "--stream"], ".", "\u{1e}[[],[]]\n\u{1e}[[],{}]\n", "", 0),
    ("\u{1e}{\"a\":[1,{\"b\":null}],\"c\":\"d\"}\n", &["-c", "--seq", "--stream"], ".", "\u{1e}[[\"a\",0],1]\n\u{1e}[[\"a\",1,\"b\"],null]\n\u{1e}[[\"a\",1,\"b\"]]\n\u{1e}[[\"a\",1]]\n\u{1e}[[\"c\"],\"d\"]\n\u{1e}[[\"c\"]]\n", "", 0),
    ("\u{1e}[1,\n\u{1e}{\"a\":2}\n", &["-c", "--seq", "--stream"], ".", "\u{1e}[[0],1]\n\u{1e}[[\"a\"],2]\n\u{1e}[[\"a\"]]\n", "jq: ignoring parse error: Truncated value at line 2, column 1\n", 0),
    ("\u{1e}[1,2", &["-c", "--seq", "--stream"], ".", "\u{1e}[[0],1]\n", "jq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 5\n", 0),
    ("\u{1e}[1,]\n\u{1e}[2]\n", &["-c", "--seq", "--stream"], ".", "\u{1e}[[0],1]\n\u{1e}[[0],2]\n\u{1e}[[0]]\n", "jq: ignoring parse error: Expected another array element at line 1, column 5 (need RS to resync)\n", 0),
    ("\u{1e}[1,2\u{1e}[3]\n", &["-c", "--seq", "--stream"], ".", "\u{1e}[[0],1]\n\u{1e}[[0],3]\n\u{1e}[[0]]\n", "jq: ignoring parse error: Truncated value at line 1, column 6\n", 0),
    ("\u{1e}[1]\n garbage\n\u{1e}[2]\n", &["-c", "--seq", "--stream"], ".", "\u{1e}[[0],1]\n\u{1e}[[0]]\n\u{1e}[[0],2]\n\u{1e}[[0]]\n", "jq: ignoring parse error: Invalid numeric literal at line 3, column 0 (need RS to resync)\n", 0),
    ("null", &["-c", "--seq", "--stream"], ".", "", "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 4\n", 0),
    ("\u{1e}12", &["-c", "--seq", "--stream"], ".", "\u{1e}[[],12]\n", "", 0),
    ("\u{1e}[1]\n\u{1e}\u{1e}[2]\n", &["-c", "--seq", "--stream"], ".", "\u{1e}[[0],1]\n\u{1e}[[0]]\n\u{1e}[[0],2]\n\u{1e}[[0]]\n", "", 0),
    ("\u{1e}[1]\n\u{1e}[2", &["-c", "--seq", "--stream"], ".", "\u{1e}[[0],1]\n\u{1e}[[0]]\n", "", 0),
    ("null", &["-c", "--seq", "--stream-errors"], ".", "\u{1e}[\"Unfinished abandoned text at EOF at line 1, column 4\",[]]\n", "", 0),
    ("\u{1e}[1,]\n\u{1e}[2]\n", &["-c", "--seq", "--stream-errors"], ".", "\u{1e}[[0],1]\n\u{1e}[\"Expected another array element at line 1, column 5 (need RS to resync)\",[1]]\n\u{1e}[[0],2]\n\u{1e}[[0]]\n", "", 0),
    ("\u{1e}[1,2\u{1e}[3]\n", &["-c", "--seq", "--stream-errors"], ".", "\u{1e}[[0],1]\n\u{1e}[\"Truncated value at line 1, column 6\",[1]]\n\u{1e}[[0],3]\n\u{1e}[[0]]\n", "", 0),
    ("\u{1e}[1,\n\u{1e}{\"a\":2}\n", &["-c", "--seq", "--stream-errors", "-s"], "length", "\u{1e}4\n", "", 0),
    ("\u{1e}[1,]\n\u{1e}[2]\n", &["-c", "--seq", "--stream-errors", "-s"], ".", "\u{1e}[[[0],1],[\"Expected another array element at line 1, column 5 (need RS to resync)\",[1]],[[0],2],[[0]]]\n", "", 0),
    ("\u{1e}[1]\n\u{1e}[2", &["-c", "--seq", "--stream-errors"], ".", "\u{1e}[[0],1]\n\u{1e}[[0]]\n", "", 0),
    ("\u{1e}[1]\n\u{1e}[2\n", &["-c", "--seq", "--stream-errors"], ".", "\u{1e}[[0],1]\n\u{1e}[[0]]\n\u{1e}[\"Unfinished JSON term at EOF at line 3, column 0\",[0]]\n", "", 0),
    ("\u{1e}[1,]\n\u{1e}[2]\n", &["-c", "--seq", "--stream", "-n"], "[inputs]", "", "jq: error (at <stdin>:1): Expected another array element at line 1, column 5 (need RS to resync)\n", 5),
    ("\u{1e}[1,]\n\u{1e}[2]\n", &["-c", "--seq", "--stream", "-n"], "try input catch .", "\u{1e}[[0],1]\n", "", 0),
    ("\u{1e}[1,]\n\u{1e}[2]\n", &["-c", "--seq", "--stream"], "input", "\u{1e}[[0]]\n", "jq: error (at <stdin>:1): Expected another array element at line 1, column 5 (need RS to resync)\n", 0),
    ("\u{1e}[1,2", &["-c", "--seq", "--stream"], "input", "", "jq: error (at <stdin>:0): Unfinished JSON term at EOF at line 1, column 5\n", 5),
    ("\u{1e}[1,2\u{1e}[3]\n", &["-c", "--seq", "--stream", "-n", "-s"], "try input catch .", "\u{1e}\"Truncated value at line 1, column 6\"\n", "", 0),
    ("\u{1e}\"u\u{1e}]tu}-:][1]", &["-c", "--seq", "--stream", "-s"], ".", "\u{1e}[[[0],1],[[0]]]\n", "jq: ignoring parse error: Invalid numeric literal at line 1, column 4\njq: ignoring parse error: Unmatched ']' at the top-level at line 1, column 5 (need RS to resync)\njq: ignoring parse error: Invalid literal at line 1, column 8 (need RS to resync)\njq: ignoring parse error: Invalid numeric literal at line 1, column 10 (need RS to resync)\njq: ignoring parse error: Unmatched ']' at the top-level at line 1, column 11 (need RS to resync)\n", 0),
    ("\u{1e}[1]\n\u{1e}[2", &["-c", "--seq", "--stream", "-s"], "input_line_number", "\u{1e}0\n", "jq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 3\n", 0),
    ("\u{1e}[1]\n\u{1e}[2", &["-c", "--seq", "--stream", "-s"], ".", "\u{1e}[[[0],1],[[0]]]\n", "jq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 3\n", 0),
    ("\u{1e}[1]\n\u{1e}[2]\n", &["-c", "--seq", "--stream", "-s"], "input_line_number", "\u{1e}2\n", "", 0),
    ("\u{1e}1.5x}[1]", &["-c", "--seq", "--stream", "-s"], "length", "\u{1e}2\n", "jq: ignoring parse error: Invalid numeric literal at line 1, column 6 (need RS to resync)\n", 0),
];

#[test]
fn the_filter_reads_the_events_of_each_record_6115() -> Result<()> {
    // Every row runs, and the ones that differ are reported together.
    let mut mismatches = Vec::new();
    for (input, flags, program, want_out, want_err, want_code) in ROWS {
        let mut args: Vec<&str> = flags.to_vec();
        args.push(program);
        let (stdout, stderr, code) = run_jq_full(&args, Some(input))?;
        if (stdout.as_str(), stderr.as_str(), code) != (*want_out, *want_err, *want_code) {
            mismatches.push(format!(
                "{input:?} {flags:?} {program}: got ({stdout:?}, {stderr:?}, {code}), want ({want_out:?}, {want_err:?}, {want_code})"
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    Ok(())
}

const BOM_ROWS: &[(&[u8], &[&str], &str, &str)] = &[
    (b"\xef\xbb\xbf\x1e[1]\n", &["-c", "--seq", "--stream", "."], "\u{1e}[[0],1]\n\u{1e}[[0]]\n", ""),
    (b"\xef\xbb\x1e[1]\n", &["-c", "--seq", "--stream", "."], "\u{1e}[[0],1]\n", ""),
    (b"\xef\x1e[1]\n\x1e[2]\n", &["-c", "--seq", "--stream", "."], "\u{1e}[[0],1]\n\u{1e}[[0],2]\n", ""),
    (b"\xef\xbb1 2", &["-c", "--seq", "--stream", "."], "\u{1e}[[],1]\n\u{1e}[[],2]\n", ""),
    (b"\xef\xbb\x1e[1]\n\x1e[2]\n", &["-c", "--seq", "--stream-errors", "-s", "."], "\u{1e}[[[0],1],[[0],2]]\n", ""),
    (b"\xef\xbb1.5r[]]:\xef\xbbtrx][][ \n", &["-c", "--seq", "--stream", "-s", "length"], "\u{1e}1\n", "jq: ignoring parse error: Invalid numeric literal at line 1, column 5 (need RS to resync)\njq: ignoring parse error: Unmatched ']' at the top-level at line 1, column 6 (need RS to resync)\njq: ignoring parse error: Unmatched ']' at the top-level at line 1, column 7 (need RS to resync)\njq: ignoring parse error: ':' not as part of an object at line 1, column 8 (need RS to resync)\njq: ignoring parse error: Invalid numeric literal at line 1, column 14 (need RS to resync)\n"),
];

/// A byte-order mark is matched by `--seq`'s reader as jq matches it: one that breaks off (`\xef\xbb`
/// and then something else) is consumed, counts no column, and wipes the parser at the start of
/// every call, which costs a container the close event that was pending.
#[test]
fn a_broken_byte_order_mark_wipes_the_parser_at_every_call_6115() -> Result<()> {
    let mut mismatches = Vec::new();
    for (input, flags, want_out, want_err) in BOM_ROWS {
        let (stdout, stderr, code, _) = run_jq_over_byte_files(flags, &[input])?;
        if (stdout.as_str(), stderr.as_str(), code) != (*want_out, *want_err, 0) {
            mismatches.push(format!(
                "{input:?} {flags:?}: got ({stdout:?}, {stderr:?}, {code})"
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    Ok(())
}

/// A token that ends a file continues in the next one, as it does in jq's single byte stream.
#[test]
fn a_record_continues_across_files_6115() -> Result<()> {
    let (stdout, stderr, code, _) = run_jq_over_byte_files(
        &["-c", "--seq", "--stream", "."],
        &[b"\x1e[1", b",2]\n\x1e3\n"],
    )?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        (
            "\u{1e}[[0],1]\n\u{1e}[[1],2]\n\u{1e}[[1]]\n\u{1e}[[],3]\n",
            "",
            0
        )
    );
    Ok(())
}
