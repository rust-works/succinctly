//! Benchmarks for the jq format functions -- `@uri`, `@html`, `@csv`, `@dsv`,
//! `@tsv`, `@sh` (issue #124).
//!
//! Run with:
//! ```bash
//! cargo bench --bench jq_format_bench
//! ```
//!
//! Two tiers, because #124 has three candidate costs stacked on top of each
//! other and they must be attributable separately:
//!
//! * `jq_format/e2e/*` -- the **gating** measurement. Drives whole queries
//!   through both evaluators. The `generic` arm is the CLI path; the `full`
//!   arm is the library's `jq::eval` entry point. Any remaining gap between
//!   them is the two evaluators' general dispatch overhead, not a format
//!   round-trip: `eval_generic` now formats values directly (this issue's
//!   round-trip-elimination half), the same as `full`.
//! * `jq_format/{scan,boundary,density}/*` -- per-format escaping throughput on
//!   a single large string, isolating the escape loop that SIMD would target.
//!   Size sweeps include the SIMD boundary sizes (15/16/17, 31/32/33, 63/64/65)
//!   so a 16-byte threshold's effect is visible.
//!
//! Everything runs in-process through the public API (unlike
//! `benches/jq_comparison.rs`, which shells out to the binary) so that
//! process-spawn noise doesn't swamp the effect being measured.
//!
//! NOTE: this repo has rejected seven optimizations that won a micro-benchmark
//! and lost end-to-end (P2.6, P2.8, P3, P5, P6, P7, P8). The `scan` tier is
//! diagnostic only; adoption decisions are made on `e2e`.

// #1670: `clippy.toml`'s `disallowed-methods` bans a bare `Vec`/`String`
// `with_capacity` crate-wide (re-enabled only in `succinctly::jq::eval`/
// `eval_generic`) -- every call site here sizes from a single collection's
// own length (or a length-times-constant) and was never part of that bug
// shape.
#![allow(clippy::disallowed_methods)]

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use succinctly::jq::{eval, eval_generic, parse, Expr, JqSemantics, QueryResult};
use succinctly::json::JsonIndex;

// ---------------------------------------------------------------------------
// Drivers
// ---------------------------------------------------------------------------

/// Full evaluator (`src/jq/eval.rs`) -- the library entry point.
fn run_full(json: &[u8], expr: &Expr) -> usize {
    let index = JsonIndex::build(json);
    let cursor = index.root(json);
    let result: QueryResult<Vec<u64>> = eval::<Vec<u64>, JqSemantics>(expr, cursor);
    result.collect_owned().len()
}

/// Generic evaluator (`src/jq/eval_generic.rs`) -- the CLI path.
fn run_generic(json: &[u8], expr: &Expr) -> usize {
    let index = JsonIndex::build(json);
    let cursor = index.root(json);
    eval_generic::eval_with_cursor(expr, cursor)
        .collect_owned()
        .expect("materializes")
        .len()
}

// ---------------------------------------------------------------------------
// Corpus generation
// ---------------------------------------------------------------------------

/// Build a payload of `len` bytes drawn from `safe`, injecting one byte from
/// `specials` every `every` positions. `every == 0` means "no specials at all",
/// the case where a SIMD scanner has the most to gain.
fn make_payload(len: usize, every: usize, safe: &[u8], specials: &[u8]) -> String {
    let mut s = String::with_capacity(len);
    for i in 0..len {
        if every != 0 && i % every == every - 1 {
            s.push(specials[(i / every) % specials.len()] as char);
        } else {
            s.push(safe[i % safe.len()] as char);
        }
    }
    s
}

/// Wrap a payload as a JSON string document, escaping what JSON requires.
fn json_string_doc(payload: &str) -> Vec<u8> {
    let mut out = String::with_capacity(payload.len() + 2);
    out.push('"');
    for c in payload.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out.into_bytes()
}

