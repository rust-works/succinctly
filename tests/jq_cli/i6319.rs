//! #6319: a number literal past the double range (`1e1000`) is kept by jq as its canonical literal
//! (`1E+1000`) wherever it travels unchanged, and a computed infinity prints as `DBL_MAX` text.
//! `succinctly jq` printed `null` for both when the answer was an owned value streamed through
//! the library's jq output convention (`stream_owned_value_json_jq_output`), which carried yq's
//! rule for an infinite number: `[1e1000] | map(.)?`, `map(.) | sort`, `min`, `max`, `reverse`,
//! `sort_by(.)` and `unique_by(.)` all printed `null`. Every expectation is what `/usr/bin/jq`
//! 1.7.1 printed.

use anyhow::Result;

use crate::run_jq_full;

/// (stdin, filter, flags, jq 1.7.1's stdout).
const ROWS: &[(&str, &str, &[&str], &str)] = &[
    // past decNumber's exponent limit jq prints the DBL_MAX text, as it does for `.`
    (
        "[1e1000000000]",
        "map(.)?",
        &["-c"],
        "[1.7976931348623157e+308]\n",
    ),
    (
        "[-1e999999999999]",
        "map(.)|sort",
        &["-c"],
        "[-1.7976931348623157e+308]\n",
    ),
    // `-S` orders the keys of an object holding one
    (
        r#"{"b":1e1000,"a":[-1e400]}"#,
        "map(.)?",
        &["-S", "-c"],
        "[1E+1000,[-1E+400]]\n",
    ),
    ("[1e1000]", "map(.)?", &["-c"], "[1E+1000]\n"),
    ("[1e1000]", "map(.)|sort", &["-c"], "[1E+1000]\n"),
    ("[1e1000]", "map(.)|sort|.[0]", &["-c"], "1E+1000\n"),
    ("[1e1000]", "map(.)|min", &["-c"], "1E+1000\n"),
    ("[1e1000]", "map(.)|max", &["-c"], "1E+1000\n"),
    ("[1e1000]", "map(.)|reverse", &["-c"], "[1E+1000]\n"),
    ("[1e1000]", "map(.)|sort_by(.)", &["-c"], "[1E+1000]\n"),
    ("[1e1000]", "map(.)|unique_by(.)", &["-c"], "[1E+1000]\n"),
    ("[-1e1000]", "map(.)?", &["-c"], "[-1E+1000]\n"),
    ("[1e309]", "map(.)?", &["-c"], "[1E+309]\n"),
    (
        "[1.7976931348623157e309]",
        "map(.)?",
        &["-c"],
        "[1.7976931348623157E+309]\n",
    ),
    (
        "[1e1000,1,-1e400]",
        "map(.)|sort",
        &["-c"],
        "[-1E+400,1,1E+1000]\n",
    ),
    (r#"{"a":[1e1000]}"#, "map(.)?", &["-c"], "[[1E+1000]]\n"),
    ("[1e1000]", "map(.)?", &[], "[\n  1E+1000\n]\n"),
    ("[1e1000]", "map(.)?", &["--tab"], "[\n\t1E+1000\n]\n"),
    ("[1e308]", "map(.)?", &["-c"], "[1E+308]\n"),
    ("[1e-400]", "map(.)|sort", &["-c"], "[1E-400]\n"),
    (
        "[100000000000000000000]",
        "map(.)|sort",
        &["-c"],
        "[100000000000000000000]\n",
    ),
];

#[test]
fn test_an_out_of_range_literal_survives_an_owned_answer_6319() -> Result<()> {
    for &(input, filter, flags, expected) in ROWS {
        let mut args: Vec<&str> = flags.to_vec();
        args.push(filter);
        let (stdout, stderr, code) = run_jq_full(&args, Some(input))?;
        assert_eq!(code, 0, "{input} | {filter} {flags:?}: {stderr}");
        assert_eq!(stdout, expected, "{input} | {filter} {flags:?}");
    }
    Ok(())
}

/// A computed infinity is `DBL_MAX` text in jq, and was already in the CLI's own printer; the
/// owned streamer now agrees with it, so an infinity that reaches it prints the same.
#[test]
fn test_a_computed_infinity_prints_as_dbl_max_through_an_owned_answer_6319() -> Result<()> {
    for filter in [
        "[infinite, -infinite] | map(.)?",
        "[1e1000, -1e1000] | map(. + 0)?",
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(code, 0, "{filter}: {stderr}");
        assert_eq!(
            stdout, "[1.7976931348623157e+308,-1.7976931348623157e+308]\n",
            "{filter}"
        );
    }
    Ok(())
}

#[test]
fn test_a_sorted_computed_infinity_prints_as_dbl_max_6319() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-nc", "[infinite, -infinite] | map(.) | sort"], None)?;
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        stdout,
        "[-1.7976931348623157e+308,1.7976931348623157e+308]\n"
    );
    Ok(())
}

/// `--preserve-input` is this tool's own flag (jq has none): it echoes an overflowing literal as
/// written, like every other literal under it.
#[test]
fn test_preserve_input_echoes_an_out_of_range_literal_as_written_6319() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["--preserve-input", "-c", "map(.)|sort"],
        Some("[1e1000,-1e400,1e309]"),
    )?;
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, "[-1e400,1e309,1e1000]\n");
    Ok(())
}
