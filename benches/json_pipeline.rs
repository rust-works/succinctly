//! Benchmark the full JSON processing pipeline: indexing, navigation, and printing.
//!
//! This benchmark measures where time is spent in real-world JSON processing:
//! - Indexing: Building the semi-index (IB + BP bitvectors)
//! - Navigation: Traversing the JSON structure to extract values
//! - Printing: Converting extracted values back to JSON text
//!
//! Run with:
//! ```bash
//! cargo bench --bench json_pipeline
//! ```

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use std::io::Write;
use succinctly::bp::BalancedParens;
use succinctly::json::light::{JsonIndex, StandardJson};
use succinctly::select_in_word;

// ============================================================================
// JsonIndexV2: Uses BalancedParens for O(1) rank1
// ============================================================================

/// V2 index with O(1) rank1 operations.
struct JsonIndexV2 {
    ib: Vec<u64>,
    #[allow(dead_code)]
    ib_len: usize,
    ib_rank: Vec<u32>,
    bp: BalancedParens<Vec<u64>>,
}

impl JsonIndexV2 {
    fn build(json: &[u8]) -> Self {
        #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
        let semi = succinctly::json::simd::build_semi_index_standard(json);

        #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
        let semi = succinctly::json::standard::build_semi_index(json);

        let ib_len = json.len();
        let bp_bit_count = count_bp_bits(&semi.bp);
        let ib_rank = build_ib_rank(&semi.ib);

        Self {
            ib: semi.ib,
            ib_len,
            ib_rank,
            bp: BalancedParens::new(semi.bp, bp_bit_count),
        }
    }

