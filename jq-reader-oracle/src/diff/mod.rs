//! The differential harness: `succinctly jq` against jq 1.7.1's and jq 1.8.2's own readers.
//!
//! For each generated case ([`gen`]), it writes the files, runs both C readers in-process
//! ([`oracle`]) and the `succinctly` binary as a subprocess, and classifies the difference
//! ([`classify`]). With `--jq171`/`--jq182` it also spawns the real jq binaries on every `k`-th
//! case and counts where the in-process reader disagrees with them, which checks the oracle on the
//! shapes this generator makes. Differing cases go to `cases.jsonl`; up to `--keep` per signature
//! (class, family, flags, kind) are minimised ([`minimize`]) and written as repros.
//!
//! Exit status: 0 when no case is a bug, 1 when one is, 3 when the oracle disagreed with a real jq
//! binary, 2 on a setup error. That is the contract of a `[[sweep]]` entry of `succinctly dev
//! compare jq` (`comparison/jq/manifest.toml`), which is how a fixed-seed 3,000-case subset can
//! join it.

pub mod classify;
pub mod gen;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use classify::{Class, Kind};
use gen::{Case, Entry, Family, Generator};

use crate::reader::{self, Item, Opts, Reader, Release, Transcript, UtilReader};

/// Items the trace program reads at most (`Malformed BOM` repeats forever under `input`).
pub const TRACE_LIMIT: u64 = 64;

/// How the case's output carries its values.
fn shape(case: &Case, cfg: &Config) -> classify::Shape {
    classify::Shape {
        trace: case.flags.null_input,
        slurp: case.flags.slurp,
        seq: case.flags.seq,
        lenient_e1: cfg.lenient_e1,
    }
}

/// The jq program a case runs.
pub fn program(case: &Case) -> String {
    if case.flags.null_input {
        reader::trace_program(TRACE_LIMIT)
    } else {
        ".".into()
    }
}

/// A case written to disk: the directory, the files' absolute paths, and the stdin file.
struct Staged {
    paths: Vec<String>,
    stdin: Option<PathBuf>,
}

fn stage(dir: &Path, case: &Case) -> std::io::Result<Staged> {
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::create_dir_all(dir)?;
    let mut paths = Vec::new();
    for f in &case.files {
        let p = dir.join(&f.name);
        if let Some(d) = &f.data {
            std::fs::write(&p, d)?;
        }
        paths.push(p.to_string_lossy().into_owned());
    }
    let stdin = match case.entry {
        Entry::Files => None,
        Entry::StdinFile | Entry::StdinPipe => {
            let p = dir.join("stdin.json");
            std::fs::write(&p, case.stream())?;
            Some(p)
        }
    };
    Ok(Staged { paths, stdin })
}

/// Both C readers on a staged case.
pub mod oracle {
    use super::*;

    /// What jq `release` prints for `case`, from its own reader in-process, and how many items
    /// its reader gave before the first parse error (`None` when it gave none). Standard input
    /// is read from the staged stdin file: jq's `fgets` loop reads a pipe and a file alike, and
    /// only `input_filename` tells them apart (jq names standard input `"<stdin>"`).
    pub(super) fn run(
        release: Release,
        case: &Case,
        staged: &Staged,
    ) -> reader::Result<(Transcript, Option<usize>)> {
        let opts = Opts {
            flags: if case.flags.seq { reader::PARSE_SEQ } else { 0 },
            raw: false,
            slurp: case.flags.slurp,
        };
        let files: Vec<String> = match &staged.stdin {
            Some(p) => vec![p.to_string_lossy().into_owned()],
            None => staged.paths.clone(),
        };
        let mut r = Counting {
            inner: UtilReader::new(release, opts, &files)?,
            values: 0,
            before_error: None,
        };
        let mut t = if case.flags.null_input {
            reader::run_trace(&mut r, TRACE_LIMIT, case.flags.seq)?
        } else {
            reader::run_main(&mut r, case.flags.seq)?
        };
        if let Some(p) = &staged.stdin {
            let name = reader::json_string(p.to_string_lossy().as_bytes())?;
            t.stdout = replace(&t.stdout, &name, b"\"<stdin>\"");
        }
        Ok((t, r.before_error))
    }

