//! A/B micro-benchmarks: `memchr::memmem` vs Rust std substring search for the
//! jq substring builtins — `index` / `rindex` / `indices` / `contains` / `split`
//! (issue #303, follow-up to #126's O6 recon).
//!
//! # Why this file exists
//!
//! Rust's std substring search (`str::find` / `rfind` / `contains` / `split(&str)`)
//! is the **scalar** Two-Way algorithm in `core::str::pattern`; `memchr::memmem`
//! is **genuinely SIMD**. #126 found one narrow cell with real headroom — long
//! haystacks with a rare needle — and #303 scopes an isolated measurement of it.
//!
//! # Bench-only — this is the whole of Phase 1
//!
//! Both implementations under test are defined **in this file**. `src/jq/eval.rs`
//! is deliberately **not** touched (hard non-goal of #303). The std variants here
//! mirror the exact call shapes in `eval.rs` so the comparison is honest:
//!
//! | Op        | eval.rs site        | primitive mirrored here                    |
//! |-----------|---------------------|--------------------------------------------|
//! | `index`   | `builtin_index`     | `cow.find(pattern_str.as_str())`           |
//! | `rindex`  | `builtin_rindex`    | `cow.rfind(pattern_str.as_str())`          |
//! | `indices` | `builtin_indices`   | overlapping `find` loop, `start += pos + 1`|
//! | `contains`| `owned_contains`    | `a_str.contains(b_str.as_str())`           |
//! | `split`   | `builtin_split`     | `cow.split(&sep).map(to_string).collect()` |
//!
//! Note `eval.rs` always searches with a **`&str`** needle (never a `char`), so
//! even a 1-byte needle takes std's scalar Two-Way path — not the memchr
//! fast-path that `str::find(char)` would get. The 1-byte column below is
//! therefore a real comparison, not a straw man.
//!
//! # What a green result here does *not* mean
//!
//! A long-haystack micro-bench will almost certainly show memmem winning — that
//! is the regime it is built for. This codebase has been burned seven times by
//! exactly that (P2.6, P2.8, P3, P5, P8, all rejected after a good micro-bench).
//! Adoption is gated **end-to-end** on a realistic large-single-string workload
//! (#301), which does not yet exist. See `docs/optimizations/jq-string-search.md`.

use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use memchr::memmem;

// ---------------------------------------------------------------------------
// Implementations under test — std (scalar Two-Way) vs memchr::memmem (SIMD).
// Each pair returns identical output on the ASCII corpus below (see `mod tests`).
// ---------------------------------------------------------------------------

fn index_std(hay: &str, needle: &str) -> Option<usize> {
    hay.find(needle)
}

fn index_memmem(hay: &str, needle: &str) -> Option<usize> {
    memmem::find(hay.as_bytes(), needle.as_bytes())
}

fn rindex_std(hay: &str, needle: &str) -> Option<usize> {
    hay.rfind(needle)
}

fn rindex_memmem(hay: &str, needle: &str) -> Option<usize> {
    memmem::rfind(hay.as_bytes(), needle.as_bytes())
}

fn contains_std(hay: &str, needle: &str) -> bool {
    hay.contains(needle)
}

fn contains_memmem(hay: &str, needle: &str) -> bool {
    memmem::find(hay.as_bytes(), needle.as_bytes()).is_some()
}

/// `indices` finds **overlapping** occurrences (`eval.rs` advances `start += pos + 1`),
/// so `indices("aaaa", "aa") == [0, 1, 2]`. Both variants keep that manual loop —
/// `memmem::find_iter` is **non-overlapping** and would silently change the output.
fn indices_std(hay: &str, needle: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(pos) = hay[start..].find(needle) {
        out.push(start + pos);
        start += pos + 1;
        if start >= hay.len() {
            break;
        }
    }
    out
}

fn indices_memmem(hay: &str, needle: &str) -> Vec<usize> {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    // Fresh searcher each iteration, mirroring `str::find`'s per-call Two-Way
    // construction — this keeps the per-call setup cost (caveat 4) in the picture.
    while let Some(pos) = memmem::find(&h[start..], n) {
        out.push(start + pos);
        start += pos + 1;
        if start >= h.len() {
            break;
        }
    }
    out
}

/// std `split(&str)` for a non-empty separator: allocates a `String` per part.
fn split_std(hay: &str, sep: &str) -> Vec<String> {
    hay.split(sep).map(str::to_string).collect()
}

