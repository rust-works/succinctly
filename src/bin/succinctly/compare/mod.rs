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
//! checked against the `divergences/` ledger.

mod cli;
mod docs;
mod env;
mod exec;
mod features;
mod fuzz;
mod harness;
mod jsonconf;
mod ledger;
mod manifest;
mod operational;
mod perf;
mod report;
mod robust;
mod suites;
mod verify;

use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use harness::{Row, Tool, Verdict};
use manifest::{sha256_hex, Manifest};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

pub const SCHEMA_VERSION: u32 = 1;

/// Where `--ledger-only` looks for a reference jq when the pinned build is absent.
const SYSTEM_JQ: &str = "/usr/bin/jq";

/// Whether a reference reporting `reported` stands in for the pinned `jq-<pinned>`. Only
/// `--ledger-only` (`loose`) accepts a vendor build of it: macOS ships `jq-1.7.1-apple`.
fn reference_version_ok(reported: &str, pinned: &str, loose: bool) -> bool {
    let want = format!("jq-{pinned}");
    reported == want || (loose && reported.starts_with(&format!("{want}-")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Dimension {
    Smoke,
    /// A: language and builtin coverage
    Features,
    /// G: CLI flags and the succinctly-only surface
    Cli,
    /// B: the upstream jq test files, differentially
    Differential,
    /// B: grammar-based differential fuzzing (`scripts/jq-grammar-fuzz.py`)
    Fuzz,
    /// C: JSON input conformance (JSONTestSuite)
    Input,
    /// D and E: wall time and peak memory, interleaved, output-gated
    Performance,
    /// F: deep nesting, runaway generators, huge values, SIGPIPE
    Robustness,
    /// H: binary size, libraries, dependencies, parallelism, licences
    Operational,
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

    /// Operational: also time a cold build of each tool from source, one run each (minutes: a
    /// fat-LTO release build of succinctly, twice, and both pinned jq builds)
    #[arg(long)]
    build_time: bool,

    /// Performance: also count each tool's instructions under `valgrind --tool=cachegrind`, a
    /// signal load cannot move (Linux with valgrind installed; rows over 1 MB are not counted)
    #[arg(long)]
    cachegrind: bool,

    /// Re-run the deterministic dimensions and spot-check timing rows against a
    /// recorded results.json; exit 1 on any difference
    #[arg(long)]
    verify: Option<PathBuf>,

    /// Timing rows --verify re-measures
    #[arg(long, default_value_t = 5)]
    spot: usize,

    /// Write docs/compare/jq-<platform>.{md,json} and the index docs/compare/jq.md
    /// (refused unless the run is complete and valid)
    #[arg(long)]
    write_docs: bool,

    /// Time jq against itself instead of succinctly: the harness's own noise floor
    #[arg(long)]
    control: bool,

    /// Build the pinned reference jqs with scripts/compare-jq-build.sh if missing
    #[arg(long)]
    build_jq: bool,

    /// Run only the probes the `divergences/` ledger lists for this OS, and report entries that no
    /// longer diverge (the stale half of the `Ledger drift` CI job, #5585). Needs no jq 1.8.1
    /// build and falls back to `/usr/bin/jq` when the pinned 1.7.1 is not built; skips
    /// `since-secondary` entries, entries for another OS and the perf/operational dimensions.
    /// An unlisted divergence is not found this way: that half stays in CI
    #[arg(
        long,
        conflicts_with_all = ["full", "verify", "write_docs", "write_summary", "control", "query"]
    )]
    ledger_only: bool,

    /// Compare against this jq binary instead of the manifest's build
    /// (its --version must still equal the primary reference)
    #[arg(long)]
    jq: Option<PathBuf>,

    /// Time and measure this succinctly binary instead of the harness itself. The harness needs
    /// `bench-runner`, which the shipped CLI does not carry (#5551), so build a plain `cli` binary
    /// and pass it here with `--subject-features`
    #[arg(long, requires = "subject_features")]
    subject: Option<PathBuf>,

    /// The cargo features `--subject` was built with, comma-separated (not detectable from a binary)
    #[arg(long, requires = "subject")]
    subject_features: Option<String>,
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
    /// A, syntax forms and the manual's sections.
    pub syntax: Vec<cli::CliRow>,
    /// G.
    pub cli: Vec<cli::CliRow>,
    /// Succinctly-only surface, not counted as coverage.
    pub extensions: Vec<cli::CliRow>,
    /// B, one summary per vendored upstream file.
    pub differential: Vec<suites::SuiteSummary>,
    /// B, the secondary reference's own test files against it (#5622). Informational: never
    /// counted in `differential`, never checked against the ledger.
    #[serde(default)]
    pub differential_secondary: Vec<suites::SuiteSummary>,
    /// Secondary-reference suites that could not run, with why (#5622).
    #[serde(default)]
    pub secondary_suites_skipped: Vec<String>,
    /// C.
    pub input: Option<jsonconf::InputSummary>,
    /// B, the grammar fuzzer's runs and minimized findings (#5552).
    #[serde(default)]
    pub fuzz: Option<fuzz::Summary>,
    /// D and E, one row per query × input.
    pub perf: Vec<perf::PerfRow>,
    pub perf_note: Option<String>,
    /// F.
    pub robust: Vec<robust::Row>,
    /// H.
    pub operational: Option<operational::Operational>,
    pub known_divergences: Vec<ledger::Entry>,
    pub ledger: ledger::Outcome,
}

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

/// The secondary reference a suite is pinned to, or `None` for a suite that runs against the
/// primary (#5622).
fn suite_reference<'a>(
    manifest: &'a Manifest,
    suite: &manifest::Suite,
) -> Option<&'a manifest::Reference> {
    let id = suite.reference.as_deref()?;
    manifest
        .references
        .iter()
        .find(|r| r.id == id && r.role == "secondary")
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

