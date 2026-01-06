//! Benchmark comparing different NEON movemask implementations.
//!
//! This benchmark measures the performance difference between:
//! - Serial loop implementation (naive)
//! - Parallel shift + horizontal add (optimized)
//! - Alternative approaches (SHRN-based, etc.)

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

#[cfg(target_arch = "aarch64")]
mod aarch64_benches {
    use super::*;
    use core::arch::aarch64::*;

    /// Expose the internal movemask functions for benchmarking
    mod movemask_impls {
        use core::arch::aarch64::*;

        /// Serial movemask - the naive approach
        #[inline]
        #[target_feature(enable = "neon")]
        pub unsafe fn serial(v: uint8x16_t) -> u16 {
            let mut mask: u16 = 0;
            let arr: [u8; 16] = unsafe { core::mem::transmute(v) };
            for (i, &byte) in arr.iter().enumerate() {
                if byte & 0x80 != 0 {
                    mask |= 1 << i;
                }
            }
            mask
        }

        /// Parallel movemask - optimized with weighted shifts
        #[inline]
        #[target_feature(enable = "neon")]
        pub unsafe fn parallel(v: uint8x16_t) -> u16 {
            unsafe {
                // Shift each byte right by 7 to get just the high bit (0 or 1)
                let high_bits = vshrq_n_u8::<7>(v);

                // Create shift amounts: [0,1,2,3,4,5,6,7, 0,1,2,3,4,5,6,7]
                let shift_amounts: [i8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 0, 1, 2, 3, 4, 5, 6, 7];
                let shifts = vld1q_s8(shift_amounts.as_ptr());

                // Shift each byte left by its lane index (multiply by 2^i)
                let shifted = vshlq_u8(high_bits, shifts);

                // Split into low and high halves
                let low = vget_low_u8(shifted);
                let high = vget_high_u8(shifted);

                // Horizontal add within each half to get a single byte
                let low_sum = vaddv_u8(low) as u16;
                let high_sum = vaddv_u8(high) as u16;

                low_sum | (high_sum << 8)
            }
        }

        /// Alternative using multiplication instead of variable shifts
        #[inline]
        #[target_feature(enable = "neon")]
        pub unsafe fn parallel_mul(v: uint8x16_t) -> u16 {
            unsafe {
                // Shift each byte right by 7 to get just the high bit (0 or 1)
                let high_bits = vshrq_n_u8::<7>(v);

                // Split into halves
                let lo = vget_low_u8(high_bits);
                let hi = vget_high_u8(high_bits);

                // Weight by position using multiplication
                let weights: [u8; 8] = [1, 2, 4, 8, 16, 32, 64, 128];
                let w = vld1_u8(weights.as_ptr());

                let weighted_lo = vmul_u8(lo, w);
                let weighted_hi = vmul_u8(hi, w);

                let sum_lo = vaddv_u8(weighted_lo) as u16;
                let sum_hi = vaddv_u8(weighted_hi) as u16;

                sum_lo | (sum_hi << 8)
            }
        }

        /// Using pairwise addition reduction
        #[inline]
        #[target_feature(enable = "neon")]
        pub unsafe fn parallel_padd(v: uint8x16_t) -> u16 {
            unsafe {
                // Shift each byte right by 7 to get just the high bit (0 or 1)
                let high_bits = vshrq_n_u8::<7>(v);

                // Create shift amounts
                let shift_amounts: [i8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 0, 1, 2, 3, 4, 5, 6, 7];
                let shifts = vld1q_s8(shift_amounts.as_ptr());

                // Shift each byte left by its lane index
                let shifted = vshlq_u8(high_bits, shifts);

                // Use pairwise addition to reduce
                // After first padd: 8 bytes, each is sum of 2 adjacent bytes
                let p1 = vpaddq_u8(shifted, shifted);
                // After second padd: 4 values of interest
                let p2 = vpaddq_u8(p1, p1);
                // After third padd: 2 values of interest
                let p3 = vpaddq_u8(p2, p2);

                // Extract the two bytes we need
                let arr: [u8; 16] = core::mem::transmute(p3);
                (arr[0] as u16) | ((arr[1] as u16) << 8)
            }
        }
    }

