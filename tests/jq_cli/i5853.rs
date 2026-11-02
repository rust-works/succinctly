//! #5853: under `-n`, `input`/`inputs` over one plain-JSON file convert the documents before the
//! first fault when the program reads them, and the eager route reads the stream from there. Every
//! row's expected text was captured from jq 1.7.1 (`/usr/bin/jq`); the placement of the fault, the
//! documents jq reads after resuming and `input_line_number` have to be the eager route's own.

use anyhow::Result;

use crate::{run_jq_full, run_jq_stdin};

/// The first seven reads of `input`, each parse error caught: what jq answers, fault included.
const READS: &str = "[limit(7; repeat(try input catch .))]";
/// The same with `input_line_number` after each read.
const READS_AND_LINES: &str = "[limit(7; repeat((try input catch .), input_line_number))]";

fn null_input(filter: &str, input: &str) -> Result<(String, i32)> {
    run_jq_stdin(filter, input, &["-nc"])
}

#[test]
fn a_stream_that_turns_malformed_reads_as_jq_reads_it_5853() -> Result<()> {
    for (input, filter, want) in [
        // A trailing `}` after a clean start: the documents, the error, then the end.
        (
            "1\n2\n3\n}\n",
            READS,
            "[1,2,3,\"Unmatched '}' at line 4, column 1\",\"break\",\"break\",\"break\"]\n",
        ),
        (
            "1\n2\n3\n}\n",
            READS_AND_LINES,
            "[1,1,2,2,3,3,\"Unmatched '}' at line 4, column 1\"]\n",
        ),
        // jq resumes after the fault and reads the document that follows it.
        (
            "{\"a\":1}\n{\"a\":2}\n[3]\n}\n{\"a\":4}\n",
            READS,
            "[{\"a\":1},{\"a\":2},[3],\"Unmatched '}' at line 4, column 1\",{\"a\":4},\"break\",\"break\"]\n",
        ),
        (
            "{\"a\":1}\n{\"a\":2}\n[3]\n}\n{\"a\":4}\n",
            READS_AND_LINES,
            "[{\"a\":1},1,{\"a\":2},2,[3],3,\"Unmatched '}' at line 4, column 1\"]\n",
        ),
        // A number the fault's own byte completes is never read: `3}` is the error, not `3`.
        (
            "1 2 3}",
            READS,
            "[1,2,\"Unmatched '}' at line 1, column 6\",\"break\",\"break\",\"break\",\"break\"]\n",
        ),
        (
            "1 2 3}",
            READS_AND_LINES,
            "[1,0,2,0,\"Unmatched '}' at line 1, column 6\",0,\"break\"]\n",
        ),
        // `nan` and `007` are numbers jq reads and `serde_json` refuses.
        (
            "\"a\" \"b\" nan 3\n[4]\n",
            READS,
            "[\"a\",\"b\",null,3,[4],\"break\",\"break\"]\n",
        ),
        (
            "[1]\n[2]\n007\n[3]\n",
            READS_AND_LINES,
            "[[1],1,[2],2,7,3,[3]]\n",
        ),
        // A truncated last document.
        (
            "[1]\n[2]\n[3,\n",
            READS,
            "[[1],[2],\"Unfinished JSON term at EOF at line 4, column 0\",\"break\",\"break\",\"break\",\"break\"]\n",
        ),
        (
            "[1]\n[2]\n[3,\n",
            READS_AND_LINES,
            "[[1],1,[2],2,\"Unfinished JSON term at EOF at line 4, column 0\",3,\"break\"]\n",
        ),
    ] {
        assert_eq!(
            null_input(filter, input)?,
            (want.to_string(), 0),
            "{filter} over {input:?}"
        );
    }
    Ok(())
}

/// `first(inputs)` is answered by the lazy start alone, so a fault far behind it is never met.
#[test]
fn an_early_stop_never_meets_a_fault_behind_it_5853() -> Result<()> {
    for input in ["1\n2\n3\n}\n", "{\"a\":1}\n[3]\n}\n{\"a\":4}\n"] {
        let (out, code) = null_input("first(inputs)", input)?;
        assert_eq!(code, 0, "{input:?}");
        assert_eq!(out.lines().count(), 1, "{input:?}");
    }
    Ok(())
}

/// More than one parse error after the lazy start, each read in turn, and a document deeper than
/// the lazy route converts (it ends the lazy start, and the eager loop reads it).
#[test]
fn several_faults_and_a_deep_document_read_as_jq_reads_them_5853() -> Result<()> {
    // Two faults in a row: which reads are errors and which documents follow them is what the lazy
    // start must not move. (The second error's own line and column differ from jq's on the eager
    // route too: jq does not count the bytes it drops after the first fault, #5923.)
    let (out, code) = null_input(
        "[limit(8; repeat(try input catch .))] | map(if type == \"string\" then \"error\" else . end)",
        "[1]\n[2]\n[3]\n}\n}\n[4]\n",
    )?;
    assert_eq!(
        (out.as_str(), code),
        (
            "[[1],[2],[3],\"error\",\"error\",[4],\"error\",\"error\"]\n",
            0
        )
    );
    let deep = format!(
        "[1]\n[2]\n[3]\n{}{}\n}}\n[9]",
        "[".repeat(70),
        "]".repeat(70)
    );
    let (out, code) = null_input(
        "[limit(8; repeat(try input catch .))] | map(if type == \"string\" then . else \"doc\" end)",
        &deep,
    )?;
    assert_eq!(
        (out.as_str(), code),
        (
            "[\"doc\",\"doc\",\"doc\",\"doc\",\"Unmatched '}' at line 5, column 1\",\"doc\",\"break\",\"break\"]\n",
            0
        )
    );
    // A fault so early that the lazy start is under a quarter of the stream: the eager route
    // reads all of it, with the same answers.
    let (out, code) = null_input(
        "[limit(10; repeat(try input catch .))]",
        "1\n}\n2\n3\n4\n5\n6\n7\n",
    )?;
    assert_eq!(
        (out.as_str(), code),
        (
            "[1,\"Unmatched '}' at line 2, column 1\",2,3,4,5,6,7,\"break\",\"break\"]\n",
            0
        )
    );
    Ok(())
}

/// Without `-n` the driver loop pops its documents from the same queue: it prints the documents
/// before the fault and stops at it with exit 5, and reads nothing after it.
#[test]
fn the_driver_loop_stops_at_the_fault_after_the_lazy_start_5853() -> Result<()> {
    for (input, filter, want) in [
        ("1\n2\n3\n}\n4\n", ".", "1\n2\n3\n"),
        ("1\n2\n3\n}\n4\n", "input_line_number", "1\n2\n3\n"),
        (
            "1\n2\n3\n}\n4\n",
            "[., input_line_number]",
            "[1,1]\n[2,2]\n[3,3]\n",
        ),
        (
            "{\"a\":1}\n[2]\n[3]\n}\n}\n5\n",
            "[., input_line_number]",
            "[{\"a\":1},1]\n[[2],2]\n[[3],3]\n",
        ),
    ] {
        let (out, _, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!((out.as_str(), code), (want, 5), "{filter} over {input:?}");
    }
    Ok(())
}
