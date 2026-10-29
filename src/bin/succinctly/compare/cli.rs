//! Dimension G: CLI and I/O compatibility, plus the succinctly-only surface (#5409).
//!
//! A probe file is `id<TAB>area<TAB>args-as-JSON-array<TAB>stdin-as-JSON-string`
//! (`-` for no stdin), run in `comparison/jq/fixtures/` so `-f`, `--slurpfile`
//! and `-L` have files to read. Stdout and exit status must be identical;
//! stderr wording is the error-probe suite's job and is not compared.

use super::exec::exec;
use super::harness::Tool;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Probe {
    pub id: String,
    pub area: String,
    pub args: Vec<String>,
    pub stdin: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    Match,
    Differs,
    /// Succinctly-only surface the reference rejects, as intended.
    Extension,
    /// A probe listed as an extension that the reference accepts, or that
    /// succinctly rejects: the list is wrong.
    Unexpected,
}

#[derive(Debug, Clone, Serialize)]
pub struct CliRow {
    pub id: String,
    pub area: String,
    pub verdict: Verdict,
    pub reference_exit: Option<i32>,
    pub subject_exit: Option<i32>,
}

pub fn parse_probes(text: &str) -> Result<Vec<Probe>> {
    let mut out = vec![];
    for (n, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.splitn(4, '\t').collect();
        if cols.len() != 4 {
            bail!("probe line {}: expected 4 tab-separated columns", n + 1);
        }
        let args: Vec<String> = serde_json::from_str(cols[2]).with_context(|| {
            format!("probe line {}: args are not a JSON array of strings", n + 1)
        })?;
        let stdin = if cols[3] == "-" {
            String::new()
        } else {
            serde_json::from_str(cols[3])
                .with_context(|| format!("probe line {}: stdin is not a JSON string", n + 1))?
        };
        out.push(Probe {
            id: cols[0].into(),
            area: cols[1].into(),
            args,
            stdin,
        });
    }
    Ok(out)
}

/// `extension`: true for the succinctly-only file.
pub fn run(
    reference: &Tool,
    subject: &Tool,
    probes: &[Probe],
    extension: bool,
    cwd: &Path,
) -> Result<Vec<CliRow>> {
    let mut rows = vec![];
    for p in probes {
        let r = exec(reference, &p.args, &p.stdin, cwd)?;
        let s = exec(subject, &p.args, &p.stdin, cwd)?;
        let verdict = if extension {
            if r.exit != Some(0) && s.exit == Some(0) {
                Verdict::Extension
            } else {
                Verdict::Unexpected
            }
        } else if r.exit == s.exit && r.stdout == s.stdout {
            Verdict::Match
        } else {
            Verdict::Differs
        };
        rows.push(CliRow {
            id: p.id.clone(),
            area: p.area.clone(),
            verdict,
            reference_exit: r.exit,
            subject_exit: s.exit,
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Tool {
        Tool {
            name: "sh".into(),
            program: "sh".into(),
            prefix: vec!["-c".into(), script.into(), "sh".into()],
        }
    }

    fn probe() -> Vec<Probe> {
        parse_probes("p\tflag\t[\"-c\",\".\"]\t\"1\\n\"\n").unwrap()
    }

    #[test]
    fn parses_json_columns() {
        let p = &probe()[0];
        assert_eq!(
            (p.args.clone(), p.stdin.as_str()),
            (vec!["-c".into(), ".".into()], "1\n")
        );
        assert!(parse_probes("a\tb\tnot-json\t-\n").is_err());
        assert!(parse_probes("a\tb\n").is_err());
    }

    /// Both directions of the gate must be able to fail.
    #[test]
    fn differing_stdout_or_exit_is_a_difference() {
        let ok = sh("cat >/dev/null; echo 1");
        let other = sh("cat >/dev/null; echo 2");
        let bad_exit = sh("cat >/dev/null; echo 1; exit 2");
        let cwd = Path::new(".");
        assert_eq!(
            run(&ok, &ok, &probe(), false, cwd).unwrap()[0].verdict,
            Verdict::Match
        );
        assert_eq!(
            run(&ok, &other, &probe(), false, cwd).unwrap()[0].verdict,
            Verdict::Differs
        );
        assert_eq!(
            run(&ok, &bad_exit, &probe(), false, cwd).unwrap()[0].verdict,
            Verdict::Differs
        );
    }

    #[test]
    fn an_extension_must_be_rejected_by_the_reference_and_accepted_by_succinctly() {
        let accepts = sh("cat >/dev/null; echo 1");
        let rejects = sh("cat >/dev/null; exit 2");
        let cwd = Path::new(".");
        let v = |r: &Tool, s: &Tool| run(r, s, &probe(), true, cwd).unwrap()[0].verdict;
        assert_eq!(v(&rejects, &accepts), Verdict::Extension);
        assert_eq!(v(&accepts, &accepts), Verdict::Unexpected);
        assert_eq!(v(&rejects, &rejects), Verdict::Unexpected);
    }
}
