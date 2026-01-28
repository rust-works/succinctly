//! Metadata collection for benchmark runs.
//!
//! Collects git, system, and toolchain information.

use super::utils::{get_cpu_info, get_os_version, get_total_ram_gb};
use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use std::process::Command;

/// Git repository information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitInfo {
    pub commit: String,
    pub commit_short: String,
    pub branch: String,
    pub dirty: bool,
    pub untracked_files: usize,
    pub modified_files: usize,
    pub commit_date: String,
    pub commit_message: String,
}

/// System information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    pub cpu: String,
    pub cpu_cores: u32,
    pub ram_gb: f64,
    pub os: String,
    pub os_version: String,
    pub arch: String,
}

/// Rust toolchain information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolchainInfo {
    pub rustc_version: String,
    pub cargo_version: String,
    pub target: String,
}

/// Complete benchmark run metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkMetadata {
    pub timestamp: String,
    pub timestamp_local: String,
    pub git: GitInfo,
    pub system: SystemInfo,
    pub toolchain: ToolchainInfo,
}

impl BenchmarkMetadata {
    /// Collect all metadata.
    pub fn collect() -> Result<Self> {
        let now_utc: DateTime<Utc> = Utc::now();
        let now_local: DateTime<Local> = Local::now();

        Ok(Self {
            timestamp: now_utc.to_rfc3339(),
            timestamp_local: now_local.format("%Y-%m-%d %H:%M:%S %Z").to_string(),
            git: collect_git_info()?,
            system: collect_system_info(),
            toolchain: collect_toolchain_info()?,
        })
    }

    /// Generate output directory name from metadata.
    pub fn output_dir_name(&self) -> String {
        let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
        if self.git.dirty {
            format!("{}_{}_dirty", timestamp, self.git.commit_short)
        } else {
            format!("{}_{}", timestamp, self.git.commit_short)
        }
    }
}

/// Run a git command and return trimmed output.
fn run_git(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("Failed to run git {:?}", args))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {:?} failed: {}", args, stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Collect git repository information.
pub fn collect_git_info() -> Result<GitInfo> {
    let commit = run_git(&["rev-parse", "HEAD"])?;
    let commit_short = run_git(&["rev-parse", "--short", "HEAD"])?;
    let branch = run_git(&["rev-parse", "--abbrev-ref", "HEAD"])?;

    // Check for dirty working tree
    let status = run_git(&["status", "--porcelain"])?;
    let dirty = !status.is_empty();

    // Count untracked and modified files
    let mut untracked_files = 0;
    let mut modified_files = 0;
    for line in status.lines() {
        if line.starts_with("??") {
            untracked_files += 1;
        } else if !line.is_empty() {
            modified_files += 1;
        }
    }

    let commit_date = run_git(&["log", "-1", "--format=%cI"])?;
    let commit_message = run_git(&["log", "-1", "--format=%s"])?;

    Ok(GitInfo {
        commit,
        commit_short,
        branch,
        dirty,
        untracked_files,
        modified_files,
        commit_date,
        commit_message,
    })
}

/// Collect system information.
pub fn collect_system_info() -> SystemInfo {
    let cpu = get_cpu_info();
    let cpu_cores = std::thread::available_parallelism()
        .map(|p| p.get() as u32)
        .unwrap_or(1);
    let ram_gb = get_total_ram_gb();
    let os = std::env::consts::OS.to_string();
    let os_version = get_os_version();
    let arch = std::env::consts::ARCH.to_string();

    SystemInfo {
        cpu,
        cpu_cores,
        ram_gb,
        os,
        os_version,
        arch,
    }
}

/// Collect Rust toolchain information.
pub fn collect_toolchain_info() -> Result<ToolchainInfo> {
    let rustc_output = Command::new("rustc")
        .arg("--version")
        .output()
        .context("Failed to run rustc --version")?;
    let rustc_version = String::from_utf8_lossy(&rustc_output.stdout)
        .trim()
        .to_string();

    let cargo_output = Command::new("cargo")
        .arg("--version")
        .output()
        .context("Failed to run cargo --version")?;
    let cargo_version = String::from_utf8_lossy(&cargo_output.stdout)
        .trim()
        .to_string();

    // Get target triple
    let target = if let Ok(output) = Command::new("rustc").args(["--print", "cfg"]).output() {
        let cfg = String::from_utf8_lossy(&output.stdout);
        // Parse target from cfg output
        let mut target_arch = "";
        let mut target_os = "";
        let mut target_env = "";
        for line in cfg.lines() {
            if line.starts_with("target_arch=") {
                target_arch = line
                    .strip_prefix("target_arch=")
                    .unwrap_or("")
                    .trim_matches('"');
            } else if line.starts_with("target_os=") {
                target_os = line
                    .strip_prefix("target_os=")
                    .unwrap_or("")
                    .trim_matches('"');
            } else if line.starts_with("target_env=") {
                target_env = line
                    .strip_prefix("target_env=")
                    .unwrap_or("")
                    .trim_matches('"');
            }
        }
        if !target_env.is_empty() {
            format!("{}-unknown-{}-{}", target_arch, target_os, target_env)
        } else {
            format!("{}-{}", target_arch, target_os)
        }
    } else {
        format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
    };

    Ok(ToolchainInfo {
        rustc_version,
        cargo_version,
        target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_system_info() {
        let info = collect_system_info();
        assert!(!info.cpu.is_empty());
        assert!(info.cpu_cores > 0);
        assert!(!info.os.is_empty());
    }

    #[test]
    fn test_collect_toolchain_info() {
        let info = collect_toolchain_info().unwrap();
        assert!(info.rustc_version.contains("rustc"));
        assert!(info.cargo_version.contains("cargo"));
    }

    #[test]
    fn test_output_dir_name() {
        let meta = BenchmarkMetadata {
            timestamp: "2026-01-29T12:00:00Z".to_string(),
            timestamp_local: "2026-01-29 12:00:00 PST".to_string(),
            git: GitInfo {
                commit: "abc1234567890".to_string(),
                commit_short: "abc1234".to_string(),
                branch: "main".to_string(),
                dirty: false,
                untracked_files: 0,
                modified_files: 0,
                commit_date: "2026-01-29T11:00:00Z".to_string(),
                commit_message: "test commit".to_string(),
            },
            system: collect_system_info(),
            toolchain: collect_toolchain_info().unwrap(),
        };

        let name = meta.output_dir_name();
        assert!(name.contains("abc1234"));
        assert!(!name.contains("dirty"));
    }
}