    /// A reader that counts the values it gave before its first error.
    struct Counting<R> {
        inner: R,
        values: usize,
        before_error: Option<usize>,
    }

    impl<R: Reader> Reader for Counting<R> {
        fn next(&mut self) -> reader::Result<Option<Item>> {
            let item = self.inner.next()?;
            match &item {
                Some(Item::Value(_)) => self.values += 1,
                Some(Item::Error(_)) => {
                    self.before_error.get_or_insert(self.values);
                }
                None => {}
            }
            Ok(item)
        }
        fn errors(&self) -> i32 {
            self.inner.errors()
        }
        fn take_errs(&mut self) -> Vec<u8> {
            self.inner.take_errs()
        }
        fn filename(&self) -> reader::Result<Option<Vec<u8>>> {
            self.inner.filename()
        }
        fn line(&self) -> u64 {
            self.inner.line()
        }
    }
}

/// `case` with every escape jq's parser rejects made valid in place
/// ([`classify::escapes_made_valid`]), file by file.
fn escapes_made_valid(case: &Case) -> Case {
    with_stream(case, &classify::escapes_made_valid(&case.stream()))
}

/// `case` with its files' bytes replaced, in order, by `stream`'s, which is as long as
/// `case.stream()`.
fn with_stream(case: &Case, stream: &[u8]) -> Case {
    let mut at = 0;
    let mut c = case.clone();
    for f in &mut c.files {
        if let Some(d) = f.data.as_mut() {
            let n = d.len();
            d.copy_from_slice(&stream[at..at + n]);
            at += n;
        }
    }
    c
}

fn replace(hay: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(hay.len());
    let mut i = 0;
    while i < hay.len() {
        if hay[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(hay[i]);
            i += 1;
        }
    }
    out
}

/// Run a jq-like binary (`succinctly jq` or a real jq) on a staged case. `prefix` is the
/// arguments before jq's own (`["jq"]` for succinctly).
fn spawn(
    bin: &Path,
    prefix: &[&str],
    case: &Case,
    staged: &Staged,
    dir: &Path,
) -> std::io::Result<Transcript> {
    let out_path = dir.join(".stdout");
    let err_path = dir.join(".stderr");
    let mut cmd = Command::new(bin);
    cmd.args(prefix)
        .arg("-c")
        .args(case.flags.args())
        .arg(program(case))
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .env("HOME", dir)
        .stdout(std::fs::File::create(&out_path)?)
        .stderr(std::fs::File::create(&err_path)?);
    if case.entry == Entry::Files {
        cmd.args(&staged.paths).stdin(Stdio::null());
    }
    match (&staged.stdin, case.entry) {
        (Some(p), Entry::StdinFile) => {
            cmd.stdin(std::fs::File::open(p)?);
        }
        (Some(_), Entry::StdinPipe) => {
            cmd.stdin(Stdio::piped());
        }
        _ => {}
    }
    limit_child(&mut cmd);
    let mut child = cmd.spawn()?;
    let writer = match (case.entry, child.stdin.take()) {
        (Entry::StdinPipe, Some(mut w)) => {
            let data = case.stream();
            Some(std::thread::spawn(move || {
                let _ = w.write_all(&data);
            }))
        }
        _ => None,
    };
    let start = Instant::now();
    let mut nap = Duration::from_micros(200);
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if start.elapsed() > Duration::from_secs(10) {
            // The child leads its own process group (`limit_child`): kill the whole group.
            let pgid = libc::pid_t::try_from(child.id()).unwrap_or(0);
            if pgid > 0 {
                // SAFETY: a plain syscall on a process group this harness created.
                unsafe { libc::killpg(pgid, libc::SIGKILL) };
            }
            let _ = child.kill();
            let _ = child.wait();
            if let Some(w) = writer {
                let _ = w.join();
            }
            return Ok(Transcript {
                stdout: Vec::new(),
                stderr: b"<timeout after 10 s>".to_vec(),
                status: -2,
            });
        }
        std::thread::sleep(nap);
        nap = (nap * 2).min(Duration::from_millis(5));
    };
    if let Some(w) = writer {
        let _ = w.join();
    }
    Ok(Transcript {
        stdout: std::fs::read(&out_path)?,
        stderr: std::fs::read(&err_path)?,
        status: exit_code(status),
    })
}

