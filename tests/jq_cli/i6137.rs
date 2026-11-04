//! #6137: under `--seq` without `-n`, a program that reads `input`/`inputs` meets the reader's
//! parse errors as jq's does: the read that reaches a fault raises it, and one the program does not
//! read is printed as `jq: ignoring parse error: ...` when the main loop reaches it, after the
//! values before it, rather than up front. The records after a fault then pair with the main loop
//! as in jq. The reader's message-less return (#5662), `break` to `input`, ends the main loop.
//! `-s` still warns as it reads. Rows, messages, locations and exit statuses were captured from
//! jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `(stdin, filter, slurp, stdout, stderr, exit status)`.
const ROWS: &[(&str, &str, bool, &str, &str, i32)] = &[
    ("\x1e1\n\x1e[1,]\n\x1e2\n\x1e3\n", "[.,input]", false, "\x1e[2,3]\n", "jq: error (at <stdin>:2): Expected another array element at line 2, column 5 (need RS to resync)\n", 0),
    ("\x1e1\n\x1e[1,]\n\x1e2\n\x1e3\n", "[inputs]", false, "\x1e[3]\n", "jq: error (at <stdin>:2): Expected another array element at line 2, column 5 (need RS to resync)\n", 0),
    ("\x1e1\n\x1e[1,]\n\x1e2\n\x1e3\n", "[.,(try inputs catch .)]", false, "\x1e[1,\"Expected another array element at line 2, column 5 (need RS to resync)\"]\n\x1e[2,3]\n", "", 0),
    ("\x1e1\n\x1e[1,]\n\x1e2\n\x1e3\n", "first(inputs)", false, "\x1e3\n", "jq: error (at <stdin>:2): Expected another array element at line 2, column 5 (need RS to resync)\n", 0),
    ("\x1e1\n\x1e[1,]\n\x1e[2,]\n\x1e3\n\x1e4\n", "[.,input]", false, "\x1e[3,4]\n", "jq: error (at <stdin>:2): Expected another array element at line 2, column 5 (need RS to resync)\njq: ignoring parse error: Expected another array element at line 3, column 5 (need RS to resync)\n", 0),
    ("\x1e[1,]\n\x1e2\n\x1e3\n", "[.,input]", false, "\x1e[2,3]\n", "jq: ignoring parse error: Expected another array element at line 1, column 5 (need RS to resync)\n", 0),
    ("\x1e1\n\x1e2\n\x1e[1,]\n", "[.,input]", false, "\x1e[1,2]\n", "jq: ignoring parse error: Expected another array element at line 3, column 5 (need RS to resync)\n", 0),
    ("\x1e1\n\x1e[1,]\n\x1e2\n\x1e3\n", ".,input_line_number", false, "\x1e1\n\x1e1\n\x1e2\n\x1e3\n\x1e3\n\x1e4\n", "jq: ignoring parse error: Expected another array element at line 2, column 5 (need RS to resync)\n", 0),
    // The main loop prints a warning after the values before it: `halt` on the first suppresses
    // it, and an error on the first comes ahead of it.
    ("\x1e1\n\x1e[1,]\n\x1e2\n", "if . == 1 then halt else . end, input_line_number", false, "", "", 0),
    ("\x1e1\n\x1e[1,]\n\x1e2\n", "if . == 1 then error(\"E\") else . end, input_line_number", false, "\x1e2\n\x1e3\n", "jq: error (at <stdin>:1): E\njq: ignoring parse error: Expected another array element at line 2, column 5 (need RS to resync)\n", 0),
    // The message-less return ends the main loop, and `input` reads on past it.
    ("\x1e0\n\x1e1[", "if . == 1 then input else . end", false, "\x1e0\n", "", 0),
    ("\x1e0\n\x1e1[", "[.,(try input catch .)]", false, "\x1e[0,\"break\"]\n\x1e[1,\"Unfinished JSON term at EOF at line 2, column 3\"]\n", "", 0),
    ("\x1e0\n\x1e1[", "[.,input]", false, "", "jq: error (at <stdin>:1): break\njq: error (at <unknown>): Unfinished JSON term at EOF at line 2, column 3\n", 5),
    ("\x1e0\n\x1e1", "[limit(4;repeat(try input catch .))]", false, "\x1e[\"break\",\"Potentially truncated top-level numeric value at EOF at line 2, column 2\",\"break\",\"break\"]\n", "", 0),
    // An empty record's stop is the same return (#6141's rows over documents): `inputs` ends at
    // it, and the main loop reads on to the value and the fault behind it.
    ("\x1e1\n\x1e\x1e2", "[inputs]", false, "\x1e[]\n", "jq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 2, column 3\n", 0),
    ("\x1e1\n\x1e\x1e2", "[.,(try input catch .)], input_line_number", false, "\x1e[1,\"break\"]\n\x1e1\n", "jq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 2, column 3\n", 0),
    ("\x1e1\n\x1e\x1e2", "first(inputs)", false, "", "jq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 2, column 3\n", 0),
    // `-s` reads the whole stream, warning as it goes, before the program runs.
    ("\x1e1\n\x1e[1,]\n\x1e2\n\x1e3\n", "[.,input]", true, "", "jq: ignoring parse error: Expected another array element at line 2, column 5 (need RS to resync)\njq: error (at <unknown>): break\n", 5),
];

