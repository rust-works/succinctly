//! jq benchmarking module - compares succinctly jq vs system jq.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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
    pub status: String,
    pub jq: ToolResult,
    pub succinctly: ToolResult,
}

/// Result from running a single tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub hash: String,
    pub output_size: u64,
    pub peak_memory_bytes: u64,
    pub wall_time_ms: f64,
    pub user_time_ms: f64,
    pub sys_time_ms: f64,
}

/// Configuration for the benchmark
#[derive(Debug, Clone)]
pub struct BenchConfig {
    pub data_dir: PathBuf,
    pub patterns: Vec<String>,
    pub sizes: Vec<String>,
    pub succinctly_binary: PathBuf,
    pub warmup_runs: usize,
    pub benchmark_runs: usize,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("data/bench/generated"),
            patterns: vec![
                "arrays".into(),
                "comprehensive".into(),
                "literals".into(),
                "mixed".into(),
                "nested".into(),
                "numbers".into(),
                "pathological".into(),
                "strings".into(),
                "unicode".into(),
                "users".into(),
            ],
            sizes: vec![
                "1kb".into(),
                "10kb".into(),
                "100kb".into(),
                "1mb".into(),
                "10mb".into(),
                "100mb".into(),
            ],
            succinctly_binary: PathBuf::from("./target/release/succinctly"),
            warmup_runs: 1,
            benchmark_runs: 3,
        }
    }
}

/// Run the jq benchmark suite
pub fn run_benchmark(
    config: &BenchConfig,
    output_jsonl: Option<&Path>,
    output_md: Option<&Path>,
) -> Result<Vec<BenchmarkResult>> {
    let mut results = Vec::new();

    // Check that jq is available
    check_jq_available()?;

    // Check that succinctly binary exists
    if !config.succinctly_binary.exists() {
        anyhow::bail!(
            "succinctly binary not found at {}. Run: cargo build --release --features cli",
            config.succinctly_binary.display()
        );
    }

    // Set up Ctrl+C handler
    let interrupted = Arc::new(AtomicBool::new(false));
    let interrupted_clone = Arc::clone(&interrupted);
    ctrlc::set_handler(move || {
        interrupted_clone.store(true, Ordering::SeqCst);
        eprintln!("\nInterrupted! Writing partial results...");
    })
    .context("Failed to set Ctrl+C handler")?;

    eprintln!("Running jq benchmark suite...");
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

            let file_path = config.data_dir.join(pattern).join(format!("{}.json", size));

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
                    let status_icon = if result.status == "match" {
                        "✓"
                    } else {
                        "✗"
                    };
                    let speedup = result.jq.wall_time_ms / result.succinctly.wall_time_ms;
                    eprintln!(
                        "{} jq={:.1}ms succ={:.1}ms ({:.2}x)",
                        status_icon,
                        result.jq.wall_time_ms,
                        result.succinctly.wall_time_ms,
                        speedup
                    );

                    // Write to JSONL file immediately
                    if let Some(ref mut f) = jsonl_file {
                        writeln!(f, "{}", serde_json::to_string(&result)?)?;
                    }

                    results.push(result);
                }
                Err(e) => {
                    eprintln!("ERROR: {}", e);
                }
            }
        }
    }

    let was_interrupted = interrupted.load(Ordering::SeqCst);

    // Generate markdown output (even if interrupted)
    if let Some(md_path) = output_md {
        let markdown = generate_markdown(&results);
        std::fs::write(md_path, markdown)?;
    }

    // Check for hash mismatches
    let mismatches: Vec<_> = results.iter().filter(|r| r.status != "match").collect();

    // Print summary
    if was_interrupted {
        eprintln!("\nBenchmark interrupted: {} files processed", results.len());
    } else {
        eprintln!("\nBenchmark complete: {} files processed", results.len());
    }

    // Print paths to generated files
    if !results.is_empty() {
        eprintln!("\nGenerated files:");
        if let Some(jsonl_path) = output_jsonl {
            eprintln!("  {}", jsonl_path.display());
        }
        if let Some(md_path) = output_md {
            eprintln!("  {}", md_path.display());
        }
    }

    // Report mismatches and fail if any found
    if !mismatches.is_empty() {
        eprintln!("\nHash mismatches detected ({}):", mismatches.len());
        for m in &mismatches {
            eprintln!(
                "  {} {}: jq={} succinctly={}",
                m.pattern, m.size, m.jq.hash, m.succinctly.hash
            );
        }
        anyhow::bail!(
            "{} hash mismatch(es) detected - jq and succinctly produced different output",
            mismatches.len()
        );
    }

    Ok(results)
}

