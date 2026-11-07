//! #6454: a depth refusal inside an object (`{` then 300 `[`) is named by the document walk, not by
//! the parser model, and that route kept no parser end, so the next fault was placed past the bytes
//! jq drops after the refusal (the rest of the `fgets` read, its newline included). jq places it from
//! where the refusal left its counters. Every expected string was captured from jq 1.7.1
//! (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

const FILTER: &str = "[limit(8; repeat(try input catch .))]";

#[test]
fn a_fault_after_a_depth_refusal_in_an_object_is_placed_from_the_refusal_6454() -> Result<()> {
    let deep = format!("{{{}", "[".repeat(300));
    for (tail, want) in [
        // The fault is on the line after the refusal: jq never counted the newline.
        ("\n\"k\": 1}\n{\"b\":2}\n", "[\"Exceeds depth limit for parsing at line 1, column 257\",\"k\",\"Expected string key before ':' at line 1, column 261\",{\"b\":2},\"break\",\"break\",\"break\",\"break\"]\n"),
        // A second newline is read, so it is counted.
        ("\n\n\"k\": 1}\n", "[\"Exceeds depth limit for parsing at line 1, column 257\",\"k\",\"Expected string key before ':' at line 2, column 4\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        ("\n\"k\" }\n", "[\"Exceeds depth limit for parsing at line 1, column 257\",\"k\",\"Unmatched '}' at line 1, column 262\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        // The stale line carries on to the faults after it.
        ("\n\"k\" 1 }\n[3]\n}\n", "[\"Exceeds depth limit for parsing at line 1, column 257\",\"k\",1,\"Unmatched '}' at line 1, column 264\",[3],\"Unmatched '}' at line 2, column 1\",\"break\",\"break\"]\n"),
    ] {
        let input = format!("{deep}{tail}");
        let (stdout, _, code) = run_jq_full(&["-nc", FILTER], Some(&input))?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{tail:?}");
    }
    Ok(())
}

/// The issue's own repro: a document before the refusal puts it on line 2.
#[test]
fn the_issue_repro_names_the_stale_position_6454() -> Result<()> {
    let input = format!(
        "{{\"a\":1}}\n{{{}\n\"k\": 1}}\n{{\"b\":2}}\n",
        "[".repeat(300)
    );
    let (stdout, _, code) = run_jq_full(&["-c", "try ([inputs]|length) catch ."], Some(&input))?;
    assert_eq!(
        (stdout.as_str(), code),
        (
            "\"Exceeds depth limit for parsing at line 2, column 257\"\n\"Expected string key before ':' at line 2, column 261\"\n0\n",
            0
        )
    );
    Ok(())
}
