//! `jq-reader-oracle`: jq's own input reader in-process, and the differential harness.
//!
//! ```text
//! jq-reader-oracle read [--release 1.7.1|1.8.2] [--engine util|port]
//!                       [--mode main|trace|inputs|count|sync-check] [--seq] [--stream]
//!                       [--stream-errors] [-s] [-R] [--lim N] [--limit N] [--print-program] [FILE...]
//! jq-reader-oracle diff --subject PATH [--cases N] [--start I] [--seed S] [--gen edge|spike]
//!                       [--jobs J] [--jq171 PATH] [--jq182 PATH] [--verify-every K]
//!                       [--out DIR] [--keep K] [--minimize-evals E] [--tmp DIR] [--c-only] [--lenient-e1]
//! jq-reader-oracle show [--gen edge|spike] [--seed S] --case I
//! jq-reader-oracle check --subject PATH [-n] [-s] [--seq] [--stdin|--pipe] FILE...
//! ```
//!
//! `read` prints what jq of the chosen release prints for `jq -c .` (`main`), for the `-n` trace
//! program (`trace`; `--print-program` prints it), for the reader model's `inputs` program, a count
//! of items, or a sync-point resume check (`--engine port`, jq 1.8.2). `diff` runs the harness
//! (see the crate's `diff` module); `show` prints one generated case and its repro script;
//! `check` runs and classifies given files (a repro found elsewhere) as `diff` would.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use jq_reader_oracle::diff::{self, gen::Generator, Config};
use jq_reader_oracle::reader::{
    self, Item, Opts, PortReader, Reader, Release, Source, Transcript, UtilReader,
};

fn usage() -> ExitCode {
    eprintln!(
        "usage: jq-reader-oracle read [options] [FILE...] | diff --subject PATH [options] | show --case I [options]\n(see src/main.rs)"
    );
    ExitCode::from(2)
}

struct Args(Vec<String>);

impl Args {
    fn flag(&mut self, names: &[&str]) -> bool {
        let before = self.0.len();
        self.0.retain(|a| !names.contains(&a.as_str()));
        self.0.len() != before
    }
    fn value(&mut self, name: &str) -> Result<Option<String>, String> {
        let Some(i) = self.0.iter().position(|a| a == name) else {
            return Ok(None);
        };
        if i + 1 >= self.0.len() {
            return Err(format!("{name} needs a value"));
        }
        let v = self.0.remove(i + 1);
        self.0.remove(i);
        Ok(Some(v))
    }
    fn num<T: std::str::FromStr>(&mut self, name: &str, default: T) -> Result<T, String> {
        match self.value(name)? {
            None => Ok(default),
            Some(v) => v.parse().map_err(|_| format!("{name}: not a number: {v}")),
        }
    }
}

fn main() -> ExitCode {
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        return usage();
    }
    let cmd = argv.remove(0);
    let mut a = Args(argv);
    let r = match cmd.as_str() {
        "read" => read(&mut a),
        "diff" => run_diff(&mut a),
        "show" => show(&mut a),
        "check" => check(&mut a),
        _ => return usage(),
    };
    match r {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("jq-reader-oracle {cmd}: {e}");
            ExitCode::from(2)
        }
    }
}

