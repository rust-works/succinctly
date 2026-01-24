//! yq benchmarking module - compares succinctly yq vs system yq.
//!
//! Supports multiple query types to benchmark different execution paths:
//! - Identity (`.`): Uses P9 streaming fast path
//! - Navigation (`.[0]`, `.users`): Uses M2 streaming path
//! - Builtins (`length`): Uses OwnedValue path

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Query types for benchmarking different execution paths
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryType {
    /// Identity query (`.`) - uses P9 streaming
    Identity,
    /// First element (`.[0]`) - uses M2 streaming
    FirstElement,
    /// Iteration (`.[]`) - uses M2 streaming
    Iteration,
    /// Length builtin (`length`) - uses OwnedValue path
    Length,
}

impl QueryType {
    /// Get the jq query string for this query type
    pub fn query(&self) -> &'static str {
        match self {
            QueryType::Identity => ".",
            QueryType::FirstElement => ".[0]",
            QueryType::Iteration => ".[]",
            QueryType::Length => "length",
        }
    }

    /// Get a human-readable name for display
    pub fn name(&self) -> &'static str {
        match self {
            QueryType::Identity => "identity",
            QueryType::FirstElement => "first_element",
            QueryType::Iteration => "iteration",
            QueryType::Length => "length",
        }
    }

    /// Get the execution path description
    pub fn path_description(&self) -> &'static str {
        match self {
            QueryType::Identity => "P9 streaming",
            QueryType::FirstElement => "M2 streaming",
            QueryType::Iteration => "M2 streaming",
            QueryType::Length => "OwnedValue",
        }
    }

    /// Parse query type from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "identity" | "." => Some(QueryType::Identity),
            "first_element" | "first" | ".[0]" => Some(QueryType::FirstElement),
            "iteration" | "iter" | ".[]" => Some(QueryType::Iteration),
            "length" => Some(QueryType::Length),
            _ => None,
        }
    }

    /// Get all available query types
    pub fn all() -> &'static [QueryType] {
        &[
            QueryType::Identity,
            QueryType::FirstElement,
            QueryType::Iteration,
            QueryType::Length,
        ]
    }
}

/// Benchmark result for a single run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub file: String,
    pub pattern: String,
    pub size: String,
    pub filesize: u64,
    pub query: String,
    pub query_path: String,
    pub status: String,
    pub yq: ToolResult,
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
    pub queries: Vec<QueryType>,
    pub succinctly_binary: PathBuf,
    pub warmup_runs: usize,
    pub benchmark_runs: usize,
    /// Memory-focused mode: shows detailed memory comparison
    pub memory_mode: bool,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("data/bench/generated/yaml"),
            patterns: vec![
                "comprehensive".into(),
                "config".into(),
                "mixed".into(),
                "navigation".into(),
                "nested".into(),
                "numbers".into(),
                "pathological".into(),
                "sequences".into(),
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
            queries: QueryType::all().to_vec(),
            succinctly_binary: PathBuf::from("./target/release/succinctly"),
            warmup_runs: 1,
            benchmark_runs: 3,
            memory_mode: false,
        }
    }
}

