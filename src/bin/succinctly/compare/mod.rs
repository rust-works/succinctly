//! `succinctly dev compare jq`: one reproducible comparison of `jq` and
//! `succinctly jq` (#5409).
//!
//! Phase 1: manifest, hermetic reference jq, `env.json`,
//! schema-versioned `results.json`, generated `report.md`, and the
//! interleaved timer with its output-identity gate. The `smoke` suite runs
//! end to end; the dimensions of the issue that are not built yet are listed
//! in the report as not implemented rather than silently absent.
//!
//! Phase 2: dimension A (every builtin the reference lists, plus those new in
//! the secondary reference), G (CLI flags) and the succinctly-only surface,
//! checked against the `divergences.toml` ledger.

mod cli;
mod env;
mod exec;
mod features;
mod harness;
mod jsonconf;
mod ledger;
mod manifest;
mod perf;
mod report;
mod suites;

use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use harness::{Row, Tool, Verdict};
use manifest::{sha256_hex, Manifest};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Dimension {
    Smoke,
    /// A: language and builtin coverage
    Features,
    /// G: CLI flags and the succinctly-only surface
    Cli,
    /// B: the upstream jq test files, differentially
    Differential,
    /// C: JSON input conformance (JSONTestSuite)
    Input,
    /// D and E: wall time and peak memory, interleaved, output-gated
    Performance,
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

    /// Write docs/compare/jq-coverage.json and refresh the figures quoted in
    /// docs/reference/jq-language.md (commit them from a clean, idle machine)
    #[arg(long)]
    write_summary: bool,

    /// Performance: run only the queries whose id contains this text
    #[arg(long)]
    query: Option<String>,

    /// Time jq against itself instead of succinctly: the harness's own noise floor
    #[arg(long)]
    control: bool,

    /// Build the pinned reference jqs with scripts/compare-jq-build.sh if missing
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
    /// A, against the primary reference.
    pub features: Option<features::FeatureSummary>,
    /// A, the builtins new in the secondary reference, against it alone.
    pub since_secondary: Option<features::FeatureSummary>,
    pub secondary_note: Option<String>,
    /// G.
    pub cli: Vec<cli::CliRow>,
    /// Succinctly-only surface, not counted as coverage.
    pub extensions: Vec<cli::CliRow>,
    /// B, one summary per vendored upstream file.
    pub differential: Vec<suites::SuiteSummary>,
    /// C.
    pub input: Option<jsonconf::InputSummary>,
    /// D and E, one row per query × input.
    pub perf: Vec<perf::PerfRow>,
    pub perf_note: Option<String>,
    pub known_divergences: Vec<ledger::Entry>,
    pub ledger: ledger::Outcome,
}

const PENDING: &[(&str, &str)] = &[
    ("F. Robustness", "not implemented (phase 5)"),
    ("H. Platform & operational", "not implemented (phase 6)"),
];

fn build_reference(root: &std::path::Path, version: &str) -> Result<()> {
    let status = Command::new(root.join("scripts/compare-jq-build.sh"))
        .arg(version)
        .stdout(std::process::Stdio::null())
        .status()
        .context("running compare-jq-build.sh")?;
    if !status.success() {
        bail!("building jq-{version} failed");
    }
    Ok(())
}

