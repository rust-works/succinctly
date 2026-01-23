//! Micro-benchmark for BP select1 performance.
//!
//! Compares:
//! 1. O(1) select1 using WithSelect (new)
//! 2. O(log n) binary search on rank1 (old approach)

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use succinctly::trees::{BalancedParens, WithSelect};

/// Generate a balanced parentheses sequence of given depth.
fn generate_bp(num_opens: usize, seed: u64) -> Vec<u64> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let total_bits = num_opens * 2;
    let word_count = total_bits.div_ceil(64);
    let mut words = vec![0u64; word_count];

    // Generate valid balanced parentheses
    let mut depth = 0;
    let mut opens_remaining = num_opens;
    let mut closes_remaining = num_opens;

    for bit_pos in 0..total_bits {
        let word_idx = bit_pos / 64;
        let bit_idx = bit_pos % 64;

        // Decide: open (1) or close (0)
        let can_open = opens_remaining > 0;
        let can_close = depth > 0 && closes_remaining > 0;

        let is_open = if can_open && can_close {
            // Random choice, but bias toward opens if depth is low
            rng.gen_bool(0.5 + 0.1 * (1.0 - depth as f64 / num_opens as f64).max(0.0))
        } else {
            can_open
        };

        if is_open {
            words[word_idx] |= 1 << bit_idx;
            opens_remaining -= 1;
            depth += 1;
        } else {
            closes_remaining -= 1;
            depth -= 1;
        }
    }

    words
}

fn bench_select1_with_select(c: &mut Criterion) {
    let mut group = c.benchmark_group("bp_select1");

    for num_opens in [1_000, 10_000, 100_000, 1_000_000] {
        let words = generate_bp(num_opens, 42);
        let len = num_opens * 2;
        let bp: BalancedParens<Vec<u64>, WithSelect> = BalancedParens::new_with_select(words, len);

        // Generate random queries
        let mut rng = ChaCha8Rng::seed_from_u64(123);
        let queries: Vec<usize> = (0..10000).map(|_| rng.gen_range(0..num_opens)).collect();

        group.bench_with_input(
            BenchmarkId::new("select1", format!("{}k", num_opens / 1000)),
            &(&bp, &queries),
            |b, (bp, queries)| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &q in queries.iter() {
                        if let Some(pos) = bp.select1(black_box(q)) {
                            sum += pos;
                        }
                    }
                    sum
                })
            },
        );

        // Also benchmark the old approach: binary search on rank1
        group.bench_with_input(
            BenchmarkId::new("binary_search_rank1", format!("{}k", num_opens / 1000)),
            &(&bp, &queries),
            |b, (bp, queries)| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &q in queries.iter() {
                        let target_rank = black_box(q) + 1;
                        let bp_len = bp.len();

                        // Binary search to find bp_pos where rank1(bp_pos + 1) >= target_rank
                        let mut lo = 0;
                        let mut hi = bp_len;
                        while lo < hi {
                            let mid = lo + (hi - lo) / 2;
                            if bp.rank1(mid + 1) < target_rank {
                                lo = mid + 1;
                            } else {
                                hi = mid;
                            }
                        }

                        if lo < bp_len && bp.is_open(lo) {
                            sum += lo;
                        }
                    }
                    sum
                })
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_select1_with_select);
criterion_main!(benches);
