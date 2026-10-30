//! Dimensions D and E: wall time and peak memory per query × input (#5409).
//!
//! Method (docs/guides/benchmarking.md § A/B): process-spawn timings, every tool
//! once per repetition with the order rotated so none is always the one that
//! starts warm (or loaded), min/median/MAD, and an output-identity gate before
//! any number is believed. A row whose output differs from the primary
//! reference is reported as a correctness row and gets no timing. Memory is read
//! from separate, untimed runs, so a wrapper's own spawn never lands in a
//! wall-clock sample. Peak RSS comes from `/usr/bin/time`; it counts the pages of
//! the mapped executable, so a 24 MiB binary reads several MiB above a 2.5 MiB one
//! with the same heap (#5578). Private memory (anonymous pages only) is read beside it.

use super::exec::{exec_within, PERF_DEADLINE};
use super::harness::{stats, Stats, Tool};
use super::manifest::sha256_hex;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

/// Tiers in ascending size; a query's `max_tier` caps how far up it runs.
pub const TIERS: &[&str] = &["1mb", "10mb", "100mb"];

#[derive(Debug, Clone, Deserialize)]
pub struct InputSpec {
    pub id: String,
    pub shape: String,
    pub tier: String,
    /// `succinctly json generate -p`.
    pub pattern: String,
    pub size: String,
    pub seed: u64,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct PerfQuery {
    pub id: String,
    pub flags: Vec<String>,
    pub filter: String,
    /// Shapes the query applies to; `none` runs it with no input file.
    pub shapes: Vec<String>,
    pub max_tier: String,
}

/// `id<TAB>flags<TAB>filter<TAB>shape,shape<TAB>max_tier` (`-` for no flags).
pub fn parse_queries(text: &str) -> Result<Vec<PerfQuery>> {
    let mut out = vec![];
    for (n, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() != 5 {
            bail!(
                "perf query line {}: expected 5 tab-separated columns",
                n + 1
            );
        }
        if !TIERS.contains(&c[4]) {
            bail!("perf query line {}: unknown tier `{}`", n + 1, c[4]);
        }
        out.push(PerfQuery {
            id: c[0].into(),
            flags: if c[1] == "-" {
                vec![]
            } else {
                c[1].split(' ').map(String::from).collect()
            },
            filter: c[2].into(),
            shapes: c[3].split(',').map(String::from).collect(),
            max_tier: c[4].into(),
        });
    }
    Ok(out)
}

fn tier_rank(t: &str) -> usize {
    TIERS.iter().position(|x| *x == t).unwrap_or(usize::MAX)
}

/// The (query, input) rows to run at `tiers` (inclusive upper bound).
pub fn plan<'a>(
    queries: &'a [PerfQuery],
    inputs: &'a [InputSpec],
    top_tier: &str,
) -> Vec<(&'a PerfQuery, Option<&'a InputSpec>)> {
    let mut rows = vec![];
    for q in queries {
        for shape in &q.shapes {
            if shape == "none" {
                rows.push((q, None));
                continue;
            }
            for i in inputs.iter().filter(|i| &i.shape == shape) {
                if tier_rank(&i.tier) <= tier_rank(top_tier).min(tier_rank(&q.max_tier)) {
                    rows.push((q, Some(i)));
                }
            }
        }
    }
    rows
}

