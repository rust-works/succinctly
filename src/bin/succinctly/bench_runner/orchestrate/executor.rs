//! Phase 2 orchestration executor: isolation-aware parallel execution across
//! selected nodes, using `scheduler.rs`'s wave packing.
//!
//! Each selected node runs on its own thread (`std::thread::scope`); within
//! a node, waves run sequentially and the jobs inside a wave run
//! concurrently, bounded by the node's `max_concurrent`. Result download
//! and aggregation happen after every node finishes executing.

use super::aggregate::{aggregate_results, write_run_metadata, RunMetadata};
use super::config::{load_config, Architecture, BenchmarkOverride, Config, NodeConfig};
use super::scheduler::{schedule_node, Wave};
use super::ssh::RemoteExec;
use super::sync::{sync_node, BuildRunner};
use crate::bench_runner::registry::BenchmarkInfo;
use anyhow::{Context, Result};
use clap::Parser;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// `succinctly bench orchestrate` — run benchmarks across configured nodes.
#[derive(Debug, Parser)]
pub struct OrchestrateArgs {
    /// Config file describing nodes and coordinator settings
    #[arg(short, long, default_value = "nodes.yaml")]
    pub config: PathBuf,

    /// Run only on specific node(s) (repeatable)
    #[arg(short = 'n', long = "node")]
    pub node: Vec<String>,

    /// Run only on nodes with this architecture
    #[arg(short, long)]
    pub arch: Option<Architecture>,

    /// Benchmark names to run
    #[arg(value_name = "BENCHMARKS")]
    pub benchmarks: Vec<String>,

    /// Run all registered benchmarks
    #[arg(long)]
    pub all: bool,

    /// Show what would run without executing anything
    #[arg(long)]
    pub dry_run: bool,

    /// Skip the binary sync step during node preparation
    #[arg(long)]
    pub no_sync: bool,

    /// Only collect/aggregate results from a prior run, don't execute
    #[arg(long)]
    pub collect_only: bool,
}

/// Run `bench orchestrate`.
pub fn run_orchestrate(args: OrchestrateArgs) -> Result<()> {
    // Ctrl+C registration is process-global and can only succeed once, so it
    // belongs in this real entry point (invoked once per process) rather
    // than in `run_orchestrate_with` — tests call that directly, in-process,
    // multiple times, injecting their own cancellation flag instead.
    let cancelled = Arc::new(AtomicBool::new(false));
    let handler_flag = Arc::clone(&cancelled);
    ctrlc::set_handler(move || {
        handler_flag.store(true, Ordering::SeqCst);
        eprintln!("\nInterrupted! Finishing in-flight benchmarks, then stopping...");
    })
    .context("Failed to set Ctrl+C handler")?;

    run_orchestrate_with(
        args,
        &super::ssh::SystemSsh,
        &super::sync::SystemBuild,
        &cancelled,
    )
}

/// Testable entry point: takes the [`RemoteExec`]/[`BuildRunner`]
/// implementations and the cancellation flag as parameters, so unit tests
/// can inject fakes and a fresh local flag instead of touching global
/// process state or shelling to real `ssh`/`cargo build`.
pub(crate) fn run_orchestrate_with(
    args: OrchestrateArgs,
    remote: &dyn RemoteExec,
    build: &dyn BuildRunner,
    cancelled: &AtomicBool,
) -> Result<()> {
    let config = load_config(&args.config)?;

    let nodes = select_nodes(&config, &args);
    if nodes.is_empty() {
        anyhow::bail!("No nodes selected. Check --node/--arch against nodes.yaml.");
    }

    let benchmarks = select_benchmarks(&args);
    if benchmarks.is_empty() {
        anyhow::bail!("No benchmarks selected. Use --all or specify benchmark names.");
    }

    let run_id = run_id_now();
    let run_dir = config.coordinator.results_dir.join(&run_id);

    if args.dry_run {
        print_dry_run_plan(&nodes, &benchmarks, &config.benchmarks, &run_dir);
        return Ok(());
    }

    let started_at = timestamp_now();
    std::fs::create_dir_all(&run_dir)
        .with_context(|| format!("Failed to create {}", run_dir.display()))?;

    if !args.collect_only {
        let print_lock = Mutex::new(());
        let ssh_timeout = Duration::from_secs(config.coordinator.ssh_timeout);
        let no_sync = args.no_sync;
        std::thread::scope(|scope| {
            for node in &nodes {
                let config = &config;
                let benchmarks = &benchmarks;
                let run_id = &run_id;
                let print_lock = &print_lock;
                scope.spawn(move || {
                    if !no_sync {
                        if let Err(e) = sync_node(remote, build, node, false, false, ssh_timeout) {
                            log(
                                print_lock,
                                &format!("Warning: sync to node '{}' failed: {e}", node.name),
                            );
                        }
                    }
                    if let Err(e) = run_node_scheduled(
                        remote, config, node, benchmarks, run_id, print_lock, cancelled,
                    ) {
                        log(
                            print_lock,
                            &format!("Warning: node '{}' failed: {e}", node.name),
                        );
                    }
                });
            }
        });
    }

    for node in &nodes {
        if let Err(e) = collect_node_results(remote, node, &run_id, &run_dir) {
            eprintln!(
                "Warning: failed to collect results from node '{}': {e}",
                node.name
            );
        }
    }

    let node_names: Vec<String> = nodes.iter().map(|n| n.name.clone()).collect();
    let benchmark_names: Vec<String> = benchmarks.iter().map(|b| b.name.to_string()).collect();
    aggregate_results(&run_dir, &node_names)?;

    let meta = RunMetadata {
        run_id: run_id.clone(),
        git_commit: crate::bench_runner::metadata::collect_git_info()
            .ok()
            .map(|g| g.commit),
        config_path: args.config.display().to_string(),
        nodes: node_names,
        benchmarks: benchmark_names,
        started_at,
        completed_at: timestamp_now(),
    };
    write_run_metadata(&run_dir, &meta)?;

    eprintln!("Orchestration run complete: {}", run_dir.display());
    Ok(())
}

