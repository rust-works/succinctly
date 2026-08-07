//! Node status and EC2 lifecycle management (Phase 4, issue #98).
//!
//! `bench nodes` reports each configured node's reachability (and, for
//! EC2-backed nodes, instance state), and can start/stop those instances.
//! AWS CLI invocation is a distinct local side effect from SSH exec, so it
//! gets its own thin [`Ec2Control`] abstraction — the same
//! dependency-injection shape as `sync.rs`'s `BuildRunner`.
//!
//! `bench orchestrate` deliberately does **not** auto-start a stopped EC2
//! node — that's a silent, cost-incurring side effect a user should choose
//! explicitly via `bench nodes --start`.

use super::config::{load_config, Config, NodeConfig};
use super::ssh::RemoteExec;
use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// `succinctly bench nodes` — report node status, or start/stop EC2 instances.
#[derive(Debug, Parser)]
pub struct NodesArgs {
    /// Config file describing nodes and coordinator settings
    #[arg(short, long, default_value = "nodes.yaml")]
    pub config: PathBuf,

    /// Start any stopped EC2 instances among the configured nodes
    #[arg(long)]
    pub start: bool,

    /// Stop any running EC2 instances among the configured nodes
    #[arg(long)]
    pub stop: bool,

    /// Show detailed node status (the default when no flag is given)
    #[arg(long)]
    pub status: bool,
}

/// Run `bench nodes`.
pub fn run_nodes(args: NodesArgs) -> Result<()> {
    run_nodes_with(args, &super::ssh::SystemSsh, &SystemAwsCli)
}

pub(crate) fn run_nodes_with(
    args: NodesArgs,
    remote: &dyn RemoteExec,
    ec2: &dyn Ec2Control,
) -> Result<()> {
    let config = load_config(&args.config)?;

    if args.start {
        for_each_ec2_node(&config, ec2, "starting", &|ec2, id, region| {
            ec2.start(id, region)
        });
        return Ok(());
    }

    if args.stop {
        for_each_ec2_node(&config, ec2, "stopping", &|ec2, id, region| {
            ec2.stop(id, region)
        });
        return Ok(());
    }

    print_status_table(&config, remote, ec2);
    Ok(())
}

fn for_each_ec2_node(
    config: &Config,
    ec2: &dyn Ec2Control,
    verb: &str,
    action: &dyn Fn(&dyn Ec2Control, &str, &str) -> Result<()>,
) {
    let mut acted = false;
    for node in &config.nodes {
        if let Some(id) = &node.ec2_instance_id {
            acted = true;
            let region = node.ec2_region.as_deref().unwrap_or_default();
            eprintln!("[{}] {verb} EC2 instance {id}...", node.name);
            if let Err(e) = action(ec2, id, region) {
                eprintln!("Warning: failed to {verb} node '{}': {e}", node.name);
            }
        }
    }
    if !acted {
        eprintln!("No EC2-backed nodes (with ec2_instance_id) in this config.");
    }
}

fn print_status_table(config: &Config, remote: &dyn RemoteExec, ec2: &dyn Ec2Control) {
    println!("{:<20} {:<10} {:<12} HOST", "NAME", "ARCH", "STATUS");
    println!("{:-<20} {:-<10} {:-<12} {:-<20}", "", "", "", "");
    for node in &config.nodes {
        let status = node_status(node, remote, ec2);
        println!(
            "{:<20} {:<10} {:<12} {}",
            node.name, node.arch, status, node.host
        );
    }
}

fn node_status(node: &NodeConfig, remote: &dyn RemoteExec, ec2: &dyn Ec2Control) -> String {
    if let Some(id) = &node.ec2_instance_id {
        let region = node.ec2_region.as_deref().unwrap_or_default();
        match ec2.describe(id, region) {
            Ok(Ec2State::Stopped) => return "stopped".to_string(),
            Ok(Ec2State::Other(state)) => return state,
            Ok(Ec2State::Running) => {} // fall through to an SSH reachability check
            Err(_) => return "unknown".to_string(),
        }
    }

    if node.is_local() {
        return "ready".to_string();
    }

    match remote.exec(node, "echo ok", Duration::from_secs(5)) {
        Ok(out) if out.success() => "ready".to_string(),
        _ => "unreachable".to_string(),
    }
}

/// An EC2 instance's reported state, collapsed to the cases this tool acts
/// on (`Running`/`Stopped`) plus a catch-all for AWS's other states
/// (`pending`, `stopping`, `terminated`, …), surfaced verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ec2State {
    Running,
    Stopped,
    Other(String),
}

/// Abstraction over AWS EC2 instance control — the local (non-`RemoteExec`,
/// non-`BuildRunner`) side effect specific to `bench nodes`. `SystemAwsCli`
/// shells to the real `aws` CLI; tests use an in-memory fake instead.
pub trait Ec2Control: Send + Sync {
    fn describe(&self, instance_id: &str, region: &str) -> Result<Ec2State>;
    fn start(&self, instance_id: &str, region: &str) -> Result<()>;
    fn stop(&self, instance_id: &str, region: &str) -> Result<()>;
}

/// Real implementation: shells to the system `aws` CLI. Requires the AWS
/// CLI to be installed and configured with credentials for `region` — a
/// documented precondition, not something this tool sets up.
pub struct SystemAwsCli;

