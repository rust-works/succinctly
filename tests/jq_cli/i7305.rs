//! #7305: on x86_64 the JSON index kernel (AVX2 and SSE2 alike) skips a whole run of
//! 32-byte blocks that lie inside a string with one write. [`input`] holds strings whose
//! plain runs span one to five blocks and end at every offset of a block, each followed by
//! an escaped quote or an escaped backslash and then by values, so a run's end meets every
//! block position and an escape. Every expected answer below is what jq 1.7.1 prints for
//! this input.

use anyhow::Result;

use crate::run_jq_full;

/// The escape after object `i`'s plain run: an escaped quote or an escaped backslash.
fn escape(i: usize) -> &'static str {
    if i % 2 == 0 {
        r#"\""#
    } else {
        r"\\"
    }
}

/// The length of object `i`'s plain run: 32 to 191 bytes, every remainder mod 32.
fn run_len(i: usize) -> usize {
    32 * (1 + i % 5) + (i * 7) % 32
}

/// 160 compact objects; each string is a long plain run, an escape and a short tail.
fn input() -> String {
    let objects: Vec<String> = (0..160)
        .map(|i| {
            let s = format!("{}{}x", "r".repeat(run_len(i)), escape(i));
            format!(r#"{{"s":"{s}","n":{i},"v":[false,"q",{{"k":[]}}]}}"#)
        })
        .collect();
    format!("[{}]", objects.join(","))
}

#[test]
fn compact_identity_reproduces_the_document() -> Result<()> {
    let input = input();
    let (out, err, code) = run_jq_full(&["-c", "."], Some(&input))?;
    assert_eq!((err.as_str(), code), ("", 0));
    assert_eq!(out, format!("{input}\n"));
    Ok(())
}

#[test]
fn every_value_after_a_skipped_run_is_found() -> Result<()> {
    let input = input();
    // Each decoded string is the run, one escaped character and `x`.
    let total: usize = (0..160).map(|i| run_len(i) + 2).sum();
    let rows: &[(&str, String)] = &[
        ("[.[] | .s | length] | add", format!("{total}\n")),
        ("[.[] | .n] | add", "12720\n".to_string()),
        ("[.[] | .v[1]] | unique", "[\"q\"]\n".to_string()),
        ("[.[] | .v[2].k | length] | add", "0\n".to_string()),
        (
            ".[37] | [.n, (.s | length), (.s | .[-2:])]",
            format!("[37,{},\"\\\\x\"]\n", run_len(37) + 2),
        ),
        (
            ".[64] | [.n, (.s | length), (.s | .[-2:])]",
            format!("[64,{},\"\\\"x\"]\n", run_len(64) + 2),
        ),
    ];
    for (filter, expected) in rows {
        let (out, err, code) = run_jq_full(&["-c", filter], Some(&input))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (expected.as_str(), "", 0),
            "{filter}"
        );
    }
    Ok(())
}
