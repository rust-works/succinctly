use std::fs;
use std::time::Instant;
use succinctly::dsv::{build_index, DsvConfig, DsvRows};

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
            "Index build: {:.3}ms/iter, {:.1} MiB/s",
            per_iter * 1000.0,
            throughput
        );

        // Benchmark 2: Parsing + row iteration
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
        let per_iter = elapsed.as_secs_f64() / iterations as f64;
        let throughput = (data.len() as f64 / per_iter) / (1024.0 * 1024.0);
        println!(
            "Index + iterate: {:.3}ms/iter, {:.1} MiB/s ({} fields/iter)",
            per_iter * 1000.0,
            throughput,
            total_fields / iterations
        );

        // Benchmark 3: Parsing + row iteration + string conversion
        let start = Instant::now();
        let iterations = 10; // Fewer iterations for slower benchmark
        for _ in 0..iterations {
            let index = build_index(&data, &config);
            let rows = DsvRows::new(&data, &index);
            let mut _results = Vec::new();
            for row in rows {
                let fields: Vec<String> = row
                    .fields()
                    .map(|f| String::from_utf8_lossy(f).into_owned())
                    .collect();
                _results.push(fields);
            }
        }
        let elapsed = start.elapsed();
        let per_iter = elapsed.as_secs_f64() / iterations as f64;
        let throughput = (data.len() as f64 / per_iter) / (1024.0 * 1024.0);
        println!(
            "With string conversion: {:.3}ms/iter, {:.1} MiB/s",
            per_iter * 1000.0,
            throughput
        );
    }
}
