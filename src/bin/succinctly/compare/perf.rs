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
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
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
pub const TIERS: &[&str] = &["1mb", "10mb", "100mb", HUGE_TIER];

/// The opt-in tier above 100 MB (`--huge`, #6915). Its rows are kept apart from the headline
/// ones (`Results::perf_huge`), time and measure one run where a smaller tier takes several, and
/// record how each tool ended, so a kill or a deadline is a result and not a missing row.
pub const HUGE_TIER: &str = "1gb";
/// Timed repetitions of a [`HUGE_TIER`] row, as at 100 MB: a repetition is a minute or more.
pub const HUGE_REPS: usize = 3;

#[derive(Debug, Clone, Deserialize)]
pub struct InputSpec {
    pub id: String,
    pub shape: String,
    pub tier: String,
    /// `succinctly json generate -p`. Unused by a corpus input.
    #[serde(default)]
    pub pattern: String,
    #[serde(default)]
    pub size: String,
    #[serde(default)]
    pub seed: u64,
    pub sha256: String,
    /// A real file from the benchmark corpus (`file` in `tests/data/bench-corpus/manifest.json`)
    /// instead of a generated one (#5552). It is copied to the inputs directory, so a query
    /// names it like any other input.
    #[serde(default)]
    pub corpus: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PerfQuery {
    pub id: String,
    pub flags: Vec<String>,
    pub filter: String,
    /// Shapes the query applies to; `none` runs it with no input file.
    pub shapes: Vec<String>,
    pub max_tier: String,
    /// The input is written to a pipe on the tool's stdin instead of named as a file
    /// argument (`@stdin`, #6914).
    pub stdin: bool,
}

/// The pseudo-flag that feeds the input through a pipe on stdin: the row's file is read once
/// and written to the tool's standard input, which is a pipe and never a redirected regular
/// file (a tool can tell the two apart, #6914). No file argument is passed.
pub const STDIN_FLAG: &str = "@stdin";
/// A stdin row's id ends with this, after its file-route twin's id.
pub const STDIN_SUFFIX: &str = "_stdin";

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
        if flags.contains(&STDIN_FLAG) {
            if shapes.contains(&"none") {
                bail!(
                    "perf query line {}: `{STDIN_FLAG}` feeds an input file, and shape `none` has none",
                    n + 1
                );
            }
            if flags
                .iter()
                .any(|f| *f == INPUT_FLAG || f.starts_with(FILES_FLAG_PREFIX))
            {
                bail!(
                    "perf query line {}: `{STDIN_FLAG}` passes no file argument, so `{INPUT_FLAG}` and `{FILES_FLAG_PREFIX}N` have nothing to name",
                    n + 1
                );
            }
            if !c[0].ends_with(STDIN_SUFFIX) {
                bail!(
                    "perf query line {}: a `{STDIN_FLAG}` row's id ends with `{STDIN_SUFFIX}`, after its file-route twin's",
                    n + 1
                );
            }
        }
        if c[0].ends_with(STDIN_SUFFIX) && !flags.contains(&STDIN_FLAG) {
            bail!(
                "perf query line {}: `{}` ends with `{STDIN_SUFFIX}`, which names a `{STDIN_FLAG}` row, and has no `{STDIN_FLAG}`",
                n + 1,
                c[0]
            );
        }
        if flags.contains(&INPUT_FLAG) && flags.iter().any(|f| f.starts_with(FILES_FLAG_PREFIX)) {
            bail!(
                "perf query line {}: `{INPUT_FLAG}` names the file once, so `{FILES_FLAG_PREFIX}N` has nothing to repeat",
                n + 1
            );
        }
        out.push(PerfQuery {
            id: c[0].into(),
            flags: if c[1] == "-" {
                vec![]
            } else {
                c[1].split(' ')
                    .filter(|f| *f != STDIN_FLAG)
                    .map(String::from)
                    .collect()
            },
            filter: c[2].into(),
            shapes: c[3].split(',').map(String::from).collect(),
            max_tier: c[4].into(),
            stdin: flags.contains(&STDIN_FLAG),
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

/// Where a corpus file may be: the vendored seed first, then the copy
/// `scripts/sync-bench-corpus.sh` fetches (a file too large to commit).
fn corpus_candidates(root: &Path, file: &str) -> [PathBuf; 2] {
    [
        root.join("tests/data/bench-corpus/seed").join(file),
        root.join("data/bench/corpus").join(file),
    ]
}

/// The corpus file behind `spec`, or `None` when it is a generated input or the file is not
/// on this machine (a fetched one before `scripts/sync-bench-corpus.sh` has run).
pub fn corpus_source(root: &Path, spec: &InputSpec) -> Option<PathBuf> {
    let file = spec.corpus.as_deref()?;
    corpus_candidates(root, file)
        .into_iter()
        .find(|p| p.is_file())
}

/// Whether `spec` can be produced here: a generated input always can, a corpus input only
/// when its file is present.
pub fn available(root: &Path, spec: &InputSpec) -> bool {
    spec.corpus.is_none() || corpus_source(root, spec).is_some()
}

/// Generate `spec` into `dir` if absent (or copy its corpus file there) and verify its
/// sha256, so a changed generator or a changed corpus file is a failure and not a silently
/// different workload.
pub fn ensure_input(
    root: &Path,
    generator: &Path,
    dir: &Path,
    spec: &InputSpec,
) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.json", spec.id));
    if let Some(file) = &spec.corpus {
        // A copy left by an earlier run is refreshed when it no longer matches the pin
        // (the corpus file was re-pinned), and only then judged against it below.
        let cached_ok = std::fs::read(&path).is_ok_and(|b| sha256_hex(&b) == spec.sha256);
        if !cached_ok {
            let Some(from) = corpus_source(root, spec) else {
                bail!(
                    "{} names corpus file {file}, which is in neither {} (run \
                     scripts/sync-bench-corpus.sh to fetch it)",
                    spec.id,
                    corpus_candidates(root, file)
                        .map(|p| p.display().to_string())
                        .join(" nor ")
                );
            };
            std::fs::copy(&from, &path)
                .with_context(|| format!("copying {} to {}", from.display(), path.display()))?;
        }
    } else if !path.exists() {
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
            "{} has sha256 {got}, the manifest pins {} ({}; \
             delete {} to regenerate, or re-pin on purpose)",
            spec.id,
            spec.sha256,
            if spec.corpus.is_some() {
                "the corpus file changed"
            } else {
                "the generator or a stale file changed"
            },
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
    /// A tool was killed by a signal or ran past the deadline on a [`HUGE_TIER`] row, so there is
    /// no output to compare. Its exit status and peak memory are recorded, and it is no timing
    /// row (#6915).
    Incomplete,
}

/// How one tool's gate run ended, recorded for a [`HUGE_TIER`] row (#6915): an out-of-memory
/// kill, a crash and a refusal are results there, not a hole in the table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunEnd {
    /// The exit status; `None` when a signal or the deadline ended the run.
    pub exit: Option<i32>,
    /// The signal that killed it (`9`: the kernel's out-of-memory kill, or macOS's jetsam).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
    /// It ran past the deadline and was killed by the harness.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub timed_out: bool,
}

impl RunEnd {
    fn of(e: &super::exec::Exec) -> Self {
        RunEnd {
            exit: e.exit,
            signal: e.signal,
            timed_out: e.timed_out,
        }
    }

    /// No exit status to compare: a signal or the deadline ended it.
    pub fn killed(&self) -> bool {
        self.signal.is_some() || self.timed_out
    }
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
    /// Instructions executed (`I refs` of one `valgrind --tool=cachegrind` run, #5552): a
    /// count that machine load cannot move, beside the wall-clock. `None` without
    /// `--cachegrind`, on a row too large to run under valgrind, or where valgrind failed.
    pub instructions: Option<u64>,
    /// How the gate run ended. Recorded on [`HUGE_TIER`] rows only, so every other row serializes
    /// as it always has.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<RunEnd>,
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
    /// The same ratio over instructions executed (`ToolResult::instructions`).
    pub ir_ratio_primary: Option<f64>,
    /// Any tool's CV exceeds 5%.
    pub noisy: bool,
    /// The input file's size in bytes, recorded on [`HUGE_TIER`] rows so the report can state
    /// peak memory as a multiple of the input (#6915).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_bytes: Option<u64>,
}

/// The command line of one run: the flags, the filter, then the input file. `@INPUT@` in
/// the flags is the file's name at that position, and then it is not appended again;
/// `@files=N` appends it `N` times; `@stdin` appends nothing (the input is piped).
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
    // A stdin row passes the input on the pipe, not as an argument.
    if !named && !q.stdin {
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
}

/// The stdin of a run: a pipe when the row feeds its input (see [`with_feed`]), else nothing.
fn stdin_for(feed: Option<&[u8]>) -> Stdio {
    if feed.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    }
}

/// Run `f` on `child` while `feed` is written to its piped stdin on another thread, so a
/// tool that reads its input as it goes and one that slurps it all see the same pipe a
/// producer (`curl`, `kubectl`) would give them (#6914). The pipe is closed when the bytes
/// are written, and a tool that stops reading (`first`, `limit`) ends the write on `EPIPE`,
/// which is ignored. Without a `feed` this is just `f(child)`.
fn with_feed<T>(child: &mut Child, feed: Option<&[u8]>, f: impl FnOnce(&mut Child) -> T) -> T {
    let sin = child.stdin.take();
    std::thread::scope(|s| {
        if let (Some(mut sin), Some(bytes)) = (sin, feed) {
            s.spawn(move || {
                let _ = sin.write_all(bytes);
            });
        }
        f(child)
    })
}

fn time_once(tool: &Tool, args: &[String], cwd: &Path, feed: Option<&[u8]>) -> Result<f64> {
    let start = Instant::now();
    let mut child = configure(&mut Command::new(&tool.program), tool, args, cwd)
        .stdin(stdin_for(feed))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawning {}", tool.name))?;
    let status = with_feed(&mut child, feed, Child::wait);
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

fn mem_once(tool: &Tool, args: &[String], cwd: &Path, feed: Option<&[u8]>) -> MemSample {
    let report = time_report(tool, args, cwd, feed);
    MemSample {
        rss_kb: report.as_deref().and_then(parse_rss_kb),
        private_kb: private_once(tool, args, cwd, feed, report.as_deref()),
    }
}

#[cfg(target_os = "macos")]
fn private_once(
    _: &Tool,
    _: &[String],
    _: &Path,
    _: Option<&[u8]>,
    report: Option<&str>,
) -> Option<u64> {
    report.and_then(parse_footprint_kb)
}

/// Spawn the tool and read `RssAnon` from `/proc/<pid>/status` for as long as it runs.
/// The kernel drops the counters when the process exits, so the figure is the largest
/// sample, not an exact peak, and it is a second run of the tool beside `time_report`'s.
/// `spawn` returns only once the child has called `exec`, so the first sample is the
/// tool's own address space and never a copy of this process's.
#[cfg(target_os = "linux")]
fn private_once(
    tool: &Tool,
    args: &[String],
    cwd: &Path,
    feed: Option<&[u8]>,
    _: Option<&str>,
) -> Option<u64> {
    let mut child = configure(&mut Command::new(&tool.program), tool, args, cwd)
        .stdin(stdin_for(feed))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let path = format!("/proc/{}/status", child.id());
    let start = Instant::now();
    with_feed(&mut child, feed, |child| {
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
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn private_once(
    _: &Tool,
    _: &[String],
    _: &Path,
    _: Option<&[u8]>,
    _: Option<&str>,
) -> Option<u64> {
    None
}

/// `/usr/bin/time`'s report (`-l` on macOS, `-v` on Linux) for one untimed run.
fn time_report(tool: &Tool, args: &[String], cwd: &Path, feed: Option<&[u8]>) -> Option<String> {
    let flag = if cfg!(target_os = "macos") {
        "-l"
    } else {
        "-v"
    };
    let mut child = configure(
        Command::new("/usr/bin/time").arg(flag).arg(&tool.program),
        tool,
        args,
        cwd,
    )
    .stdin(stdin_for(feed))
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .spawn()
    .ok()?;
    let mut err = child.stderr.take()?;
    let report = with_feed(&mut child, feed, |c| {
        let mut buf = vec![];
        let _ = err.read_to_end(&mut buf);
        let _ = c.wait();
        buf
    });
    Some(String::from_utf8_lossy(&report).into_owned())
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
#[cfg(test)]
fn is_slow(samples: &[Vec<f64>]) -> bool {
    is_slow_over(samples, SLOW_ROW_MS)
}

/// [`is_slow`] against a threshold of the caller's choosing (#5931): a test that must not depend
/// on how fast the host spawns a process names its own.
fn is_slow_over(samples: &[Vec<f64>], slow_ms: f64) -> bool {
    samples
        .iter()
        .filter(|s| !s.is_empty())
        .any(|s| stats(s).median_ms > slow_ms)
}

/// How many repetitions to add to `done` already taken: none for a row whose
/// slowest median is at most [`SLOW_ROW_MS`]; otherwise up to
/// [`SLOW_ROW_REPS`] in total, held to [`SLOW_ROW_EXTRA_BUDGET_MS`] of extra
/// wall time across every tool.
#[cfg(test)]
fn extra_reps(samples: &[Vec<f64>], done: usize) -> usize {
    extra_reps_over(samples, done, SLOW_ROW_MS)
}

/// [`extra_reps`] against the threshold [`is_slow_over`] takes.
fn extra_reps_over(samples: &[Vec<f64>], done: usize, slow_ms: f64) -> usize {
    if !is_slow_over(samples, slow_ms) {
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

/// The instruction count in cachegrind's log (`I   refs:      1,234,567`).
pub fn parse_ir(log: &str) -> Option<u64> {
    log.lines().find_map(|line| {
        let v = line.split_once("I   refs:")?.1;
        v.trim().replace(',', "").parse().ok()
    })
}

/// The longest one tool's run under valgrind may take (~50x its native time).
const IR_DEADLINE: std::time::Duration = std::time::Duration::from_secs(300);

/// Instructions one run of `tool` executes, from `valgrind --tool=cachegrind`. The count is
/// stable to within a hair (a hash seed or an allocator can move it by a few instructions), so
/// one run is enough for a signal read at a 5% band. Cache simulation is off (only `Ir` is
/// read), and the run has the timed runs' working directory and environment. A tool that exits
/// non-zero still has a valid count (valgrind passes its status through, and the output gate
/// has already matched it to the reference's), so the count is whatever the log holds: `None`
/// when valgrind could not run the tool at all, or ran past [`IR_DEADLINE`].
fn ir_once(
    valgrind: &Path,
    tool: &Tool,
    args: &[String],
    cwd: &Path,
    feed: Option<&[u8]>,
) -> Option<u64> {
    let name: String = tool
        .name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let dir = std::env::temp_dir().join(format!("compare-cg-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let log = dir.join("cg.log");
    let mut child = Command::new(valgrind)
        .args(["--tool=cachegrind", "--cache-sim=no"])
        .arg(format!("--log-file={}", log.display()))
        .arg(format!(
            "--cachegrind-out-file={}",
            dir.join("cg.out").display()
        ))
        .arg(&tool.program)
        .args(&tool.prefix)
        .args(args)
        .current_dir(cwd)
        .env("TZ", "UTC")
        .env("LC_ALL", "C")
        .stdin(stdin_for(feed))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    let finished = with_feed(&mut child, feed, |child| loop {
        match child.try_wait() {
            Ok(Some(_)) => break true,
            Ok(None) if start.elapsed() < IR_DEADLINE => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    });
    let count = std::fs::read_to_string(&log)
        .ok()
        .and_then(|t| parse_ir(&t))
        .filter(|_| finished);
    let _ = std::fs::remove_dir_all(&dir);
    count
}

/// A tool's entry on a row that was not timed: how its gate run ended and the peak memory of one
/// untimed run (#6915).
fn untimed_result(
    tool: &Tool,
    args: &[String],
    cwd: &Path,
    feed: Option<&[u8]>,
    end: &RunEnd,
) -> ToolResult {
    let mem = mem_once(tool, args, cwd, feed);
    ToolResult {
        tool: tool.name.clone(),
        stats: None,
        cv_pct: None,
        peak_rss_kb: mem.rss_kb,
        private_kb: mem.private_kb,
        instructions: None,
        end: Some(end.clone()),
    }
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
    run_row_with(tools, q, input, inputs_dir, reps, None)
}

/// [`run_row`], also counting each tool's instructions under the `valgrind` at that path (#5552).
pub fn run_row_with(
    tools: &[Tool],
    q: &PerfQuery,
    input: Option<&InputSpec>,
    inputs_dir: &Path,
    reps: usize,
    valgrind: Option<&Path>,
) -> Result<PerfRow> {
    run_row_over(tools, q, input, inputs_dir, reps, valgrind, SLOW_ROW_MS)
}

/// [`run_row_with`] against the slow-row threshold of the caller's choosing (#5931).
fn run_row_over(
    tools: &[Tool],
    q: &PerfQuery,
    input: Option<&InputSpec>,
    inputs_dir: &Path,
    reps: usize,
    valgrind: Option<&Path>,
    slow_ms: f64,
) -> Result<PerfRow> {
    run_row_bounded(
        tools,
        q,
        input,
        inputs_dir,
        reps,
        valgrind,
        slow_ms,
        PERF_DEADLINE,
    )
}

/// [`run_row_over`] with the gate's deadline of the caller's choosing: a test cannot wait out
/// [`PERF_DEADLINE`] to see a run recorded as having exceeded it (#6915).
#[allow(clippy::too_many_arguments)]
fn run_row_bounded(
    tools: &[Tool],
    q: &PerfQuery,
    input: Option<&InputSpec>,
    inputs_dir: &Path,
    reps: usize,
    valgrind: Option<&Path>,
    slow_ms: f64,
    deadline: std::time::Duration,
) -> Result<PerfRow> {
    let file = input.map(|i| format!("{}.json", i.id));
    let args = args_for(q, file.as_deref());
    // A stdin row reads its input once and pipes it into every run, the gate's included.
    let fed: Option<Vec<u8>> = match (q.stdin, &file) {
        (true, Some(f)) => {
            Some(std::fs::read(inputs_dir.join(f)).with_context(|| format!("reading {f}"))?)
        }
        _ => None,
    };
    let feed = fed.as_deref();
    // The opt-in 1 GB tier records how each tool ended and times one run where a smaller tier
    // times several (#6915).
    let huge = input.is_some_and(|i| i.tier == HUGE_TIER);
    // Output-identity gate: hash each tool's stdout, compare with its exit status.
    let mut digest = vec![];
    let mut ends = vec![];
    for t in tools {
        let e = exec_within(t, &args, feed.unwrap_or_default(), inputs_dir, deadline)?;
        // Past the deadline a smaller row has nothing to say and aborts the run; a 1 GB row is
        // the one where a slow tool is the finding, so it is recorded.
        if e.timed_out && !huge {
            bail!(
                "{} did not finish `{}` on {:?} within {:?}",
                t.name,
                q.id,
                file,
                deadline
            );
        }
        ends.push(RunEnd::of(&e));
        digest.push((e.exit, sha256_hex(&e.stdout)));
    }
    let gate_of = |i: usize| {
        if huge && (ends[0].killed() || ends[i].killed()) {
            Gate::Incomplete
        } else if digest[i] == digest[0] {
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
        ir_ratio_primary: None,
        noisy: false,
        input_bytes: None,
    };
    if huge {
        row.input_bytes = file
            .as_ref()
            .and_then(|f| std::fs::metadata(inputs_dir.join(f)).ok())
            .map(|m| m.len());
    }
    // How many untimed runs each memory figure is the median of.
    let mem_runs = if huge { 1 } else { 3 };
    if gate == Gate::Incomplete || (huge && gate == Gate::Differs) {
        // No timing either way, but on a 1 GB row what each tool did is the finding: the peak
        // memory a killed tool reached, or the exit status of one that refused (#6915).
        for (i, t) in tools.iter().enumerate() {
            row.tools
                .push(untimed_result(t, &args, inputs_dir, feed, &ends[i]));
        }
        return Ok(row);
    }
    if gate == Gate::Differs || reps == 0 {
        return Ok(row);
    }
    let mut samples: Vec<Vec<f64>> = vec![vec![]; tools.len()];
    // One untimed warm-up of each, then rotate the starting tool every repetition. A 1 GB row's
    // gate run has read the file into the page cache already, and a second run of a tool that
    // takes minutes is not worth its cost.
    if !huge {
        for t in tools {
            time_once(t, &args, inputs_dir, feed)?;
        }
    }
    let mut rep = 0;
    let mut total = reps;
    while rep < total {
        for k in 0..tools.len() {
            let i = (rep + k) % tools.len();
            samples[i].push(time_once(&tools[i], &args, inputs_dir, feed)?);
        }
        rep += 1;
        if rep == reps {
            total += extra_reps_over(&samples, reps, slow_ms);
        }
    }
    row.reps = total;
    for (i, t) in tools.iter().enumerate() {
        let mem: Vec<MemSample> = (0..mem_runs)
            .map(|_| mem_once(t, &args, inputs_dir, feed))
            .collect();
        let rss = median_u64(mem.iter().filter_map(|m| m.rss_kb).collect());
        let private = median_u64(mem.iter().filter_map(|m| m.private_kb).collect());
        row.tools.push(ToolResult {
            tool: t.name.clone(),
            stats: Some(stats(&samples[i])),
            cv_pct: Some(cv_pct(&samples[i])),
            peak_rss_kb: rss,
            private_kb: private,
            // The secondary reference is not counted: nothing reports it.
            instructions: valgrind
                .filter(|_| i < 2)
                .and_then(|v| ir_once(v, t, &args, inputs_dir, feed)),
            end: huge.then(|| ends[i].clone()),
        });
    }
    row.noisy = row.tools.iter().any(|t| t.cv_pct.is_some_and(|c| c > 5.0));
    // A slow row's median moves with the run (a control read 0.75x-1.40x, #5581) while its
    // minimum does not, so a row that got the full top-up rates each pair by the minimum.
    row.ratio_from_min = is_slow_over(&samples, slow_ms) && total >= SLOW_ROW_REPS;
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
    row.ir_ratio_primary = ratio(row.tools[0].instructions, row.tools[1].instructions);
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
            corpus: None,
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
        assert!(parse_queries("a\t-n @INPUT@ @files=2\t.\tusers\t1mb\n").is_err());
    }

    #[test]
    fn stdin_is_a_pseudo_flag_that_drops_the_file_argument() {
        let q = parse_queries("a_stdin\t-c @stdin\t.\tusers\t1mb\n").unwrap();
        assert!(q[0].stdin);
        assert_eq!(
            q[0].flags,
            ["-c"],
            "the pseudo-flag is not passed to the tool"
        );
        assert_eq!(args_for(&q[0], Some("u.json")), ["-c", "."]);
        // Alone, it leaves no flags at all.
        let q = parse_queries("a_stdin\t@stdin\t.\tusers\t1mb\n").unwrap();
        assert_eq!(args_for(&q[0], Some("u.json")), ["."]);
        // A file row is not a stdin row.
        let q = parse_queries("a\t-c\t.\tusers\t1mb\n").unwrap();
        assert!(!q[0].stdin);
    }

    #[test]
    fn a_stdin_row_must_have_an_input_to_pipe_and_name_its_twin() {
        assert!(parse_queries("a_stdin\t-n @stdin\t1\tnone\t1mb\n").is_err());
        assert!(parse_queries("a_stdin\t@stdin @INPUT@\t.\tusers\t1mb\n").is_err());
        assert!(parse_queries("a_stdin\t@stdin @files=2\t.\tusers\t1mb\n").is_err());
        assert!(
            parse_queries("a\t@stdin\t.\tusers\t1mb\n").is_err(),
            "the id ends with _stdin, so the report can find the file row"
        );
        assert!(
            parse_queries("a_stdin\t-c\t.\tusers\t1mb\n").is_err(),
            "a _stdin id without the flag would be a file row the report never pairs"
        );
    }

    /// Every stdin row has a file-route twin with the same flags, filter, shapes and cap, or the
    /// side-by-side table compares two different programs.
    #[test]
    fn every_checked_in_stdin_row_has_a_file_twin_that_runs_the_same_program() {
        let qs = parse_queries(include_str!("../../../../comparison/jq/queries/perf.tsv")).unwrap();
        let piped: Vec<&PerfQuery> = qs.iter().filter(|q| q.stdin).collect();
        assert!(piped.len() >= 6, "the issue's six rows");
        for p in piped {
            let twin_id = p.id.strip_suffix(STDIN_SUFFIX).unwrap();
            let twin = qs
                .iter()
                .find(|q| q.id == twin_id)
                .unwrap_or_else(|| panic!("{} has no file twin {twin_id}", p.id));
            assert!(!twin.stdin);
            assert_eq!(
                (&twin.flags, &twin.filter),
                (&p.flags, &p.filter),
                "{}",
                p.id
            );
            for shape in &p.shapes {
                assert!(twin.shapes.contains(shape), "{twin_id} lacks shape {shape}");
            }
            assert!(
                tier_rank(&p.max_tier) <= tier_rank(&twin.max_tier),
                "{} runs a tier its twin does not",
                p.id
            );
        }
    }

    #[test]
    fn checked_in_queries_parse_and_name_known_shapes() {
        let text = include_str!("../../../../comparison/jq/queries/perf.tsv");
        let qs = parse_queries(text).unwrap();
        assert!(qs.len() >= 25);
        // A shape no input has would run no row and say nothing (#5552): the shapes are the
        // manifest's own, so a corpus file added there is not also listed here.
        let (m, _) = super::super::manifest::Manifest::parse(include_str!(
            "../../../../comparison/jq/manifest.toml"
        ))
        .unwrap();
        for s in qs.iter().flat_map(|q| &q.shapes) {
            assert!(
                s == "none" || m.inputs.iter().any(|i| &i.shape == s),
                "no input has shape {s}"
            );
        }
        // And the other way: an input no query uses is dead weight in the manifest.
        for i in &m.inputs {
            assert!(
                qs.iter().any(|q| q.shapes.contains(&i.shape)),
                "no query uses {}",
                i.id
            );
        }
    }

    fn corpus_spec(file: &str, content: &[u8]) -> InputSpec {
        InputSpec {
            id: "c".into(),
            shape: "corpus-c".into(),
            tier: "1mb".into(),
            pattern: String::new(),
            size: String::new(),
            seed: 0,
            sha256: sha256_hex(content),
            corpus: Some(file.into()),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("compare-perf-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A corpus input is copied from the seed or the fetched tree (the seed first), checked
    /// against its pin, and refused when the file changed or is missing.
    #[test]
    fn a_corpus_input_is_copied_from_the_seed_or_the_fetch_and_checked_against_its_pin() {
        let root = scratch("corpus");
        let out = root.join("out");
        let spec = corpus_spec("json/x/a.json", b"{\"a\":1}\n");
        let gen = Path::new("/nonexistent/generator");
        assert!(!available(&root, &spec));
        let err = ensure_input(&root, gen, &out, &spec)
            .unwrap_err()
            .to_string();
        assert!(err.contains("sync-bench-corpus.sh"), "{err}");

        let fetched = root.join("data/bench/corpus/json/x");
        std::fs::create_dir_all(&fetched).unwrap();
        std::fs::write(fetched.join("a.json"), b"{\"a\":1}\n").unwrap();
        assert!(available(&root, &spec));
        let path = ensure_input(&root, gen, &out, &spec).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"a\":1}\n");

        // A stale copy from an earlier run is replaced, not judged.
        std::fs::write(&path, b"stale").unwrap();
        ensure_input(&root, gen, &out, &spec).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"a\":1}\n");

        // The seed wins over the fetch, and a file that differs from the pin is refused.
        let seed = root.join("tests/data/bench-corpus/seed/json/x");
        std::fs::create_dir_all(&seed).unwrap();
        std::fs::write(seed.join("a.json"), b"{\"a\":2}\n").unwrap();
        assert_eq!(
            corpus_source(&root, &spec).unwrap(),
            seed.join("a.json"),
            "the vendored seed is the first candidate"
        );
        std::fs::remove_file(&path).unwrap();
        let err = ensure_input(&root, gen, &out, &spec)
            .unwrap_err()
            .to_string();
        assert!(err.contains("manifest pins"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_instruction_count_is_read_from_cachegrinds_log() {
        let log = "==12== Cachegrind, a high-precision tracing profiler\n\
                   ==12== \n==12== I   refs:      1,234,567\n";
        assert_eq!(parse_ir(log), Some(1_234_567));
        assert_eq!(parse_ir("==12== I refs: 5"), None);
        assert_eq!(parse_ir("no counts here"), None);
        assert_eq!(parse_ir("==1== I   refs:      x"), None);
    }

    /// A stand-in `valgrind` that writes the log it is asked for: the run's arguments, working
    /// directory and failure handling are the harness's, not valgrind's.
    #[cfg(unix)]
    #[test]
    fn a_failed_or_silent_valgrind_gives_no_count() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("valgrind");
        let stub = |name: &str, body: &str| {
            let p = dir.join(name);
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        let log = r#"for a in "$@"; do case $a in --log-file=*) f=${a#--log-file=};; esac; done"#;
        let tool = Tool {
            name: "jq-1.7.1 (control)".into(),
            program: "true".into(),
            prefix: vec![],
        };
        // A stub written a moment ago can fail to exec with ETXTBSY: another test's `fork` in
        // this binary inherits the stub's write descriptor until that child's own `exec` closes
        // it (#5970). `ir_once` reports the failed spawn as no count, so a stub expected to
        // count is run again; a rename into place would not help, the inode is the same.
        let counted = |valgrind: &Path| {
            let mut got = None;
            for _ in 0..100 {
                got = ir_once(valgrind, &tool, &[], &dir, None);
                if got.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            got
        };
        let ok = stub(
            "ok",
            &format!("{log}\necho '==1== I   refs:      42' > \"$f\""),
        );
        assert_eq!(counted(&ok), Some(42));
        // The traced program's own non-zero exit passes through valgrind; its count is valid.
        let failing = stub(
            "failing",
            &format!("{log}\necho '==1== I   refs:      42' > \"$f\"\nexit 1"),
        );
        assert_eq!(counted(&failing), Some(42));
        let silent = stub("silent", "exit 0");
        assert_eq!(ir_once(&silent, &tool, &[], &dir, None), None);
        assert_eq!(ir_once(&dir.join("absent"), &tool, &[], &dir, None), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_generated_input_is_always_available() {
        assert!(available(
            Path::new("/nonexistent"),
            &spec("u", "users", "1mb")
        ));
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
        let kb = private_once(&tool, &args, Path::new("."), None, None).expect("sampled");
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

    /// Fast or slow is a comparison with the threshold, so a threshold no spawn can exceed makes the
    /// row fast whatever the host's load (#5931: spawning `sh` took over 15 ms at load average 25
    /// and the row was topped up to 21 repetitions). The real 15 ms threshold is pinned against fixed
    /// timings by `only_a_slow_row_is_topped_up_and_the_extra_time_is_capped`.
    #[test]
    fn a_fast_row_keeps_the_requested_repetitions() {
        let q = parse_queries("q\t-\tx\tnone\t1mb\n").unwrap().remove(0);
        let t = sh("t", ":");
        let row =
            run_row_over(&[t.clone(), t], &q, None, Path::new("."), 3, None, f64::MAX).unwrap();
        assert_eq!(row.reps, 3);
        assert!(!row.ratio_from_min, "a fast row keeps the median ratio");
        assert!(row.tools.iter().all(|t| t.stats.as_ref().unwrap().n == 3));
    }

    /// The bytes arrive whole through the pipe, however much the pipe buffer holds (a pipe holds
    /// 64 KiB on Linux, so 1 MiB needs the writer to run beside the reader), and a tool that
    /// never reads does not hang the writer.
    #[cfg(unix)]
    #[test]
    fn a_fed_run_delivers_every_byte_and_a_tool_that_stops_reading_does_not_hang() {
        let dir = scratch("feed");
        let bytes: Vec<u8> = (0..1_048_576u32).map(|i| (i % 251) as u8).collect();
        let out = dir.join("got");
        let cat = sh("cat", "cat > \"$1\"");
        time_once(
            &cat,
            &[out.display().to_string()],
            &dir,
            Some(bytes.as_slice()),
        )
        .unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), bytes);
        // The untimed memory run reads it too: `/usr/bin/time` passes its stdin on.
        std::fs::remove_file(&out).unwrap();
        let report = time_report(
            &cat,
            &[out.display().to_string()],
            &dir,
            Some(bytes.as_slice()),
        );
        assert!(report.is_some());
        assert_eq!(std::fs::read(&out).unwrap(), bytes);
        let quitter = sh("true", ":");
        time_once(&quitter, &[], &dir, Some(bytes.as_slice())).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stdin row's every run, the gate's included, gives the tool a pipe and the whole input:
    /// a tool that finds a regular file or a short read on fd 0 records it in a marker file.
    #[cfg(unix)]
    #[test]
    fn a_stdin_row_pipes_the_input_through_every_kind_of_run() {
        let dir = scratch("stdin-row");
        let input = b"{\"users\":[1,2,3]}\n".repeat(50_000);
        std::fs::write(dir.join("u.json"), &input).unwrap();
        let marker = dir.join("marker");
        let script = format!(
            "[ -p /dev/stdin ] || echo notpipe >> {m}; n=$(wc -c); [ \"$n\" -eq {len} ] || echo short >> {m}; echo \"$n\"",
            m = marker.display(),
            len = input.len()
        );
        let tool = sh("t", &script);
        let q = parse_queries("t_stdin\t@stdin\tx\tusers\t1mb\n")
            .unwrap()
            .remove(0);
        let row = run_row_over(
            &[tool.clone(), tool],
            &q,
            Some(&spec("u", "users", "1mb")),
            &dir,
            3,
            None,
            f64::MAX,
        )
        .unwrap();
        assert_eq!(row.gate, Gate::Match);
        assert!(row.speedup_primary.is_some());
        assert!(
            !marker.exists(),
            "{}",
            std::fs::read_to_string(&marker).unwrap_or_default()
        );
        // The same row as a file row names the file and pipes nothing.
        let file_q = parse_queries("t\t-\tx\tusers\t1mb\n").unwrap().remove(0);
        let sniff = sh("t", "[ -p /dev/stdin ] && echo piped; echo \"$2\"");
        let row = run_row_over(
            &[sniff.clone(), sniff],
            &file_q,
            Some(&spec("u", "users", "1mb")),
            &dir,
            1,
            None,
            f64::MAX,
        )
        .unwrap();
        assert_eq!(row.gate, Gate::Match);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The injected threshold is the only thing that decides: the same timings are fast over one
    /// threshold and slow over another.
    #[test]
    fn the_slow_row_threshold_can_be_named_by_the_caller() {
        let samples = vec![vec![40.0; 3], vec![50.0; 3]];
        assert!(is_slow(&samples));
        assert!(is_slow_over(&samples, SLOW_ROW_MS));
        assert!(!is_slow_over(&samples, 60.0));
        assert_eq!(extra_reps_over(&samples, 3, 60.0), 0);
        assert_eq!(extra_reps_over(&samples, 3, SLOW_ROW_MS), SLOW_ROW_REPS - 3);
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

    /// The opt-in tier is reached only by naming it: a `1gb` cap does not make a row run under
    /// `--full`'s `100mb` top (#6915).
    #[test]
    fn the_one_gigabyte_tier_runs_only_when_it_is_the_top_tier() {
        let qs = parse_queries("a\t-\t.\tusers\t1gb\nb\t-\t.\tusers\t100mb\n").unwrap();
        let inputs = [
            spec("u100", "users", "100mb"),
            spec("u1g", "users", HUGE_TIER),
        ];
        let ids = |top: &str| -> Vec<String> {
            plan(&qs, &inputs, top)
                .iter()
                .map(|(q, i)| format!("{}@{}", q.id, i.map_or("none", |i| i.id.as_str())))
                .collect()
        };
        assert_eq!(ids("100mb"), ["a@u100", "b@u100"]);
        assert_eq!(ids(HUGE_TIER), ["a@u100", "a@u1g", "b@u100"]);
        assert!(parse_queries("a\t-\t.\tusers\t1tb\n").is_err());
    }

    /// The six rows #6915 names are the ones capped at `1gb`, and each has a `1gb` input to run
    /// on: a row capped there with no input would run nothing and say nothing.
    #[test]
    fn the_checked_in_one_gigabyte_rows_are_the_six_the_issue_names_and_each_has_an_input() {
        let qs = parse_queries(include_str!("../../../../comparison/jq/queries/perf.tsv")).unwrap();
        let (m, _) = super::super::manifest::Manifest::parse(include_str!(
            "../../../../comparison/jq/manifest.toml"
        ))
        .unwrap();
        let mut capped: Vec<&str> = qs
            .iter()
            .filter(|q| q.max_tier == HUGE_TIER)
            .map(|q| q.id.as_str())
            .collect();
        capped.sort_unstable();
        assert_eq!(
            capped,
            [
                "identity",
                "identity_compact",
                "ndjson_field",
                "ndjson_inputs_first",
                "ndjson_inputs_reduce",
                "users_first_early_exit"
            ]
        );
        let planned = plan(&qs, &m.inputs, HUGE_TIER);
        for id in &capped {
            assert!(
                planned
                    .iter()
                    .any(|(q, i)| q.id == *id && i.is_some_and(|i| i.tier == HUGE_TIER)),
                "{id} has no 1gb input"
            );
        }
        // And nothing else reaches it: a plain `--full` plans no 1gb row.
        assert!(plan(&qs, &m.inputs, "100mb")
            .iter()
            .all(|(_, i)| i.map_or(true, |i| i.tier != HUGE_TIER)));
        for i in m.inputs.iter().filter(|i| i.tier == HUGE_TIER) {
            assert_eq!(i.sha256.len(), 64, "{} is pinned", i.id);
        }
    }

    fn huge_setup(name: &str) -> (PathBuf, PerfQuery, InputSpec) {
        let dir = scratch(name);
        std::fs::write(dir.join("u.json"), b"{\"users\":[]}\n").unwrap();
        let q = parse_queries("q\t-\tx\tusers\t1gb\n").unwrap().remove(0);
        (dir, q, spec("u", "users", HUGE_TIER))
    }

    /// A 1 GB row that runs cleanly is timed like any other, and records how each tool ended
    /// and how big the input was; a smaller row records neither, so its json is unchanged.
    #[cfg(unix)]
    #[test]
    fn a_clean_huge_row_records_exit_status_and_input_size_and_a_small_row_does_not() {
        let (dir, q, input) = huge_setup("huge-clean");
        let t = sh("t", "echo ok");
        let row = run_row_over(
            &[t.clone(), t.clone()],
            &q,
            Some(&input),
            &dir,
            3,
            None,
            f64::MAX,
        )
        .unwrap();
        assert_eq!(row.gate, Gate::Match);
        assert_eq!(row.input_bytes, Some(13));
        let clean = RunEnd {
            exit: Some(0),
            signal: None,
            timed_out: false,
        };
        for tr in &row.tools {
            assert_eq!(tr.end.as_ref(), Some(&clean));
            assert_eq!(tr.stats.as_ref().unwrap().n, 3, "three timed runs");
        }
        // The same program on a 1 MB input leaves both fields out of the json.
        let small = run_row_over(
            &[t.clone(), t],
            &q,
            Some(&spec("u", "users", "1mb")),
            &dir,
            3,
            None,
            f64::MAX,
        )
        .unwrap();
        assert!(small.input_bytes.is_none() && small.tools.iter().all(|t| t.end.is_none()));
        let json = serde_json::to_value(&small).unwrap();
        assert!(json.get("input_bytes").is_none());
        assert!(json["tools"][0].get("end").is_none());
        // And a clean end omits its optional keys.
        assert_eq!(
            serde_json::to_value(&clean).unwrap(),
            serde_json::json!({"exit": 0})
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A subject the kernel killed is a result on a 1 GB row: the row is `Incomplete`, untimed,
    /// with the signal, the reference's clean exit and a peak-memory entry for each tool.
    #[cfg(unix)]
    #[test]
    fn a_tool_killed_on_a_huge_row_is_an_incomplete_row_with_its_signal() {
        let (dir, q, input) = huge_setup("huge-killed");
        let (subject, reference) = (sh("sub", "kill -9 $$"), sh("ref", "echo ok"));
        let row = run_row_over(
            &[subject, reference],
            &q,
            Some(&input),
            &dir,
            3,
            None,
            f64::MAX,
        )
        .unwrap();
        assert_eq!(row.gate, Gate::Incomplete);
        assert!(row.speedup_primary.is_none() && row.reps == 0);
        assert_eq!(row.tools.len(), 2, "both tools are listed");
        let (s, r) = (
            row.tools[0].end.as_ref().unwrap(),
            row.tools[1].end.as_ref().unwrap(),
        );
        assert_eq!((s.signal, s.exit, s.killed()), (Some(9), None, true));
        assert_eq!((r.signal, r.exit, r.killed()), (None, Some(0), false));
        assert!(row.tools.iter().all(|t| t.stats.is_none()));
        // The same kill on a 1 MB row is an output difference, as it always was.
        let small = run_row_over(
            &[sh("sub", "kill -9 $$"), sh("ref", "echo ok")],
            &q,
            Some(&spec("u", "users", "1mb")),
            &dir,
            3,
            None,
            f64::MAX,
        )
        .unwrap();
        assert_eq!(small.gate, Gate::Differs);
        assert!(small.tools.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A 1 GB tool past the deadline is recorded as having run past it, where a smaller row's
    /// deadline still aborts the run.
    #[cfg(unix)]
    #[test]
    fn a_huge_row_records_a_deadline_where_a_small_row_aborts() {
        let (dir, q, input) = huge_setup("huge-deadline");
        let slow = || sh("slow", "sleep 5");
        let ok = || sh("ref", "echo ok");
        let short = std::time::Duration::from_millis(200);
        let row = run_row_bounded(
            &[slow(), ok()],
            &q,
            Some(&input),
            &dir,
            3,
            None,
            f64::MAX,
            short,
        )
        .unwrap();
        assert_eq!(row.gate, Gate::Incomplete);
        let end = row.tools[0].end.as_ref().unwrap();
        assert!(end.timed_out && end.killed() && end.exit.is_none());
        let err = run_row_bounded(
            &[slow(), ok()],
            &q,
            Some(&spec("u", "users", "1mb")),
            &dir,
            3,
            None,
            f64::MAX,
            short,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("did not finish"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A refusal is a result too: both tools finished but exited differently, so the row stays a
    /// correctness row, and on a 1 GB row it still lists each tool's exit status.
    #[cfg(unix)]
    #[test]
    fn a_refusal_on_a_huge_row_keeps_its_exit_status() {
        let (dir, q, input) = huge_setup("huge-refusal");
        let row = run_row_over(
            &[sh("sub", "exit 5"), sh("ref", "echo ok")],
            &q,
            Some(&input),
            &dir,
            3,
            None,
            f64::MAX,
        )
        .unwrap();
        assert_eq!(row.gate, Gate::Differs);
        assert!(row.speedup_primary.is_none());
        let exits: Vec<Option<i32>> = row
            .tools
            .iter()
            .map(|t| t.end.as_ref().unwrap().exit)
            .collect();
        assert_eq!(exits, [Some(5), Some(0)]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
