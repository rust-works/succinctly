//! Benchmark comparing succinctly yq vs system yq for the identity filter.
//!
//! This benchmark measures end-to-end performance of the `.` query on generated
//! sample YAML files, comparing against the system yq command.
//!
//! Run with:
//! ```bash
//! cargo bench --bench yq_comparison
//! ```
//!
//! Prerequisites:
//! - System yq installed (`brew install yq` or equivalent)
//! - Generated benchmark files (`cargo run --release --features cli -- yaml generate-suite`)

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::process::{Command, Stdio};
use std::time::Duration;

// Same source file the `succinctly` binary compiles, rather than a second
// pattern list, so a pattern added to `ALL_PATTERNS` is benchmarked here
// without a second edit (#517).
#[path = "../src/bin/succinctly/yaml_pattern_registry.rs"]
mod yaml_pattern_registry;

/// Every generated suite pattern, alphabetically. Mirrors
/// `yq_bench::pattern_names()` in `src/bin/succinctly/yq_bench.rs`.
fn pattern_names() -> Vec<String> {
    let mut names: Vec<String> = yaml_pattern_registry::ALL_PATTERNS
        .iter()
        .map(|(name, _, _)| (*name).to_string())
        .collect();
    names.sort_unstable();
    names
}

const SIZES: &[&str] = &["1kb", "10kb", "100kb", "1mb"];

/// Patterns deliberately excluded from this bench's size ladder. `config` is
/// `PatternScale::Fixed` — it generates one `config.yaml`, not a `{size}.yaml`
/// per rung — so it never has a file at `file_path("config", size)` for any
/// `SIZES` entry (`dev bench yq` has the same gap).
///
/// Named here rather than left as a bare `!exists()` fallthrough (#517), so
/// `check_skip_list` below can pin it: a *new* `PatternScale::Fixed` pattern
/// that isn't added to this list fails loudly instead of silently vanishing
/// into the same fallthrough.
const SKIP: &[&str] = &["config"];

/// #517 guard: every name in `SKIP` is a real pattern, and every
/// `PatternScale::Fixed` pattern — the only kind with no per-size ladder — is
/// in `SKIP`. Mirrors `check_parity()` in `jq_string_ops_bench.rs`: a
/// `harness = false` bench has no libtest to run `#[test]`s, so this runs at
/// the top of `bench_succinctly_identity` instead, on every `cargo bench` /
/// `cargo test --bench yq_comparison`.
fn check_skip_list() {
    let names = pattern_names();
    for skipped in SKIP {
        assert!(
            names.iter().any(|n| n == skipped),
            "SKIP names {skipped:?}, which is not in ALL_PATTERNS"
        );
    }
    for (name, _, scale) in yaml_pattern_registry::ALL_PATTERNS {
        if *scale == yaml_pattern_registry::PatternScale::Fixed {
            assert!(
                SKIP.contains(name),
                "{name} is PatternScale::Fixed (no {{size}}.yaml ladder) but is not in \
                 SKIP — add it with a reason, or give it a size ladder"
            );
        }
    }
}

// This process-spawns real `yq`/`succinctly` per iteration, so deriving the
// full pattern list (#517, was a hardcoded 5-pattern subset) roughly triples
// benchmark-id count. Trimmed from criterion's defaults (3s warm-up + 5s
// measurement per id) to keep total runtime close to the old 5-pattern total
// — see docs/guides/benchmarking.md#patterns-and-what-they-cover for the
// measured before/after.
const WARM_UP: Duration = Duration::from_secs(1);
const MEASUREMENT: Duration = Duration::from_secs(2);

fn file_path(pattern: &str, size: &str) -> String {
    format!("data/bench/generated/yaml/{pattern}/{size}.yaml")
}

fn get_succinctly_binary() -> Option<std::path::PathBuf> {
    let release = std::path::Path::new("target/release/succinctly");
    if release.exists() {
        return Some(release.to_path_buf());
    }
    let debug = std::path::Path::new("target/debug/succinctly");
    if debug.exists() {
        return Some(debug.to_path_buf());
    }
    None
}

