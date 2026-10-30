//! `env.json`: the machine and build a run was taken on (#5409 rule 4).

use crate::bench_runner::metadata::{
    collect_git_info, collect_system_info, collect_toolchain_info, GitInfo, SystemInfo,
    ToolchainInfo,
};
use anyhow::Result;
use serde::Serialize;
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
pub struct Env {
    pub git: GitInfo,
    pub system: SystemInfo,
    pub toolchain: ToolchainInfo,
    pub on_battery: Option<bool>,
    pub load_average_1m: Option<f64>,
    pub succinctly_sha256: String,
    pub succinctly_profile: &'static str,
    pub succinctly_features: Vec<String>,
    /// The timed binary is a separate `--subject` build, not the harness itself (#5570).
    pub subject_is_harness: bool,
    pub jq_sha256: String,
    pub jq_version: String,
    pub jq_build_configuration: String,
    /// Why this run's timings must not be quoted; empty when valid.
    pub invalid_reasons: Vec<String>,
}

fn cmd_stdout(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn on_battery() -> Option<bool> {
    if cfg!(target_os = "macos") {
        return cmd_stdout("pmset", &["-g", "batt"]).map(|s| s.contains("Battery Power"));
    }
    let mut seen = false;
    for e in std::fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let p = e.path();
        if std::fs::read_to_string(p.join("type")).ok()?.trim() == "Battery" {
            seen = true;
            if std::fs::read_to_string(p.join("status")).is_ok_and(|s| s.trim() == "Discharging") {
                return Some(true);
            }
        }
    }
    seen.then_some(false)
}

fn load_average() -> Option<f64> {
    let raw = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .or_else(|| cmd_stdout("sysctl", &["-n", "vm.loadavg"]))?;
    raw.trim_matches(|c| c == '{' || c == '}' || c == ' ')
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn features() -> Vec<String> {
    let mut f: Vec<&'static str> = vec![];
    if cfg!(feature = "cli") {
        f.push("cli");
    }
    if cfg!(feature = "simd") {
        f.push("simd");
    }
    if cfg!(feature = "regex") {
        f.push("regex");
    }
    if cfg!(feature = "serde") {
        f.push("serde");
    }
    for (on, name) in [
        (cfg!(feature = "scalar-yaml"), "scalar-yaml"),
        (cfg!(feature = "portable-popcount"), "portable-popcount"),
        (cfg!(feature = "unshared-containers"), "unshared-containers"),
        (cfg!(feature = "select-stats"), "select-stats"),
        (cfg!(feature = "share-stats"), "share-stats"),
    ] {
        if on {
            f.push(name);
        }
    }
    f.into_iter().map(String::from).collect()
}

impl Env {
    /// `subject` is the binary under test and its features; `None` times the harness itself.
    pub fn collect(
        jq: &std::path::Path,
        jq_sha256: String,
        subject: Option<(&std::path::Path, Vec<String>)>,
    ) -> Result<Self> {
        let git = collect_git_info()?;
        let on_battery = on_battery();
        let profile = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        let (subject_exe, feats) = match subject {
            Some((exe, feats)) => (exe.to_path_buf(), feats),
            None => (std::env::current_exe()?, features()),
        };
        let subject_is_harness = std::env::current_exe()? == subject_exe;
        let mut invalid = vec![];
        if git.dirty {
            invalid.push("dirty working tree".to_string());
        }
        if on_battery == Some(true) {
            invalid.push("running on battery power".to_string());
        }
        if profile != "release" {
            invalid.push("succinctly is not a release build".to_string());
        }
        for bad in [
            "scalar-yaml",
            "portable-popcount",
            "unshared-containers",
            "select-stats",
            "share-stats",
        ] {
            if feats.iter().any(|f| f == bad) {
                invalid.push(format!("measurement-distorting feature `{bad}` is enabled"));
            }
        }
        let jq_str = jq.to_string_lossy();
        Ok(Self {
            git,
            system: collect_system_info(),
            toolchain: collect_toolchain_info()?,
            on_battery,
            load_average_1m: load_average(),
            succinctly_sha256: super::manifest::sha256_hex(&std::fs::read(&subject_exe)?),
            succinctly_profile: profile,
            succinctly_features: feats,
            subject_is_harness,
            jq_sha256,
            jq_version: cmd_stdout(&jq_str, &["--version"]).unwrap_or_default(),
            jq_build_configuration: cmd_stdout(&jq_str, &["--build-configuration"])
                .unwrap_or_default(),
            invalid_reasons: invalid,
        })
    }
}
