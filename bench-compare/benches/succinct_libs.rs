//! Benchmark comparing succinctly's `BitVec` against other Rust succinct-structure crates.
//!
//! Compares rank/select structures that carry both a rank index and a select index:
//!
//! - **succinctly**: `BitVec` (3-level `RankDirectory` + sampled `SelectIndex`)
//! - **vers-vecs**: `RsVec`
//! - **sucds**: `Rank9Sel`
//! - **sux**: `SelectAdapt<Rank9<AddNumBits<BitVec>>>`
//!
//! Every structure is built from the same seeded random words, so all four see identical
//! bits. Each builder takes `&[u64]` and returns a fully owned structure, which keeps the
//! construction and memory comparison apples-to-apples: no crate is charged for, or
//! credited with, borrowing the caller's buffer.
//!
//! Note that both succinctly (`BitVec::from_words`) and vers-vecs (`BitVec::from_vec`)
//! additionally accept an owned `Vec<u64>` by move, which is free. sucds and sux have no
//! bulk-word constructor and must be filled per bit; that cost is real and is what the
//! `construction` group measures for them.
//!
//! Run from the bench-compare directory:
//! ```bash
//! cargo bench --bench succinct_libs
//! ```

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

// ============================================================================
// Resident Memory Tracking Allocator
// ============================================================================

struct TrackingAllocator {
    current: AtomicUsize,
}

impl TrackingAllocator {
    const fn new() -> Self {
        Self {
            current: AtomicUsize::new(0),
        }
    }

    fn reset(&self) {
        self.current.store(0, Ordering::SeqCst);
    }

    /// Bytes currently held. Read while a structure is alive, this is its resident size.
    fn current(&self) -> usize {
        self.current.load(Ordering::SeqCst)
    }
}

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            self.current.fetch_add(layout.size(), Ordering::SeqCst);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.current.fetch_sub(layout.size(), Ordering::SeqCst);
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            let old_size = layout.size();
            if new_size > old_size {
                self.current
                    .fetch_add(new_size - old_size, Ordering::SeqCst);
            } else {
                self.current
                    .fetch_sub(old_size - new_size, Ordering::SeqCst);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator::new();

// ============================================================================
// Test Data
// ============================================================================

/// Sizes in bits. Matches `benches/rank_select.rs` in the main crate.
const SIZES: &[usize] = &[1_000_000, 10_000_000];

/// Fraction of set bits.
const DENSITIES: &[f64] = &[0.1, 0.5, 0.9];

/// Generate random words with the given density of set bits.
///
/// Mirrors `generate_bitvec` in `benches/rank_select.rs` so results are comparable.
fn generate_words(size: usize, density: f64, seed: u64) -> Vec<u64> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let word_count = size.div_ceil(64);
    let mut words = Vec::with_capacity(word_count);

    let threshold = (density * u64::MAX as f64) as u64;
    for _ in 0..word_count {
        let mut word = 0u64;
        for bit in 0..64 {
            if rng.r#gen::<u64>() < threshold {
                word |= 1 << bit;
            }
        }
        words.push(word);
    }
    words
}

/// Generate random query positions.
fn generate_queries(count: usize, max: usize, seed: u64) -> Vec<usize> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    (0..count).map(|_| rng.gen_range(0..max)).collect()
}

fn bit_at(words: &[u64], i: usize) -> bool {
    words[i / 64] >> (i % 64) & 1 == 1
}

// ============================================================================
// Per-Crate Builders
//
// Each takes a borrowed slice and returns a fully owned rank+select structure.
// ============================================================================

fn build_succinctly(words: &[u64], len: usize) -> succinctly::BitVec {
    succinctly::BitVec::from_words(words.to_vec(), len)
}

fn build_vers(words: &[u64], _len: usize) -> vers_vecs::RsVec {
    vers_vecs::RsVec::from_bit_vec(vers_vecs::BitVec::from_vec(words.to_vec()))
}

fn build_sucds(words: &[u64], len: usize) -> sucds::bit_vectors::Rank9Sel {
    use sucds::bit_vectors::Build;
    // sucds has no bulk-word constructor; per-bit is the only public path.
    sucds::bit_vectors::Rank9Sel::build_from_bits(
        (0..len).map(|i| bit_at(words, i)),
        true,
        true,
        true,
    )
    .expect("sucds build failed")
}