/// `valgrind`, for `--cachegrind`. It exists for Linux only (none for Apple Silicon), so
/// elsewhere, or without it, the flag is an error and not a silent no-op (#5552).
fn find_valgrind() -> Result<PathBuf> {
    if !cfg!(target_os = "linux") {
        bail!("--cachegrind needs valgrind, which runs on Linux only");
    }
    let works = Command::new("valgrind")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !works {
        bail!("--cachegrind needs a working `valgrind` on PATH (apt-get install valgrind)");
    }
    Ok(PathBuf::from("valgrind"))
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
    if args.cachegrind
        && (args.ledger_only
            || (!args.dimension.is_empty() && !args.dimension.contains(&Dimension::Performance)))
    {
        bail!(
            "--cachegrind counts instructions in the performance dimension, which this run skips"
        );
    }
    if (args.build_time || args.cachegrind) && args.verify.is_some() {
        bail!("--build-time and --cachegrind measure; --verify re-runs only the recorded sections");
    }
    if args.build_time
        && (args.ledger_only
            || (!args.dimension.is_empty() && !args.dimension.contains(&Dimension::Operational)))
    {
        bail!("--build-time times builds in the operational dimension, which this run skips");
    }
    let valgrind = args.cachegrind.then(find_valgrind).transpose()?;
    let root = repo_root()?;
    let (manifest, manifest_sha256) = Manifest::load(&root)?;
    let primary = manifest.primary().clone();

    let mut jq = args
        .jq
        .clone()
        .unwrap_or_else(|| Manifest::jq_path(&root, &primary));
    let pinned_jq = jq.clone();
    if args.ledger_only
        && args.jq.is_none()
        && !jq.exists()
        && std::path::Path::new(SYSTEM_JQ).exists()
    {
        jq = PathBuf::from(SYSTEM_JQ);
    }
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
    if !reference_version_ok(&version, &primary.version, args.ledger_only) {
        bail!(
            "{} reports `{version}`, the manifest pins jq-{}",
            jq.display(),
            primary.version
        );
    }
    if args.ledger_only && jq != pinned_jq {
        eprintln!(
            "ledger-only: reference is {version} at {}, not the pinned build ({}); an entry that \
             depends on the libm or on a 1.7.1 patch level can disagree with CI",
            jq.display(),
            pinned_jq.display()
        );
    }

    let subject_exe = match &args.subject {
        Some(p) => Some(
            p.canonicalize()
                .with_context(|| format!("--subject {}", p.display()))?,
        ),
        None => None,
    };
    let subject_build = subject_exe
        .as_deref()
        .zip(args.subject_features.as_deref())
        .map(|(exe, feats)| {
            (
                exe,
                feats
                    .split(',')
                    .map(|f| f.trim().to_string())
                    .collect::<Vec<_>>(),
            )
        });
    let mut env = env::Env::collect(&jq, sha256_hex(&std::fs::read(&jq)?), subject_build)?;
    let mut declared: Vec<String> = manifest
        .succinctly
        .features
        .split(',')
        .map(String::from)
        .collect();
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
        program: match &subject_exe {
            Some(p) => p.to_string_lossy().into(),
            None => std::env::current_exe()?.to_string_lossy().into(),
        },
        prefix: vec!["jq".into()],
    };

    let verifying = args.verify.is_some() && args.dimension.is_empty();
    let want = |d: Dimension| {
        if args.ledger_only {
            return matches!(
                d,
                Dimension::Features
                    | Dimension::Cli
                    | Dimension::Differential
                    | Dimension::Input
                    | Dimension::Robustness
            ) && (args.dimension.is_empty() || args.dimension.contains(&d));
        }
        if verifying {
            return !matches!(
                d,
                Dimension::Smoke
                    | Dimension::Performance
                    | Dimension::Operational
                    | Dimension::Fuzz
            );
        }
        args.dimension.is_empty() || args.dimension.contains(&d)
    };
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
    let secondary = if args.ledger_only {
        Err("--ledger-only checks against the primary reference alone".into())
    } else if want(Dimension::Features)
        || want(Dimension::Performance)
        || want(Dimension::Operational)
        || (want(Dimension::Differential)
            && manifest
                .suites
                .iter()
                .any(|s| suite_reference(&manifest, s).is_some()))
    {
        secondary_tool(&root, &manifest, args.build_jq)
    } else {
        Err("not needed".into())
    };
    let fixtures = root.join(&manifest.probes.fixtures);
    let ledger_path = root.join(&manifest.probes.divergences);
    let known = ledger::load(&ledger_path)?;
    // `--ledger-only` runs just the probes the ledger lists for this host.
    let only = |suite: &str| args.ledger_only.then(|| ledger::ids_in(&known, suite));
    let mut observed = BTreeSet::new();
    let mut run_suites: Vec<String> = vec![];
    let (mut feats, mut since, mut secondary_note) = (None, None, None);
    let mut syntax_rows = vec![];
    if want(Dimension::Features) {
        run_suites.extend(["features".into(), "syntax".into()]);
        let sum = features::run(
            &reference,
            &primary.id,
            &subject,
            only("features").as_ref(),
            &fixtures,
        )?;
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
        let text = std::fs::read_to_string(root.join(&manifest.probes.syntax))
            .with_context(|| manifest.probes.syntax.clone())?;
        syntax_rows = cli::run(
            &reference,
            &subject,
            &cli::parse_probes(&text)?,
            false,
            only("syntax").as_ref(),
            &fixtures,
        )?;
        for r in &syntax_rows {
            if r.verdict == cli::Verdict::Differs {
                observed.insert(format!("syntax:{}", r.id));
            }
        }
        dimensions.push(DimensionStatus {
            name: "A. Language & builtin coverage",
            status: "implemented (builtins and syntax forms; `import`/`include` beyond the library-path fixtures and the manual's prose are not probed)",
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
            only("cli").as_ref(),
            &fixtures,
        )?;
        ext_rows = cli::run(
            &reference,
            &subject,
            &cli::parse_probes(&read(&manifest.probes.extensions)?)?,
            true,
            only("extensions").as_ref(),
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
            status: "implemented (flags, I/O, exit codes, numbers; JQ_COLORS, NO_COLOR and the locale variables through per-probe environments)",
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
    let mut differential_secondary = vec![];
    let mut secondary_suites_skipped: Vec<String> = vec![];
    if want(Dimension::Differential) {
        for suite in &manifest.suites {
            if let Some(r) = suite_reference(&manifest, suite) {
                // The secondary's own files: informational, so not in `run_suites` (the ledger
                // never learns of them), not in `observed`, and never fatal: a file that cannot
                // run is named in the report instead. `--ledger-only` checks the primary alone.
                if args.ledger_only {
                    continue;
                }
                let ran = match &secondary {
                    Ok(sec) => Manifest::read_pinned(&root, suite)
                        .and_then(|text| {
                            suites::run(&suite.id, &text, sec, &subject, None, &fixtures)
                        })
                        .map_err(|e| e.to_string()),
                    Err(why) => Err(format!("{}: {why}", r.id)),
                };
                match ran {
                    Ok(sum) => differential_secondary.push(sum),
                    Err(why) => secondary_suites_skipped.push(format!("{}: {why}", suite.id)),
                }
                continue;
            }
            run_suites.push(suite.id.clone());
            let text = Manifest::read_pinned(&root, suite)?;
            let sum = suites::run(
                &suite.id,
                &text,
                &reference,
                &subject,
                only(&suite.id).as_ref(),
                &fixtures,
            )?;
            for row in sum.differing() {
                observed.insert(row.id.clone());
            }
            differential.push(sum);
        }
        dimensions.push(DimensionStatus {
            name: "B. Behavioural correctness (differential)",
            status: "implemented (upstream test files; the sweep scripts are not yet folded in; grammar fuzzing is the fuzz dimension)",
        });
    }
    let mut fuzz_summary = None;
    if want(Dimension::Fuzz) {
        if fuzz::python_available() {
            run_suites.push("fuzz".into());
            let sum = fuzz::run(&root, &manifest.fuzz, &reference, &subject)?;
            for f in &sum.findings {
                observed.insert(f.id());
            }
            dimensions.push(DimensionStatus {
                name: "B. Grammar-based differential fuzzing",
                status: "implemented (fixed seeds; findings are minimized and keyed by program)",
            });
            fuzz_summary = Some(sum);
        } else if args.dimension.contains(&Dimension::Fuzz) {
            bail!("the fuzz dimension runs scripts/jq-grammar-fuzz.py, and python3 was not found");
        } else {
            dimensions.push(DimensionStatus {
                name: "B. Grammar-based differential fuzzing",
                status: "not run (python3 was not found)",
            });
        }
    }
    let mut input = None;
    if want(Dimension::Input) {
        run_suites.push("json-validate".into());
        run_suites.push("json-output".into());
        let text = Manifest::read_pinned(&root, &manifest.json_suite)?;
        let listed = args.ledger_only.then(|| {
            let mut ids = ledger::ids_in(&known, "json-validate");
            ids.extend(ledger::ids_in(&known, "json-output"));
            ids
        });
        let sum = jsonconf::run(&text, &reference, &subject, listed.as_ref(), &fixtures)?;
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
    let mut robust_rows = vec![];
    if want(Dimension::Robustness) {
        run_suites.push("robust".into());
        let text = std::fs::read_to_string(root.join(&manifest.probes.robust))?;
        robust_rows = robust::run(
            &reference,
            &subject,
            &robust::parse_probes(&text)?,
            only("robust").as_ref(),
            &fixtures,
        )?;
        // A crash or hang of succinctly is listed even when jq does the same.
        for r in robust_rows.iter().filter(|r| {
            !r.matches || matches!(r.subject, robust::Class::Crash | robust::Class::Timeout)
        }) {
            observed.insert(format!("robust:{}", r.id));
        }
        dimensions.push(DimensionStatus {
            name: "F. Robustness",
            status: "implemented (nesting, recursion, generators, regex, large values, SIGPIPE, the memory-limit probes are not yet included)",
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
        // A fetched corpus file that is not on this machine drops its rows, and says so (only the
        // rows this run would have timed).
        let (planned, dropped): (Vec<_>, Vec<_>) = perf::plan(&queries, &manifest.inputs, top)
            .into_iter()
            // `map_or(true, ..)`: the crate's MSRV predates `Option::is_none_or`.
            .partition(|(_, i)| i.map_or(true, |i| perf::available(&root, i)));
        let missing: std::collections::BTreeSet<&str> = dropped
            .iter()
            .filter_map(|(_, i)| i.map(|i| i.id.as_str()))
            .collect();
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
        if !missing.is_empty() {
            let skipped = format!(
                "corpus files not on this machine, so their rows are not run (scripts/sync-bench-corpus.sh fetches them): {}",
                missing.iter().copied().collect::<Vec<_>>().join(", ")
            );
            perf_note = Some(match perf_note {
                Some(n) => format!("{n}; {skipped}"),
                None => skipped,
            });
        }
        // A query with no input (`startup`) runs here too, before any generator has.
        std::fs::create_dir_all(&inputs_dir)?;
        for (query, input) in &planned {
            if let Some(spec) = input {
                perf::ensure_input(&root, &generator, &inputs_dir, spec)?;
            }
            let big = input.is_some_and(|i| i.tier == "100mb");
            let mid = input.is_some_and(|i| i.tier == "10mb");
            let n = match (args.full, big, mid) {
                (false, ..) => 3,
                (true, true, _) => 3,
                (true, _, true) => 9,
                _ => 21,
            };
            // Under valgrind a 10 MB row is minutes per tool: only rows up to 1 MB are counted.
            let counted = valgrind
                .as_deref()
                .filter(|_| input.map_or(true, |i| i.tier == "1mb"));
            let row = perf::run_row_with(&tools, query, *input, &inputs_dir, n, counted)?;
            if row.gate == perf::Gate::Differs {
                observed.insert(format!("perf:{}@{}", row.query, row.input));
            }
            perf_rows.push(row);
        }
        if valgrind.is_some() {
            // A valgrind that could not run the tools must not read as a clean run with no table.
            let (mut wanted, mut got) = (0, 0);
            for r in perf_rows
                .iter()
                .filter(|r| r.gate == perf::Gate::Match && r.tools.len() > 1)
                .filter(|r| r.tier == "1mb" || r.tier == "startup")
            {
                wanted += 2;
                got += r
                    .tools
                    .iter()
                    .take(2)
                    .filter(|t| t.instructions.is_some())
                    .count();
            }
            if wanted > 0 && got == 0 {
                bail!("--cachegrind: valgrind gave no instruction count for any of the {wanted} tool runs it was asked to count");
            }
            if got < wanted {
                let note = format!(
                    "{} of {wanted} cachegrind counts are missing (valgrind could not run that tool, or it ran over 300 s)",
                    wanted - got
                );
                perf_note = Some(match perf_note {
                    Some(n) => format!("{n}; {note}"),
                    None => note,
                });
            }
        }
        dimensions.push(DimensionStatus {
            name: "D. Performance / E. Memory",
            status: if valgrind.is_some() {
                "implemented (spawn timings and peak RSS on generated shapes and the corpus's JSON files; cachegrind instruction counts for the rows up to 1 MB)"
            } else {
                "implemented (spawn timings and peak RSS on generated shapes and the corpus's JSON files; cachegrind instruction counts only with --cachegrind, on Linux)"
            },
        });
    }
    let mut operational_info = None;
    if want(Dimension::Operational) {
        let mut tools = vec![subject.clone(), reference.clone()];
        if let Ok(sec) = &secondary {
            tools.push(sec.clone());
        }
        operational_info = Some(operational::collect(
            &root,
            &tools,
            &manifest.succinctly.features,
            &std::env::current_exe()?,
            &manifest.inputs,
            &root.join("data/compare/inputs"),
            if args.build_time {
                let versions: Vec<String> = manifest
                    .references
                    .iter()
                    .map(|r| r.version.clone())
                    .collect();
                operational::measure_build_times(&root, &versions, &manifest.succinctly.features)
            } else {
                vec![]
            },
        )?);
        dimensions.push(DimensionStatus {
            name: "H. Platform & operational",
            status: if args.build_time {
                "implemented (sizes, libraries, dependencies, parallelism, licences, build time)"
            } else {
                "implemented (sizes, libraries, dependencies, parallelism, licences; build time only with --build-time)"
            },
        });
    }
    if args.ledger_only && run_suites.is_empty() {
        bail!(
            "--ledger-only ran no suite: --dimension names only dimensions it skips (smoke, \
             performance, operational, fuzz); the ledger is fed by features, cli, differential, input \
             and robustness"
        );
    }
    let ledger_outcome = ledger::check(&known, &observed, &run_suites);
    if args.ledger_only {
        let skipped = ledger::unchecked(&known, &run_suites);
        println!(
            "ledger-only: checked {} of {} ledger entries against {version}",
            known.len() - skipped.len(),
            known.len()
        );
        for (id, why) in skipped {
            println!("  not checked {id}: {why}");
        }
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
            "./target/release/succinctly dev compare jq {}{}{}{}{}",
            if args.full { "--full" } else { "--quick" },
            if args.control { " --control" } else { "" },
            if args.cachegrind { " --cachegrind" } else { "" },
            if args.build_time { " --build-time" } else { "" },
            match (&args.subject, &args.subject_features) {
                (Some(p), Some(f)) => format!(
                    " --subject {} --subject-features {f}",
                    p.strip_prefix(&root).unwrap_or(p).display()
                ),
                _ => String::new(),
            }
        ),
        reps,
        manifest_sha256,
        env,
        dimensions,
        smoke,
        features: feats,
        since_secondary: since,
        secondary_note,
        syntax: syntax_rows,
        cli: cli_rows,
        extensions: ext_rows,
        differential,
        differential_secondary,
        secondary_suites_skipped,
        input,
        fuzz: fuzz_summary,
        robust: robust_rows,
        operational: operational_info,
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
    if args.write_docs {
        write_docs(&root, &results, &args)?;
    }
    let mut verify_failures = vec![];
    if let Some(path) = &args.verify {
        verify_failures = run_verify(
            path,
            &results,
            &VerifyCtx {
                root: &root,
                manifest: &manifest,
                reference: &reference,
                subject: &subject,
                secondary: &secondary,
            },
            args.spot,
        )?;
        for f in &verify_failures {
            eprintln!("verify: {f}");
        }
        if verify_failures.is_empty() {
            println!("verify: recorded results reproduced");
        }
    }

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
        !differing.is_empty() || !results.ledger.is_clean() || !verify_failures.is_empty(),
    ))
}

struct VerifyCtx<'a> {
    root: &'a std::path::Path,
    manifest: &'a Manifest,
    reference: &'a Tool,
    subject: &'a Tool,
    secondary: &'a std::result::Result<Tool, String>,
}

/// Differences between a recorded run and this one: the deterministic sections
/// must be identical, and `spot` timing rows must land within their tolerance.
fn run_verify(
    path: &std::path::Path,
    fresh: &Results,
    ctx: &VerifyCtx,
    spot: usize,
) -> Result<Vec<String>> {
    let old: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
    )?;
    if old["schema_version"] != SCHEMA_VERSION {
        bail!(
            "{} has schema_version {}, this build writes {SCHEMA_VERSION}",
            path.display(),
            old["schema_version"]
        );
    }
    let new = serde_json::to_value(fresh)?;
    let mut failures = verify::diff(&verify::fingerprint(&old), &verify::fingerprint(&new));
    let queries = perf::parse_queries(&std::fs::read_to_string(
        ctx.root.join(&ctx.manifest.perf.queries),
    )?)?;
    let inputs_dir = ctx.root.join("data/compare/inputs");
    std::fs::create_dir_all(&inputs_dir)?;
    let generator = std::env::current_exe()?;
    let mut tools = vec![ctx.subject.clone(), ctx.reference.clone()];
    if let Ok(sec) = ctx.secondary {
        tools.push(sec.clone());
    }
    let recorded: Vec<&serde_json::Value> = old["perf"]
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    for (query, input) in verify::spot_rows(&old, spot) {
        let Some(q) = queries.iter().find(|q| q.id == query) else {
            failures.push(format!(
                "perf:{query}@{input} names a query this checkout does not have"
            ));
            continue;
        };
        let spec = ctx.manifest.inputs.iter().find(|i| i.id == input);
        if let Some(spec) = spec {
            if !perf::available(ctx.root, spec) {
                failures.push(format!(
                    "perf:{query}@{input} needs corpus file {}, which is not on this machine \
                     (scripts/sync-bench-corpus.sh fetches it)",
                    spec.corpus.as_deref().unwrap_or_default()
                ));
                continue;
            }
            perf::ensure_input(ctx.root, &generator, &inputs_dir, spec)?;
        }
        let row = perf::run_row(&tools, q, spec, &inputs_dir, 5)?;
        let now = serde_json::to_value(&row)?;
        let was = recorded
            .iter()
            .find(|r| r["query"] == query && r["input"] == input);
        let (Some(was), Some(n)) = (was, now["speedup_primary"].as_f64()) else {
            failures.push(format!("perf:{query}@{input} produced no timing"));
            continue;
        };
        let o = was["speedup_primary"].as_f64().unwrap_or(f64::NAN);
        if !verify::timing_agrees(o, n, verify::max_cv(was), verify::max_cv(&now)) {
            let basis = |min: bool| if min { "minimum" } else { "median" };
            let was_min = was["ratio_from_min"].as_bool().unwrap_or(false);
            let note = if was_min == row.ratio_from_min {
                String::new()
            } else {
                format!(
                    " (the recorded ratio is of the {}, this one of the {}: regenerate the report)",
                    basis(was_min),
                    basis(row.ratio_from_min)
                )
            };
            failures.push(format!(
                "perf:{query}@{input} speedup was {o:.2}x, now {n:.2}x{note}"
            ));
        }
    }
    Ok(failures)
}

fn write_docs(root: &std::path::Path, results: &Results, args: &CompareJqArgs) -> Result<()> {
    if !results.env.invalid_reasons.is_empty() {
        bail!(
            "--write-docs refused: this run is invalid ({})",
            results.env.invalid_reasons.join("; ")
        );
    }
    if !args.dimension.is_empty() || args.control || args.query.is_some() || args.verify.is_some() {
        bail!("--write-docs needs a complete run: no --dimension, --query, --control or --verify");
    }
    let dir = root.join("docs/compare");
    std::fs::create_dir_all(&dir)?;
    let p = docs::PLATFORM;
    std::fs::write(
        dir.join(format!("jq-{p}.md")),
        docs::sanitize(
            &format!("{}{}", docs::HEADER, report::render(results)),
            root,
        ),
    )?;
    std::fs::write(
        dir.join(format!("jq-{p}.json")),
        docs::sanitize(&(serde_json::to_string_pretty(results)? + "\n"), root),
    )?;
    let mut rows = vec![];
    for e in std::fs::read_dir(&dir)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(plat) = name
            .strip_prefix("jq-")
            .and_then(|n| n.strip_suffix(".json"))
        {
            if plat == "coverage" {
                continue;
            }
            rows.push((
                plat.to_string(),
                serde_json::from_str(&std::fs::read_to_string(e.path())?)?,
            ));
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    std::fs::write(dir.join("jq.md"), docs::index(&rows))?;
    println!("wrote docs/compare/jq-{p}.md, jq-{p}.json and jq.md");
    Ok(())
}

#[cfg(test)]
mod ledger_only_tests {
    use super::reference_version_ok;

    #[test]
    fn a_vendor_build_of_the_pinned_version_stands_in_only_for_ledger_only() {
        assert!(reference_version_ok("jq-1.7.1", "1.7.1", false));
        assert!(reference_version_ok("jq-1.7.1", "1.7.1", true));
        assert!(reference_version_ok("jq-1.7.1-apple", "1.7.1", true));
        assert!(!reference_version_ok("jq-1.7.1-apple", "1.7.1", false));
        // A different version, or a longer number, is never the pinned one.
        assert!(!reference_version_ok("jq-1.8.1", "1.7.1", true));
        assert!(!reference_version_ok("jq-1.7.10", "1.7.1", true));
        assert!(!reference_version_ok("jq-1.7", "1.7.1", true));
    }
}