#[cfg(unix)]
fn exit_code(s: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    s.code().unwrap_or_else(|| 128 + s.signal().unwrap_or(0))
}

/// Each child gets its own process group (killed whole on a timeout), no core dumps (a limit of
/// 1, which a piped `core_pattern` also honours) and 2 GiB of address space.
fn limit_child(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
    // SAFETY: `setrlimit` is async-signal-safe; nothing else runs between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            let core = libc::rlimit {
                rlim_cur: 1,
                rlim_max: 1,
            };
            libc::setrlimit(libc::RLIMIT_CORE, &core);
            #[cfg(target_os = "linux")]
            {
                let space = libc::rlimit {
                    rlim_cur: 2 << 30,
                    rlim_max: 2 << 30,
                };
                libc::setrlimit(libc::RLIMIT_AS, &space);
            }
            Ok(())
        });
    }
}

/// One case's outcome.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub case: Case,
    pub class: Class,
    /// How succinctly differs from 1.7.1 (the pin).
    pub kind: Kind,
    pub subject: Transcript,
    pub jq171: Transcript,
    pub jq182: Transcript,
    /// The real jq binaries' disagreements with the in-process readers, when checked.
    pub oracle_mismatch: Vec<(Release, Transcript)>,
    /// The recorded divergence, for [`Class::Recorded`].
    pub reason: Option<&'static str>,
}

/// The harness's settings.
#[derive(Debug, Clone)]
pub struct Config {
    pub subject: PathBuf,
    pub generator: Generator,
    pub seed: u64,
    pub start: u64,
    pub cases: u64,
    pub jobs: usize,
    pub jq171: Option<PathBuf>,
    pub jq182: Option<PathBuf>,
    pub verify_every: u64,
    pub out: Option<PathBuf>,
    pub keep: usize,
    pub minimize_evals: usize,
    pub tmp: PathBuf,
    /// Run the C readers only (the subject is taken to equal jq 1.7.1): their rate alone.
    pub c_only: bool,
    /// Spike E's line-prefix E1, which also accepts values lost before a terminating fault.
    pub lenient_e1: bool,
}

/// Run one case: stage it in `dir`, run both C readers and the subject, and classify.
pub fn run_case(cfg: &Config, case: Case, dir: &Path) -> Result<Outcome, String> {
    let staged = stage(dir, &case).map_err(|e| format!("staging case {}: {e}", case.index))?;
    let (jq171, k171) = oracle::run(Release::V171, &case, &staged).map_err(|e| e.to_string())?;
    let (jq182, k182) = oracle::run(Release::V182, &case, &staged).map_err(|e| e.to_string())?;
    let subject = if cfg.c_only {
        jq171.clone()
    } else {
        spawn(&cfg.subject, &["jq"], &case, &staged, dir)
            .map_err(|e| format!("running {}: {e}", cfg.subject.display()))?
    };
    let mut oracle_mismatch = Vec::new();
    if cfg.verify_every > 0 && case.index.is_multiple_of(cfg.verify_every) {
        for (release, bin, want) in [
            (Release::V171, &cfg.jq171, &jq171),
            (Release::V182, &cfg.jq182, &jq182),
        ] {
            if let Some(bin) = bin {
                let got = spawn(bin, &[], &case, &staged, dir)
                    .map_err(|e| format!("running {}: {e}", bin.display()))?;
                if &got != want {
                    oracle_mismatch.push((release, got));
                }
            }
        }
    }
    let stream = case.stream();
    let escapes_valid = if classify::bad_escape(&stream) {
        let valid = escapes_made_valid(&case);
        let staged = stage(&dir.join("valid"), &valid).map_err(|e| e.to_string())?;
        let run = |r| oracle::run(r, &valid, &staged).map(|o| o.0);
        Some((
            run(Release::V171).map_err(|e| e.to_string())?,
            run(Release::V182).map_err(|e| e.to_string())?,
        ))
    } else {
        None
    };
    let nul_twin = match classify::nul_twin(&stream, case.flags.seq) {
        Some(twin) => {
            let twin = with_stream(&case, &twin);
            let staged = stage(&dir.join("twin"), &twin).map_err(|e| e.to_string())?;
            let run = |r| oracle::run(r, &twin, &staged).map(|o| o.0);
            Some((
                run(Release::V171).map_err(|e| e.to_string())?,
                run(Release::V182).map_err(|e| e.to_string())?,
            ))
        }
        None => None,
    };
    let facts = classify::Facts {
        before_fault: [k171, k182],
        before_nul: classify::values_before_nul(&stream, case.flags.seq)
            .map_err(|e| e.to_string())?,
        escapes_valid,
        nul_twin,
    };
    let class = classify::classify(&stream, &subject, &jq171, &jq182, shape(&case, cfg), &facts);
    let kind = classify::kind(&subject, &jq171);
    let reason = if class == Class::Recorded {
        classify::recorded(&stream, &subject, &jq171, &jq182, shape(&case, cfg), &facts)
    } else {
        None
    };
    Ok(Outcome {
        case,
        class,
        kind,
        reason,
        subject,
        jq171,
        jq182,
        oracle_mismatch,
    })
}

