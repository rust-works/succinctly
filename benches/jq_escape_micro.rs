//! A/B micro-benchmarks: the old per-char JSON escapers vs the shared
//! SIMD-backed escaper (issue #91).
//!
//! # What is actually being measured
//!
//! Not the SIMD. `docs/benchmarks/corpus-shape.md` puts real JSON string lengths
//! at p50 = 7 bytes, p90 = 11, p99 = 24, max 51, with an escape density of
//! 0.00 per KiB. The scanners engage at 16 bytes (NEON/SSE2) and 32 (AVX2), so
//! **over 90% of real strings never reach a SIMD kernel at all** and the ones
//! that do find nothing to escape.
//!
//! What #91 actually changed on those inputs is the shape of the scalar work:
//!
//! | | before | after |
//! |---|---|---|
//! | safe runs | `result.push(c)` per `char` | one `write_str` per span |
//! | control chars | `format!("\\u{:04x}")` — a heap allocation each | 4 writes from a nibble table |
//! | decoding | UTF-8 decoded to `char`, then re-encoded | bytes copied as bytes |
//!
//! So the `SHORT` sweep below is the one that decides this issue. A win at 1 KiB
//! with no escapes is not evidence of anything — that is precisely the shape of
//! the P2.6 / P2.8 / P3 / P5 micro-benchmark wins this project has rejected four
//! times for failing to survive end-to-end.
//!
//! # Self-contained A/B
//!
//! Both implementations live in this file, following `jq_string_ops_bench.rs`:
//! the `frozen_*` functions are verbatim copies of the four per-char escapers
//! that lived in `src/bin/succinctly/output.rs` before #91. That makes each run a
//! single-commit measurement needing no cross-branch criterion baseline, and it
//! stops the reference drifting as the real escaper evolves. **Never "fix" them.**
//!
//! Run with: `cargo bench --bench jq_escape_micro`

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use std::time::Duration;

use succinctly::json::escape::{body_to_string, EscapeStyle};

// ---------------------------------------------------------------------------
// Frozen per-char escapers — the pre-#91 implementations, verbatim.
// ---------------------------------------------------------------------------

fn frozen_jq(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\x08' => result.push_str("\\b"),
            '\x0C' => result.push_str("\\f"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_control() => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => result.push(c),
        }
    }
    result
}

fn frozen_jq_ascii(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\x08' => result.push_str("\\b"),
            '\x0C' => result.push_str("\\f"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_control() => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            c if !c.is_ascii() => {
                let code = c as u32;
                if code <= 0xFFFF {
                    result.push_str(&format!("\\u{code:04x}"));
                } else {
                    let adjusted = code - 0x10000;
                    let high = 0xD800 + (adjusted >> 10);
                    let low = 0xDC00 + (adjusted & 0x3FF);
                    result.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
                }
            }
            c => result.push(c),
        }
    }
    result
}

fn frozen_yq(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => result.push(c),
        }
    }
    result
}

fn frozen_yq_ascii(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            c if !c.is_ascii() => {
                let code = c as u32;
                if code <= 0xFFFF {
                    result.push_str(&format!("\\u{code:04x}"));
                } else {
                    let adjusted = code - 0x10000;
                    let high = 0xD800 + (adjusted >> 10);
                    let low = 0xDC00 + (adjusted & 0x3FF);
                    result.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
                }
            }
            c => result.push(c),
        }
    }
    result
}

