//! #7554, #7555: a reader of an array with a stray, doubled or missing `,` raises jq's parse
//! error exactly where the reader jq defines it as already raised:
//!
//! - bare `first` like `.[0]` (`def first: .[0];`): `[,1] | first` printed `1`, exit 0 (#7554);
//! - `all(f)`/`any(f)` like `all(.[]; f)`: a walk that reads every element read across the
//!   fault and answered (`[1 2] | all(. == 1)` was `false`, from the `2`), and arity-0
//!   `any`/`all` on the zero-member `{,}` answered where `{"a":1,}` raises (#7555).
//!
//! The rows under "unchanged" pin what stays as it was: well-formed input, the arity-0 array
//! scans #2692 decided read no element, an early decision that stops before the fault as
//! `.[]` stops (#2103).
//!
//! Every expected stdout, stderr and exit status was captured from jq 1.7.1 (`/usr/bin/jq`); jq
//! 1.8.2 answers the same for every fault row here.

use anyhow::Result;

use crate::{run_jq_full, spawn_jq};

/// A faulty array, jq's message for it, and the 1-based column jq names at the top level.
const ARRAYS: &[(&str, &str, usize)] = &[
    ("[,1]", "Expected value before ','", 2),
    ("[1,]", "Expected another array element", 4),
    ("[1 2]", "Expected separator between values", 5),
    ("[1,,2]", "Expected value before ','", 4),
    ("[true false]", "Expected separator between values", 12),
    ("[1,false,]", "Expected another array element", 10),
];

/// A reader of an array, and what jq prints for it on `[]` (the NDJSON context's first line).
const ARRAY_READERS: &[(&str, &str)] = &[
    ("first", "null"),
    (".[0]", "null"),
    ("first | type", r#""null""#),
    ("all(. < 5)", "true"),
    ("any(. > 5)", "false"),
    ("all(.[]; . < 5)", "true"),
    ("any(.[]; . > 5)", "false"),
    ("[all(. < 5), any(. > 5)]", "[true,false]"),
];

/// Where the container sits: text before and after it, the filter prefix that reaches it, the
/// columns that moves it right by, and whether it is the second line of a stream whose first
/// line is the empty container (jq answers that line before it meets the fault).
struct Context {
    name: &'static str,
    before: &'static str,
    after: &'static str,
    prefix: &'static str,
    shift: usize,
    second_line: bool,
}

const CONTEXTS: &[Context] = &[
    Context {
        name: "top level",
        before: "",
        after: "",
        prefix: "",
        shift: 0,
        second_line: false,
    },
    Context {
        name: "array element",
        before: "[",
        after: "]",
        prefix: ".[0] | ",
        shift: 1,
        second_line: false,
    },
    Context {
        name: "object value",
        before: r#"{"k":"#,
        after: "}",
        prefix: ".k | ",
        shift: 5,
        second_line: false,
    },
    Context {
        name: "NDJSON line",
        before: "EMPTY\n",
        after: "\n",
        prefix: "",
        shift: 0,
        second_line: true,
    },
];

/// `input` as a file, run with `-c filter`: stdout, stderr with the file's name as `<f>`, and the
/// exit status.
fn run_file(filter: &str, input: &[u8]) -> Result<(String, String, i32)> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.json");
    std::fs::write(&path, input)?;
    let name = path.to_string_lossy().into_owned();
    let (out, status) = spawn_jq(&["-c", filter, &name], None)?;
    Ok((
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).replace(&name, "<f>"),
        status,
    ))
}

/// Every fault x reader x context, on stdin and (top level and NDJSON) from a file: jq's parse
/// error with jq's column, exit 5, and only the answer to a preceding NDJSON line on stdout.
fn check_faults(
    faults: &[(&str, &str, usize)],
    readers: &[(&str, &str)],
    empty: &str,
) -> Result<Vec<String>> {
    let mut failures = Vec::new();
    for &(doc, message, column) in faults {
        for context in CONTEXTS {
            let before = context.before.replace("EMPTY", empty);
            let input = format!("{before}{doc}{}", context.after);
            let line = if context.second_line { 2 } else { 1 };
            let stderr = format!(
                "jq: parse error: {message} at line {line}, column {}\n",
                column + context.shift
            );
            for &(reader, on_empty) in readers {
                let filter = format!("{}{reader}", context.prefix);
                let stdout = if context.second_line {
                    format!("{on_empty}\n")
                } else {
                    String::new()
                };
                let want = (stdout, stderr.clone(), 5);
                let got = run_jq_full(&["-c", &filter], Some(&input))?;
                if got != want {
                    failures.push(format!(
                        "{} stdin {input:?} | {filter}: {got:?}",
                        context.name
                    ));
                }
                if context.shift == 0 {
                    let got = run_file(&filter, input.as_bytes())?;
                    if got != want {
                        failures.push(format!(
                            "{} file {input:?} | {filter}: {got:?}",
                            context.name
                        ));
                    }
                }
            }
        }
    }
    Ok(failures)
}

