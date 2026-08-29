//! Benchmarks for UTF-8 validation.
//!
//! These benchmarks measure the performance of UTF-8 validation across
//! different content types and sizes.
//!
//! ## Content Types
//!
//! - **ASCII**: Pure 7-bit ASCII content (fastest to validate)
//! - **Mixed UTF-8**: Realistic mix of ASCII and multi-byte characters
//! - **Multi-byte Heavy**: Predominantly 2-4 byte UTF-8 sequences
//! - **CJK Text**: Chinese/Japanese/Korean characters (3-byte sequences)
//! - **Emoji Heavy**: Heavy use of 4-byte sequences (emojis)
//! - **Corpus**: the committed real-workload seed (`tests/data/bench-corpus/seed/`),
//!   whose ASCII-run/multi-byte mix the synthetic generators do not reproduce
//!

// #1670: `clippy.toml`'s `disallowed-methods` bans a bare `Vec`/`String`
// `with_capacity` crate-wide (re-enabled only in `succinctly::jq::eval`/
// `eval_generic`) -- every call site here sizes from a single collection's
// own length (or a length-times-constant) and was never part of that bug
// shape.
#![allow(clippy::disallowed_methods)]

//! ## Sizes
//!
//! Benchmarks run at multiple sizes to show scaling characteristics:
//! - 1KB, 10KB, 100KB, 1MB, 10MB

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
#[cfg(target_arch = "x86_64")]
use succinctly::text::utf8::validate_utf8_simd;
use succinctly::text::utf8::{validate_utf8_broadword, validate_utf8_scalar};

/// Benchmark every validation engine on the same input, plus a `std` baseline.
/// Each becomes an arm under the enclosing group so results compare directly.
///
/// | Arm         | Engine                                                    |
/// |-------------|-----------------------------------------------------------|
/// | `std`       | `core::str::from_utf8` — the bar to clear (see below)      |
/// | `scalar`    | portable byte-at-a-time reference validator               |
/// | `broadword` | broadword ASCII skip + whole-sequence validation          |
/// | `simd`      | AVX2 block kernel (x86_64 only)                           |
///
/// The `std` arm is not decoration. `core::str::from_utf8` already has a
/// two-word ASCII fast path with an alignment prologue and works under
/// `no_std`, so it is the honest bar this optimisation has to clear: if the
/// broadword kernels cannot beat it, the right implementation of
/// `validate_utf8` is a delegation to std rather than a hand-written kernel.
///
/// Caveat: on *invalid* input the comparison is not like-for-like. `std`
/// returns `valid_up_to`/`error_len` from its single scan, while every
/// succinctly engine re-runs `validate_utf8_scalar` to produce line, column
/// and error kind — so `std` looks artificially fast in the
/// `utf8_error_at_end` group. Only the valid-input groups compare directly.
macro_rules! bench_engines {
    ($group:expr, $name:expr, $data:expr) => {{
        let data: &[u8] = $data;
        $group.bench_with_input(BenchmarkId::new("std", $name), data, |b, data| {
            b.iter(|| std::str::from_utf8(black_box(data)).is_ok());
        });
        $group.bench_with_input(BenchmarkId::new("scalar", $name), data, |b, data| {
            b.iter(|| validate_utf8_scalar(black_box(data)));
        });
        $group.bench_with_input(BenchmarkId::new("broadword", $name), data, |b, data| {
            b.iter(|| validate_utf8_broadword(black_box(data)));
        });
        #[cfg(target_arch = "x86_64")]
        $group.bench_with_input(BenchmarkId::new("simd", $name), data, |b, data| {
            b.iter(|| validate_utf8_simd(black_box(data)));
        });
    }};
}

