//! Quick comparison benchmark for AVX-512 vs AVX2 vs SSE2 JSON parsing.
//!
//! This benchmark uses synthetic JSON data to quickly measure the
//! performance difference between SIMD implementations.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

/// Generate synthetic JSON of a given size.
fn generate_json(size_bytes: usize) -> Vec<u8> {
    let mut json = Vec::new();
    json.push(b'[');

    let item = br#"{"id":12345,"name":"Alice Johnson","email":"alice@example.com","active":true},"#;
    let item_size = item.len();
    let count = (size_bytes / item_size).max(1);

    for i in 0..count {
        if i > 0 {
            json.push(b',');
        }
        json.extend_from_slice(item);
    }

    // Remove trailing comma
    if json.last() == Some(&b',') {
        json.pop();
    }

    json.push(b']');
    json
}

#[cfg(target_arch = "x86_64")]
fn bench_simd_levels(c: &mut Criterion) {
    let sizes = [
        ("1KB", 1024),
        ("10KB", 10 * 1024),
        ("100KB", 100 * 1024),
        ("1MB", 1024 * 1024),
    ];

    for (size_name, size_bytes) in sizes {
        let json = generate_json(size_bytes);
        let actual_size = json.len() as u64;

        let mut group = c.benchmark_group(format!("json_parse/{}", size_name));
        group.throughput(Throughput::Bytes(actual_size));
        group.sample_size(50); // Faster benchmarking

        // AVX-512 (if available)
        if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512bw") {
            group.bench_function("AVX512", |b| {
                b.iter(|| {
                    succinctly::json::simd::avx512::build_semi_index_standard(black_box(&json))
                })
            });
        }

        // AVX2 (if available)
        if is_x86_feature_detected!("avx2") {
            group.bench_function("AVX2", |b| {
                b.iter(|| {
                    succinctly::json::simd::avx2::build_semi_index_standard(black_box(&json))
                })
            });
        }

        // SSE4.2 (if available)
        if is_x86_feature_detected!("sse4.2") {
            group.bench_function("SSE4.2", |b| {
                b.iter(|| {
                    succinctly::json::simd::sse42::build_semi_index_standard(black_box(&json))
                })
            });
        }

        // SSE2 (always available)
        group.bench_function("SSE2", |b| {
            b.iter(|| {
                succinctly::json::simd::x86::build_semi_index_standard(black_box(&json))
            })
        });

        // Scalar baseline
        group.bench_function("Scalar", |b| {
            b.iter(|| succinctly::json::standard::build_semi_index(black_box(&json)))
        });

        group.finish();
    }
}

/// Benchmark alignment effects for AVX-512 (64-byte boundaries).
#[cfg(target_arch = "x86_64")]
fn bench_avx512_alignment(c: &mut Criterion) {
    if !is_x86_feature_detected!("avx512f") || !is_x86_feature_detected!("avx512bw") {
        return;
    }

    let test_cases = [
        ("aligned_64B", 64),
        ("aligned_128B", 128),
        ("aligned_256B", 256),
        ("aligned_512B", 512),
        ("aligned_1KB", 1024),
        ("unaligned_63B", 63),
        ("unaligned_65B", 65),
        ("unaligned_127B", 127),
        ("unaligned_129B", 129),
    ];

    let mut group = c.benchmark_group("json_avx512_alignment");

    for (name, size) in test_cases {
        let json = generate_json(size);
        let actual_size = json.len() as u64;
        group.throughput(Throughput::Bytes(actual_size));

        group.bench_function(name, |b| {
            b.iter(|| {
                succinctly::json::simd::avx512::build_semi_index_standard(black_box(&json))
            })
        });
    }

    group.finish();
}

/// Benchmark different JSON patterns.
#[cfg(target_arch = "x86_64")]
fn bench_json_patterns(c: &mut Criterion) {
    // Different JSON structures
    let patterns = [
        ("flat_object", br#"{"a":1,"b":2,"c":3,"d":4,"e":5,"f":6,"g":7,"h":8,"i":9,"j":10}"#.to_vec()),
        ("nested_objects", br#"{"a":{"b":{"c":{"d":{"e":{"f":{"g":{"h":{"i":{"j":1}}}}}}}}}}"#.to_vec()),
        ("array_numbers", br#"[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20]"#.to_vec()),
        ("array_strings", br#"["a","b","c","d","e","f","g","h","i","j","k","l","m","n","o","p"]"#.to_vec()),
        ("mixed", br#"{"nums":[1,2,3],"strs":["a","b"],"bool":true,"null":null}"#.to_vec()),
    ];

    for (pattern_name, json) in patterns {
        // Repeat pattern to get ~10KB
        let mut large_json = Vec::new();
        large_json.push(b'[');
        let target_size = 10 * 1024;
        while large_json.len() < target_size {
            if large_json.len() > 1 {
                large_json.push(b',');
            }
            large_json.extend_from_slice(&json);
        }
        large_json.push(b']');

        let actual_size = large_json.len() as u64;
        let mut group = c.benchmark_group(format!("json_pattern/{}", pattern_name));
        group.throughput(Throughput::Bytes(actual_size));

        // AVX-512
        if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512bw") {
            group.bench_function("AVX512", |b| {
                b.iter(|| {
                    succinctly::json::simd::avx512::build_semi_index_standard(black_box(&large_json))
                })
            });
        }

        // AVX2
        if is_x86_feature_detected!("avx2") {
            group.bench_function("AVX2", |b| {
                b.iter(|| {
                    succinctly::json::simd::avx2::build_semi_index_standard(black_box(&large_json))
                })
            });
        }

        group.finish();
    }
}

#[cfg(target_arch = "x86_64")]
criterion_group!(
    benches,
    bench_simd_levels,
    bench_avx512_alignment,
    bench_json_patterns
);

#[cfg(not(target_arch = "x86_64"))]
criterion_group!(benches,);

criterion_main!(benches);
