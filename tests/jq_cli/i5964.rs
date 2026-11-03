//! #5964: jq's parser advances its column once per byte it reads, and the substitution of an invalid
//! UTF-8 byte (a three-byte U+FFFD) happens after that, so a parse error names the column over the
//! bytes as they were. `\xff }` reports column 2, not 4. A U+FFFD that was in the input is three
//! bytes to jq as well, so the columns cannot be fixed by subtracting two for each one in the
//! text. Every expected string was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_over_byte_files;

const FILTER: &str = "[limit(8; repeat(try input catch .))]";

/// `(files, flags, stdout, stderr)`: a row that exits 5.
type Row = (
    &'static [&'static [u8]],
    &'static [&'static str],
    &'static str,
    &'static str,
);

/// `try input catch .`, over the files in order: the column of each fault the program reads.
#[test]
fn a_fault_after_an_invalid_byte_is_counted_over_the_bytes_read_5964() -> Result<()> {
    let rows: &[(&[&[u8]], &str)] = &[
        (&[b"\xff }"], "[\"Invalid numeric literal at line 1, column 2\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        // A truncated sequence in a string is one U+FFFD for two bytes.
        (&[b"\"a\xe2\x82\"} 23null\t"], "[\"a�\",\"Unmatched '}' at line 1, column 6\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\xff\xff\xff }"], "[\"Invalid numeric literal at line 1, column 4\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        // A valid multi-byte character was never affected.
        (&[b"\xc3\xa9 }"], "[\"Invalid numeric literal at line 1, column 3\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        // A U+FFFD in the input is three bytes, with or without an invalid byte beside it.
        (&[b"1 \xef\xbf\xbd }"], "[1,\"Invalid numeric literal at line 1, column 6\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"1 \xef\xbf\xbd\xff }"], "[1,\"Invalid numeric literal at line 1, column 7\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        // The next fault goes on from where the first left the parser.
        (&[b"}\n\xff }\n\xff }"], "[\"Unmatched '}' at line 1, column 1\",\"Invalid numeric literal at line 1, column 3\",\"Invalid numeric literal at line 1, column 5\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\xff }", b"\xff }"], "[\"Invalid numeric literal at line 1, column 2\",\"Invalid numeric literal at line 1, column 4\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"[1,\xff,2]"], "[\"Invalid numeric literal at line 1, column 5\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\"\xff\" }"], "[\"�\",\"Unmatched '}' at line 1, column 5\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\x80 }"], "[\"Invalid numeric literal at line 1, column 2\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\xe2\x82 }"], "[\"Invalid numeric literal at line 1, column 3\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\xff }", b"\xef\xbf\xbd }", b"\xff\xff }"], "[\"Invalid numeric literal at line 1, column 2\",\"Invalid numeric literal at line 1, column 6\",\"Invalid numeric literal at line 1, column 9\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
    ];
    for (files, want) in rows {
        let (stdout, _, code, _) = run_jq_over_byte_files(&["-nc", FILTER], files)?;
        assert_eq!((stdout.as_str(), code), (*want, 0), "{files:?}");
    }
    Ok(())
}

/// The same count in the fault the main loop meets, a slurped stream, and `--stream`, which read
/// the stream on other routes.
#[test]
fn the_other_routes_count_the_bytes_read_too_5964() -> Result<()> {
    // `(files, flags, stdout, stderr)`; every one exits 5.
    let rows: &[Row] = &[
        (
            &[b"\xff }"],
            &["-c", "."],
            "",
            "jq: parse error: Invalid numeric literal at line 1, column 2\n",
        ),
        (
            &[b"1\n\xff }"],
            &["-c", "."],
            "1\n",
            "jq: parse error: Invalid numeric literal at line 2, column 2\n",
        ),
        (
            &[b"\xff\n\xff }"],
            &["-c", "."],
            "",
            "jq: parse error: Invalid numeric literal at line 2, column 0\n",
        ),
        (
            &[b"\xff }", b"\xff }"],
            &["-c", "."],
            "",
            "jq: parse error: Invalid numeric literal at line 1, column 2\n",
        ),
        (
            &[b"1 \xef\xbf\xbd\xff }"],
            &["-c", "."],
            "1\n",
            "jq: parse error: Invalid numeric literal at line 1, column 7\n",
        ),
        (
            &[b"\xff }"],
            &["-c", "-s", "."],
            "",
            "jq: parse error: Invalid numeric literal at line 1, column 2\n",
        ),
        (
            &[b"\xff }", b"\xff }"],
            &["-c", "-s", "."],
            "",
            "jq: parse error: Invalid numeric literal at line 1, column 2\n",
        ),
        (
            &[b"1 \xef\xbf\xbd\xff }"],
            &["-c", "-s", "."],
            "",
            "jq: parse error: Invalid numeric literal at line 1, column 7\n",
        ),
        (
            &[b"\xff }"],
            &["-c", "--stream", "."],
            "",
            "jq: parse error: Invalid numeric literal at line 1, column 2\n",
        ),
        (
            &[b"1\n\xff }"],
            &["-c", "--stream", "."],
            "[[],1]\n",
            "jq: parse error: Invalid numeric literal at line 2, column 2\n",
        ),
    ];
    for (files, flags, want_out, want_err) in rows {
        let (stdout, stderr, code, _) = run_jq_over_byte_files(flags, files)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (*want_out, *want_err, 5),
            "{files:?} {flags:?}"
        );
    }
    Ok(())
}