    fn ib_select1(&self, k: usize) -> Option<usize> {
        if self.ib.is_empty() {
            return None;
        }
        let k32 = k as u32;
        let mut lo = 0usize;
        let mut hi = self.ib.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.ib_rank[mid + 1] <= k32 {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo >= self.ib.len() {
            return None;
        }
        let word = self.ib[lo];
        let prev_count = self.ib_rank[lo] as usize;
        let k_in_word = k - prev_count;
        let pos_in_word = select_in_word(word, k_in_word as u32);
        if pos_in_word >= 64 {
            return None;
        }
        Some(lo * 64 + pos_in_word as usize)
    }

    fn text_position(&self, bp_pos: usize) -> Option<usize> {
        let rank = self.bp.rank1(bp_pos);
        self.ib_select1(rank)
    }
}

fn build_ib_rank(words: &[u64]) -> Vec<u32> {
    let mut rank = Vec::with_capacity(words.len() + 1);
    let mut cumulative: u32 = 0;
    rank.push(0);
    for &word in words {
        cumulative += word.count_ones();
        rank.push(cumulative);
    }
    rank
}

fn count_bp_bits(words: &[u64]) -> usize {
    words.iter().map(|w| w.count_ones() as usize).sum::<usize>() * 2
}

/// Test file configuration
const TEST_FILE: &str = "data/bench/generated/comprehensive/10mb.json";

/// Load test file or skip benchmark
fn load_test_file() -> Option<Vec<u8>> {
    let path = std::path::Path::new(TEST_FILE);
    if !path.exists() {
        eprintln!("Skipping benchmark: {} not found", TEST_FILE);
        eprintln!("Run: ./target/release/succinctly json generate-suite");
        return None;
    }
    Some(std::fs::read(path).expect("Failed to read test file"))
}

/// Benchmark 1: Indexing only
/// Measures the time to build the semi-index from raw JSON bytes.
fn bench_indexing(c: &mut Criterion) {
    let Some(bytes) = load_test_file() else {
        return;
    };
    let file_size = bytes.len() as u64;

    let mut group = c.benchmark_group("pipeline_indexing");
    group.throughput(Throughput::Bytes(file_size));
    group.sample_size(20);

    group.bench_function("build_index", |b| {
        b.iter(|| JsonIndex::build(black_box(&bytes)))
    });

    group.finish();
}

/// Benchmark 2: Navigation/Query
/// Measures the time to traverse the JSON structure and count elements.
fn bench_navigation(c: &mut Criterion) {
    let Some(bytes) = load_test_file() else {
        return;
    };
    let file_size = bytes.len() as u64;

    // Pre-build the index (not part of navigation benchmark)
    let index = JsonIndex::build(&bytes);

    let mut group = c.benchmark_group("pipeline_navigation");
    group.throughput(Throughput::Bytes(file_size));
    group.sample_size(20);

    // Count all elements at root level (shallow traversal)
    group.bench_function("count_root_elements", |b| {
        b.iter(|| {
            let root = index.root(black_box(&bytes));
            match root.value() {
                StandardJson::Array(elements) => {
                    let mut count = 0usize;
                    let mut iter = elements;
                    while let Some((_, rest)) = iter.uncons() {
                        count += 1;
                        iter = rest;
                    }
                    count
                }
                StandardJson::Object(fields) => {
                    let mut count = 0usize;
                    let mut iter = fields;
                    while let Some((_, rest)) = iter.uncons() {
                        count += 1;
                        iter = rest;
                    }
                    count
                }
                _ => 1,
            }
        })
    });

    // Deep traversal - count all leaf values
    group.bench_function("count_all_leaves", |b| {
        b.iter(|| {
            let root = index.root(black_box(&bytes));
            count_leaves(&root.value())
        })
    });

    // Extract all string values (simulates field extraction)
    group.bench_function("extract_all_strings", |b| {
        b.iter(|| {
            let root = index.root(black_box(&bytes));
            extract_strings(&root.value())
        })
    });

    group.finish();
}

/// Count all leaf values recursively
fn count_leaves(value: &StandardJson) -> usize {
    match value {
        StandardJson::Array(elements) => {
            let mut count = 0;
            let mut iter = *elements;
            while let Some((elem, rest)) = iter.uncons() {
                count += count_leaves(&elem);
                iter = rest;
            }
            count
        }
        StandardJson::Object(fields) => {
            let mut count = 0;
            let mut iter = *fields;
            while let Some((field, rest)) = iter.uncons() {
                count += count_leaves(&field.value());
                iter = rest;
            }
            count
        }
        _ => 1, // Leaf value
    }
}

/// Extract all string values and count total characters
fn extract_strings(value: &StandardJson) -> usize {
    match value {
        StandardJson::Array(elements) => {
            let mut count = 0;
            let mut iter = *elements;
            while let Some((elem, rest)) = iter.uncons() {
                count += extract_strings(&elem);
                iter = rest;
            }
            count
        }
        StandardJson::Object(fields) => {
            let mut count = 0;
            let mut iter = *fields;
            while let Some((field, rest)) = iter.uncons() {
                // Count key length
                if let StandardJson::String(key) = field.key()
                    && let Ok(k) = key.as_str()
                {
                    count += k.len();
                }
                count += extract_strings(&field.value());
                iter = rest;
            }
            count
        }
        StandardJson::String(s) => s.as_str().map(|s| s.len()).unwrap_or(0),
        _ => 0,
    }
}

/// Benchmark 3: Printing/Serialization
/// Measures the time to convert JSON values back to text.
fn bench_printing(c: &mut Criterion) {
    let Some(bytes) = load_test_file() else {
        return;
    };
    let file_size = bytes.len() as u64;

    // Pre-build the index
    let index = JsonIndex::build(&bytes);

    let mut group = c.benchmark_group("pipeline_printing");
    group.throughput(Throughput::Bytes(file_size));
    group.sample_size(20);

    // Print all values to a sink (measure serialization speed)
    group.bench_function("print_all_values", |b| {
        let mut buffer = Vec::with_capacity(file_size as usize);
        b.iter(|| {
            buffer.clear();
            let root = index.root(black_box(&bytes));
            print_value(&root.value(), &mut buffer);
            buffer.len()
        })
    });

    // Print just string values (common operation)
    group.bench_function("print_strings_only", |b| {
        let mut buffer = Vec::with_capacity(file_size as usize / 2);
        b.iter(|| {
            buffer.clear();
            let root = index.root(black_box(&bytes));
            print_strings(&root.value(), &mut buffer);
            buffer.len()
        })
    });

    group.finish();
}

/// Print a JSON value to a writer
fn print_value<W: Write>(value: &StandardJson, out: &mut W) {
    match value {
        StandardJson::Null => {
            let _ = out.write_all(b"null");
        }
        StandardJson::Bool(b) => {
            let _ = out.write_all(if *b { b"true" } else { b"false" });
        }
        StandardJson::Number(n) => {
            if let Ok(i) = n.as_i64() {
                let _ = write!(out, "{}", i);
            } else if let Ok(f) = n.as_f64() {
                let _ = write!(out, "{}", f);
            }
        }
        StandardJson::String(s) => {
            let _ = out.write_all(b"\"");
            if let Ok(s) = s.as_str() {
                let _ = out.write_all(s.as_bytes());
            }
            let _ = out.write_all(b"\"");
        }
        StandardJson::Array(elements) => {
            let _ = out.write_all(b"[");
            let mut first = true;
            let mut iter = *elements;
            while let Some((elem, rest)) = iter.uncons() {
                if !first {
                    let _ = out.write_all(b",");
                }
                first = false;
                print_value(&elem, out);
                iter = rest;
            }
            let _ = out.write_all(b"]");
        }
        StandardJson::Object(fields) => {
            let _ = out.write_all(b"{");
            let mut first = true;
            let mut iter = *fields;
            while let Some((field, rest)) = iter.uncons() {
                if !first {
                    let _ = out.write_all(b",");
                }
                first = false;
                let _ = out.write_all(b"\"");
                if let StandardJson::String(key) = field.key()
                    && let Ok(k) = key.as_str()
                {
                    let _ = out.write_all(k.as_bytes());
                }
                let _ = out.write_all(b"\":");
                print_value(&field.value(), out);
                iter = rest;
            }
            let _ = out.write_all(b"}");
        }
        StandardJson::Error(_) => {
            let _ = out.write_all(b"null");
        }
    }
}

/// Print only string values (line-separated)
fn print_strings<W: Write>(value: &StandardJson, out: &mut W) {
    match value {
        StandardJson::Array(elements) => {
            let mut iter = *elements;
            while let Some((elem, rest)) = iter.uncons() {
                print_strings(&elem, out);
                iter = rest;
            }
        }
        StandardJson::Object(fields) => {
            let mut iter = *fields;
            while let Some((field, rest)) = iter.uncons() {
                // Print key
                if let StandardJson::String(key) = field.key()
                    && let Ok(k) = key.as_str()
                {
                    let _ = out.write_all(k.as_bytes());
                    let _ = out.write_all(b"\n");
                }
                print_strings(&field.value(), out);
                iter = rest;
            }
        }
        StandardJson::String(s) => {
            if let Ok(s) = s.as_str() {
                let _ = out.write_all(s.as_bytes());
                let _ = out.write_all(b"\n");
            }
        }
        _ => {}
    }
}

/// Benchmark 4: Full pipeline comparison
/// Measures end-to-end time for different operations.
fn bench_full_pipeline(c: &mut Criterion) {
    let Some(bytes) = load_test_file() else {
        return;
    };
    let file_size = bytes.len() as u64;

    let mut group = c.benchmark_group("pipeline_full");
    group.throughput(Throughput::Bytes(file_size));
    group.sample_size(20);

    // Full pipeline: index + navigate + print
    group.bench_function("index_navigate_print", |b| {
        let mut buffer = Vec::with_capacity(file_size as usize);
        b.iter(|| {
            buffer.clear();
            let index = JsonIndex::build(black_box(&bytes));
            let root = index.root(&bytes);
            print_value(&root.value(), &mut buffer);
            buffer.len()
        })
    });

    // Just index + count leaves (no printing)
    group.bench_function("index_navigate_only", |b| {
        b.iter(|| {
            let index = JsonIndex::build(black_box(&bytes));
            let root = index.root(&bytes);
            count_leaves(&root.value())
        })
    });

    // Compare with serde_json parsing (if available)
    group.bench_function("serde_json_parse", |b| {
        b.iter(|| {
            let _: serde_json::Value = serde_json::from_slice(black_box(&bytes)).unwrap();
        })
    });

    // serde_json parse + serialize
    group.bench_function("serde_json_roundtrip", |b| {
        let mut buffer = Vec::with_capacity(file_size as usize);
        b.iter(|| {
            buffer.clear();
            let value: serde_json::Value = serde_json::from_slice(black_box(&bytes)).unwrap();
            serde_json::to_writer(&mut buffer, &value).unwrap();
            buffer.len()
        })
    });

    group.finish();
}

/// Benchmark V1 vs V2 text_position (the core operation that uses rank1).
/// This directly compares the impact of O(1) vs O(n/block) rank1.
fn bench_v1_vs_v2_text_position(c: &mut Criterion) {
    let Some(bytes) = load_test_file() else {
        return;
    };
    let _file_size = bytes.len() as u64;

    // Build both indexes
    let index_v1 = JsonIndex::build(&bytes);
    let index_v2 = JsonIndexV2::build(&bytes);

    // Generate query positions (random BP positions that are valid opens)
    let bp_len = index_v1.bp().len();
    let mut queries = Vec::with_capacity(10_000);
    let mut pos = 0usize;
    while pos < bp_len && queries.len() < 10_000 {
        if index_v1.bp().is_open(pos) {
            queries.push(pos);
        }
        pos += 17; // Skip some positions to get a spread
    }

    let mut group = c.benchmark_group("v1_vs_v2_text_position");
    group.throughput(Throughput::Elements(queries.len() as u64));
    group.sample_size(50);

    // Benchmark V1 text_position (uses rank1 internally)
    group.bench_function("V1_text_position", |b| {
        b.iter(|| {
            let mut sum = 0usize;
            for &bp_pos in &queries {
                // V1: rank1 + select1
                let rank = index_v1.bp().rank1(bp_pos);
                sum += rank;
            }
            sum
        })
    });

    // Benchmark V2 text_position
    group.bench_function("V2_text_position", |b| {
        b.iter(|| {
            let mut sum = 0usize;
            for &bp_pos in &queries {
                // V2: O(1) rank1 + select1
                let rank = index_v2.bp.rank1(bp_pos);
                sum += rank;
            }
            sum
        })
    });

    group.finish();
}

/// Full navigation benchmark comparing V1 vs V2.
/// This traverses the entire JSON structure and measures total time.
fn bench_v1_vs_v2_full_traverse(c: &mut Criterion) {
    let Some(bytes) = load_test_file() else {
        return;
    };
    let file_size = bytes.len() as u64;

    // Build both indexes
    let index_v1 = JsonIndex::build(&bytes);
    let index_v2 = JsonIndexV2::build(&bytes);

    let mut group = c.benchmark_group("v1_vs_v2_full_traverse");
    group.throughput(Throughput::Bytes(file_size));
    group.sample_size(20);

    // V1: Full traversal counting nodes via text_position calls
    group.bench_function("V1_traverse_count_nodes", |b| {
        b.iter(|| {
            let root = index_v1.root(black_box(&bytes));
            count_leaves(&root.value())
        })
    });

    // V2: Same traversal pattern but with V2's O(1) rank1
    // Uses depth-first traversal with first_child/next_sibling like V1's count_leaves
    group.bench_function("V2_traverse_with_text_position", |b| {
        b.iter(|| {
            // Depth-first traversal calling text_position on each node
            let mut count = 0usize;
            let mut text_sum = 0usize;
            let mut stack = vec![0usize]; // Start at root

            while let Some(pos) = stack.pop() {
                // Call text_position (this is what we're benchmarking)
                if let Some(text_pos) = index_v2.text_position(pos) {
                    text_sum += text_pos;
                }
                count += 1;

                // Push next sibling first (so it's processed after children)
                if let Some(sib) = index_v2.bp.next_sibling(pos) {
                    stack.push(sib);
                }

                // Then push first child (so it's processed next)
                if let Some(child) = index_v2.bp.first_child(pos) {
                    stack.push(child);
                }
            }
            black_box(text_sum);
            count
        })
    });

    group.finish();
}

/// Benchmark sequential vs random select access patterns.
/// This demonstrates the benefit of exponential search over binary search.
fn bench_select_patterns(c: &mut Criterion) {
    let Some(bytes) = load_test_file() else {
        return;
    };

    // Build index
    let index = JsonIndex::build(&bytes);

    // Get total IB 1-bits for query generation
    let ib = index.ib();
    let total_ones: u32 = ib.iter().map(|w| w.count_ones()).sum();
    if total_ones == 0 {
        return;
    }

    // Generate sequential queries (0, 1, 2, ..., N-1)
    let sequential_queries: Vec<usize> = (0..10_000.min(total_ones as usize)).collect();

    // Generate random queries
    use rand::{Rng, SeedableRng};
    use rand_chacha::ChaCha8Rng;
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let random_queries: Vec<usize> = (0..10_000)
        .map(|_| rng.gen_range(0..total_ones as usize))
        .collect();

    let mut group = c.benchmark_group("select_patterns");
    group.throughput(criterion::Throughput::Elements(10_000));
    group.sample_size(50);

    // Benchmark sequential access WITH hints (exponential search - optimal for iteration)
    group.bench_function("sequential_exponential", |b| {
        b.iter(|| {
            let mut sum = 0usize;
            let mut hint = 0usize;
            for &k in &sequential_queries {
                if let Some(pos) = index.ib_select1_from(k, hint) {
                    sum += pos;
                    hint = pos / 64; // Update hint for next iteration
                }
            }
            sum
        })
    });

    // Benchmark sequential access with binary search (suboptimal for iteration)
    group.bench_function("sequential_binary", |b| {
        b.iter(|| {
            let mut sum = 0usize;
            for &k in &sequential_queries {
                if let Some(pos) = index.ib_select1(k) {
                    sum += pos;
                }
            }
            sum
        })
    });

    // Benchmark random access WITH hints (exponential search - suboptimal for random)
    group.bench_function("random_exponential", |b| {
        b.iter(|| {
            let mut sum = 0usize;
            let mut hint = 0usize;
            for &k in &random_queries {
                if let Some(pos) = index.ib_select1_from(k, hint) {
                    sum += pos;
                    hint = pos / 64;
                }
            }
            sum
        })
    });

    // Benchmark random access with binary search (optimal for random)
    group.bench_function("random_binary", |b| {
        b.iter(|| {
            let mut sum = 0usize;
            for &k in &random_queries {
                if let Some(pos) = index.ib_select1(k) {
                    sum += pos;
                }
            }
            sum
        })
    });

    group.finish();
}

/// Benchmark array indexing: get() vs get_fast().
/// This directly measures the optimization where get_fast() uses only BP operations
/// for navigation and only calls text_position once at the target element.
fn bench_array_indexing(c: &mut Criterion) {
    let Some(bytes) = load_test_file() else {
        return;
    };

    // Build index
    let index = JsonIndex::build(&bytes);
    let root = index.root(&bytes);

    // Find an array to work with
    let StandardJson::Object(fields) = root.value() else {
        eprintln!("Root is not an object, skipping array indexing benchmark");
        return;
    };

    // Find any field that contains a decent-sized array
    let mut test_array = None;
    let mut field_iter = fields;
    while let Some((field, rest)) = field_iter.uncons() {
        if let StandardJson::Array(elements) = field.value() {
            let count = elements.count();
            if count >= 100 {
                test_array = Some((field.value(), count));
                break;
            }
        }
        field_iter = rest;
    }

    let Some((array_value, array_len)) = test_array else {
        eprintln!("No suitable array found, skipping array indexing benchmark");
        return;
    };

    let StandardJson::Array(elements) = array_value else {
        unreachable!()
    };

    println!(
        "Using array with {} elements for indexing benchmark",
        array_len
    );

    let mut group = c.benchmark_group("array_indexing");
    group.sample_size(100);

    // Test accessing element at various positions
    let positions = [0, 10, 50, 100.min(array_len - 1)];

    for &idx in &positions {
        if idx >= array_len {
            continue;
        }

        // Benchmark get() - O(n) IB selects
        group.bench_function(format!("get_{}", idx), |b| {
            b.iter(|| black_box(elements.get(black_box(idx))))
        });

        // Benchmark get_fast() - O(n) BP ops + O(log n) IB select
        group.bench_function(format!("get_fast_{}", idx), |b| {
            b.iter(|| black_box(elements.get_fast(black_box(idx))))
        });
    }

    group.finish();
}

/// Benchmark jq evaluation with array indexing.
/// This tests the actual jq evaluator to measure end-to-end impact.
fn bench_jq_array_index(c: &mut Criterion) {
    use succinctly::jq;

    let Some(bytes) = load_test_file() else {
        return;
    };

    // Build index
    let index = JsonIndex::build(&bytes);

    // Parse a jq expression that accesses array elements by index
    // Typical pattern: .[50] or .items[50]
    let expr = jq::parse(".[50]").expect("Failed to parse jq expression");

    let mut group = c.benchmark_group("jq_array_index");
    group.sample_size(100);

    group.bench_function("jq_root_array_index", |b| {
        b.iter(|| {
            let cursor = index.root(black_box(&bytes));
            let result = jq::eval(&expr, cursor);
            black_box(result)
        })
    });

    // Try field access + array index pattern
    if let Ok(field_expr) = jq::parse(".users[50]") {
        group.bench_function("jq_field_array_index", |b| {
            b.iter(|| {
                let cursor = index.root(black_box(&bytes));
                let result = jq::eval(&field_expr, cursor);
                black_box(result)
            })
        });
    }

    // Compare with iteration (should be slower due to multiple text_positions)
    let iter_expr = jq::parse(".[]").expect("Failed to parse iterate expression");

    group.bench_function("jq_iterate_to_50", |b| {
        b.iter(|| {
            let cursor = index.root(black_box(&bytes));
            let result = jq::eval_lenient(&iter_expr, cursor);
            // Only look at first 51 elements
            let first_51: Vec<_> = result.into_iter().take(51).collect();
            black_box(first_51)
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_indexing,
    bench_navigation,
    bench_printing,
    bench_full_pipeline,
    bench_v1_vs_v2_text_position,
    bench_v1_vs_v2_full_traverse,
    bench_select_patterns,
    bench_array_indexing,
    bench_jq_array_index,
);
criterion_main!(benches);
