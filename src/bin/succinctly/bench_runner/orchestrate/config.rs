//! `nodes.yaml` configuration parsing for distributed benchmark orchestration.
//!
//! See `nodes.yaml.example` at the repo root for an annotated template.

use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// Top-level orchestration configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub coordinator: CoordinatorConfig,
    pub nodes: Vec<NodeConfig>,
    /// Per-benchmark isolation overrides; a name not listed here keeps the
    /// registry's `default_isolation`. See [`super::scheduler::resolve_isolation`].
    #[serde(default)]
    pub benchmarks: Vec<BenchmarkOverride>,
}

/// Overrides a single benchmark's orchestration isolation.
#[derive(Debug, Clone, Deserialize)]
pub struct BenchmarkOverride {
    pub name: String,
    /// `None` means "no override, inherit the registry default" — distinct
    /// from an explicit isolation value, so a future override field being
    /// added to this struct can't silently reset isolation via
    /// `#[serde(default)]` on a bare (non-`Option`) enum.
    #[serde(default)]
    pub isolation: Option<crate::bench_runner::registry::Isolation>,
}

/// Coordinator-wide settings.
#[derive(Debug, Clone, Deserialize)]
pub struct CoordinatorConfig {
    pub results_dir: PathBuf,
    /// Per-node vs. global work queue. Reserved for a future global
    /// work-stealing scheduler; `scheduler.rs` currently always schedules
    /// per-node regardless of this setting (`Global` is not yet implemented).
    #[serde(default)]
    #[allow(dead_code)] // STYLE-0005: reserved for a future global-queue scheduler
    pub parallelism: Parallelism,
    #[serde(default = "default_ssh_timeout")]
    pub ssh_timeout: u64,
    #[serde(default = "default_benchmark_timeout")]
    pub benchmark_timeout: u64,
}

const fn default_ssh_timeout() -> u64 {
    30
}

const fn default_benchmark_timeout() -> u64 {
    3600
}

/// How the coordinator schedules work across nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Parallelism {
    /// Each node schedules its own work queue independently.
    #[default]
    PerNode,
    /// The coordinator manages one global work queue across all nodes.
    Global,
}

/// A single benchmark node (local or SSH-reachable).
#[derive(Debug, Clone, Deserialize)]
pub struct NodeConfig {
    pub name: String,
    /// SSH destination, e.g. `user@host` or a Tailscale MagicDNS name.
    /// `localhost`/`127.0.0.1` runs commands directly with no `ssh` wrapper.
    pub host: String,
    pub arch: Architecture,
    /// SIMD/CPU features; informational, recorded in each run's
    /// `node_info.json` for cross-platform result comparisons.
    #[serde(default)]
    pub features: Vec<String>,
    /// How many benchmarks may run concurrently on this node (enforced
    /// starting Phase 2's scheduler; Phase 1 always runs sequentially).
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
    #[serde(default)]
    pub ssh_key: Option<PathBuf>,
    #[serde(default)]
    pub working_dir: Option<PathBuf>,
    /// Explicit target triple for cross-compilation, e.g. `aarch64-apple-darwin`
    /// vs `aarch64-unknown-linux-gnu` — `arch` alone can't disambiguate these.
    #[serde(default)]
    pub target_triple: Option<String>,
    /// EC2 instance ID, if this node is EC2-backed — enables `bench nodes
    /// --start/--stop` and skips SSH probing a known-stopped instance.
    #[serde(default)]
    pub ec2_instance_id: Option<String>,
    #[serde(default)]
    pub ec2_region: Option<String>,
}

const fn default_max_concurrent() -> usize {
    1
}

impl NodeConfig {
    /// True if commands should run directly rather than over `ssh`.
    pub fn is_local(&self) -> bool {
        self.host == "localhost" || self.host == "127.0.0.1"
    }

    /// Remote (or local) working directory to `cd` into before running commands.
    pub fn working_dir_str(&self) -> &str {
        self.working_dir
            .as_deref()
            .and_then(Path::to_str)
            .unwrap_or(".")
    }

    /// `ssh_key` with a leading `~` expanded against `$HOME`.
    pub fn expanded_ssh_key(&self) -> Option<PathBuf> {
        self.ssh_key.as_deref().map(expand_tilde)
    }
}

