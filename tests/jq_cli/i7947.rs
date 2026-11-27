//! #7947: after a caught fault, an overlong, surrogate or out-of-range UTF-8 sequence in a later
//! string decoded as one U+FFFD per byte, where jq gives one for the whole sequence.
//!
//! The text was substituted once, whole, deciding in or out of a string by its quotes; past a fault
//! jq drops the rest of the read, so the quote of `a"` that it never reads flipped the parity and a
//! real string body was substituted by the rule for bytes outside strings. A value read past a
//! fault is now substituted again from the bytes jq read, from its own first byte.
//!
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`) and jq 1.8.2 (`/opt/homebrew/bin/jq`), which
//! agree on each.

use anyhow::Result;

use crate::run_jq_over_byte_files;

const PROGRAM: &str = "[limit(3; repeat(try input catch .))]";
const EXPECTED: &str = "[1,\"Invalid numeric literal at line 1, column 4\",\"\u{fffd}\"]\n";

/// `1 a"`, then (when `pad`) spaces to byte 4,095, and a string holding `sequence`.
fn input(sequence: &[u8], pad: bool) -> Vec<u8> {
    let mut text = b"1 a\"".to_vec();
    if pad {
        text.resize(4095, b' ');
    } else {
        text.push(b'\n');
    }
    text.push(b'"');
    text.extend_from_slice(sequence);
    text.extend_from_slice(b"\"\n");
    text
}

#[test]
fn a_string_past_a_fault_is_substituted_as_jq_reads_it_7947() -> Result<()> {
    for sequence in [
        &b"\xe0\x80\x80"[..],
        b"\xed\xa0\x80",
        b"\xf4\x90\x80\x80",
        b"\xe0\x80",
        b"\xf0\x80\x80",
        b"\xe1A",
    ] {
        for pad in [true, false] {
            let (out, err, code, _) =
                run_jq_over_byte_files(&["-nc", PROGRAM], &[&input(sequence, pad)])?;
            assert_eq!(
                (out.as_str(), err.as_str(), code),
                (EXPECTED, "", 0),
                "{sequence:x?} pad {pad}"
            );
        }
    }
    Ok(())
}
