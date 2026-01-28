//! Benchmark runner implementation.
//!
//! Handles list and run commands.

use super::metadata::BenchmarkMetadata;
use super::registry::{
    filter_by_category, filter_by_names, BenchmarkCategory, BenchmarkInfo, BenchmarkType,
    BENCHMARKS,
};
use super::utils::format_duration;
use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

/// Arguments for list command.
#[derive(Debug, Parser)]
pub struct ListArgs {
    /// Filter by category (core, json, yaml, dsv, cross-parser)
    #[arg(short, long)]
    pub category: Option<String>,

    /// Output format (table, json)
    #[arg(short, long, default_value = "table")]
    pub format: String,
}

/// Arguments for run command.
#[derive(Debug, Parser)]
pub struct RunArgs {
    /// Benchmark names to run
    #[arg(value_name = "NAMES")]
    pub names: Vec<String>,

    /// Run all benchmarks
    #[arg(long)]
    pub all: bool,

    /// Filter by category (core, json, yaml, dsv, cross-parser)
    /// Can be comma-separated: --category core,json
    #[arg(short, long)]
    pub category: Option<String>,

    /// Output directory for results
    #[arg(short, long, default_value = "data/bench/results")]
    pub output_dir: PathBuf,

    /// Skip metadata collection
    #[arg(long)]
    pub no_metadata: bool,

    /// Continue running even if a benchmark fails
    #[arg(long)]
    pub continue_on_error: bool,

    /// Dry run - show what would be executed without running
    #[arg(long)]
    pub dry_run: bool,

    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

/// Result from a single benchmark execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub name: String,
    pub category: String,
    pub bench_type: String,
    pub success: bool,
    pub duration_seconds: f64,
    pub exit_code: Option<i32>,
    pub error_message: Option<String>,
}

/// Summary of a benchmark run session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub metadata: Option<BenchmarkMetadata>,
    pub results: Vec<BenchmarkResult>,
    pub total_duration_seconds: f64,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
}

/// Run the list command.
pub fn run_list(args: ListArgs) -> Result<()> {
    let benchmarks: Vec<&BenchmarkInfo> = if let Some(cat_str) = &args.category {
        let category: BenchmarkCategory =
            cat_str.parse().map_err(|e: String| anyhow::anyhow!(e))?;
        filter_by_category(category)
    } else {
        BENCHMARKS.iter().collect()
    };

    match args.format.as_str() {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&benchmarks)?);
        }
        _ => {
            println!(
                "{:<25} {:<12} {:<12} Description",
                "Name", "Category", "Type"
            );
            println!("{:-<25} {:-<12} {:-<12} {:-<40}", "", "", "", "");
            for b in benchmarks {
                println!(
                    "{:<25} {:<12} {:<12} {}",
                    b.name,
                    b.category.to_string(),
                    b.bench_type.to_string(),
                    b.description
                );
            }
            println!();
            println!("Total: {} benchmarks", BENCHMARKS.len());
        }
    }

    Ok(())
}