/// CPU architecture of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Architecture {
    #[value(name = "x86_64")]
    X86_64,
    #[value(name = "aarch64")]
    Aarch64,
}

impl fmt::Display for Architecture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `f.pad(..)`, not `write!(f, ..)` — the latter ignores width/
        // alignment specifiers like `{:<10}`, which `nodes.rs`'s status
        // table relies on.
        f.pad(match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        })
    }
}

/// Expand a leading `~` (or `~/...`) against `$HOME`. Paths without a leading
/// `~` are returned unchanged.
pub fn expand_tilde(path: &Path) -> PathBuf {
    let Ok(rest) = path.strip_prefix("~") else {
        return path.to_path_buf();
    };
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(rest),
        None => path.to_path_buf(),
    }
}

impl Config {
    /// Validate cross-field invariants that `serde` alone can't express:
    /// non-empty node list, unique names, and (if set) a readable `ssh_key`.
    pub fn validate(&self) -> Result<()> {
        if self.nodes.is_empty() {
            anyhow::bail!("nodes.yaml must define at least one node");
        }

        let mut seen = HashSet::new();
        for node in &self.nodes {
            if !seen.insert(node.name.as_str()) {
                anyhow::bail!("duplicate node name: '{}'", node.name);
            }
            if let Some(key) = node.expanded_ssh_key() {
                if !key.exists() {
                    anyhow::bail!("node '{}': ssh_key not found: {}", node.name, key.display());
                }
            }
        }

        Ok(())
    }
}

/// Load and validate a `nodes.yaml` configuration file.
pub fn load_config(path: &Path) -> Result<Config> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;
    let config: Config = serde_yaml::from_str(&text)
        .with_context(|| format!("Failed to parse YAML config: {}", path.display()))?;
    config.validate()?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_yaml() -> &'static str {
        r"
coordinator:
  results_dir: data/bench/distributed
nodes:
  - name: local
    host: localhost
    arch: x86_64
  - name: sydney
    host: ec2-user@example.com
    arch: aarch64
    max_concurrent: 4
"
    }

    #[test]
    fn parses_valid_config_with_defaults() {
        let config: Config = serde_yaml::from_str(sample_yaml()).unwrap();
        config.validate().unwrap();

        assert_eq!(config.nodes.len(), 2);
        assert_eq!(config.coordinator.ssh_timeout, 30);
        assert_eq!(config.coordinator.benchmark_timeout, 3600);
        assert_eq!(config.coordinator.parallelism, Parallelism::PerNode);
        assert_eq!(config.nodes[0].max_concurrent, 1);
        assert_eq!(config.nodes[1].max_concurrent, 4);
        assert!(config.nodes[0].is_local());
        assert!(!config.nodes[1].is_local());
    }

    #[test]
    fn rejects_invalid_yaml() {
        let err = serde_yaml::from_str::<Config>("not: [valid, config").unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn rejects_empty_node_list() {
        let config: Config = serde_yaml::from_str(
            r"
coordinator:
  results_dir: data/bench/distributed
nodes: []
",
        )
        .unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("at least one node"));
    }

    #[test]
    fn rejects_duplicate_node_names() {
        let config: Config = serde_yaml::from_str(
            r"
coordinator:
  results_dir: data/bench/distributed
nodes:
  - name: dup
    host: localhost
    arch: x86_64
  - name: dup
    host: localhost
    arch: aarch64
",
        )
        .unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("duplicate node name"));
    }

    #[test]
    fn rejects_missing_ssh_key() {
        let config: Config = serde_yaml::from_str(
            r"
coordinator:
  results_dir: data/bench/distributed
nodes:
  - name: remote
    host: example.com
    arch: aarch64
    ssh_key: /nonexistent/path/to/key.pem
",
        )
        .unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("ssh_key not found"));
    }

    #[test]
    fn expands_tilde_in_ssh_key() {
        let node = NodeConfig {
            name: "n".to_string(),
            host: "h".to_string(),
            arch: Architecture::X86_64,
            features: vec![],
            max_concurrent: 1,
            ssh_key: Some(PathBuf::from("~/.ssh/id_rsa")),
            working_dir: None,
            target_triple: None,
            ec2_instance_id: None,
            ec2_region: None,
        };
        let expanded = node.expanded_ssh_key().unwrap();
        assert!(!expanded.to_string_lossy().starts_with('~'));
    }
}
