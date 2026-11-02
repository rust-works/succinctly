//! Dimension B: the `scripts/jq-*-sweep.*` oracle sweeps, run as named suites (#5552).
//!
//! Each sweep was written for one fix and judges its own rows: it runs a grid of programs
//! through the build and the reference jq, holds its own list of the residuals it expects, and
//! exits 1 on anything else (a new divergence, or a listed residual that no longer shows). This
//! module runs the ones listed in `manifest.toml`'s `[[sweep]]` against the harness's subject and
//! reference and reads only that exit status, so a script keeps its own row model and the runner
//! adds what the scripts lack on their own: one place that runs them all, a deadline, a report,
//! and the divergence ledger.
//!
//! A diverging sweep is keyed `sweep:<id>`. That is coarser than a probe id: once listed, a
//! second divergence in the same sweep does not change the run's outcome. The script's own
//! output tail is kept in the report, so the entry's `reason` can name the rows it covers.

use super::exec::exec_with;
use super::harness::Tool;
use super::manifest::Sweep;
use anyhow::{bail, Result};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

/// Output lines kept from a sweep, from its end: the scripts print their summary last.
const TAIL_LINES: usize = 12;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Run {
    pub id: String,
    pub script: String,
    /// Exit 1: the script found a divergence it does not expect.
    pub diverges: bool,
    pub seconds: f64,
    /// The last lines of its stdout and stderr.
    pub tail: Vec<String>,
}

impl Run {
    pub fn ledger_id(&self) -> String {
        format!("sweep:{}", self.id)
    }
}

/// The interpreter for a script, from its extension.
fn interpreter(script: &str) -> Result<Vec<String>> {
    Ok(
        match Path::new(script).extension().and_then(|e| e.to_str()) {
            // -I: nothing from the working directory or the user's site is imported.
            Some("py") => vec!["python3".into(), "-I".into()],
            Some("sh") => vec!["bash".into()],
            _ => bail!("sweep script {script} is neither .py nor .sh"),
        },
    )
}

/// `{subject}` and `{jq}` in an argument or variable, replaced by the two binaries.
// The placeholders are the manifest's own syntax, not format arguments.
#[allow(clippy::literal_string_with_formatting_args)]
fn expand(text: &str, reference: &Tool, subject: &Tool) -> String {
    text.replace("{subject}", &subject.program)
        .replace("{jq}", &reference.program)
}

fn last_lines(out: &[u8], err: &[u8]) -> Vec<String> {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(out),
        String::from_utf8_lossy(err)
    );
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(TAIL_LINES)..]
        .iter()
        .map(|l| (*l).to_string())
        .collect()
}

/// Run one sweep. Exit 0 agrees and exit 1 diverges; any other exit, a signal or the deadline
/// is an error carrying the output's tail, never a clean run.
pub fn run_one(root: &Path, cfg: &Sweep, reference: &Tool, subject: &Tool) -> Result<Run> {
    if !reference.prefix.is_empty() || subject.prefix != ["jq"] {
        bail!("a sweep runs `<jq> ...` against `<succinctly> jq ...`");
    }
    let mut argv = interpreter(&cfg.script)?;
    argv.push(root.join(&cfg.script).to_string_lossy().into());
    argv.extend(cfg.args.iter().map(|a| expand(a, reference, subject)));
    let tool = Tool {
        name: cfg.id.clone(),
        program: argv.remove(0),
        prefix: argv,
    };
    // The scripts that take the oracle from the environment read one of these.
    let mut env = vec![
        ("JQ_ORACLE".to_string(), reference.program.clone()),
        ("JQ".to_string(), reference.program.clone()),
    ];
    env.extend(
        cfg.env
            .iter()
            .map(|(k, v)| (k.clone(), expand(v, reference, subject))),
    );
    let start = Instant::now();
    let e = exec_with(
        &tool,
        &[],
        "",
        root,
        Duration::from_secs(cfg.timeout_s),
        &env,
    )?;
    let seconds = start.elapsed().as_secs_f64();
    let tail = last_lines(&e.stdout, &e.stderr);
    // An uncaught Python exception also exits 1. Read as a divergence it would keep a ledgered
    // sweep "still diverging" while the script no longer runs at all.
    let crashed = String::from_utf8_lossy(&e.stderr).contains("Traceback (most recent call last)");
    match e.exit {
        Some(0 | 1) if !e.timed_out && !crashed => Ok(Run {
            id: cfg.id.clone(),
            script: cfg.script.clone(),
            diverges: e.exit == Some(1),
            seconds,
            tail,
        }),
        _ => bail!(
            "sweep {} ({}) {}:\n{}",
            cfg.id,
            cfg.script,
            if e.timed_out {
                format!("passed its {} s deadline", cfg.timeout_s)
            } else if crashed {
                "raised an exception".to_string()
            } else {
                format!("exited {:?}", e.exit)
            },
            tail.join("\n")
        ),
    }
}

