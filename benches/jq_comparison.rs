//! Benchmark comparing succinctly jq vs system jq for the identity filter.
//!
//! This benchmark measures end-to-end performance of the `.` query on generated
//! sample JSON files, comparing against the system jq command.
//!
//! Run with:
//! ```bash
//! cargo bench --bench jq_comparison
//! ```
//!
//! Prerequisites:
//! - System jq installed (`brew install jq` or equivalent)
//! - Generated benchmark files (`cargo run --release --features cli -- json generate-suite`)

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::process::{Command, Stdio};

const PATTERNS: &[&str] = &["comprehensive", "users", "nested", "arrays", "strings"];
const SIZES: &[&str] = &["1kb", "10kb", "100kb", "1mb"];

fn file_path(pattern: &str, size: &str) -> String {
    format!("data/bench/generated/{}/{}.json", pattern, size)
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

fn has_system_jq() -> bool {
    Command::new("jq")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Benchmark succinctly jq with identity filter
fn bench_succinctly_identity(c: &mut Criterion) {
    let Some(binary) = get_succinctly_binary() else {
        eprintln!("Skipping benchmark: succinctly binary not found. Run `cargo build --release --features cli`");
        return;
    };

    let mut group = c.benchmark_group("succinctly_jq_identity");

    for pattern in PATTERNS {
        for size in SIZES {
            let path = file_path(pattern, size);
            let path_obj = std::path::Path::new(&path);

            if !path_obj.exists() {
                continue;
            }

            let file_size = path_obj.metadata().map(|m| m.len()).unwrap_or(0);
            group.throughput(Throughput::Bytes(file_size));

            group.bench_with_input(
                BenchmarkId::new(*pattern, *size),
                &(&binary, &path),
                |b, (binary, path)| {
                    b.iter(|| {
                        let output = Command::new(binary)
                            .args(["jq", "-c", ".", path])
                            .stdout(Stdio::piped())
                            .stderr(Stdio::null())
                            .output()
                            .expect("Failed to execute succinctly");
                        assert!(output.status.success());
                        output.stdout
                    })
                },
            );
        }
    }

    group.finish();
}

/// Benchmark system jq with identity filter
fn bench_system_jq_identity(c: &mut Criterion) {
    if !has_system_jq() {
        eprintln!("Skipping benchmark: system jq not found");
        return;
    }

    let mut group = c.benchmark_group("system_jq_identity");

    for pattern in PATTERNS {
        for size in SIZES {
            let path = file_path(pattern, size);
            let path_obj = std::path::Path::new(&path);

            if !path_obj.exists() {
                continue;
            }

            let file_size = path_obj.metadata().map(|m| m.len()).unwrap_or(0);
            group.throughput(Throughput::Bytes(file_size));

            group.bench_with_input(BenchmarkId::new(*pattern, *size), &path, |b, path| {
                b.iter(|| {
                    let output = Command::new("jq")
                        .args(["-c", ".", path])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .output()
                        .expect("Failed to execute jq");
                    assert!(output.status.success());
                    output.stdout
                })
            });
        }
    }

    group.finish();
}

/// Side-by-side comparison benchmark
fn bench_jq_comparison(c: &mut Criterion) {
    let succinctly_binary = get_succinctly_binary();
    let has_jq = has_system_jq();

    if succinctly_binary.is_none() && !has_jq {
        eprintln!("Skipping comparison: neither succinctly nor system jq available");
        return;
    }

    let mut group = c.benchmark_group("jq_identity_comparison");

    // Use a subset for the comparison to keep it focused
    let comparison_sizes = &["10kb", "100kb", "1mb"];

    for size in comparison_sizes {
        let path = file_path("comprehensive", size);
        let path_obj = std::path::Path::new(&path);

        if !path_obj.exists() {
            continue;
        }

        let file_size = path_obj.metadata().map(|m| m.len()).unwrap_or(0);
        group.throughput(Throughput::Bytes(file_size));

        if let Some(ref binary) = succinctly_binary {
            group.bench_with_input(
                BenchmarkId::new("succinctly", *size),
                &(binary, &path),
                |b, (binary, path)| {
                    b.iter(|| {
                        let output = Command::new(binary)
                            .args(["jq", "-c", ".", path])
                            .stdout(Stdio::piped())
                            .stderr(Stdio::null())
                            .output()
                            .expect("Failed to execute succinctly");
                        assert!(output.status.success());
                        output.stdout
                    })
                },
            );
        }

        if has_jq {
            group.bench_with_input(BenchmarkId::new("jq", *size), &path, |b, path| {
                b.iter(|| {
                    let output = Command::new("jq")
                        .args(["-c", ".", path])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .output()
                        .expect("Failed to execute jq");
                    assert!(output.status.success());
                    output.stdout
                })
            });
        }
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_succinctly_identity,
    bench_system_jq_identity,
    bench_jq_comparison,
);
criterion_main!(benches);
