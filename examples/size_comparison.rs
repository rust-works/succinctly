//! Quick benchmark comparing succinctly vs serde_json at different file sizes

use std::time::Instant;
use succinctly::json::light::{JsonCursor, JsonIndex, StandardJson};

fn main() {
    let sizes = [
        ("10mb", "data/bench/generated/comprehensive/10mb.json"),
        ("100mb", "data/bench/generated/comprehensive/100mb.json"),
        ("1gb", "data/bench/generated/comprehensive/1gb.json"),
    ];

    for (name, path) in sizes {
        let path = std::path::Path::new(path);
        if !path.exists() {
            eprintln!("Skipping {}: file not found", name);
            continue;
        }

        let bytes = std::fs::read(path).expect("Failed to read");
        let file_size_mb = bytes.len() as f64 / 1024.0 / 1024.0;

        println!("\n=== {} ({:.1} MB) ===", name, file_size_mb);

        // serde_json parse
        let start = Instant::now();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let serde_parse = start.elapsed();

        // serde_json traverse
        let start = Instant::now();
        let serde_count = count_serde(&value);
        let serde_traverse = start.elapsed();

        // succinctly index
        let start = Instant::now();
        let index = JsonIndex::build(&bytes);
        let succ_index = start.elapsed();

        // succinctly traverse (with text_position on every node - old/slow)
        let start = Instant::now();
        let root = index.root(&bytes);
        let succ_count = count_succinctly(root.value());
        let succ_traverse = start.elapsed();

        // succinctly traverse (using children() - optimized API)
        let start = Instant::now();
        let root = index.root(&bytes);
        let succ_fast_count = count_succinctly_fast(root);
        let succ_fast_traverse = start.elapsed();

        println!(
            "serde_json:  parse={:>7.1}ms  traverse={:>7.1}ms  total={:>7.1}ms  nodes={}",
            serde_parse.as_secs_f64() * 1000.0,
            serde_traverse.as_secs_f64() * 1000.0,
            (serde_parse + serde_traverse).as_secs_f64() * 1000.0,
            serde_count
        );
        println!(
            "succ(old):   index={:>7.1}ms  traverse={:>7.1}ms  total={:>7.1}ms  nodes={}",
            succ_index.as_secs_f64() * 1000.0,
            succ_traverse.as_secs_f64() * 1000.0,
            (succ_index + succ_traverse).as_secs_f64() * 1000.0,
            succ_count
        );
        println!(
            "succ(fast):  index={:>7.1}ms  traverse={:>7.1}ms  total={:>7.1}ms  nodes={}",
            succ_index.as_secs_f64() * 1000.0,
            succ_fast_traverse.as_secs_f64() * 1000.0,
            (succ_index + succ_fast_traverse).as_secs_f64() * 1000.0,
            succ_fast_count
        );

        // Compare old vs new succinctly
        let _old_total = (succ_index + succ_traverse).as_secs_f64();
        let new_total = (succ_index + succ_fast_traverse).as_secs_f64();
        let traverse_speedup = succ_traverse.as_secs_f64() / succ_fast_traverse.as_secs_f64();
        println!(
            "Traverse speedup: {:.1}x ({:.1}ms -> {:.1}ms)",
            traverse_speedup,
            succ_traverse.as_secs_f64() * 1000.0,
            succ_fast_traverse.as_secs_f64() * 1000.0
        );

        // Compare with serde_json
        let serde_total = (serde_parse + serde_traverse).as_secs_f64();
        if new_total < serde_total {
            println!(
                "succinctly(fast) is {:.2}x faster than serde_json",
                serde_total / new_total
            );
        } else {
            println!(
                "serde_json is {:.2}x faster than succinctly(fast)",
                new_total / serde_total
            );
        }
    }
}

fn count_serde(v: &serde_json::Value) -> usize {
    match v {
        serde_json::Value::Array(arr) => 1 + arr.iter().map(count_serde).sum::<usize>(),
        serde_json::Value::Object(obj) => 1 + obj.values().map(count_serde).sum::<usize>(),
        _ => 1,
    }
}

fn count_succinctly(v: StandardJson) -> usize {
    match v {
        StandardJson::Array(elements) => {
            let mut count = 1;
            for elem in elements {
                count += count_succinctly(elem);
            }
            count
        }
        StandardJson::Object(entries) => {
            let mut count = 1;
            for entry in entries {
                count += count_succinctly(entry.value());
            }
            count
        }
        _ => 1,
    }
}

/// Count nodes using the optimized children() API - no text_position() calls
fn count_succinctly_fast(cursor: JsonCursor) -> usize {
    let mut count = 1; // Count this node
    for child in cursor.children() {
        count += count_succinctly_fast(child);
    }
    count
}