/// Run the run command.
pub fn run_benchmarks(args: RunArgs) -> Result<()> {
    // Select benchmarks to run
    let benchmarks = select_benchmarks(&args)?;

    if benchmarks.is_empty() {
        eprintln!("No benchmarks selected. Use --all, --category, or specify benchmark names.");
        eprintln!("\nAvailable benchmarks:");
        for b in BENCHMARKS.iter() {
            eprintln!("  {} ({})", b.name, b.category);
        }
        return Ok(());
    }

    // Dry run mode
    if args.dry_run {
        println!("Dry run - would execute {} benchmarks:", benchmarks.len());
        println!();
        for b in &benchmarks {
            println!("  {} ({}, {})", b.name, b.category, b.bench_type);
            match b.bench_type {
                BenchmarkType::Criterion => {
                    println!(
                        "    Command: cargo bench --bench {}",
                        b.criterion_name.unwrap_or(b.name)
                    );
                }
                BenchmarkType::CliBench => {
                    println!(
                        "    Command: ./target/release/succinctly dev bench {}",
                        b.cli_subcommand.unwrap_or("?")
                    );
                }
                BenchmarkType::CrossParser => {
                    println!(
                        "    Command: cd bench-compare && cargo bench --bench {}",
                        b.criterion_name.unwrap_or(b.name)
                    );
                }
            }
        }
        return Ok(());
    }

    // Collect metadata
    let metadata = if args.no_metadata {
        None
    } else {
        match BenchmarkMetadata::collect() {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("Warning: Failed to collect metadata: {}", e);
                None
            }
        }
    };

    // Create output directory
    let output_dir = create_output_dir(&args.output_dir, &metadata)?;
    eprintln!("Output directory: {}", output_dir.display());

    // Create stdout subdirectory
    let stdout_dir = output_dir.join("stdout");
    fs::create_dir_all(&stdout_dir)?;

    // Write metadata
    if let Some(ref meta) = metadata {
        let meta_path = output_dir.join("metadata.json");
        let meta_json = serde_json::to_string_pretty(meta)?;
        fs::write(&meta_path, meta_json)?;
        if args.verbose {
            eprintln!("Wrote metadata to {}", meta_path.display());
        }
    }

    // Run benchmarks
    let mut results = Vec::new();
    let start = Instant::now();

    eprintln!();
    eprintln!("Running {} benchmarks...", benchmarks.len());
    eprintln!();

    for (i, benchmark) in benchmarks.iter().enumerate() {
        eprint!(
            "[{}/{}] {} ({})... ",
            i + 1,
            benchmarks.len(),
            benchmark.name,
            benchmark.category
        );
        std::io::stderr().flush().ok();

        let result = run_single_benchmark(benchmark, &stdout_dir, args.verbose);

        match &result {
            Ok(r) if r.success => {
                eprintln!("OK ({:.1}s)", r.duration_seconds);
            }
            Ok(r) => {
                eprintln!("FAILED (exit code: {:?})", r.exit_code);
                if let Some(msg) = &r.error_message {
                    if args.verbose {
                        eprintln!("    Error: {}", msg);
                    }
                }
            }
            Err(e) => {
                eprintln!("ERROR: {}", e);
            }
        }

        let result = result.unwrap_or_else(|e| BenchmarkResult {
            name: benchmark.name.to_string(),
            category: benchmark.category.to_string(),
            bench_type: benchmark.bench_type.to_string(),
            success: false,
            duration_seconds: 0.0,
            exit_code: None,
            error_message: Some(e.to_string()),
        });

        if !result.success && !args.continue_on_error {
            results.push(result);
            eprintln!("\nStopping due to failure. Use --continue-on-error to run all benchmarks.");
            break;
        }

        results.push(result);
    }

    let total_duration = start.elapsed().as_secs_f64();

    // Calculate summary
    let passed = results.iter().filter(|r| r.success).count();
    let failed = results.iter().filter(|r| !r.success).count();
    let skipped = benchmarks.len() - results.len();

    let summary = RunSummary {
        metadata,
        results,
        total_duration_seconds: total_duration,
        passed,
        failed,
        skipped,
    };

    // Write summary
    let summary_path = output_dir.join("summary.json");
    let summary_json = serde_json::to_string_pretty(&summary)?;
    fs::write(&summary_path, summary_json)?;

    // Print summary
    eprintln!();
    eprintln!("Benchmark run complete:");
    eprintln!("  Passed:   {}", passed);
    eprintln!("  Failed:   {}", failed);
    if skipped > 0 {
        eprintln!("  Skipped:  {}", skipped);
    }
    eprintln!("  Duration: {}", format_duration(total_duration));
    eprintln!("  Results:  {}", output_dir.display());

    if failed > 0 {
        std::process::exit(1);
    }

    Ok(())
}

/// Select benchmarks based on arguments.
fn select_benchmarks(args: &RunArgs) -> Result<Vec<&'static BenchmarkInfo>> {
    if args.all {
        return Ok(BENCHMARKS.iter().collect());
    }

    if let Some(cat_str) = &args.category {
        let mut result = Vec::new();
        for cat in cat_str.split(',') {
            let category: BenchmarkCategory =
                cat.trim().parse().map_err(|e: String| anyhow::anyhow!(e))?;
            result.extend(filter_by_category(category));
        }
        return Ok(result);
    }

    if !args.names.is_empty() {
        let benchmarks = filter_by_names(&args.names);
        // Check for unknown names
        for name in &args.names {
            if !benchmarks.iter().any(|b| b.name == name) {
                eprintln!("Warning: Unknown benchmark '{}'", name);
            }
        }
        return Ok(benchmarks);
    }

    Ok(Vec::new())
}

/// Create the output directory.
fn create_output_dir(base: &Path, metadata: &Option<BenchmarkMetadata>) -> Result<PathBuf> {
    let dir_name = if let Some(meta) = metadata {
        meta.output_dir_name()
    } else {
        chrono::Local::now().format("%Y%m%d_%H%M%S").to_string()
    };

    let dir = base.join(&dir_name);
    fs::create_dir_all(&dir).with_context(|| format!("Failed to create {}", dir.display()))?;

    Ok(dir)
}

/// Run a single benchmark.
fn run_single_benchmark(
    benchmark: &BenchmarkInfo,
    stdout_dir: &Path,
    verbose: bool,
) -> Result<BenchmarkResult> {
    let start = Instant::now();

    let (exit_code, error_msg) = match benchmark.bench_type {
        BenchmarkType::Criterion => run_criterion_benchmark(benchmark, stdout_dir, verbose)?,
        BenchmarkType::CliBench => run_cli_benchmark(benchmark, stdout_dir, verbose)?,
        BenchmarkType::CrossParser => run_cross_parser_benchmark(benchmark, stdout_dir, verbose)?,
    };

    let duration = start.elapsed().as_secs_f64();
    let success = exit_code == Some(0);

    Ok(BenchmarkResult {
        name: benchmark.name.to_string(),
        category: benchmark.category.to_string(),
        bench_type: benchmark.bench_type.to_string(),
        success,
        duration_seconds: duration,
        exit_code,
        error_message: if success { None } else { error_msg },
    })
}

