//! #7931: where jq's 4,095-byte reads end on a line that holds invalid UTF-8, behind a byte-order
//! mark.
//!
//! jq counts its reads in the bytes it read. `succinctly jq` counted them over the text after each
//! invalid byte had become a three-byte U+FFFD, two bytes more per byte replaced, which the mark's
//! three bytes hid until #7569 counted them in the first read: since then a read boundary after an
//! invalid byte fell early. So a fault met in the read that holds the line break was placed in the
//! read before it, one line early (`(at f:0)`), and the read after a fault started inside the
//! replacement characters, where the fault was met a second time.
//!
//! Every expected row was captured from jq 1.7.1 (`/usr/bin/jq`). jq 1.8.2 (`/opt/homebrew/bin/jq`)
//! gives the same answer except where a row says "1.7.1 only": its reads are 4,091 bytes long.

use anyhow::Result;

use crate::run_jq_over_byte_files;

const MARK: &[u8] = b"\xef\xbb\xbf";

fn run(args: &[&str], text: &[u8]) -> Result<(String, String, i32)> {
    let (out, err, code, paths) = run_jq_over_byte_files(args, &[text])?;
    let err = err.replace("succinctly jq:", "jq:").replace(&paths[0], "f");
    Ok((out, err, code))
}

/// The issue's first case: a mark, 8,182 `x`s, `0xC3`, a space, `1` and a newline (8,189 bytes).
/// The fault is met in the second read, which holds the newline.
fn lead_byte_at_the_read_end() -> Vec<u8> {
    [MARK, &[b'x'; 8182], b"\xc3 1\n"].concat()
}

/// The issue's second case: a mark and `"a"` 681 times, a mark, a surrogate's three bytes and
/// `"s"` and a newline (4,096 bytes). The fault after `"a"` drops the rest of the first read,
/// `"s"` included; the second read is the newline.
fn surrogate_before_the_read_end() -> Vec<u8> {
    let mut text = [MARK, b"\"a\""].concat().repeat(681);
    text.extend_from_slice(MARK);
    text.extend_from_slice(b"\xed\xa0\x80\"s\"\n");
    text
}

#[test]
fn a_fault_in_the_read_that_holds_the_line_break_names_line_1_7931() -> Result<()> {
    let text = lead_byte_at_the_read_end();
    assert_eq!(
        run(&["-nc", "[inputs]"], &text)?,
        (
            String::new(),
            "jq: error (at f:1): Invalid numeric literal at line 1, column 8184\n".to_string(),
            5
        ),
    );
    Ok(())
}

#[test]
fn a_fault_jq_drops_with_its_read_is_not_met_again_7931() -> Result<()> {
    let text = surrogate_before_the_read_end();
    // 1.7.1 only: jq 1.8.2 reads `"s"` as well (`["s","E"]`).
    assert_eq!(
        run(&["-c", "[., (try input catch \"E\")]"], &text)?,
        ("[\"a\",\"E\"]\n".to_string(), String::new(), 0),
    );
    assert_eq!(
        run(&["-c", "first(inputs)?"], &text)?,
        (String::new(), String::new(), 0),
    );
    // 1.7.1 only: jq 1.8.2 reads `"s"` after the fault.
    assert_eq!(
        run(&["-c", "-n", "[limit(8; repeat(try input catch .))]"], &text)?,
        (
            "[\"a\",\"Invalid numeric literal at line 1, column 7\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n".to_string(),
            String::new(),
            0
        ),
    );
    Ok(())
}

/// #7921's recorded residual, without a mark: `0`, a newline, spaces, `}`, the byte 0x80, a quote,
/// a newline and then values on lines of their own. Both jq releases give the `}` fault and the
/// values; the read after the fault started inside the replacement character.
#[test]
fn the_read_after_a_fault_past_an_invalid_byte_starts_where_jqs_does_7931() -> Result<()> {
    let padded = |pad: usize, tail: &[u8]| [b"0\n".as_slice(), &vec![b' '; pad], tail].concat();
    let reads = ["-c", "-n", "[limit(6; repeat(try input catch .))]"];
    assert_eq!(
        run(&reads, &padded(4092, b"}\x80\"\n8\n9\n"))?,
        (
            "[0,\"Unmatched '}' at line 2, column 4093\",8,9,\"break\",\"break\"]\n".to_string(),
            String::new(),
            0
        ),
    );
    assert_eq!(
        run(&reads, &padded(4091, b"}\x80\"\n5\n"))?,
        (
            "[0,\"Unmatched '}' at line 2, column 4092\",5,\"break\",\"break\",\"break\"]\n"
                .to_string(),
            String::new(),
            0
        ),
    );
    Ok(())
}
