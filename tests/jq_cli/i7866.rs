//! #7866: under `--raw-input --slurp`, a 3- or 4-byte UTF-8 lead byte with fewer bytes than it
//! declares before the line break keeps the line break in `succinctly jq`. jq 1.7.1 (`/usr/bin/jq`)
//! and jq 1.8.2 (`/opt/homebrew/bin/jq`) take the line break into the U+FFFD, because each `fgets`
//! read is its own slice there, and so merge two lines. Matching would delete a byte of the input,
//! so the difference is kept (ADR-0018, recorded in `docs/compliance/jq/limitations.md`).
//!
//! Every jq column below was captured from both versions on macOS, as `explode` output, and the
//! two agreed on each row unless the row says otherwise.

use anyhow::Result;

use crate::{run_jq_binary_stdin, run_jq_over_byte_files};

fn slurp_raw(input: &[u8]) -> Result<(String, i32)> {
    let (stdout, code) = run_jq_binary_stdin("explode", input, &["-R", "-s", "-c"])?;
    Ok((String::from_utf8(stdout)?, code))
}

/// The kept difference: jq's answer is in the comment, succinctly's is asserted.
#[test]
fn raw_slurp_keeps_the_line_break_after_a_truncated_lead_byte_7866() -> Result<()> {
    for (input, want) in [
        // jq: [97,10,65533,98,10]
        (&b"a\n\xef\nb\n"[..], "[97,10,65533,10,98,10]"),
        // jq: [65533,98]
        (b"\xe1\nb", "[65533,10,98]"),
        // jq: [65533,10,10,98]
        (b"\xf0\n\n\nb", "[65533,10,10,10,98]"),
        // jq: [97,65533,98]
        (b"a\xf0\x9f\nb", "[97,65533,10,98]"),
        // jq: [97,65533,98] (the CR LF goes whole)
        (b"a\xf0\r\nb", "[97,65533,13,10,98]"),
        // jq: [97,65533,98]
        (b"a\xed\nb", "[97,65533,10,98]"),
        // jq: [97,65533,98]
        (b"a\xf4\x90\nb", "[97,65533,10,98]"),
        // jq: [97,65533,10]
        (b"a\xe1\n\n", "[97,65533,10,10]"),
        // jq: [97,65533,65533,65533,122]
        (
            b"a\xe1\n\xe1\n\xe1\nz",
            "[97,65533,10,65533,10,65533,10,122]",
        ),
        // jq: [97,65533,233,98]
        (b"a\xe1\n\xc3\xa9b", "[97,65533,10,233,98]"),
        // jq: [97,65533,128512,98]
        (b"a\xf0\n\xf0\x9f\x98\x80b", "[97,65533,10,128512,98]"),
        // jq: [97,65533,0,98]
        (b"a\xe1\n\x00b", "[97,65533,10,0,98]"),
        // jq: [65279,97,65533,98]
        (b"\xef\xbb\xbfa\xe1\nb", "[65279,97,65533,10,98]"),
    ] {
        let (stdout, code) = slurp_raw(input)?;
        assert_eq!(
            (stdout.as_str(), code),
            (format!("{want}\n").as_str(), 0),
            "{input:?}"
        );
    }
    Ok(())
}

/// The neighbours where enough bytes remain before the line break, or the byte is no lead, or the
/// file ends: jq 1.7.1, jq 1.8.2 and succinctly print the same thing.
#[test]
fn raw_slurp_rows_that_agree_with_jq_7866() -> Result<()> {
    for (input, want) in [
        // A 2-byte lead needs only the line break itself to be present.
        (&b"a\xc3\nb"[..], "[97,65533,10,98]"),
        (b"a\xc3\r\nb", "[97,65533,13,10,98]"),
        // One byte short: the line break makes up the count.
        (b"a\xe1\x80\nb", "[97,65533,10,98]"),
        (b"a\xef\xbb\nb", "[97,65533,10,98]"),
        (b"a\xed\xa0\nb", "[97,65533,10,98]"),
        (b"a\xf0\x9f\x98\nb", "[97,65533,10,98]"),
        (b"a\xe1\r\nb\r\n", "[97,65533,13,10,98,13,10]"),
        (b"a\xf0\x9f\r\nb", "[97,65533,13,10,98]"),
        (b"a\xe1\x00\nb", "[97,65533,0,10,98]"),
        // Not a lead byte.
        (b"a\xff\nb", "[97,65533,10,98]"),
        (b"a\x80\nb", "[97,65533,10,98]"),
        // No line break after it: the rest of the declared length goes, as everywhere (#1717).
        (b"a\xe1Ab", "[97,65533,65,98]"),
        // At the end of the input, with and without a final line break: jq's read and the
        // whole-file slice end at the same byte, so the line break goes in all three.
        (b"a\xe1", "[97,65533]"),
        (b"a\xf0\x9f", "[97,65533]"),
        (b"a\xc3", "[97,65533]"),
        (b"a\xe1\n", "[97,65533]"),
        (b"a\xf0\n", "[97,65533]"),
        (b"a\xc3\n", "[97,65533,10]"),
    ] {
        let (stdout, code) = slurp_raw(input)?;
        assert_eq!(
            (stdout.as_str(), code),
            (format!("{want}\n").as_str(), 0),
            "{input:?}"
        );
    }
    Ok(())
}

