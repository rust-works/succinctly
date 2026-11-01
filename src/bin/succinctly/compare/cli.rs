//! Dimension G: CLI and I/O compatibility, plus the succinctly-only surface (#5409).
//!
//! A probe file is `id<TAB>area<TAB>args-as-JSON-array<TAB>stdin-as-JSON-string`
//! (`-` for no stdin) and an optional fifth column, a JSON object of environment variables
//! to set for both tools (`{"JQ_COLORS":"1;31"}`; `-` or absent for none), run in
//! `comparison/jq/fixtures/` so `-f`, `--slurpfile`
//! and `-L` have files to read. Stdout and exit status must be identical;
//! stderr wording is the error-probe suite's job and is not compared.

use super::exec::exec_env;
use super::harness::Tool;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Probe {
    pub id: String,
    pub area: String,
    pub args: Vec<String>,
    pub stdin: String,
    /// Variables set on top of the fixed environment (`LC_ALL=C`, no `JQ_COLORS`/`NO_COLOR`).
    pub env: Vec<(String, String)>,
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
        let cols: Vec<&str> = line.splitn(5, '\t').collect();
        if cols.len() < 4 {
            bail!(
                "probe line {}: expected 4 or 5 tab-separated columns",
                n + 1
            );
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
        let env: Vec<(String, String)> = match cols.get(4) {
            None | Some(&"-") => vec![],
            Some(text) => serde_json::from_str::<std::collections::BTreeMap<String, String>>(text)
                .with_context(|| {
                    format!("probe line {}: env is not a JSON object of strings", n + 1)
                })?
                .into_iter()
                .collect(),
        };
        out.push(Probe {
            id: cols[0].into(),
            area: cols[1].into(),
            args,
            stdin,
            env,
        });
    }
    Ok(out)
}

