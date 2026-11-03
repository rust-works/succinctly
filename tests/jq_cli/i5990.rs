//! #5990: jq's lexer refuses a raw control character in a string before it substitutes invalid
//! UTF-8, so the drop quirk (a lead byte taking the rest of a short string with it, #1743) never
//! hides one: `"\xe1\n"` is `Invalid string: control characters ...` at the newline, not a lone
//! U+FFFD. Every expected string was captured from jq 1.7.1 (`/usr/bin/jq`).

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

/// `try input catch .` over one file: the fault each string with a raw control character raises.
#[test]
fn a_control_character_in_a_string_is_refused_before_the_substitution_5990() -> Result<()> {
    let rows: &[(&[&[u8]], &str)] = &[
        (&[b"\"\xe1\n\" 1 }"], "[\"Invalid string: control characters from U+0000 through U+001F must be escaped at line 2, column 1\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\"a\xe1\x01b\" 1"], "[\"Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"{\"k\xe1\t\":1}"], "[\"Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\"\xf0\x90\nA\" 2"], "[\"Invalid string: control characters from U+0000 through U+001F must be escaped at line 2, column 2\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"1 \"\xe1\x1f\" 2"], "[1,\"Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\"\xe1A\" 1 \"\xe1\n\""], "[\"�\",1,\"Invalid string: control characters from U+0000 through U+001F must be escaped at line 2, column 1\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (&[b"\"ok\n\xe1\""], "[\"Invalid string: control characters from U+0000 through U+001F must be escaped at line 2, column 2\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
    ];
    for (files, want) in rows {
        let (stdout, _, code, _) = run_jq_over_byte_files(&["-nc", FILTER], files)?;
        assert_eq!((stdout.as_str(), code), (*want, 0), "{files:?}");
    }
    Ok(())
}

/// The same refusal on the main loop's route, `--stream` and `-s`.
#[test]
fn the_other_routes_refuse_it_too_5990() -> Result<()> {
    let rows: &[RouteRow] = &[
        (&[b"\"\xe1\n\" 1 }"], &["-c", "."], "", "jq: parse error: Invalid string: control characters from U+0000 through U+001F must be escaped at line 2, column 1\n", 5),
        (&[b"[1,\"a\xe1\x01b\",2]"], &["-c", "--stream", "."], "[[0],1]\n", "jq: parse error: Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 9\n", 5),
        (&[b"\"\xe1\n\" 1"], &["-cs", "."], "", "jq: parse error: Invalid string: control characters from U+0000 through U+001F must be escaped at line 2, column 1\n", 5),
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