/// The secondary reference's binary, or why it is not available.
fn secondary_tool(
    root: &std::path::Path,
    manifest: &Manifest,
    build: bool,
) -> std::result::Result<Tool, String> {
    let Some(r) = manifest.references.iter().find(|r| r.role == "secondary") else {
        return Err("the manifest names no secondary reference".into());
    };
    let path = Manifest::jq_path(root, r);
    if !path.exists() && build {
        build_reference(root, &r.version).map_err(|e| e.to_string())?;
    }
    if !path.exists() {
        return Err(format!(
            "{} not built (scripts/compare-jq-build.sh {}, or --build-jq)",
            r.id, r.version
        ));
    }
    let v = Command::new(&path)
        .arg("--version")
        .output()
        .map_err(|e| e.to_string())?;
    let v = String::from_utf8_lossy(&v.stdout).trim().to_string();
    if v != format!("jq-{}", r.version) {
        return Err(format!(
            "{} reports `{v}`, the manifest pins jq-{}",
            path.display(),
            r.version
        ));
    }
    Ok(Tool {
        name: r.id.clone(),
        program: path.to_string_lossy().into(),
        prefix: vec![],
    })
}

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
        build_reference(&root, &primary.version)?;
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
    let secondary = if want(Dimension::Features) || want(Dimension::Performance) {
        secondary_tool(&root, &manifest, args.build_jq)
    } else {
        Err("not needed".into())
    };
    let fixtures = root.join(&manifest.probes.fixtures);
    let ledger_path = root.join(&manifest.probes.divergences);
    let known = ledger::load(&ledger_path)?;
    let mut observed = BTreeSet::new();
    let mut run_suites: Vec<String> = vec![];
    let (mut feats, mut since, mut secondary_note) = (None, None, None);
    if want(Dimension::Features) {
        run_suites.push("features".into());
        let sum = features::run(&reference, &primary.id, &subject, None, &fixtures)?;
        for r in &sum.rows {
            if matches!(
                r.status,
                features::Status::Differs | features::Status::NotImplemented
            ) {
                observed.insert(format!("features:{}", r.id));
            }
        }
        match &secondary {
            Ok(sec) => {
                let before: BTreeSet<String> = features::inventory(&reference, &fixtures)?
                    .into_iter()
                    .collect();
                let added: BTreeSet<String> = features::inventory(sec, &fixtures)?
                    .into_iter()
                    .filter(|b| !before.contains(b))
                    .collect();
                let s = features::run(sec, &sec.name, &subject, Some(&added), &fixtures)?;
                run_suites.push("since-secondary".into());
                for r in &s.rows {
                    if matches!(
                        r.status,
                        features::Status::Differs | features::Status::NotImplemented
                    ) {
                        observed.insert(format!("since-secondary:{}", r.id));
                    }
                }
                since = Some(s);
            }
            Err(why) => secondary_note = Some(why.clone()),
        }
        feats = Some(sum);
        dimensions.push(DimensionStatus {
            name: "A. Language & builtin coverage",
            status:
                "implemented (builtins; syntax forms and the manual's sections are not yet probed)",
        });
    }
    let (mut cli_rows, mut ext_rows) = (vec![], vec![]);
    if want(Dimension::Cli) {
        run_suites.extend(["cli".into(), "extensions".into()]);
        let read = |p: &str| std::fs::read_to_string(root.join(p)).with_context(|| p.to_string());
        cli_rows = cli::run(
            &reference,
            &subject,
            &cli::parse_probes(&read(&manifest.probes.cli)?)?,
            false,
            &fixtures,
        )?;
        ext_rows = cli::run(
            &reference,
            &subject,
            &cli::parse_probes(&read(&manifest.probes.extensions)?)?,
            true,
            &fixtures,
        )?;
        for r in &cli_rows {
            if r.verdict == cli::Verdict::Differs {
                observed.insert(format!("cli:{}", r.id));
            }
        }
        for r in &ext_rows {
            if r.verdict == cli::Verdict::Unexpected {
                observed.insert(format!("extensions:{}", r.id));
            }
        }
        dimensions.push(DimensionStatus {
            name: "G. CLI & I/O compatibility",
            status: "implemented (flags, I/O, exit codes, numbers; colour and locale partially)",
        });
    }
    if args.write_summary {
        let Some(f) = &feats else {
            bail!("--write-summary needs the features dimension");
        };
        let cov = features::Coverage::from(f);
        let json = root.join("docs/compare/jq-coverage.json");
        std::fs::create_dir_all(json.parent().expect("has parent"))?;
        std::fs::write(&json, serde_json::to_string_pretty(&cov)? + "\n")?;
        let doc = root.join("docs/reference/jq-language.md");
        std::fs::write(
            &doc,
            features::patch_doc(&std::fs::read_to_string(&doc)?, &cov)?,
        )?;
    }
    let mut differential = vec![];
    if want(Dimension::Differential) {
        for suite in &manifest.suites {
            run_suites.push(suite.id.clone());
            let text = Manifest::read_pinned(&root, suite)?;
            let sum = suites::run(&suite.id, &text, &reference, &subject, &fixtures)?;
            for row in sum.differing() {
                observed.insert(row.id.clone());
            }
            differential.push(sum);
        }
        dimensions.push(DimensionStatus {
            name: "B. Behavioural correctness (differential)",
            status: "implemented (upstream test files; grammar fuzzing and the sweep scripts are not yet folded in)",
        });
    }
    let mut input = None;
    if want(Dimension::Input) {
        run_suites.push("json-validate".into());
        run_suites.push("json-output".into());
        let text = Manifest::read_pinned(&root, &manifest.json_suite)?;
        let sum = jsonconf::run(&text, &reference, &subject, &fixtures)?;
        for r in &sum.rows {
            if (r.expect == 'y' && !r.validate_accepts) || (r.expect == 'n' && r.validate_accepts) {
                observed.insert(format!("json-validate:{}", r.id));
            }
            if r.output_differs {
                observed.insert(format!("json-output:{}", r.id));
            }
        }
        input = Some(sum);
        dimensions.push(DimensionStatus {
            name: "C. Input handling & validation",
            status:
                "implemented (JSONTestSuite; BOM, NUL, depth and stream shapes are probed under G)",
        });
    }
    let (mut perf_rows, mut perf_note) = (vec![], None);
    if want(Dimension::Performance) {
        run_suites.push("perf".into());
        let inputs_dir = root.join("data/compare/inputs");
        let top = if args.full { "100mb" } else { "1mb" };
        let q = std::fs::read_to_string(root.join(&manifest.perf.queries))?;
        let mut queries = perf::parse_queries(&q)?;
        if let Some(f) = &args.query {
            queries.retain(|x| x.id.contains(f.as_str()));
            if queries.is_empty() {
                bail!("no performance query contains `{f}`");
            }
        }
        let planned = perf::plan(&queries, &manifest.inputs, top);
        let generator = std::env::current_exe()?;
        let mut tools = vec![
            if args.control {
                Tool {
                    name: format!("{} (control)", reference.name),
                    ..reference.clone()
                }
            } else {
                subject.clone()
            },
            reference.clone(),
        ];
        match &secondary {
            Ok(sec) => tools.push(sec.clone()),
            Err(why) => perf_note = Some(format!("no secondary column: {why}")),
        }
        for (query, input) in &planned {
            if let Some(spec) = input {
                perf::ensure_input(&generator, &inputs_dir, spec)?;
            }
            let big = input.is_some_and(|i| i.tier == "100mb");
            let mid = input.is_some_and(|i| i.tier == "10mb");
            let n = match (args.full, big, mid) {
                (false, ..) => 3,
                (true, true, _) => 3,
                (true, _, true) => 9,
                _ => 21,
            };
            let row = perf::run_row(&tools, query, *input, &inputs_dir, n)?;
            if row.gate == perf::Gate::Differs {
                observed.insert(format!("perf:{}@{}", row.query, row.input));
            }
            perf_rows.push(row);
        }
        dimensions.push(DimensionStatus {
            name: "D. Performance / E. Memory",
            status: "implemented (spawn timings and peak RSS on generated shapes; real-world corpus files, cachegrind and the memory-limit probes are not yet included)",
        });
    }
    let ledger_outcome = ledger::check(&known, &observed, &run_suites);
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
            "./target/release/succinctly dev compare jq {}{}",
            if args.full { "--full" } else { "--quick" },
            if args.control { " --control" } else { "" }
        ),
        reps,
        manifest_sha256,
        env,
        dimensions,
        smoke,
        features: feats,
        since_secondary: since,
        secondary_note,
        cli: cli_rows,
        extensions: ext_rows,
        differential,
        input,
        perf: perf_rows,
        perf_note,
        known_divergences: known,
        ledger: ledger_outcome,
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
    }
    for (what, ids) in [
        ("unlisted divergence(s)", &results.ledger.unlisted),
        ("listed but no longer diverging", &results.ledger.stale),
    ] {
        if !ids.is_empty() {
            eprintln!("{what}: {}", ids.join(", "));
        }
    }
    Ok(i32::from(
        !differing.is_empty() || !results.ledger.is_clean(),
    ))
}
