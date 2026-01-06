//! Criterion benchmarks for balanced parentheses operations.
//!
//! Compares:
//! - Linear scan (Phase 1) vs RangeMin (Phase 2)
//! - Various tree depths and sizes
//! - find_close, find_open, enclose operations

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use succinctly::bp::{self, BalancedParens};

/// Generate a balanced parentheses sequence with specified properties.
///
/// Creates a random tree structure with approximately `node_count` nodes.
/// The `max_depth` parameter controls how deep the tree can get.
fn generate_balanced_parens(node_count: usize, max_depth: usize, seed: u64) -> (Vec<u64>, usize) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut bits = Vec::with_capacity(node_count * 2);
    let mut depth = 0;

    // Generate balanced sequence
    while bits.len() < node_count * 2 {
        if depth == 0 {
            // Must open
            bits.push(true);
            depth += 1;
        } else if depth >= max_depth {
            // Must close
            bits.push(false);
            depth -= 1;
        } else {
            // Random choice, biased towards opening to create varied structures
            if rng.gen_bool(0.55) {
                bits.push(true);
                depth += 1;
            } else {
                bits.push(false);
                depth -= 1;
            }
        }
    }

    // Close any remaining open parens
    while depth > 0 {
        bits.push(false);
        depth -= 1;
    }

    let len = bits.len();

    // Pack into words
    let word_count = len.div_ceil(64);
    let mut words = vec![0u64; word_count];
    for (i, &bit) in bits.iter().enumerate() {
        if bit {
            words[i / 64] |= 1 << (i % 64);
        }
    }

    (words, len)
}

/// Generate a deeply nested structure: (((((...)))))
fn generate_deep_nested(depth: usize) -> (Vec<u64>, usize) {
    let len = depth * 2;
    let word_count = len.div_ceil(64);
    let mut words = vec![0u64; word_count];

    // First `depth` bits are opens (1s)
    for i in 0..depth {
        words[i / 64] |= 1 << (i % 64);
    }
    // Remaining `depth` bits are closes (0s) - already 0

    (words, len)
}

/// Generate a flat sequence: ()()()()...
fn generate_flat_siblings(count: usize) -> (Vec<u64>, usize) {
    let len = count * 2;
    let word_count = len.div_ceil(64);
    let mut words = vec![0u64; word_count];

    // Pattern: 1,0,1,0,1,0,... (open, close, open, close, ...)
    for i in 0..count {
        let bit_pos = i * 2;
        words[bit_pos / 64] |= 1 << (bit_pos % 64);
    }

    (words, len)
}

/// Generate random open positions for queries.
fn generate_open_queries(bp: &BalancedParens, count: usize, seed: u64) -> Vec<usize> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut queries = Vec::with_capacity(count);
    let len = bp.len();

    while queries.len() < count {
        let pos = rng.gen_range(0..len);
        if bp.is_open(pos) {
            queries.push(pos);
        }
    }

    queries
}

/// Generate random close positions for queries.
fn generate_close_queries(bp: &BalancedParens, count: usize, seed: u64) -> Vec<usize> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut queries = Vec::with_capacity(count);
    let len = bp.len();

    while queries.len() < count {
        let pos = rng.gen_range(0..len);
        if bp.is_close(pos) {
            queries.push(pos);
        }
    }

    queries
}

