//! #5555: a malformed input document is worded as jq's parser words it, at the position it
//! names, which counts lines across the files as one stream. Every expected message was captured
//! from jq 1.7.1 (`/usr/bin/jq`).
//!
//! A malformed *member* that the semi-index accepts is found when the filter reads it and keeps
//! the evaluator's wording; this file pins the faults the splitter or a `input`/`inputs` read
//! meets, which are jq's.

use anyhow::Result;

use crate::{run_jq_full, run_jq_over_files};

const A: &str = "1\n2\n";
const B: &str = "3\n{\"a\":1,}\n";
const C: &str = "4\n";
const TRUNC: &str = "1 [2,";
const BRACE: &str = "1 2 }";

#[test]
fn test_the_default_route_names_the_splitters_fault_5555() -> Result<()> {
    for (files, out, message) in [
        (
            &[TRUNC][..],
            "1\n",
            "Unfinished JSON term at EOF at line 1, column 5",
        ),
        (
            &[A, TRUNC],
            "1\n2\n1\n",
            "Unfinished JSON term at EOF at line 3, column 5",
        ),
        (&[BRACE], "1\n2\n", "Unmatched '}' at line 1, column 5"),
    ] {
        let (stdout, stderr, code, _) = run_jq_over_files(&["-c", "."], files)?;
        assert_eq!(code, 5, "{files:?}");
        assert_eq!(stdout, out, "{files:?}");
        assert_eq!(stderr, format!("jq: parse error: {message}\n"), "{files:?}");
    }
    Ok(())
}

#[test]
fn test_slurp_names_the_first_fault_in_the_stream_5555() -> Result<()> {
    // A fault inside a value the splitter delimits comes before the one it stops at, in this
    // file or an earlier one.
    for (files, message) in [
        (
            &[TRUNC][..],
            "Unfinished JSON term at EOF at line 1, column 5",
        ),
        (&[BRACE], "Unmatched '}' at line 1, column 5"),
        (
            &["[1.5,-2\ne3] nul"],
            "Invalid numeric literal at line 2, column 3",
        ),
        (
            &["[1.5,-2\ne3]", TRUNC],
            "Invalid numeric literal at line 2, column 3",
        ),
        (
            &[A, "[1.5,-2\ne3] nul"],
            "Invalid numeric literal at line 4, column 3",
        ),
        // A bare token is cut by the newline after it, which jq counts.
        (&["ul\n"][..], "Invalid numeric literal at line 2, column 0"),
    ] {
        let (stdout, stderr, code, _) = run_jq_over_files(&["-sc", "."], files)?;
        assert_eq!((stdout.as_str(), code), ("", 5), "{files:?}");
        assert_eq!(stderr, format!("jq: parse error: {message}\n"), "{files:?}");
    }
    Ok(())
}

#[test]
fn test_slurp_with_input_builtins_reads_the_fault_before_the_program_5555() -> Result<()> {
    for filter in ["input", "[inputs]"] {
        let (stdout, stderr, code, _) = run_jq_over_files(&["-sc", filter], &[A, B, C])?;
        assert_eq!((stdout.as_str(), code), ("", 5), "{filter}");
        assert_eq!(
            stderr, "jq: parse error: Expected another key-value pair at line 4, column 8\n",
            "{filter}"
        );
    }
    Ok(())
}

#[test]
fn test_inputs_raise_the_fault_at_the_position_the_parser_stood_on_5555() -> Result<()> {
    // The file and line are the read that met the fault: line 2 of `b`, where jq's parser is
    // when it reaches the trailing comma, not the first file (line 0) a slurp used to name.
    for args in [&["-nc", "[inputs]"][..], &["-nc", "-s", "[inputs]"]] {
        let (stdout, stderr, code, paths) = run_jq_over_files(args, &[A, B, C])?;
        assert_eq!((stdout.as_str(), code), ("", 5), "{args:?}");
        assert_eq!(
            stderr,
            format!(
                "jq: error (at {}:2): Expected another key-value pair at line 4, column 8\n",
                paths[1]
            ),
            "{args:?}"
        );
    }
    // A fault at EOF reads as line 0 of the file that ended.
    let (_, stderr, code, paths) = run_jq_over_files(&["-nc", "-s", "[inputs]"], &[A, TRUNC])?;
    assert_eq!(code, 5);
    assert_eq!(
        stderr,
        format!(
            "jq: error (at {}:0): Unfinished JSON term at EOF at line 3, column 5\n",
            paths[1]
        )
    );
    // The newline after a bare token is part of its fault.
    let (_, stderr, code, paths) = run_jq_over_files(&["-nc", "input"], &["ul\n"])?;
    assert_eq!(code, 5);
    assert_eq!(
        stderr,
        format!(
            "jq: error (at {}:1): Invalid numeric literal at line 2, column 0\n",
            paths[0]
        )
    );
    // Standard input is one source.
    let (_, stderr, code) = run_jq_full(&["-nc", "-s", "[inputs]"], Some(B))?;
    assert_eq!(code, 5);
    assert_eq!(
        stderr,
        "jq: error (at <stdin>:2): Expected another key-value pair at line 2, column 8\n"
    );
    Ok(())
}

#[test]
fn test_a_stream_the_splitter_cannot_delimit_keeps_the_splitters_wording_with_stream_5555(
) -> Result<()> {
    // `--stream` reads with a parser that has no depth limit, and has no origin to place the
    // fault by here, so it keeps the splitter's wording.
    let (_, stderr, code) = run_jq_full(&["-c", "--stream", "."], Some("1 2 }"))?;
    assert_eq!(code, 5);
    assert!(stderr.contains("Invalid JSON text"), "{stderr:?}");
    Ok(())
}
