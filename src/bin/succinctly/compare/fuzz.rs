//! Dimension B: grammar-based differential fuzzing (#5552).
//!
//! `scripts/jq-grammar-fuzz.py` (#5696) generates jq programs and JSON documents from a typed
//! grammar under a fixed seed, runs both tools and compares exit status, stdout and the
//! normalized first line of stderr, then minimizes each distinct divergence. This module drives
//! it once per seed in the manifest and reads its `--json` result, so there is one generator and
//! one minimizer rather than a Rust port kept in step with the script.
//!
//! A finding is keyed `fuzz:p-<12 hex digits>` in the divergence ledger: the leading digits of
//! the SHA-256 of the minimized program, because a ledger id is a file path and a program is not
//! one (`comparison/jq/divergences/fuzz/p-<digits>.toml`). The program text is stable for a given
//! grammar and seed list, and the manifest says that changing either moves what is covered.

use super::harness::Tool;
use super::manifest::{sha256_hex, Fuzz};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

/// One tool's outcome on a program, as the script normalizes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub exit: Option<i32>,
    pub stdout: String,
    /// The first line of stderr with positions and values removed: only the fault's class.
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub seed: u64,
    /// `mismatch`, `ours-only` or `theirs-only`.
    pub kind: String,
    /// Programs of this seed that diverged the same way (the shown one is the smallest).
    pub members: u64,
    pub program: String,
    pub doc: String,
    pub reference: Option<Outcome>,
    pub subject: Option<Outcome>,
}

