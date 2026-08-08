//! Micro-benchmark for `BalancedParens::rank1` performance.
//!
//! Baseline for #596 (F1: drop BP's `rank_l2`). `rank1` is the crate's
//! hottest path (hit on every cursor navigation) and currently answers in
//! O(1) via a packed 9-bit-per-word `rank_l2` lookup; #596 proposes
//! replacing that lookup with popcounting up to 7 words within a 512-bit
//! block. This benchmark exists so that trade can be measured against a
//! dedicated baseline instead of noisy end-to-end `yaml_bench`/`yq` numbers
//! (`benches/bp_select_micro.rs` already covers the select-side half).
//!
//! Covers three tree shapes (block-density patterns) at four sizes each:
//! - `flat`: a wide array of leaves (`(()()()...)`) — opens and closes
//!   alternate every bit, the common shape for JSON/YAML arrays of scalars.
//! - `deep`: a single deeply nested chain (`((((...))))`) — long runs of
//!   opens then closes, the common shape for deeply nested objects.
//! - `mixed`: randomly depth-biased nesting (same generator as
//!   `bp_select_micro.rs`) — representative of typical real-world documents
//!   mixing objects and arrays.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::hint::black_box;
use succinctly::trees::BalancedParens;

/// Wide array of leaves: root open, `num_opens - 1` `()` leaf pairs, root close.
fn generate_bp_flat(num_opens: usize) -> Vec<u64> {
    let total_bits = num_opens * 2;
    let word_count = total_bits.div_ceil(64);
    let mut words = vec![0u64; word_count];

    words[0] |= 1; // root open at bit 0
    for i in 0..num_opens - 1 {
        let bit_pos = 1 + i * 2; // leaf open; leaf close is the implicit 0 bit after it
        words[bit_pos / 64] |= 1 << (bit_pos % 64);
    }
    // Root close is the implicit 0 bit at the final position.

    words
}

/// Deeply nested chain: `num_opens` opens followed by `num_opens` closes.
fn generate_bp_deep(num_opens: usize) -> Vec<u64> {
    let total_bits = num_opens * 2;
    let word_count = total_bits.div_ceil(64);
    let mut words = vec![0u64; word_count];

    for bit_pos in 0..num_opens {
        words[bit_pos / 64] |= 1 << (bit_pos % 64);
    }

    words
}

/// Randomly depth-biased nesting (mirrors `bp_select_micro.rs`'s generator).
fn generate_bp_mixed(num_opens: usize, seed: u64) -> Vec<u64> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let total_bits = num_opens * 2;
    let word_count = total_bits.div_ceil(64);
    let mut words = vec![0u64; word_count];

    let mut depth = 0;
    let mut opens_remaining = num_opens;
    let mut closes_remaining = num_opens;

    for bit_pos in 0..total_bits {
        let word_idx = bit_pos / 64;
        let bit_idx = bit_pos % 64;

        let can_open = opens_remaining > 0;
        let can_close = depth > 0 && closes_remaining > 0;

        let is_open = if can_open && can_close {
            rng.random_bool(0.5 + 0.1 * (1.0 - depth as f64 / num_opens as f64).max(0.0))
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

fn bench_rank1(c: &mut Criterion) {
    let mut group = c.benchmark_group("bp_rank1");

    for num_opens in [1_000, 10_000, 100_000, 1_000_000] {
        let len = num_opens * 2;
        let mut rng = ChaCha8Rng::seed_from_u64(123);
        let queries: Vec<usize> = (0..10000).map(|_| rng.random_range(0..len)).collect();

        let shapes: [(&str, Vec<u64>); 3] = [
            ("flat", generate_bp_flat(num_opens)),
            ("deep", generate_bp_deep(num_opens)),
            ("mixed", generate_bp_mixed(num_opens, 42)),
        ];

        for (name, words) in shapes {
            let bp = BalancedParens::new(words, len);

            group.bench_with_input(
                BenchmarkId::new(name, format!("{}k", num_opens / 1000)),
                &(&bp, &queries),
                |b, (bp, queries)| {
                    b.iter(|| {
                        let mut sum = 0usize;
                        for &q in *queries {
                            sum += bp.rank1(black_box(q));
                        }
                        sum
                    });
                },
            );
        }
    }

    group.finish();
}

criterion_group!(benches, bench_rank1);
criterion_main!(benches);
