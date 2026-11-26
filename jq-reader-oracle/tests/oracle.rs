//! The in-process readers against captured jq output.
//!
//! `data/reader_model_cases.rs` holds 75 cases of the reader model (#7970's spike, option G,
//! checked the same set). Each expected string is the output of the real jq 1.7.1 or jq 1.8.2
//! binary on terminus, with files named `f0`, `f1`, ...:
//! - `inputs_*`: `jq -nc '<inputs_program(12)>' f0 f1 ...`;
//! - `main_*`: `jq -c . f0 f1 ...`'s standard output, then `!` and the message of jq's
//!   `jq: parse error: ...` line when the main loop stopped on one (exit 5);
//! - `slurp_*`: `inputs_program(4)` under `-nc -s`.
//!
//! Every case is checked against both releases' builds, with both engines: 75 x 3 programs x 2
//! releases x 2 engines. 72 of the 75 inputs read differently in the two releases, so a build
//! linked against the wrong release's symbols fails here.

use std::path::{Path, PathBuf};

use jq_reader_oracle::reader::{self, Opts, PortReader, Reader, Release, Source, UtilReader};

#[derive(Clone, Copy)]
enum Piece {
    Lit(&'static [u8]),
    Run(u8, usize),
}
use Piece::{Lit, Run};

struct Case {
    name: &'static str,
    files: &'static [&'static [Piece]],
    inputs_171: &'static str,
    main_171: &'static str,
    slurp_171: &'static str,
    inputs_182: &'static str,
    main_182: &'static str,
    slurp_182: &'static str,
    /// The reader model did not port 1.8.2 here; the C reader is jq's own, so it is checked anyway.
    #[allow(dead_code)]
    skip_182: bool,
}

include!("data/reader_model_cases.rs");