fn read(a: &mut Args) -> Result<u8, String> {
    let release = match a.value("--release")? {
        None => Release::V171,
        Some(v) => Release::parse(&v).ok_or(format!("--release: {v} is not 1.7.1 or 1.8.2"))?,
    };
    let engine = a.value("--engine")?.unwrap_or_else(|| "util".into());
    let mode = a.value("--mode")?.unwrap_or_else(|| "main".into());
    let lim = a.num("--lim", u64::MAX)?;
    let limit = a.num("--limit", 12usize)?;
    let mut flags = 0;
    let seq = a.flag(&["--seq"]);
    if seq {
        flags |= reader::PARSE_SEQ;
    }
    if a.flag(&["--stream"]) {
        flags |= reader::PARSE_STREAMING;
    }
    if a.flag(&["--stream-errors"]) {
        flags |= reader::PARSE_STREAMING | reader::PARSE_STREAM_ERRORS;
    }
    let opts = Opts {
        flags,
        slurp: a.flag(&["-s", "--slurp"]),
        raw: a.flag(&["-R", "--raw-input"]),
    };
    if a.flag(&["--print-program"]) {
        match mode.as_str() {
            "inputs" => println!("{}", reader::inputs_program(limit)),
            _ => println!("{}", reader::trace_program(lim)),
        }
        return Ok(0);
    }
    if let Some(bad) = a.0.iter().find(|f| f.starts_with("--")) {
        return Err(format!("unknown option {bad}"));
    }
    let files = std::mem::take(&mut a.0);
    let sources = || -> Vec<Source> {
        files
            .iter()
            .map(|f| Source {
                name: f.clone(),
                data: std::fs::read(f).ok(),
            })
            .collect()
    };
    if mode == "sync-check" {
        let c =
            reader::sync_check(release, opts, &sources(), limit, lim).map_err(|e| e.to_string())?;
        println!(
            "items={} sync_points={} checked={} mismatches={}",
            c.items, c.sync_points, c.checked, c.mismatches
        );
        return Ok(u8::from(c.mismatches != 0));
    }
    let mut r: Box<dyn Reader> = match engine.as_str() {
        "util" => Box::new(UtilReader::new(release, opts, &files).map_err(|e| e.to_string())?),
        "port" => Box::new(PortReader::new(release, opts, sources()).map_err(|e| e.to_string())?),
        _ => return Err(format!("--engine: {engine} is not util or port")),
    };
    let t = match mode.as_str() {
        "main" => reader::run_main(r.as_mut(), seq),
        "trace" => reader::run_trace(r.as_mut(), lim, seq),
        "inputs" => reader::run_inputs(r.as_mut(), limit).map(|mut o| {
            if seq {
                o.insert(0, 0x1e);
            }
            Transcript {
                stdout: o,
                stderr: r.take_errs(),
                status: if r.errors() != 0 { 2 } else { 0 },
            }
        }),
        "count" => {
            let (mut v, mut e, mut bytes) = (0u64, 0u64, 0u64);
            loop {
                match r.next() {
                    Ok(Some(Item::Value(t))) => {
                        v += 1;
                        bytes += t.len() as u64;
                    }
                    Ok(Some(Item::Error(_))) => e += 1,
                    Ok(None) => break,
                    Err(err) => return Err(err.to_string()),
                }
            }
            Ok(Transcript {
                stdout: format!("values={v} errors={e} dumped_bytes={bytes}\n").into_bytes(),
                ..Transcript::default()
            })
        }
        _ => {
            return Err(format!(
                "--mode: {mode} is not main, trace, inputs, count or sync-check"
            ))
        }
    }
    .map_err(|e| e.to_string())?;
    let _ = std::io::stdout().write_all(&t.stdout);
    let _ = std::io::stderr().write_all(&t.stderr);
    Ok(u8::try_from(t.status).unwrap_or(1))
}

fn generator(a: &mut Args) -> Result<Generator, String> {
    match a.value("--gen")? {
        None => Ok(Generator::Edge),
        Some(g) => Generator::parse(&g).ok_or(format!("--gen: {g} is not edge or spike")),
    }
}

