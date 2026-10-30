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
mod docs;
mod env;
mod exec;
mod features;
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

    let subject_exe = match &args.subject {
        Some(p) => Some(
            p.canonicalize()
                .with_context(|| format!("--subject {}", p.display()))?,
        ),
        None => None,
    };
    let subject_build = subject_exe.as_deref().zip(args.subject_features.as_deref()).map(
        |(exe, feats)| {
            (
                exe,
                feats.split(',').map(|f| f.trim().to_string()).collect::<Vec<_>>(),
            )
        },
    );
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
        if verifying {
            return !matches!(
                d,
                Dimension::Smoke | Dimension::Performance | Dimension::Operational
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
    let secondary = if want(Dimension::Features)
        || want(Dimension::Performance)
        || want(Dimension::Operational)
    {
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
    let mut robust_rows = vec![];
    if want(Dimension::Robustness) {
        run_suites.push("robust".into());
        let text = std::fs::read_to_string(root.join(&manifest.probes.robust))?;
        robust_rows = robust::run(
            &reference,
            &subject,
            &robust::parse_probes(&text)?,
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
            status: "implemented (nesting, recursion, generators, regex, large values, SIGPIPE; the memory-limit probes are not yet included)",
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
        // A query with no input (`startup`) runs here too, before any generator has.
        std::fs::create_dir_all(&inputs_dir)?;
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
        )?);
        dimensions.push(DimensionStatus {
            name: "H. Platform & operational",
            status: "implemented (sizes, libraries, dependencies, parallelism, licences; build time is not measured)",
        });
    }
    let ledger_outcome = ledger::check(&known, &observed, &run_suites);

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
            "./target/release/succinctly dev compare jq {}{}{}",
            if args.full { "--full" } else { "--quick" },
            if args.control { " --control" } else { "" },
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
        cli: cli_rows,
        extensions: ext_rows,
        differential,
        input,
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
            perf::ensure_input(&generator, &inputs_dir, spec)?;
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
            failures.push(format!(
                "perf:{query}@{input} speedup was {o:.2}x, now {n:.2}x"
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
