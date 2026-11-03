//! #5965: a parse error the main loop meets, for a program that uses the input builtins and is run
//! without `-n`, is jq's `jq: parse error: ...` with no input location, as it is when the program
//! reads no input. A parse error the program's own `input` reads keeps its `(at F:N)` marker. Every
//! row was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `(stream, program, extra flags, stdout, stderr, exit code)`: a row over standard input.
type Row = (&'static str, &'static str, &'static [&'static str], &'static str, &'static str, i32);

#[test]
fn test_5965_main_loop_parse_error_is_not_a_program_error() -> Result<()> {
    let rows: &[Row] = &[
        (
            "1\n}",
            "input_line_number",
            &["-c"],
            "1\n",
            "jq: parse error: Unmatched '}' at line 2, column 1\n",
            5,
        ),
        (
            "1\n2\n}\n3",
            "input_line_number",
            &["-c"],
            "1\n2\n",
            "jq: parse error: Unmatched '}' at line 3, column 1\n",
            5,
        ),
        // The main loop stops at the first fault, as jq's does.
        (
            "1\n}\n2\n}",
            "input_line_number",
            &["-c"],
            "1\n",
            "jq: parse error: Unmatched '}' at line 2, column 1\n",
            5,
        ),
        // Some of the stream is read by the program, the fault is met by the main loop.
        (
            "1\n2\n}\n3",
            "[.,(try input catch \"E\")]",
            &["-c"],
            "[1,2]\n",
            "jq: parse error: Unmatched '}' at line 3, column 1\n",
            5,
        ),
        // The program reads the fault itself: caught, so nothing is reported.
        ("1\n}", "[.,(try input catch \"E\")]", &["-c"], "[1,\"E\"]\n", "", 0),
        // `--stream` words it as its own parser does.
        (
            "1\n}",
            "input_line_number",
            &["-c", "--stream"],
            "1\n",
            "jq: parse error: Unmatched '}' at the top-level at line 2, column 1\n",
            5,
        ),
        // Uncaught in the program's own `input`, it is an error of the program.
        (
            "1\n}",
            "input",
            &["-c"],
            "",
            "jq: error (at <stdin>:1): Unmatched '}' at line 2, column 1\n",
            5,
        ),
    ];
    for &(stream, program, flags, want_out, want_err, want_code) in rows {
        let mut args: Vec<&str> = flags.to_vec();
        args.push(program);
        let (out, err, code) = run_jq_full(&args, Some(stream))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (want_out, want_err, want_code),
            "{stream:?} | jq {flags:?} {program}"
        );
    }
    Ok(())
}