fn bench_find_close(c: &mut Criterion) {
    let mut group = c.benchmark_group("bp/find_close");

    // Test different tree sizes and structures
    let configs = [
        ("10K_random", 10_000usize, 100usize),
        ("100K_random", 100_000, 200),
        ("1M_random", 1_000_000, 500),
    ];

    for (name, nodes, max_depth) in configs {
        let (words, len) = generate_balanced_parens(nodes, max_depth, 42);
        let bp = BalancedParens::new(words.clone(), len);
        let queries = generate_open_queries(&bp, 1000, 123);

        // Benchmark RangeMin (Phase 2)
        group.bench_with_input(
            BenchmarkId::new("rangemin", name),
            &(&bp, &queries),
            |b, (bp, queries)| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &q in queries.iter() {
                        if let Some(pos) = bp.find_close(black_box(q)) {
                            sum += pos;
                        }
                    }
                    sum
                })
            },
        );

        // Benchmark linear scan (Phase 1)
        group.bench_with_input(
            BenchmarkId::new("linear", name),
            &(&words, len, &queries),
            |b, (words, len, queries)| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &q in queries.iter() {
                        if let Some(pos) = bp::find_close(words, *len, black_box(q)) {
                            sum += pos;
                        }
                    }
                    sum
                })
            },
        );
    }

    group.finish();
}

fn bench_find_open(c: &mut Criterion) {
    let mut group = c.benchmark_group("bp/find_open");

    let configs = [
        ("10K_random", 10_000usize, 100usize),
        ("100K_random", 100_000, 200),
        ("1M_random", 1_000_000, 500),
    ];

    for (name, nodes, max_depth) in configs {
        let (words, len) = generate_balanced_parens(nodes, max_depth, 42);
        let bp = BalancedParens::new(words.clone(), len);
        let queries = generate_close_queries(&bp, 1000, 123);

        // Benchmark RangeMin (Phase 2)
        group.bench_with_input(
            BenchmarkId::new("rangemin", name),
            &(&bp, &queries),
            |b, (bp, queries)| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &q in queries.iter() {
                        if let Some(pos) = bp.find_open(black_box(q)) {
                            sum += pos;
                        }
                    }
                    sum
                })
            },
        );

        // Benchmark linear scan (Phase 1)
        group.bench_with_input(
            BenchmarkId::new("linear", name),
            &(&words, len, &queries),
            |b, (words, len, queries)| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &q in queries.iter() {
                        if let Some(pos) = bp::find_open(words, *len, black_box(q)) {
                            sum += pos;
                        }
                    }
                    sum
                })
            },
        );
    }

    group.finish();
}

fn bench_enclose(c: &mut Criterion) {
    let mut group = c.benchmark_group("bp/enclose");

    let configs = [
        ("10K_random", 10_000usize, 100usize),
        ("100K_random", 100_000, 200),
    ];

    for (name, nodes, max_depth) in configs {
        let (words, len) = generate_balanced_parens(nodes, max_depth, 42);
        let bp = BalancedParens::new(words.clone(), len);
        // Query positions that have a parent (not at root level)
        let queries = generate_open_queries(&bp, 1000, 456);

        // Benchmark RangeMin (Phase 2)
        group.bench_with_input(
            BenchmarkId::new("rangemin", name),
            &(&bp, &queries),
            |b, (bp, queries)| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &q in queries.iter() {
                        if let Some(pos) = bp.enclose(black_box(q)) {
                            sum += pos;
                        }
                    }
                    sum
                })
            },
        );

        // Benchmark linear scan (Phase 1)
        group.bench_with_input(
            BenchmarkId::new("linear", name),
            &(&words, len, &queries),
            |b, (words, len, queries)| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &q in queries.iter() {
                        if let Some(pos) = bp::enclose(words, *len, black_box(q)) {
                            sum += pos;
                        }
                    }
                    sum
                })
            },
        );
    }

    group.finish();
}

