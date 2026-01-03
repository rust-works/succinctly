//! Criterion benchmarks for JSON SIMD parsing operations.
//!
//! Measures performance of different SIMD implementations:
//! - x86_64: SSE2, SSE4.2, AVX2
//! - Scalar reference implementations
//!
//! Run with:
//! ```bash
//! cargo bench --bench json_simd
//! ```

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

/// Generate a realistic JSON document with nested structures.
fn generate_json(approx_size: usize) -> String {
    let mut json = String::with_capacity(approx_size);
    json.push_str("{\"users\":[");

    let num_users = approx_size / 150; // Each user ~150 bytes
    for i in 0..num_users {
        if i > 0 {
            json.push(',');
        }
        json.push_str(&format!(
            "{{\"id\":{},\"name\":\"User{}\",\"email\":\"user{}@example.com\",\"active\":true,\"score\":{}}}",
            i, i, i, i * 10
        ));
    }

    json.push_str("]}");
    json
}

/// Benchmark character classification across SIMD levels (x86_64).
#[cfg(target_arch = "x86_64")]
fn bench_classify_chars(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_classify_chars");

    // Test data sizes that align with SIMD chunk sizes
    let sizes = vec![
        ("16B_SSE", 16),
        ("32B_AVX", 32),
        ("1KB", 1024),
        ("16KB", 16 * 1024),
    ];

    for (name, size) in sizes {
        let json = generate_json(size);
        let bytes = json.as_bytes();

        group.throughput(Throughput::Bytes(bytes.len() as u64));

        // SSE2 baseline (always available)
        group.bench_with_input(BenchmarkId::new("SSE2", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::simd::x86::build_semi_index_standard(black_box(bytes)))
        });

        // SSE4.2 (if available)
        if is_x86_feature_detected!("sse4.2") {
            group.bench_with_input(BenchmarkId::new("SSE4.2", name), &bytes, |b, bytes| {
                b.iter(|| {
                    succinctly::json::simd::sse42::build_semi_index_standard(black_box(bytes))
                })
            });
        }

        // AVX2 (if available)
        if is_x86_feature_detected!("avx2") {
            group.bench_with_input(BenchmarkId::new("AVX2", name), &bytes, |b, bytes| {
                b.iter(|| succinctly::json::simd::avx2::build_semi_index_standard(black_box(bytes)))
            });
        }

        // Scalar reference
        group.bench_with_input(BenchmarkId::new("Scalar", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::standard::build_semi_index(black_box(bytes)))
        });
    }

    group.finish();
}

/// Benchmark simple cursor across SIMD levels (x86_64).
#[cfg(target_arch = "x86_64")]
fn bench_simple_cursor(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_simple_cursor");

    let sizes = vec![("16B", 16), ("32B", 32), ("1KB", 1024), ("16KB", 16 * 1024)];

    for (name, size) in sizes {
        let json = generate_json(size);
        let bytes = json.as_bytes();

        group.throughput(Throughput::Bytes(bytes.len() as u64));

        // SSE2
        group.bench_with_input(BenchmarkId::new("SSE2", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::simd::x86::build_semi_index_simple(black_box(bytes)))
        });

        // SSE4.2 (if available)
        if is_x86_feature_detected!("sse4.2") {
            group.bench_with_input(BenchmarkId::new("SSE4.2", name), &bytes, |b, bytes| {
                b.iter(|| succinctly::json::simd::sse42::build_semi_index_simple(black_box(bytes)))
            });
        }

        // AVX2 (if available)
        if is_x86_feature_detected!("avx2") {
            group.bench_with_input(BenchmarkId::new("AVX2", name), &bytes, |b, bytes| {
                b.iter(|| succinctly::json::simd::avx2::build_semi_index_simple(black_box(bytes)))
            });
        }

        // Scalar reference
        group.bench_with_input(BenchmarkId::new("Scalar", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::simple::build_semi_index(black_box(bytes)))
        });
    }

    group.finish();
}