/// `extension`: true for the succinctly-only file. `only`: run just these probe ids.
pub fn run(
    reference: &Tool,
    subject: &Tool,
    probes: &[Probe],
    extension: bool,
    only: Option<&BTreeSet<String>>,
    cwd: &Path,
) -> Result<Vec<CliRow>> {
    let mut rows = vec![];
    for p in probes {
        if only.is_some_and(|s| !s.contains(&p.id)) {
            continue;
        }
        let r = exec_env(reference, &p.args, &p.stdin, cwd, &p.env)?;
        let s = exec_env(subject, &p.args, &p.stdin, cwd, &p.env)?;
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

    #[test]
    fn an_optional_fifth_column_sets_environment_variables() {
        let p = |line: &str| parse_probes(line).unwrap().remove(0).env;
        let none = Vec::<(String, String)>::new();
        assert_eq!(p("a\tb\t[]\t-\n"), none);
        assert_eq!(p("a\tb\t[]\t-\t-\n"), none);
        assert_eq!(
            p("a\tb\t[]\t-\t{\"LC_ALL\":\"de_DE.UTF-8\",\"JQ_COLORS\":\"1\"}\n"),
            [
                ("JQ_COLORS".to_string(), "1".to_string()),
                ("LC_ALL".to_string(), "de_DE.UTF-8".to_string())
            ]
        );
        assert!(parse_probes("a\tb\t[]\t-\tnot-json\n").is_err());
        assert!(parse_probes("a\tb\t[]\t-\t{\"K\":1}\n").is_err());
    }

    /// Both tools see the probe's variables, so a difference can only come from the tools.
    #[test]
    fn the_probe_environment_reaches_both_tools() {
        let echo = |tag: &str| sh(&format!("cat >/dev/null; echo {tag}${{JQ_COLORS-unset}}"));
        let same = |t: &Tool, line: &str| {
            run(
                t,
                t,
                &parse_probes(line).unwrap(),
                false,
                None,
                Path::new("."),
            )
            .unwrap()
            .remove(0)
            .verdict
        };
        let (a, b) = (echo("x"), echo("y"));
        let line = "p\tflag\t[]\t-\t{\"JQ_COLORS\":\"1\"}\n";
        assert_eq!(same(&a, line), Verdict::Match);
        let probes = parse_probes(line).unwrap();
        assert_eq!(
            run(&a, &b, &probes, false, None, Path::new(".")).unwrap()[0].verdict,
            Verdict::Differs
        );
    }

    /// Both directions of the gate must be able to fail.
    #[test]
    fn differing_stdout_or_exit_is_a_difference() {
        let ok = sh("cat >/dev/null; echo 1");
        let other = sh("cat >/dev/null; echo 2");
        let bad_exit = sh("cat >/dev/null; echo 1; exit 2");
        let cwd = Path::new(".");
        assert_eq!(
            run(&ok, &ok, &probe(), false, None, cwd).unwrap()[0].verdict,
            Verdict::Match
        );
        assert_eq!(
            run(&ok, &other, &probe(), false, None, cwd).unwrap()[0].verdict,
            Verdict::Differs
        );
        assert_eq!(
            run(&ok, &bad_exit, &probe(), false, None, cwd).unwrap()[0].verdict,
            Verdict::Differs
        );
    }

    /// The syntax-form file is data in the CLI probe format; a duplicate id would merge two
    /// ledger entries, and a missing area would drop a row from the report's table.
    #[test]
    fn the_checked_in_syntax_probes_parse_with_unique_ids_and_cover_every_form() {
        let probes =
            parse_probes(include_str!("../../../../comparison/jq/probes/syntax.tsv")).unwrap();
        assert!(probes.len() > 400, "{}", probes.len());
        let ids: std::collections::BTreeSet<&str> = probes.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids.len(), probes.len(), "duplicate probe id");
        let areas: std::collections::BTreeSet<&str> =
            probes.iter().map(|p| p.area.as_str()).collect();
        for a in [
            "operators",
            "alternative",
            "paths-slices",
            "construction",
            "control",
            "variables",
            "functions",
            "strings-formats",
            "syntax",
            "assignment",
            "modules",
        ] {
            assert!(areas.contains(a), "no probe in area {a}");
        }
        // the forms the issue names: `?//`, reduce, foreach, label, import, include, a format string
        let programs: String = probes.iter().flat_map(|p| p.args.iter()).cloned().collect();
        for form in [
            "?//",
            "reduce ",
            "foreach ",
            "label $",
            "import \"",
            "include \"",
            "@base64 \"",
        ] {
            assert!(programs.contains(form), "no probe uses `{form}`");
        }
    }

    /// The colour and locale probes need their environment: a row that lost it would run under the
    /// fixed `LC_ALL=C` with no `JQ_COLORS` and pass as a vacuous match (#5552).
    #[test]
    fn the_checked_in_cli_probes_have_unique_ids_and_set_the_environment_they_probe() {
        let probes =
            parse_probes(include_str!("../../../../comparison/jq/probes/cli.tsv")).unwrap();
        let ids: BTreeSet<&str> = probes.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids.len(), probes.len(), "duplicate probe id");
        let has = |p: &Probe, k: &str| p.env.iter().any(|(n, _)| n == k);
        let jqcolors: Vec<_> = probes
            .iter()
            .filter(|p| p.id.starts_with("jqcolors-"))
            .collect();
        assert!(jqcolors.len() >= 20, "{}", jqcolors.len());
        assert!(
            jqcolors.iter().all(|p| has(p, "JQ_COLORS")),
            "a jqcolors probe without JQ_COLORS"
        );
        assert!(probes
            .iter()
            .filter(|p| p.id.starts_with("nocolor-"))
            .all(|p| has(p, "NO_COLOR")));
        // `LC_ALL` set to the empty string is what hands the choice to the other variables: a
        // `locale-var-*` row that lost it would run under the base `LC_ALL=C` and match vacuously.
        assert!(probes
            .iter()
            .filter(|p| p.id.starts_with("locale-var-"))
            .all(|p| has(p, "LC_ALL")));
        let locale: Vec<_> = probes.iter().filter(|p| p.area == "locale").collect();
        assert!(locale.len() >= 100, "{}", locale.len());
        assert!(
            locale.iter().all(|p| [
                "LC_ALL",
                "LANG",
                "LC_CTYPE",
                "LC_TIME",
                "LC_COLLATE",
                "LC_NUMERIC"
            ]
            .iter()
            .any(|k| has(p, k))),
            "a locale probe that sets no locale variable"
        );
    }

    #[test]
    fn an_extension_must_be_rejected_by_the_reference_and_accepted_by_succinctly() {
        let accepts = sh("cat >/dev/null; echo 1");
        let rejects = sh("cat >/dev/null; exit 2");
        let cwd = Path::new(".");
        let v = |r: &Tool, s: &Tool| run(r, s, &probe(), true, None, cwd).unwrap()[0].verdict;
        assert_eq!(v(&rejects, &accepts), Verdict::Extension);
        assert_eq!(v(&accepts, &accepts), Verdict::Unexpected);
        assert_eq!(v(&rejects, &rejects), Verdict::Unexpected);
    }

    #[test]
    fn only_runs_the_listed_probe_ids() {
        let ok = sh("cat >/dev/null; echo 1");
        let cwd = Path::new(".");
        let listed: BTreeSet<String> = ["p".to_string()].into();
        let other: BTreeSet<String> = ["not-p".to_string()].into();
        assert_eq!(
            run(&ok, &ok, &probe(), false, Some(&listed), cwd)
                .unwrap()
                .len(),
            1
        );
        assert!(run(&ok, &ok, &probe(), false, Some(&other), cwd)
            .unwrap()
            .is_empty());
    }
}
