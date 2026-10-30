//! Dimension F: robustness and resource limits (#5409).
//!
//! Each probe is run through both tools under a deadline and classified as
//! `ok`, `error` (non-zero exit), `crash` (killed by a signal, or a Rust panic)
//! or `timeout`. Stdout and exit status must match the reference; the class is
//! reported regardless, so a crash that jq shares is still visible. Inputs that
//! would be megabytes on disk (deep nesting, a 10 MB string, a million keys)
//! are generated here from a `@gen:` spec rather than checked in.

use super::exec::{exec_within, Exec};
use super::harness::Tool;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct Probe {
    pub id: String,
    pub area: String,
    pub args: Vec<String>,
    pub stdin: String,
    /// `head` pipes the tool into `head -n1` to exercise SIGPIPE handling.
    pub wrap: Option<String>,
}

/// Expand a `@gen:<kind>:<n>` stdin spec.
pub fn generate(spec: &str) -> Result<String> {
    let mut it = spec.strip_prefix("@gen:").unwrap_or(spec).splitn(2, ':');
    let kind = it.next().unwrap_or_default();
    let n: usize = it
        .next()
        .unwrap_or_default()
        .parse()
        .context("@gen needs a count")?;
    Ok(match kind {
        "nested-array" => format!("{}{}", "[".repeat(n), "]".repeat(n)),
        "nested-object" => format!("{}1{}", "{\"a\":".repeat(n), "}".repeat(n)),
        "long-string" => format!("\"{}\"", "a".repeat(n)),
        "wide-object" => {
            let body: Vec<String> = (0..n).map(|i| format!("\"k{i}\":{i}")).collect();
            format!("{{{}}}", body.join(","))
        }
        "int-array" => format!(
            "[{}]",
            (0..n).map(|i| i.to_string()).collect::<Vec<_>>().join(",")
        ),
        other => bail!("unknown @gen kind `{other}`"),
    })
}

