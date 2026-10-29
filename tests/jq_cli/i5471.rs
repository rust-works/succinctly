//! #5471: jq's exit status is the last top-level input's outcome, so an error on an earlier input
//! followed by a clean last one exits 0. `--on-error=continue` (exit 5 if any input failed) and
//! `--on-error=stop` (evaluate nothing after the first failure, exit 5) are succinctly extensions.
//! The stdout and default exit status of every row were captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::{run_jq_full, run_jq_over_files};

/// Fails a number, so `.a` raises on `1` and answers on an object.
const RAISE_ON_NUMBER: &str = r#".[] | if type == "number" then error("n") else . end"#;

#[test]
fn test_exit_status_is_the_last_inputs_and_on_error_overrides_it_5471() -> Result<()> {
    // (input, args, jq's stdout = continue's, jq's code, continue's code, stop's stdout)
    let rows: &[(&str, &[&str], &str, i32, i32, &str)] = &[
        // The issue's own repro: an error on an early input, a clean last one.
        ("1 {\"a\":2}", &[".a"], "2\n", 0, 5, ""),
        ("{\"a\":2} 1", &[".a"], "2\n", 5, 5, "2\n"),
        ("1 {\"a\":2}", &["-e", ".a"], "2\n", 0, 5, ""),
        ("1 {\"a\":null}", &["-e", ".a"], "null\n", 1, 5, ""),
        // An input that printed and then raised leaves `-e`'s last result as it found it.
        (
            "[false,1] []",
            &["-e", RAISE_ON_NUMBER],
            "false\n",
            4,
            5,
            "false\n",
        ),
        (
            "[true] [false,1] []",
            &["-e", RAISE_ON_NUMBER],
            "true\nfalse\n",
            0,
            5,
            "true\nfalse\n",
        ),
        (
            "[false,1] []",
            &[RAISE_ON_NUMBER],
            "false\n",
            0,
            5,
            "false\n",
        ),
        // The general path, and a flag that keeps `.a` off the M2 fast path's colorless writer.
        ("1 {\"a\":2}", &[".a | tostring"], "\"2\"\n", 0, 5, ""),
        ("1 {\"a\":2}", &["-S", ".a"], "2\n", 0, 5, ""),
        // jq runs the program once per `--stream` event and once per `-R` line.
        (
            "[1,\"x\"]",
            &["--stream", "if .[1] == 1 then error(\"e\") else . end"],
            "[[1],\"x\"]\n[[1]]\n",
            0,
            5,
            "",
        ),
        (
            "a\nb\n",
            &["-R", "if . == \"a\" then error(\"e\") else . end"],
            "\"b\"\n",
            0,
            5,
            "",
        ),
        // An input builtin routes through the input queue's own loop.
        (
            "1 2",
            &["input_line_number as $n | if . == 1 then error(\"e\") else . end"],
            "2\n",
            0,
            5,
            "",
        ),
        // A parse error ends the stream, so it is the last input's outcome.
        ("1 2 {", &["."], "1\n2\n", 5, 5, "1\n2\n"),
        ("1 {\"a\":2} {", &[".a"], "2\n", 5, 5, ""),
        // A `--seq` parse error is ignored and leaves the status the input before it set.
        ("\u{1e}1\n\u{1e}{\n", &["--seq", ".a"], "", 5, 5, ""),
        // A NUL under `--raw-output0` fails the input it was printed for.
        (
            "\"a\" \"b\\u0000c\" \"d\"",
            &["-r", "--raw-output0", "."],
            "a\0d\0",
            0,
            5,
            "a\0",
        ),
        // `halt_error` outranks all three.
        (
            "1 2",
            &["if . == 1 then halt_error(3) else . end"],
            "",
            3,
            3,
            "",
        ),
        ("", &["-e", "."], "", 4, 4, ""),
        ("1 false", &["-e", "."], "1\nfalse\n", 1, 1, "1\nfalse\n"),
    ];
    for &(input, args, stdout, jq_code, any_code, stop_stdout) in rows {
        for (on_error, want_stdout, want_code) in [
            (None, stdout, jq_code),
            (Some("--on-error=continue"), stdout, any_code),
            (Some("--on-error=stop"), stop_stdout, any_code),
        ] {
            let mut full = vec!["-c"];
            full.extend(on_error);
            full.extend_from_slice(args);
            let (out, err, code) = run_jq_full(&full, Some(input))?;
            assert_eq!(
                (out.as_str(), code),
                (want_stdout, want_code),
                "{full:?} over {input:?}: {err}"
            );
        }
    }
    Ok(())
}

/// jq stops at a malformed document and exits 5; succinctly reads on past it (it validates
/// only what a filter reads), so a decode failure still fails the run when a clean document
/// follows. jq 1.7.1 prints nothing here; the `1` is that recorded divergence.
#[test]
fn test_a_malformed_earlier_document_still_fails_the_run_5471() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-c", ".a"], Some("{\"a\":tru} {\"a\":1}"))?;
    assert_eq!((out.as_str(), code), ("1\n", 5), "{err}");
    Ok(())
}

/// The same rule across files: the last file's input decides.
#[test]
fn test_exit_status_follows_the_last_file_5471() -> Result<()> {
    let filter = "if . == 1 then error(\"e\") else . end";
    let (_, err, code, _) = run_jq_over_files(&["-c", filter], &["1\n", "2\n"])?;
    assert_eq!(code, 0, "{err}");
    let (_, err, code, _) = run_jq_over_files(&["-c", filter], &["2\n", "1\n"])?;
    assert_eq!(code, 5, "{err}");
    let (out, err, code, _) =
        run_jq_over_files(&["-c", "--on-error=stop", filter], &["1\n", "2\n"])?;
    assert_eq!((out.as_str(), code), ("", 5), "{err}");
    Ok(())
}

/// DSV input has no jq counterpart; each row is a top-level input, as each line is under `-R`.
#[test]
fn test_each_dsv_row_is_an_input_5471() -> Result<()> {
    for (on_error, want_stdout, want_code) in [
        (None, "1\n", 0),
        (Some("--on-error=continue"), "1\n", 5),
        (Some("--on-error=stop"), "", 5),
    ] {
        let mut args = vec!["-c", "--input-dsv", ","];
        args.extend(on_error);
        args.push(".[0] | tonumber");
        let (out, err, code) = run_jq_full(&args, Some("x\n1\n"))?;
        assert_eq!(
            (out.as_str(), code),
            (want_stdout, want_code),
            "{args:?}: {err}"
        );
    }
    Ok(())
}

/// The last `--on-error` given wins, and an unknown mode is a usage error.
#[test]
fn test_on_error_flag_parsing_5471() -> Result<()> {
    let (_, err, code) = run_jq_full(
        &["-c", "--on-error=stop", "--on-error=continue", ".a"],
        Some("1 {\"a\":2}"),
    )?;
    assert_eq!(code, 5, "{err}");
    let (_, err, code) = run_jq_full(&["--on-error=bogus", "."], Some("1"))?;
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("bogus"), "{err}");
    Ok(())
}