/// Run every sweep, or only those in `only` (`--ledger-only`: the ids the ledger lists).
pub fn run(
    root: &Path,
    sweeps: &[Sweep],
    reference: &Tool,
    subject: &Tool,
    only: Option<&BTreeSet<String>>,
) -> Result<Vec<Run>> {
    sweeps
        .iter()
        // `map_or(true, ..)`: the crate's MSRV predates `Option::is_none_or`.
        .filter(|s| only.map_or(true, |ids| ids.contains(&s.id)))
        .map(|s| run_one(root, s, reference, subject))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn tools() -> (Tool, Tool) {
        let reference = Tool {
            name: "jq".into(),
            program: "/opt/jq".into(),
            prefix: vec![],
        };
        let subject = Tool {
            name: "succinctly jq".into(),
            program: "/opt/succinctly".into(),
            prefix: vec!["jq".into()],
        };
        (reference, subject)
    }

    fn sweep(id: &str, script: &str, args: &[&str]) -> Sweep {
        Sweep {
            id: id.into(),
            script: script.into(),
            args: args.iter().map(|a| (*a).to_string()).collect(),
            env: BTreeMap::new(),
            timeout_s: 30,
        }
    }

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("compare-sweep-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn placeholders_name_the_two_binaries() {
        let (r, s) = tools();
        assert_eq!(
            expand("--bin={subject} --jq {jq}", &r, &s),
            "--bin=/opt/succinctly --jq /opt/jq"
        );
    }

    #[test]
    fn the_interpreter_follows_the_extension() {
        assert_eq!(interpreter("a.py").unwrap(), ["python3", "-I"]);
        assert_eq!(interpreter("a.sh").unwrap(), ["bash"]);
        assert!(interpreter("a.rb").is_err());
        assert!(interpreter("a").is_err());
    }

    #[test]
    fn the_tail_is_the_last_non_blank_lines_of_both_streams() {
        let out: String = (1..=20).map(|i| format!("line {i}\n\n")).collect();
        let tail = last_lines(out.as_bytes(), b"err\n");
        assert_eq!(tail.len(), TAIL_LINES);
        assert_eq!(tail.first().unwrap(), "line 10");
        assert_eq!(tail.last().unwrap(), "err");
    }

    /// The script sees the expanded arguments and the oracle variables, and its exit status
    /// alone decides the verdict.
    #[test]
    fn exit_0_agrees_and_exit_1_diverges() {
        let d = dir("a");
        std::fs::write(
            d.join("s.sh"),
            "echo \"args=$* oracle=$JQ_ORACLE jq=$JQ x=$X\"\nexit \"$CODE\"\n",
        )
        .unwrap();
        let (r, s) = tools();
        let mut cfg = sweep("s", "s.sh", &["--bin", "{subject}"]);
        cfg.env.insert("X".into(), "{jq}".into());
        cfg.env.insert("CODE".into(), "0".into());
        let ok = run_one(&d, &cfg, &r, &s).unwrap();
        cfg.env.insert("CODE".into(), "1".into());
        let bad = run_one(&d, &cfg, &r, &s).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        assert!(!ok.diverges && bad.diverges);
        assert_eq!(bad.ledger_id(), "sweep:s");
        assert_eq!(
            ok.tail,
            ["args=--bin /opt/succinctly oracle=/opt/jq jq=/opt/jq x=/opt/jq"]
        );
    }

    /// The gate must be able to fail: a crash or a hang is an error with the output, not a pass.
    #[test]
    fn another_exit_or_the_deadline_is_an_error() {
        let d = dir("b");
        std::fs::write(d.join("crash.sh"), "echo boom\nexit 2\n").unwrap();
        std::fs::write(d.join("hang.sh"), "sleep 30\n").unwrap();
        std::fs::write(
            d.join("raise.sh"),
            "echo 'Traceback (most recent call last):' >&2\necho 'KeyError: x' >&2\nexit 1\n",
        )
        .unwrap();
        let (r, s) = tools();
        let crash = run_one(&d, &sweep("c", "crash.sh", &[]), &r, &s)
            .unwrap_err()
            .to_string();
        let raised = run_one(&d, &sweep("r", "raise.sh", &[]), &r, &s)
            .unwrap_err()
            .to_string();
        let mut hang = sweep("h", "hang.sh", &[]);
        hang.timeout_s = 1;
        let hang = run_one(&d, &hang, &r, &s).unwrap_err().to_string();
        let _ = std::fs::remove_dir_all(&d);
        assert!(
            crash.contains("exited Some(2)") && crash.contains("boom"),
            "{crash}"
        );
        // Exit 1 with a traceback is a crash, not the script's "diverges".
        assert!(
            raised.contains("raised an exception") && raised.contains("KeyError"),
            "{raised}"
        );
        assert!(hang.contains("1 s deadline"), "{hang}");
    }

    #[test]
    fn ledger_only_runs_just_the_listed_sweeps() {
        let d = dir("c");
        std::fs::write(d.join("ok.sh"), "exit 0\n").unwrap();
        let (r, s) = tools();
        let all = [sweep("a", "ok.sh", &[]), sweep("b", "ok.sh", &[])];
        let only: BTreeSet<String> = ["b".to_string()].into();
        let ran = run(&d, &all, &r, &s, Some(&only)).unwrap();
        let every = run(&d, &all, &r, &s, None).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(ran.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["b"]);
        assert_eq!(every.len(), 2);
    }

    #[test]
    fn a_subject_that_is_not_succinctly_jq_is_refused() {
        let (r, mut s) = tools();
        s.prefix = vec![];
        assert!(run_one(Path::new("."), &sweep("a", "a.sh", &[]), &r, &s).is_err());
    }
}