/// Generate `spec` into `dir` if absent and verify its sha256, so a changed
/// generator is a failure and not a silently different workload.
pub fn ensure_input(generator: &Path, dir: &Path, spec: &InputSpec) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.json", spec.id));
    if !path.exists() {
        let status = Command::new(generator)
            .args([
                "json",
                "generate",
                &spec.size,
                "-p",
                &spec.pattern,
                "-s",
                &spec.seed.to_string(),
                "-o",
            ])
            .arg(&path)
            .stdout(Stdio::null())
            .status()
            .context("running `json generate`")?;
        if !status.success() {
            bail!("generating {} failed", spec.id);
        }
    }
    let got = sha256_hex(&std::fs::read(&path)?);
    if got != spec.sha256 {
        bail!(
            "{} has sha256 {got}, the manifest pins {} (the generator or a stale file changed; \
             delete {} to regenerate, or re-pin on purpose)",
            spec.id,
            spec.sha256,
            path.display()
        );
    }
    Ok(path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Gate {
    Match,
    Differs,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolResult {
    pub tool: String,
    pub stats: Option<Stats>,
    /// Standard deviation over mean, in percent.
    pub cv_pct: Option<f64>,
    pub peak_rss_kb: Option<u64>,
    /// Private (anonymous) memory in KiB, without the file-backed pages of the mapped
    /// executable and libraries that peak RSS charges (#5578). Linux: the largest
    /// `RssAnon` seen while the process ran (sampled, so a lower bound for a run too
    /// short to sample twice). macOS: `peak memory footprint`. `None` elsewhere.
    pub private_kb: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PerfRow {
    pub query: String,
    pub input: String,
    pub tier: String,
    /// Subject vs the primary reference. A differing row is untimed.
    pub gate: Gate,
    /// Subject vs the secondary reference, when there is one.
    pub gate_secondary: Option<Gate>,
    pub tools: Vec<ToolResult>,
    /// Median ratios (reference ÷ subject); `None` unless that pair's output matched.
    pub speedup_primary: Option<f64>,
    pub speedup_secondary: Option<f64>,
    /// Secondary ÷ primary median, so a change in the reference's own speed is visible.
    pub secondary_over_primary: Option<f64>,
    pub rss_ratio_primary: Option<f64>,
    /// The same ratio over private memory (`ToolResult::private_kb`).
    pub private_ratio_primary: Option<f64>,
    /// Any tool's CV exceeds 5%.
    pub noisy: bool,
}

fn args_for(q: &PerfQuery, file: Option<&str>) -> Vec<String> {
    let mut a = q.flags.clone();
    a.push(q.filter.clone());
    a.extend(file.map(String::from));
    a
}

fn time_once(tool: &Tool, args: &[String], cwd: &Path) -> Result<f64> {
    let start = Instant::now();
    let status = Command::new(&tool.program)
        .args(&tool.prefix)
        .args(args)
        .current_dir(cwd)
        .env("TZ", "UTC")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("spawning {}", tool.name))?;
    let ms = start.elapsed().as_secs_f64() * 1e3;
    let _ = status;
    Ok(ms)
}

/// Peak resident set size in KiB from `/usr/bin/time`'s report (`-l` on macOS, `-v` on Linux).
pub fn parse_rss_kb(report: &str) -> Option<u64> {
    for line in report.lines() {
        let l = line.trim();
        if let Some(v) = l.strip_suffix("maximum resident set size") {
            return v.trim().parse::<u64>().ok().map(|b| b / 1024);
        }
        if let Some(v) = l.strip_prefix("Maximum resident set size (kbytes):") {
            return v.trim().parse().ok();
        }
    }
    None
}

/// macOS `/usr/bin/time -l` prints the peak memory footprint (dirty and compressed pages,
/// no file-backed ones) in bytes; returned in KiB.
#[cfg(any(target_os = "macos", test))]
pub fn parse_footprint_kb(report: &str) -> Option<u64> {
    report.lines().find_map(|line| {
        let v = line.trim().strip_suffix("peak memory footprint")?;
        v.trim().parse::<u64>().ok().map(|b| b / 1024)
    })
}

/// The `RssAnon` line of a `/proc/<pid>/status`, in KiB.
#[cfg(any(target_os = "linux", test))]
pub fn parse_rss_anon_kb(status: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        let v = line.strip_prefix("RssAnon:")?;
        v.trim().strip_suffix("kB")?.trim().parse().ok()
    })
}

/// One untimed run's memory: peak RSS and private memory, each `None` where this
/// platform cannot read it.
struct MemSample {
    rss_kb: Option<u64>,
    private_kb: Option<u64>,
}

fn mem_once(tool: &Tool, args: &[String], cwd: &Path) -> MemSample {
    let report = time_report(tool, args, cwd);
    MemSample {
        rss_kb: report.as_deref().and_then(parse_rss_kb),
        private_kb: private_once(tool, args, cwd, report.as_deref()),
    }
}

#[cfg(target_os = "macos")]
fn private_once(_: &Tool, _: &[String], _: &Path, report: Option<&str>) -> Option<u64> {
    report.and_then(parse_footprint_kb)
}

