//! Benchmarks for JSON validation (RFC 8259).
//!
//! Two source groups, both mandatory:
//!
//! * **Synthetic** — the generated pattern/size ladder under
//!   `data/bench/generated/`, which sweeps shapes (structural density, string
//!   length, whitespace) far wider than any real file does.
//! * **Real workload** — the `json/` tier of the corpus described by
//!   `tests/data/bench-corpus/manifest.json`. Synthetic patterns are chosen to
//!   stress the parser, not to resemble anything; per
//!   `docs/guides/benchmarking.md` a synthetic-only win is not evidence.
//!
//! Run with:
//! ```bash
//! succinctly json generate-suite --max-size 10mb   # once, populates the ladder
//! cargo bench --bench json_validate_bench
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use succinctly::json::validate;

/// Test file patterns available in data/bench/generated/
const PATTERNS: &[&str] = &[
    "arrays",
    "comprehensive",
    "literals",
    "mixed",
    "nested",
    "numbers",
    "pathological",
    "pretty",
    "strings",
    "unicode",
    "users",
];

/// Test file sizes
const SIZES: &[&str] = &["1kb", "10kb", "100kb", "1mb", "10mb"];

/// Base directory for generated files
const BASE_DIR: &str = "data/bench/generated";

/// Real-workload corpus root, populated by `scripts/sync-bench-corpus.sh`.
const CORPUS_DIR: &str = "data/bench/corpus";

/// Committed subset of the real-workload corpus, always present in a checkout.
const CORPUS_SEED_DIR: &str = "tests/data/bench-corpus/seed";

/// The command that populates [`BASE_DIR`].
const GENERATE_CMD: &str =
    "cargo run --release --features cli --bin succinctly -- json generate-suite --max-size 10mb";

fn generated_path(pattern: &str, size: &str) -> PathBuf {
    PathBuf::from(format!("{BASE_DIR}/{pattern}/{size}.json"))
}

/// Panic unless the whole pattern/size matrix is present.
///
/// This used to `continue` past missing files, which meant a fresh checkout
/// (where `BASE_DIR` is gitignored and absent) produced **zero** benchmarks and
/// still exited 0 — a green run that measured nothing. A benchmark that cannot
/// measure must fail, not report success.
fn require_generated_corpus() {
    let missing: Vec<String> = PATTERNS
        .iter()
        .flat_map(|p| SIZES.iter().map(move |s| (p, s)))
        .filter(|(p, s)| !generated_path(p, s).exists())
        .map(|(p, s)| format!("{p}/{s}.json"))
        .collect();

    assert!(
        missing.is_empty(),
        "json_validate_bench: {} of {} generated corpus files are missing from {BASE_DIR}/ \
         (first few: {}).\nGenerate them with:\n  {GENERATE_CMD}",
        missing.len(),
        PATTERNS.len() * SIZES.len(),
        missing
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
    );
}

/// Load a generated test file. Call [`require_generated_corpus`] first.
fn load_file(pattern: &str, size: &str) -> Vec<u8> {
    let path = generated_path(pattern, size);
    fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read {}: {e}\nGenerate with:\n  {GENERATE_CMD}",
            path.display()
        )
    })
}

/// Collect `*.json` / `*.geojson` files under `dir`, recursively, sorted.
fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("json" | "geojson")
        ) {
            out.push(path);
        }
    }
}

/// The real-workload JSON corpus: the synced tree if present, else the
/// committed seed. The seed is always in a checkout, so this never returns
/// empty in a well-formed tree — and asserts rather than skipping if it does.
fn real_corpus_files() -> Vec<(String, Vec<u8>)> {
    let synced = Path::new(CORPUS_DIR).join("json");
    let root = if synced.is_dir() {
        synced
    } else {
        Path::new(CORPUS_SEED_DIR).join("json")
    };

    let mut paths = Vec::new();
    collect_json_files(&root, &mut paths);
    paths.sort();

    assert!(
        !paths.is_empty(),
        "json_validate_bench: no real-workload JSON found under {}. \
         The committed seed at {CORPUS_SEED_DIR}/json/ should always be present; \
         run ./scripts/sync-bench-corpus.sh for the full corpus.",
        root.display(),
    );

    paths
        .into_iter()
        .map(|p| {
            let name = p
                .strip_prefix(&root)
                .unwrap_or(&p)
                .to_string_lossy()
                .into_owned();
            let bytes =
                fs::read(&p).unwrap_or_else(|e| panic!("failed to read {}: {e}", p.display()));
            (name, bytes)
        })
        .collect()
}