/// `id<TAB>area<TAB>args-as-JSON-array<TAB>stdin<TAB>wrap`, stdin being `-`,
/// a JSON string, or a `@gen:` spec; `wrap` is `-` or `head`.
pub fn parse_probes(text: &str) -> Result<Vec<Probe>> {
    let mut out = vec![];
    for (n, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() != 5 {
            bail!("robust line {}: expected 5 tab-separated columns", n + 1);
        }
        let stdin = match c[3] {
            "-" => String::new(),
            s if s.starts_with("@gen:") => generate(s)?,
            s => {
                serde_json::from_str(s).with_context(|| format!("robust line {}: stdin", n + 1))?
            }
        };
        out.push(Probe {
            id: c[0].into(),
            area: c[1].into(),
            args: serde_json::from_str(c[2])
                .with_context(|| format!("robust line {}: args", n + 1))?,
            stdin,
            wrap: (c[4] != "-").then(|| c[4].to_string()),
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Class {
    Ok,
    Error,
    Crash,
    Timeout,
}

pub fn classify(e: &Exec) -> Class {
    if e.timed_out {
        Class::Timeout
    } else if e.exit.is_none() || String::from_utf8_lossy(&e.stderr).contains("panicked at") {
        Class::Crash
    } else if e.exit == Some(0) {
        Class::Ok
    } else {
        Class::Error
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub id: String,
    pub area: String,
    pub matches: bool,
    pub reference: Class,
    pub subject: Class,
    pub reference_exit: Option<i32>,
    pub subject_exit: Option<i32>,
    pub reference_ms: f64,
    pub subject_ms: f64,
}

fn wrapped(tool: &Tool, wrap: Option<&str>) -> Tool {
    match wrap {
        Some("head") => Tool {
            name: tool.name.clone(),
            program: "bash".into(),
            prefix: [
                "-c",
                "set -o pipefail; \"$@\" | head -n1",
                "bash",
                tool.program.as_str(),
            ]
            .iter()
            .map(ToString::to_string)
            .chain(tool.prefix.iter().cloned())
            .collect(),
        },
        _ => tool.clone(),
    }
}

fn timed(tool: &Tool, p: &Probe, cwd: &Path) -> Result<(Exec, f64)> {
    let t = wrapped(tool, p.wrap.as_deref());
    let start = std::time::Instant::now();
    let e = exec_within(&t, &p.args, &p.stdin, cwd, DEADLINE)?;
    Ok((e, start.elapsed().as_secs_f64() * 1e3))
}

/// `only`: run just these probe ids.
pub fn run(
    reference: &Tool,
    subject: &Tool,
    probes: &[Probe],
    only: Option<&BTreeSet<String>>,
    cwd: &Path,
) -> Result<Vec<Row>> {
    let mut rows = vec![];
    for p in probes {
        if only.is_some_and(|s| !s.contains(&p.id)) {
            continue;
        }
        let (r, rms) = timed(reference, p, cwd)?;
        let (s, sms) = timed(subject, p, cwd)?;
        rows.push(Row {
            id: p.id.clone(),
            area: p.area.clone(),
            matches: r.exit == s.exit && r.stdout == s.stdout && !(r.timed_out ^ s.timed_out),
            reference: classify(&r),
            subject: classify(&s),
            reference_exit: r.exit,
            subject_exit: s.exit,
            reference_ms: rms,
            subject_ms: sms,
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(exit: Option<i32>, err: &str, timed_out: bool) -> Exec {
        Exec {
            exit,
            stdout: vec![],
            stderr: err.into(),
            timed_out,
        }
    }

    #[test]
    fn generators_build_the_documented_shapes() {
        assert_eq!(generate("@gen:nested-array:3").unwrap(), "[[[]]]");
        assert_eq!(
            generate("@gen:nested-object:2").unwrap(),
            "{\"a\":{\"a\":1}}"
        );
        assert_eq!(generate("@gen:long-string:2").unwrap(), "\"aa\"");
        assert_eq!(
            generate("@gen:wide-object:2").unwrap(),
            "{\"k0\":0,\"k1\":1}"
        );
        assert_eq!(generate("@gen:int-array:3").unwrap(), "[0,1,2]");
        assert!(generate("@gen:nope:1").is_err() && generate("@gen:long-string:x").is_err());
    }

    #[test]
    fn classes_separate_error_crash_and_timeout() {
        assert_eq!(classify(&e(Some(0), "", false)), Class::Ok);
        assert_eq!(classify(&e(Some(5), "", false)), Class::Error);
        assert_eq!(classify(&e(None, "", false)), Class::Crash);
        assert_eq!(
            classify(&e(Some(101), "thread 'main' panicked at x", false)),
            Class::Crash
        );
        assert_eq!(classify(&e(None, "", true)), Class::Timeout);
    }

    #[test]
    fn the_checked_in_probes_parse() {
        let text = include_str!("../../../../comparison/jq/probes/robust.tsv");
        assert!(parse_probes(text).unwrap().len() >= 30);
        assert!(parse_probes("a\tb\n").is_err());
    }

    fn sh(script: &str) -> Tool {
        Tool {
            name: "sh".into(),
            program: "sh".into(),
            prefix: vec!["-c".into(), script.into(), "sh".into()],
        }
    }

    /// The gate must be able to fail on a crash the reference does not have.
    #[test]
    fn a_subject_crash_is_a_mismatch_and_is_classified() {
        let probes = parse_probes("p\tx\t[]\t-\t-\n").unwrap();
        let ok = sh("echo 1");
        let crash = sh("kill -SEGV $$");
        let rows = run(&ok, &crash, &probes, None, Path::new(".")).unwrap();
        assert!(!rows[0].matches);
        assert_eq!(
            (rows[0].reference, rows[0].subject),
            (Class::Ok, Class::Crash)
        );
        assert!(run(&ok, &ok, &probes, None, Path::new(".")).unwrap()[0].matches);
    }

    #[test]
    fn only_runs_the_listed_probe_ids() {
        let probes = parse_probes("p\tx\t[]\t-\t-\n").unwrap();
        let ok = sh("echo 1");
        let none: BTreeSet<String> = ["other".to_string()].into();
        let one: BTreeSet<String> = ["p".to_string()].into();
        let cwd = Path::new(".");
        assert!(run(&ok, &ok, &probes, Some(&none), cwd).unwrap().is_empty());
        assert_eq!(run(&ok, &ok, &probes, Some(&one), cwd).unwrap().len(), 1);
    }

    #[test]
    fn head_wrap_turns_a_pipe_close_into_the_shell_status() {
        let probes = parse_probes("p\tpipe\t[]\t-\thead\n").unwrap();
        let writer = sh("yes");
        let rows = run(&writer, &writer, &probes, None, Path::new(".")).unwrap();
        assert!(rows[0].matches);
        assert_eq!(rows[0].subject_exit, Some(141));
    }
}