#[test]
fn test_seq_input_raises_the_readers_parse_errors_without_null_input_6137() -> Result<()> {
    for (stdin, filter, slurp, stdout, stderr, code) in ROWS {
        let mut args = vec!["-c", "--seq"];
        if *slurp {
            args.push("-s");
        }
        args.push(filter);
        let (out, err, status) = run_jq_full(&args, Some(stdin))?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (*stdout, *stderr, *code),
            "{args:?} {stdin:?}"
        );
    }
    Ok(())
}

/// `-e` judges the last output, past a fault the main loop skipped or `input` raised.
#[test]
fn test_seq_input_exit_status_past_a_fault_6137() -> Result<()> {
    /// `(stdin, filter, stdout, stderr, exit status)`.
    const EXIT_ROWS: &[(&str, &str, &str, &str, i32)] = &[
        ("\x1e1\n\x1e[1,]\n\x1e2\n\x1e3\n", "[.,input] | .[1] > 5", "\x1efalse\n", "jq: error (at <stdin>:2): Expected another array element at line 2, column 5 (need RS to resync)\n", 1),
        ("\x1e1\n\x1e2\n\x1e[1,]\n", "[.,input] | .[1] == 2", "\x1etrue\n", "jq: ignoring parse error: Expected another array element at line 3, column 5 (need RS to resync)\n", 0),
        ("\x1e1\n\x1e[1,]\n\x1e2\n", "if . == 1 then true else input end", "\x1etrue\n", "jq: ignoring parse error: Expected another array element at line 2, column 5 (need RS to resync)\njq: error (at <stdin>:3): break\n", 5),
        ("\x1e1\n\x1e[1,]\n\x1e2\n", "input_line_number == 3", "\x1efalse\n\x1etrue\n", "jq: ignoring parse error: Expected another array element at line 2, column 5 (need RS to resync)\n", 0),
    ];
    for (stdin, filter, stdout, stderr, code) in EXIT_ROWS {
        let args = ["-ce", "--seq", filter];
        let (out, err, status) = run_jq_full(&args, Some(stdin))?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (*stdout, *stderr, *code),
            "{args:?} {stdin:?}"
        );
    }
    Ok(())
}

/// The fault names the file it is in, and `input_filename` the file of the value read after it.
#[test]
fn test_seq_input_names_the_file_of_a_fault_6137() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let a = dir.path().join("a.json");
    let b = dir.path().join("b.json");
    std::fs::write(&a, "\x1e1\n")?;
    std::fs::write(&b, "\x1e[1,]\n\x1e3\n\x1e4\n")?;
    let (a, b) = (a.to_str().unwrap(), b.to_str().unwrap());
    let fault = "Expected another array element at line 2, column 5 (need RS to resync)";

    let (out, err, status) =
        run_jq_full(&["-c", "--seq", "[., input, input_filename]", a, b], None)?;
    assert_eq!(
        (out, err, status),
        (
            format!("\x1e[3,4,\"{b}\"]\n"),
            format!("jq: error (at {b}:1): {fault}\n"),
            0
        )
    );

    let (out, err, status) = run_jq_full(
        &[
            "-c",
            "--seq",
            "[., (try input catch .), input_filename]",
            a,
            b,
        ],
        None,
    )?;
    assert_eq!(
        (out, err.as_str(), status),
        (
            format!("\x1e[1,\"{fault}\",\"{b}\"]\n\x1e[3,4,\"{b}\"]\n"),
            "",
            0
        )
    );
    Ok(())
}