fn run_diff(a: &mut Args) -> Result<u8, String> {
    let c_only = a.flag(&["--c-only"]);
    let subject = match a.value("--subject")? {
        Some(p) => PathBuf::from(p),
        None if c_only => PathBuf::new(),
        None => return Err("--subject PATH is required (or --c-only)".into()),
    };
    let generator = generator(a)?;
    let jobs = a.num(
        "--jobs",
        std::thread::available_parallelism().map_or(4, |n| n.get().min(8)),
    )?;
    let cfg = Config {
        subject,
        generator,
        seed: a.num("--seed", 1)?,
        start: a.num("--start", 0)?,
        cases: a.num("--cases", 3000)?,
        jobs,
        jq171: a.value("--jq171")?.map(PathBuf::from),
        jq182: a.value("--jq182")?.map(PathBuf::from),
        verify_every: a.num("--verify-every", 0)?,
        out: a.value("--out")?.map(PathBuf::from),
        keep: a.num("--keep", 2)?,
        minimize_evals: a.num("--minimize-evals", 300)?,
        c_only,
        lenient_e1: a.flag(&["--lenient-e1"]),
        tmp: a.value("--tmp")?.map(PathBuf::from).unwrap_or_else(|| {
            std::env::temp_dir().join(format!("jq-reader-oracle-{}", std::process::id()))
        }),
    };
    if let Some(bad) = a.0.first() {
        return Err(format!("unexpected argument {bad}"));
    }
    let rc = diff::run(&cfg);
    let _ = std::fs::remove_dir_all(&cfg.tmp);
    Ok(u8::try_from(rc).unwrap_or(2))
}

fn show(a: &mut Args) -> Result<u8, String> {
    let generator = generator(a)?;
    let seed = a.num("--seed", 1)?;
    let index = a.num::<u64>("--case", u64::MAX)?;
    if index == u64::MAX {
        return Err("--case I is required".into());
    }
    let c = generator.case(seed, index);
    println!(
        "# case {index}: family={} edge={} flags={} entry={} shape={}",
        c.family.name(),
        c.edge.name(),
        c.flags.name(),
        c.entry.name(),
        c.shape
    );
    print!("{}", diff::repro_script(&c));
    Ok(0)
}

fn check(a: &mut Args) -> Result<u8, String> {
    use jq_reader_oracle::diff::gen::{Case, Edge, Entry, Family, File, Flags};
    let subject = PathBuf::from(a.value("--subject")?.ok_or("--subject PATH is required")?);
    let flags = Flags::new(a.flag(&["-n"]), a.flag(&["-s"]), a.flag(&["--seq"]));
    let entry = if a.flag(&["--pipe"]) {
        Entry::StdinPipe
    } else if a.flag(&["--stdin"]) {
        Entry::StdinFile
    } else {
        Entry::Files
    };
    let mut files = Vec::new();
    for (i, f) in std::mem::take(&mut a.0).iter().enumerate() {
        let data = std::fs::read(f).map_err(|e| format!("{f}: {e}"))?;
        files.push(File {
            name: format!("f{i}"),
            data: Some(data),
        });
    }
    let case = Case {
        index: 0,
        family: Family::Spike,
        edge: Edge::Start,
        flags,
        entry,
        shape: "given files".into(),
        files,
    };
    let tmp = std::env::temp_dir().join(format!("jq-reader-oracle-check-{}", std::process::id()));
    let cfg = Config {
        subject,
        generator: Generator::Edge,
        seed: 0,
        start: 0,
        cases: 1,
        jobs: 1,
        jq171: None,
        jq182: None,
        verify_every: 0,
        out: None,
        keep: 0,
        minimize_evals: 0,
        c_only: false,
        lenient_e1: a.flag(&["--lenient-e1"]),
        tmp: tmp.clone(),
    };
    let o = diff::run_case(&cfg, case, &tmp.join("w"));
    let _ = std::fs::remove_dir_all(&tmp);
    let o = o?;
    println!(
        "class: {}  kind vs 1.7.1: {}{}",
        o.class.name(),
        o.kind.name(),
        o.reason.map(|r| format!("  ({r})")).unwrap_or_default()
    );
    println!("jq 1.7.1:   {}", diff::show(&o.jq171));
    println!("jq 1.8.2:   {}", diff::show(&o.jq182));
    println!("succinctly: {}", diff::show(&o.subject));
    Ok(u8::from(o.class.is_bug()))
}
