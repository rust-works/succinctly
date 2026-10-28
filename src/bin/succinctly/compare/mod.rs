//! `succinctly dev compare jq`: one reproducible comparison of `jq` and
//! `succinctly jq` (#5409).
//!
//! Phase 1 (this module): manifest, hermetic reference jq, `env.json`,
//! schema-versioned `results.json`, generated `report.md`, and the
//! interleaved timer with its output-identity gate. The `smoke` suite runs
//! end to end; the dimensions of the issue that are not built yet are listed
//! in the report as not implemented rather than silently absent.

mod env;
mod harness;
mod manifest;
mod report;

use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use harness::{Row, Tool, Verdict};
use manifest::{sha256_hex, Manifest};
use serde::Serialize;
use std::path::PathBuf;
use std::process::Command;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Dimension {
    Smoke,
}

#[derive(Debug, Parser)]
pub struct CompareJqArgs {
    /// Limit the run to these dimensions (default: all implemented)
    #[arg(long, value_enum)]
    dimension: Vec<Dimension>,

    /// Few repetitions; a smoke test of the harness itself
    #[arg(long, conflicts_with = "full")]
    quick: bool,

    /// Full repetition count
    #[arg(long)]
    full: bool,

    /// Output directory (default: data/compare/<run_id>)
    #[arg(long)]
    out: Option<PathBuf>,

    /// Build the pinned reference jq with scripts/compare-jq-build.sh if missing
    #[arg(long)]
    build_jq: bool,

    /// Compare against this jq binary instead of the manifest's build
    /// (its --version must still equal the primary reference)
    #[arg(long)]
    jq: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
pub struct DimensionStatus {
    pub name: &'static str,
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct Results {
    pub schema_version: u32,
    pub run_id: String,
    pub command: String,
    pub reps: usize,
    pub manifest_sha256: String,
    pub env: env::Env,
    pub dimensions: Vec<DimensionStatus>,
    pub smoke: Vec<Row>,
}

const PENDING: &[(&str, &str)] = &[
    (
        "A. Language & builtin coverage",
        "not implemented (phase 2)",
    ),
    (
        "B. Behavioural correctness (differential)",
        "not implemented (phase 3)",
    ),
    (
        "C. Input handling & validation",
        "not implemented (phase 3)",
    ),
    ("D. Performance", "not implemented (phase 4)"),
    ("E. Memory", "not implemented (phase 4)"),
    ("F. Robustness", "not implemented (phase 5)"),
    ("G. CLI & I/O compatibility", "not implemented (phase 2)"),
    ("H. Platform & operational", "not implemented (phase 6)"),
];

fn repo_root() -> Result<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if !out.status.success() {
        bail!("run from inside the succinctly checkout");
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

pub fn run(args: CompareJqArgs) -> Result<i32> {
    let root = repo_root()?;
    let (manifest, manifest_sha256) = Manifest::load(&root)?;
    let primary = manifest.primary().clone();

    let jq = args
        .jq
        .clone()
        .unwrap_or_else(|| Manifest::jq_path(&root, &primary));
    if !jq.exists() && args.jq.is_none() {
        if !args.build_jq {
            bail!(
                "pinned {} not found at {}; run scripts/compare-jq-build.sh {} (or pass --build-jq)",
                primary.id, jq.display(), primary.version
            );
        }
        let status = Command::new(root.join("scripts/compare-jq-build.sh"))
            .arg(&primary.version)
            .status()
            .context("running compare-jq-build.sh")?;
        if !status.success() {
            bail!("building {} failed", primary.id);
        }
    }
    let version = Command::new(&jq)
        .arg("--version")
        .output()
        .with_context(|| format!("running {}", jq.display()))?;
    let version = String::from_utf8_lossy(&version.stdout).trim().to_string();
    if version != format!("jq-{}", primary.version) {
        bail!(
            "{} reports `{version}`, the manifest pins jq-{}",
            jq.display(),
            primary.version
        );
    }

    let mut env = env::Env::collect(&jq, sha256_hex(&std::fs::read(&jq)?))?;
    let mut declared: Vec<&str> = manifest.succinctly.features.split(',').collect();
    let mut built = env.succinctly_features.clone();
    declared.sort_unstable();
    built.sort_unstable();
    if declared != built || manifest.succinctly.profile != env.succinctly_profile {
        env.invalid_reasons.push(format!(
            "build ({} / {}) differs from the manifest's ({} / {})",
            env.succinctly_profile,
            built.join(","),
            manifest.succinctly.profile,
            manifest.succinctly.features
        ));
    }
    let reps = if args.full { 21 } else { 5 };
    let reference = Tool {
        name: primary.id.clone(),
        program: jq.to_string_lossy().into(),
        prefix: vec![],
    };
    let subject = Tool {
        name: "succinctly jq".into(),
        program: std::env::current_exe()?.to_string_lossy().into(),
        prefix: vec!["jq".into()],
    };

    let want = |d: Dimension| args.dimension.is_empty() || args.dimension.contains(&d);
    let mut dimensions = vec![];
    let mut smoke = vec![];
    if want(Dimension::Smoke) {
        let text = std::fs::read_to_string(root.join(&manifest.queries.smoke))?;
        for q in harness::parse_queries(&text)? {
            smoke.push(harness::compare(&reference, &subject, &q, reps)?);
        }
        dimensions.push(DimensionStatus {
            name: "Smoke (harness check)",
            status: "implemented",
        });
    }
    for (name, status) in PENDING {
        dimensions.push(DimensionStatus { name, status });
    }

    let run_id = {
        let t = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        format!(
            "{t}_{}{}",
            env.git.commit_short,
            if env.git.dirty { "_dirty" } else { "" }
        )
    };
    let out_dir = args
        .out
        .clone()
        .unwrap_or_else(|| root.join("data/compare").join(&run_id));
    std::fs::create_dir_all(&out_dir)?;

    let results = Results {
        schema_version: SCHEMA_VERSION,
        run_id,
        command: format!(
            "./target/release/succinctly dev compare jq {}",
            if args.full { "--full" } else { "--quick" }
        ),
        reps,
        manifest_sha256,
        env,
        dimensions,
        smoke,
    };
    std::fs::write(
        out_dir.join("env.json"),
        serde_json::to_string_pretty(&results.env)?,
    )?;
    std::fs::write(
        out_dir.join("results.json"),
        serde_json::to_string_pretty(&results)?,
    )?;
    std::fs::write(out_dir.join("report.md"), report::render(&results))?;
    println!("wrote {}", out_dir.display());

    let differing: Vec<&str> = results
        .smoke
        .iter()
        .filter(|r| r.verdict == Verdict::Differs)
        .map(|r| r.id.as_str())
        .collect();
    if !differing.is_empty() {
        eprintln!("unlisted divergence(s): {}", differing.join(", "));
        return Ok(1);
    }
    Ok(0)
}