/// A whole-string escaper: name, style, ASCII mode, and the frozen per-char
/// implementation to compare against.
type Mode = (&'static str, EscapeStyle, bool, fn(&str) -> String);

/// The four (style, ascii) modes, each with its frozen counterpart.
const MODES: [Mode; 4] = [
    ("jq", EscapeStyle::Jq, false, frozen_jq),
    ("jq_ascii", EscapeStyle::Jq, true, frozen_jq_ascii),
    ("yq", EscapeStyle::Yq, false, frozen_yq),
    ("yq_ascii", EscapeStyle::Yq, true, frozen_yq_ascii),
];

// ---------------------------------------------------------------------------
// Parity guard.
//
// A `harness = false` bench has no libtest, so this runs at the top of the first
// group instead — timing two functions that disagree measures nothing. #91
// deliberately diverges from the frozen escapers on exactly one input class (the
// C1 block under jq's non-ASCII mode), so that class is excluded here and pinned
// by `c1_block_is_raw_under_jq_and_escaped_by_the_frozen_escaper` in
// src/json/escape.rs instead.
// ---------------------------------------------------------------------------

fn check_parity() {
    let corpus: Vec<String> = vec![
        String::new(),
        "plain ascii, the common case".into(),
        "quote \" backslash \\ newline \n tab \t".into(),
        "\u{0}\u{1}\u{8}\u{c}\u{1f}".into(),
        "\u{7f} DEL and \u{a0} nbsp".into(),
        "café au lait — naïve, £5, «quoted»".into(),
        "日本語 😀 astral".into(),
        "x".repeat(1000),
    ];
    for s in &corpus {
        for (name, style, ascii, frozen) in MODES {
            // The one approved divergence: C1 under jq's non-ASCII mode.
            if style == EscapeStyle::Jq
                && !ascii
                && s.chars().any(|c| ('\u{80}'..='\u{9f}').contains(&c))
            {
                continue;
            }
            assert_eq!(
                body_to_string(s, style, ascii),
                frozen(s),
                "parity broken for {name} on {s:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Payload generators.
// ---------------------------------------------------------------------------

/// Escape-free ASCII — what `corpus-shape.md` says real data looks like.
fn no_escape(n: usize) -> String {
    let pattern = b"abcdefghijklmnopqrstuvwxyz0123456789 ";
    (0..n).map(|i| pattern[i % pattern.len()] as char).collect()
}

/// One escape every 20 bytes, matching the "realistic" shape #87 reported, so
/// these numbers are comparable to the O3 write-up.
fn escape_every_20(n: usize) -> String {
    let mut s: Vec<char> = no_escape(n).chars().collect();
    for i in (20..n).step_by(20) {
        s[i] = match (i / 20) % 3 {
            0 => '"',
            1 => '\\',
            _ => '\n',
        };
    }
    s.into_iter().collect()
}

/// Escape only at the very end: the full-scan worst case.
fn escape_at_end(n: usize) -> String {
    if n == 0 {
        return String::new();
    }
    let mut s = no_escape(n - 1);
    s.push('"');
    s
}

/// Escape only in the scalar remainder, past both SIMD chunk loops.
fn escape_only_in_remainder(n: usize) -> String {
    let body = n.saturating_sub(1) / 32 * 32;
    let mut s = no_escape(body);
    s.push_str(&no_escape(n.saturating_sub(body + 1)));
    s.push('"');
    s
}

/// Control characters every 8 bytes — the shape that most stresses the per-escape
/// `format!` allocation #91 removes.
fn control_heavy(n: usize) -> String {
    let mut s: Vec<char> = no_escape(n).chars().collect();
    for i in (0..n).step_by(8) {
        s[i] = match (i / 8) % 4 {
            0 => '\u{8}',
            1 => '\u{c}',
            2 => '\u{1}',
            _ => '\u{7f}',
        };
    }
    s.into_iter().collect()
}

/// CJK and astral characters: exercises the ASCII modes' per-char `\uXXXX` and
/// surrogate-pair path, and the non-ASCII modes' bulk-copy path.
fn heavy_unicode(n: usize) -> String {
    let pattern = ['日', '本', '語', '😀', 'a', 'é'];
    let mut s = String::with_capacity(n);
    let mut i = 0;
    while s.len() < n {
        s.push(pattern[i % pattern.len()]);
        i += 1;
    }
    s
}

/// Latin-1 punctuation, all of which is UTF-8 lead byte 0xC2.
///
/// Kept as a standing guard: #91 chose a pure-ASCII jq predicate precisely so
/// this input has no false-positive stops. If a future change escapes the C1
/// block by flagging 0xC2, this is the cell that will show the bill.
fn latin1_heavy(n: usize) -> String {
    let pattern = ['«', '°', '»', '¢', '£', '¿', 'a', 'b'];
    let mut s = String::with_capacity(n);
    let mut i = 0;
    while s.len() < n {
        s.push(pattern[i % pattern.len()]);
        i += 1;
    }
    s
}

/// A payload generator: builds a test string of the requested byte length.
type Generator = fn(usize) -> String;

/// Corpus-derived sizes. These decide acceptance; the tail sizes are context.
const SHORT: [usize; 8] = [1, 4, 7, 11, 15, 16, 17, 24];
const REAL: [usize; 3] = [31, 32, 51];
const TAIL: [usize; 5] = [64, 256, 1024, 4096, 16384];

// ---------------------------------------------------------------------------
// Benchmarks.
// ---------------------------------------------------------------------------

/// The acceptance gate: strings at the real corpus percentiles.
///
/// The go/no-go for #91 is that no cell here is more than ~2% slower than its
/// frozen counterpart. A regression at these sizes means the escaper is paying
/// SIMD dispatch it can never earn back, which is exactly the failure O3 hit
/// before the 16-byte threshold and `#[inline(always)]` went in.
fn bench_short_strings(c: &mut Criterion) {
    check_parity();

    let mut group = c.benchmark_group("jq_escape/short");
    // Resolving a ~2% effect on a ~10ns operation needs more samples than the
    // criterion default budget gives.
    group.sample_size(200);
    group.measurement_time(Duration::from_secs(5));

    for size in SHORT.iter().chain(REAL.iter()).copied() {
        let input = no_escape(size);
        group.throughput(Throughput::Bytes(size as u64));
        for (name, style, ascii, frozen) in MODES {
            group.bench_with_input(
                BenchmarkId::new(format!("{name}/simd"), size),
                &input,
                |b, s| b.iter(|| body_to_string(black_box(s), style, ascii)),
            );
            group.bench_with_input(
                BenchmarkId::new(format!("{name}/frozen"), size),
                &input,
                |b, s| b.iter(|| frozen(black_box(s))),
            );
        }
    }
    group.finish();
}

/// Object-shaped traffic at the corpus median: a few short keys and values, the
/// mix a real `jq` run actually emits.
fn bench_object_shaped(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_escape/object_shaped");
    group.sample_size(200);
    group.measurement_time(Duration::from_secs(5));

    // corpus-shape.md: object keys p50 = 2, key len 4-8, string value len p50 = 7.
    let fields: Vec<String> = ["userId", "name", "email", "active"]
        .iter()
        .map(|k| (*k).to_string())
        .chain((0..4).map(|i| no_escape(7 + i)))
        .collect();

    for (name, style, ascii, frozen) in MODES {
        group.bench_function(BenchmarkId::new(format!("{name}/simd"), "fields"), |b| {
            b.iter(|| {
                for f in &fields {
                    black_box(body_to_string(black_box(f), style, ascii));
                }
            });
        });
        group.bench_function(BenchmarkId::new(format!("{name}/frozen"), "fields"), |b| {
            b.iter(|| {
                for f in &fields {
                    black_box(frozen(black_box(f)));
                }
            });
        });
    }
    group.finish();
}

/// Payload shapes at a fixed, SIMD-reachable size.
///
/// This is where the scanner earns its keep — and where a win proves the least,
/// since strings this long are past the corpus p99.
fn bench_payload_shapes(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_escape/shapes");

    let shapes: [(&str, Generator); 7] = [
        ("no_escape", no_escape),
        ("escape_every_20", escape_every_20),
        ("escape_at_end", escape_at_end),
        ("escape_only_in_remainder", escape_only_in_remainder),
        ("control_heavy", control_heavy),
        ("heavy_unicode", heavy_unicode),
        ("latin1_heavy", latin1_heavy),
    ];

    for (shape, gen) in shapes {
        let input = gen(256);
        group.throughput(Throughput::Bytes(input.len() as u64));
        for (name, style, ascii, frozen) in MODES {
            group.bench_with_input(
                BenchmarkId::new(format!("{name}/simd"), shape),
                &input,
                |b, s| b.iter(|| body_to_string(black_box(s), style, ascii)),
            );
            group.bench_with_input(
                BenchmarkId::new(format!("{name}/frozen"), shape),
                &input,
                |b, s| b.iter(|| frozen(black_box(s))),
            );
        }
    }
    group.finish();
}

/// The long tail. Included for completeness; a win here is not evidence that
/// #91 helps real workloads (see the module docs).
fn bench_long_strings(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_escape/long");

    for size in TAIL {
        for (shape, gen) in [
            ("no_escape", no_escape as Generator),
            ("escape_every_20", escape_every_20),
        ] {
            let input = gen(size);
            group.throughput(Throughput::Bytes(size as u64));
            // Only the two non-ASCII modes here: the ASCII ones are dominated by
            // per-char \uXXXX emission, which the shapes group already covers.
            for (name, style, ascii, frozen) in MODES.iter().filter(|m| !m.2) {
                group.bench_with_input(
                    BenchmarkId::new(format!("{name}/simd/{shape}"), size),
                    &input,
                    |b, s| b.iter(|| body_to_string(black_box(s), *style, *ascii)),
                );
                group.bench_with_input(
                    BenchmarkId::new(format!("{name}/frozen/{shape}"), size),
                    &input,
                    |b, s| b.iter(|| frozen(black_box(s))),
                );
            }
        }
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(2))
        .sample_size(100);
    targets =
        bench_short_strings,
        bench_object_shaped,
        bench_payload_shapes,
        bench_long_strings,
}
criterion_main!(benches);