type SuxRankSel = sux::rank_sel::SelectAdapt<
    sux::rank_sel::Rank9<sux::traits::AddNumBits<sux::bits::BitVec<Vec<usize>>>>,
>;

fn build_sux(words: &[u64], len: usize) -> SuxRankSel {
    use sux::traits::BitVecOpsMut;
    let mut bv = sux::bits::BitVec::<Vec<usize>>::new(len);
    for i in 0..len {
        bv.set(i, bit_at(words, i));
    }
    sux::rank_sel::SelectAdapt::new(sux::rank_sel::Rank9::new(sux::traits::AddNumBits::from(bv)))
}

// ============================================================================
// Benchmarks
// ============================================================================

fn label(size: usize, density: f64) -> String {
    format!("{:.0}M/{:.0}%", size as f64 / 1e6, density * 100.0)
}

fn bench_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("construction");

    for &size in SIZES {
        for &density in DENSITIES {
            let words = generate_words(size, density, 42);
            let id = label(size, density);

            group.bench_with_input(BenchmarkId::new("succinctly", &id), &words, |b, w| {
                b.iter(|| build_succinctly(black_box(w), size));
            });
            group.bench_with_input(BenchmarkId::new("vers-vecs", &id), &words, |b, w| {
                b.iter(|| build_vers(black_box(w), size));
            });
            group.bench_with_input(BenchmarkId::new("sucds", &id), &words, |b, w| {
                b.iter(|| build_sucds(black_box(w), size));
            });
            group.bench_with_input(BenchmarkId::new("sux", &id), &words, |b, w| {
                b.iter(|| build_sux(black_box(w), size));
            });
        }
    }
    group.finish();
}

fn bench_rank(c: &mut Criterion) {
    use succinctly::RankSelect;
    use sucds::bit_vectors::Rank;
    use sux::traits::Rank as SuxRank;

    let mut group = c.benchmark_group("rank1");

    for &size in SIZES {
        for &density in DENSITIES {
            let words = generate_words(size, density, 42);
            let queries = generate_queries(10_000, size, 123);
            let id = label(size, density);

            let succ = build_succinctly(&words, size);
            group.bench_with_input(BenchmarkId::new("succinctly", &id), &queries, |b, qs| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &q in qs {
                        sum += succ.rank1(black_box(q));
                    }
                    sum
                });
            });
            drop(succ);

            let vers = build_vers(&words, size);
            group.bench_with_input(BenchmarkId::new("vers-vecs", &id), &queries, |b, qs| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &q in qs {
                        sum += vers.rank1(black_box(q));
                    }
                    sum
                });
            });
            drop(vers);

            let sucds_bv = build_sucds(&words, size);
            group.bench_with_input(BenchmarkId::new("sucds", &id), &queries, |b, qs| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &q in qs {
                        sum += sucds_bv.rank1(black_box(q)).unwrap_or(0);
                    }
                    sum
                });
            });
            drop(sucds_bv);

            let sux_bv = build_sux(&words, size);
            group.bench_with_input(BenchmarkId::new("sux", &id), &queries, |b, qs| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &q in qs {
                        sum += sux_bv.rank(black_box(q));
                    }
                    sum
                });
            });
            drop(sux_bv);
        }
    }
    group.finish();
}

fn bench_select(c: &mut Criterion) {
    use succinctly::RankSelect;
    use sucds::bit_vectors::Select;
    use sux::traits::Select as SuxSelect;

    let mut group = c.benchmark_group("select1");

    for &size in SIZES {
        for &density in DENSITIES {
            let words = generate_words(size, density, 42);
            let ones: usize = words.iter().map(|w| w.count_ones() as usize).sum();
            if ones == 0 {
                continue;
            }
            let queries = generate_queries(10_000, ones, 123);
            let id = label(size, density);

            let succ = build_succinctly(&words, size);
            group.bench_with_input(BenchmarkId::new("succinctly", &id), &queries, |b, qs| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &q in qs {
                        if let Some(p) = succ.select1(black_box(q)) {
                            sum += p;
                        }
                    }
                    sum
                });
            });
            drop(succ);

            let vers = build_vers(&words, size);
            group.bench_with_input(BenchmarkId::new("vers-vecs", &id), &queries, |b, qs| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &q in qs {
                        sum += vers.select1(black_box(q));
                    }
                    sum
                });
            });
            drop(vers);

            let sucds_bv = build_sucds(&words, size);
            group.bench_with_input(BenchmarkId::new("sucds", &id), &queries, |b, qs| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &q in qs {
                        sum += sucds_bv.select1(black_box(q)).unwrap_or(0);
                    }
                    sum
                });
            });
            drop(sucds_bv);

            let sux_bv = build_sux(&words, size);
            group.bench_with_input(BenchmarkId::new("sux", &id), &queries, |b, qs| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &q in qs {
                        sum += sux_bv.select(black_box(q)).unwrap_or(0);
                    }
                    sum
                });
            });
            drop(sux_bv);
        }
    }
    group.finish();
}

