use std::time::Instant;
use succinctly::{BitVec, RankSelect};

fn main() {
    // Create a large bitvector with some pattern
    let size = 10_000_000; // 10M bits
    let mut bits = vec![0u64; size / 64];

    // Set every 10th bit
    for i in 0..size {
        if i % 10 == 0 {
            let word_idx = i / 64;
            let bit_idx = i % 64;
            bits[word_idx] |= 1u64 << bit_idx;
        }
    }

    let bitvec = BitVec::from_words(bits, size);

    println!(
        "BitVec size: {} bits, {} set bits",
        size,
        bitvec.count_ones()
    );

    // Benchmark rank1
    let iterations = 1_000_000;
    let start = Instant::now();
    let mut sum = 0;
    for i in 0..iterations {
        let pos = (i * 7) % size; // Pseudo-random positions
        sum += bitvec.rank1(pos);
    }
    let elapsed = start.elapsed();
    let ns_per_op = (elapsed.as_nanos() as f64) / iterations as f64;
    println!(
        "rank1: {:.1}ns per operation ({} iterations, sum={})",
        ns_per_op, iterations, sum
    );

    // Benchmark select1
    let iterations = 100_000; // Fewer iterations since select is slower
    let start = Instant::now();
    let mut sum = 0;
    let num_ones = bitvec.count_ones();
    for i in 0..iterations {
        let rank = (i * 13) % num_ones; // Pseudo-random ranks
        if let Some(pos) = bitvec.select1(rank) {
            sum += pos;
        }
    }
    let elapsed = start.elapsed();
    let ns_per_op = (elapsed.as_nanos() as f64) / iterations as f64;
    println!(
        "select1: {:.1}ns per operation ({} iterations, sum={})",
        ns_per_op, iterations, sum
    );

    // Simulate DSV field iteration pattern
    println!("\n=== Simulating DSV field iteration ===");
    let iterations = 10_000;
    let start = Instant::now();
    let mut position = 0;
    let mut field_count = 0;

    for _ in 0..iterations {
        // Simulate processing fields
        for _ in 0..20 {
            // 20 fields per row
            // current_field: rank1 + select1
            let current_rank = bitvec.rank1(position);
            let field_end = bitvec.select1(current_rank).unwrap_or(size);

            // Check for newline: rank1 x2
            let _rank1 = bitvec.rank1(field_end);
            let _rank2 = bitvec.rank1(field_end + 1);

            // next_field: rank1 + select1
            let next_rank = bitvec.rank1(position);
            position = bitvec.select1(next_rank).map(|p| p + 1).unwrap_or(size);

            // at_newline: rank1 x2
            if position > 0 {
                let _prev_rank = bitvec.rank1(position - 1);
                let _curr_rank = bitvec.rank1(position);
            }

            field_count += 1;
            if position >= size {
                break;
            }
        }
        position = (position * 7) % size; // Reset to pseudo-random position
    }

    let elapsed = start.elapsed();
    let us_per_field = (elapsed.as_micros() as f64) / field_count as f64;
    println!(
        "Field iteration: {:.3}μs per field ({} fields)",
        us_per_field, field_count
    );
    println!("Estimated: {} rank1 + {} select1 calls per field", 6, 2);
}
