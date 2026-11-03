//! #5994: jq substitutes nothing outside a string: an invalid byte there is part of a literal
//! token, and the white space after it is still white space. The document substitution used the
//! string-scoped short-tail rule on the bytes between strings, so a lead byte took the newline after
//! it (`\xe1\n"a" 1`), and the read that resumed after the fault lost `"a"` and `1`. Every expected
//! string was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_over_byte_files;

const FILTER: &str = "[limit(8; repeat(try input catch .))]";

/// `(files, flags, stdout, stderr, exit code)`.
type RouteRow = (
    &'static [&'static [u8]],
    &'static [&'static str],
    &'static str,
    &'static str,
    i32,
);

/// `try input catch .` over one file: the fault, then the values after it.
#[test]
fn the_values_after_a_bad_byte_outside_a_string_are_read_5994() -> Result<()> {
    let rows: &[(&[&[u8]], &str)] = &[
        (&[b"\xe1\n\"a\" 1"], "[\"Invalid numeric literal at line 2, column 0\",\"a\",1,\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"1 \xe1\n\"a\" 1"], "[1,\"Invalid numeric literal at line 2, column 0\",\"a\",1,\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\xf0\x90\n\"a\" 1"], "[\"Invalid numeric literal at line 2, column 0\",\"a\",1,\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\xe1\t\"a\" 1"], "[\"Invalid numeric literal at line 1, column 2\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"[\xe1\n,\"a\"] 1"], "[\"Invalid numeric literal at line 2, column 0\",\"Expected value before ',' at line 2, column 1\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\xe1 \"a\" 1"], "[\"Invalid numeric literal at line 1, column 2\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\xe1\n\n\"a\" 1"], "[\"Invalid numeric literal at line 2, column 0\",\"a\",1,\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
    ];
    for (files, want) in rows {
        let (stdout, _, code, _) = run_jq_over_byte_files(&["-nc", FILTER], files)?;
        assert_eq!((stdout.as_str(), code), (*want, 0), "{files:?}");
    }
    Ok(())
}

/// The main loop's route, `--seq` and `--stream` over the same bytes.
#[test]
fn the_other_routes_read_them_too_5994() -> Result<()> {
    let rows: &[RouteRow] = &[
        (
            &[b"\xe1\n\"a\" 1"],
            &["-c", "."],
            "",
            "jq: parse error: Invalid numeric literal at line 2, column 0\n",
            5,
        ),
        (
            &[b"\xe1\n\"a\" 1"],
            &["-c", "--seq", "."],
            "",
            "jq: ignoring parse error: Unfinished abandoned text at EOF at line 2, column 5\n",
            0,
        ),
        (
            &[b"1 \xe1\n\"a\" 1"],
            &["-c", "--stream", "."],
            "[[],1]\n",
            "jq: parse error: Invalid numeric literal at line 2, column 0\n",
            5,
        ),
    ];
    for (files, flags, want_out, want_err, want_code) in rows {
        let (stdout, stderr, code, _) = run_jq_over_byte_files(flags, files)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (*want_out, *want_err, *want_code),
            "{files:?} {flags:?}"
        );
    }
    Ok(())
}