/// Generate pure ASCII content of the specified size.
fn generate_ascii(size: usize) -> Vec<u8> {
    let pattern =
        b"The quick brown fox jumps over the lazy dog. 0123456789!@#$%^&*()_+-=[]{}|;':\",./<>?\n";
    let mut result = Vec::with_capacity(size);
    while result.len() < size {
        let remaining = size - result.len();
        let chunk = &pattern[..remaining.min(pattern.len())];
        result.extend_from_slice(chunk);
    }
    result
}

/// Generate mixed UTF-8 content (ASCII with occasional multi-byte).
/// Approximately 70% ASCII, 20% 2-byte, 8% 3-byte, 2% 4-byte.
fn generate_mixed(size: usize) -> Vec<u8> {
    let pattern = "Hello, world! Café résumé naïve über. 日本語 中文 한국어. Emoji: 🎉🚀💻. More ASCII text here.\n";
    let pattern_bytes = pattern.as_bytes();
    let mut result = Vec::with_capacity(size);
    while result.len() < size {
        let remaining = size - result.len();
        if remaining >= pattern_bytes.len() {
            result.extend_from_slice(pattern_bytes);
        } else {
            // Careful: don't split multi-byte sequences
            // Just pad with ASCII to avoid partial sequences
            result.extend(std::iter::repeat(b'A').take(remaining));
        }
    }
    result.truncate(size);
    result
}

/// Generate predominantly multi-byte content (CJK characters).
fn generate_cjk(size: usize) -> Vec<u8> {
    // Each CJK character is 3 bytes
    let cjk_chars = "日本語中文韓國語漢字假名平仮名片仮名ひらがなカタカナ한글조선어";
    let cjk_bytes = cjk_chars.as_bytes();
    let mut result = Vec::with_capacity(size);
    while result.len() < size {
        let remaining = size - result.len();
        if remaining >= cjk_bytes.len() {
            result.extend_from_slice(cjk_bytes);
        } else {
            // Pad with ASCII to avoid partial sequences
            result.extend(std::iter::repeat(b'X').take(remaining));
        }
    }
    result.truncate(size);
    result
}

/// Generate emoji-heavy content (4-byte sequences).
fn generate_emoji(size: usize) -> Vec<u8> {
    // Each emoji is 4 bytes
    let emojis = "🎉🚀💻🔥🌍😀🎯💡🌟⭐🎨🎭🎪🎢🎡🎠🎰🎲🎳🎯🎱🎾🏀🏈⚽🏐🏉🎿⛷️🏂";
    let emoji_bytes = emojis.as_bytes();
    let mut result = Vec::with_capacity(size);
    while result.len() < size {
        let remaining = size - result.len();
        if remaining >= emoji_bytes.len() {
            result.extend_from_slice(emoji_bytes);
        } else {
            // Pad with ASCII to avoid partial sequences
            result.extend(std::iter::repeat(b'E').take(remaining));
        }
    }
    result.truncate(size);
    result
}

/// Generate 2-byte character content (Latin Extended, Greek, Cyrillic).
fn generate_2byte(size: usize) -> Vec<u8> {
    // 2-byte characters: Latin Extended, Greek, Cyrillic
    let chars =
        "éèêëàâäùûüôöîïçñÉÈÊËÀÂÄÙÛÜÔÖÎÏÇÑαβγδεζηθικλμνξοπρστυφχψωАБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯ";
    let char_bytes = chars.as_bytes();
    let mut result = Vec::with_capacity(size);
    while result.len() < size {
        let remaining = size - result.len();
        if remaining >= char_bytes.len() {
            result.extend_from_slice(char_bytes);
        } else {
            result.extend(std::iter::repeat(b'L').take(remaining));
        }
    }
    result.truncate(size);
    result
}

