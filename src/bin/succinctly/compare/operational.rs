//! Dimension H: operational properties (#5409).
//!
//! Only what can be measured or read from a checked-in source is reported:
//! binary sizes, dynamic libraries, the Rust dependency count, whether either
//! tool uses more than one core, and a few facts taken from `Cargo.toml`,
//! `src/lib.rs`, `ci.yml` and jq's vendored licence. Build time is measured only with
//! `--build-time` (a cold fat-LTO build is minutes and would dominate a `--quick` run, #5552).

use super::harness::Tool;
use super::perf::{ensure_input, InputSpec};
use anyhow::Result;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Serialize)]
pub struct BinaryInfo {
    pub tool: String,
    pub bytes: u64,
    pub dynamic_libs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Parallelism {
    pub tool: String,
    /// (user + sys) ÷ real on `.` over a 10 MB document; near 1 means single-threaded.
    pub cpu_over_wall: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Fact {
    pub item: String,
    pub value: String,
    /// Where the value was read from.
    pub source: String,
}

/// One from-source build, timed with a monotonic clock (`--build-time`, #5552).
#[derive(Debug, Clone, Serialize)]
pub struct BuildTime {
    pub what: String,
    /// What was run, for the report.
    pub command: String,
    /// `None` when the build failed or could not be timed.
    pub seconds: Option<f64>,
    /// Why it failed (the last lines of its stderr), or a caveat about the figure.
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Operational {
    pub binaries: Vec<BinaryInfo>,
    /// Distinct crates in `cargo tree -e normal` for the shipped feature set.
    pub rust_dependencies: Option<usize>,
    pub parallelism: Vec<Parallelism>,
    pub facts: Vec<Fact>,
    /// Empty unless the run asked for `--build-time`.
    #[serde(default)]
    pub build_times: Vec<BuildTime>,
}

/// Seconds from the file `scripts/compare-jq-build.sh` writes (milliseconds, one integer).
pub fn parse_timing_ms(text: &str) -> Option<f64> {
    text.trim().parse::<u64>().ok().map(|ms| ms as f64 / 1000.0)
}

/// The last few lines of a failed build's stderr, for the report.
fn stderr_tail(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(4)..].join(" / ");
    if tail.is_empty() {
        "no output on stderr".into()
    } else {
        tail
    }
}

/// Run `cmd` with its stdout discarded and return how long it took, or why it failed.
fn time_command(cmd: &mut Command) -> Result<f64, String> {
    let start = std::time::Instant::now();
    let out = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("could not start: {e}"))?;
    if out.status.success() {
        Ok(start.elapsed().as_secs_f64())
    } else {
        Err(format!("{}: {}", out.status, stderr_tail(&out.stderr)))
    }
}

/// Removes the scratch directory when dropped, so an early return or a panic does not leave a
/// target directory of several GB behind (a Ctrl-C still does; the next run clears it first).
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Time a cold build of each pinned jq (`versions`) and of succinctly, and the rebuild of
/// succinctly after a source file is touched, one run each in a scratch directory under
/// `target/compare/build-time/` that is removed afterwards. The jq tarball download and the
/// crates' download (`cargo fetch`) are outside the clock. The `touch` is undone: the file's
/// modification time is put back, so the checkout's own `target/` does not see a change.
pub fn measure_build_times(root: &Path, versions: &[String], features: &str) -> Vec<BuildTime> {
    let scratch = Scratch(root.join("target/compare/build-time"));
    let _ = std::fs::remove_dir_all(&scratch.0);
    let _ = std::fs::create_dir_all(&scratch.0);
    let mut out = vec![];
    for v in versions {
        let timing = scratch.0.join(format!("jq-{v}.ms"));
        let run = time_command(
            Command::new(root.join("scripts/compare-jq-build.sh"))
                .arg(v)
                .env("COMPARE_JQ_DEST", scratch.0.join(format!("jq-{v}")))
                .env("COMPARE_JQ_TIMING_FILE", &timing),
        );
        let (seconds, note) = match run {
            Ok(_) => match std::fs::read_to_string(&timing)
                .ok()
                .and_then(|t| parse_timing_ms(&t))
            {
                Some(s) => (Some(s), None),
                None => (None, Some("the build script wrote no timing".to_string())),
            },
            Err(why) => (None, Some(why)),
        };
        out.push(BuildTime {
            what: format!("jq {v}, cold"),
            command:
                "./configure --with-oniguruma=builtin --disable-shared && make -j && make install"
                    .into(),
            seconds,
            note,
        });
    }
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let target = scratch.0.join("succinctly-target");
    // Untimed: the crates must be on disk before the clock starts. If this fails, the download
    // lands inside the cold build, and the row says so.
    let fetched = Command::new(&cargo)
        .args(["fetch", "--locked"])
        .current_dir(root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    let build_args = ["build", "--release", "--locked", "--features", features];
    let command = format!("cargo build --release --locked --features {features}");
    let build = || {
        time_command(
            Command::new(&cargo)
                .args(build_args)
                .arg("--target-dir")
                .arg(&target)
                .current_dir(root),
        )
    };
    let cold = build();
    out.push(BuildTime {
        what: "succinctly, cold".into(),
        command: command.clone(),
        seconds: cold.as_ref().ok().copied(),
        note: match &cold {
            Err(why) => Some(why.clone()),
            Ok(_) if !fetched => Some(
                "`cargo fetch` failed, so crate downloads may be inside this figure".to_string(),
            ),
            Ok(_) => None,
        },
    });
    // The rebuild needs a changed mtime on a tracked file; the original is put back afterwards.
    let lib = root.join("src/lib.rs");
    let reference = scratch.0.join("lib.rs.mtime");
    let kept = Command::new("cp")
        .arg("-p")
        .arg(&lib)
        .arg(&reference)
        .status()
        .is_ok_and(|s| s.success());
    let touched = kept
        && Command::new("touch")
            .arg(&lib)
            .status()
            .is_ok_and(|s| s.success());
    let rebuild = if touched {
        build()
    } else {
        Err("could not touch src/lib.rs (read-only checkout?)".to_string())
    };
    if touched {
        let _ = Command::new("touch")
            .arg("-r")
            .arg(&reference)
            .arg(&lib)
            .status();
    }
    out.push(BuildTime {
        what: "succinctly, rebuild after touching src/lib.rs".into(),
        command,
        seconds: rebuild.as_ref().ok().copied(),
        note: rebuild.err(),
    });
    out
}

/// Library paths from `otool -L` (skipping the binary's own line).
pub fn parse_otool(out: &str) -> Vec<String> {
    out.lines()
        .skip(1)
        .filter_map(|l| l.trim().split(" (").next())
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// Library names from `ldd` (`name => path (addr)` or `name (addr)`).
pub fn parse_ldd(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| l.trim().split([' ', '\t']).next())
        .filter(|l| !l.is_empty() && !l.starts_with("linux-vdso"))
        .map(String::from)
        .collect()
}

fn dynamic_libs(path: &str) -> Vec<String> {
    let (cmd, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("otool", vec!["-L", path])
    } else {
        ("ldd", vec![path])
    };
    let Ok(out) = Command::new(cmd).args(args).output() else {
        return vec![];
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut libs = if cfg!(target_os = "macos") {
        parse_otool(&text)
    } else {
        parse_ldd(&text)
    };
    libs.sort();
    libs
}

/// Number of distinct crates in `cargo tree` output (the root counts as none).
pub fn count_tree(out: &str) -> usize {
    let mut names: Vec<&str> = out
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    names.sort_unstable();
    names.dedup();
    names.len().saturating_sub(1)
}

fn rust_dependencies(root: &Path, features: &str) -> Option<usize> {
    let out = Command::new("cargo")
        .current_dir(root)
        .args([
            "tree",
            "-e",
            "normal",
            "--prefix",
            "none",
            "--features",
            features,
        ])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| count_tree(&String::from_utf8_lossy(&out.stdout)))
}

/// (user + sys) ÷ real from `time -p` output.
pub fn parse_time_p(report: &str) -> Option<f64> {
    let get = |k: &str| {
        report
            .lines()
            .find_map(|l| l.strip_prefix(k))
            .and_then(|v| v.trim().parse::<f64>().ok())
    };
    let (real, user, sys) = (get("real")?, get("user")?, get("sys")?);
    (real > 0.0).then(|| (user + sys) / real)
}

fn cpu_over_wall(tool: &Tool, file: &Path) -> Option<f64> {
    let out = Command::new("/usr/bin/time")
        .arg("-p")
        .arg(&tool.program)
        .args(&tool.prefix)
        .args([".", &file.to_string_lossy()])
        .stdout(Stdio::null())
        .output()
        .ok()?;
    parse_time_p(&String::from_utf8_lossy(&out.stderr))
}

fn read(root: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(root.join(rel)).ok()
}

fn facts(root: &Path) -> Vec<Fact> {
    let mut out = vec![];
    let mut push = |item: &str, value: String, source: &str| {
        out.push(Fact {
            item: item.into(),
            value,
            source: source.into(),
        });
    };
    if let Some(l) = read(root, "Cargo.toml").and_then(|t| {
        t.lines()
            .find(|l| l.starts_with("license ="))
            .map(|l| l.replace('"', ""))
    }) {
        push(
            "succinctly licence",
            l.trim_start_matches("license =").trim().into(),
            "Cargo.toml",
        );
    }
    if let Some(c) = read(root, "comparison/jq/vendor/jq-1.7.1/COPYING") {
        let mit = c.contains("Permission is hereby granted");
        push(
            "jq licence",
            if mit {
                "MIT".into()
            } else {
                "see COPYING".into()
            },
            "comparison/jq/vendor/jq-1.7.1/COPYING",
        );
    }
    if let Some(lib) = read(root, "src/lib.rs") {
        push(
            "library without std",
            if lib.contains("no_std") {
                "yes (`no_std` + `alloc`)".into()
            } else {
                "no".into()
            },
            "src/lib.rs",
        );
    }
    if let Some(ci) = read(root, ".github/workflows/ci.yml") {
        let mut runners: Vec<String> = ci
            .lines()
            .filter_map(|l| l.trim().strip_prefix("runs-on:"))
            .map(|r| r.trim().to_string())
            .filter(|r| !r.contains("${{"))
            .collect();
        runners.sort();
        runners.dedup();
        push(
            "succinctly CI runners",
            runners.join(", "),
            ".github/workflows/ci.yml",
        );
    }
    out
}

pub fn collect(
    root: &Path,
    tools: &[Tool],
    features: &str,
    generator: &Path,
    inputs: &[InputSpec],
    inputs_dir: &Path,
    build_times: Vec<BuildTime>,
) -> Result<Operational> {
    let binaries = tools
        .iter()
        .filter_map(|t| {
            let bytes = std::fs::metadata(&t.program).ok()?.len();
            Some(BinaryInfo {
                tool: t.name.clone(),
                bytes,
                dynamic_libs: dynamic_libs(&t.program),
            })
        })
        .collect();
    let file = inputs
        .iter()
        .find(|i| i.id == "users-10mb")
        .and_then(|i| ensure_input(root, generator, inputs_dir, i).ok());
    let parallelism = tools
        .iter()
        .map(|t| Parallelism {
            tool: t.name.clone(),
            cpu_over_wall: file.as_deref().and_then(|f| cpu_over_wall(t, f)),
        })
        .collect();
    Ok(Operational {
        binaries,
        rust_dependencies: rust_dependencies(root, features),
        parallelism,
        facts: facts(root),
        build_times,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn otool_and_ldd_output_are_parsed() {
        let o = "/bin/jq:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)\n\t/usr/lib/libm.dylib (x)\n";
        assert_eq!(
            parse_otool(o),
            ["/usr/lib/libSystem.B.dylib", "/usr/lib/libm.dylib"]
        );
        let l = "\tlinux-vdso.so.1 (0x1)\n\tlibm.so.6 => /lib/libm.so.6 (0x2)\n\t/lib64/ld-linux-x86-64.so.2 (0x3)\n";
        assert_eq!(parse_ldd(l), ["libm.so.6", "/lib64/ld-linux-x86-64.so.2"]);
    }

    #[test]
    fn tree_count_is_distinct_crates_without_the_root() {
        assert_eq!(
            count_tree("succinctly v0.8.0 (/x)\nserde v1\nserde v1\nregex v1\n"),
            2
        );
        assert_eq!(count_tree(""), 0);
    }

    #[test]
    fn cpu_over_wall_is_read_from_time_p() {
        assert_eq!(parse_time_p("real 2.00\nuser 1.00\nsys 1.00\n"), Some(1.0));
        assert_eq!(parse_time_p("real 1.00\nuser 3.00\nsys 0.00\n"), Some(3.0));
        assert_eq!(parse_time_p("nothing"), None);
        assert_eq!(parse_time_p("real 0.00\nuser 0\nsys 0\n"), None);
    }

    #[test]
    fn a_jq_build_timing_file_is_milliseconds() {
        assert_eq!(parse_timing_ms("61234\n"), Some(61.234));
        assert_eq!(parse_timing_ms(""), None);
        assert_eq!(parse_timing_ms("12.5s"), None);
    }

    /// A build that fails has no time, not the time it took to fail.
    #[test]
    fn a_command_is_timed_only_when_it_succeeds() {
        let took = time_command(Command::new("sh").args(["-c", "sleep 0.05"])).unwrap();
        assert!((0.05..5.0).contains(&took), "{took}");
        let why =
            time_command(Command::new("sh").args(["-c", "echo one >&2; echo two >&2; exit 3"]))
                .unwrap_err();
        assert!(why.contains("one / two"), "the stderr tail says why: {why}");
        assert!(time_command(&mut Command::new("/nonexistent/build"))
            .unwrap_err()
            .starts_with("could not start"));
    }

    #[test]
    fn a_stderr_tail_is_the_last_four_lines_and_never_empty() {
        assert_eq!(stderr_tail(b"a\nb\nc\nd\ne\n\n"), "b / c / d / e");
        assert_eq!(stderr_tail(b""), "no output on stderr");
    }

    #[test]
    fn facts_come_from_the_checked_in_files() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let f = facts(root);
        let get = |i: &str| f.iter().find(|x| x.item == i).map(|x| x.value.clone());
        assert_eq!(get("succinctly licence").as_deref(), Some("MIT"));
        assert_eq!(get("jq licence").as_deref(), Some("MIT"));
        assert!(get("library without std").unwrap().starts_with("yes"));
        assert!(get("succinctly CI runners").unwrap().contains("ubuntu"));
    }
}
