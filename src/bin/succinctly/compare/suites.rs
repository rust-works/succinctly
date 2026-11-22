//! Dimension B: the upstream jq test files run as a differential (#5409).
//!
//! `jq.test`, `onig.test`, `man.test`, `optional.test`, `base64.test` and `manonig.test`
//! (all six files jq 1.7.1 ships) are vendored from the pinned source tarball
//! (sha256-pinned in the manifest). Each case runs through both tools; stdout and exit
//! status must be identical, and for a `%%FAIL` case so must the error messages (see
//! [`fail_messages_match`], #7710). The files' own expected outputs are not consulted: the
//! reference *is* the oracle, so a case the reference itself fails on still compares
//! like for like. A suite the manifest pins to the secondary reference (jq 1.8.2's own files,
//! #5622) runs the same way against that reference and is reported apart.

use super::exec::exec;
use super::harness::Tool;
use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    /// 1-based line of the case's first line in the file.
    pub line: usize,
    pub program: String,
    /// `None` for a `%%FAIL` (compile-error) case.
    pub input: Option<String>,
    /// A `%%FAIL IGNORE MSG` case: upstream's own runner checks only that it fails, so its
    /// message is not compared either.
    pub ignore_msg: bool,
}

/// Parse jq's test-file format: blank-line separated blocks of
/// `program / input / expected outputs`, or `%%FAIL[ ...] / program / message`.
pub fn parse(text: &str) -> Vec<Case> {
    let mut cases = vec![];
    let mut block: Vec<(usize, &str)> = vec![];
    let mut flush = |block: &mut Vec<(usize, &str)>| {
        if block.is_empty() {
            return;
        }
        let first = block[0];
        if first.1.starts_with("%%FAIL") {
            if let Some(&(_, program)) = block.get(1) {
                cases.push(Case {
                    line: first.0,
                    program: program.into(),
                    input: None,
                    ignore_msg: first.1.contains("IGNORE MSG"),
                });
            }
        } else if block.len() >= 2 {
            cases.push(Case {
                line: first.0,
                program: first.1.into(),
                input: Some(block[1].1.into()),
                ignore_msg: false,
            });
        }
        block.clear();
    };
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            flush(&mut block);
        } else if line.starts_with('#') && block.is_empty() {
            // a comment between cases
        } else {
            block.push((i + 1, line));
        }
    }
    flush(&mut block);
    cases
}