    pub fn bench_movemask_variants(c: &mut Criterion) {
        let mut group = c.benchmark_group("neon_movemask");

        // Test patterns with different bit densities
        let patterns: &[(&str, [u8; 16])] = &[
            ("all_zeros", [0u8; 16]),
            ("all_high", [0x80; 16]),
            ("all_ff", [0xFF; 16]),
            (
                "alternating",
                [
                    0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80,
                    0x00, 0x80, 0x00,
                ],
            ),
            (
                "sparse",
                [
                    0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x80,
                ],
            ),
            (
                "dense",
                [
                    0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00,
                    0x00, 0x00, 0x00,
                ],
            ),
            (
                "random",
                [
                    0x12, 0x83, 0x45, 0xC7, 0x89, 0xAB, 0xCD, 0xEF, 0x21, 0x43, 0x65, 0x87, 0xA9,
                    0xCB, 0xED, 0x0F,
                ],
            ),
        ];

        for (name, pattern) in patterns {
            unsafe {
                let v = vld1q_u8(pattern.as_ptr());

                group.bench_function(BenchmarkId::new("serial", name), |b| {
                    b.iter(|| movemask_impls::serial(black_box(v)))
                });

                group.bench_function(BenchmarkId::new("parallel", name), |b| {
                    b.iter(|| movemask_impls::parallel(black_box(v)))
                });

                group.bench_function(BenchmarkId::new("parallel_mul", name), |b| {
                    b.iter(|| movemask_impls::parallel_mul(black_box(v)))
                });

                group.bench_function(BenchmarkId::new("parallel_padd", name), |b| {
                    b.iter(|| movemask_impls::parallel_padd(black_box(v)))
                });
            }
        }

        group.finish();
    }

    /// Benchmark the full semi-index building with different movemask implementations
    pub fn bench_json_indexing_movemask(c: &mut Criterion) {
        let test_files = [
            ("10mb", "data/bench/generated/comprehensive/10mb.json"),
            ("100mb", "data/bench/generated/comprehensive/100mb.json"),
        ];

        for (name, path) in test_files {
            let test_file = std::path::Path::new(path);
            if !test_file.exists() {
                eprintln!(
                    "Skipping {} JSON indexing benchmark: {} not found",
                    name,
                    test_file.display()
                );
                eprintln!("Run: ./target/release/succinctly json generate-suite");
                continue;
            }

            let bytes = std::fs::read(test_file).expect("Failed to read test file");
            let file_size = bytes.len() as u64;

            let mut group = c.benchmark_group(format!("json_indexing_{}", name));
            group.throughput(Throughput::Bytes(file_size));
            group.sample_size(if name == "100mb" { 10 } else { 20 });

            // Benchmark with NEON (uses parallel movemask)
            group.bench_function("NEON_parallel", |b| {
                b.iter(|| {
                    succinctly::json::simd::neon::build_semi_index_standard(black_box(&bytes))
                })
            });

            // Benchmark with scalar (baseline)
            group.bench_function("Scalar", |b| {
                b.iter(|| succinctly::json::standard::build_semi_index(black_box(&bytes)))
            });

            group.finish();
        }
    }

    /// Correctness test - ensure all implementations produce the same result
    pub fn verify_correctness() {
        let patterns: &[[u8; 16]] = &[
            [0u8; 16],
            [0x80; 16],
            [0xFF; 16],
            [
                0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00,
                0x80, 0x00,
            ],
            [
                0x12, 0x83, 0x45, 0xC7, 0x89, 0xAB, 0xCD, 0xEF, 0x21, 0x43, 0x65, 0x87, 0xA9, 0xCB,
                0xED, 0x0F,
            ],
        ];

        for pattern in patterns {
            unsafe {
                let v = vld1q_u8(pattern.as_ptr());

                let serial = movemask_impls::serial(v);
                let parallel = movemask_impls::parallel(v);
                let parallel_mul = movemask_impls::parallel_mul(v);
                let parallel_padd = movemask_impls::parallel_padd(v);

                assert_eq!(
                    serial, parallel,
                    "Mismatch: serial={:016b}, parallel={:016b}",
                    serial, parallel
                );
                assert_eq!(
                    serial, parallel_mul,
                    "Mismatch: serial={:016b}, parallel_mul={:016b}",
                    serial, parallel_mul
                );
                assert_eq!(
                    serial, parallel_padd,
                    "Mismatch: serial={:016b}, parallel_padd={:016b}",
                    serial, parallel_padd
                );
            }
        }
        println!("All movemask implementations produce identical results ✓");
    }
}

#[cfg(target_arch = "aarch64")]
fn bench_movemask(c: &mut Criterion) {
    // First verify correctness
    aarch64_benches::verify_correctness();

    // Then run benchmarks
    aarch64_benches::bench_movemask_variants(c);
    aarch64_benches::bench_json_indexing_movemask(c);
}

#[cfg(not(target_arch = "aarch64"))]
fn bench_movemask(_c: &mut Criterion) {
    eprintln!("NEON movemask benchmark only available on aarch64");
}

criterion_group!(benches, bench_movemask);
criterion_main!(benches);
