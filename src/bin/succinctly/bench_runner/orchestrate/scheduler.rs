//! Isolation-aware work scheduling (Phase 2, issue #98).
//!
//! Decides, for each selected node, which benchmarks run in which "wave" —
//! a batch of jobs that execute concurrently, bounded by `max_concurrent`.
//! Exclusive benchmarks always get a solo wave. This is pure data
//! transformation: no threads, no I/O, fully unit-testable — `executor.rs`
//! is the only module that turns a [`NodeSchedule`] into real work.

use super::config::{BenchmarkOverride, NodeConfig};
use crate::bench_runner::registry::{BenchmarkInfo, Isolation};

/// A batch of benchmarks that run concurrently on a node (bounded by
/// `max_concurrent`), or a single exclusive benchmark.
pub type Wave<'a> = Vec<&'a BenchmarkInfo>;

/// One node's ordered list of waves, run sequentially wave-by-wave.
#[derive(Debug)]
pub struct NodeSchedule<'a> {
    pub waves: Vec<Wave<'a>>,
}

/// Resolve a benchmark's isolation: an explicit `nodes.yaml` override wins;
/// otherwise fall back to the registry's `default_isolation`.
pub fn resolve_isolation(benchmark: &BenchmarkInfo, overrides: &[BenchmarkOverride]) -> Isolation {
    overrides
        .iter()
        .find(|o| o.name == benchmark.name)
        .and_then(|o| o.isolation)
        .unwrap_or(benchmark.default_isolation)
}

/// Build a wave schedule for `node`, preserving `benchmarks`' relative
/// order: each `Exclusive` benchmark closes out any in-progress concurrent
/// wave and gets its own solo wave; `Concurrent` benchmarks pack into waves
/// of at most `node.max_concurrent` (clamped to at least 1).
pub fn schedule_node<'a>(
    node: &NodeConfig,
    benchmarks: &[&'a BenchmarkInfo],
    overrides: &[BenchmarkOverride],
) -> NodeSchedule<'a> {
    let max_concurrent = node.max_concurrent.max(1);
    let mut waves: Vec<Wave<'a>> = Vec::new();
    let mut current_wave: Wave<'a> = Vec::new();

    for &benchmark in benchmarks {
        match resolve_isolation(benchmark, overrides) {
            Isolation::Exclusive => {
                if !current_wave.is_empty() {
                    waves.push(std::mem::take(&mut current_wave));
                }
                waves.push(vec![benchmark]);
            }
            Isolation::Concurrent => {
                current_wave.push(benchmark);
                if current_wave.len() >= max_concurrent {
                    waves.push(std::mem::take(&mut current_wave));
                }
            }
        }
    }
    if !current_wave.is_empty() {
        waves.push(current_wave);
    }

    NodeSchedule { waves }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench_runner::registry::{BenchmarkCategory, BenchmarkType};

    fn benchmark(name: &'static str, isolation: Isolation) -> BenchmarkInfo {
        BenchmarkInfo {
            name,
            description: "test",
            category: BenchmarkCategory::Core,
            bench_type: BenchmarkType::Criterion,
            criterion_name: Some(name),
            cli_subcommand: None,
            working_dir: ".",
            default_isolation: isolation,
        }
    }

    fn node(max_concurrent: usize) -> NodeConfig {
        NodeConfig {
            name: "n".to_string(),
            host: "localhost".to_string(),
            arch: super::super::config::Architecture::X86_64,
            features: vec![],
            max_concurrent,
            ssh_key: None,
            working_dir: None,
            target_triple: None,
            ec2_instance_id: None,
            ec2_region: None,
        }
    }

    #[test]
    fn resolve_isolation_falls_back_to_registry_default() {
        let b = benchmark("a", Isolation::Exclusive);
        assert_eq!(resolve_isolation(&b, &[]), Isolation::Exclusive);
    }

    #[test]
    fn resolve_isolation_honors_explicit_override() {
        let b = benchmark("a", Isolation::Exclusive);
        let overrides = vec![BenchmarkOverride {
            name: "a".to_string(),
            isolation: Some(Isolation::Concurrent),
        }];
        assert_eq!(resolve_isolation(&b, &overrides), Isolation::Concurrent);
    }

    #[test]
    fn resolve_isolation_ignores_override_with_no_isolation_field() {
        // An override entry present for a different reason, with no
        // `isolation` set, must NOT reset isolation to Concurrent.
        let b = benchmark("a", Isolation::Exclusive);
        let overrides = vec![BenchmarkOverride {
            name: "a".to_string(),
            isolation: None,
        }];
        assert_eq!(resolve_isolation(&b, &overrides), Isolation::Exclusive);
    }

    #[test]
    fn exclusive_benchmarks_each_get_a_solo_wave() {
        let n = node(4);
        let a = benchmark("a", Isolation::Exclusive);
        let b = benchmark("b", Isolation::Exclusive);
        let schedule = schedule_node(&n, &[&a, &b], &[]);

        assert_eq!(schedule.waves.len(), 2);
        assert_eq!(schedule.waves[0], vec![&a]);
        assert_eq!(schedule.waves[1], vec![&b]);
    }

    #[test]
    fn concurrent_benchmarks_pack_up_to_max_concurrent() {
        let n = node(2);
        let a = benchmark("a", Isolation::Concurrent);
        let b = benchmark("b", Isolation::Concurrent);
        let c = benchmark("c", Isolation::Concurrent);
        let schedule = schedule_node(&n, &[&a, &b, &c], &[]);

        assert_eq!(schedule.waves.len(), 2);
        assert_eq!(schedule.waves[0], vec![&a, &b]);
        assert_eq!(schedule.waves[1], vec![&c]);
    }

    #[test]
    fn mixed_exclusive_and_concurrent_never_share_a_wave() {
        let n = node(4);
        let a = benchmark("a", Isolation::Concurrent);
        let b = benchmark("b", Isolation::Exclusive);
        let c = benchmark("c", Isolation::Concurrent);
        let schedule = schedule_node(&n, &[&a, &b, &c], &[]);

        // a's in-progress concurrent wave closes out before b's solo wave;
        // c starts a fresh wave afterward.
        assert_eq!(schedule.waves.len(), 3);
        assert_eq!(schedule.waves[0], vec![&a]);
        assert_eq!(schedule.waves[1], vec![&b]);
        assert_eq!(schedule.waves[2], vec![&c]);

        for wave in &schedule.waves {
            let all_exclusive = wave
                .iter()
                .all(|b| resolve_isolation(b, &[]) == Isolation::Exclusive);
            let all_concurrent = wave
                .iter()
                .all(|b| resolve_isolation(b, &[]) == Isolation::Concurrent);
            assert!(all_exclusive || all_concurrent);
        }
    }

    #[test]
    fn max_concurrent_of_zero_is_clamped_to_one() {
        let n = node(0);
        let a = benchmark("a", Isolation::Concurrent);
        let b = benchmark("b", Isolation::Concurrent);
        let schedule = schedule_node(&n, &[&a, &b], &[]);

        assert_eq!(schedule.waves.len(), 2);
    }
}