/// Assert all four structures answer rank1/select1 identically before timing them.
///
/// A benchmark of structures that disagree measures nothing, so this guards the rest of
/// the file. It also pins down the API differences worth knowing: vers-vecs `select1`
/// returns `usize` where succinctly and sucds return `Option`, and sux names the
/// operations `rank`/`select`.
fn verify_agreement(_c: &mut Criterion) {
    use succinctly::RankSelect;
    use sucds::bit_vectors::{Rank, Select};
    use sux::traits::{Rank as SuxRank, Select as SuxSelect};

    let size = 100_000;
    let words = generate_words(size, 0.3, 7);
    let ones: usize = words.iter().map(|w| w.count_ones() as usize).sum();

    let succ = build_succinctly(&words, size);
    let vers = build_vers(&words, size);
    let sucds_bv = build_sucds(&words, size);
    let sux_bv = build_sux(&words, size);

    for i in (0..size).step_by(997) {
        let expected = succ.rank1(i);
        assert_eq!(vers.rank1(i), expected, "vers-vecs rank1({i})");
        assert_eq!(sucds_bv.rank1(i), Some(expected), "sucds rank1({i})");
        assert_eq!(sux_bv.rank(i), expected, "sux rank({i})");
    }

    for k in (0..ones).step_by(499) {
        let expected = succ.select1(k).expect("succinctly select1");
        assert_eq!(vers.select1(k), expected, "vers-vecs select1({k})");
        assert_eq!(sucds_bv.select1(k), Some(expected), "sucds select1({k})");
        assert_eq!(sux_bv.select(k), Some(expected), "sux select({k})");
    }

    println!("\nverify: all four libraries agree on rank1/select1 over {size} bits");
}

/// Report resident bytes per structure and the overhead over the raw bits.
///
/// This prints rather than times: it is the space half of the comparison, and the
/// numbers it emits are what `docs/benchmarks/rust-succinct-libs.md` records.
fn report_memory(_c: &mut Criterion) {
    println!("\n{:=^78}", " Resident Size (rank + select structure) ");
    println!(
        "{:<14} {:<10} {:>12} {:>12} {:>12}",
        "size/density", "library", "raw KiB", "resident KiB", "overhead"
    );
    println!("{:-<78}", "");

    for &size in SIZES {
        for &density in DENSITIES {
            let words = generate_words(size, density, 42);
            let raw = size.div_ceil(8);
            let id = label(size, density);

            let measure = |name: &str, resident: usize| {
                let overhead = (resident as f64 - raw as f64) / raw as f64 * 100.0;
                println!(
                    "{:<14} {:<10} {:>12.1} {:>12.1} {:>11.1}%",
                    id,
                    name,
                    raw as f64 / 1024.0,
                    resident as f64 / 1024.0,
                    overhead
                );
            };

            ALLOCATOR.reset();
            let s = build_succinctly(&words, size);
            let r = ALLOCATOR.current();
            black_box(&s);
            drop(s);
            measure("succinctly", r);

            ALLOCATOR.reset();
            let s = build_vers(&words, size);
            let r = ALLOCATOR.current();
            black_box(&s);
            drop(s);
            measure("vers-vecs", r);

            ALLOCATOR.reset();
            let s = build_sucds(&words, size);
            let r = ALLOCATOR.current();
            black_box(&s);
            drop(s);
            measure("sucds", r);

            ALLOCATOR.reset();
            let s = build_sux(&words, size);
            let r = ALLOCATOR.current();
            black_box(&s);
            drop(s);
            measure("sux", r);
        }
    }
    println!("{:=<78}\n", "");
}

criterion_group!(
    benches,
    verify_agreement,
    bench_construction,
    bench_rank,
    bench_select,
    report_memory
);
criterion_main!(benches);
