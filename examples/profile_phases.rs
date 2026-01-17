// Profile phases script - run with: cargo run --release --example profile_phases --features cli
use std::time::Instant;
use succinctly::yaml::YamlIndex;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("Usage: profile_phases <file>");

    // Read file
    let t0 = Instant::now();
    let data = std::fs::read(path).expect("failed to read file");
    let read_time = t0.elapsed();

    // Build index
    let t1 = Instant::now();
    let index = YamlIndex::build(&data).expect("failed to build index");
    let build_time = t1.elapsed();

    // Access root (cursor creation)
    let t2 = Instant::now();
    let root = index.root(&data);
    let cursor_time = t2.elapsed();

    // Convert to JSON
    let t3 = Instant::now();
    let json = root.to_json_document();
    let json_time = t3.elapsed();

    // Print stats
    println!("File size: {} bytes", data.len());
    println!("Read:      {:>8.3} ms", read_time.as_secs_f64() * 1000.0);
    println!(
        "Build:     {:>8.3} ms ({:.1} MiB/s)",
        build_time.as_secs_f64() * 1000.0,
        data.len() as f64 / build_time.as_secs_f64() / 1024.0 / 1024.0
    );
    println!("Cursor:    {:>8.3} ms", cursor_time.as_secs_f64() * 1000.0);
    println!(
        "To JSON:   {:>8.3} ms ({:.1} MiB/s)",
        json_time.as_secs_f64() * 1000.0,
        data.len() as f64 / json_time.as_secs_f64() / 1024.0 / 1024.0
    );
    println!("JSON len:  {} bytes", json.len());
}