/// Spawn the tool and read `RssAnon` from `/proc/<pid>/status` for as long as it runs.
/// The kernel drops the counters when the process exits, so the figure is the largest
/// sample, not an exact peak.
#[cfg(target_os = "linux")]
fn private_once(tool: &Tool, args: &[String], cwd: &Path, _: Option<&str>) -> Option<u64> {
    let mut child = Command::new(&tool.program)
        .args(&tool.prefix)
        .args(args)
        .current_dir(cwd)
        .env("TZ", "UTC")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let path = format!("/proc/{}/status", child.id());
    let mut best = None;
    while matches!(child.try_wait(), Ok(None)) {
        if let Some(kb) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| parse_rss_anon_kb(&t))
        {
            best = best.max(Some(kb));
        }
    }
    best
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn private_once(_: &Tool, _: &[String], _: &Path, _: Option<&str>) -> Option<u64> {
    None
}

/// `/usr/bin/time`'s report (`-l` on macOS, `-v` on Linux) for one untimed run.
fn time_report(tool: &Tool, args: &[String], cwd: &Path) -> Option<String> {
    let flag = if cfg!(target_os = "macos") {
        "-l"
    } else {
        "-v"
    };
    let out = Command::new("/usr/bin/time")
        .arg(flag)
        .arg(&tool.program)
        .args(&tool.prefix)
        .args(args)
        .current_dir(cwd)
        .env("TZ", "UTC")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stderr).into_owned())
}

fn cv_pct(samples: &[f64]) -> f64 {
    let n = samples.len() as f64;
    let mean = samples.iter().sum::<f64>() / n;
    if samples.len() < 2 || mean == 0.0 {
        return 0.0;
    }
    let var = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    100.0 * var.sqrt() / mean
}

fn median_u64(mut v: Vec<u64>) -> Option<u64> {
    v.sort_unstable();
    v.get(v.len() / 2).copied()
}