/// Run a criterion benchmark.
fn run_criterion_benchmark(
    benchmark: &BenchmarkInfo,
    stdout_dir: &Path,
    _verbose: bool,
) -> Result<(Option<i32>, Option<String>)> {
    let bench_name = benchmark.criterion_name.unwrap_or(benchmark.name);

    let output = Command::new("cargo")
        .args(["bench", "--bench", bench_name])
        .current_dir(benchmark.working_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("Failed to run cargo bench --bench {}", bench_name))?;

    // Save stdout
    let stdout_path = stdout_dir.join(format!("{}.txt", benchmark.name));
    let mut file = fs::File::create(&stdout_path)?;
    file.write_all(&output.stdout)?;
    file.write_all(b"\n--- stderr ---\n")?;
    file.write_all(&output.stderr)?;

    let exit_code = output.status.code();
    let error_msg = if !output.status.success() {
        Some(String::from_utf8_lossy(&output.stderr).to_string())
    } else {
        None
    };

    Ok((exit_code, error_msg))
}

/// Run a CLI benchmark.
fn run_cli_benchmark(
    benchmark: &BenchmarkInfo,
    stdout_dir: &Path,
    _verbose: bool,
) -> Result<(Option<i32>, Option<String>)> {
    let subcommand = benchmark.cli_subcommand.unwrap_or("?");

    // Use the release binary
    let binary = PathBuf::from("./target/release/succinctly");
    if !binary.exists() {
        anyhow::bail!(
            "succinctly binary not found at {}. Run: cargo build --release --features cli",
            binary.display()
        );
    }

    let output = Command::new(&binary)
        .args(["dev", "bench", subcommand])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("Failed to run succinctly dev bench {}", subcommand))?;

    // Save stdout
    let stdout_path = stdout_dir.join(format!("{}.txt", benchmark.name));
    let mut file = fs::File::create(&stdout_path)?;
    file.write_all(&output.stdout)?;
    file.write_all(b"\n--- stderr ---\n")?;
    file.write_all(&output.stderr)?;

    let exit_code = output.status.code();
    let error_msg = if !output.status.success() {
        Some(String::from_utf8_lossy(&output.stderr).to_string())
    } else {
        None
    };

    Ok((exit_code, error_msg))
}

/// Run a cross-parser benchmark.
fn run_cross_parser_benchmark(
    benchmark: &BenchmarkInfo,
    stdout_dir: &Path,
    _verbose: bool,
) -> Result<(Option<i32>, Option<String>)> {
    let bench_name = benchmark.criterion_name.unwrap_or(benchmark.name);

    // Check that bench-compare directory exists
    let bench_compare_dir = PathBuf::from("bench-compare");
    if !bench_compare_dir.exists() {
        anyhow::bail!(
            "bench-compare directory not found. Cross-parser benchmarks require the bench-compare subproject."
        );
    }

    let output = Command::new("cargo")
        .args(["bench", "--bench", bench_name])
        .current_dir(&bench_compare_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| {
            format!(
                "Failed to run cargo bench --bench {} in bench-compare",
                bench_name
            )
        })?;

    // Save stdout
    let stdout_path = stdout_dir.join(format!("{}.txt", benchmark.name));
    let mut file = fs::File::create(&stdout_path)?;
    file.write_all(&output.stdout)?;
    file.write_all(b"\n--- stderr ---\n")?;
    file.write_all(&output.stderr)?;

    let exit_code = output.status.code();
    let error_msg = if !output.status.success() {
        Some(String::from_utf8_lossy(&output.stderr).to_string())
    } else {
        None
    };

    Ok((exit_code, error_msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_all_benchmarks() {
        let args = RunArgs {
            names: vec![],
            all: true,
            category: None,
            output_dir: PathBuf::from("data/bench/results"),
            no_metadata: false,
            continue_on_error: false,
            dry_run: false,
            verbose: false,
        };
        let benchmarks = select_benchmarks(&args).unwrap();
        assert_eq!(benchmarks.len(), BENCHMARKS.len());
    }

    #[test]
    fn test_select_by_category() {
        let args = RunArgs {
            names: vec![],
            all: false,
            category: Some("core".to_string()),
            output_dir: PathBuf::from("data/bench/results"),
            no_metadata: false,
            continue_on_error: false,
            dry_run: false,
            verbose: false,
        };
        let benchmarks = select_benchmarks(&args).unwrap();
        assert!(benchmarks
            .iter()
            .all(|b| b.category == BenchmarkCategory::Core));
    }

    #[test]
    fn test_select_by_names() {
        let args = RunArgs {
            names: vec!["rank_select".to_string(), "yaml_bench".to_string()],
            all: false,
            category: None,
            output_dir: PathBuf::from("data/bench/results"),
            no_metadata: false,
            continue_on_error: false,
            dry_run: false,
            verbose: false,
        };
        let benchmarks = select_benchmarks(&args).unwrap();
        assert_eq!(benchmarks.len(), 2);
    }
}