/// Generate content with ASCII runs of exactly `run` bytes separated by a
/// single 3-byte character.
///
/// The other generators all sit at the extremes — 100% ASCII, or multi-byte
/// every few bytes — so none of them shows where the broadword ASCII skip stops
/// paying for itself. That crossover is the only thing the broadword engines
/// change, and it should land near the 8-byte word: below it the skip rarely
/// fires and the per-sequence handoff dominates, above it the skip should win
/// outright.
fn generate_ascii_runs(size: usize, run: usize) -> Vec<u8> {
    let separator = "€".as_bytes(); // 3-byte sequence
    let mut result = Vec::with_capacity(size + separator.len());
    while result.len() < size {
        for i in 0..run {
            result.push(b'a' + (i % 26) as u8);
        }
        result.extend_from_slice(separator);
    }
    // Truncate on a character boundary so the input stays valid UTF-8.
    while !result.is_empty() && std::str::from_utf8(&result).is_err() {
        result.pop();
    }
    result
}

/// Generate worst-case content: invalid byte at various positions.
/// This tests early-exit behavior.
fn generate_with_error_at_end(size: usize) -> Vec<u8> {
    let mut data = generate_ascii(size);
    if !data.is_empty() {
        // Put invalid byte near the end
        let pos = data.len().saturating_sub(1);
        data[pos] = 0x80; // Invalid lead byte
    }
    data
}

fn bench_ascii(c: &mut Criterion) {
    let mut group = c.benchmark_group("utf8_ascii");

    for size in [1024, 10 * 1024, 100 * 1024, 1024 * 1024, 10 * 1024 * 1024] {
        let data = generate_ascii(size);
        let size_name = format_size(size);

        group.throughput(Throughput::Bytes(size as u64));
        bench_engines!(group, &size_name, &data);
    }

    group.finish();
}

fn bench_mixed(c: &mut Criterion) {
    let mut group = c.benchmark_group("utf8_mixed");

    for size in [1024, 10 * 1024, 100 * 1024, 1024 * 1024, 10 * 1024 * 1024] {
        let data = generate_mixed(size);
        let size_name = format_size(size);

        group.throughput(Throughput::Bytes(size as u64));
        bench_engines!(group, &size_name, &data);
    }

    group.finish();
}

fn bench_cjk(c: &mut Criterion) {
    let mut group = c.benchmark_group("utf8_cjk");

    for size in [1024, 10 * 1024, 100 * 1024, 1024 * 1024, 10 * 1024 * 1024] {
        let data = generate_cjk(size);
        let size_name = format_size(size);

        group.throughput(Throughput::Bytes(size as u64));
        bench_engines!(group, &size_name, &data);
    }

    group.finish();
}

fn bench_emoji(c: &mut Criterion) {
    let mut group = c.benchmark_group("utf8_emoji");

    for size in [1024, 10 * 1024, 100 * 1024, 1024 * 1024, 10 * 1024 * 1024] {
        let data = generate_emoji(size);
        let size_name = format_size(size);

        group.throughput(Throughput::Bytes(size as u64));
        bench_engines!(group, &size_name, &data);
    }

    group.finish();
}

fn bench_2byte(c: &mut Criterion) {
    let mut group = c.benchmark_group("utf8_2byte");

    for size in [1024, 10 * 1024, 100 * 1024, 1024 * 1024, 10 * 1024 * 1024] {
        let data = generate_2byte(size);
        let size_name = format_size(size);

        group.throughput(Throughput::Bytes(size as u64));
        bench_engines!(group, &size_name, &data);
    }

    group.finish();
}

fn bench_error_at_end(c: &mut Criterion) {
    let mut group = c.benchmark_group("utf8_error_at_end");

    for size in [1024, 10 * 1024, 100 * 1024, 1024 * 1024] {
        let data = generate_with_error_at_end(size);
        let size_name = format_size(size);

        group.throughput(Throughput::Bytes(size as u64));
        bench_engines!(group, &size_name, &data);
    }

    group.finish();
}