impl Finding {
    /// The ledger id: `fuzz:p-` and the first 12 hex digits of the program's SHA-256.
    pub fn id(&self) -> String {
        format!("fuzz:p-{}", &sha256_hex(self.program.as_bytes())[..12])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeedRun {
    pub seed: u64,
    pub programs: u64,
    /// Both tools gave the same answer, including the same error.
    pub agree: u64,
    /// Of those, the programs both tools answered with exit 0: the yield that counts.
    pub answered: u64,
    /// Programs jq does not compile, or either tool timed out: the generator's, not evidence.
    pub skipped: u64,
    /// Distinct divergence signatures (each is minimized).
    pub signatures: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Summary {
    pub runs: Vec<SeedRun>,
    /// Across seeds, one per distinct minimized program.
    pub findings: Vec<Finding>,
}

#[derive(Deserialize)]
struct Report {
    seed: u64,
    programs: u64,
    #[serde(default)]
    counts: BTreeMap<String, u64>,
    signatures: u64,
    #[serde(default)]
    findings: Vec<ReportFinding>,
}

#[derive(Deserialize)]
struct ReportFinding {
    kind: String,
    members: u64,
    program: String,
    doc: String,
    reference: Option<Outcome>,
    subject: Option<Outcome>,
}

/// Read the script's `--json` result for one seed.
pub fn parse_report(json: &str) -> Result<(SeedRun, Vec<Finding>)> {
    let r: Report = serde_json::from_str(json).context("the fuzzer's --json output")?;
    let count = |k: &str| r.counts.get(k).copied().unwrap_or(0);
    let run = SeedRun {
        seed: r.seed,
        programs: r.programs,
        agree: count("agree"),
        answered: count("agree-ok"),
        skipped: count("skip"),
        signatures: r.signatures,
    };
    let findings = r
        .findings
        .into_iter()
        .map(|f| Finding {
            seed: r.seed,
            kind: f.kind,
            members: f.members,
            program: f.program,
            doc: f.doc,
            reference: f.reference,
            subject: f.subject,
        })
        .collect();
    Ok((run, findings))
}

/// Whether a `python3` is on `PATH`.
pub fn python_available() -> bool {
    Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// One run of the script for `seed`. Exit 1 is the script's "found a divergence" and carries
/// the same JSON; anything else, or no JSON, is an error with the script's stderr.
fn run_seed(
    root: &Path,
    cfg: &Fuzz,
    reference: &Tool,
    subject: &Tool,
    seed: u64,
    programs: u32,
) -> Result<(SeedRun, Vec<Finding>)> {
    if !reference.prefix.is_empty() || subject.prefix != ["jq"] {
        bail!("the fuzzer runs `<jq> ...` against `<succinctly> jq ...`");
    }
    let out = Command::new("python3")
        // -I: nothing from the working directory or the user's site is imported.
        .arg("-I")
        .arg(root.join(&cfg.script))
        .args(["--bin", &subject.program, "--jq", &reference.program])
        .args(["-n", &programs.to_string()])
        .args(["--seed", &seed.to_string()])
        .args(["--depth", &cfg.depth.to_string()])
        .arg("--json")
        .current_dir(root)
        .output()
        .with_context(|| format!("running {}", cfg.script))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    match out.status.code() {
        Some(0 | 1) => parse_report(&stdout),
        code => bail!(
            "{} (seed {seed}) exited {code:?}: {}",
            cfg.script,
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
}

/// Run every seed of `cfg`. `--quick` does not shrink it: a ledgered finding has to be found
/// by every run, or the run that did not generate it would call its entry stale.
pub fn run(root: &Path, cfg: &Fuzz, reference: &Tool, subject: &Tool) -> Result<Summary> {
    let mut sum = Summary::default();
    let mut seen = BTreeSet::new();
    for &seed in &cfg.seeds {
        let (run, findings) = run_seed(root, cfg, reference, subject, seed, cfg.programs)?;
        sum.runs.push(run);
        for f in findings {
            if seen.insert(f.id()) {
                sum.findings.push(f);
            }
        }
    }
    Ok(sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = r#"{"seed":7,"programs":100,"depth":3,"signatures":1,
        "counts":{"agree":90,"agree-ok":40,"skip":6,"ours-only":4},
        "findings":[{"kind":"ours-only","members":4,"program":".a|tojson","doc":"{}",
          "reference":{"exit":5,"stdout":"","error":"jq: error: x"},
          "subject":{"exit":0,"stdout":"\"1\"\n","error":""}}]}"#;

    #[test]
    fn a_report_gives_the_counts_and_each_finding() {
        let (run, findings) = parse_report(REPORT).unwrap();
        assert_eq!(
            run,
            SeedRun {
                seed: 7,
                programs: 100,
                agree: 90,
                answered: 40,
                skipped: 6,
                signatures: 1
            }
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].id(), "fuzz:p-aae09b953011");
        assert_eq!(findings[0].seed, 7);
        assert_eq!(findings[0].reference.as_ref().unwrap().exit, Some(5));
        assert_eq!(findings[0].subject.as_ref().unwrap().stdout, "\"1\"\n");
    }

    #[test]
    fn a_report_without_findings_or_counts_is_a_clean_run() {
        let (run, findings) =
            parse_report(r#"{"seed":1,"programs":5,"signatures":0,"counts":{}}"#).unwrap();
        assert_eq!((run.agree, run.signatures), (0, 0));
        assert!(findings.is_empty());
    }

    #[test]
    fn text_that_is_not_the_report_is_an_error() {
        assert!(parse_report("seed=1 n=5 0 distinct divergence").is_err());
        assert!(parse_report("").is_err());
    }

    fn cfg(script: &str) -> Fuzz {
        Fuzz {
            script: script.into(),
            seeds: vec![1, 2],
            programs: 10,
            depth: 3,
        }
    }

    fn tools() -> (Tool, Tool) {
        let reference = Tool {
            name: "jq".into(),
            program: "jq".into(),
            prefix: vec![],
        };
        let subject = Tool {
            name: "succinctly jq".into(),
            program: "succinctly".into(),
            prefix: vec!["jq".into()],
        };
        (reference, subject)
    }

    /// A stand-in for the script that echoes its arguments into the report, so the test sees
    /// what the runner passed and that a finding shared by two seeds is kept once.
    fn fake_script(dir: &Path, exit: i32) -> String {
        let name = "fake-fuzz.py";
        std::fs::write(
            dir.join(name),
            format!(
                r#"import json, sys
a = sys.argv
n = int(a[a.index("-n") + 1]); seed = int(a[a.index("--seed") + 1])
print(json.dumps({{"seed": seed, "programs": n, "signatures": 1, "counts": {{"agree": n}},
  "findings": [{{"kind": "mismatch", "members": seed, "program": ".same", "doc": "1",
                 "reference": None, "subject": None}}]}}))
sys.exit({exit})
"#
            ),
        )
        .unwrap();
        name.into()
    }

    #[test]
    fn every_seed_runs_and_a_repeated_program_is_one_finding() {
        if !python_available() {
            eprintln!("python3 not found; skipping");
            return;
        }
        let dir = std::env::temp_dir().join(format!("compare-fuzz-a-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = fake_script(&dir, 1);
        let (r, s) = tools();
        let full = run(&dir, &cfg(&script), &r, &s).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            full.runs
                .iter()
                .map(|x| (x.seed, x.programs))
                .collect::<Vec<_>>(),
            [(1, 10), (2, 10)]
        );
        assert_eq!(full.findings.len(), 1, "the same program from two seeds");
        assert_eq!(full.findings[0].seed, 1, "the first seed to find it");
    }

    /// The gate must be able to fail: a script that crashes is an error carrying its stderr,
    /// not a clean run.
    #[test]
    fn a_script_that_fails_is_an_error() {
        if !python_available() {
            eprintln!("python3 not found; skipping");
            return;
        }
        let dir = std::env::temp_dir().join(format!("compare-fuzz-b-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("bad.py"),
            "import sys\nprint('boom', file=sys.stderr)\nsys.exit(2)\n",
        )
        .unwrap();
        let (r, s) = tools();
        let err = run(&dir, &cfg("bad.py"), &r, &s).unwrap_err().to_string();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            err.contains("exited Some(2)") && err.contains("boom"),
            "{err}"
        );
    }
}