/// Shrink `case`'s files while it keeps `class` and `kind`: delete chunks of each file, halving the
/// chunk size, for at most `evals` runs. Deleting filler moves the edge, so a read-boundary case
/// keeps most of its filler; what goes is everything the difference does not need.
pub fn minimize(
    cfg: &Config,
    case: &Case,
    class: Class,
    kind: Kind,
    dir: &Path,
    evals: usize,
) -> Case {
    let mut best = case.clone();
    let mut left = evals;
    let holds = |c: &Case, left: &mut usize| -> bool {
        if *left == 0 {
            return false;
        }
        *left -= 1;
        let mut probe = cfg.clone();
        probe.verify_every = 0;
        matches!(run_case(&probe, c.clone(), dir), Ok(o) if o.class == class && o.kind == kind)
    };
    // Whole files first (not the first: it carries the edge).
    let mut i = 1;
    while i < best.files.len() {
        let mut c = best.clone();
        c.files.remove(i);
        if holds(&c, &mut left) {
            best = c;
        } else {
            i += 1;
        }
    }
    for fi in 0..best.files.len() {
        let Some(len) = best.files[fi].data.as_ref().map(Vec::len) else {
            continue;
        };
        let mut chunk = len.next_power_of_two() / 2;
        while chunk >= 1 && left > 0 {
            let mut at = 0;
            while left > 0 {
                let cur = best.files[fi].data.as_ref().map_or(0, Vec::len);
                if at >= cur {
                    break;
                }
                let mut c = best.clone();
                if let Some(d) = c.files[fi].data.as_mut() {
                    d.drain(at..(at + chunk).min(cur));
                }
                if holds(&c, &mut left) {
                    best = c;
                } else {
                    at += chunk;
                }
            }
            if chunk == 1 {
                break;
            }
            chunk /= 2;
        }
    }
    best
}

/// A Python expression for `bytes`: runs of 8 or more of one byte as `b'x'*n`.
pub fn py_bytes(bytes: &[u8]) -> String {
    let mut parts = Vec::new();
    let mut lit = String::new();
    let mut i = 0;
    let flush = |lit: &mut String, parts: &mut Vec<String>| {
        if !lit.is_empty() {
            parts.push(format!("b'{lit}'"));
            lit.clear();
        }
    };
    while i < bytes.len() {
        let b = bytes[i];
        let run = bytes[i..].iter().take_while(|&&c| c == b).count();
        if run >= 8 {
            flush(&mut lit, &mut parts);
            parts.push(format!("b'{}'*{run}", esc(b)));
            i += run;
        } else {
            lit.push_str(&esc(b));
            i += 1;
        }
    }
    flush(&mut lit, &mut parts);
    if parts.is_empty() {
        "b''".into()
    } else {
        parts.join(" + ")
    }
}

fn esc(b: u8) -> String {
    match b {
        b'\\' => "\\\\".into(),
        b'\'' => "\\'".into(),
        b'\n' => "\\n".into(),
        b'\r' => "\\r".into(),
        b'\t' => "\\t".into(),
        0x20..=0x7e => (b as char).to_string(),
        _ => format!("\\x{b:02x}"),
    }
}