/// Benchmark validation across all patterns for a given size
fn bench_validate_by_size(c: &mut Criterion) {
    require_generated_corpus();

    // Group benchmarks by size
    for size in SIZES {
        let mut group = c.benchmark_group(format!("validate_{size}"));

        for pattern in PATTERNS {
            let bytes = load_file(pattern, size);

            group.throughput(Throughput::Bytes(bytes.len() as u64));

            group.bench_with_input(BenchmarkId::from_parameter(pattern), &bytes, |b, bytes| {
                b.iter(|| {
                    let result = validate::validate(black_box(bytes));
                    black_box(result)
                });
            });
        }

        group.finish();
    }
}

/// Benchmark validation across all sizes for a given pattern
fn bench_validate_by_pattern(c: &mut Criterion) {
    require_generated_corpus();

    // Focus on comprehensive pattern as it tests all JSON features
    let pattern = "comprehensive";
    let mut group = c.benchmark_group(format!("validate_{pattern}"));

    for size in SIZES {
        let bytes = load_file(pattern, size);

        group.throughput(Throughput::Bytes(bytes.len() as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size), &bytes, |b, bytes| {
            b.iter(|| {
                let result = validate::validate(black_box(bytes));
                black_box(result)
            });
        });
    }

    group.finish();
}

/// Benchmark validation of the largest files (10mb) to measure sustained throughput
fn bench_validate_large_files(c: &mut Criterion) {
    require_generated_corpus();

    let mut group = c.benchmark_group("validate_10mb");
    group.sample_size(10); // Fewer samples for large files

    for pattern in PATTERNS {
        let bytes = load_file(pattern, "10mb");

        group.throughput(Throughput::Bytes(bytes.len() as u64));

        group.bench_with_input(BenchmarkId::from_parameter(pattern), &bytes, |b, bytes| {
            b.iter(|| {
                let result = validate::validate(black_box(bytes));
                black_box(result)
            });
        });
    }

    group.finish();
}

/// Verify all generated files pass validation (not a benchmark, but useful for testing)
fn verify_all_files_valid(c: &mut Criterion) {
    let mut group = c.benchmark_group("validate_verify_all");
    group.sample_size(10);

    require_generated_corpus();

    // Collect all valid files
    let mut all_files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total_bytes = 0u64;

    for pattern in PATTERNS {
        for size in SIZES {
            let bytes = load_file(pattern, size);
            total_bytes += bytes.len() as u64;
            all_files.push((format!("{pattern}/{size}"), bytes));
        }
    }

    group.throughput(Throughput::Bytes(total_bytes));

    group.bench_function("all_files", |b| {
        b.iter(|| {
            let mut valid_count = 0;
            for (name, bytes) in &all_files {
                match validate::validate(black_box(bytes)) {
                    Ok(()) => valid_count += 1,
                    Err(e) => panic!("Validation failed for {name}: {e:?}"),
                }
            }
            valid_count
        });
    });

    group.finish();
}

/// Benchmark validation over the real-workload corpus.
///
/// This is the leg that decides merges. The synthetic patterns above sweep
/// shapes chosen to stress the parser; this one measures files that actually
/// occur. Per `docs/guides/benchmarking.md` and the P5 precedent, a synthetic
/// win that does not appear here is not a win.
fn bench_validate_real_corpus(c: &mut Criterion) {
    let files = real_corpus_files();

    let mut group = c.benchmark_group("validate_real_corpus");

    for (name, bytes) in &files {
        // Fail loudly rather than benchmark a document we would reject: a
        // corpus file that does not validate means the corpus or the validator
        // is broken, and timing it would measure the error path.
        if let Err(e) = validate::validate(bytes) {
            panic!("real-workload corpus file {name} failed validation: {e}");
        }

        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), bytes, |b, bytes| {
            b.iter(|| {
                let result = validate::validate(black_box(bytes.as_slice()));
                black_box(result)
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_validate_by_size,
    bench_validate_by_pattern,
    bench_validate_large_files,
    bench_validate_real_corpus,
    verify_all_files_valid,
);
criterion_main!(benches);
