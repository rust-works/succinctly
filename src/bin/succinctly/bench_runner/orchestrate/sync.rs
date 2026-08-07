//! Binary sync (Phase 3, issue #98): cross-compile the release binary for a
//! node's target triple, upload it if the node's reported version differs
//! from the local one (or `--force`), and re-verify afterward.
//!
//! Cross-compilation itself is a local side effect (unlike everything else
//! in `orchestrate/`, which talks to a node via [`RemoteExec`]), so it gets
//! its own small [`BuildRunner`] abstraction — the same dependency-injection
//! shape as `ssh.rs`'s `RemoteExec`, so tests never shell out to a real
//! `cargo build`.

use super::config::{load_config, Architecture, Config, NodeConfig};
use super::ssh::RemoteExec;
use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// `succinctly bench sync` — cross-compile and deploy the release binary to
/// configured nodes.
#[derive(Debug, Parser)]
pub struct SyncArgs {
    /// Config file describing nodes and coordinator settings
    #[arg(short, long, default_value = "nodes.yaml")]
    pub config: PathBuf,

    /// Sync only specific node(s) (repeatable)
    #[arg(short = 'n', long = "node")]
    pub node: Vec<String>,

    /// Force sync even if the reported version already matches
    #[arg(long)]
    pub force: bool,

    /// Show what would sync without cross-compiling or uploading
    #[arg(long)]
    pub dry_run: bool,
}

/// Run `bench sync`.
pub fn run_sync(args: SyncArgs) -> Result<()> {
    run_sync_with(args, &super::ssh::SystemSsh, &SystemBuild)
}

pub(crate) fn run_sync_with(
    args: SyncArgs,
    remote: &dyn RemoteExec,
    build: &dyn BuildRunner,
) -> Result<()> {
    let config = load_config(&args.config)?;
    let nodes = select_nodes(&config, &args.node);
    if nodes.is_empty() {
        anyhow::bail!("No nodes selected. Check --node against nodes.yaml.");
    }

    let connect_timeout = Duration::from_secs(config.coordinator.ssh_timeout);
    for node in &nodes {
        if let Err(e) = sync_node(
            remote,
            build,
            node,
            args.force,
            args.dry_run,
            connect_timeout,
        ) {
            eprintln!("Warning: sync to node '{}' failed: {e}", node.name);
        }
    }

    Ok(())
}

fn select_nodes<'a>(config: &'a Config, names: &[String]) -> Vec<&'a NodeConfig> {
    let mut nodes: Vec<&NodeConfig> = config.nodes.iter().collect();
    if !names.is_empty() {
        nodes.retain(|n| names.contains(&n.name));
    }
    nodes
}

/// Map a node's architecture (+ optional explicit `target_triple`) to a Rust
/// target triple. The explicit override is needed because `arch` alone
/// can't disambiguate e.g. `aarch64-apple-darwin` vs
/// `aarch64-unknown-linux-gnu`.
pub(crate) fn target_triple_for(arch: Architecture, explicit: Option<&str>) -> String {
    if let Some(triple) = explicit {
        return triple.to_string();
    }
    match arch {
        Architecture::X86_64 => "x86_64-unknown-linux-gnu".to_string(),
        Architecture::Aarch64 => "aarch64-unknown-linux-gnu".to_string(),
    }
}

fn local_version_string() -> String {
    format!("succinctly {}", env!("CARGO_PKG_VERSION"))
}

/// Check-and-conditionally-sync one node: skips local nodes entirely (a
/// no-op, there's nothing to upload to), checks the remote `--version`
/// against the local build, and only cross-compiles/uploads/re-verifies if
/// they differ (or `--force`).
pub(crate) fn sync_node(
    remote: &dyn RemoteExec,
    build: &dyn BuildRunner,
    node: &NodeConfig,
    force: bool,
    dry_run: bool,
    connect_timeout: Duration,
) -> Result<()> {
    if node.is_local() {
        eprintln!("[{}] local node, sync is a no-op", node.name);
        return Ok(());
    }

    let working_dir = node.working_dir_str();
    let version_cmd = format!("cd {working_dir} && ./target/release/succinctly --version");
    let remote_version = remote
        .exec(node, &version_cmd, connect_timeout)
        .ok()
        .filter(super::ssh::ExecOutput::success)
        .map(|r| r.stdout.trim().to_string());

    let local_version = local_version_string();
    let up_to_date = !force && remote_version.as_deref() == Some(local_version.as_str());

    if up_to_date {
        eprintln!("[{}] up to date ({local_version})", node.name);
        return Ok(());
    }

    let triple = target_triple_for(node.arch, node.target_triple.as_deref());

    if dry_run {
        eprintln!(
            "[{}] dry run: would cross-compile for {triple} and upload",
            node.name
        );
        return Ok(());
    }

    eprintln!("[{}] syncing (target {triple})...", node.name);
    let local_binary = build
        .cross_compile(&triple)
        .with_context(|| format!("cross-compile for {triple} failed"))?;

    let remote_dir = format!("{working_dir}/target/release");
    let _ = remote.exec(node, &format!("mkdir -p {remote_dir}"), connect_timeout);

    let remote_binary = format!("{remote_dir}/succinctly");
    remote.upload(node, &local_binary, &remote_binary, connect_timeout)?;

    let verify = remote.exec(node, &version_cmd, connect_timeout)?;
    if !verify.success() {
        anyhow::bail!(
            "post-upload version check failed on node '{}': {}",
            node.name,
            verify.stderr
        );
    }
    eprintln!("[{}] synced: {}", node.name, verify.stdout.trim());

    Ok(())
}

