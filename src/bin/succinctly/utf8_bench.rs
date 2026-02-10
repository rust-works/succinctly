//! UTF-8 validation benchmarking module.
//!
//! Benchmarks the scalar UTF-8 validation implementation across different
//! content patterns and sizes. This serves as the baseline for future
//! SIMD implementations.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Benchmark result for a single run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub file: String,
    pub pattern: String,
    pub size: String,
    pub filesize: u64,
    pub valid: bool,
    pub wall_time_ms: f64,
    pub throughput_mib_s: f64,
}

/// Configuration for the benchmark
#[derive(Debug, Clone)]
pub struct BenchConfig {
    pub data_dir: PathBuf,
    pub patterns: Vec<String>,
    pub sizes: Vec<String>,
    pub warmup_runs: usize,
    pub benchmark_runs: usize,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("data/bench/generated/utf8"),
            patterns: vec![
                "ascii".into(),
                "latin".into(),
                "greek_cyrillic".into(),
                "cjk".into(),
                "emoji".into(),
                "mixed".into(),
                "all_lengths".into(),
                "log_file".into(),
                "source_code".into(),
                "json_like".into(),
                "pathological".into(),
            ],
            sizes: vec![
                "1kb".into(),
                "10kb".into(),
                "100kb".into(),
                "1mb".into(),
                "10mb".into(),
                "100mb".into(),
            ],
            warmup_runs: 1,
            benchmark_runs: 3,
        }
    }
}

/// Run the UTF-8 benchmark suite
pub fn run_benchmark(
    config: &BenchConfig,
    output_jsonl: Option<&Path>,
    output_md: Option<&Path>,
) -> Result<Vec<BenchmarkResult>> {
    let mut results = Vec::new();

    // Set up Ctrl+C handler
    let interrupted = Arc::new(AtomicBool::new(false));
    let interrupted_clone = Arc::clone(&interrupted);
    ctrlc::set_handler(move || {
        interrupted_clone.store(true, Ordering::SeqCst);
        eprintln!("\nInterrupted! Writing partial results...");
    })
    .context("Failed to set Ctrl+C handler")?;

    eprintln!("Running UTF-8 validation benchmark suite (scalar)...");
    eprintln!("  Data directory: {}", config.data_dir.display());
    eprintln!("  Warmup runs: {}", config.warmup_runs);
    eprintln!("  Benchmark runs: {}", config.benchmark_runs);
    eprintln!();

    // Open JSONL file for streaming output if requested
    let mut jsonl_file = output_jsonl
        .map(|p| {
            std::fs::File::create(p).with_context(|| format!("Failed to create {}", p.display()))
        })
        .transpose()?;

    'outer: for pattern in &config.patterns {
        for size in &config.sizes {
            // Check for Ctrl+C
            if interrupted.load(Ordering::SeqCst) {
                break 'outer;
            }

            let file_path = config.data_dir.join(pattern).join(format!("{}.txt", size));

            if !file_path.exists() {
                eprintln!("  Skipping {} (not found)", file_path.display());
                continue;
            }

            let filesize = std::fs::metadata(&file_path)?.len();

            eprint!(
                "  {} {} ({})... ",
                pattern,
                size,
                format_bytes(filesize as usize)
            );
            std::io::stderr().flush()?;

            // Run benchmark
            match benchmark_file(&file_path, config) {
                Ok(result) => {
                    eprintln!(
                        "{:.2}ms ({:.1} MiB/s){}",
                        result.wall_time_ms,
                        result.throughput_mib_s,
                        if result.valid { "" } else { " [INVALID]" }
                    );

                    // Write to JSONL immediately
                    if let Some(ref mut f) = jsonl_file {
                        serde_json::to_writer(&mut *f, &result)?;
                        writeln!(f)?;
                        f.flush()?;
                    }

                    results.push(result);
                }
                Err(e) => {
                    eprintln!("ERROR: {}", e);
                }
            }
        }
    }

    // Write markdown summary if requested
    if let Some(md_path) = output_md {
        write_markdown_summary(&results, md_path)?;
    }

    // Print summary
    eprintln!();
    eprintln!("Completed {} benchmarks", results.len());

    Ok(results)
}

/// Benchmark a single file
fn benchmark_file(file_path: &Path, config: &BenchConfig) -> Result<BenchmarkResult> {
    let data = std::fs::read(file_path)?;
    let filesize = data.len() as u64;

    let pattern = file_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let size = file_path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    // Warmup
    for _ in 0..config.warmup_runs {
        let _ = succinctly::text::utf8::validate_utf8(&data);
    }

    // Benchmark
    let mut times = Vec::with_capacity(config.benchmark_runs);
    let mut valid = false;
    for _ in 0..config.benchmark_runs {
        let start = Instant::now();
        valid = succinctly::text::utf8::validate_utf8(&data).is_ok();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = times[times.len() / 2];

    // Calculate throughput in MiB/s
    let throughput = (filesize as f64 / (1024.0 * 1024.0)) / (median / 1000.0);

    Ok(BenchmarkResult {
        file: file_path.display().to_string(),
        pattern,
        size,
        filesize,
        valid,
        wall_time_ms: median,
        throughput_mib_s: throughput,
    })
}

/// Write markdown summary
fn write_markdown_summary(results: &[BenchmarkResult], path: &Path) -> Result<()> {
    let mut md = String::new();

    md.push_str("# UTF-8 Validation Benchmark Results (Scalar)\n\n");
    md.push_str("Baseline scalar implementation of `succinctly::text::utf8::validate_utf8`.\n\n");

    // Group by pattern
    let mut patterns: Vec<&str> = results.iter().map(|r| r.pattern.as_str()).collect();
    patterns.sort();
    patterns.dedup();

    for pattern in patterns {
        md.push_str(&format!("## {}\n\n", pattern));
        md.push_str("| Size | Time (ms) | Throughput (MiB/s) |\n");
        md.push_str("|------|-----------|--------------------|\n");

        for result in results.iter().filter(|r| r.pattern == pattern) {
            md.push_str(&format!(
                "| {} | {:.2} | {:.1} |\n",
                result.size, result.wall_time_ms, result.throughput_mib_s
            ));
        }
        md.push('\n');
    }

    std::fs::write(path, md)?;
    Ok(())
}

/// Format bytes as human-readable string
fn format_bytes(bytes: usize) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.00 GB");
    }
}
