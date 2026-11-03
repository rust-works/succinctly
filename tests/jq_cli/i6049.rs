//! #6049: a `-0` array index or slice bound is `-0` in the path jq records, not `0`: `path(.[-0])` is
//! `[-0]`. The parser folded the literal to the integer index 0 and a computed key (`.[-0,1]`,
//! `.[-0|.]`) read its spelling as the integer too. Every row is what `/usr/bin/jq` 1.7.1 printed
//! for the same program or document filter.

use anyhow::Result;

use crate::{run_jq_full, run_jq_stdin};

/// `-nc PROGRAM`: stdout of a run that must succeed.
fn out(program: &str) -> Result<String> {
    let (stdout, _stderr, code) = run_jq_full(&["-nc", program], None)?;
    assert_eq!(code, 0, "`{program}` failed");
    Ok(stdout.trim().to_string())
}

#[test]
fn a_negative_zero_index_or_bound_is_negative_zero_in_the_path() -> Result<()> {
    let rows: &[(&str, &str)] = &[
        ("[1,2,3] | path(.[-0])", "[-0]"),
        ("{\"a\":[1]} | path(.a[-0])", "[\"a\",-0]"),
        ("[[1]] | path(.[-0][-0])", "[-0,-0]"),
        (
            "[1,2,3] | [path(.[-0:])]",
            "[[{\"start\":-0,\"end\":null}]]",
        ),
        (
            "[1,2,3] | [path(.[:-0])]",
            "[[{\"start\":null,\"end\":-0}]]",
        ),
        (
            "[1,2,3] | [path(.[-0:-0])]",
            "[[{\"start\":-0,\"end\":-0}]]",
        ),
        // The other spellings, and a key that is computed rather than folded.
        ("[1,2,3] | [path(.[-00])]", "[[-0]]"),
        ("[1,2,3] | [path(.[(-0)])]", "[[-0]]"),
        ("[1,2,3] | [path(.[- 0])]", "[[-0]]"),
        ("[1,2,3] | [path(.[-0,1])]", "[[-0],[1]]"),
        ("[1,2,3] | [path(.[1,-0])]", "[[1],[-0]]"),
        ("[1,2,3] | [path(.[-0|.])]", "[[-0]]"),
        ("[1,2,3] | [path(.[if true then -0 else 1 end])]", "[[-0]]"),
        ("null | [path(.[-0])]", "[[-0]]"),
        (
            "[1,2,3] | [path(.[(-0):])]",
            "[[{\"start\":-0,\"end\":null}]]",
        ),
        ("[1,2,3] | [path(.[-0]?)]", "[[-0]]"),
        (
            "[1,2,3] | [path(.[-0:]?)]",
            "[[{\"start\":-0,\"end\":null}]]",
        ),
        // A second negation of the same zero is a positive zero, as in jq.
        ("[1,2,3] | [path(.[-1 * -0])]", "[[0]]"),
        ("[1,2,3] | [path(.[-(-0)])]", "[[0]]"),
        ("[1,2,3] | [path(.[-0 | -.])]", "[[0]]"),
        // Neighbours that must not change: the value, the writes and the other zeros.
        ("[1,2,3] | .[-0]", "1"),
        ("[1,2,3] | .[-0:]", "[1,2,3]"),
        ("[1,2,3] | (.[-0] = 9)", "[9,2,3]"),
        ("[1,2,3] | del(.[-0])", "[2,3]"),
        ("[1,2,3] | pick(.[-0])", "[1]"),
        ("[1,2,3] | [paths]", "[[0],[1],[2]]"),
        ("[1,2,3] | [path(.[0])]", "[[0]]"),
        ("[1,2,3] | [path(.[-1])]", "[[-1]]"),
        ("[1,2,3] | [path(.[-0.0])]", "[[-0]]"),
        ("[1,2,3] | [path(.[0.0])]", "[[0.0]]"),
        ("[1,2,3] | [path(.[-0+0])]", "[[0]]"),
    ];
    for (program, want) in rows {
        assert_eq!(out(program)?, *want, "`{program}`");
    }
    Ok(())
}

#[test]
fn a_negative_zero_index_in_a_document_filter_is_negative_zero_in_the_path() -> Result<()> {
    for (filter, want) in [
        ("path(.[-0])", "[-0]"),
        ("[path(.[-0,1])]", "[[-0],[1]]"),
        ("[path(.[-0:])]", "[[{\"start\":-0,\"end\":null}]]"),
    ] {
        let (stdout, code) = run_jq_stdin(filter, "[1,2,3]", &["-c"])?;
        assert_eq!(code, 0, "`{filter}` failed");
        assert_eq!(stdout.trim(), want, "`{filter}`");
    }
    Ok(())
}