/// A repro as a shell script: write the files, then run each tool the way the harness did.
pub fn repro_script(case: &Case) -> String {
    let mut s = String::from(
        "#!/bin/sh\n# Generated by jq-reader-oracle diff; run in an empty directory.\n",
    );
    for f in &case.files {
        if let Some(d) = &f.data {
            let _ = writeln!(
                s,
                // A quoted heredoc: the shell leaves the Python literal alone.
                "python3 - > {1} <<'PY'\nimport sys; sys.stdout.buffer.write({0})\nPY",
                py_bytes(d),
                f.name
            );
        }
    }
    let prog = program(case).replace('\'', "'\\''");
    let flags = case.flags.args().join(" ");
    let names: Vec<&str> = case.files.iter().map(|f| f.name.as_str()).collect();
    let cat: Vec<&str> = case
        .files
        .iter()
        .filter(|f| f.data.is_some())
        .map(|f| f.name.as_str())
        .collect();
    let (pre, post) = match case.entry {
        Entry::Files => (String::new(), format!(" {}", names.join(" "))),
        Entry::StdinFile => {
            let _ = writeln!(s, "cat {} > stdin.json", cat.join(" "));
            (String::new(), " < stdin.json".into())
        }
        Entry::StdinPipe => (format!("cat {} | ", cat.join(" ")), String::new()),
    };
    for tool in [
        "${JQ171:-jq}",
        "${JQ182:-jq-1.8.2}",
        "${SUCCINCTLY:-succinctly} jq",
    ] {
        let _ = writeln!(s, "{pre}{tool} -c {flags} '{prog}'{post}; echo \"exit $?\"");
    }
    s
}

pub fn show(t: &Transcript) -> String {
    let cut = |b: &[u8]| String::from_utf8_lossy(&b[..b.len().min(400)]).into_owned();
    format!(
        "rc={} stdout={:?} stderr={:?}",
        t.status,
        cut(&t.stdout),
        cut(&t.stderr)
    )
}

fn json_line(o: &Outcome) -> String {
    let tr = |t: &Transcript| {
        serde_json::json!({
            "rc": t.status,
            "stdout": String::from_utf8_lossy(&t.stdout[..t.stdout.len().min(600)]),
            "stdout_len": t.stdout.len(),
            "stderr": String::from_utf8_lossy(&t.stderr[..t.stderr.len().min(600)]),
        })
    };
    serde_json::json!({
        "index": o.case.index,
        "class": o.class.name(),
        "kind": o.kind.name(),
        "reason": o.reason,
        "family": o.case.family.name(),
        "edge": o.case.edge.name(),
        "flags": o.case.flags.name(),
        "entry": o.case.entry.name(),
        "shape": o.case.shape,
        "s": tr(&o.subject),
        "j171": tr(&o.jq171),
        "j182": tr(&o.jq182),
    })
    .to_string()
}

/// A signature groups like differences for the report and for keeping repros.
fn signature(o: &Outcome) -> Signature {
    (
        o.class,
        o.case.family,
        o.case.flags.name(),
        o.kind,
        o.reason.unwrap_or(""),
    )
}

