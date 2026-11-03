//! #5975: a parse error's column is counted over the bytes jq read, also where the substitution
//! of invalid UTF-8 collapsed or dropped bytes inside a string (`"\xe1A"` is one U+FFFD for two
//! bytes, since the lead byte takes the `A` with it), and the fault of a read that starts inside a
//! character is the parser's own (`Invalid numeric literal at line 1, column 11`), not
//! `Invalid JSON text`. Every expected string was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_over_byte_files;

const FILTER: &str = "[limit(8; repeat(try input catch .))]";

fn check(rows: &[(&[&[u8]], &str)]) -> Result<()> {
    for (files, want) in rows {
        let (stdout, _, code, _) = run_jq_over_byte_files(&["-nc", FILTER], files)?;
        assert_eq!((stdout.as_str(), code), (*want, 0), "{files:?}");
    }
    Ok(())
}

/// A run jq collapses or drops inside a string is one U+FFFD in the text for more than one byte.
#[test]
fn a_fault_after_a_run_jq_collapses_inside_a_string_is_counted_over_the_bytes_5975() -> Result<()> {
    check(&[
        (&[b"\"\xe1A\" }"], "[\"�\",\"Unmatched '}' at line 1, column 6\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\"x\xe1A\" }"], "[\"x�\",\"Unmatched '}' at line 1, column 7\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\"\xe1A\" \"\xf0\x90B\" }"], "[\"�\",\"�\",\"Unmatched '}' at line 1, column 12\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"{\"k\xe1A\":1} }"], "[{\"k�\":1},\"Unmatched '}' at line 1, column 11\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\"\xf0\x90A\" }"], "[\"�\",\"Unmatched '}' at line 1, column 7\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        // A lone invalid byte outside a string, beside one inside.
        (&[b"\xff \"\xe1A\" }"], "[\"Invalid numeric literal at line 1, column 2\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\"\xe1A\"\n\"\xe1A\" }"], "[\"�\",\"�\",\"Unmatched '}' at line 2, column 6\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
    ])
}

/// A string whose escapes had to be decoded to judge the lead byte is rewritten whole.
#[test]
fn a_fault_after_a_string_rewritten_whole_is_counted_over_the_bytes_5975() -> Result<()> {
    check(&[
        (&[b"\"\xe1\\u0041\" }"], "[\"�\",\"Unmatched '}' at line 1, column 11\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\"\xe1\\n\" 1 }"], "[\"�\",1,\"Unmatched '}' at line 1, column 9\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
    ])
}

/// A character cut by the end of a file is completed by the next file, and a fault met before it
/// resumes in the middle of that character.
#[test]
fn a_read_that_starts_inside_a_character_raises_the_parsers_own_fault_5975() -> Result<()> {
    check(&[
        (&[b"[\t23null\t\"\xe2\x82", b"\xac\"\n"], "[\"Invalid numeric literal at line 1, column 9\",\"Invalid numeric literal at line 1, column 11\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"1 }\n\"\xe2", b"\x82\xac\" }"], "[1,\"Unmatched '}' at line 1, column 3\",\"€\",\"Unmatched '}' at line 1, column 10\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"[1 2]\"\xf0\x9f", b"\x98\x80\" }"], "[\"Expected separator between values at line 1, column 5\",\"Invalid numeric literal at line 1, column 8\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
    ])?;
    // The same read boundary in one file: jq reads 4095 bytes at a time, so `}` padded to end the
    // first read inside `é` starts the second on a continuation byte.
    let mut padded = vec![b'}'; 4094];
    padded.extend_from_slice("é 7\n8\n".as_bytes());
    check(&[(
        &[padded.as_slice()],
        "[\"Unmatched '}' at line 1, column 1\",\"Invalid numeric literal at line 1, column 3\",8,\"break\",\"break\",\"break\",\"break\",\"break\"]\n",
    )])?;
    // With an invalid byte later in the file, the text is the original up to the read boundary, so
    // the read is taken from the text's substitution of the file and the fault is still jq's.
    let mut padded = vec![b'}'; 4094];
    padded.extend_from_slice("é".as_bytes());
    padded.extend_from_slice(b" 7\n\xff 8\n9\n");
    check(&[(
        &[padded.as_slice()],
        "[\"Unmatched '}' at line 1, column 1\",\"Invalid numeric literal at line 1, column 3\",\"Invalid numeric literal at line 1, column 5\",9,\"break\",\"break\",\"break\",\"break\"]\n",
    )])
}
