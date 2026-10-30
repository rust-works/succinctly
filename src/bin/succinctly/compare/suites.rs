//! Dimension B: the upstream jq test files run as a differential (#5409).
//!
//! `jq.test`, `onig.test`, `man.test` and `optional.test` are vendored from the
//! pinned source tarball (sha256-pinned in the manifest). Each case runs through
//! both tools; stdout and exit status must be identical. The files' own expected
//! outputs are not consulted: the reference *is* the oracle, so a case the
//! reference itself fails on still compares like for like.

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
                });
            }
        } else if block.len() >= 2 {
            cases.push(Case {
                line: first.0,
                program: first.1.into(),
                input: Some(block[1].1.into()),
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
            matches: r.exit == s.exit && r.stdout == s.stdout,
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

    #[test]
    fn only_runs_the_cases_at_the_listed_lines() {
        let text = ".\n1\n1\n\n.\n2\n2\n";
        let at =
            |lines: &[&str]| -> BTreeSet<String> { lines.iter().map(|s| s.to_string()).collect() };
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
        assert!(run_only(&at(&["2"])).is_empty());
    }
}
