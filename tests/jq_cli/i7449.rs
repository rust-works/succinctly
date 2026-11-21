//! #7449: a filter that branches on a malformed scalar token (`tru`, `nulll`, `x`, `1.2.3`, `-`,
//! `1e`) raises jq's parse error, as `type` on the same token already did. `not`, `if`, `and`,
//! `or`, `select`, `//`, `any` and `all` answered as if the token were truthy, so `[nul] | .[0] |
//! not` was `false` where `[null]` gives `true`.
//!
//! Every expected stdout, stderr and exit status was captured from jq 1.7.1 (`/usr/bin/jq`); jq
//! 1.8.2 answers the same for every row here. A string with an invalid escape is not part of this:
//! its truthiness is the documented #5308 leniency, pinned unchanged by the last test.

use anyhow::Result;

use crate::{run_jq_full, spawn_jq};

/// A token jq refuses, and whether jq calls it an `Invalid literal` (a word that starts like a
/// keyword) rather than an `Invalid numeric literal`.
const BAD: &[(&str, bool)] = &[
    ("tru", true),
    ("nulll", true),
    ("nul", true),
    ("fals", true),
    ("True", false),
    ("x", false),
    ("1.2.3", false),
    ("-", false),
    ("1e", false),
    ("+", false),
    ("1ee5", false),
];

/// A filter that branches on its input, and what jq prints for it on a truthy input (`0`) and on
/// `null` (one line each, joined by `\n`; empty for no output).
const CONSUMERS: &[(&str, &str, &str)] = &[
    ("not", "false", "true"),
    ("if . then 1 else 2 end", "1", "2"),
    (". and true", "true", "false"),
    (". or false", "true", "false"),
    ("true and .", "true", "false"),
    ("false or .", "true", "false"),
    ("select(.) | 1", "1", ""),
    ("(. // 7) | 1", "1", "1"),
    ("[.] | any", "true", "false"),
    ("[.] | all", "true", "false"),
    ("[.] | map(not)", "[false]", "[true]"),
    ("any(.; .)", "true", "false"),
    ("all(.; not)", "false", "true"),
    ("(not) as $x | 1", "1", "1"),
    ("[select(.)] | length", "1", "0"),
    ("if not then 1 else 2 end", "2", "1"),
    ("[. // 7] | length", "1", "1"),
];

/// Where the token sits: the text before and after it, the filter prefix that reaches it, and the
/// columns that moves it right by. `None` for the second line of a two-line stream whose first line
/// is `null`, which jq answers before it names the fault on line 3, column 0.
struct Context {
    name: &'static str,
    before: &'static str,
    after: &'static str,
    prefix: &'static str,
    shift: Option<usize>,
}

const CONTEXTS: &[Context] = &[
    Context {
        name: "top level",
        before: "",
        after: "",
        prefix: "",
        shift: Some(0),
    },
    Context {
        name: "array element",
        before: "[",
        after: "]",
        prefix: ".[0] | ",
        shift: Some(2),
    },
    Context {
        name: "object value",
        before: r#"{"a":"#,
        after: "}",
        prefix: ".a | ",
        shift: Some(6),
    },
    Context {
        name: "NDJSON line",
        before: "null\n",
        after: "\n",
        prefix: "",
        shift: None,
    },
];

fn lines(text: &str) -> String {
    if text.is_empty() {
        String::new()
    } else {
        format!("{text}\n")
    }
}

/// `input` as a file, run with `-c filter`: stdout, stderr and the exit status.
fn run_file(filter: &str, input: &[u8]) -> Result<(String, String, i32)> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.json");
    std::fs::write(&path, input)?;
    let name = path.to_string_lossy().into_owned();
    let (out, status) = spawn_jq(&["-c", filter, &name], None)?;
    Ok((
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        status,
    ))
}

