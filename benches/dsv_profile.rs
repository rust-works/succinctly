use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use std::fs;
use succinctly::dsv::{build_index, DsvConfig, DsvRows};

fn profile_dsv_parsing(c: &mut Criterion) {
    let sizes = vec![
        ("1mb", "data/bench/generated/dsv/strings/1mb.csv"),
        ("10mb", "data/bench/generated/dsv/strings/10mb.csv"),
    ];

    let config = DsvConfig::csv();

    for (name, path) in sizes {
        eprintln!("Loading {}...", path);
        if let Ok(data) = fs::read(path) {
            eprintln!("Loaded {} bytes", data.len());
            let mut group = c.benchmark_group(format!("dsv_parse_{}", name));
            group.throughput(Throughput::Bytes(data.len() as u64));

            // Benchmark 1: Pure parsing (index building)
            group.bench_function("index_build", |b| {
                b.iter(|| {
                    let index = build_index(black_box(&data), black_box(&config));
                    black_box(index)
                })
            });

            // Benchmark 2: Parsing + row iteration (no JSON)
            group.bench_function("index_and_iterate", |b| {
                b.iter(|| {
                    let index = build_index(black_box(&data), black_box(&config));
                    let rows = DsvRows::new(black_box(&data), black_box(&index));
                    let mut count = 0;
                    for row in rows {
                        for field in row.fields() {
                            black_box(field);
                            count += 1;
                        }
                    }
                    black_box(count)
                })
            });

            // Benchmark 3: Parsing + row iteration + string conversion
            group.bench_function("with_string_conversion", |b| {
                b.iter(|| {
                    let index = build_index(black_box(&data), black_box(&config));
                    let rows = DsvRows::new(black_box(&data), black_box(&index));
                    let mut results = Vec::new();
                    for row in rows {
                        let fields: Vec<String> = row
                            .fields()
                            .map(|f| String::from_utf8_lossy(f).into_owned())
                            .collect();
                        results.push(fields);
                    }
                    black_box(results)
                })
            });

            group.finish();
        }
    }
}

criterion_group!(benches, profile_dsv_parsing);
criterion_main!(benches);
