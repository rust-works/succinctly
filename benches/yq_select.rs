//! Benchmark comparing succinctly yq vs system yq for partial selection queries.
//!
//! This benchmark measures end-to-end performance when selecting ~5% of the input,
//! demonstrating the lazy evaluation benefits of the semi-index architecture.
//!
//! Run with:
//! ```bash
//! cargo bench --bench yq_select
//! ```
//!
//! Prerequisites:
//! - System yq installed (`brew install yq` or equivalent)
//! - Generated benchmark files (`cargo run --release --features cli -- yaml generate-suite`)

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::process::{Command, Stdio};

/// Selection queries that extract ~5% of the input.
/// Each tuple: (pattern, size, succinctly_query, yq_query, description)
/// Note: yq uses `.[0:N]` syntax while succinctly supports `[:N]`
const SELECT_QUERIES: &[(&str, &str, &str, &str, &str)] = &[
    // users pattern: select first ~5% of user records
    (
        "users",
        "10kb",
        ".users[:4]",
        ".users | .[0:4]",
        "4 of 75 users",
    ),
    (
        "users",
        "100kb",
        ".users[:37]",
        ".users | .[0:37]",
        "37 of 742 users",
    ),
    (
        "users",
        "1mb",
        ".users[:377]",
        ".users | .[0:377]",
        "377 of 7540 users",
    ),
    // sequences pattern: select first ~5% of items
    (
        "sequences",
        "10kb",
        ".sequences[:50]",
        ".sequences | .[0:50]",
        "first 50 items",
    ),
    (
        "sequences",
        "100kb",
        ".sequences[:500]",
        ".sequences | .[0:500]",
        "first 500 items",
    ),
    (
        "sequences",
        "1mb",
        ".sequences[:5000]",
        ".sequences | .[0:5000]",
        "first 5000 items",
    ),
];

/// Single field extraction (minimal output, tests navigation speed)
const FIELD_QUERIES: &[(&str, &str, &str, &str)] = &[
    // Extract just one user's name (near start)
    ("users", "10kb", ".users[0].name", "first user name"),
    ("users", "100kb", ".users[0].name", "first user name"),
    ("users", "1mb", ".users[0].name", "first user name"),
    // Extract user near middle of array
    ("users", "10kb", ".users[37]", "user at 50%"),
    ("users", "100kb", ".users[371]", "user at 50%"),
    ("users", "1mb", ".users[3770]", "user at 50%"),
    // Extract user near end of array (tests skip efficiency)
    ("users", "10kb", ".users[70]", "user at 93%"),
    ("users", "100kb", ".users[700]", "user at 94%"),
    ("users", "1mb", ".users[7000]", "user at 93%"),
];