#[test]
fn test_branching_on_a_malformed_scalar_token_raises_jqs_parse_error_7449() -> Result<()> {
    let mut failures = Vec::new();
    for &(token, keyword) in BAD {
        let kind = if keyword {
            "Invalid literal"
        } else {
            "Invalid numeric literal"
        };
        for context in CONTEXTS {
            let input = format!("{}{token}{}", context.before, context.after);
            let stderr = match context.shift {
                Some(0) => format!(
                    "jq: parse error: {kind} at EOF at line 1, column {}\n",
                    token.len()
                ),
                Some(shift) => format!(
                    "jq: parse error: {kind} at line 1, column {}\n",
                    token.len() + shift
                ),
                None => format!("jq: parse error: {kind} at line 3, column 0\n"),
            };
            for &(consumer, _, on_null) in CONSUMERS {
                let filter = format!("{}({consumer})", context.prefix);
                let stdout = if context.shift.is_none() {
                    lines(on_null)
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
                // A file reaches the same predicate; the two contexts without a prefix show it.
                if context.prefix.is_empty() {
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
    assert!(
        failures.is_empty(),
        "{} rows differ from jq:\n{}",
        failures.len(),
        failures.join("\n")
    );
    Ok(())
}

#[test]
fn test_branching_on_a_well_formed_scalar_is_unchanged_7449() -> Result<()> {
    let mut failures = Vec::new();
    for (token, truthy) in [
        ("true", true),
        ("false", false),
        ("null", false),
        ("0", true),
        ("-1", true),
        ("1e5", true),
        ("\"\"", true),
        ("\"\\u00e9\"", true),
        ("[]", true),
        ("{}", true),
    ] {
        for context in CONTEXTS {
            let input = format!("{}{token}{}", context.before, context.after);
            for &(consumer, on_truthy, on_null) in CONSUMERS {
                let filter = format!("{}({consumer})", context.prefix);
                // `false` and `null` answer alike for every consumer here.
                let answer = lines(if truthy { on_truthy } else { on_null });
                let stdout = if context.shift.is_none() {
                    format!("{}{answer}", lines(on_null))
                } else {
                    answer
                };
                let got = run_jq_full(&["-c", &filter], Some(&input))?;
                if got != (stdout, String::new(), 0) {
                    failures.push(format!("{} {input:?} | {filter}: {got:?}", context.name));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} rows differ from jq:\n{}",
        failures.len(),
        failures.join("\n")
    );
    Ok(())
}

/// A stream over 4 MiB, read from a file or from stdin, whose last line holds the malformed token:
/// jq answers every line before it, then names the fault.
#[test]
fn test_branching_on_a_malformed_token_ending_a_large_stream_raises_7449() -> Result<()> {
    const LINES: usize = 40_000;
    let line = format!("{{\"a\":true,\"pad\":\"{}\"}}\n", "x".repeat(100));
    let mut input = line.repeat(LINES).into_bytes();
    input.extend_from_slice(b"{\"a\":tru}\n");
    assert!(input.len() > 4 << 20);
    let stderr = format!(
        "jq: parse error: Invalid literal at line {}, column 9\n",
        LINES + 1
    );
    for (filter, each) in [
        (".a | not", "false\n"),
        ("select(.a) | 1", "1\n"),
        ("if .a then 1 else 2 end", "1\n"),
        (".a and true", "true\n"),
        ("(.a // 7) | 1", "1\n"),
    ] {
        let want = (each.repeat(LINES), stderr.clone(), 5);
        let (out, status) = spawn_jq(&["-c", filter], Some(input.as_slice()))?;
        let got = (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            status,
        );
        assert!(got == want, "stdin | {filter}: {:?}", (&got.1, got.2));
        let got = run_file(filter, &input)?;
        assert!(got == want, "file | {filter}: {:?}", (&got.1, got.2));
    }
    Ok(())
}

/// The documented #5308 leniency, unchanged: a string is truthy whatever its escapes spell, so
/// branching on `"\x"` answers (jq 1.7.1 rejects the document, as for the tokens above).
#[test]
fn test_branching_on_a_string_with_an_invalid_escape_still_answers_5308_7449() -> Result<()> {
    for (filter, want) in [
        (".[0] | not", "false\n"),
        ("if .[0] then 1 else 2 end", "1\n"),
        (".[0] and true", "true\n"),
        ("[.[] | select(.) | 1]", "[1]\n"),
    ] {
        let got = run_jq_full(&["-c", filter], Some(r#"["\x"]"#))?;
        assert_eq!(got, (want.to_string(), String::new(), 0), "{filter}");
    }
    Ok(())
}