/// (class, family, flags, kind against 1.7.1, recorded reason).
type Signature = (Class, Family, String, Kind, &'static str);

/// Run the harness; returns the exit status.
pub fn run(cfg: &Config) -> i32 {
    match run_inner(cfg) {
        Ok(rc) => rc,
        Err(e) => {
            eprintln!("jq-reader-oracle diff: {e}");
            2
        }
    }
}

fn run_inner(cfg: &Config) -> Result<i32, String> {
    if !cfg.c_only && !cfg.subject.is_file() {
        return Err(format!("no subject binary at {}", cfg.subject.display()));
    }
    std::fs::create_dir_all(&cfg.tmp).map_err(|e| format!("{}: {e}", cfg.tmp.display()))?;
    if let Some(out) = &cfg.out {
        std::fs::create_dir_all(out.join("repros"))
            .map_err(|e| format!("{}: {e}", out.display()))?;
    }
    // Warm both releases up on this thread first: jq initialises its hash seed and decNumber
    // tables lazily.
    {
        let warm = cfg.tmp.join("warm");
        let case = cfg.generator.case(cfg.seed, cfg.start);
        let staged = stage(&warm, &case).map_err(|e| e.to_string())?;
        for r in Release::ALL {
            oracle::run(r, &case, &staged).map_err(|e| e.to_string())?;
        }
    }
    let next = Arc::new(AtomicU64::new(cfg.start));
    let end = cfg.start + cfg.cases;
    let (tx, rx) = mpsc::channel::<Result<Outcome, String>>();
    let t0 = Instant::now();
    let mut workers = Vec::new();
    for w in 0..cfg.jobs.max(1) {
        let (tx, next, cfg) = (tx.clone(), Arc::clone(&next), cfg.clone());
        workers.push(std::thread::spawn(move || {
            let dir = cfg.tmp.join(format!("w{w}"));
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= end {
                    break;
                }
                let case = cfg.generator.case(cfg.seed, i);
                if tx.send(run_case(&cfg, case, &dir)).is_err() {
                    break;
                }
            }
        }));
    }
    drop(tx);

    let mut counts: BTreeMap<(Family, Class), u64> = BTreeMap::new();
    let mut by_flags: BTreeMap<(String, Class), u64> = BTreeMap::new();
    let mut sigs: BTreeMap<Signature, (u64, Vec<Outcome>)> = BTreeMap::new();
    let mut verified = 0u64;
    let mut mismatches = Vec::new();
    let mut jsonl = match &cfg.out {
        Some(out) => {
            Some(std::fs::File::create(out.join("cases.jsonl")).map_err(|e| e.to_string())?)
        }
        None => None,
    };
    let mut done = 0u64;
    let mut first_err = None;
    for r in rx {
        let o = match r {
            Ok(o) => o,
            Err(e) => {
                first_err.get_or_insert(e);
                continue;
            }
        };
        done += 1;
        if cfg.verify_every > 0 && o.case.index.is_multiple_of(cfg.verify_every) {
            verified += 1;
        }
        if !o.oracle_mismatch.is_empty() {
            mismatches.push(o.clone());
        }
        *counts.entry((o.case.family, o.class)).or_default() += 1;
        *by_flags.entry((o.case.flags.name(), o.class)).or_default() += 1;
        if o.class != Class::Same && o.class != Class::Split171 {
            if let Some(f) = jsonl.as_mut() {
                let _ = writeln!(f, "{}", json_line(&o));
            }
            let e = sigs.entry(signature(&o)).or_default();
            e.0 += 1;
            if e.1.len() < cfg.keep {
                e.1.push(o);
            }
        }
    }
    for w in workers {
        let _ = w.join();
    }
    if let Some(e) = first_err {
        return Err(e);
    }
    let secs = t0.elapsed().as_secs_f64();

    let mut report = String::new();
    let _ = writeln!(
        report,
        "jq-reader-oracle diff: generator={:?} seed={} cases {}..{} ({} run) in {:.1} s, {:.0} cases/s, {} jobs",
        cfg.generator,
        cfg.seed,
        cfg.start,
        end,
        done,
        secs,
        done as f64 / secs.max(1e-9),
        cfg.jobs
    );
    let _ = writeln!(report, "subject: {}", cfg.subject.display());
    table(
        &mut report,
        "family",
        counts
            .iter()
            .map(|((f, c), n)| (f.name().to_string(), *c, *n)),
    );
    table(
        &mut report,
        "flags",
        by_flags.iter().map(|((f, c), n)| (f.clone(), *c, *n)),
    );
    let _ = writeln!(
        report,
        "\nsignatures (class, family, flags, kind vs 1.7.1): count, first case indices"
    );
    for ((class, fam, flags, kind, reason), (n, kept)) in &sigs {
        let ids: Vec<String> = kept.iter().map(|o| o.case.index.to_string()).collect();
        let _ = writeln!(
            report,
            "  {:<11} {:<19} {:<12} {:<16} {:>6}  {}{}",
            class.name(),
            fam.name(),
            flags,
            kind.name(),
            n,
            ids.join(","),
            if reason.is_empty() {
                String::new()
            } else {
                format!("  ({reason})")
            }
        );
    }
    if cfg.verify_every > 0 && (cfg.jq171.is_some() || cfg.jq182.is_some()) {
        let _ = writeln!(
            report,
            "\noracle check against the real jq binaries: {verified} cases, {} disagree",
            mismatches.len()
        );
        for o in mismatches.iter().take(5) {
            for (r, got) in &o.oracle_mismatch {
                let want = if *r == Release::V171 {
                    &o.jq171
                } else {
                    &o.jq182
                };
                let _ = writeln!(
                    report,
                    "  case {} jq {}:\n    jq     {}\n    oracle {}",
                    o.case.index,
                    r.version(),
                    show(got),
                    show(want)
                );
            }
        }
    }

    if let Some(out) = &cfg.out {
        let mut n = 0;
        let mmdir = cfg.tmp.join("min");
        for ((class, fam, flags, kind, _), (_, kept)) in &sigs {
            for o in kept {
                let small = if cfg.minimize_evals > 0 && (class.is_bug() || n < 200) {
                    minimize(cfg, &o.case, *class, *kind, &mmdir, cfg.minimize_evals)
                } else {
                    o.case.clone()
                };
                let tag = format!(
                    "{}-{}-{}-{}",
                    class.name().replace(['(', ')', '='], "").replace('.', ""),
                    fam.name(),
                    flags.replace([' ', '(', ')'], "").replace("--", ""),
                    o.case.index
                );
                let dir = out.join("repros").join(tag);
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(dir.join("repro.sh"), repro_script(&small));
                let staged = stage(&mmdir, &small).ok();
                let mut md = format!(
                    "case {} ({:?} seed {}), {} / {} / {} / {}, minimised from {} to {} bytes\n",
                    o.case.index,
                    cfg.generator,
                    cfg.seed,
                    class.name(),
                    fam.name(),
                    flags,
                    kind.name(),
                    o.case.stream().len(),
                    small.stream().len()
                );
                if let Some(st) = staged {
                    for r in Release::ALL {
                        if let Ok((t, _)) = oracle::run(r, &small, &st) {
                            let _ = writeln!(md, "jq {}:     {}", r.version(), show(&t));
                        }
                    }
                    if let Ok(t) = spawn(&cfg.subject, &["jq"], &small, &st, &mmdir) {
                        let _ = writeln!(md, "succinctly: {}", show(&t));
                    }
                }
                let _ = std::fs::write(dir.join("README.txt"), md);
                n += 1;
            }
        }
        let _ = std::fs::write(out.join("summary.txt"), &report);
    }
    print!("{report}");
    let bugs: u64 = sigs
        .iter()
        .filter(|(k, _)| k.0.is_bug())
        .map(|(_, v)| v.0)
        .sum();
    Ok(if !mismatches.is_empty() {
        3
    } else if bugs > 0 {
        1
    } else {
        0
    })
}

