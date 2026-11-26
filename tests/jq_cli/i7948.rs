//! #7948, shape 2: `input_line_number` one short (`0` where jq says `1`) past invalid UTF-8 after a
//! fault, on a file whose lazy prefix the route keeps.
//!
//! That hand-off did not count its lines in the bytes jq read (#7934's `mark_lazy_read_as`), so the
//! reads were counted over the substituted text, three bytes for each replaced byte, and the read
//! that reaches the line break ended before it.
//!
//! Shape 1, a NUL-ended number in a first read with no line break, is not fixed here: a NUL outside
//! a string is to become a parse error, which takes that shape with it.
//!
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`) and jq 1.8.2 (`/opt/homebrew/bin/jq`), which
//! agree on each.

use anyhow::Result;

use crate::run_jq_over_byte_files;

/// The issue's minimised second shape: a 4,090-byte line with invalid UTF-8 before and after the
/// number `1`, and a bare `0xC3` at byte 4,086 that is jq's fault.
fn lazy_prefix_past_invalid_utf8() -> Vec<u8> {
    let mut text = vec![b' '; 4089];
    for (at, bytes) in [
        (1191, &b"\"\x80\""[..]),
        (2986, b"\"\xff\""),
        (3518, b"1"),
        (3533, b"\"\xe1A\""),
        (4086, b"\xc3"),
    ] {
        text[at..at + bytes.len()].copy_from_slice(bytes);
    }
    text.push(b'\n');
    text
}

#[test]
fn a_lazy_prefix_counts_its_line_in_the_bytes_jq_read_7948() -> Result<()> {
    let (out, err, code, paths) = run_jq_over_byte_files(
        &["select(type==\"number\") | input_line_number"],
        &[&lazy_prefix_past_invalid_utf8()],
    )?;
    let err = err.replace("succinctly jq:", "jq:").replace(&paths[0], "f");
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        (
            "1\n",
            "jq: parse error: Invalid numeric literal at line 1, column 4088\n",
            5
        )
    );
    Ok(())
}
