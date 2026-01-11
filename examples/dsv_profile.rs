use std::fs;
use std::time::Instant;
use succinctly::dsv::{
    build_index, build_index_lightweight, DsvConfig, DsvRows, DsvRowsLightweight, DsvRowsOptimized,
};

fn main() {
    let paths = vec![
        ("strings-1mb", "data/bench/generated/dsv/strings/1mb.csv"),
        ("strings-10mb", "data/bench/generated/dsv/strings/10mb.csv"),
    ];

    let config = DsvConfig::csv();

    for (name, path) in paths {
        println!("\n=== {} ===", name);
        let data = fs::read(path).expect("Failed to read file");
        println!("File size: {} bytes", data.len());

        // Benchmark 1: Pure parsing (index building)
        let start = Instant::now();
        let iterations = 100;
        for _ in 0..iterations {
            let _index = build_index(&data, &config);
        }
        let elapsed = start.elapsed();
        let per_iter = elapsed.as_secs_f64() / iterations as f64;
        let throughput = (data.len() as f64 / per_iter) / (1024.0 * 1024.0);
        println!(
            "Index build (BitVec): {:.3}ms/iter, {:.1} MiB/s",
            per_iter * 1000.0,
            throughput
        );

        // Benchmark 1b: Lightweight index building
        let start = Instant::now();
        let iterations = 100;
        for _ in 0..iterations {
            let _index = build_index_lightweight(&data, &config);
        }
        let elapsed = start.elapsed();
        let per_iter = elapsed.as_secs_f64() / iterations as f64;
        let throughput = (data.len() as f64 / per_iter) / (1024.0 * 1024.0);
        println!(
            "Index build (Lightweight): {:.3}ms/iter, {:.1} MiB/s",
            per_iter * 1000.0,
            throughput
        );

        // Benchmark 2a: Original - Parsing + row iteration
        let start = Instant::now();
        let iterations = 100;
        let mut total_fields = 0;
        for _ in 0..iterations {
            let index = build_index(&data, &config);
            let rows = DsvRows::new(&data, &index);
            for row in rows {
                for _field in row.fields() {
                    total_fields += 1;
                }
            }
        }
        let elapsed = start.elapsed();
        let per_iter_orig = elapsed.as_secs_f64() / iterations as f64;
        let throughput_orig = (data.len() as f64 / per_iter_orig) / (1024.0 * 1024.0);
        println!(
            "Original iterate: {:.3}ms/iter, {:.1} MiB/s ({} fields/iter)",
            per_iter_orig * 1000.0,
            throughput_orig,
            total_fields / iterations
        );

        // Benchmark 2b: Optimized with rank caching
        let start = Instant::now();
        let iterations = 100;
        let mut total_fields = 0;
        for _ in 0..iterations {
            let index = build_index(&data, &config);
            let rows = DsvRowsOptimized::new(&data, &index);
            for row in rows {
                for _field in row {
                    total_fields += 1;
                }
            }
        }
        let elapsed = start.elapsed();
        let per_iter_opt = elapsed.as_secs_f64() / iterations as f64;
        let throughput_opt = (data.len() as f64 / per_iter_opt) / (1024.0 * 1024.0);
        let speedup = throughput_opt / throughput_orig;
        println!(
            "Optimized (rank cache): {:.3}ms/iter, {:.1} MiB/s [{}x vs original]",
            per_iter_opt * 1000.0,
            throughput_opt,
            speedup
        );

        // Benchmark 2c: Lightweight index
        let start = Instant::now();
        let iterations = 100;
        let mut total_fields = 0;
        for _ in 0..iterations {
            let index = build_index_lightweight(&data, &config);
            let rows = DsvRowsLightweight::new(&data, &index);
            for row in rows {
                for _field in row {
                    total_fields += 1;
                }
            }
        }
        let elapsed = start.elapsed();
        let per_iter_light = elapsed.as_secs_f64() / iterations as f64;
        let throughput_light = (data.len() as f64 / per_iter_light) / (1024.0 * 1024.0);
        let speedup_vs_orig = throughput_light / throughput_orig;
        let speedup_vs_opt = throughput_light / throughput_opt;
        println!(
            "Lightweight index: {:.3}ms/iter, {:.1} MiB/s [{}x vs original, {}x vs rank cache]",
            per_iter_light * 1000.0,
            throughput_light,
            speedup_vs_orig,
            speedup_vs_opt
        );
    }
}