/// Benchmark a single file
fn benchmark_file(file_path: &Path, config: &BenchConfig) -> Result<BenchmarkResult> {
    let file_str = file_path.to_string_lossy().to_string();
    let pattern = file_path
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let size = file_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let filesize = std::fs::metadata(file_path)?.len();

    // Warmup runs
    for _ in 0..config.warmup_runs {
        let _ = run_jq(file_path);
        let _ = run_succinctly(file_path, &config.succinctly_binary);
    }

    // Benchmark runs for jq
    let mut jq_results = Vec::new();
    for _ in 0..config.benchmark_runs {
        jq_results.push(run_jq(file_path)?);
    }

    // Benchmark runs for succinctly
    let mut succ_results = Vec::new();
    for _ in 0..config.benchmark_runs {
        succ_results.push(run_succinctly(file_path, &config.succinctly_binary)?);
    }

    // Take median of wall time
    let jq_result = select_median(jq_results);
    let succ_result = select_median(succ_results);

    // Check if outputs match
    let status = if jq_result.hash == succ_result.hash {
        "match".to_string()
    } else {
        "different".to_string()
    };

    Ok(BenchmarkResult {
        file: file_str,
        pattern,
        size,
        filesize,
        status,
        jq: jq_result,
        succinctly: succ_result,
    })
}

/// Select the run with median wall time
fn select_median(mut results: Vec<ToolResult>) -> ToolResult {
    if results.is_empty() {
        panic!("No results to select from");
    }
    results.sort_by(|a, b| a.wall_time_ms.partial_cmp(&b.wall_time_ms).unwrap());
    results.remove(results.len() / 2)
}

/// Run system jq and measure
fn run_jq(file_path: &Path) -> Result<ToolResult> {
    run_command_with_timing("jq", &[".", file_path.to_str().unwrap()])
}

/// Run succinctly jq and measure
fn run_succinctly(file_path: &Path, binary: &Path) -> Result<ToolResult> {
    run_command_with_timing(
        binary.to_str().unwrap(),
        &["jq", ".", file_path.to_str().unwrap()],
    )
}

/// Run a command and measure timing/memory
fn run_command_with_timing(program: &str, args: &[&str]) -> Result<ToolResult> {
    let start = Instant::now();

    // Use /usr/bin/time for memory measurement on macOS/Linux
    let (output, peak_memory, user_time, sys_time) = if cfg!(target_os = "macos") {
        run_with_time_macos(program, args)?
    } else if cfg!(target_os = "linux") {
        run_with_time_linux(program, args)?
    } else {
        // Fallback: no memory measurement
        let output = Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("Failed to run {}", program))?;
        (output.stdout, 0, 0.0, 0.0)
    };

    let wall_time = start.elapsed();

    // Calculate MD5 hash of output
    let hash = format!("{:x}", md5::compute(&output));

    Ok(ToolResult {
        hash,
        output_size: output.len() as u64,
        peak_memory_bytes: peak_memory,
        wall_time_ms: wall_time.as_secs_f64() * 1000.0,
        user_time_ms: user_time * 1000.0,
        sys_time_ms: sys_time * 1000.0,
    })
}

/// Run command with /usr/bin/time on macOS
fn run_with_time_macos(program: &str, args: &[&str]) -> Result<(Vec<u8>, u64, f64, f64)> {
    let mut time_args = vec!["-l", program];
    time_args.extend(args);

    let output = Command::new("/usr/bin/time")
        .args(&time_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| "Failed to run /usr/bin/time")?;

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Parse macOS time output
    let peak_memory = parse_macos_memory(&stderr);
    let (user_time, sys_time) = parse_macos_times(&stderr);

    Ok((output.stdout, peak_memory, user_time, sys_time))
}

/// Run command with /usr/bin/time on Linux
fn run_with_time_linux(program: &str, args: &[&str]) -> Result<(Vec<u8>, u64, f64, f64)> {
    let mut time_args = vec!["-v", program];
    time_args.extend(args);

    let output = Command::new("/usr/bin/time")
        .args(&time_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| "Failed to run /usr/bin/time")?;

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Parse Linux time output
    let peak_memory = parse_linux_memory(&stderr);
    let (user_time, sys_time) = parse_linux_times(&stderr);

    Ok((output.stdout, peak_memory, user_time, sys_time))
}

/// Parse peak memory from macOS time output (bytes)
fn parse_macos_memory(stderr: &str) -> u64 {
    for line in stderr.lines() {
        if line.contains("maximum resident set size") {
            if let Some(num) = line.split_whitespace().next() {
                if let Ok(bytes) = num.parse::<u64>() {
                    return bytes;
                }
            }
        }
    }
    0
}