fn build(files: &[&[Piece]]) -> Vec<Vec<u8>> {
    files
        .iter()
        .map(|pieces| {
            let mut bytes = Vec::new();
            for piece in *pieces {
                match *piece {
                    Lit(text) => bytes.extend_from_slice(text),
                    Run(byte, count) => bytes.resize(bytes.len() + count, byte),
                }
            }
            bytes
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
enum Engine {
    Util,
    Port,
}

#[derive(Clone, Copy, Debug)]
enum Program {
    Inputs,
    Main,
    Slurp,
}

/// A directory of the case's files, named `f0`, `f1`, ...; removed on drop.
struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn stage(tag: &str, files: &[Vec<u8>]) -> (Dir, Vec<String>) {
    let dir = std::env::temp_dir().join(format!(
        "jq-reader-oracle-test-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mut names = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let p = dir.join(format!("f{i}"));
        std::fs::write(&p, f).expect("write");
        names.push(p.to_string_lossy().into_owned());
    }
    (Dir(dir), names)
}

fn reader(
    engine: Engine,
    release: Release,
    opts: Opts,
    names: &[String],
    files: &[Vec<u8>],
) -> Box<dyn Reader> {
    match engine {
        Engine::Util => Box::new(UtilReader::new(release, opts, names).expect("util reader")),
        Engine::Port => Box::new(
            PortReader::new(
                release,
                opts,
                names
                    .iter()
                    .zip(files)
                    .map(|(n, d)| Source {
                        name: n.clone(),
                        data: Some(d.clone()),
                    })
                    .collect(),
            )
            .expect("port reader"),
        ),
    }
}

fn render(case: &Case, release: Release, engine: Engine, run: Program) -> String {
    let files = build(case.files);
    let tag = format!("{}-{}-{engine:?}-{run:?}", case.name, release.version());
    let (dir, names) = stage(&tag, &files);
    let slurp = matches!(run, Program::Slurp);
    let mut r = reader(
        engine,
        release,
        Opts {
            slurp,
            ..Opts::default()
        },
        &names,
        &files,
    );
    let text = match run {
        Program::Inputs | Program::Slurp => {
            let out = reader::run_inputs(r.as_mut(), if slurp { 4 } else { 12 }).expect("inputs");
            let mut s = String::from_utf8_lossy(&out).into_owned();
            s.truncate(s.trim_end_matches('\n').len());
            s
        }
        Program::Main => {
            let t = reader::run_main(r.as_mut(), false).expect("main");
            let mut s = String::from_utf8_lossy(&t.stdout).into_owned();
            if t.status == 5 {
                let e = String::from_utf8_lossy(&t.stderr);
                s.push('!');
                s.push_str(
                    e.trim_start_matches("jq: parse error: ")
                        .trim_end_matches('\n'),
                );
            }
            s
        }
    };
    // The expectations name the files `f0`, `f1`, ...: drop the directory.
    let prefix = reader::json_string(format!("{}/", dir.0.display()).as_bytes()).expect("json");
    let prefix = String::from_utf8_lossy(&prefix[..prefix.len() - 1]).into_owned();
    text.replace(&prefix, "\"")
}

fn check(release: Release, engine: Engine, run: Program) {
    let mut failed = Vec::new();
    for case in CASES {
        let expected = match (release, run) {
            (Release::V171, Program::Inputs) => case.inputs_171,
            (Release::V171, Program::Main) => case.main_171,
            (Release::V171, Program::Slurp) => case.slurp_171,
            (Release::V182, Program::Inputs) => case.inputs_182,
            (Release::V182, Program::Main) => case.main_182,
            (Release::V182, Program::Slurp) => case.slurp_182,
        };
        let got = render(case, release, engine, run);
        if got != expected {
            failed.push(format!(
                "{}:\n  oracle: {got}\n  jq:     {expected}",
                case.name
            ));
        }
    }
    assert!(
        failed.is_empty(),
        "jq {} {engine:?} {run:?}: {} of {} cases differ:\n{}",
        release.version(),
        failed.len(),
        CASES.len(),
        failed.join("\n")
    );
}

#[test]
fn there_are_seventy_five_cases_and_most_split_the_releases() {
    assert_eq!(CASES.len(), 75);
    let split = CASES
        .iter()
        .filter(|c| {
            (c.inputs_171, c.main_171, c.slurp_171) != (c.inputs_182, c.main_182, c.slurp_182)
        })
        .count();
    assert!(split >= 70, "{split} of the cases tell the releases apart");
}

macro_rules! checks {
    ($($name:ident: $release:expr, $engine:expr, $run:expr;)*) => {
        $(
            #[test]
            fn $name() {
                check($release, $engine, $run);
            }
        )*
    };
}

checks! {
    util_171_inputs: Release::V171, Engine::Util, Program::Inputs;
    util_171_main: Release::V171, Engine::Util, Program::Main;
    util_171_slurp: Release::V171, Engine::Util, Program::Slurp;
    util_182_inputs: Release::V182, Engine::Util, Program::Inputs;
    util_182_main: Release::V182, Engine::Util, Program::Main;
    util_182_slurp: Release::V182, Engine::Util, Program::Slurp;
    port_171_inputs: Release::V171, Engine::Port, Program::Inputs;
    port_171_main: Release::V171, Engine::Port, Program::Main;
    port_171_slurp: Release::V171, Engine::Port, Program::Slurp;
    port_182_inputs: Release::V182, Engine::Port, Program::Inputs;
    port_182_main: Release::V182, Engine::Port, Program::Main;
    port_182_slurp: Release::V182, Engine::Port, Program::Slurp;
}

/// A run that stops before end of input (a parse error under `jq -c .`) must not leak the file
/// it was reading: jq's `jq_util_input_free` never closes it, which the vendored util.c fixes.
#[test]
fn a_run_that_stops_early_closes_its_input() {
    let (_dir, names) = stage("fd", &[b"1 x 2\n".to_vec(), b"3\n".to_vec()]);
    let open_fds = || std::fs::read_dir("/dev/fd").map(|d| d.count()).unwrap_or(0);
    let before = open_fds();
    for _ in 0..200 {
        for release in Release::ALL {
            let mut r = UtilReader::new(release, Opts::default(), &names).expect("reader");
            let t = reader::run_main(&mut r, false).expect("main");
            assert_eq!(t.status, 5);
        }
    }
    // Other tests run in parallel threads and open files of their own; a leak would be 400.
    assert!(open_fds() < before + 100, "{before} -> {}", open_fds());
}

/// The sync-point patch: on jq 1.8.2 a reader resumed at any sync point gives the rest of the run.
#[test]
fn sync_point_resumes_are_exact() {
    let mut data = vec![b' '; 4090];
    data.extend_from_slice(b"1\n[2,\n3]\n\"ab\"\n");
    data.extend(b"4 ".repeat(3000));
    let sources = vec![
        Source {
            name: "a".into(),
            data: Some(data),
        },
        Source {
            name: "b".into(),
            data: Some(b"5\n6".to_vec()),
        },
    ];
    let c = reader::sync_check(Release::V182, Opts::default(), &sources, 64, u64::MAX)
        .expect("sync check");
    assert!(c.sync_points >= 2 && c.checked >= 2, "{c:?}");
    assert_eq!(c.mismatches, 0, "{c:?}");
    assert!(reader::sync_check(Release::V171, Opts::default(), &sources, 4, u64::MAX).is_err());
}

/// Every global symbol of both C builds carries its release's prefix (vendor/rename-*.h), so the
/// two builds cannot bind to each other's functions. Needs `nm`; skipped without it.
#[test]
fn every_global_symbol_is_renamed() {
    let out = Path::new(env!("OUT_DIR"));
    let mut objects = Vec::new();
    let mut stack = vec![out.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("OUT_DIR").flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "o") {
                objects.push(p);
            }
        }
    }
    assert!(
        objects.len() >= 24,
        "{} objects in {}",
        objects.len(),
        out.display()
    );
    let Ok(nm) = std::process::Command::new("nm")
        .arg("-g")
        .args(&objects)
        .output()
    else {
        eprintln!("SKIPPED: no nm");
        return;
    };
    let text = String::from_utf8_lossy(&nm.stdout);
    let mut bad = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() != 3 || parts[1].eq_ignore_ascii_case("u") {
            continue;
        }
        let name = parts[2]
            .strip_prefix('_')
            .filter(|_| cfg!(target_os = "macos"))
            .unwrap_or(parts[2]);
        if !(name.starts_with("sjq171_") || name.starts_with("sjq182_")) {
            bad.push(name.to_string());
        }
    }
    assert!(
        bad.is_empty(),
        "unrenamed globals (re-run scripts/gen-rename.py): {bad:?}"
    );
}
