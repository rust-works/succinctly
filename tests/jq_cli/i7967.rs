//! #7967: ADR-0018's #7963 amendment E1. Through the first fault of a stream `succinctly jq` owes jq
//! everything: the values before it and their positions, the fault's message, line and column, and
//! the exit status when it ends the run. After it, output is best effort above a floor: reading
//! resumes no later than the start of the next line (the next RS under `--seq`), every value printed
//! is a complete JSON text lying wholly after the fault, a value neither jq release prints is a bug,
//! and the exit status is zero exactly when jq 1.7.1's is.
//!
//! `succinctly jq` resumes where jq's next read starts, which is the start of the next line for a
//! line of up to 4,095 bytes and the next 4,095-byte piece of a longer one, so it reads every value
//! jq reads after a fault, in jq's order with the faults between them. What it no longer models is
//! where jq's parser stands once a fault has made it drop the rest of a read: a later fault is
//! placed where it is in the text, where jq does not count the bytes it dropped (#5923).
//!
//! [`masked`] is the floor's check for the tests that pinned jq's exact positions after a fault:
//! every expected string there was captured from jq 1.7.1 (`/usr/bin/jq`), and still is what jq
//! prints. Each now asserts that output, with the position of every fault after the first masked.

use anyhow::Result;

use crate::run_jq_full;

/// `text` with the `line N, column M` of every parse error after the first replaced by
/// `line _, column _` (#7967): the first fault's position is owed exactly, the later ones are best
/// effort. Everything else, the values, the order of values and faults and each fault's wording,
/// is kept as it is, so a test of the floor still sees a value lost, added or moved.
pub(crate) fn masked(text: &str) -> String {
    const LINE: &str = "line ";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut seen = false;
    while let Some(at) = rest.find(LINE) {
        let after = &rest[at + LINE.len()..];
        let digits = |s: &str| s.bytes().take_while(u8::is_ascii_digit).count();
        let line_len = digits(after);
        let tail = &after[line_len..];
        let column = tail.strip_prefix(", column ").filter(|_| line_len > 0);
        match column.map(|c| (c, digits(c))) {
            Some((c, n)) if n > 0 => {
                out.push_str(&rest[..at]);
                if seen {
                    out.push_str("line _, column _");
                } else {
                    out.push_str(&rest[at..at + LINE.len() + line_len + ", column ".len() + n]);
                    seen = true;
                }
                rest = &c[n..];
            }
            _ => {
                out.push_str(&rest[..at + LINE.len()]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[test]
fn masked_keeps_the_first_fault_and_masks_the_rest_7967() {
    assert_eq!(
        masked("[\"Unmatched '}' at line 1, column 1\",\"Unmatched '}' at line 1, column 2\",[4]]"),
        "[\"Unmatched '}' at line 1, column 1\",\"Unmatched '}' at line _, column _\",[4]]"
    );
    // Text that is no position is kept, and so is a value that only looks like one.
    assert_eq!(masked("a line of text"), "a line of text");
    assert_eq!(masked("line 3, column x"), "line 3, column x");
    assert_eq!(
        masked("\"line 1, column 2\"\n\"line 3, column 4\"\n"),
        "\"line 1, column 2\"\n\"line _, column _\"\n"
    );
}

/// The issue's named risk: `1 } 2`, 6,000 spaces and `3` on one line, then `4`. jq's read after the
/// fault starts 4,095 bytes into the line, so it reads the `3` (captured from jq 1.7.1 and 1.8.2:
/// `[1,"Unmatched '}' at line 1, column 3",3,4,"break"]`, exit 0). The reader resumes where jq does,
/// so it does too: the floor would allow losing the `3`, and nothing here needs it lost.
#[test]
fn a_long_line_after_a_fault_is_read_on_from_jqs_next_read_7967() -> Result<()> {
    let input = format!("1 }} 2{}3\n4\n", " ".repeat(6000));
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "-n", "[limit(5; repeat(try input catch .))]"],
        Some(&input),
    )?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        (
            "[1,\"Unmatched '}' at line 1, column 3\",3,4,\"break\"]\n",
            "",
            0
        )
    );
    // The main loop ends at the fault, as jq's does (captured from both releases).
    let (stdout, stderr, code) = run_jq_full(&["-c", "."], Some(&input))?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        (
            "1\n",
            "jq: parse error: Unmatched '}' at line 1, column 3\n",
            5
        )
    );
    Ok(())
}

/// A later fault is placed where it is in the text (best effort above the floor). jq 1.7.1 and 1.8.2
/// place the second `}` of `}\n}\n` at line 1, column 2, from where the first fault left their parser
/// (they do not count the newline the first fault's read dropped); the values and the order of the
/// faults stay jq's.
#[test]
fn a_later_fault_is_placed_where_it_is_in_the_text_7967() -> Result<()> {
    let filter = "[limit(4; repeat(try input catch .))]";
    for (input, jq, ours) in [
        (
            "}\n}\n[4]\n",
            "[\"Unmatched '}' at line 1, column 1\",\"Unmatched '}' at line 1, column 2\",[4],\"break\"]\n",
            "[\"Unmatched '}' at line 1, column 1\",\"Unmatched '}' at line 2, column 1\",[4],\"break\"]\n",
        ),
        (
            "tru\n}\n5\n",
            "[\"Invalid literal at line 2, column 0\",\"Unmatched '}' at line 2, column 1\",5,\"break\"]\n",
            "[\"Invalid literal at line 2, column 0\",\"Unmatched '}' at line 2, column 1\",5,\"break\"]\n",
        ),
    ] {
        let (stdout, _, code) = run_jq_full(&["-nc", filter], Some(input))?;
        assert_eq!((stdout.as_str(), code), (ours, 0), "{input:?}");
        assert_eq!(masked(&stdout), masked(jq), "{input:?}: the floor");
    }
    Ok(())
}
