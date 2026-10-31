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

/// A row whose slowest tool has a median above this many milliseconds is long
/// enough that run-to-run noise (scheduler, frequency, cache state) dominates
/// the figure, so it is given more repetitions (#5581). The shorter rows are
/// stable and stay as they are.
pub const SLOW_ROW_MS: f64 = 15.0;
/// How many repetitions a slow row is topped up to, at least.
pub const SLOW_ROW_REPS: usize = 21;
/// The most extra wall time, summed over every tool, a row may spend on timed top-up
/// repetitions (the untimed peak-RSS runs are not counted), so a 100 MB row of several seconds is not tripled.
pub const SLOW_ROW_EXTRA_BUDGET_MS: f64 = 30_000.0;

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

/// A flag that stands for the input file's name, where an option takes one
/// (`--slurpfile d @INPUT@`). A query that uses it passes the file only there.
pub const INPUT_FLAG: &str = "@INPUT@";
/// A pseudo-flag, `@files=N`, that passes the input file `N` times as positional
/// arguments, so a run reads several files.
pub const FILES_FLAG_PREFIX: &str = "@files=";
/// The most copies `@files=N` may ask for.
const MAX_FILE_COPIES: usize = 16;

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
        let flags: Vec<&str> = c[1].split(' ').collect();
        let shapes: Vec<&str> = c[3].split(',').collect();
        for f in flags.iter().filter(|f| f.starts_with(FILES_FLAG_PREFIX)) {
            let copies = f[FILES_FLAG_PREFIX.len()..].parse::<usize>();
            if !matches!(copies, Ok(n) if (1..=MAX_FILE_COPIES).contains(&n)) {
                bail!(
                    "perf query line {}: `{f}` wants a count from 1 to {MAX_FILE_COPIES}",
                    n + 1
                );
            }
        }
        if shapes.contains(&"none")
            && flags
                .iter()
                .any(|f| *f == INPUT_FLAG || f.starts_with(FILES_FLAG_PREFIX))
        {
            bail!(
                "perf query line {}: `{INPUT_FLAG}` and `{FILES_FLAG_PREFIX}N` need an input file, and shape `none` has none",
                n + 1
            );
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
    /// Timed repetitions each tool ran (a slow row is topped up past the requested count).
    pub reps: usize,
    /// Median ratios (reference ÷ subject); `None` unless that pair's output matched.
    pub speedup_primary: Option<f64>,
    /// The speedups above are ratios of each tool's minimum rather than its median: set for a
    /// slow row ([`SLOW_ROW_MS`]) that reached [`SLOW_ROW_REPS`] repetitions, whose median the
    /// run-to-run noise moves. A slow row the time budget cut short keeps the median, since the
    /// minimum of a handful of runs is no steadier. `secondary_over_primary` is always a median.
    pub ratio_from_min: bool,
    pub speedup_secondary: Option<f64>,
    /// Secondary ÷ primary median, so a change in the reference's own speed is visible.
    pub secondary_over_primary: Option<f64>,
    pub rss_ratio_primary: Option<f64>,
    /// The same ratio over private memory (`ToolResult::private_kb`).
    pub private_ratio_primary: Option<f64>,
    /// Any tool's CV exceeds 5%.
    pub noisy: bool,
}

/// The command line of one run: the flags, the filter, then the input file. `@INPUT@` in
/// the flags is the file's name at that position, and then it is not appended again;
/// `@files=N` appends it `N` times.
fn args_for(q: &PerfQuery, file: Option<&str>) -> Vec<String> {
    let mut a = vec![];
    let mut named = false;
    let mut copies = 1;
    for f in &q.flags {
        if f == INPUT_FLAG {
            a.extend(file.map(String::from));
            named = true;
        } else if let Some(n) = f.strip_prefix(FILES_FLAG_PREFIX) {
            copies = n.parse().unwrap_or(1);
        } else {
            a.push(f.clone());
        }
    }
    a.push(q.filter.clone());
    if !named {
        for _ in 0..copies {
            a.extend(file.map(String::from));
        }
    }
    a
}