/// Benchmark comparing different byte sequence lengths at same total size.
fn bench_sequence_types(c: &mut Criterion) {
    let mut group = c.benchmark_group("utf8_sequence_types_1mb");
    let size = 1024 * 1024; // 1MB

    group.throughput(Throughput::Bytes(size as u64));

    // ASCII (1-byte)
    let ascii = generate_ascii(size);
    bench_engines!(group, "ascii_1byte", &ascii);

    // 2-byte sequences
    let twobyte = generate_2byte(size);
    bench_engines!(group, "extended_2byte", &twobyte);

    // 3-byte sequences (CJK)
    let cjk = generate_cjk(size);
    bench_engines!(group, "cjk_3byte", &cjk);

    // 4-byte sequences (emoji)
    let emoji = generate_emoji(size);
    bench_engines!(group, "emoji_4byte", &emoji);

    // Mixed
    let mixed = generate_mixed(size);
    bench_engines!(group, "mixed", &mixed);

    group.finish();
}

/// Sweep ASCII run length to locate the broadword crossover point.
///
/// Fixed 1MB total so only the run length varies. Runs shorter than the 8-byte
/// word should favour the engines that do not pay for a failed skip; longer
/// runs should favour the broadword ones.
fn bench_ascii_runs(c: &mut Criterion) {
    let mut group = c.benchmark_group("utf8_ascii_runs_1mb");
    let size = 1024 * 1024;

    group.throughput(Throughput::Bytes(size as u64));

    for run in [4usize, 8, 16, 64, 256] {
        let data = generate_ascii_runs(size, run);
        bench_engines!(group, &format!("run{run}"), &data);
    }

    group.finish();
}

/// Validate the committed real-workload corpus seed.
///
/// The synthetic generators above are uniform by construction; real text mixes
/// long ASCII runs with sparse multi-byte characters, which is exactly the shape
/// the broadword ASCII fast path is tuned for. Gating throughput claims on this
/// corpus is what #301 asks of its consumer issues (#133 among them).
///
/// Prefers the fully synced corpus at `data/bench/corpus/` (seed **plus** the
/// larger fetched files, per `./scripts/sync-bench-corpus.sh`), falling back to
/// the always-committed `tests/data/bench-corpus/seed/` so this group still runs
/// offline — the seed alone is only 0.5-4.5 KB per file, which measures
/// small-input behaviour rather than steady-state throughput.
///
/// Files are walked in sorted order to keep benchmark IDs stable across runs.
fn bench_corpus(c: &mut Criterion) {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let synced = manifest.join("data").join("bench").join("corpus");
    let seed = manifest
        .join("tests")
        .join("data")
        .join("bench-corpus")
        .join("seed");

    let mut root = synced;
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    if files.is_empty() {
        eprintln!(
            "utf8_corpus: {} is empty; falling back to the committed seed \
             (run ./scripts/sync-bench-corpus.sh for the full corpus)",
            root.display()
        );
        root = seed;
        collect_files(&root, &mut files);
    }
    files.sort();

    if files.is_empty() {
        eprintln!("utf8_corpus: no corpus files found; skipping");
        return;
    }

    let mut group = c.benchmark_group("utf8_corpus");

    for path in &files {
        let Ok(data) = fs::read(path) else { continue };
        if data.is_empty() {
            continue;
        }
        // `<format>/<workload>/<file>` — e.g. `yaml/actions/prometheus-ci.yml`.
        let name = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();

        group.throughput(Throughput::Bytes(data.len() as u64));
        bench_engines!(group, &name, &data);
    }

    group.finish();
}

/// Recursively collect every regular file under `dir` into `out`.
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn format_size(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{}mb", bytes / (1024 * 1024))
    } else if bytes >= 1024 {
        format!("{}kb", bytes / 1024)
    } else {
        format!("{bytes}b")
    }
}

criterion_group!(
    benches,
    bench_ascii,
    bench_mixed,
    bench_cjk,
    bench_emoji,
    bench_2byte,
    bench_error_at_end,
    bench_sequence_types,
    bench_ascii_runs,
    bench_corpus,
);

criterion_main!(benches);