/// Parse user/sys time from macOS time output (seconds)
fn parse_macos_times(stderr: &str) -> (f64, f64) {
    let mut user = 0.0;
    let mut sys = 0.0;

    for line in stderr.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            if parts[1] == "user" || line.ends_with(" user") {
                if let Ok(t) = parts[0].parse::<f64>() {
                    user = t;
                }
            } else if parts[1] == "sys" || line.ends_with(" sys") {
                if let Ok(t) = parts[0].parse::<f64>() {
                    sys = t;
                }
            }
        }
    }

    (user, sys)
}

/// Parse peak memory from Linux time output (KB -> bytes)
fn parse_linux_memory(stderr: &str) -> u64 {
    for line in stderr.lines() {
        if line.contains("Maximum resident set size") {
            if let Some(num_str) = line.split(':').next_back() {
                if let Ok(kb) = num_str.trim().parse::<u64>() {
                    return kb * 1024;
                }
            }
        }
    }
    0
}

/// Parse user/sys time from Linux time output (seconds)
fn parse_linux_times(stderr: &str) -> (f64, f64) {
    let mut user = 0.0;
    let mut sys = 0.0;

    for line in stderr.lines() {
        if line.contains("User time") {
            if let Some(num_str) = line.split(':').next_back() {
                if let Ok(t) = num_str.trim().parse::<f64>() {
                    user = t;
                }
            }
        } else if line.contains("System time") {
            if let Some(num_str) = line.split(':').next_back() {
                if let Ok(t) = num_str.trim().parse::<f64>() {
                    sys = t;
                }
            }
        }
    }

    (user, sys)
}

/// Check if jq is available
fn check_jq_available() -> Result<()> {
    Command::new("jq").arg("--version").output().context(
        "jq not found. Please install jq: brew install jq (macOS) or apt install jq (Linux)",
    )?;
    Ok(())
}

/// Get CPU information for the current system
fn get_cpu_info() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
        {
            if output.status.success() {
                return String::from_utf8_lossy(&output.stdout).trim().to_string();
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/cpuinfo") {
            for line in content.lines() {
                if line.starts_with("model name") {
                    if let Some(name) = line.split(':').nth(1) {
                        return name.trim().to_string();
                    }
                }
            }
        }
    }

    "Unknown CPU".to_string()
}

/// Generate markdown tables from results
pub fn generate_markdown(results: &[BenchmarkResult]) -> String {
    let mut md = String::new();

    md.push_str("# jq vs succinctly Benchmark Results\n\n");
    md.push_str("Generated by `succinctly dev bench jq`\n\n");

    // Add CPU info
    let cpu_info = get_cpu_info();
    md.push_str(&format!("**CPU**: {}\n\n", cpu_info));

    // Group by pattern (alphabetical order)
    let patterns = [
        "arrays",
        "comprehensive",
        "literals",
        "mixed",
        "nested",
        "numbers",
        "pathological",
        "strings",
        "unicode",
        "users",
    ];

    // Size order: largest to smallest
    let size_order = ["100mb", "10mb", "1mb", "100kb", "10kb", "1kb"];

    for pattern in patterns {
        let pattern_results: Vec<_> = results.iter().filter(|r| r.pattern == pattern).collect();

        if pattern_results.is_empty() {
            continue;
        }

        md.push_str(&format!("## Pattern: {}\n\n", pattern));
        // Column widths (content only, not including | separators):
        // Size: 9 chars (e.g., "**10mb** ")
        // jq: 8 chars (e.g., "  336.7ms")
        // succinctly: 12 chars (e.g., "**  57.9ms**")
        // Speedup: 13 chars (e.g., "**     5.8x**")
        // jq Mem: 7 chars (e.g., "  25 MB")
        // succ Mem: 8 chars (e.g., "   29 MB")
        // Mem Ratio: 10 chars (e.g., "     1.16x")
        md.push_str("| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |\n");
        md.push_str("|-----------|----------|--------------|---------------|---------|----------|------------|\n");

        // Sort by size (largest first)
        let mut sorted = pattern_results.clone();
        sorted.sort_by(|a, b| {
            let idx_a = size_order.iter().position(|s| *s == a.size).unwrap_or(999);
            let idx_b = size_order.iter().position(|s| *s == b.size).unwrap_or(999);
            idx_a.cmp(&idx_b)
        });

        for r in sorted {
            let speedup = r.jq.wall_time_ms / r.succinctly.wall_time_ms;
            let mem_ratio = r.succinctly.peak_memory_bytes as f64 / r.jq.peak_memory_bytes as f64;

            // Format values - format_time_fixed returns 8 chars, format_memory_fixed returns 7 chars
            let jq_time = format_time_fixed(r.jq.wall_time_ms);
            let succ_time = format_time_fixed(r.succinctly.wall_time_ms);
            let jq_mem = format_memory_fixed(r.jq.peak_memory_bytes);
            let succ_mem = format_memory_fixed(r.succinctly.peak_memory_bytes);

            // Size column: **name** with trailing padding (9 chars total)
            let size_bold = format!("**{}**", r.size);
            let size_col = format!("{:<9}", size_bold);

            // jq time column: already 8 chars from format_time_fixed
            let jq_col = jq_time;

            // Succinctly time column: bold if faster (12 chars total)
            // succ_time is 8 chars with leading spaces for alignment
            // Move leading spaces outside ** so markdown renders correctly
            let succ_col = if speedup >= 1.0 {
                let trimmed = succ_time.trim_start();
                let padding = succ_time.len() - trimmed.len();
                format!("{}**{}**", " ".repeat(padding), trimmed) // spaces before **, not after
            } else {
                format!("  {}  ", succ_time) // 8 + 4 = 12 chars, same visual width
            };

            // Speedup column: bold if >= 1.0 (13 chars visual width)
            // Move leading spaces outside ** so markdown renders correctly
            let speedup_str = format!("{:.1}x", speedup);
            let speedup_col = if speedup >= 1.0 {
                // Right-align: total 9 chars visual, with ** markers = 13 chars
                let padded = format!("{:>9}", speedup_str);
                let trimmed = padded.trim_start();
                let padding = padded.len() - trimmed.len();
                format!("{}**{}**", " ".repeat(padding), trimmed)
            } else {
                format!("{:>13}", speedup_str) // Same visual width without bold
            };

            // Memory columns: right-aligned, format_memory_fixed returns 7 chars
            let jq_mem_col = jq_mem;
            let succ_mem_col = format!("{:>8}", succ_mem); // 8 chars for succ Mem column
            let mem_ratio_col = format!("{:>9.2}x", mem_ratio);

            md.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                size_col, jq_col, succ_col, speedup_col, jq_mem_col, succ_mem_col, mem_ratio_col
            ));
        }

        md.push('\n');
    }

    md
}