/// Abstraction over "produce a release binary for this target triple" — the
/// one local (non-`RemoteExec`) side effect in `orchestrate/`. `SystemBuild`
/// shells to a real `cargo build`; tests use a fake that returns a path to a
/// small stand-in file instead of cross-compiling for real.
pub trait BuildRunner: Send + Sync {
    /// Returns the path to the built binary on success.
    fn cross_compile(&self, target_triple: &str) -> Result<PathBuf>;
}

/// Real implementation: `cargo build --release --target <triple> --features
/// bench-runner`. Requires the target's toolchain to already be installed
/// (`rustup target add <triple>`) — that's a documented precondition, not
/// something this tool installs.
pub struct SystemBuild;

impl BuildRunner for SystemBuild {
    fn cross_compile(&self, target_triple: &str) -> Result<PathBuf> {
        let status = Command::new("cargo")
            .args([
                "build",
                "--release",
                "--target",
                target_triple,
                "--features",
                "bench-runner",
            ])
            .status()
            .context("failed to spawn cargo build")?;
        if !status.success() {
            anyhow::bail!("cargo build --target {target_triple} failed");
        }

        let path = PathBuf::from(format!("target/{target_triple}/release/succinctly"));
        if !path.exists() {
            anyhow::bail!(
                "expected cross-compiled binary not found at {}",
                path.display()
            );
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::super::ssh::ExecOutput;
    use super::super::test_support::{FakeBuild, FakeExec};
    use super::*;

    fn node(name: &str, target_triple: Option<&str>) -> NodeConfig {
        NodeConfig {
            name: name.to_string(),
            host: "example.com".to_string(),
            arch: Architecture::Aarch64,
            features: vec![],
            max_concurrent: 1,
            ssh_key: None,
            working_dir: None,
            target_triple: target_triple.map(str::to_string),
            ec2_instance_id: None,
            ec2_region: None,
        }
    }

    #[test]
    fn target_triple_prefers_explicit_override() {
        assert_eq!(
            target_triple_for(Architecture::Aarch64, Some("aarch64-apple-darwin")),
            "aarch64-apple-darwin"
        );
    }

    #[test]
    fn target_triple_falls_back_by_arch() {
        assert_eq!(
            target_triple_for(Architecture::X86_64, None),
            "x86_64-unknown-linux-gnu"
        );
        assert_eq!(
            target_triple_for(Architecture::Aarch64, None),
            "aarch64-unknown-linux-gnu"
        );
    }

    #[test]
    fn matching_version_skips_cross_compile_and_upload() {
        let n = node("sydney", None);
        let fake = FakeExec::new().respond(
            "sydney",
            "--version",
            ExecOutput {
                stdout: local_version_string(),
                stderr: String::new(),
                exit_code: 0,
            },
        );
        let build = FakeBuild::new();

        sync_node(&fake, &build, &n, false, false, Duration::from_secs(5)).unwrap();

        assert_eq!(build.call_count(), 0);
        assert!(!fake.calls().iter().any(|(_, cmd)| cmd.contains("mkdir")));
    }

    #[test]
    fn mismatched_version_triggers_cross_compile_and_upload() {
        let n = node("sydney", None);
        let fake = FakeExec::new().respond(
            "sydney",
            "--version",
            ExecOutput {
                stdout: "succinctly 0.0.1-stale".to_string(),
                stderr: String::new(),
                exit_code: 0,
            },
        );
        let build = FakeBuild::new();

        sync_node(&fake, &build, &n, false, false, Duration::from_secs(5)).unwrap();

        assert_eq!(build.call_count(), 1);
        assert_eq!(
            build.last_triple(),
            Some("aarch64-unknown-linux-gnu".to_string())
        );
    }

    #[test]
    fn force_triggers_cross_compile_even_when_versions_match() {
        let n = node("sydney", None);
        let fake = FakeExec::new().respond(
            "sydney",
            "--version",
            ExecOutput {
                stdout: local_version_string(),
                stderr: String::new(),
                exit_code: 0,
            },
        );
        let build = FakeBuild::new();

        sync_node(&fake, &build, &n, true, false, Duration::from_secs(5)).unwrap();

        assert_eq!(build.call_count(), 1);
    }

    #[test]
    fn dry_run_never_cross_compiles() {
        let n = node("sydney", None);
        let fake = FakeExec::new().respond(
            "sydney",
            "--version",
            ExecOutput {
                stdout: "succinctly 0.0.1-stale".to_string(),
                stderr: String::new(),
                exit_code: 0,
            },
        );
        let build = FakeBuild::new();

        sync_node(&fake, &build, &n, false, true, Duration::from_secs(5)).unwrap();

        assert_eq!(build.call_count(), 0);
    }

    #[test]
    fn local_node_is_always_a_no_op() {
        let mut n = node("local", None);
        n.host = "localhost".to_string();
        let fake = FakeExec::new();
        let build = FakeBuild::new();

        sync_node(&fake, &build, &n, true, false, Duration::from_secs(5)).unwrap();

        assert!(fake.calls().is_empty());
        assert_eq!(build.call_count(), 0);
    }
}
