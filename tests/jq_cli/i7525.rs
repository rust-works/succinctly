//! #7525: a bare `n` token is `Invalid numeric literal` whatever an earlier token left in jq's
//! token buffer, on every route that reads a stream of JSON texts. jq 1.7.1's `check_literal` read
//! `tokenbuf[1]` without checking `tokenpos`, so a bare `n` after a token that left `u` there
//! (`"iu"`, `"nu"`, `null`) was `Invalid literal`; jq 1.8.2 tests `tokenpos > 1` first
//! (jqlang/jq#3324, a fix for an uninitialised read). `--seq` and the `--stream` and
//! `--stream-errors` parsers modelled the quirk; the plain, `-n inputs` and `-s` routes never
//! did, so one set of bytes got a different word depending on the route. Every row is the
//! (stdin, arguments, stdout, stderr, exit status) jq 1.8.2 produced, captured from
//! `/opt/homebrew/bin/jq`; a token that really is `nu...` (the `nu` rows) is `Invalid literal`
//! in both versions and stays so.

use anyhow::Result;

use crate::run_jq_full;

type Row = (
    &'static [u8],
    &'static [&'static str],
    &'static [u8],
    &'static [u8],
    i32,
);

const ROWS: &[Row] = &[
        (
            b"\x1e\x22iu\x22\x0a\x1en\x0a",
            &["--seq", "-c", "."],
            b"\x1e\x22iu\x22\x0a",
            b"jq: ignoring parse error: Invalid numeric literal at line 3, column 0 (need RS to resync)\x0a",
            0,
        ),
        (
            b"\x1e\x22nu\x22\x0a\x1en\x0a",
            &["--seq", "-c", "."],
            b"\x1e\x22nu\x22\x0a",
            b"jq: ignoring parse error: Invalid numeric literal at line 3, column 0 (need RS to resync)\x0a",
            0,
        ),
        (
            b"\x1enull\x0a\x1en\x0a",
            &["--seq", "-c", "."],
            b"\x1enull\x0a",
            b"jq: ignoring parse error: Invalid numeric literal at line 3, column 0 (need RS to resync)\x0a",
            0,
        ),
        (
            b"\x1en",
            &["--seq", "-c", "."],
            b"",
            b"jq: ignoring parse error: Invalid numeric literal at EOF at line 1, column 2\x0a",
            0,
        ),
        (
            b"\x1enu\x0a",
            &["--seq", "-c", "."],
            b"",
            b"jq: ignoring parse error: Invalid literal at line 2, column 0 (need RS to resync)\x0a",
            0,
        ),
        (
            b"null [n ",
            &["-c", "."],
            b"null\x0a",
            b"jq: parse error: Invalid numeric literal at line 1, column 8\x0a",
            5,
        ),
        (
            b"null [n ",
            &["-c", "--stream", "."],
            b"[[],null]\x0a",
            b"jq: parse error: Invalid numeric literal at line 1, column 8\x0a",
            5,
        ),
        (
            b"null [n ",
            &["-c", "--stream-errors", "."],
            b"[[],null]\x0a[\x22Invalid numeric literal at line 1, column 8\x22,[0]]\x0a",
            b"",
            0,
        ),
        (
            b"null [n ",
            &["-c", "-n", "[inputs]"],
            b"",
            b"jq: error (at <stdin>:0): Invalid numeric literal at line 1, column 8\x0a",
            5,
        ),
        (
            b"null [n ",
            &["-c", "-s", "."],
            b"",
            b"jq: parse error: Invalid numeric literal at line 1, column 8\x0a",
            5,
        ),
        (
            b"\x22iu\x22 n",
            &["-c", "."],
            b"\x22iu\x22\x0a",
            b"jq: parse error: Invalid numeric literal at EOF at line 1, column 6\x0a",
            5,
        ),
        (
            b"\x22iu\x22 n",
            &["-c", "--stream", "."],
            b"[[],\x22iu\x22]\x0a",
            b"jq: parse error: Invalid numeric literal at EOF at line 1, column 6\x0a",
            5,
        ),
        (
            b"\x22iu\x22 n",
            &["-c", "--stream-errors", "."],
            b"[[],\x22iu\x22]\x0a[\x22Invalid numeric literal at EOF at line 1, column 6\x22,[]]\x0a",
            b"",
            0,
        ),
        (
            b"nul n",
            &["-c", "."],
            b"",
            b"jq: parse error: Invalid literal at line 1, column 4\x0a",
            5,
        ),
        (
            b"n",
            &["-c", "--stream", "."],
            b"",
            b"jq: parse error: Invalid numeric literal at EOF at line 1, column 1\x0a",
            5,
        ),
        (
            b"nu",
            &["-c", "--stream", "."],
            b"",
            b"jq: parse error: Invalid literal at EOF at line 1, column 2\x0a",
            5,
        ),
];

#[test]
fn test_bare_n_is_a_numeric_literal_fault_on_every_route_7525() -> Result<()> {
    for (input, args, stdout, stderr, code) in ROWS {
        let input = std::str::from_utf8(input)?;
        let (got_out, got_err, got_code) = run_jq_full(args, Some(input))?;
        let context = format!("{input:?} {args:?}");
        assert_eq!(got_out.as_bytes(), *stdout, "stdout: {context}");
        assert_eq!(got_err.as_bytes(), *stderr, "stderr: {context}");
        assert_eq!(got_code, *code, "exit status: {context}");
    }
    Ok(())
}