fn table(out: &mut String, label: &str, rows: impl Iterator<Item = (String, Class, u64)>) {
    let mut m: BTreeMap<String, BTreeMap<Class, u64>> = BTreeMap::new();
    for (k, c, n) in rows {
        *m.entry(k).or_default().entry(c).or_default() += n;
    }
    let _ = write!(out, "\n{label:<20}");
    for c in Class::ALL {
        let _ = write!(out, " {:>11}", c.name());
    }
    let _ = writeln!(out, " {:>8}", "total");
    let mut tot: BTreeMap<Class, u64> = BTreeMap::new();
    for (k, row) in &m {
        let _ = write!(out, "{k:<20}");
        for c in Class::ALL {
            let n = row.get(&c).copied().unwrap_or(0);
            *tot.entry(c).or_default() += n;
            let _ = write!(out, " {n:>11}");
        }
        let _ = writeln!(out, " {:>8}", row.values().sum::<u64>());
    }
    let _ = write!(out, "{:<20}", "total");
    for c in Class::ALL {
        let _ = write!(out, " {:>11}", tot.get(&c).copied().unwrap_or(0));
    }
    let _ = writeln!(out, " {:>8}", tot.values().sum::<u64>());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn py_bytes_run_length_encodes() {
        assert_eq!(py_bytes(b"        1\0"), "b' '*8 + b'1\\x00'");
        assert_eq!(py_bytes(b""), "b''");
        assert_eq!(py_bytes(b"a'b"), "b'a\\'b'");
    }
}