fn run_id_now() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H-%M-%S").to_string()
}

fn timestamp_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Select nodes matching `--node`/`--arch`, defaulting to all configured
/// nodes when neither flag is given.
fn select_nodes<'a>(config: &'a Config, args: &OrchestrateArgs) -> Vec<&'a NodeConfig> {
    let mut nodes: Vec<&NodeConfig> = config.nodes.iter().collect();
    if !args.node.is_empty() {
        nodes.retain(|n| args.node.contains(&n.name));
    }
    if let Some(arch) = args.arch {
        nodes.retain(|n| n.arch == arch);
    }
    nodes
}

/// Select benchmarks from `--all` or positional args, warning (not failing)
/// on names not present in the registry.
fn select_benchmarks(args: &OrchestrateArgs) -> Vec<&'static BenchmarkInfo> {
    use crate::bench_runner::registry::{filter_by_names, BENCHMARKS};

    if args.all {
        return BENCHMARKS.iter().collect();
    }

    if !args.benchmarks.is_empty() {
        let found = filter_by_names(&args.benchmarks);
        for name in &args.benchmarks {
            if !found.iter().any(|b| b.name == name) {
                eprintln!("Warning: unknown benchmark '{name}'");
            }
        }
        return found;
    }

    Vec::new()
}

fn print_dry_run_plan(
    nodes: &[&NodeConfig],
    benchmarks: &[&'static BenchmarkInfo],
    overrides: &[BenchmarkOverride],
    run_dir: &Path,
) {
    println!(
        "Dry run - would execute {} benchmark(s) across {} node(s):",
        benchmarks.len(),
        nodes.len()
    );
    for node in nodes {
        println!(
            "  {} ({}, {}, max_concurrent={})",
            node.name, node.arch, node.host, node.max_concurrent
        );
        let schedule = schedule_node(node, benchmarks, overrides);
        for (i, wave) in schedule.waves.iter().enumerate() {
            let names: Vec<&str> = wave.iter().map(|b| b.name).collect();
            println!("    wave {}: {}", i + 1, names.join(", "));
        }
    }
    println!("Results would be written under: {}", run_dir.display());
}

/// Connectivity check, then run each wave of `node`'s schedule in order,
/// stopping (without erroring) before the next wave if `cancelled` is set.
/// A single benchmark failing does not stop the rest.
fn run_node_scheduled(
    remote: &dyn RemoteExec,
    config: &Config,
    node: &NodeConfig,
    benchmarks: &[&'static BenchmarkInfo],
    run_id: &str,
    print_lock: &Mutex<()>,
    cancelled: &AtomicBool,
) -> Result<()> {
    let ssh_timeout = Duration::from_secs(config.coordinator.ssh_timeout);
    let check = remote
        .exec(node, "echo ok", ssh_timeout)
        .with_context(|| format!("connectivity check to node '{}' failed", node.name))?;
    if !check.success() {
        anyhow::bail!(
            "connectivity check to node '{}' failed: {}",
            node.name,
            check.stderr
        );
    }

    let schedule = schedule_node(node, benchmarks, &config.benchmarks);
    let benchmark_timeout = Duration::from_secs(config.coordinator.benchmark_timeout);
    let working_dir = node.working_dir_str();

    for wave in &schedule.waves {
        if cancelled.load(Ordering::SeqCst) {
            log(
                print_lock,
                &format!("[{}] cancelled, stopping before next wave", node.name),
            );
            break;
        }
        run_wave(
            remote,
            node,
            wave,
            run_id,
            working_dir,
            benchmark_timeout,
            print_lock,
        );
    }

    Ok(())
}

/// Run every job in `wave` concurrently, joining before returning.
fn run_wave(
    remote: &dyn RemoteExec,
    node: &NodeConfig,
    wave: &Wave<'static>,
    run_id: &str,
    working_dir: &str,
    benchmark_timeout: Duration,
    print_lock: &Mutex<()>,
) {
    std::thread::scope(|scope| {
        for benchmark in wave.iter().copied() {
            scope.spawn(move || {
                run_one_benchmark(
                    remote,
                    node,
                    benchmark.name,
                    run_id,
                    working_dir,
                    benchmark_timeout,
                    print_lock,
                );
            });
        }
    });
}

fn run_one_benchmark(
    remote: &dyn RemoteExec,
    node: &NodeConfig,
    name: &str,
    run_id: &str,
    working_dir: &str,
    benchmark_timeout: Duration,
    print_lock: &Mutex<()>,
) {
    let remote_output_dir = format!("/tmp/bench-{run_id}/{name}");
    let command = format!(
        "cd {working_dir} && ./target/release/succinctly bench run {name} --output-dir {remote_output_dir}"
    );
    log(print_lock, &format!("[{}] running {name}...", node.name));
    match remote.exec(node, &command, benchmark_timeout) {
        Ok(result) if result.success() => {
            log(print_lock, &format!("[{}] {name} OK", node.name));
        }
        Ok(result) => {
            log(
                print_lock,
                &format!(
                    "Warning: benchmark '{name}' failed on node '{}': {}",
                    node.name, result.stderr
                ),
            );
        }
        Err(e) => {
            log(
                print_lock,
                &format!(
                    "Warning: benchmark '{name}' errored on node '{}': {e}",
                    node.name
                ),
            );
        }
    }
}

/// `eprintln!` serialized behind `print_lock` so concurrent node/wave
/// threads never interleave partial lines.
fn log(print_lock: &Mutex<()>, message: &str) {
    let _guard = print_lock.lock().unwrap();
    eprintln!("{message}");
}

/// Download a node's results directory, write its `node_info.json`, and
/// best-effort clean up the remote scratch directory.
fn collect_node_results(
    remote: &dyn RemoteExec,
    node: &NodeConfig,
    run_id: &str,
    run_dir: &Path,
) -> Result<()> {
    let local_dir = run_dir.join(&node.name);
    let remote_dir = format!("/tmp/bench-{run_id}");
    remote.download(node, &remote_dir, &local_dir, Duration::from_secs(30))?;

    std::fs::create_dir_all(&local_dir)
        .with_context(|| format!("Failed to create {}", local_dir.display()))?;
    let info = serde_json::json!({
        "name": node.name,
        "host": node.host,
        "arch": node.arch,
        "features": node.features,
    });
    std::fs::write(
        local_dir.join("node_info.json"),
        serde_json::to_string_pretty(&info)?,
    )
    .with_context(|| format!("Failed to write {}/node_info.json", local_dir.display()))?;

    // Best-effort: a failed cleanup shouldn't fail the whole collection step.
    let _ = remote.exec(
        node,
        &format!("rm -rf {remote_dir}"),
        Duration::from_secs(30),
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{FakeBuild, FakeExec};
    use super::*;

    fn args(config: PathBuf, extra: &[&str]) -> OrchestrateArgs {
        let mut argv = vec!["orchestrate", "--config"];
        let config_str = config.to_str().unwrap().to_string();
        argv.push(&config_str);
        argv.extend_from_slice(extra);
        OrchestrateArgs::try_parse_from(argv).unwrap()
    }

    fn write_config(dir: &Path, yaml: &str) -> PathBuf {
        let path = dir.join("nodes.yaml");
        std::fs::write(&path, yaml).unwrap();
        path
    }

    fn run(parsed: OrchestrateArgs, remote: &dyn RemoteExec) -> Result<()> {
        run_orchestrate_with(parsed, remote, &FakeBuild::new(), &AtomicBool::new(false))
    }

    const TWO_NODE_YAML: &str = r"
coordinator:
  results_dir: data/bench/distributed
nodes:
  - name: local
    host: localhost
    arch: x86_64
  - name: sydney
    host: sydney.example.com
    arch: aarch64
";

    #[test]
    fn select_nodes_filters_by_name_and_arch() {
        let config: Config = serde_yaml::from_str(TWO_NODE_YAML).unwrap();

        let a = OrchestrateArgs {
            config: PathBuf::new(),
            node: vec!["sydney".to_string()],
            arch: None,
            benchmarks: vec![],
            all: false,
            dry_run: false,
            no_sync: false,
            collect_only: false,
        };
        assert_eq!(select_nodes(&config, &a).len(), 1);

        let b = OrchestrateArgs {
            config: PathBuf::new(),
            node: vec![],
            arch: Some(Architecture::Aarch64),
            benchmarks: vec![],
            all: false,
            dry_run: false,
            no_sync: false,
            collect_only: false,
        };
        let filtered = select_nodes(&config, &b);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "sydney");
    }

    #[test]
    fn dry_run_never_calls_exec() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = write_config(dir.path(), TWO_NODE_YAML);
        let parsed = args(config_path, &["--all", "--dry-run"]);

        let fake = FakeExec::new();
        run(parsed, &fake).unwrap();

        assert!(fake.calls().is_empty());
    }

    #[test]
    fn sequential_run_executes_each_selected_benchmark() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = format!(
            "coordinator:\n  results_dir: {}\nnodes:\n  - name: local\n    host: localhost\n    arch: x86_64\n",
            dir.path().join("results").display()
        );
        let config_path = write_config(dir.path(), &yaml);
        let parsed = args(config_path, &["rank_select"]);

        let fake = FakeExec::new();
        run(parsed, &fake).unwrap();

        let calls = fake.calls();
        // connectivity check + 1 benchmark + cleanup = 3 exec calls
        assert_eq!(calls.len(), 3);
        assert!(calls[0].1.contains("echo ok"));
        assert!(calls[1].1.contains("bench run rank_select"));
        assert!(calls[2].1.contains("rm -rf"));
    }

    #[test]
    fn one_node_failure_does_not_abort_others() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = format!(
            "coordinator:\n  results_dir: {}\nnodes:\n  - name: bad\n    host: unreachable.example.com\n    arch: x86_64\n  - name: local\n    host: localhost\n    arch: x86_64\n",
            dir.path().join("results").display()
        );
        let config_path = write_config(dir.path(), &yaml);
        let parsed = args(config_path, &["rank_select"]);

        let fake = FakeExec::new().fail_connectivity_check("bad");
        // Should not error even though node 'bad' fails its connectivity check.
        run(parsed, &fake).unwrap();

        let calls = fake.calls();
        assert!(calls
            .iter()
            .any(|(node, cmd)| node == "local" && cmd.contains("bench run")));
    }

    #[test]
    fn max_concurrency_is_respected_across_concurrent_benchmarks() {
        // rank_select/balanced_parens/bp_select_micro/elias_fano are all
        // Core-category, registry-default Exclusive — override them all to
        // Concurrent so this test observes real parallelism.
        let dir = tempfile::tempdir().unwrap();
        let yaml = format!(
            "coordinator:\n  results_dir: {}\nnodes:\n  - name: local\n    host: localhost\n    arch: x86_64\n    max_concurrent: 2\nbenchmarks:\n  - name: rank_select\n    isolation: concurrent\n  - name: balanced_parens\n    isolation: concurrent\n  - name: bp_select_micro\n    isolation: concurrent\n  - name: elias_fano\n    isolation: concurrent\n",
            dir.path().join("results").display()
        );
        let config_path = write_config(dir.path(), &yaml);
        let parsed = args(
            config_path,
            &[
                "rank_select",
                "balanced_parens",
                "bp_select_micro",
                "elias_fano",
            ],
        );

        let fake = FakeExec::new();
        run(parsed, &fake).unwrap();

        assert!(fake.max_concurrent_seen() >= 2);
        assert!(fake.max_concurrent_seen() <= 2);
    }

    #[test]
    fn exclusive_benchmarks_never_overlap_on_the_same_node() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = format!(
            "coordinator:\n  results_dir: {}\nnodes:\n  - name: local\n    host: localhost\n    arch: x86_64\n    max_concurrent: 4\n",
            dir.path().join("results").display()
        );
        let config_path = write_config(dir.path(), &yaml);
        // All default to Exclusive in the registry: run several at once and
        // confirm we never observe more than 1 concurrent exec call, aside
        // from the fact that the connectivity check + cleanup are also
        // `exec` calls (but those never overlap benchmark execution either,
        // since they're outside the wave loop).
        let parsed = args(config_path, &["rank_select", "balanced_parens"]);

        let fake = FakeExec::new();
        run(parsed, &fake).unwrap();

        assert_eq!(fake.max_concurrent_seen(), 1);
    }
}
