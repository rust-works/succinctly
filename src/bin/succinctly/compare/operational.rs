//! Dimension H: operational properties (#5409).
//!
//! Only what can be measured or read from a checked-in source is reported:
//! binary sizes, dynamic libraries, the Rust dependency count, whether either
//! tool uses more than one core, and a few facts taken from `Cargo.toml`,
//! `src/lib.rs`, `ci.yml` and jq's vendored licence. Build time is not measured
//! (a cold fat-LTO build is minutes and would dominate a `--quick` run).

use super::harness::Tool;
use super::perf::{ensure_input, InputSpec};
use anyhow::Result;
use serde::Serialize;
use std::path::Path;
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

#[derive(Debug, Clone, Serialize)]
pub struct Operational {
    pub binaries: Vec<BinaryInfo>,
    /// Distinct crates in `cargo tree -e normal` for the shipped feature set.
    pub rust_dependencies: Option<usize>,
    pub parallelism: Vec<Parallelism>,
    pub facts: Vec<Fact>,
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