fn file_path(pattern: &str, size: &str) -> String {
    format!("data/bench/generated/yaml/{}/{}.yaml", pattern, size)
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
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Benchmark succinctly yq with ~5% selection queries
fn bench_succinctly_select(c: &mut Criterion) {
    let Some(binary) = get_succinctly_binary() else {
        eprintln!("Skipping benchmark: succinctly binary not found. Run `cargo build --release --features cli`");
        return;
    };

    let mut group = c.benchmark_group("succinctly_yq_select_5pct");

    for (pattern, size, succinctly_query, _yq_query, desc) in SELECT_QUERIES {
        let path = file_path(pattern, size);
        let path_obj = std::path::Path::new(&path);

        if !path_obj.exists() {
            eprintln!("Skipping {}/{}: file not found", pattern, size);
            continue;
        }

        let file_size = path_obj.metadata().map(|m| m.len()).unwrap_or(0);
        group.throughput(Throughput::Bytes(file_size));

        let label = format!("{}/{} ({})", pattern, size, desc);
        group.bench_with_input(
            BenchmarkId::from_parameter(&label),
            &(&binary, &path, succinctly_query),
            |b, (binary, path, query)| {
                b.iter(|| {
                    let output = Command::new(binary)
                        .args(["yq", "-o", "json", "-I", "0", query, path])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .output()
                        .expect("Failed to execute succinctly");
                    assert!(
                        output.status.success(),
                        "succinctly yq failed on {} with query {}",
                        path,
                        query
                    );
                    output.stdout
                })
            },
        );
    }

    group.finish();
}

/// Benchmark system yq with ~5% selection queries
fn bench_system_yq_select(c: &mut Criterion) {
    if !has_system_yq() {
        eprintln!("Skipping benchmark: system yq not found");
        return;
    }

    let mut group = c.benchmark_group("system_yq_select_5pct");

    for (pattern, size, _succinctly_query, yq_query, desc) in SELECT_QUERIES {
        let path = file_path(pattern, size);
        let path_obj = std::path::Path::new(&path);

        if !path_obj.exists() {
            continue;
        }

        let file_size = path_obj.metadata().map(|m| m.len()).unwrap_or(0);
        group.throughput(Throughput::Bytes(file_size));

        let label = format!("{}/{} ({})", pattern, size, desc);
        group.bench_with_input(
            BenchmarkId::from_parameter(&label),
            &(&path, yq_query),
            |b, (path, query)| {
                b.iter(|| {
                    let output = Command::new("yq")
                        .args(["-o=json", "-I=0", query, path])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .output()
                        .expect("Failed to execute yq");
                    assert!(output.status.success(), "system yq failed on {}", path);
                    output.stdout
                })
            },
        );
    }

    group.finish();
}

/// Benchmark succinctly yq with single field extraction
fn bench_succinctly_field(c: &mut Criterion) {
    let Some(binary) = get_succinctly_binary() else {
        eprintln!("Skipping benchmark: succinctly binary not found");
        return;
    };

    let mut group = c.benchmark_group("succinctly_yq_field");

    for (pattern, size, query, desc) in FIELD_QUERIES {
        let path = file_path(pattern, size);
        let path_obj = std::path::Path::new(&path);

        if !path_obj.exists() {
            continue;
        }

        let file_size = path_obj.metadata().map(|m| m.len()).unwrap_or(0);
        group.throughput(Throughput::Bytes(file_size));

        let label = format!("{}/{} ({})", pattern, size, desc);
        group.bench_with_input(
            BenchmarkId::from_parameter(&label),
            &(&binary, &path, query),
            |b, (binary, path, query)| {
                b.iter(|| {
                    let output = Command::new(binary)
                        .args(["yq", "-o", "json", "-I", "0", query, path])
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

    group.finish();
}

/// Benchmark system yq with single field extraction
fn bench_system_yq_field(c: &mut Criterion) {
    if !has_system_yq() {
        eprintln!("Skipping benchmark: system yq not found");
        return;
    }

    let mut group = c.benchmark_group("system_yq_field");

    for (pattern, size, query, desc) in FIELD_QUERIES {
        let path = file_path(pattern, size);
        let path_obj = std::path::Path::new(&path);

        if !path_obj.exists() {
            continue;
        }

        let file_size = path_obj.metadata().map(|m| m.len()).unwrap_or(0);
        group.throughput(Throughput::Bytes(file_size));

        let label = format!("{}/{} ({})", pattern, size, desc);
        group.bench_with_input(
            BenchmarkId::from_parameter(&label),
            &(&path, query),
            |b, (path, query)| {
                b.iter(|| {
                    let output = Command::new("yq")
                        .args(["-o=json", "-I=0", query, path])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .output()
                        .expect("Failed to execute yq");
                    assert!(output.status.success());
                    output.stdout
                })
            },
        );
    }

    group.finish();
}

/// Side-by-side comparison for ~5% selection
fn bench_select_comparison(c: &mut Criterion) {
    let succinctly_binary = get_succinctly_binary();
    let has_yq = has_system_yq();

    if succinctly_binary.is_none() && !has_yq {
        eprintln!("Skipping comparison: neither succinctly nor system yq available");
        return;
    }

    let mut group = c.benchmark_group("yq_select_5pct_comparison");

    // Focus on 1MB files where lazy evaluation should shine
    // (pattern, size, succinctly_query, yq_query, desc)
    let comparison_queries: &[(&str, &str, &str, &str, &str)] = &[
        (
            "users",
            "1mb",
            ".users[:377]",
            ".users | .[0:377]",
            "5% slice",
        ),
        (
            "users",
            "1mb",
            ".users[0].name",
            ".users[0].name",
            "first field",
        ),
        (
            "users",
            "1mb",
            ".users[7000]",
            ".users[7000]",
            "near-end access",
        ),
    ];

    for (pattern, size, succinctly_query, yq_query, desc) in comparison_queries {
        let path = file_path(pattern, size);
        let path_obj = std::path::Path::new(&path);

        if !path_obj.exists() {
            continue;
        }

        let file_size = path_obj.metadata().map(|m| m.len()).unwrap_or(0);
        group.throughput(Throughput::Bytes(file_size));

        if let Some(ref binary) = succinctly_binary {
            let label = format!("succinctly/{}", desc);
            group.bench_with_input(
                BenchmarkId::from_parameter(&label),
                &(binary, &path, succinctly_query),
                |b, (binary, path, query)| {
                    b.iter(|| {
                        let output = Command::new(binary)
                            .args(["yq", "-o", "json", "-I", "0", query, path])
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

        if has_yq {
            let label = format!("yq/{}", desc);
            group.bench_with_input(
                BenchmarkId::from_parameter(&label),
                &(&path, yq_query),
                |b, (path, query)| {
                    b.iter(|| {
                        let output = Command::new("yq")
                            .args(["-o=json", "-I=0", query, path])
                            .stdout(Stdio::piped())
                            .stderr(Stdio::null())
                            .output()
                            .expect("Failed to execute yq");
                        assert!(output.status.success());
                        output.stdout
                    })
                },
            );
        }
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_succinctly_select,
    bench_system_yq_select,
    bench_succinctly_field,
    bench_system_yq_field,
    bench_select_comparison,
);
criterion_main!(benches);