/// Wrap a payload as a single-element JSON array document (for `@csv`/`@tsv`).
fn json_array_doc(payload: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 4);
    out.push(b'[');
    out.extend_from_slice(&json_string_doc(payload));
    out.push(b']');
    out
}

/// `{"u":[{"n":"...","v":N},...]}` -- the record shape the CLI actually sees.
fn users_doc(records: usize) -> Vec<u8> {
    let mut out = String::from(r#"{"u":["#);
    for i in 0..records {
        if i > 0 {
            out.push(',');
        }
        // Names carry characters every format has to escape: space, `&`, `<`,
        // `/` and a quote.
        out.push_str(&format!(r#"{{"n":"user {i} &<name>/x \"q\"","v":{i}}}"#));
    }
    out.push_str("]}");
    out.into_bytes()
}

const URI_SAFE: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_.~";
const URI_SPECIAL: &[u8] = b" /?&=%+#";
const HTML_SAFE: &[u8] = b"abcdefghijklmnopqrstuvwxyz ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const HTML_SPECIAL: &[u8] = b"<>&'\"";
const CSV_SAFE: &[u8] = b"abcdefghijklmnopqrstuvwxyz ,;0123456789";
const CSV_SPECIAL: &[u8] = b"\"";
const TSV_SAFE: &[u8] = b"abcdefghijklmnopqrstuvwxyz 0123456789";
const TSV_SPECIAL: &[u8] = b"\t\\";
const SH_SAFE: &[u8] = b"abcdefghijklmnopqrstuvwxyz 0123456789";
const SH_SPECIAL: &[u8] = b"'";

const SIZES: &[usize] = &[16, 32, 64, 128, 256, 512, 1024, 4096];
/// Sizes straddling the 16/32/64-byte SIMD chunk boundaries.
const BOUNDARY_SIZES: &[usize] = &[15, 16, 17, 31, 32, 33, 63, 64, 65];

/// Every scalar-input format, with the corpus alphabet that exercises it.
/// `array` selects the document shape the format demands.
struct FormatCase {
    name: &'static str,
    filter: &'static str,
    safe: &'static [u8],
    special: &'static [u8],
    array: bool,
}

const CASES: &[FormatCase] = &[
    FormatCase {
        name: "uri",
        filter: "@uri",
        safe: URI_SAFE,
        special: URI_SPECIAL,
        array: false,
    },
    FormatCase {
        name: "html",
        filter: "@html",
        safe: HTML_SAFE,
        special: HTML_SPECIAL,
        array: false,
    },
    FormatCase {
        name: "sh",
        filter: "@sh",
        safe: SH_SAFE,
        special: SH_SPECIAL,
        array: false,
    },
    FormatCase {
        name: "csv",
        filter: "@csv",
        safe: CSV_SAFE,
        special: CSV_SPECIAL,
        array: true,
    },
    FormatCase {
        name: "tsv",
        filter: "@tsv",
        safe: TSV_SAFE,
        special: TSV_SPECIAL,
        array: true,
    },
    FormatCase {
        name: "dsv",
        filter: r#"@dsv("|")"#,
        safe: CSV_SAFE,
        special: CSV_SPECIAL,
        array: true,
    },
];

fn doc_for(case: &FormatCase, payload: &str) -> Vec<u8> {
    if case.array {
        json_array_doc(payload)
    } else {
        json_string_doc(payload)
    }
}

// ---------------------------------------------------------------------------
// scan: per-format escaping throughput over a size sweep (no escapes present)
// ---------------------------------------------------------------------------

fn bench_scan(c: &mut Criterion) {
    for case in CASES {
        let mut group = c.benchmark_group(format!("jq_format/scan/{}", case.name));
        let expr = parse(case.filter).expect("parse");
        for &size in SIZES {
            let payload = make_payload(size, 0, case.safe, case.special);
            let doc = doc_for(case, &payload);
            group.throughput(Throughput::Bytes(doc.len() as u64));
            group.bench_with_input(BenchmarkId::from_parameter(size), &doc, |b, doc| {
                b.iter(|| black_box(run_full(black_box(doc), &expr)));
            });
        }
        group.finish();
    }
}

// ---------------------------------------------------------------------------
// boundary: sizes straddling the SIMD chunk boundaries
// ---------------------------------------------------------------------------

fn bench_boundary(c: &mut Criterion) {
    for case in CASES.iter().filter(|c| matches!(c.name, "uri" | "html")) {
        let mut group = c.benchmark_group(format!("jq_format/boundary/{}", case.name));
        let expr = parse(case.filter).expect("parse");
        for &size in BOUNDARY_SIZES {
            let payload = make_payload(size, 0, case.safe, case.special);
            let doc = doc_for(case, &payload);
            group.throughput(Throughput::Bytes(doc.len() as u64));
            group.bench_with_input(BenchmarkId::from_parameter(size), &doc, |b, doc| {
                b.iter(|| black_box(run_full(black_box(doc), &expr)));
            });
        }
        group.finish();
    }
}

// ---------------------------------------------------------------------------
// density: how the win decays as escapes get more frequent
// ---------------------------------------------------------------------------

fn bench_density(c: &mut Criterion) {
    // "none" is the SIMD best case; 1-in-10 is the worst realistic case.
    const DENSITIES: &[(&str, usize)] = &[("none", 0), ("sparse_1_100", 100), ("dense_1_10", 10)];
    for case in CASES {
        let mut group = c.benchmark_group(format!("jq_format/density/{}", case.name));
        let expr = parse(case.filter).expect("parse");
        for &(label, every) in DENSITIES {
            let payload = make_payload(4096, every, case.safe, case.special);
            let doc = doc_for(case, &payload);
            group.throughput(Throughput::Bytes(doc.len() as u64));
            group.bench_with_input(BenchmarkId::from_parameter(label), &doc, |b, doc| {
                b.iter(|| black_box(run_full(black_box(doc), &expr)));
            });
        }
        group.finish();
    }
}

// ---------------------------------------------------------------------------
// e2e: the gating measurement -- generic (CLI) vs full evaluator
// ---------------------------------------------------------------------------

fn bench_e2e(c: &mut Criterion) {
    // Queries that put a format at the end of a realistic pipeline. The first
    // two pipe a *constructed* array into the format, which routes through
    // `eval_on_owned`; the rest format a field directly.
    const QUERIES: &[(&str, &str)] = &[
        ("csv_records", r".u[] | [.n,.v] | @csv"),
        ("tsv_records", r".u[] | [.n,.v] | @tsv"),
        ("dsv_records", r#".u[] | [.n,.v] | @dsv("|")"#),
        ("sh_field", r".u[] | .n | @sh"),
        ("uri_field", r".u[] | .n | @uri"),
        ("html_field", r".u[] | .n | @html"),
        ("json_field", r".u[] | .n | @json"),
        ("collect_csv", r"[.u[].n] | @csv"),
    ];
    const RECORDS: &[usize] = &[10, 100, 1000];

    for &(name, filter) in QUERIES {
        let mut group = c.benchmark_group(format!("jq_format/e2e/{name}"));
        let expr = parse(filter).expect("parse");
        for &records in RECORDS {
            let doc = users_doc(records);
            group.throughput(Throughput::Bytes(doc.len() as u64));
            group.bench_with_input(BenchmarkId::new("generic", records), &doc, |b, doc| {
                b.iter(|| black_box(run_generic(black_box(doc), &expr)));
            });
            group.bench_with_input(BenchmarkId::new("full", records), &doc, |b, doc| {
                b.iter(|| black_box(run_full(black_box(doc), &expr)));
            });
        }
        group.finish();
    }
}

criterion_group!(
    benches,
    bench_e2e,
    bench_scan,
    bench_boundary,
    bench_density
);
criterion_main!(benches);