/// Benchmark JSON parsing throughput on realistic documents (x86_64).
#[cfg(target_arch = "x86_64")]
fn bench_json_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_throughput");

    let sizes = vec![
        ("1KB", 1024),
        ("16KB", 16 * 1024),
        ("128KB", 128 * 1024),
        ("1MB", 1024 * 1024),
    ];

    for (name, size) in sizes {
        let json = generate_json(size);
        let bytes = json.as_bytes();

        group.throughput(Throughput::Bytes(bytes.len() as u64));

        // AVX2 (if available)
        if is_x86_feature_detected!("avx2") {
            group.bench_with_input(BenchmarkId::new("AVX2", name), &bytes, |b, bytes| {
                b.iter(|| succinctly::json::simd::avx2::build_semi_index_standard(black_box(bytes)))
            });
        }

        // SSE4.2 (if available)
        if is_x86_feature_detected!("sse4.2") {
            group.bench_with_input(BenchmarkId::new("SSE4.2", name), &bytes, |b, bytes| {
                b.iter(|| {
                    succinctly::json::simd::sse42::build_semi_index_standard(black_box(bytes))
                })
            });
        }

        // SSE2 (always available)
        group.bench_with_input(BenchmarkId::new("SSE2", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::simd::x86::build_semi_index_standard(black_box(bytes)))
        });

        // Scalar baseline
        group.bench_with_input(BenchmarkId::new("Scalar", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::standard::build_semi_index(black_box(bytes)))
        });
    }

    group.finish();
}

/// Benchmark simple vs standard cursor (x86_64).
#[cfg(target_arch = "x86_64")]
fn bench_cursor_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("cursor_comparison");

    let json = generate_json(64 * 1024); // 64KB
    let bytes = json.as_bytes();

    group.throughput(Throughput::Bytes(bytes.len() as u64));

    // Test best available SIMD for each cursor type
    if is_x86_feature_detected!("avx2") {
        group.bench_function("standard_avx2", |b| {
            b.iter(|| succinctly::json::simd::avx2::build_semi_index_standard(black_box(bytes)))
        });

        group.bench_function("simple_avx2", |b| {
            b.iter(|| succinctly::json::simd::avx2::build_semi_index_simple(black_box(bytes)))
        });
    } else {
        group.bench_function("standard_sse2", |b| {
            b.iter(|| succinctly::json::simd::x86::build_semi_index_standard(black_box(bytes)))
        });

        group.bench_function("simple_sse2", |b| {
            b.iter(|| succinctly::json::simd::x86::build_semi_index_simple(black_box(bytes)))
        });
    }

    group.bench_function("standard_scalar", |b| {
        b.iter(|| succinctly::json::standard::build_semi_index(black_box(bytes)))
    });

    group.bench_function("simple_scalar", |b| {
        b.iter(|| succinctly::json::simple::build_semi_index(black_box(bytes)))
    });

    group.finish();
}

/// Benchmark JSON parsing with different content characteristics (x86_64).
#[cfg(target_arch = "x86_64")]
fn bench_content_types(c: &mut Criterion) {
    let mut group = c.benchmark_group("content_types");

    // Different JSON patterns stress different code paths
    let test_cases = vec![
        ("dense_objects", r#"{"a":"b","c":"d","e":"f","g":"h","i":"j","k":"l","m":"n","o":"p","q":"r","s":"t"}"#.repeat(100)),
        ("nested_arrays", "[[[[[[1,2,3,4,5]]]]]]".repeat(50)),
        ("long_strings", r#"{"data":"Lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore et dolore magna aliqua"}"#.repeat(50)),
        ("mixed", r#"{"users":[{"id":1,"active":true,"tags":["a","b","c"]},{"id":2,"active":false,"tags":["d","e"]}]}"#.repeat(25)),
    ];

    for (name, json) in test_cases {
        let bytes = json.as_bytes();
        group.throughput(Throughput::Bytes(bytes.len() as u64));

        // AVX2 (if available)
        if is_x86_feature_detected!("avx2") {
            group.bench_with_input(BenchmarkId::new("AVX2", name), &bytes, |b, bytes| {
                b.iter(|| succinctly::json::simd::avx2::build_semi_index_standard(black_box(bytes)))
            });
        }

        // SSE2 (always available)
        group.bench_with_input(BenchmarkId::new("SSE2", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::simd::x86::build_semi_index_standard(black_box(bytes)))
        });

        // Scalar baseline
        group.bench_with_input(BenchmarkId::new("Scalar", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::standard::build_semi_index(black_box(bytes)))
        });
    }

    group.finish();
}

// Only include x86_64 benchmarks when on x86_64
#[cfg(target_arch = "x86_64")]
criterion_group!(
    benches,
    bench_classify_chars,
    bench_simple_cursor,
    bench_json_throughput,
    bench_cursor_comparison,
    bench_content_types,
);

// Placeholder for non-x86_64 architectures
#[cfg(not(target_arch = "x86_64"))]
criterion_group!(benches,);

criterion_main!(benches);