fn bench_tree_structures(c: &mut Criterion) {
    let mut group = c.benchmark_group("bp/tree_structure");

    // Deep nested: (((((...))))) - worst case for matching far away
    let (deep_words, deep_len) = generate_deep_nested(10_000);
    let deep_bp = BalancedParens::new(deep_words.clone(), deep_len);

    // Flat siblings: ()()()... - best case for matching nearby
    let (flat_words, flat_len) = generate_flat_siblings(10_000);
    let flat_bp = BalancedParens::new(flat_words.clone(), flat_len);

    // Query from root in deep structure
    group.bench_function("deep_10K/rangemin", |b| {
        b.iter(|| deep_bp.find_close(black_box(0)))
    });

    group.bench_function("deep_10K/linear", |b| {
        b.iter(|| bp::find_close(&deep_words, deep_len, black_box(0)))
    });

    // Query first pair in flat structure
    group.bench_function("flat_10K/rangemin", |b| {
        b.iter(|| flat_bp.find_close(black_box(0)))
    });

    group.bench_function("flat_10K/linear", |b| {
        b.iter(|| bp::find_close(&flat_words, flat_len, black_box(0)))
    });

    group.finish();
}

fn bench_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("bp/construction");

    for nodes in [10_000usize, 100_000, 1_000_000] {
        let (words, len) = generate_balanced_parens(nodes, 200, 42);

        group.bench_with_input(
            BenchmarkId::new("rangemin", format!("{}K", nodes / 1000)),
            &(words, len),
            |b, (words, len)| b.iter(|| BalancedParens::new(black_box(words.clone()), *len)),
        );
    }

    group.finish();
}

fn bench_navigation(c: &mut Criterion) {
    let mut group = c.benchmark_group("bp/navigation");

    let (words, len) = generate_balanced_parens(100_000, 200, 42);
    let bp = BalancedParens::new(words, len);

    // Traverse tree depth-first from root
    group.bench_function("depth_first_100K", |b| {
        b.iter(|| {
            let mut count = 0usize;
            let mut stack = vec![0usize]; // Start at root

            while let Some(pos) = stack.pop() {
                count += 1;

                // Try next sibling first (push onto stack)
                if let Some(sib) = bp.next_sibling(pos) {
                    stack.push(sib);
                }

                // Then first child (process next)
                if let Some(child) = bp.first_child(pos) {
                    stack.push(child);
                }

                // Limit iterations for benchmark stability
                if count > 10_000 {
                    break;
                }
            }

            count
        })
    });

    // Get excess at various positions
    let queries = generate_open_queries(&bp, 1000, 789);
    group.bench_function("excess_1K_queries", |b| {
        b.iter(|| {
            let mut sum = 0i32;
            for &q in queries.iter() {
                sum += bp.excess(black_box(q));
            }
            sum
        })
    });

    group.finish();
}

/// Benchmark rank1 performance.
fn bench_rank1(c: &mut Criterion) {
    let mut group = c.benchmark_group("bp/rank1");

    let configs = [
        ("10K_random", 10_000usize, 100usize),
        ("100K_random", 100_000, 200),
        ("1M_random", 1_000_000, 500),
    ];

    for (name, nodes, max_depth) in configs {
        let (words, len) = generate_balanced_parens(nodes, max_depth, 42);
        let bp = BalancedParens::new(words, len);

        // Generate random query positions
        let queries = generate_open_queries(&bp, 1000, 123);

        group.bench_with_input(
            BenchmarkId::new("rank1", name),
            &(&bp, &queries),
            |b, (bp, queries)| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &q in queries.iter() {
                        sum += bp.rank1(black_box(q));
                    }
                    sum
                })
            },
        );
    }

    group.finish();
}

/// Benchmark excess performance.
fn bench_excess(c: &mut Criterion) {
    let mut group = c.benchmark_group("bp/excess");

    let (words, len) = generate_balanced_parens(100_000, 200, 42);
    let bp = BalancedParens::new(words, len);

    let queries = generate_open_queries(&bp, 1000, 789);

    group.bench_function("excess_100K", |b| {
        b.iter(|| {
            let mut sum = 0i32;
            for &q in queries.iter() {
                sum += bp.excess(black_box(q));
            }
            sum
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_find_close,
    bench_find_open,
    bench_enclose,
    bench_tree_structures,
    bench_construction,
    bench_navigation,
    bench_rank1,
    bench_excess,
);
criterion_main!(benches);
