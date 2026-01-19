//! Detailed popcount strategy benchmarks.
//!
//! Compares performance across different popcount implementations:
//! - Default: Rust's count_ones() (auto-vectorizes)
//! - SIMD: Explicit SIMD intrinsics with AVX512-VPOPCNTDQ dispatch
//! - Portable: Pure bitwise algorithm
//!
//! Run with:
//! - `cargo bench --bench popcount_strategies --features simd`

#[cfg(feature = "simd")]
use criterion::black_box;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

#[cfg(feature = "simd")]
use succinctly::popcount_words;

fn bench_popcount_strategies(c: &mut Criterion) {
    let mut group = c.benchmark_group("popcount_strategies");

    // Test various data sizes (in words)
    let sizes = [
        ("64B", 8),        // 8 words = 64 bytes (fits in cache)
        ("512B", 64),      // 64 words = 512 bytes
        ("4KB", 512),      // 512 words = 4KB (L1 cache)
        ("32KB", 4096),    // 4KB = 32KB (typical L1 cache)
        ("256KB", 32768),  // 256KB (L2 cache)
        ("1MB", 131072),   // 1MB (L3 cache)
        ("10MB", 1310720), // 10MB (exceeds cache)
    ];

    // Different data patterns affect popcount performance
    type PatternFn = fn(u64) -> u64;
    let patterns: [(&str, PatternFn); 6] = [
        ("zeros", |_| 0u64),
        ("ones", |_| u64::MAX),
        ("sparse", |i| i & 0x0001_0001_0001_0001),
        ("dense", |i| i | 0xF0F0_F0F0_F0F0_F0F0),
        ("alternating", |i| {
            if i % 2 == 0 {
                0xAAAA_AAAA_AAAA_AAAA
            } else {
                0x5555_5555_5555_5555
            }
        }),
        ("random", |i| i.wrapping_mul(0x1234_5678_9ABC_DEF0)),
    ];

    for (size_name, word_count) in sizes {
        for (pattern_name, pattern_fn) in &patterns {
            let words: Vec<u64> = (0u64..word_count as u64).map(pattern_fn).collect();

            #[cfg(feature = "simd")]
            group.bench_with_input(
                BenchmarkId::new(format!("{}/{}", size_name, pattern_name), "simd"),
                &words,
                |b, words| b.iter(|| popcount_words(black_box(words))),
            );

            // Always benchmark scalar as baseline
            group.bench_with_input(
                BenchmarkId::new(format!("{}/{}", size_name, pattern_name), "scalar"),
                &words,
                |b, words| {
                    b.iter(|| {
                        let mut total = 0u32;
                        for &word in words {
                            total += word.count_ones();
                        }
                        total
                    })
                },
            );
        }
    }

    group.finish();
}

/// Benchmark specifically targeting AVX512-VPOPCNTDQ performance.
///
/// Tests the critical 8-word alignment case where AVX512 excels.
#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn bench_avx512_alignment(c: &mut Criterion) {
    let mut group = c.benchmark_group("avx512_alignment");

    // Only run if AVX512-VPOPCNTDQ is available
    if !is_x86_feature_detected!("avx512vpopcntdq") {
        eprintln!("Skipping AVX512 benchmarks: CPU doesn't support AVX512-VPOPCNTDQ");
        group.finish();
        return;
    }

    // Test alignment-specific cases
    let test_cases = [
        ("aligned_8", 8),     // Exactly 8 words - one AVX512 load
        ("aligned_16", 16),   // 16 words - two AVX512 loads
        ("aligned_64", 64),   // 64 words - eight AVX512 loads
        ("aligned_512", 512), // 512 words - 64 AVX512 loads
        ("unaligned_7", 7),   // 7 words - no AVX512, fallback
        ("unaligned_9", 9),   // 9 words - one AVX512 + 1 scalar
        ("unaligned_15", 15), // 15 words - one AVX512 + 7 scalar
        ("unaligned_17", 17), // 17 words - two AVX512 + 1 scalar
    ];

    for (name, count) in test_cases {
        let words: Vec<u64> = (0u64..count)
            .map(|i| i.wrapping_mul(0x1234_5678_9ABC_DEF0))
            .collect();

        group.bench_with_input(BenchmarkId::new(name, ""), &words, |b, words| {
            b.iter(|| popcount_words(black_box(words)))
        });
    }

    group.finish();
}

/// Benchmark throughput in GB/s.
///
/// Helps understand real-world performance: "How fast can we process data?"
fn bench_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput");
    group.throughput(criterion::Throughput::Bytes(1024 * 1024)); // 1MB in bytes

    // 1MB of data = 131,072 u64 words
    let words: Vec<u64> = (0u64..131072)
        .map(|i| i.wrapping_mul(0x1234_5678_9ABC_DEF0))
        .collect();

    #[cfg(feature = "simd")]
    group.bench_function("simd_1MB", |b| b.iter(|| popcount_words(black_box(&words))));

    group.bench_function("scalar_1MB", |b| {
        b.iter(|| {
            let mut total = 0u32;
            for word in &words {
                total += word.count_ones();
            }
            total
        })
    });

    group.finish();
}

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
criterion_group!(
    benches,
    bench_popcount_strategies,
    bench_avx512_alignment,
    bench_throughput
);

#[cfg(not(all(feature = "simd", target_arch = "x86_64")))]
criterion_group!(benches, bench_popcount_strategies, bench_throughput);

criterion_main!(benches);
