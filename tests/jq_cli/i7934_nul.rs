//! #7934: since jq's reads past invalid UTF-8 are counted in the bytes jq read, a NUL that starts
//! the read after a fault's dropped read is met as it is on ASCII input. Re-pinned by #7966: that
//! NUL is the second fault, `Invalid character` at its own column (ADR-0018's E2, worded as jq
//! master, commit fbee8f4, words it), where it was `Invalid numeric literal at EOF` before.
//!
//! The shape: `[1,2]` and a newline, spaces, `xa`, an invalid sequence, `]` or `}`, and a NUL that
//! is the file's last byte, at line offset 8,190. jq meets the fault at `x` in its second
//! 4,095-byte read, drops the rest of that read, and resumes at the third, which is the NUL alone
//! with no newline: jq 1.7.1's `strlen` drops it and the stream ends, exit 0, and jq 1.8.2 reads it
//! as an empty token there and says nothing either. `succinctly jq` reports such a NUL (`Invalid
//! character at line 2, column 8191`, exit 5), as it does for `[1]` and a NUL
//! (`docs/compliance/jq/limitations.md`, #7966). Before #7934 the read was counted over the text
//! after the invalid sequence became U+FFFD, two bytes early, so the run resumed inside the
//! replacement, never met the NUL, and matched jq by accident. Its ASCII twin (`z` for each invalid
//! byte, the same bytes and the same reads) gave the divergence then too.
//!
//! The rows pin `succinctly jq`'s side, captured from the binary. jq 1.7.1 and 1.8.2
//! (`/usr/bin/jq`, `/opt/homebrew/bin/jq`) give the same output without the NUL's line, exit 0 for
//! the two `try` programs and exit 5 with the one fault for `[inputs|type]`.

use anyhow::Result;

use crate::run_jq_over_byte_files;

const MARK: &[u8] = b"\xef\xbb\xbf";

/// The invalid sequences of one to four bytes the shape holds.
const INVALID: [&[u8]; 6] = [
    b"\x80",
    b"\xc3",
    b"\xe4\xb8",
    b"\xed\xa0\x80",
    b"\xf4\x90\x80\x80",
    b"\xff",
];

/// The three programs: the driver loop, one `try input`, and `input` read on past each fault.
const PROGRAMS: [&[&str]; 3] = [
    &["-c", "[inputs|type]"],
    &["-c", "[., (try input catch \"E\")]"],
    &["-nc", "[limit(8; repeat(try input catch .))]"],
];

/// The shape with `invalid` and `closer`, 8,197 bytes, or 8,200 behind a mark: the NUL at line
/// offset 8,190 either way.
fn shape(mark: bool, invalid: &[u8], closer: &[u8]) -> Vec<u8> {
    let head = [if mark { MARK } else { b"" }, b"[1,2]\n"].concat();
    let tail = [b"xa".as_slice(), invalid, closer, b"\x00"].concat();
    let mut text = head;
    text.resize(8197 + if mark { MARK.len() } else { 0 } - tail.len(), b' ');
    text.extend_from_slice(&tail);
    text
}

fn run(args: &[&str], text: &[u8]) -> Result<(String, String, i32)> {
    let (out, err, code, paths) = run_jq_over_byte_files(args, &[text])?;
    let err = err.replace("succinctly jq:", "jq:").replace(&paths[0], "f");
    Ok((out, err, code))
}

#[test]
fn a_nul_read_after_a_dropped_read_past_invalid_utf8_is_reported_as_on_ascii_7934() -> Result<()> {
    let text = shape(false, b"\xc3", b"]");
    assert_eq!(text.len(), 8197);
    assert_eq!(
        run(PROGRAMS[0], &text)?,
        (
            String::new(),
            "jq: error (at f:1): Invalid numeric literal at line 2, column 8190\njq: parse error: Invalid character at line 2, column 8191\n".to_string(),
            5
        ),
    );
    assert_eq!(
        run(PROGRAMS[1], &text)?,
        (
            "[[1,2],\"E\"]\n".to_string(),
            "jq: parse error: Invalid character at line 2, column 8191\n".to_string(),
            5
        ),
    );
    assert_eq!(
        run(PROGRAMS[2], &text)?,
        (
            "[[1,2],\"Invalid numeric literal at line 2, column 8190\",\"Invalid character at line 2, column 8191\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n".to_string(),
            String::new(),
            0
        ),
    );
    Ok(())
}

/// Every invalid sequence, closer and mark, under each program, answers as its ASCII twin.
#[test]
fn each_shape_answers_as_its_ascii_twin_7934() -> Result<()> {
    for mark in [false, true] {
        for invalid in INVALID {
            for closer in [b"]".as_slice(), b"}"] {
                let text = shape(mark, invalid, closer);
                let twin = shape(mark, &vec![b'z'; invalid.len()], closer);
                for program in PROGRAMS {
                    assert_eq!(
                        run(program, &text)?,
                        run(program, &twin)?,
                        "mark {mark}, invalid {invalid:x?}, closer {closer:x?}, {program:?}",
                    );
                }
            }
        }
    }
    Ok(())
}