fn has_system_yq() -> bool {
    Command::new("yq")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Benchmark succinctly yq with identity filter
fn bench_succinctly_identity(c: &mut Criterion) {
    check_skip_list();

    let Some(binary) = get_succinctly_binary() else {
        eprintln!("Skipping benchmark: succinctly binary not found. Run `cargo build --release --features cli`");
        return;
    };

    let mut group = c.benchmark_group("succinctly_yq_identity");
    group.warm_up_time(WARM_UP).measurement_time(MEASUREMENT);

    for pattern in pattern_names() {
        for size in SIZES {
            let path = file_path(&pattern, size);
            let path_obj = std::path::Path::new(&path);

            if !path_obj.exists() {
                continue;
            }

            let file_size = path_obj.metadata().map_or(0, |m| m.len());
            group.throughput(Throughput::Bytes(file_size));

            group.bench_with_input(
                BenchmarkId::new(pattern.as_str(), *size),
                &(&binary, &path),
                |b, (binary, path)| {
                    b.iter(|| {
                        let output = Command::new(binary)
                            .args(["yq", "-o=json", "-I=0", ".", path])
                            .stdout(Stdio::piped())
                            .stderr(Stdio::null())
                            .output()
                            .expect("Failed to execute succinctly");
                        assert!(output.status.success(), "succinctly yq failed on {path}");
                        output.stdout
                    });
                },
            );
        }
    }

    group.finish();
}

/// Benchmark system yq with identity filter
fn bench_system_yq_identity(c: &mut Criterion) {
    if !has_system_yq() {
        eprintln!("Skipping benchmark: system yq not found");
        return;
    }

    let mut group = c.benchmark_group("system_yq_identity");
    group.warm_up_time(WARM_UP).measurement_time(MEASUREMENT);

    for pattern in pattern_names() {
        for size in SIZES {
            let path = file_path(&pattern, size);
            let path_obj = std::path::Path::new(&path);

            if !path_obj.exists() {
                continue;
            }

            let file_size = path_obj.metadata().map_or(0, |m| m.len());
            group.throughput(Throughput::Bytes(file_size));

            group.bench_with_input(
                BenchmarkId::new(pattern.as_str(), *size),
                &path,
                |b, path| {
                    b.iter(|| {
                        let output = Command::new("yq")
                            .args(["-o=json", "-I=0", ".", path])
                            .stdout(Stdio::piped())
                            .stderr(Stdio::null())
                            .output()
                            .expect("Failed to execute yq");
                        assert!(output.status.success(), "system yq failed on {path}");
                        output.stdout
                    });
                },
            );
        }
    }

    group.finish();
}

/// Side-by-side comparison benchmark
fn bench_yq_comparison(c: &mut Criterion) {
    let succinctly_binary = get_succinctly_binary();
    let has_yq = has_system_yq();

    if succinctly_binary.is_none() && !has_yq {
        eprintln!("Skipping comparison: neither succinctly nor system yq available");
        return;
    }

    let mut group = c.benchmark_group("yq_identity_comparison");

    // Use a subset for the comparison to keep it focused
    let comparison_sizes = &["10kb", "100kb", "1mb"];

    for size in comparison_sizes {
        let path = file_path("comprehensive", size);
        let path_obj = std::path::Path::new(&path);

        if !path_obj.exists() {
            continue;
        }

        let file_size = path_obj.metadata().map_or(0, |m| m.len());
        group.throughput(Throughput::Bytes(file_size));

        if let Some(ref binary) = succinctly_binary {
            group.bench_with_input(
                BenchmarkId::new("succinctly", *size),
                &(binary, &path),
                |b, (binary, path)| {
                    b.iter(|| {
                        let output = Command::new(binary)
                            .args(["yq", "-o=json", "-I=0", ".", path])
                            .stdout(Stdio::piped())
                            .stderr(Stdio::null())
                            .output()
                            .expect("Failed to execute succinctly");
                        assert!(output.status.success());
                        output.stdout
                    });
                },
            );
        }

        if has_yq {
            group.bench_with_input(BenchmarkId::new("yq", *size), &path, |b, path| {
                b.iter(|| {
                    let output = Command::new("yq")
                        .args(["-o=json", "-I=0", ".", path])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .output()
                        .expect("Failed to execute yq");
                    assert!(output.status.success());
                    output.stdout
                });
            });
        }
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_succinctly_identity,
    bench_system_yq_identity,
    bench_yq_comparison,
);
criterion_main!(benches);
