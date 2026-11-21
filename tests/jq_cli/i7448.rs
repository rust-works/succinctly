//! #7448: a field lookup on an object whose only member has no value (`{"a":}`, `{"a"}`,
//! `{"a":@}`, `{"a":,}`) or no key (`{:1}`) raises jq's parse error, as reading any other
//! malformed member does. It answered `null` (or `.a // 5`'s default) with exit 0, because the
//! lookup's walk pairs children two at a time and a lone child left it with no member to check.
//!
//! Every expected stdout, stderr and exit status was captured from jq 1.7.1 (`/usr/bin/jq`); jq
//! 1.8.2 answers the same for every row here.

use anyhow::Result;

use crate::{run_jq_full, spawn_jq};

/// A one-member object that is not `key: value`, jq's message for it, and the 1-based column jq
/// names when the object is the whole document.
const BAD: &[(&str, &str, usize)] = &[
    (r#"{"a":}"#, "Unmatched '}'", 6),
    (r#"{"b":}"#, "Unmatched '}'", 6),
    (r#"{"a": }"#, "Unmatched '}'", 7),
    (r#"{"a"}"#, "Objects must consist of key:value pairs", 5),
    (r#"{"a":@}"#, "Invalid numeric literal", 7),
    (r#"{"a":,}"#, "Expected value before ','", 6),
    (r#"{:1}"#, "Expected string key before ':'", 2),
];

/// A filter that reads the object's members by name, and what jq prints for it on `{}` and on
/// `{"a":1}`.
const CONSUMERS: &[(&str, &str, &str)] = &[
    (".a", "null", "1"),
    (".b", "null", "null"),
    (r#".["a"]"#, "null", "1"),
    (r#"getpath(["a"])"#, "null", "1"),
    ("path(.a)", r#"["a"]"#, r#"["a"]"#),
    (".a // 5", "5", "1"),
    (".a?", "null", "1"),
    (r#"try .a catch "E""#, "null", "1"),
    (".a as $x | 1", "1", "1"),
    ("[.a, .b]", "[null,null]", "[1,null]"),
    ("{x: .a}", r#"{"x":null}"#, r#"{"x":1}"#),
    (r#"has("a")"#, "false", "true"),
    (".a | type", r#""null""#, r#""number""#),
    ("del(.a)", "{}", "{}"),
    (".a = 1", r#"{"a":1}"#, r#"{"a":1}"#),
];

/// Where the object sits: text before and after it, the filter prefix that reaches it, the
/// columns that moves it right by, and whether it is the second line of a two-line stream whose
/// first line is `{}` (jq answers that line before it meets the fault).
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
        before: r#"{"x":"#,
        after: "}",
        prefix: ".x | ",
        shift: 5,
        second_line: false,
    },
    Context {
        name: "NDJSON line",
        before: "{}\n",
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

#[test]
fn test_a_lookup_on_a_one_member_object_without_a_value_raises_jqs_parse_error_7448() -> Result<()>
{
    let mut failures = Vec::new();
    for &(doc, message, column) in BAD {
        for context in CONTEXTS {
            let input = format!("{}{doc}{}", context.before, context.after);
            let line = if context.second_line { 2 } else { 1 };
            let stderr = format!(
                "jq: parse error: {message} at line {line}, column {}\n",
                column + context.shift
            );
            for &(consumer, on_empty, _) in CONSUMERS {
                let filter = format!("{}{consumer}", context.prefix);
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
                // A file reaches the same walk; one context of each kind is enough to show it.
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
    assert!(
        failures.is_empty(),
        "{} rows differ from jq:\n{}",
        failures.len(),
        failures.join("\n")
    );
    Ok(())
}

#[test]
fn test_a_lookup_on_a_well_formed_object_is_unchanged_7448() -> Result<()> {
    let mut failures = Vec::new();
    for (doc, pick) in [("{}", 0), (r#"{"a":1}"#, 1)] {
        for context in CONTEXTS {
            let input = format!("{}{doc}{}", context.before, context.after);
            for &(consumer, on_empty, on_a1) in CONSUMERS {
                let filter = format!("{}{consumer}", context.prefix);
                let answer = if pick == 0 { on_empty } else { on_a1 };
                let stdout = if context.second_line {
                    format!("{on_empty}\n{answer}\n")
                } else {
                    format!("{answer}\n")
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

/// A stream over 4 MiB, read from a file or from stdin, whose last line is the malformed object:
/// jq answers every line before it, then names the fault on the last line.
#[test]
fn test_a_lookup_on_a_one_member_object_ending_a_large_stream_raises_7448() -> Result<()> {
    const LINES: usize = 40_000;
    let line = format!("{{\"a\":1,\"pad\":\"{}\"}}\n", "x".repeat(100));
    let mut input = line.repeat(LINES).into_bytes();
    input.extend_from_slice(b"{\"a\":}\n");
    assert!(input.len() > 4 << 20);
    let stderr = format!(
        "jq: parse error: Unmatched '}}' at line {}, column 6\n",
        LINES + 1
    );
    for (filter, each) in [
        (".a", "1\n"),
        ("[.a, .b]", "[1,null]\n"),
        (".a // 5", "1\n"),
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
