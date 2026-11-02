//! #5853: a stream that turns malformed after a clean start keeps that start lazy (#5661) and
//! hands the rest to the eager route. What the program sees is what it saw when the whole stream
//! was read up front, and what jq 1.7.1 answers (expected values captured from `/usr/bin/jq`):
//! the documents before the fault, the number or literal the fault's own byte completes (never
//! read), the error and its marker, the documents jq reads after resuming, and the reads past
//! the end.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_a_fault_after_a_clean_start_matches_jq_5853() -> Result<()> {
    for (stream, program, expected_stdout, expected_stderr, expected_code) in [
        // `3` is completed by the fault's own byte `}`, so jq never reads it.
        (
            "1\n2\n3}\n4\n",
            "[inputs]",
            "",
            "jq: error (at <stdin>:3): Unmatched '}' at line 3, column 2\n",
            5,
        ),
        (
            "1\n2\n3}\n4\n",
            r#"[.,(try input catch "E"),(try input catch "E"),(try input catch "E"),(try input catch "E")]"#,
            "[null,1,2,\"E\",4]\n",
            "",
            0,
        ),
        (
            "1 \"a\" [2]\n}\n5\n",
            "[inputs]",
            "",
            "jq: error (at <stdin>:2): Unmatched '}' at line 2, column 1\n",
            5,
        ),
        (
            "{\"a\":1}\n[2]\n3x\n6\n",
            "[.,(try input catch .),input_line_number]",
            "[null,{\"a\":1},1]\n",
            "",
            0,
        ),
        // A program that stops before the fault never meets it.
        (
            "\"s\"\n\"t\"\nnan\n\"u\"\n",
            "first(inputs)",
            "\"s\"\n",
            "",
            0,
        ),
        (
            "\"s\"\n\"t\"\n}\n\"u\"\n",
            "[limit(3; inputs)]",
            "",
            "jq: error (at <stdin>:3): Unmatched '}' at line 3, column 1\n",
            5,
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", program], Some(stream))?;
        assert_eq!(stdout, expected_stdout, "{program} over {stream:?}");
        assert_eq!(stderr, expected_stderr, "{program} over {stream:?}");
        assert_eq!(code, expected_code, "{program} over {stream:?}");
    }
    Ok(())
}

/// A long clean start before the fault: stopping early, reading to the fault and resuming past it
/// answer the same whether the start is converted up front or when read.
#[test]
fn test_a_long_clean_start_before_a_fault_5853() -> Result<()> {
    const DOCUMENTS: usize = 20_000;
    let mut stream: String = (0..DOCUMENTS)
        .map(|id| format!("{{\"id\":{id}}}\n"))
        .collect();
    stream.push_str("}\n{\"id\":-1}\n");
    for (program, expected) in [
        ("first(inputs) | .id", "0".to_string()),
        ("[limit(3; inputs) | .id]", "[0,1,2]".to_string()),
        (
            "reduce range(\\(DOCUMENTS)) as $i (0; . + (input | .id))",
            ((DOCUMENTS - 1) * DOCUMENTS / 2).to_string(),
        ),
        (
            "[range(20000) | input | .id] | length, (try input catch \"fault\"), (input | .id)",
            format!("{DOCUMENTS}\n\"fault\"\n-1"),
        ),
        ("input_line_number", "0".to_string()),
    ] {
        // `\(DOCUMENTS)` in a program above is a literal count, not a jq interpolation.
        let program = program.replace("\\(DOCUMENTS)", &DOCUMENTS.to_string());
        let (stdout, stderr, code) = run_jq_full(&["-nc", &program], Some(&stream))?;
        assert_eq!(code, 0, "{program}: {stderr:?}");
        assert_eq!(stdout.trim_end(), expected, "{program}");
    }
    // Reading through the fault without catching it is jq's error at the fault's line.
    let (stdout, stderr, code) = run_jq_full(&["-nc", "[inputs] | length"], Some(&stream))?;
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        format!(
            "jq: error (at <stdin>:{}): Unmatched '}}' at line {}, column 1\n",
            DOCUMENTS + 1,
            DOCUMENTS + 1
        )
    );
    assert_eq!(code, 5);
    Ok(())
}