/// A file that ends in the truncated lead byte and its line break, followed by another file: the
/// slice is the file, so the line break goes here as it does in both jq versions.
#[test]
fn raw_slurp_scopes_the_substitution_to_each_file_as_jq_does_7866() -> Result<()> {
    let (stdout, _, code, _) =
        run_jq_over_byte_files(&["-R", "-s", "-c", "explode"], &[b"a\xe1\n", b"b\n"])?;
    assert_eq!((stdout.as_str(), code), ("[97,65533,98,10]\n", 0));
    let (stdout, _, code, _) =
        run_jq_over_byte_files(&["-R", "-s", "-c", "explode"], &[b"a\xe1", b"b\n"])?;
    assert_eq!((stdout.as_str(), code), ("[97,65533,98,10]\n", 0));
    Ok(())
}

/// Without `--slurp` the lines are split before the substitution (#1742), so nothing differs:
/// both jq versions print these too.
#[test]
fn raw_input_per_line_routes_agree_with_jq_7866() -> Result<()> {
    for (input, args, filter, want) in [
        (
            &b"a\n\xef\nb\n"[..],
            &["-R", "-c"][..],
            "explode",
            "[97]\n[65533]\n[98]\n",
        ),
        (
            b"a\xf0\x9f\nb",
            &["-R", "-c"],
            "explode",
            "[97,65533]\n[98]\n",
        ),
        (
            b"a\xf0\r\nb",
            &["-R", "-c"],
            "explode",
            "[97,65533]\n[98]\n",
        ),
        (
            b"a\n\xef\nb\n",
            &["-nR", "-c"],
            "[inputs|explode]",
            "[[97],[65533],[98]]\n",
        ),
        (b"\xe1\nb", &["-nR", "-c"], "input|explode", "[65533]\n"),
    ] {
        let (stdout, code) = run_jq_binary_stdin(filter, input, args)?;
        assert_eq!(
            (String::from_utf8(stdout)?.as_str(), code),
            (want, 0),
            "{input:?} {args:?}"
        );
    }
    Ok(())
}

/// A JSON string is its own slice under `--seq` and `--slurp` (#1743), so a truncated lead byte
/// before a line break inside one never reaches past the string: both jq versions print these.
#[test]
fn seq_and_slurp_json_strings_agree_with_jq_7866() -> Result<()> {
    for (input, args, filter, want) in [
        (
            &b"\x1e\"a\xef\"\n\x1e\"b\"\n"[..],
            &["--seq", "-c"][..],
            "explode",
            "\x1e[97,65533]\n\x1e[98]\n",
        ),
        (
            b"\"a\xef\"\n\"b\"\n",
            &["-s", "-c"],
            "map(explode)",
            "[[97,65533],[98]]\n",
        ),
    ] {
        let (stdout, code) = run_jq_binary_stdin(filter, input, args)?;
        assert_eq!(
            (String::from_utf8(stdout)?.as_str(), code),
            (want, 0),
            "{input:?} {args:?}"
        );
    }
    Ok(())
}

/// `prefix` ASCII bytes, `tail`, a line break, then `z` and a line break.
fn long_line(prefix: usize, tail: &[u8]) -> Vec<u8> {
    let mut input = vec![b'a'; prefix];
    input.extend_from_slice(tail);
    input.extend_from_slice(b"\nz\n");
    input
}

/// Near a 4095-byte read boundary of a long line the two jq versions differ from each other, and
/// succinctly prints what one of them prints. The last row is the ordinary difference again, far
/// from a boundary.
#[test]
fn raw_slurp_long_line_rows_match_one_jq_version_7866() -> Result<()> {
    for (prefix, tail, want) in [
        // jq 1.7.1: [97,65533,10,122,10] (as here); jq 1.8.2: [97,97,65533,122,10].
        (4094, &b"\xe1"[..], "[97,65533,10,122,10]"),
        // jq 1.8.2: [97,65533,10,122,10] (as here); jq 1.7.1: [97,97,65533,122,10].
        (4090, b"\xe1", "[97,65533,10,122,10]"),
        // jq 1.8.2: [65533,88,10,122,10] (as here); jq 1.7.1: [97,65533,10,122,10], without the X.
        (4093, b"\xe1X", "[65533,88,10,122,10]"),
        // Away from a boundary, both jq versions: [97,97,65533,122,10].
        (1000, b"\xe1", "[97,65533,10,122,10]"),
    ] {
        let input = long_line(prefix, tail);
        let (stdout, code) = run_jq_binary_stdin("explode|.[-5:]", &input, &["-R", "-s", "-c"])?;
        assert_eq!(
            (String::from_utf8(stdout)?.as_str(), code),
            (format!("{want}\n").as_str(), 0),
            "{prefix} {tail:?}"
        );
    }
    Ok(())
}
