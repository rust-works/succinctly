//! Micro-benchmarks for JSON escape scanning SIMD optimization (Issue #87).
//!
//! Measures the performance of `find_json_escape` which uses SIMD to find bytes
//! that need JSON escaping (", \, or control characters 0x00-0x1F).

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use succinctly::yaml::simd::find_json_escape;

/// Scalar implementation for comparison
fn find_json_escape_scalar(input: &[u8], start: usize) -> usize {
    for (i, &b) in input[start..].iter().enumerate() {
        if b == b'"' || b == b'\\' || b < 0x20 {
            return start + i;
        }
    }
    input.len()
}

/// Generate a string with no escape characters (best case for SIMD)
fn make_no_escapes(size: usize) -> Vec<u8> {
    // Use characters that don't need escaping: letters, digits, spaces
    let pattern = b"abcdefghijklmnopqrstuvwxyz0123456789 ";
    let mut result = Vec::with_capacity(size);
    for i in 0..size {
        result.push(pattern[i % pattern.len()]);
    }
    result
}

/// Generate a string with escape at the end (stress test - full scan)
fn make_escape_at_end(size: usize) -> Vec<u8> {
    let mut result = make_no_escapes(size);
    if !result.is_empty() {
        result[size - 1] = b'"';
    }
    result
}

/// Generate a string with escape at the beginning (best case for early exit)
fn make_escape_at_start(size: usize) -> Vec<u8> {
    let mut result = make_no_escapes(size);
    if !result.is_empty() {
        result[0] = b'"';
    }
    result
}

/// Generate a string with frequent escapes (realistic JSON-heavy case)
fn make_frequent_escapes(size: usize) -> Vec<u8> {
    // Every ~10 characters has an escape
    let mut result = make_no_escapes(size);
    for i in (10..size).step_by(10) {
        result[i] = match i % 30 {
            10 => b'"',
            20 => b'\\',
            _ => b'\n',
        };
    }
    result
}

/// Benchmark SIMD vs scalar for various string sizes
fn bench_simd_vs_scalar(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_escape/simd_vs_scalar");

    for size in [16, 32, 64, 128, 256, 512, 1024, 4096] {
        let input = make_no_escapes(size);
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(BenchmarkId::new("simd", size), &input, |b, input| {
            b.iter(|| find_json_escape(black_box(input), 0))
        });

        group.bench_with_input(BenchmarkId::new("scalar", size), &input, |b, input| {
            b.iter(|| find_json_escape_scalar(black_box(input), 0))
        });
    }

    group.finish();
}

/// Benchmark escape position impact
fn bench_escape_position(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_escape/position");
    let size = 256;

    // No escape (full scan, return None)
    let no_escape = make_no_escapes(size);
    group.throughput(Throughput::Bytes(size as u64));
    group.bench_function("no_escape", |b| {
        b.iter(|| find_json_escape(black_box(&no_escape), 0))
    });

    // Escape at start (early exit)
    let at_start = make_escape_at_start(size);
    group.bench_function("at_start", |b| {
        b.iter(|| find_json_escape(black_box(&at_start), 0))
    });

    // Escape in middle
    let mut at_middle = make_no_escapes(size);
    at_middle[size / 2] = b'"';
    group.bench_function("at_middle", |b| {
        b.iter(|| find_json_escape(black_box(&at_middle), 0))
    });

    // Escape at end
    let at_end = make_escape_at_end(size);
    group.bench_function("at_end", |b| {
        b.iter(|| find_json_escape(black_box(&at_end), 0))
    });

    group.finish();
}

/// Benchmark different escape types
fn bench_escape_types(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_escape/types");
    let size = 256;

    // Quote character
    let mut with_quote = make_no_escapes(size);
    with_quote[size / 2] = b'"';
    group.throughput(Throughput::Bytes(size as u64));
    group.bench_function("quote", |b| {
        b.iter(|| find_json_escape(black_box(&with_quote), 0))
    });

    // Backslash
    let mut with_backslash = make_no_escapes(size);
    with_backslash[size / 2] = b'\\';
    group.bench_function("backslash", |b| {
        b.iter(|| find_json_escape(black_box(&with_backslash), 0))
    });

    // Newline (control char)
    let mut with_newline = make_no_escapes(size);
    with_newline[size / 2] = b'\n';
    group.bench_function("newline", |b| {
        b.iter(|| find_json_escape(black_box(&with_newline), 0))
    });

    // Tab (control char)
    let mut with_tab = make_no_escapes(size);
    with_tab[size / 2] = b'\t';
    group.bench_function("tab", |b| {
        b.iter(|| find_json_escape(black_box(&with_tab), 0))
    });

    // Null (control char - edge case)
    let mut with_null = make_no_escapes(size);
    with_null[size / 2] = 0;
    group.bench_function("null", |b| {
        b.iter(|| find_json_escape(black_box(&with_null), 0))
    });

    group.finish();
}

/// Benchmark realistic workloads
fn bench_realistic(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_escape/realistic");

    // Typical JSON field value (short, no escapes)
    let short_value = b"hello_world_value_123";
    group.throughput(Throughput::Bytes(short_value.len() as u64));
    group.bench_function("short_value", |b| {
        b.iter(|| find_json_escape(black_box(short_value), 0))
    });

    // Typical JSON with quotes (common in config files)
    let with_quotes = br#"This string has "quotes" inside it"#;
    group.throughput(Throughput::Bytes(with_quotes.len() as u64));
    group.bench_function("with_quotes", |b| {
        b.iter(|| find_json_escape(black_box(with_quotes), 0))
    });

    // Multiline text with newlines
    let multiline = b"Line one\nLine two\nLine three\nLine four";
    group.throughput(Throughput::Bytes(multiline.len() as u64));
    group.bench_function("multiline", |b| {
        b.iter(|| find_json_escape(black_box(multiline), 0))
    });

    // Path-like string with backslashes
    let path = br"C:\Users\Admin\Documents\file.txt";
    group.throughput(Throughput::Bytes(path.len() as u64));
    group.bench_function("path", |b| b.iter(|| find_json_escape(black_box(path), 0)));

    // Long description (common in YAML configs)
    let long_desc = make_no_escapes(500);
    group.throughput(Throughput::Bytes(long_desc.len() as u64));
    group.bench_function("long_desc", |b| {
        b.iter(|| find_json_escape(black_box(&long_desc), 0))
    });

    // Frequent escapes (stress test)
    let frequent = make_frequent_escapes(500);
    group.throughput(Throughput::Bytes(frequent.len() as u64));
    group.bench_function("frequent_escapes", |b| {
        b.iter(|| find_json_escape(black_box(&frequent), 0))
    });

    group.finish();
}

/// Benchmark alignment edge cases
fn bench_alignment(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_escape/alignment");

    // Test various sizes around SIMD boundaries (16, 32 bytes)
    for size in [15, 16, 17, 31, 32, 33, 63, 64, 65] {
        let input = make_no_escapes(size);
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size), &input, |b, input| {
            b.iter(|| find_json_escape(black_box(input), 0))
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_simd_vs_scalar,
    bench_escape_position,
    bench_escape_types,
    bench_realistic,
    bench_alignment,
);
criterion_main!(benches);