/// Hand-rolled memmem split that reproduces `str::split(&str)` **exactly** for a
/// non-empty separator (caveat 3): a trailing separator yields a trailing `""`
/// (`"a,b," -> ["a","b",""]`) and adjacent separators yield an interior `""`
/// (`"a,,b" -> ["a","","b"]`). Empty separator is special-cased per-char upstream
/// and is intentionally **not** routed through memmem.
fn split_memmem(hay: &str, sep: &str) -> Vec<String> {
    debug_assert!(
        !sep.is_empty(),
        "empty separator must stay on the per-char path"
    );
    let n = sep.as_bytes();
    // One searcher reused across the whole string — the faithful analog of a
    // single jq `split` call (std builds its searcher once per call too).
    let finder = memmem::Finder::new(n);
    let bytes = hay.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(pos) = finder.find(&bytes[start..]) {
        let end = start + pos;
        out.push(hay[start..end].to_string());
        start = end + n.len();
    }
    out.push(hay[start..].to_string());
    out
}

// ---------------------------------------------------------------------------
// Corpus generation. Filler is lowercase letters / digits / space; every needle
// and separator is uppercase-or-punctuation, so a needle never appears in the
// filler by accident and match positions are exact.
// ---------------------------------------------------------------------------

const FILLER_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789 ";

fn filler_bytes(size: usize) -> Vec<u8> {
    (0..size)
        .map(|i| FILLER_ALPHABET[i % FILLER_ALPHABET.len()])
        .collect()
}

fn filler(size: usize) -> String {
    String::from_utf8(filler_bytes(size)).expect("ascii filler is valid utf-8")
}

/// A needle of `len` uppercase bytes — none of which occur in `FILLER_ALPHABET`,
/// so it is guaranteed absent from any `filler(_)` haystack.
fn needle(len: usize) -> String {
    const POOL: &[u8] = b"MARKERWXYZBCDFGHJLNPQSTVU"; // distinct uppercase, disjoint from filler
    let bytes: Vec<u8> = (0..len).map(|i| POOL[i % POOL.len()]).collect();
    String::from_utf8(bytes).expect("ascii needle is valid utf-8")
}

#[derive(Clone, Copy)]
enum Pos {
    Start,
    Middle,
    End,
}

/// `filler(size)` with `needle` overwritten in at the requested position, so
/// exactly one match exists at a known offset.
fn haystack_with_needle(size: usize, needle: &str, pos: Pos) -> String {
    let nb = needle.as_bytes();
    assert!(size >= nb.len(), "haystack too small for needle");
    let mut bytes = filler_bytes(size);
    let at = match pos {
        Pos::Start => 0,
        Pos::Middle => (size - nb.len()) / 2,
        Pos::End => size - nb.len(),
    };
    bytes[at..at + nb.len()].copy_from_slice(nb);
    String::from_utf8(bytes).expect("ascii haystack is valid utf-8")
}

/// `filler(size)` with `sep` inserted roughly every `gap` bytes (separator
/// frequency knob for `split`). The returned length is used as the throughput.
fn haystack_with_sep(size: usize, sep: &str, gap: usize) -> String {
    filler(size)
        .as_bytes()
        .chunks(gap)
        .map(|c| std::str::from_utf8(c).expect("ascii chunk"))
        .collect::<Vec<_>>()
        .join(sep)
}

/// Haystack length sweep: short scalar fields (the dominant jq case) through
/// long single strings (the only regime with SIMD headroom).
const SIZES: [usize; 9] = [8, 16, 32, 64, 256, 1024, 4096, 16384, 65536];

// ---------------------------------------------------------------------------
// index — the primary op. Two regimes: needle ABSENT (full scan, memmem-favorable)
// and needle at START (early exit, so per-call searcher setup dominates: caveat 4).
// ---------------------------------------------------------------------------

/// Needle absent: every call scans the whole haystack — the regime memmem is
/// built for, and the one where a crossover (if any) should appear.
fn bench_index_absent(c: &mut Criterion) {
    // Verify std and memmem agree before timing anything (see `check_parity`).
    check_parity();

    let n = needle(4);
    let mut scan = c.benchmark_group("jq_index/absent");
    for size in SIZES {
        let hay = filler(size); // needle guaranteed absent -> full scan every time
        scan.throughput(Throughput::Bytes(size as u64));
        scan.bench_with_input(BenchmarkId::new("std", size), &hay, |b, hay| {
            b.iter(|| index_std(black_box(hay), black_box(&n)));
        });
        scan.bench_with_input(BenchmarkId::new("memmem", size), &hay, |b, hay| {
            b.iter(|| index_memmem(black_box(hay), black_box(&n)));
        });
    }
    scan.finish();
}