fn assert_none(failures: Vec<String>) {
    assert!(
        failures.is_empty(),
        "{} rows differ from jq:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn test_first_and_any_all_f_on_an_array_gap_fault_raise_as_index_0_does_7554_7555() -> Result<()> {
    assert_none(check_faults(ARRAYS, ARRAY_READERS, "[]")?);
    Ok(())
}

#[test]
fn test_any_all_on_a_zero_member_object_with_a_stray_comma_raise_7555() -> Result<()> {
    let faults = &[("{,}", "Expected value before ','", 2)];
    let readers = &[("any", "false"), ("all", "true"), ("any(. > 5)", "false")];
    assert_none(check_faults(faults, readers, "{}")?);
    // The control: with a member, the checked walk already raised.
    let got = run_jq_full(&["-c", "any"], Some(r#"{"a":1,}"#))?;
    let want = (
        String::new(),
        "jq: parse error: Expected another key-value pair at line 1, column 8\n".to_string(),
        5,
    );
    assert_eq!(got, want);
    Ok(())
}

/// The same faults after a 5 MiB first line, from a file (the windowed route) and from stdin:
/// jq skips the string, then names the fault on line 2.
#[test]
fn test_the_faults_raise_after_a_large_first_line_7554_7555() -> Result<()> {
    let pad = format!("\"{}\"\n", "p".repeat(5 << 20));
    let cases: &[(&str, &str, usize, &str)] = &[
        ("[,1]", "Expected value before ','", 2, "first"),
        ("[1 2]", "Expected separator between values", 5, "first"),
        ("[1,]", "Expected another array element", 4, "all(. < 5)"),
        (
            "[1 2]",
            "Expected separator between values",
            5,
            "any(. > 5)",
        ),
        ("{,}", "Expected value before ','", 2, "any"),
    ];
    for &(doc, message, column, reader) in cases {
        let input = format!("{pad}{doc}\n");
        let filter = format!("if type == \"string\" then empty else {reader} end");
        let want = (
            String::new(),
            format!("jq: parse error: {message} at line 2, column {column}\n"),
            5,
        );
        let (out, status) = spawn_jq(&["-c", &filter], Some(input.as_bytes()))?;
        let got = (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            status,
        );
        assert_eq!(got, want, "stdin {doc} | {reader}");
        assert_eq!(
            run_file(&filter, input.as_bytes())?,
            want,
            "file {doc} | {reader}"
        );
    }
    Ok(())
}

/// What stays as it was: well-formed input, and the documented answers on faulty input.
#[test]
fn test_well_formed_input_and_the_documented_answers_are_unchanged_7554() -> Result<()> {
    let rows: &[(&str, &str, &str)] = &[
        // Well-formed: what jq 1.7.1 prints.
        ("[1,2]", "first", "1"),
        ("[]", "first", "null"),
        ("null", "first", "null"),
        ("[[1,2]]", ".[0] | first", "1"),
        ("[1,2]", "all(. < 5)", "true"),
        ("[1,2]", "any(. > 5)", "false"),
        ("[1,2]", "all(. == 1)", "false"),
        ("{}", "any", "false"),
        ("{}", "all", "true"),
        (r#"{"a":true}"#, "any", "true"),
        // #2692: an arity-0 scan of an array reads no element, so it answers (pinned by
        // `test_any_all_validate_only_the_keys_they_resolve_2476_2692`); jq raises.
        ("[1,]", "any", "true"),
        ("[1 2]", "all", "true"),
        ("[,1]", "any", "true"),
        // #2103: a decision before the fault stops the walk, as `.[]` stops under
        // `any(.[]; f)`; jq raises.
        ("[1 2]", "any(. == 1)", "true"),
        ("[1 2]", "any(.[]; . == 1)", "true"),
        ("[1,false,]", "all(. == 1)", "false"),
    ];
    let mut failures = Vec::new();
    for &(doc, filter, out) in rows {
        let got = run_jq_full(&["-c", filter], Some(doc))?;
        if got != (format!("{out}\n"), String::new(), 0) {
            failures.push(format!("{doc} | {filter}: {got:?}"));
        }
    }
    assert_none(failures);
    Ok(())
}
