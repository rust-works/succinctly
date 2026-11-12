//! #7012: `--jq-extensions` reaches the text `eval(...)` runs. `eval` parses each string its
//! argument yields as a yq expression, and that text gets the same opt-in back into jq's surface
//! as the program that holds the call: a name yq's lexer rejects (`paths`, `limit`) is a parse
//! error in it by default, and runs with the flag. yq refuses those names in both positions, so
//! the flag rows are succinctly's extension (ADR-0018 rule 5); the no-flag rows are yq's own
//! answer, which this change must leave alone.

use anyhow::Result;

use crate::{run_yq_files, run_yq_stdin_with_stderr};

const DOC: &str = "s: paths\na: 4\n";

const BAD_EXPRESSION: &str = "Error: bad expression, please check expression syntax\n";

/// (filter, stdout with `--jq-extensions`).
const WITH_FLAG: &[(&str, &str)] = &[
    // The text names a gated builtin, written in the filter or read from the document.
    (r#"eval("paths")"#, "[\"s\"]\n[\"a\"]\n"),
    ("eval(.s)", "[\"s\"]\n[\"a\"]\n"),
    // The flag follows a nested `eval`.
    (r#"eval("eval(\"paths\")")"#, "[\"s\"]\n[\"a\"]\n"),
    // A builtin with arguments, so a parse that reaches the call at all is pinned.
    (r#"eval("limit(1; .a)")"#, "4\n"),
    // Path position: a write or a `del` through the text lands.
    (r#"eval("limit(1; .a)") = 5"#, "{\"s\":\"paths\",\"a\":5}\n"),
    (
        r#"eval("limit(1; .a, .s)") |= "x""#,
        "{\"s\":\"paths\",\"a\":\"x\"}\n",
    ),
    (r#"del(eval("limit(1; .a)"))"#, "{\"s\":\"paths\"}\n"),
    // Text with no gated name reads as it did.
    (r#"eval(".a")"#, "4\n"),
    (r#"eval(".a += 3")"#, "{\"s\":\"paths\",\"a\":7}\n"),
];

/// Filters whose `eval`'d text names a gated builtin: without the flag each is a parse error of
/// the text, as the same name is of the program (`paths` is not part of yq's syntax).
const WITHOUT_FLAG: &[&str] = &[
    r#"eval("paths")"#,
    "eval(.s)",
    r#"eval("eval(\"paths\")")"#,
    r#"eval("limit(1; .a)")"#,
    r#"eval("limit(1; .a)") = 5"#,
    r#"del(eval("limit(1; .a)"))"#,
];

#[test]
fn eval_text_reads_the_jq_surface_under_jq_extensions_7012() -> Result<()> {
    for (filter, want) in WITH_FLAG {
        let (out, err, code) =
            run_yq_stdin_with_stderr(filter, DOC, &["--jq-extensions", "-o=json", "-I=0"])?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*want, "", 0),
            "--jq-extensions {filter}"
        );
    }
    Ok(())
}

#[test]
fn eval_text_still_refuses_the_jq_surface_without_the_flag_7012() -> Result<()> {
    for filter in WITHOUT_FLAG {
        let (out, err, code) = run_yq_stdin_with_stderr(filter, DOC, &["-o=json", "-I=0"])?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            ("", BAD_EXPRESSION, 1),
            "{filter}"
        );
    }
    Ok(())
}

/// The program's own parse of a gated name is the stricter message the text's one stands in
/// for, so both the flag-less program and the flag-less text are refused, and only the flag
/// lifts either.
#[test]
fn the_program_and_the_eval_text_are_gated_alike_7012() -> Result<()> {
    let (_, err, code) = run_yq_stdin_with_stderr("paths", DOC, &["-o=json", "-I=0"])?;
    assert_eq!(code, 1);
    assert!(err.contains("pass --jq-extensions"), "{err}");
    let (out, _, code) = run_yq_stdin_with_stderr("paths", DOC, &["--jq-extensions", "-o=json"])?;
    assert_eq!(code, 0, "{out}");
    Ok(())
}

/// The flag is part of the parsed call, so every route that evaluates the program sees it:
/// `--arg` text, `--eval-all`, and an `--inplace` write through `eval`.
#[test]
fn the_flag_holds_on_the_other_routes_7012() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let one = dir.path().join("one.yaml");
    std::fs::write(&one, DOC)?;

    let (out, err, code) = run_yq_stdin_with_stderr(
        "eval($e)",
        DOC,
        &["--jq-extensions", "-o=json", "-I=0", "--arg", "e", "paths"],
    )?;
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        ("[\"s\"]\n[\"a\"]\n", "", 0),
        "--arg"
    );

    let (out, err, code) = run_yq_files(
        r#"eval("limit(1; .a)")"#,
        &[one.as_path()],
        &["--jq-extensions", "-o=json", "-I=0", "--eval-all"],
    )?;
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        ("4\n", "", 0),
        "--eval-all"
    );

    let (_, err, code) = run_yq_files(
        r#"eval("limit(1; .a)") = 9"#,
        &[one.as_path()],
        &["--jq-extensions", "-i"],
    )?;
    assert_eq!((err.as_str(), code), ("", 0), "--inplace");
    assert_eq!(std::fs::read_to_string(&one)?, "s: paths\na: 9\n");
    Ok(())
}