#[derive(Debug, Clone, Serialize)]
pub struct CaseRow {
    pub id: String,
    pub matches: bool,
    pub program: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SuiteSummary {
    pub suite: String,
    pub total: usize,
    pub matching: usize,
    pub rows: Vec<CaseRow>,
}

impl SuiteSummary {
    pub fn differing(&self) -> impl Iterator<Item = &CaseRow> {
        self.rows.iter().filter(|r| !r.matches)
    }
}

/// `only`: run just the cases at these line numbers.
pub fn run(
    suite: &str,
    text: &str,
    reference: &Tool,
    subject: &Tool,
    only: Option<&BTreeSet<String>>,
    cwd: &Path,
) -> Result<SuiteSummary> {
    let mut rows = vec![];
    for c in parse(text) {
        if only.is_some_and(|s| !s.contains(&c.line.to_string())) {
            continue;
        }
        let (args, stdin): (Vec<String>, String) = match &c.input {
            Some(input) => (vec!["-c".into(), c.program.clone()], format!("{input}\n")),
            None => (
                vec!["-n".into(), "-c".into(), c.program.clone()],
                String::new(),
            ),
        };
        let r = exec(reference, &args, &stdin, cwd)?;
        let s = exec(subject, &args, &stdin, cwd)?;
        rows.push(CaseRow {
            id: format!("{suite}:{}", c.line),
            matches: r.exit == s.exit
                && r.stdout == s.stdout
                && (c.input.is_some() || c.ignore_msg || fail_messages_match(&r.stderr, &s.stderr)),
            program: c.program,
        });
    }
    let matching = rows.iter().filter(|r| r.matches).count();
    Ok(SuiteSummary {
        suite: suite.into(),
        total: rows.len(),
        matching,
        rows,
    })
}

/// Whether a `%%FAIL` case's two error reports agree (#7710): the same `jq: ...` lines, each
/// message with the same text and the same file and line, and the same trailer. The source echo
/// and its padding are not compared, nor jq 1.8's `, column N` and caret line (its presentation of
/// every compile error, which `succinctly jq` does not follow). A reference message that is a
/// bison `syntax error, ...` matches any message at the same place: that wording is a function of
/// jq's parse state and is recorded as succinctly's own in `docs/compliance/jq/limitations.md`.
fn fail_messages_match(reference: &[u8], subject: &[u8]) -> bool {
    let messages = |stderr: &[u8]| -> Vec<(String, Option<String>)> {
        String::from_utf8_lossy(stderr)
            .lines()
            .filter(|line| line.starts_with("jq: "))
            .map(split_fail_location)
            .collect()
    };
    let (reference, subject) = (messages(reference), messages(subject));
    reference.len() == subject.len()
        && reference.iter().zip(&subject).all(|(r, s)| {
            r.1 == s.1
                && (r.0 == s.0
                    || (r.0.starts_with("jq: error: syntax error, ")
                        && s.0.starts_with("jq: error: ")))
        })
}

/// A report line split into its message and its ` at <where>, line <n>` location, with jq 1.8's
/// `, column <c>` dropped; a line with no location (the trailer, `module not found: m`) is whole.
fn split_fail_location(line: &str) -> (String, Option<String>) {
    let Some(body) = line.strip_suffix(':') else {
        return (line.to_string(), None);
    };
    let body = match body.rfind(", column ") {
        Some(at) if body[at + 9..].bytes().all(|b| b.is_ascii_digit()) => &body[..at],
        _ => body,
    };
    let Some(line_at) = body.rfind(", line ") else {
        return (line.to_string(), None);
    };
    match body[..line_at].rfind(" at ") {
        Some(at) if body[line_at + 7..].bytes().all(|b| b.is_ascii_digit()) => {
            (body[..at].to_string(), Some(body[at..].to_string()))
        }
        _ => (line.to_string(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cases_fail_blocks_and_comments() {
        let text = "# header\n\n.a\n{\"a\":1}\n1\n\n# between\n%%FAIL\n{(0):1}\njq: error\n\n[.[]|.+1]\n[1]\n[2]\n";
        let cs = parse(text);
        assert_eq!(cs.len(), 3);
        assert_eq!(
            (cs[0].line, cs[0].program.as_str(), cs[0].input.as_deref()),
            (3, ".a", Some("{\"a\":1}"))
        );
        assert_eq!(
            (cs[1].program.as_str(), cs[1].input.clone()),
            ("{(0):1}", None)
        );
        assert_eq!(cs[2].program, "[.[]|.+1]");
    }

    #[test]
    fn the_vendored_files_parse_to_the_expected_scale() {
        let jq = parse(include_str!(
            "../../../../comparison/jq/vendor/jq-1.7.1/jq.test"
        ));
        assert!(jq.len() > 400, "{}", jq.len());
        assert!(jq.iter().any(|c| c.input.is_none()), "no %%FAIL case found");
        assert!(
            parse(include_str!(
                "../../../../comparison/jq/vendor/jq-1.7.1/onig.test"
            ))
            .len()
                > 30
        );
    }

    /// #5622: the two small files the first four vendored did not cover, 24 cases between them.
    #[test]
    fn the_base64_and_manonig_files_are_vendored_whole() {
        let base64 = parse(include_str!(
            "../../../../comparison/jq/vendor/jq-1.7.1/base64.test"
        ));
        let manonig = parse(include_str!(
            "../../../../comparison/jq/vendor/jq-1.7.1/manonig.test"
        ));
        assert_eq!((base64.len(), manonig.len()), (7, 17));
    }

    fn sh(script: &str) -> Tool {
        Tool {
            name: "sh".into(),
            program: "sh".into(),
            prefix: vec!["-c".into(), script.into(), "sh".into()],
        }
    }

    /// The gate must be able to fail: differing output is a differing row.
    #[test]
    fn a_differing_tool_is_reported_per_case() {
        let text = ".\n1\n1\n";
        let same = run(
            "t",
            text,
            &sh("cat; echo ok"),
            &sh("cat; echo ok"),
            None,
            Path::new("."),
        )
        .unwrap();
        assert_eq!((same.total, same.matching), (1, 1));
        let diff = run(
            "t",
            text,
            &sh("cat; echo a"),
            &sh("cat; echo b"),
            None,
            Path::new("."),
        )
        .unwrap();
        assert_eq!(diff.differing().next().map(|r| r.id.as_str()), Some("t:1"));
    }

    /// #7710: a `%%FAIL` case compares the messages, so jq.test's `include "\(a)"; 0` (`Import
    /// path must be constant`) differs from a tool that calls it a bad escape, while the echo, jq
    /// 1.8's column and caret, and bison's own syntax-error wording do not count.
    #[test]
    fn a_fail_case_compares_the_messages_7710() {
        let jq171 = b"jq: error: Import path must be constant at <top-level>, line 1:\ninclude \"\\(a)\"; 0        \njq: 1 compile error\n";
        let jq182 = b"jq: error: Import path must be constant at <top-level>, line 1, column 9:\n    include \"\\(a)\"; 0\n            ^^^^^^\njq: 1 compile error\n";
        let before = b"jq: error: invalid escape sequence '\\(' at <top-level>, line 1:\ninclude \"\\(a)\"; 0         \njq: 1 compile error\n";
        let after = b"jq: error: Import path must be constant at <top-level>, line 1:\ninclude \"\\(a)\"; 0\njq: 1 compile error\n";
        assert!(fail_messages_match(jq171, after));
        assert!(fail_messages_match(jq182, after));
        assert!(!fail_messages_match(jq171, before));
        // A count, a line or a second message that differs is a difference.
        assert!(!fail_messages_match(jq171, b"jq: error: Import path must be constant at <top-level>, line 1:\njq: 2 compile errors\n"));
        assert!(!fail_messages_match(jq171, b"jq: error: Import path must be constant at <top-level>, line 2:\njq: 1 compile error\n"));
        // Bison's wording is not compared, its place is; a message that itself says ` at line`
        // keeps its own text.
        let bison = b"jq: error: syntax error, unexpected '}' (Unix shell quoting issues?) at <top-level>, line 1:\n}\njq: 1 compile error\n";
        assert!(fail_messages_match(bison, b"jq: error: unexpected character '}', expected expression at <top-level>, line 1:\n}\njq: 1 compile error\n"));
        assert!(!fail_messages_match(
            bison,
            b"jq: error: unexpected character '}' at <top-level>, line 3:\njq: 1 compile error\n"
        ));
        let escape = b"jq: error: Invalid escape at line 1, column 4 (while parsing '\"\\q\"') at <top-level>, line 1, column 2:\njq: 1 compile error\n";
        assert!(fail_messages_match(escape, b"jq: error: Invalid escape at line 1, column 4 (while parsing '\"\\q\"') at <top-level>, line 1:\njq: 1 compile error\n"));
        assert!(!fail_messages_match(escape, b"jq: error: Invalid escape at line 1, column 5 (while parsing '\"\\q\"') at <top-level>, line 1:\njq: 1 compile error\n"));
    }

    /// #7710: `%%FAIL IGNORE MSG` is upstream's own "the message does not matter".
    #[test]
    fn an_ignore_msg_case_compares_only_stdout_and_exit_7710() {
        let text = "%%FAIL IGNORE MSG\n}\njq: error\n\n%%FAIL\n}\njq: error\n";
        assert_eq!(
            parse(text).iter().map(|c| c.ignore_msg).collect::<Vec<_>>(),
            [true, false]
        );
        let summary = run(
            "t",
            text,
            &sh("echo 'jq: error: a at <top-level>, line 1:' >&2; exit 3"),
            &sh("echo 'jq: error: b at <top-level>, line 1:' >&2; exit 3"),
            None,
            Path::new("."),
        )
        .unwrap();
        assert_eq!(
            summary
                .differing()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["t:5"]
        );
    }

    #[test]
    fn only_runs_the_cases_at_the_listed_lines() {
        let text = ".\n1\n1\n\n.\n2\n2\n";
        let at = |lines: &[&str]| -> BTreeSet<String> {
            lines.iter().map(|s| (*s).to_string()).collect()
        };
        let tool = sh("cat; echo ok");
        let run_only = |only: &BTreeSet<String>| {
            run("t", text, &tool, &tool, Some(only), Path::new("."))
                .unwrap()
                .rows
                .into_iter()
                .map(|r| r.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(run_only(&at(&["5"])), ["t:5"]);
        assert_eq!(run_only(&at(&["2"])), Vec::<String>::new());
    }
}
