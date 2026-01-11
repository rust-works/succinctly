use std::time::Instant;
use succinctly::dsv::{build_index, Dsv, DsvConfig};

fn main() {
    // Generate test data - ~4MB
    let mut data = Vec::new();
    for i in 0..100_000 {
        data.extend_from_slice(format!("field1_{},field2_{},field3_{}\n", i, i*2, i*3).as_bytes());
    }

    let config = DsvConfig::default();
    let dsv = Dsv::parse_with_config(&data, &config);

    println!("Data size: {} bytes ({:.2} MB)\n", data.len(), data.len() as f64 / 1_048_576.0);

    // Test 1: Parse only
    let start = Instant::now();
    let _index = build_index(&data, &config);
    let parse_time = start.elapsed();
    println!("Parse only: {:.2}ms ({:.1} MiB/s)",
        parse_time.as_secs_f64() * 1000.0,
        data.len() as f64 / 1_048_576.0 / parse_time.as_secs_f64());

    // Test 2: Iterate fields (no string conversion)
    let start = Instant::now();
    let mut total_fields = 0;
    for row in dsv.rows() {
        for _field in row.fields() {
            total_fields += 1;
        }
    }
    let iter_time = start.elapsed();
    println!("Iteration only: {:.2}ms ({:.1} MiB/s) - {} fields",
        iter_time.as_secs_f64() * 1000.0,
        data.len() as f64 / 1_048_576.0 / iter_time.as_secs_f64(),
        total_fields);

    // Test 3: Iterate + string conversion
    let start = Instant::now();
    let mut total_len = 0;
    for row in dsv.rows() {
        for field in row.fields() {
            if let Ok(_s) = std::str::from_utf8(field) {
                total_len += field.len();
            }
        }
    }
    let convert_time = start.elapsed();
    println!("Iteration + UTF-8: {:.2}ms ({:.1} MiB/s) - {} bytes",
        convert_time.as_secs_f64() * 1000.0,
        data.len() as f64 / 1_048_576.0 / convert_time.as_secs_f64(),
        total_len);
}