/// What every run of a tool shares: its prefix and flags, the working directory and a
/// fixed environment, so the timed, peak-RSS and private-memory runs are one configuration.
fn configure<'a>(
    cmd: &'a mut Command,
    tool: &Tool,
    args: &[String],
    cwd: &Path,
) -> &'a mut Command {
    cmd.args(&tool.prefix)
        .args(args)
        .current_dir(cwd)
        .env("TZ", "UTC")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
}

fn time_once(tool: &Tool, args: &[String], cwd: &Path) -> Result<f64> {
    let start = Instant::now();
    let status = configure(&mut Command::new(&tool.program), tool, args, cwd)
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
/// sample, not an exact peak, and it is a second run of the tool beside `time_report`'s.
/// `spawn` returns only once the child has called `exec`, so the first sample is the
/// tool's own address space and never a copy of this process's.
#[cfg(target_os = "linux")]
fn private_once(tool: &Tool, args: &[String], cwd: &Path, _: Option<&str>) -> Option<u64> {
    let mut child = configure(&mut Command::new(&tool.program), tool, args, cwd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let path = format!("/proc/{}/status", child.id());
    let start = Instant::now();
    let mut best = None;
    while matches!(child.try_wait(), Ok(None)) {
        if start.elapsed() > PERF_DEADLINE {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        if let Some(kb) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| parse_rss_anon_kb(&t))
        {
            best = best.max(Some(kb));
        }
        std::thread::sleep(std::time::Duration::from_micros(100));
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
    let out = configure(
        Command::new("/usr/bin/time").arg(flag).arg(&tool.program),
        tool,
        args,
        cwd,
    )
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

/// Is any tool's median over [`SLOW_ROW_MS`]?
fn is_slow(samples: &[Vec<f64>]) -> bool {
    samples
        .iter()
        .filter(|s| !s.is_empty())
        .any(|s| stats(s).median_ms > SLOW_ROW_MS)
}

/// How many repetitions to add to `done` already taken: none for a row whose
/// slowest median is at most [`SLOW_ROW_MS`]; otherwise up to
/// [`SLOW_ROW_REPS`] in total, held to [`SLOW_ROW_EXTRA_BUDGET_MS`] of extra
/// wall time across every tool.
fn extra_reps(samples: &[Vec<f64>], done: usize) -> usize {
    if !is_slow(samples) {
        return 0;
    }
    let per_rep: f64 = samples
        .iter()
        .filter(|s| !s.is_empty())
        .map(|s| stats(s).median_ms)
        .sum();
    let affordable = (SLOW_ROW_EXTRA_BUDGET_MS / per_rep) as usize;
    SLOW_ROW_REPS.saturating_sub(done).min(affordable)
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
        reps: 0,
        speedup_primary: None,
        ratio_from_min: false,
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
    let mut rep = 0;
    let mut total = reps;
    while rep < total {
        for k in 0..tools.len() {
            let i = (rep + k) % tools.len();
            samples[i].push(time_once(&tools[i], &args, inputs_dir)?);
        }
        rep += 1;
        if rep == reps {
            total += extra_reps(&samples, reps);
        }
    }
    row.reps = total;
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
    // A slow row's median moves with the run (a control read 0.75x-1.40x, #5581) while its
    // minimum does not, so a row that got the full top-up rates each pair by the minimum.
    row.ratio_from_min = is_slow(&samples) && total >= SLOW_ROW_REPS;
    let from_min = row.ratio_from_min;
    let stat = |i: usize, min: bool| {
        row.tools[i]
            .stats
            .as_ref()
            .map(|s| if min { s.min_ms } else { s.median_ms })
            .unwrap()
    };
    row.speedup_primary = Some(stat(1, from_min) / stat(0, from_min));
    if gate_secondary == Some(Gate::Match) {
        row.speedup_secondary = Some(stat(2, from_min) / stat(0, from_min));
    }
    if tools.len() > 2 && digest[1] == digest[2] {
        // The reference's own change in speed: its run-to-run spread is the point, so a median.
        row.secondary_over_primary = Some(stat(2, false) / stat(1, false));
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

    fn args(line: &str, file: Option<&str>) -> Vec<String> {
        args_for(&parse_queries(line).unwrap()[0], file)
    }

    #[test]
    fn the_input_file_follows_the_filter_unless_a_flag_names_it() {
        assert_eq!(
            args("a\t-c\t.\tusers\t1mb\n", Some("u.json")),
            ["-c", ".", "u.json"]
        );
        assert_eq!(
            args(
                "a\t-n --slurpfile d @INPUT@\t$d|length\tusers\t1mb\n",
                Some("u.json")
            ),
            ["-n", "--slurpfile", "d", "u.json", "$d|length"]
        );
        assert_eq!(
            args("a\t-n --arg k v\t$k\tnone\t1mb\n", None),
            ["-n", "--arg", "k", "v", "$k"]
        );
    }

    #[test]
    fn files_n_passes_the_input_that_many_times_and_is_not_a_flag() {
        assert_eq!(
            args("a\t-c @files=3\t.id\tndjson\t1mb\n", Some("n.json")),
            ["-c", ".id", "n.json", "n.json", "n.json"]
        );
    }

    #[test]
    fn a_placeholder_needs_an_input_and_a_sane_count() {
        assert!(parse_queries("a\t-n @INPUT@\t.\tnone\t1mb\n").is_err());
        assert!(parse_queries("a\t@files=2\t.\tnone\t1mb\n").is_err());
        assert!(parse_queries("a\t@files=0\t.\tusers\t1mb\n").is_err());
        assert!(parse_queries("a\t@files=17\t.\tusers\t1mb\n").is_err());
        assert!(parse_queries("a\t@files=x\t.\tusers\t1mb\n").is_err());
        assert!(parse_queries("a\t@files=16\t.\tusers\t1mb\n").is_ok());
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
            "ndjson",
            "jsonseq",
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
        if Command::new("python3").arg("--version").output().is_err() {
            eprintln!("python3 not found; skipping");
            return;
        }
        let args = vec![
            "-c".to_string(),
            "import time; b = bytearray(64 * 1024 * 1024); b[::4096] = b'x' * len(b[::4096]); time.sleep(0.5)"
                .to_string(),
        ];
        let kb = private_once(&tool, &args, Path::new("."), None).expect("sampled");
        assert!(kb >= 60 * 1024, "{kb} KiB");
    }

    #[test]
    fn only_a_slow_row_is_topped_up_and_the_extra_time_is_capped() {
        let quick = vec![vec![2.0; 3], vec![3.0; 3]];
        assert_eq!(
            extra_reps(&quick, 3),
            0,
            "a row under the threshold is left alone"
        );
        let slow = vec![vec![40.0; 3], vec![50.0; 3]];
        assert_eq!(extra_reps(&slow, 3), SLOW_ROW_REPS - 3);
        assert_eq!(extra_reps(&slow, SLOW_ROW_REPS), 0, "already at the target");
        let huge = vec![vec![4_000.0; 3], vec![5_000.0; 3]];
        assert_eq!(
            extra_reps(&huge, 3),
            3,
            "the budget bounds a multi-second row"
        );
        let slower = vec![vec![40_000.0; 3], vec![50_000.0; 3]];
        assert_eq!(extra_reps(&slower, 3), 0);
    }

    /// One slow tool is enough: the row's figure is dominated by that tool's noise.
    #[test]
    fn a_single_slow_tool_tops_the_row_up() {
        let mixed = vec![vec![1.0; 3], vec![30.0; 3]];
        assert!(extra_reps(&mixed, 3) > 0);
    }

    #[test]
    fn a_slow_row_runs_the_topped_up_repetitions() {
        let q = parse_queries("q\t-\tx\tnone\t1mb\n").unwrap().remove(0);
        let slow = sh("slow", "sleep 0.02");
        let row = run_row(&[slow.clone(), slow], &q, None, Path::new("."), 3).unwrap();
        assert_eq!(row.reps, SLOW_ROW_REPS);
        assert!(row
            .tools
            .iter()
            .all(|t| t.stats.as_ref().unwrap().n == SLOW_ROW_REPS));
        assert!(
            row.ratio_from_min,
            "a slow row's ratios come from the minimum"
        );
    }

    #[test]
    fn a_fast_row_keeps_the_requested_repetitions() {
        let q = parse_queries("q\t-\tx\tnone\t1mb\n").unwrap().remove(0);
        let t = sh("t", ":");
        let row = run_row(&[t.clone(), t], &q, None, Path::new("."), 3).unwrap();
        assert_eq!(row.reps, 3);
        assert!(!row.ratio_from_min, "a fast row keeps the median ratio");
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