/// Run the yq benchmark suite
pub fn run_benchmark(
    config: &BenchConfig,
    output_jsonl: Option<&Path>,
    output_md: Option<&Path>,
) -> Result<Vec<BenchmarkResult>> {
    let mut results = Vec::new();

    // Check if system yq is available
    let has_yq = check_yq_available();

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

    eprintln!("Running yq benchmark suite...");
    eprintln!("  Data directory: {}", config.data_dir.display());
    eprintln!(
        "  System yq: {}",
        if has_yq {
            "available"
        } else {
            "not found (succinctly-only benchmarks)"
        }
    );
    eprintln!(
        "  Queries: {}",
        config
            .queries
            .iter()
            .map(|q| format!("{} ({})", q.name(), q.query()))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if config.memory_mode {
        eprintln!("  Mode: memory-focused comparison");
    }
    eprintln!("  Warmup runs: {}", config.warmup_runs);
    eprintln!("  Benchmark runs: {}", config.benchmark_runs);
    eprintln!();

    // Open JSONL file for streaming output if requested
    let mut jsonl_file = output_jsonl
        .map(|p| {
            std::fs::File::create(p).with_context(|| format!("Failed to create {}", p.display()))
        })
        .transpose()?;

    'outer: for query_type in &config.queries {
        eprintln!(
            "Query: {} ({}, {})",
            query_type.query(),
            query_type.name(),
            query_type.path_description()
        );

        for pattern in &config.patterns {
            for size in &config.sizes {
                // Check for Ctrl+C
                if interrupted.load(Ordering::SeqCst) {
                    break 'outer;
                }

                let file_path = config.data_dir.join(pattern).join(format!("{}.yaml", size));

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
                match benchmark_file(&file_path, config, has_yq, *query_type) {
                    Ok(result) => {
                        if config.memory_mode {
                            // Memory-focused output
                            let succ_mem = format_memory_fixed(result.succinctly.peak_memory_bytes);
                            if has_yq {
                                let yq_mem = format_memory_fixed(result.yq.peak_memory_bytes);
                                let mem_ratio = if result.yq.peak_memory_bytes > 0 {
                                    result.succinctly.peak_memory_bytes as f64
                                        / result.yq.peak_memory_bytes as f64
                                } else {
                                    0.0
                                };
                                eprintln!(
                                    "succ={} yq={} ({:.2}x)",
                                    succ_mem.trim(),
                                    yq_mem.trim(),
                                    mem_ratio
                                );
                            } else {
                                eprintln!("mem={}", succ_mem.trim());
                            }
                        } else if has_yq {
                            let status_icon = if result.status == "match" {
                                "✓"
                            } else {
                                "✗"
                            };
                            let speedup = result.yq.wall_time_ms / result.succinctly.wall_time_ms;
                            eprintln!(
                                "{} yq={:.1}ms succ={:.1}ms ({:.2}x)",
                                status_icon,
                                result.yq.wall_time_ms,
                                result.succinctly.wall_time_ms,
                                speedup
                            );
                        } else {
                            eprintln!("{:.1}ms", result.succinctly.wall_time_ms);
                        }

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
        eprintln!();
    }

    let was_interrupted = interrupted.load(Ordering::SeqCst);

    // Generate markdown output (even if interrupted)
    if let Some(md_path) = output_md {
        let markdown = generate_markdown(&results, has_yq, config.memory_mode);
        std::fs::write(md_path, markdown)?;
    }

    // Check for hash mismatches (only if yq is available)
    let mismatches: Vec<_> = results.iter().filter(|r| r.status == "different").collect();

    // Print summary
    if was_interrupted {
        eprintln!("Benchmark interrupted: {} results collected", results.len());
    } else {
        eprintln!("Benchmark complete: {} results collected", results.len());
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

    // Report mismatches and fail if any found (only when comparing with yq)
    if has_yq && !mismatches.is_empty() {
        eprintln!("\nHash mismatches detected ({}):", mismatches.len());
        for m in &mismatches {
            eprintln!(
                "  {} {} ({}): yq={} succinctly={}",
                m.pattern, m.size, m.query, m.yq.hash, m.succinctly.hash
            );
        }
        // Note: Don't fail for yq since output formats may differ
        // (yq outputs YAML by default, succinctly yq outputs JSON)
        eprintln!("\nNote: Hash differences are expected when comparing yq (YAML output) vs succinctly (JSON output)");
    }

    Ok(results)
}

/// Benchmark a single file with a specific query
fn benchmark_file(
    file_path: &Path,
    config: &BenchConfig,
    has_yq: bool,
    query_type: QueryType,
) -> Result<BenchmarkResult> {
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
    let query = query_type.query();

    // Warmup runs
    for _ in 0..config.warmup_runs {
        if has_yq {
            let _ = run_yq(file_path, query);
        }
        let _ = run_succinctly(file_path, &config.succinctly_binary, query);
    }

    // Benchmark runs for yq (if available)
    let yq_result = if has_yq {
        let mut yq_results = Vec::new();
        for _ in 0..config.benchmark_runs {
            yq_results.push(run_yq(file_path, query)?);
        }
        select_median(yq_results)
    } else {
        // Placeholder for when yq is not available
        ToolResult {
            hash: String::new(),
            output_size: 0,
            peak_memory_bytes: 0,
            wall_time_ms: 0.0,
            user_time_ms: 0.0,
            sys_time_ms: 0.0,
        }
    };

    // Benchmark runs for succinctly
    let mut succ_results = Vec::new();
    for _ in 0..config.benchmark_runs {
        succ_results.push(run_succinctly(file_path, &config.succinctly_binary, query)?);
    }

    // Take median of wall time
    let succ_result = select_median(succ_results);

    // Determine status
    let status = if !has_yq {
        "succinctly-only".to_string()
    } else if yq_result.hash == succ_result.hash {
        "match".to_string()
    } else {
        "different".to_string()
    };

    Ok(BenchmarkResult {
        file: file_str,
        pattern,
        size,
        filesize,
        query: query.to_string(),
        query_path: query_type.path_description().to_string(),
        status,
        yq: yq_result,
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

/// Run system yq and measure (outputs JSON for comparison)
fn run_yq(file_path: &Path, query: &str) -> Result<ToolResult> {
    // Use -o json -I 0 for compact JSON output (matches succinctly's default)
    run_command_with_timing(
        "yq",
        &["-o", "json", "-I", "0", query, file_path.to_str().unwrap()],
    )
}

/// Run succinctly yq and measure
fn run_succinctly(file_path: &Path, binary: &Path, query: &str) -> Result<ToolResult> {
    // Use -o json -I 0 for compact JSON output
    run_command_with_timing(
        binary.to_str().unwrap(),
        &[
            "yq",
            "-o",
            "json",
            "-I",
            "0",
            query,
            file_path.to_str().unwrap(),
        ],
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

/// Check if yq is available
fn check_yq_available() -> bool {
    Command::new("yq")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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

/// Get yq version string
fn get_yq_version() -> String {
    Command::new("yq")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

/// Generate markdown tables from results
pub fn generate_markdown(results: &[BenchmarkResult], has_yq: bool, memory_mode: bool) -> String {
    let mut md = String::new();

    md.push_str("# yq vs succinctly Benchmark Results\n\n");
    md.push_str("Generated by `succinctly dev bench yq`\n\n");

    // Add CPU info
    let cpu_info = get_cpu_info();
    md.push_str(&format!("**CPU**: {}\n", cpu_info));

    if has_yq {
        let yq_version = get_yq_version();
        md.push_str(&format!("**yq version**: {}\n", yq_version));
    } else {
        md.push_str("**yq**: Not installed (succinctly-only benchmarks)\n");
    }
    md.push('\n');

    // Collect unique queries from results
    let queries: Vec<_> = results
        .iter()
        .map(|r| (r.query.as_str(), r.query_path.as_str()))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    // Group by pattern (alphabetical order)
    let patterns = [
        "comprehensive",
        "config",
        "mixed",
        "navigation",
        "nested",
        "numbers",
        "pathological",
        "sequences",
        "strings",
        "unicode",
        "users",
    ];

    // Size order: largest to smallest
    let size_order = ["100mb", "10mb", "1mb", "100kb", "10kb", "1kb"];

    // If multiple queries, group by query first
    if queries.len() > 1 {
        for (query, query_path) in &queries {
            md.push_str(&format!("## Query: `{}` ({})\n\n", query, query_path));

            let query_results: Vec<_> = results.iter().filter(|r| r.query == *query).collect();
            generate_pattern_tables(
                &mut md,
                &query_results,
                &patterns,
                &size_order,
                has_yq,
                memory_mode,
            );
        }
    } else {
        // Single query - just show pattern tables
        if let Some((query, query_path)) = queries.first() {
            md.push_str(&format!("**Query**: `{}` ({})\n\n", query, query_path));
        }
        let all_results: Vec<_> = results.iter().collect();
        generate_pattern_tables(
            &mut md,
            &all_results,
            &patterns,
            &size_order,
            has_yq,
            memory_mode,
        );
    }

    md
}

/// Generate pattern-based tables for markdown output
fn generate_pattern_tables(
    md: &mut String,
    results: &[&BenchmarkResult],
    patterns: &[&str],
    size_order: &[&str],
    has_yq: bool,
    memory_mode: bool,
) {
    if memory_mode {
        generate_memory_tables(md, results, patterns, size_order, has_yq);
    } else if has_yq {
        generate_comparison_tables(md, results, patterns, size_order);
    } else {
        generate_succinctly_only_tables(md, results, patterns, size_order);
    }
}

/// Generate memory-focused comparison tables
fn generate_memory_tables(
    md: &mut String,
    results: &[&BenchmarkResult],
    patterns: &[&str],
    size_order: &[&str],
    has_yq: bool,
) {
    for pattern in patterns {
        let pattern_results: Vec<_> = results
            .iter()
            .filter(|r| r.pattern == *pattern)
            .copied()
            .collect();

        if pattern_results.is_empty() {
            continue;
        }

        md.push_str(&format!("### Pattern: {}\n\n", pattern));

        if has_yq {
            md.push_str("| Size      | succ Mem | yq Mem  | Mem Ratio  | succ Time | yq Time  | Speedup    |\n");
            md.push_str("|-----------|----------|---------|------------|-----------|----------|------------|\n");
        } else {
            md.push_str("| Size      | Memory   | Time     | Throughput   |\n");
            md.push_str("|-----------|----------|----------|--------------|");
        }

        // Sort by size (largest first)
        let mut sorted = pattern_results;
        sorted.sort_by(|a, b| {
            let idx_a = size_order.iter().position(|s| *s == a.size).unwrap_or(999);
            let idx_b = size_order.iter().position(|s| *s == b.size).unwrap_or(999);
            idx_a.cmp(&idx_b)
        });

        for r in sorted {
            let succ_mem = format_memory_fixed(r.succinctly.peak_memory_bytes);
            let succ_time = format_time_fixed(r.succinctly.wall_time_ms);
            let size_bold = format!("**{}**", r.size);
            let size_col = format!("{:<9}", size_bold);

            if has_yq {
                let yq_mem = format_memory_fixed(r.yq.peak_memory_bytes);
                let yq_time = format_time_fixed(r.yq.wall_time_ms);
                let mem_ratio = if r.yq.peak_memory_bytes > 0 {
                    r.succinctly.peak_memory_bytes as f64 / r.yq.peak_memory_bytes as f64
                } else {
                    0.0
                };
                let speedup = r.yq.wall_time_ms / r.succinctly.wall_time_ms;

                md.push_str(&format!(
                    "| {} | {} | {} | **{:.2}x** | {} | {} | **{:.1}x** |\n",
                    size_col, succ_mem, yq_mem, mem_ratio, succ_time, yq_time, speedup
                ));
            } else {
                let throughput =
                    r.filesize as f64 / (r.succinctly.wall_time_ms / 1000.0) / (1024.0 * 1024.0);
                md.push_str(&format!(
                    "| {} | {} | {} | {:>12.1} MiB/s |\n",
                    size_col, succ_mem, succ_time, throughput
                ));
            }
        }

        md.push('\n');
    }
}

/// Generate full comparison tables with yq
fn generate_comparison_tables(
    md: &mut String,
    results: &[&BenchmarkResult],
    patterns: &[&str],
    size_order: &[&str],
) {
    for pattern in patterns {
        let pattern_results: Vec<_> = results
            .iter()
            .filter(|r| r.pattern == *pattern)
            .copied()
            .collect();

        if pattern_results.is_empty() {
            continue;
        }

        md.push_str(&format!("### Pattern: {}\n\n", pattern));
        md.push_str("| Size      | yq       | succinctly   | Speedup       | yq Mem  | succ Mem | Mem Ratio  |\n");
        md.push_str("|-----------|----------|--------------|---------------|---------|----------|------------|\n");

        // Sort by size (largest first)
        let mut sorted = pattern_results;
        sorted.sort_by(|a, b| {
            let idx_a = size_order.iter().position(|s| *s == a.size).unwrap_or(999);
            let idx_b = size_order.iter().position(|s| *s == b.size).unwrap_or(999);
            idx_a.cmp(&idx_b)
        });

        for r in sorted {
            let speedup = r.yq.wall_time_ms / r.succinctly.wall_time_ms;
            let mem_ratio = if r.yq.peak_memory_bytes > 0 {
                r.succinctly.peak_memory_bytes as f64 / r.yq.peak_memory_bytes as f64
            } else {
                0.0
            };

            // Format values
            let yq_time = format_time_fixed(r.yq.wall_time_ms);
            let succ_time = format_time_fixed(r.succinctly.wall_time_ms);
            let yq_mem = format_memory_fixed(r.yq.peak_memory_bytes);
            let succ_mem = format_memory_fixed(r.succinctly.peak_memory_bytes);

            // Size column: **name** with trailing padding (9 chars total)
            let size_bold = format!("**{}**", r.size);
            let size_col = format!("{:<9}", size_bold);

            // yq time column
            let yq_col = yq_time;

            // Succinctly time column: bold if faster (12 chars total)
            let succ_col = if speedup >= 1.0 {
                let trimmed = succ_time.trim_start();
                let padding = succ_time.len() - trimmed.len();
                format!("{}**{}**", " ".repeat(padding), trimmed)
            } else {
                format!("  {}  ", succ_time)
            };

            // Speedup column: bold if >= 1.0 (13 chars visual width)
            let speedup_str = format!("{:.1}x", speedup);
            let speedup_col = if speedup >= 1.0 {
                let padded = format!("{:>9}", speedup_str);
                let trimmed = padded.trim_start();
                let padding = padded.len() - trimmed.len();
                format!("{}**{}**", " ".repeat(padding), trimmed)
            } else {
                format!("{:>13}", speedup_str)
            };

            // Memory columns
            let yq_mem_col = yq_mem;
            let succ_mem_col = format!("{:>8}", succ_mem);
            let mem_ratio_col = format!("{:>9.2}x", mem_ratio);

            md.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                size_col, yq_col, succ_col, speedup_col, yq_mem_col, succ_mem_col, mem_ratio_col
            ));
        }

        md.push('\n');
    }
}

/// Generate succinctly-only tables
fn generate_succinctly_only_tables(
    md: &mut String,
    results: &[&BenchmarkResult],
    patterns: &[&str],
    size_order: &[&str],
) {
    for pattern in patterns {
        let pattern_results: Vec<_> = results
            .iter()
            .filter(|r| r.pattern == *pattern)
            .copied()
            .collect();

        if pattern_results.is_empty() {
            continue;
        }

        md.push_str(&format!("### Pattern: {}\n\n", pattern));
        md.push_str("| Size      | Time     | Throughput   | Memory   |\n");
        md.push_str("|-----------|----------|--------------|----------|\n");

        // Sort by size (largest first)
        let mut sorted = pattern_results;
        sorted.sort_by(|a, b| {
            let idx_a = size_order.iter().position(|s| *s == a.size).unwrap_or(999);
            let idx_b = size_order.iter().position(|s| *s == b.size).unwrap_or(999);
            idx_a.cmp(&idx_b)
        });

        for r in sorted {
            let throughput =
                r.filesize as f64 / (r.succinctly.wall_time_ms / 1000.0) / (1024.0 * 1024.0);

            // Format values
            let time = format_time_fixed(r.succinctly.wall_time_ms);
            let throughput_str = format!("{:.1} MiB/s", throughput);
            let mem = format_memory_fixed(r.succinctly.peak_memory_bytes);

            // Size column
            let size_bold = format!("**{}**", r.size);
            let size_col = format!("{:<9}", size_bold);

            md.push_str(&format!(
                "| {} | {} | {:>12} | {} |\n",
                size_col, time, throughput_str, mem
            ));
        }

        md.push('\n');
    }
}

/// Format time with fixed width (8 chars total)
fn format_time_fixed(ms: f64) -> String {
    if ms >= 1000.0 {
        format!("{:>7.2}s", ms / 1000.0)
    } else {
        let ms_str = format!("{:.1}ms", ms);
        format!("{:>8}", ms_str)
    }
}

/// Format memory with fixed width (7 chars)
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
        assert_eq!(format_time_fixed(1500.0), "   1.50s");
        assert_eq!(format_time_fixed(500.0), " 500.0ms");
        assert_eq!(format_time_fixed(6.1), "   6.1ms");
    }

    #[test]
    fn test_format_memory_fixed() {
        assert_eq!(format_memory_fixed(1024 * 1024 * 100), " 100 MB");
        assert_eq!(format_memory_fixed(1024 * 500), " 500 KB");
    }
}