impl Ec2Control for SystemAwsCli {
    fn describe(&self, instance_id: &str, region: &str) -> Result<Ec2State> {
        let output = Command::new("aws")
            .args([
                "ec2",
                "describe-instances",
                "--instance-ids",
                instance_id,
                "--region",
                region,
                "--query",
                "Reservations[0].Instances[0].State.Name",
                "--output",
                "text",
            ])
            .output()
            .context("failed to spawn aws ec2 describe-instances")?;
        if !output.status.success() {
            anyhow::bail!(
                "aws ec2 describe-instances failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let state = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(match state.as_str() {
            "running" => Ec2State::Running,
            "stopped" => Ec2State::Stopped,
            other => Ec2State::Other(other.to_string()),
        })
    }

    fn start(&self, instance_id: &str, region: &str) -> Result<()> {
        run_aws_ec2(&[
            "start-instances",
            "--instance-ids",
            instance_id,
            "--region",
            region,
        ])
    }

    fn stop(&self, instance_id: &str, region: &str) -> Result<()> {
        run_aws_ec2(&[
            "stop-instances",
            "--instance-ids",
            instance_id,
            "--region",
            region,
        ])
    }
}

fn run_aws_ec2(args: &[&str]) -> Result<()> {
    let mut full_args = vec!["ec2"];
    full_args.extend_from_slice(args);
    let output = Command::new("aws")
        .args(&full_args)
        .output()
        .context("failed to spawn aws ec2")?;
    if !output.status.success() {
        anyhow::bail!(
            "aws {}: {}",
            full_args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::config::Architecture;
    use super::super::test_support::{FakeAwsCli, FakeExec};
    use super::*;

    fn config_yaml() -> &'static str {
        r"
coordinator:
  results_dir: data/bench/distributed
nodes:
  - name: local
    host: localhost
    arch: x86_64
  - name: tailscale-node
    host: user@example.ts.net
    arch: aarch64
  - name: ec2-node
    host: ec2-user@example.com
    arch: aarch64
    ec2_instance_id: i-0123456789abcdef0
    ec2_region: us-east-1
"
    }

    fn args(config: PathBuf, extra: &[&str]) -> NodesArgs {
        let mut argv = vec!["nodes", "--config"];
        let config_str = config.to_str().unwrap().to_string();
        argv.push(&config_str);
        argv.extend_from_slice(extra);
        NodesArgs::try_parse_from(argv).unwrap()
    }

    fn write_config(dir: &std::path::Path, yaml: &str) -> PathBuf {
        let path = dir.join("nodes.yaml");
        std::fs::write(&path, yaml).unwrap();
        path
    }

    #[test]
    fn start_only_acts_on_ec2_backed_nodes() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = write_config(dir.path(), config_yaml());
        let parsed = args(config_path, &["--start"]);

        let ec2 = FakeAwsCli::new();
        run_nodes_with(parsed, &FakeExec::new(), &ec2).unwrap();

        assert_eq!(ec2.start_calls(), vec!["i-0123456789abcdef0".to_string()]);
        assert!(ec2.stop_calls().is_empty());
    }

    #[test]
    fn stop_only_acts_on_ec2_backed_nodes() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = write_config(dir.path(), config_yaml());
        let parsed = args(config_path, &["--stop"]);

        let ec2 = FakeAwsCli::new();
        run_nodes_with(parsed, &FakeExec::new(), &ec2).unwrap();

        assert_eq!(ec2.stop_calls(), vec!["i-0123456789abcdef0".to_string()]);
        assert!(ec2.start_calls().is_empty());
    }

    #[test]
    fn status_reports_stopped_for_stopped_ec2_node_without_ssh_check() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = write_config(dir.path(), config_yaml());
        let parsed = args(config_path, &["--status"]);

        let ec2 = FakeAwsCli::new().set_state("i-0123456789abcdef0", Ec2State::Stopped);
        let remote = FakeExec::new();
        run_nodes_with(parsed, &remote, &ec2).unwrap();

        // A stopped instance must never be SSH-probed.
        assert!(!remote.calls().iter().any(|(n, _)| n == "ec2-node"));
    }

    #[test]
    fn status_probes_ssh_for_non_ec2_remote_nodes() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = write_config(dir.path(), config_yaml());
        let parsed = args(config_path, &["--status"]);

        let ec2 = FakeAwsCli::new().set_state("i-0123456789abcdef0", Ec2State::Running);
        let remote = FakeExec::new();
        run_nodes_with(parsed, &remote, &ec2).unwrap();

        assert!(remote
            .calls()
            .iter()
            .any(|(n, cmd)| n == "tailscale-node" && cmd.contains("echo ok")));
        // Local nodes are always "ready" without an SSH round-trip.
        assert!(!remote.calls().iter().any(|(n, _)| n == "local"));
    }

    #[test]
    fn node_status_local_is_always_ready() {
        let node = NodeConfig {
            name: "local".to_string(),
            host: "localhost".to_string(),
            arch: Architecture::X86_64,
            features: vec![],
            max_concurrent: 1,
            ssh_key: None,
            working_dir: None,
            target_triple: None,
            ec2_instance_id: None,
            ec2_region: None,
        };
        assert_eq!(
            node_status(&node, &FakeExec::new(), &FakeAwsCli::new()),
            "ready"
        );
    }
}