/// Run one row. `tools[0]` is the subject, `tools[1]` the primary reference,
/// `tools[2]` (optional) the secondary.
pub fn run_row(
    tools: &[Tool],
    q: &PerfQuery,
    input: Option<&InputSpec>,
    inputs_dir: &Path,
    reps: usize,
) -> Result<PerfRow> {
    let file = input.map(|i| format!("{}.json", i.id));
    let args = args_for(q, file.as_deref());
    // Output-identity gate: hash each tool's stdout, compare with its exit status.
    let mut digest = vec![];
    for t in tools {
        let e = exec_within(t, &args, "", inputs_dir, PERF_DEADLINE)?;
        if e.timed_out {
            bail!(
                "{} did not finish `{}` on {:?} within {:?}",
                t.name,
                q.id,
                file,
                PERF_DEADLINE
            );
        }
        digest.push((e.exit, sha256_hex(&e.stdout)));
    }
    let gate_of = |i: usize| {
        if digest[i] == digest[0] {
            Gate::Match
        } else {
            Gate::Differs
        }
    };
    let gate = gate_of(1);
    let gate_secondary = (tools.len() > 2).then(|| gate_of(2));
    let mut row = PerfRow {
        query: q.id.clone(),
        input: input.map_or_else(|| "none".into(), |i| i.id.clone()),
        tier: input.map_or_else(|| "startup".into(), |i| i.tier.clone()),
        gate,
        gate_secondary,
        tools: vec![],
        speedup_primary: None,
        speedup_secondary: None,
        secondary_over_primary: None,
        rss_ratio_primary: None,
        private_ratio_primary: None,
        noisy: false,
    };
    if gate == Gate::Differs || reps == 0 {
        return Ok(row);
    }
    let mut samples: Vec<Vec<f64>> = vec![vec![]; tools.len()];
    // One untimed warm-up of each, then rotate the starting tool every repetition.
    for t in tools {
        time_once(t, &args, inputs_dir)?;
    }
    for rep in 0..reps {
        for k in 0..tools.len() {
            let i = (rep + k) % tools.len();
            samples[i].push(time_once(&tools[i], &args, inputs_dir)?);
        }
    }
    for (i, t) in tools.iter().enumerate() {
        let mem: Vec<MemSample> = (0..3).map(|_| mem_once(t, &args, inputs_dir)).collect();
        let rss = median_u64(mem.iter().filter_map(|m| m.rss_kb).collect());
        let private = median_u64(mem.iter().filter_map(|m| m.private_kb).collect());
        row.tools.push(ToolResult {
            tool: t.name.clone(),
            stats: Some(stats(&samples[i])),
            cv_pct: Some(cv_pct(&samples[i])),
            peak_rss_kb: rss,
            private_kb: private,
        });
    }
    row.noisy = row.tools.iter().any(|t| t.cv_pct.is_some_and(|c| c > 5.0));
    let med = |i: usize| row.tools[i].stats.as_ref().map(|s| s.median_ms);
    row.speedup_primary = Some(med(1).unwrap() / med(0).unwrap());
    if gate_secondary == Some(Gate::Match) {
        row.speedup_secondary = Some(med(2).unwrap() / med(0).unwrap());
    }
    if tools.len() > 2 && digest[1] == digest[2] {
        row.secondary_over_primary = Some(med(2).unwrap() / med(1).unwrap());
    }
    let ratio = |a: Option<u64>, b: Option<u64>| match (a, b) {
        (Some(s), Some(r)) if r > 0 => Some(s as f64 / r as f64),
        _ => None,
    };
    row.rss_ratio_primary = ratio(row.tools[0].peak_rss_kb, row.tools[1].peak_rss_kb);
    row.private_ratio_primary = ratio(row.tools[0].private_kb, row.tools[1].private_kb);
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str, shape: &str, tier: &str) -> InputSpec {
        InputSpec {
            id: id.into(),
            shape: shape.into(),
            tier: tier.into(),
            pattern: shape.into(),
            size: tier.into(),
            seed: 1,
            sha256: String::new(),
        }
    }

    #[test]
    fn query_file_parses_and_rejects_bad_rows() {
        let q = parse_queries("# c\nlen\t-c\tlength\tusers,wide\t10mb\nstart\t-n\t1\tnone\t1mb\n")
            .unwrap();
        assert_eq!(
            (
                q[0].shapes.len(),
                q[0].max_tier.as_str(),
                q[0].flags.clone()
            ),
            (2, "10mb", vec!["-c".to_string()])
        );
        assert!(parse_queries("a\tb\n").is_err());
        assert!(parse_queries("a\t-\t.\tusers\t5gb\n").is_err());
    }

    #[test]
    fn checked_in_queries_parse_and_name_known_shapes() {
        let text = include_str!("../../../../comparison/jq/queries/perf.tsv");
        let qs = parse_queries(text).unwrap();
        assert!(qs.len() >= 25);
        let known = [
            "users",
            "numbers",
            "strings",
            "wide",
            "nested",
            "comprehensive",
            "none",
        ];
        assert!(qs
            .iter()
            .flat_map(|q| &q.shapes)
            .all(|s| known.contains(&s.as_str())));
    }

    #[test]
    fn plan_respects_the_run_tier_and_each_query_cap() {
        let inputs = [
            spec("u1", "users", "1mb"),
            spec("u10", "users", "10mb"),
            spec("u100", "users", "100mb"),
        ];
        let qs =
            parse_queries("a\t-\t.\tusers\t10mb\nb\t-\t.\tusers\t100mb\nc\t-n\t1\tnone\t1mb\n")
                .unwrap();
        let ids = |top: &str| -> Vec<String> {
            plan(&qs, &inputs, top)
                .iter()
                .map(|(q, i)| format!("{}@{}", q.id, i.map_or("none", |i| i.id.as_str())))
                .collect()
        };
        assert_eq!(ids("1mb"), ["a@u1", "b@u1", "c@none"]);
        assert_eq!(
            ids("100mb"),
            ["a@u1", "a@u10", "b@u1", "b@u10", "b@u100", "c@none"]
        );
    }

    #[test]
    fn rss_is_read_from_both_time_formats() {
        assert_eq!(
            parse_rss_kb("  1234567  maximum resident set size\n"),
            Some(1205)
        );
        assert_eq!(
            parse_rss_kb("\tMaximum resident set size (kbytes): 4321\n"),
            Some(4321)
        );
        assert_eq!(parse_rss_kb("nothing"), None);
    }

    #[test]
    fn private_memory_is_read_from_both_platforms() {
        assert_eq!(
            parse_footprint_kb(
                "  1294336  maximum resident set size\n   999640  peak memory footprint\n"
            ),
            Some(976)
        );
        assert_eq!(
            parse_footprint_kb("  1294336  maximum resident set size\n"),
            None
        );
        let status =
            "Name:\tjq\nVmRSS:\t   3616 kB\nRssAnon:\t     840 kB\nRssFile:\t    2776 kB\n";
        assert_eq!(parse_rss_anon_kb(status), Some(840));
        // A zombie's status has no Rss lines once its address space is gone.
        assert_eq!(parse_rss_anon_kb("Name:\tjq\nState:\tZ (zombie)\n"), None);
    }

    /// A process that holds a known amount of anonymous memory must read at least that
    /// much, and far less than its mapped file pages would add up to.
    #[cfg(target_os = "linux")]
    #[test]
    fn private_memory_of_a_running_process_is_sampled() {
        let tool = Tool {
            name: "python3".into(),
            program: "python3".into(),
            prefix: vec![],
        };
        let args = vec![
            "-c".to_string(),
            "import time; b = bytearray(64 * 1024 * 1024); b[::4096] = b'x' * len(b[::4096]); time.sleep(0.5)"
                .to_string(),
        ];
        let kb = private_once(&tool, &args, Path::new("."), None).expect("sampled");
        assert!(kb >= 60 * 1024, "{kb} KiB");
    }

    #[test]
    fn cv_is_zero_for_constant_samples_and_positive_otherwise() {
        assert_eq!(cv_pct(&[2.0, 2.0, 2.0]), 0.0);
        assert!(cv_pct(&[1.0, 2.0, 3.0]) > 40.0);
    }

    fn sh(name: &str, script: &str) -> Tool {
        Tool {
            name: name.into(),
            program: "sh".into(),
            prefix: vec!["-c".into(), script.into(), "sh".into()],
        }
    }

    /// The gate must be able to fail: a subject that prints something else is
    /// a correctness row with no timing and no speedup.
    #[test]
    fn a_differing_subject_is_untimed() {
        let q = parse_queries("q\t-\tx\tnone\t1mb\n").unwrap().remove(0);
        let ok = sh("ref", "echo 1");
        let bad = sh("sub", "echo 2");
        let row = run_row(&[bad, ok.clone(), ok], &q, None, Path::new("."), 3).unwrap();
        assert_eq!(
            (row.gate, row.gate_secondary),
            (Gate::Differs, Some(Gate::Differs))
        );
        assert!(row.tools.is_empty() && row.speedup_primary.is_none());
    }

    #[test]
    fn a_matching_row_is_timed_against_both_references() {
        let q = parse_queries("q\t-\tx\tnone\t1mb\n").unwrap().remove(0);
        let t = sh("t", "echo 1");
        let row = run_row(&[t.clone(), t.clone(), t], &q, None, Path::new("."), 3).unwrap();
        assert_eq!(row.gate, Gate::Match);
        assert_eq!(row.tools.len(), 3);
        assert!(
            row.speedup_primary.is_some()
                && row.speedup_secondary.is_some()
                && row.secondary_over_primary.is_some()
        );
    }

    /// A reference that prints differently from the other reference gets no
    /// secondary speedup, yet the primary row still times.
    #[test]
    fn a_secondary_that_disagrees_gets_no_secondary_figure() {
        let q = parse_queries("q\t-\tx\tnone\t1mb\n").unwrap().remove(0);
        let (a, b) = (sh("a", "echo 1"), sh("b", "echo 2"));
        let row = run_row(&[a.clone(), a, b], &q, None, Path::new("."), 3).unwrap();
        assert_eq!(
            (row.gate, row.gate_secondary),
            (Gate::Match, Some(Gate::Differs))
        );
        assert!(
            row.speedup_primary.is_some()
                && row.speedup_secondary.is_none()
                && row.secondary_over_primary.is_none()
        );
    }
}
