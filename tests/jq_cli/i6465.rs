//! #6465: a document past jq's depth limit with an invalid UTF-8 byte ahead of the refused bracket
//! (in a key, or in an earlier value on the same line) was refused at a column counted over the
//! substituted text, where each invalid byte is U+FFFD's three bytes. jq's parser counts the bytes
//! it read. A wrong refusal message then also missed #6454's parser end, so the fault after it was
//! placed past the bytes jq drops. Every expected string was captured from jq 1.7.1
//! (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_over_byte_files;

const FILTER: &str = "[limit(4; repeat(try input catch .))]";

/// `prefix`, 300 `[`, then a newline and `"k": 1}`: the `}` lets the splitter close the document,
/// so the depth refusal is the document walk's.
fn deep_after(prefix: &[u8]) -> Vec<u8> {
    let mut text = prefix.to_vec();
    text.extend_from_slice(&[b'['; 300]);
    text.extend_from_slice(b"\n\"k\": 1}\n");
    text
}

#[test]
fn a_depth_refusal_after_an_invalid_byte_is_placed_over_the_bytes_read_6465() -> Result<()> {
    for (prefix, main_err, inputs) in [
        (&b"{\"\xff\":"[..], "column 260", "[\"Exceeds depth limit for parsing at line 1, column 260\",\"k\",\"Expected string key before ':' at line 1, column 264\",\"break\"]\n"),
        (b"{\"\xff\xff\":", "column 261", "[\"Exceeds depth limit for parsing at line 1, column 261\",\"k\",\"Expected string key before ':' at line 1, column 265\",\"break\"]\n"),
        (b"{\"a\xffb\":", "column 262", "[\"Exceeds depth limit for parsing at line 1, column 262\",\"k\",\"Expected string key before ':' at line 1, column 266\",\"break\"]\n"),
        // The invalid byte is in an earlier document on the refused one's line.
        (b"\"\xff\" {", "column 261", "[\"\u{fffd}\",\"Exceeds depth limit for parsing at line 1, column 261\",\"k\",\"Expected string key before ':' at line 1, column 265\"]\n"),
    ] {
        let text = deep_after(prefix);
        let (_, stderr, code, _) = run_jq_over_byte_files(&["-c", "."], &[&text])?;
        assert_eq!(
            (stderr.lines().next(), code),
            (
                Some(format!("jq: parse error: Exceeds depth limit for parsing at line 1, {main_err}").as_str()),
                5
            ),
            "{prefix:?}"
        );
        let (stdout, _, code, _) = run_jq_over_byte_files(&["-nc", FILTER], &[&text])?;
        assert_eq!((stdout.as_str(), code), (inputs, 0), "{prefix:?}");
    }
    Ok(())
}

/// A valid multi-byte key is counted as it was before: the substitution leaves the text alone.
#[test]
fn a_valid_key_keeps_its_column_6465() -> Result<()> {
    let text = deep_after("{\"é\":".as_bytes());
    let (stdout, _, code, _) = run_jq_over_byte_files(&["-nc", FILTER], &[&text])?;
    assert_eq!(
        (stdout.as_str(), code),
        (
            "[\"Exceeds depth limit for parsing at line 1, column 261\",\"k\",\"Expected string key before ':' at line 1, column 265\",\"break\"]\n",
            0
        )
    );
    Ok(())
}
