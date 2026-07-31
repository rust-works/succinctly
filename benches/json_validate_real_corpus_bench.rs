//! Real-workload benchmark for JSON validation (RFC 8259).
//!
//! Split out from `json_validate_bench.rs` (#130 follow-up) because criterion
//! gives a benchmark group no way to see its own `--` filter before running:
//! `Criterion::filter_matches` is private, so `cargo bench --bench
//! json_validate_bench -- validate_real_corpus` still executes every group
//! function in the `criterion_group!`, including the ones that require the
//! synthetic pattern/size ladder under `data/bench/generated/`. A separate
//! binary is what actually makes this leg runnable without it.
//!
//! This is the leg that decides merges: the synthetic ladder in
//! `json_validate_bench` sweeps shapes chosen to stress the parser, not to
//! resemble anything real. Per `docs/guides/benchmarking.md` and the P5
//! precedent, a synthetic-only win is not evidence.
//!
//! Run with:
//! ```bash
//! ./scripts/sync-bench-corpus.sh          # once, populates the full corpus
//! cargo bench --bench json_validate_real_corpus_bench
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use succinctly::json::validate;

/// Real-workload corpus root, populated by `scripts/sync-bench-corpus.sh`.
const CORPUS_DIR: &str = "data/bench/corpus";

/// Committed subset of the real-workload corpus, always present in a checkout.
const CORPUS_SEED_DIR: &str = "tests/data/bench-corpus/seed";

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
///
/// The seed holds only a subset of the corpus's JSON files, so a seed-only run
/// measures a fraction of the intended workload. That is legitimate (it keeps
/// the bench runnable offline) but it must never be mistaken for the full
/// corpus, so which root was used is reported on stderr rather than inferred
/// from the benchmark names.
fn real_corpus_files() -> Vec<(String, Vec<u8>)> {
    let synced = Path::new(CORPUS_DIR).join("json");
    let seeded = !synced.is_dir();
    let root = if seeded {
        Path::new(CORPUS_SEED_DIR).join("json")
    } else {
        synced
    };

    let mut paths = Vec::new();
    collect_json_files(&root, &mut paths);
    paths.sort();

    assert!(
        !paths.is_empty(),
        "json_validate_real_corpus_bench: no real-workload JSON found under {}. \
         The committed seed at {CORPUS_SEED_DIR}/json/ should always be present; \
         run ./scripts/sync-bench-corpus.sh for the full corpus.",
        root.display(),
    );

    let files: Vec<(String, Vec<u8>)> = paths
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
        .collect();

    let total: usize = files.iter().map(|(_, b)| b.len()).sum();
    eprintln!(
        "json_validate_real_corpus_bench: real-workload corpus = {} file(s), {total} bytes, from {}{}",
        files.len(),
        root.display(),
        if seeded {
            " -- COMMITTED SEED ONLY, not the full corpus; \
             run ./scripts/sync-bench-corpus.sh before quoting these numbers"
        } else {
            ""
        },
    );

    files
}

/// Benchmark validation over the real-workload corpus.
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

criterion_group!(benches, bench_validate_real_corpus);
criterion_main!(benches);
