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

use crate::bench_runner::utils::{ensure_success, interleaved_reps};

use crate::yaml_generators;

/// Every generated suite pattern, alphabetically.
///
/// Derived from `yaml_generators::ALL_PATTERNS` rather than duplicated, so a
/// pattern added to the suite is benchmarked by `--patterns all` and appears in
/// the markdown tables without a second edit (#327).
fn pattern_names() -> Vec<String> {
    let mut names: Vec<String> = yaml_generators::ALL_PATTERNS
        .iter()
        .map(|(name, _, _)| (*name).to_string())
        .collect();
    names.sort_unstable();
    names
}

/// Query types for benchmarking different execution paths
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryType {
    /// Identity query (`.`) - uses P9 streaming
    Identity,
    /// First element (`.[0]`) - uses M2 streaming
    FirstElement,
    /// Iteration (`.[]`) - uses M2 streaming
    Iteration,
    /// Length builtin (`length`) - produces computed OwnedValue (not cursor-streamable)
    Length,
}

impl QueryType {
    /// Get the jq query string for this query type
    pub fn query(&self) -> &'static str {
        match self {
            Self::Identity => ".",
            Self::FirstElement => ".[0]",
            Self::Iteration => ".[]",
            Self::Length => "length",
        }
    }

    /// Get a human-readable name for display
    pub fn name(&self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::FirstElement => "first_element",
            Self::Iteration => "iteration",
            Self::Length => "length",
        }
    }

    /// Get the execution path description
    pub fn path_description(&self) -> &'static str {
        match self {
            Self::Identity => "P9 streaming",
            Self::FirstElement => "M2 streaming",
            Self::Iteration => "M2 streaming",
            Self::Length => "OwnedValue",
        }
    }

    /// Parse query type from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "identity" | "." => Some(Self::Identity),
            "first_element" | "first" | ".[0]" => Some(Self::FirstElement),
            "iteration" | "iter" | ".[]" => Some(Self::Iteration),
            "length" => Some(Self::Length),
            _ => None,
        }
    }

    /// Get all available query types
    pub fn all() -> &'static [Self] {
        &[
            Self::Identity,
            Self::FirstElement,
            Self::Iteration,
            Self::Length,
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
    /// The yq to compare against (a bare name is looked up on `PATH`).
    pub yq_binary: PathBuf,
    /// Whether `yq_binary` was asked for by name (`--yq`): a yq that will not run is then an
    /// error, not the succinctly-only run an absent default `yq` gets.
    pub yq_required: bool,
    pub warmup_runs: usize,
    pub benchmark_runs: usize,
    /// Include memory comparison in output (default: true)
    pub memory_mode: bool,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("data/bench/generated/yaml"),
            patterns: pattern_names(),
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
            yq_binary: PathBuf::from("yq"),
            yq_required: false,
            warmup_runs: 1,
            benchmark_runs: 3,
            memory_mode: true,
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

    // Check if yq is available, and record which one the tables are measured against
    let yq_version = check_yq_available(&config.yq_binary);
    if yq_version.is_none() && config.yq_required {
        anyhow::bail!(
            "yq not found at `{}`. Pass --yq <path> to a yq that runs, or drop --yq to benchmark \
             succinctly alone",
            config.yq_binary.display()
        );
    }
    let has_yq = yq_version.is_some();

    // Check that succinctly binary exists
    if !config.succinctly_binary.exists() {
        anyhow::bail!(
            "succinctly binary not found at {}. Run: cargo build --release --features cli",
            config.succinctly_binary.display()
        );
    }

    // Set up Ctrl+C handler
    let interrupted = Arc::new(AtomicBool::new(false));
    crate::interrupt::on_interrupt(&interrupted, "Interrupted! Writing partial results...")?;

    eprintln!("Running yq benchmark suite...");
    eprintln!("  Data directory: {}", config.data_dir.display());
    eprintln!(
        "  yq: {}",
        match &yq_version {
            Some(version) => format!("{} ({version})", config.yq_binary.display()),
            None => "not found (succinctly-only benchmarks)".to_string(),
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

                let file_path = config.data_dir.join(pattern).join(format!("{size}.yaml"));

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
                        eprintln!("ERROR: {e}");
                    }
                }
            }
        }
        eprintln!();
    }

    let was_interrupted = interrupted.load(Ordering::SeqCst);

    // Generate markdown output (even if interrupted)
    if let Some(md_path) = output_md {
        let header = yq_header(
            &config.yq_binary,
            yq_version.as_deref(),
            config.benchmark_runs,
        );
        let markdown = generate_markdown(&results, has_yq, config.memory_mode, &header);
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

    // Each repetition, warmups included, runs both tools and alternates which goes first, so
    // neither always runs second (benchmarking.md rule 1, #6417). Without a yq only succinctly
    // is timed.
    let (yq_results, succ_results) = if has_yq {
        interleaved_reps(
            config.warmup_runs,
            config.benchmark_runs,
            || run_yq(file_path, &config.yq_binary, query),
            || run_succinctly(file_path, &config.succinctly_binary, query),
        )?
    } else {
        let succ = interleaved_reps(
            config.warmup_runs,
            config.benchmark_runs,
            || Ok(None),
            || run_succinctly(file_path, &config.succinctly_binary, query).map(Some),
        )?
        .1
        .into_iter()
        .flatten()
        .collect();
        (Vec::new(), succ)
    };
    let yq_result = if has_yq {
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

/// Run the configured yq and measure (outputs JSON for comparison)
fn run_yq(file_path: &Path, yq_binary: &Path, query: &str) -> Result<ToolResult> {
    let yq = yq_binary
        .to_str()
        .with_context(|| format!("--yq path is not UTF-8: {}", yq_binary.display()))?;
    // Use -o json -I 0 for compact JSON output (matches succinctly's default)
    run_command_with_timing(
        yq,
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
            .with_context(|| format!("Failed to run {program}"))?;
        ensure_success(
            program,
            output.status,
            &String::from_utf8_lossy(&output.stderr),
        )?;
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

    ensure_success(program, output.status, &stderr)?;

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

    ensure_success(program, output.status, &stderr)?;

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

/// The `--version` line of `yq_binary`, or `None` when it does not run
fn check_yq_available(yq_binary: &Path) -> Option<String> {
    let output = Command::new(yq_binary).arg("--version").output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The markdown header lines naming the yq the tables were measured against (or its absence)
/// and the method.
fn yq_header(yq_binary: &Path, yq_version: Option<&str>, runs: usize) -> String {
    match yq_version {
        Some(version) => format!(
            "**yq version**: {version} (`{}`)\n**Method**: the two tools interleaved within each \
             repetition, order alternating; median wall time of {runs} run(s).\n",
            yq_binary.display()
        ),
        None => format!(
            "**yq**: Not installed (succinctly-only benchmarks)\n**Method**: median wall time \
             of {runs} run(s).\n"
        ),
    }
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
pub fn generate_markdown(
    results: &[BenchmarkResult],
    has_yq: bool,
    memory_mode: bool,
    header: &str,
) -> String {
    let mut md = String::new();

    md.push_str("# yq vs succinctly Benchmark Results\n\n");
    md.push_str("Generated by `succinctly dev bench yq` (which `bench run yq_bench` runs)\n\n");

    // Add CPU info
    let cpu_info = get_cpu_info();
    md.push_str(&format!("**CPU**: {cpu_info}\n"));

    md.push_str(header);
    md.push('\n');

    // Collect unique queries from results
    let queries: Vec<_> = results
        .iter()
        .map(|r| (r.query.as_str(), r.query_path.as_str()))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    // Group by pattern (alphabetical order)
    let pattern_names = pattern_names();
    let patterns: Vec<&str> = pattern_names.iter().map(String::as_str).collect();

    // Size order: largest to smallest
    let size_order = ["100mb", "10mb", "1mb", "100kb", "10kb", "1kb"];

    // If multiple queries, group by query first
    if queries.len() > 1 {
        for (query, query_path) in &queries {
            md.push_str(&format!("## Query: `{query}` ({query_path})\n\n"));

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
            md.push_str(&format!("**Query**: `{query}` ({query_path})\n\n"));
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

        md.push_str(&format!("### Pattern: {pattern}\n\n"));

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
            let size_col = format!("{size_bold:<9}");

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
                    "| {size_col} | {succ_mem} | {yq_mem} | **{mem_ratio:.2}x** | {succ_time} | {yq_time} | **{speedup:.1}x** |\n"
                ));
            } else {
                let throughput =
                    r.filesize as f64 / (r.succinctly.wall_time_ms / 1000.0) / (1024.0 * 1024.0);
                md.push_str(&format!(
                    "| {size_col} | {succ_mem} | {succ_time} | {throughput:>12.1} MiB/s |\n"
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

        md.push_str(&format!("### Pattern: {pattern}\n\n"));
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
            let size_col = format!("{size_bold:<9}");

            // yq time column
            let yq_col = yq_time;

            // Succinctly time column: bold if faster (12 chars total)
            let succ_col = if speedup >= 1.0 {
                let trimmed = succ_time.trim_start();
                let padding = succ_time.len() - trimmed.len();
                format!("{}**{}**", " ".repeat(padding), trimmed)
            } else {
                format!("  {succ_time}  ")
            };

            // Speedup column: bold if >= 1.0 (13 chars visual width)
            let speedup_str = format!("{speedup:.1}x");
            let speedup_col = if speedup >= 1.0 {
                let padded = format!("{speedup_str:>9}");
                let trimmed = padded.trim_start();
                let padding = padded.len() - trimmed.len();
                format!("{}**{}**", " ".repeat(padding), trimmed)
            } else {
                format!("{speedup_str:>13}")
            };

            // Memory columns
            let yq_mem_col = yq_mem;
            let succ_mem_col = format!("{succ_mem:>8}");
            let mem_ratio_col = format!("{mem_ratio:>9.2}x");

            md.push_str(&format!(
                "| {size_col} | {yq_col} | {succ_col} | {speedup_col} | {yq_mem_col} | {succ_mem_col} | {mem_ratio_col} |\n"
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

        md.push_str(&format!("### Pattern: {pattern}\n\n"));
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
            let throughput_str = format!("{throughput:.1} MiB/s");
            let mem = format_memory_fixed(r.succinctly.peak_memory_bytes);

            // Size column
            let size_bold = format!("**{}**", r.size);
            let size_col = format!("{size_bold:<9}");

            md.push_str(&format!(
                "| {size_col} | {time} | {throughput_str:>12} | {mem} |\n"
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
        let ms_str = format!("{ms:.1}ms");
        format!("{ms_str:>8}")
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
        format!("{bytes:>4} B ")
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
        format!("{bytes} bytes")
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
    #[test]
    fn the_markdown_names_the_yq_and_the_method() {
        let header = yq_header(Path::new("/opt/homebrew/bin/yq"), Some("yq v4.53.3"), 5);
        assert_eq!(
            header,
            "**yq version**: yq v4.53.3 (`/opt/homebrew/bin/yq`)\n**Method**: the two tools \
             interleaved within each repetition, order alternating; median wall time of 5 run(s).\n"
        );
        let md = generate_markdown(&[], true, false, &header);
        assert!(md.contains(&header), "{md}");
        let absent = yq_header(Path::new("yq"), None, 3);
        assert!(absent.starts_with("**yq**: Not installed (succinctly-only benchmarks)\n"));
        assert!(absent.contains("median wall time of 3 run(s)"), "{absent}");
    }

    #[test]
    fn a_yq_that_does_not_run_is_not_available() {
        assert_eq!(
            check_yq_available(Path::new("/nonexistent/yq-for-test")),
            None
        );
        #[cfg(unix)]
        assert_eq!(check_yq_available(Path::new("false")), None);
    }

    #[test]
    fn a_yq_asked_for_by_name_that_will_not_run_is_an_error() {
        let config = BenchConfig {
            yq_binary: PathBuf::from("/nonexistent/yq-for-test"),
            yq_required: true,
            ..BenchConfig::default()
        };
        let err = run_benchmark(&config, None, None).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("yq not found at `/nonexistent/yq-for-test`") && msg.contains("--yq"),
            "{msg}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_tool_that_exits_non_zero_fails_the_row() {
        let err = run_command_with_timing("sh", &["-c", "echo boom >&2; exit 3"]).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("`sh` exited with") && msg.contains("boom"),
            "{msg}"
        );
        assert!(run_command_with_timing("true", &[]).is_ok());
    }

    /// Two stub tools that log their name when run: `benchmark_file` runs both in every
    /// repetition, warmups included, and the order alternates (#6417).
    #[cfg(unix)]
    fn stub_tool(dir: &Path, name: &str, log: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!("#!/bin/sh\necho {name} >> {}\necho '{{}}'\n", log.display()),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn benchmark_file_interleaves_the_tools_and_alternates_the_order() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("log");
        let data = dir.path().join("data.yaml");
        std::fs::write(&data, "a: 1\n").unwrap();
        let config = BenchConfig {
            succinctly_binary: stub_tool(dir.path(), "succ", &log),
            yq_binary: stub_tool(dir.path(), "yq", &log),
            warmup_runs: 1,
            benchmark_runs: 2,
            ..BenchConfig::default()
        };
        let result = benchmark_file(&data, &config, true, QueryType::Identity).unwrap();
        let ran: Vec<String> = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .map(String::from)
            .collect();
        // The warmup is rep 0 and the order carries on across it: yq, succ, then succ, yq, ...
        assert_eq!(ran, ["yq", "succ", "succ", "yq", "yq", "succ"]);
        assert_eq!(result.status, "match");
    }

    #[cfg(unix)]
    #[test]
    fn without_a_yq_only_succinctly_is_timed() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("log");
        let data = dir.path().join("data.yaml");
        std::fs::write(&data, "a: 1\n").unwrap();
        let config = BenchConfig {
            succinctly_binary: stub_tool(dir.path(), "succ", &log),
            warmup_runs: 1,
            benchmark_runs: 2,
            ..BenchConfig::default()
        };
        let result = benchmark_file(&data, &config, false, QueryType::Identity).unwrap();
        let ran = std::fs::read_to_string(&log).unwrap();
        assert_eq!(ran.lines().collect::<Vec<_>>(), ["succ", "succ", "succ"]);
        assert_eq!(result.status, "succinctly-only");
    }
}