/// Needle at offset 0: the scan exits immediately, so what is left is per-call
/// searcher construction — memmem's prefilter setup vs std's Two-Way setup
/// (caveat 4: this is the dominant shape for short jq scalar fields).
fn bench_index_at_start(c: &mut Criterion) {
    let n = needle(4);
    let mut early = c.benchmark_group("jq_index/at_start");
    for size in SIZES {
        let hay = haystack_with_needle(size, &n, Pos::Start);
        early.throughput(Throughput::Bytes(size as u64));
        early.bench_with_input(BenchmarkId::new("std", size), &hay, |b, hay| {
            b.iter(|| index_std(black_box(hay), black_box(&n)));
        });
        early.bench_with_input(BenchmarkId::new("memmem", size), &hay, |b, hay| {
            b.iter(|| index_memmem(black_box(hay), black_box(&n)));
        });
    }
    early.finish();
}

// ---------------------------------------------------------------------------
// Needle length effect at a fixed long haystack (needle absent -> full scan).
// The 1-byte column is where memmem (-> memchr single-byte SIMD) can most beat
// std's scalar &str Two-Way.
// ---------------------------------------------------------------------------

fn bench_needle_len(c: &mut Criterion) {
    const HAY: usize = 16384;
    let hay = filler(HAY);
    let mut group = c.benchmark_group("jq_index/needle_len@16k_absent");
    group.throughput(Throughput::Bytes(HAY as u64));
    for nlen in [1_usize, 4, 16, 64] {
        let n = needle(nlen);
        group.bench_with_input(BenchmarkId::new("std", nlen), &n, |b, n| {
            b.iter(|| index_std(black_box(&hay), black_box(n)));
        });
        group.bench_with_input(BenchmarkId::new("memmem", nlen), &n, |b, n| {
            b.iter(|| index_memmem(black_box(&hay), black_box(n)));
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// rindex — reverse scan, needle absent (full reverse scan).
// ---------------------------------------------------------------------------

fn bench_rindex(c: &mut Criterion) {
    let n = needle(4);
    let mut group = c.benchmark_group("jq_rindex/absent");
    for size in [64_usize, 1024, 16384, 65536] {
        let hay = filler(size);
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("std", size), &hay, |b, hay| {
            b.iter(|| rindex_std(black_box(hay), black_box(&n)));
        });
        group.bench_with_input(BenchmarkId::new("memmem", size), &hay, |b, hay| {
            b.iter(|| rindex_memmem(black_box(hay), black_box(&n)));
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// contains — predicate, needle absent (miss => full scan, the worst case).
// ---------------------------------------------------------------------------

fn bench_contains(c: &mut Criterion) {
    let n = needle(4);
    let mut group = c.benchmark_group("jq_contains/miss");
    for size in [16_usize, 64, 1024, 16384] {
        let hay = filler(size);
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("std", size), &hay, |b, hay| {
            b.iter(|| contains_std(black_box(hay), black_box(&n)));
        });
        group.bench_with_input(BenchmarkId::new("memmem", size), &hay, |b, hay| {
            b.iter(|| contains_memmem(black_box(hay), black_box(&n)));
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// indices — dense overlapping matches ("aaaa..." / "aa"). Allocation-bound: the
// output Vec grows with the haystack, so the scan is a minority of wall-time.
// ---------------------------------------------------------------------------

fn bench_indices(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_indices/overlapping");
    for size in [64_usize, 1024, 16384] {
        let hay = "a".repeat(size);
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("std", size), &hay, |b, hay| {
            b.iter(|| indices_std(black_box(hay), black_box("aa")));
        });
        group.bench_with_input(BenchmarkId::new("memmem", size), &hay, |b, hay| {
            b.iter(|| indices_memmem(black_box(hay), black_box("aa")));
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// split — separator frequency knob. Both variants allocate a String per part,
// so this is dominated by allocation, not by the search.
// ---------------------------------------------------------------------------

/// Rare separator: few, large parts — the closest `split` gets to a pure scan.
fn bench_split_rare(c: &mut Criterion) {
    let mut rare = c.benchmark_group("jq_split/rare_sep");
    for size in [1024_usize, 16384, 65536] {
        let hay = haystack_with_sep(size, "<SEP>", 4096); // a handful of parts
        let len = hay.len() as u64;
        rare.throughput(Throughput::Bytes(len));
        rare.bench_with_input(BenchmarkId::new("std", size), &hay, |b, hay| {
            b.iter(|| split_std(black_box(hay), black_box("<SEP>")));
        });
        rare.bench_with_input(BenchmarkId::new("memmem", size), &hay, |b, hay| {
            b.iter(|| split_memmem(black_box(hay), black_box("<SEP>")));
        });
    }
    rare.finish();
}

/// Dense separator (CSV-like): many small parts, so the per-part `String`
/// allocation dominates and the search primitive barely registers.
fn bench_split_dense(c: &mut Criterion) {
    let mut dense = c.benchmark_group("jq_split/dense_sep");
    for size in [1024_usize, 16384] {
        let hay = haystack_with_sep(size, ",", 8); // CSV-like: many small parts
        let len = hay.len() as u64;
        dense.throughput(Throughput::Bytes(len));
        dense.bench_with_input(BenchmarkId::new("std", size), &hay, |b, hay| {
            b.iter(|| split_std(black_box(hay), black_box(",")));
        });
        dense.bench_with_input(BenchmarkId::new("memmem", size), &hay, |b, hay| {
            b.iter(|| split_memmem(black_box(hay), black_box(",")));
        });
    }
    dense.finish();
}

criterion_group! {
    name = benches;
    // Modest per-bench budget: this is a crossover study across ~70 configs, not
    // a regression gate. Enough samples for a stable median, short enough to run.
    config = Criterion::default()
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(2))
        .sample_size(60);
    targets =
        bench_index_absent,
        bench_index_at_start,
        bench_needle_len,
        bench_rindex,
        bench_contains,
        bench_indices,
        bench_split_rare,
        bench_split_dense,
}
criterion_main!(benches);

// ---------------------------------------------------------------------------
// Parity guard — the memmem variants MUST match std byte-for-byte, or the timing
// is meaningless. A `harness = false` bench has no libtest to run `#[test]`s, so
// this runs at the top of `bench_index` instead: it executes (and panics on any
// divergence) on every `cargo bench` / `cargo test --bench jq_string_ops_bench`.
// It guards the two semantics traps #303 calls out: `indices` overlap and
// `split` empty-part edge cases.
// ---------------------------------------------------------------------------

fn check_parity() {
    // `indices` is overlapping (eval.rs advances by +1), NOT memmem::find_iter's [0, 2].
    assert_eq!(indices_std("aaaa", "aa"), vec![0, 1, 2]);
    assert_eq!(indices_memmem("aaaa", "aa"), vec![0, 1, 2]);
    for (hay, ndl) in [
        ("aaaa", "aa"),
        ("abababab", "ab"),
        ("mississippi", "iss"),
        ("no-match-here", "ZZ"),
        ("", "a"),
        ("a", "a"),
    ] {
        assert_eq!(
            indices_std(hay, ndl),
            indices_memmem(hay, ndl),
            "indices mismatch for hay={hay:?} needle={ndl:?}"
        );
    }

    // `split` empty-part parity with `str::split(&str)`.
    for (hay, sep) in [
        ("a,b,c", ","),
        ("a,b,", ","), // trailing sep -> trailing ""
        ("a,,b", ","), // adjacent seps -> interior ""
        (",a", ","),   // leading sep -> leading ""
        ("abc", ","),  // no sep -> whole string
        ("", ","),     // empty haystack -> [""]
        ("a<->b<->", "<->"),
        ("no-sep", "<SEP>"),
    ] {
        assert_eq!(
            split_memmem(hay, sep),
            split_std(hay, sep),
            "split mismatch for hay={hay:?} sep={sep:?}"
        );
    }

    // index / rindex / contains agreement, including misses and boundaries.
    for (hay, ndl) in [
        ("hello world", "o"),
        ("hello world", "world"),
        ("hello world", "z"),
        ("", "x"),
        ("aXbXcX", "X"),
        ("boundary", "boundary"),
    ] {
        assert_eq!(
            index_std(hay, ndl),
            index_memmem(hay, ndl),
            "index {hay:?} {ndl:?}"
        );
        assert_eq!(
            rindex_std(hay, ndl),
            rindex_memmem(hay, ndl),
            "rindex {hay:?} {ndl:?}"
        );
        assert_eq!(
            contains_std(hay, ndl),
            contains_memmem(hay, ndl),
            "contains {hay:?} {ndl:?}"
        );
    }

    // The exact generated inputs the benchmarks time must produce identical results.
    let n = needle(4);
    for size in SIZES {
        let absent = filler(size);
        assert_eq!(index_std(&absent, &n), index_memmem(&absent, &n));
        assert_eq!(rindex_std(&absent, &n), rindex_memmem(&absent, &n));
        assert_eq!(contains_std(&absent, &n), contains_memmem(&absent, &n));
        for pos in [Pos::Start, Pos::Middle, Pos::End] {
            let hay = haystack_with_needle(size, &n, pos);
            assert_eq!(index_std(&hay, &n), index_memmem(&hay, &n));
            assert_eq!(rindex_std(&hay, &n), rindex_memmem(&hay, &n));
        }
    }
}