/// Format time with fixed width (8 chars total)
/// Examples: "   1.04s", " 765.6ms", "   6.1ms"
fn format_time_fixed(ms: f64) -> String {
    if ms >= 1000.0 {
        // Seconds: "   1.04s" = 8 chars total
        format!("{:>7.2}s", ms / 1000.0)
    } else {
        // Milliseconds: " 765.6ms" = 8 chars total (padding on left)
        let ms_str = format!("{:.1}ms", ms);
        format!("{:>8}", ms_str)
    }
}

/// Format memory with fixed width (7 chars: 4 for number + space + 2 for unit)
/// Examples: "1234 MB", " 123 MB", "  12 MB"
fn format_memory_fixed(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:>4.0} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:>4.0} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:>4.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:>4} B ", bytes)
    }
}

/// Format bytes to human-readable string (with decimals)
fn format_bytes(bytes: usize) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} bytes", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_time_fixed() {
        // 8 chars total for each
        assert_eq!(format_time_fixed(1500.0), "   1.50s");
        assert_eq!(format_time_fixed(500.0), " 500.0ms");
        assert_eq!(format_time_fixed(6.1), "   6.1ms");
    }

    #[test]
    fn test_format_memory_fixed() {
        assert_eq!(format_memory_fixed(1024 * 1024 * 100), " 100 MB");
        assert_eq!(format_memory_fixed(1024 * 500), " 500 KB");
    }

    #[test]
    fn test_bold_formatting_spaces_outside() {
        // Simulate what the code does for succ_col when speedup >= 1.0
        let succ_time = format_time_fixed(59.6); // "  59.6ms" (8 chars)
        let trimmed = succ_time.trim_start();
        let padding = succ_time.len() - trimmed.len();
        let succ_col = format!("{}**{}**", " ".repeat(padding), trimmed);

        // Spaces should be BEFORE **, not after
        assert!(
            succ_col.starts_with("  **"),
            "Expected spaces before **, got: {}",
            succ_col
        );
        assert!(
            !succ_col.contains("** "),
            "Spaces should not be inside **: {}",
            succ_col
        );
        assert_eq!(succ_col, "  **59.6ms**");
    }

    #[test]
    fn test_speedup_bold_formatting() {
        // Simulate speedup column formatting
        let speedup = 5.8;
        let speedup_str = format!("{:.1}x", speedup);
        let padded = format!("{:>9}", speedup_str);
        let trimmed = padded.trim_start();
        let padding = padded.len() - trimmed.len();
        let speedup_col = format!("{}**{}**", " ".repeat(padding), trimmed);

        // Spaces should be BEFORE **, not after
        assert!(
            speedup_col.contains("**5.8x**"),
            "Expected **5.8x**, got: {}",
            speedup_col
        );
        assert!(
            !speedup_col.contains("** "),
            "Spaces should not be inside **: {}",
            speedup_col
        );
    }
}
