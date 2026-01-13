//! Succinctly CLI tool for working with succinct data structures.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "succinctly")]
#[command(about = "Succinct data structures toolkit", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// JSON operations (generate, parse, benchmark)
    Json(JsonCommand),
    /// DSV (CSV/TSV) operations (generate, parse)
    Dsv(DsvCommand),
    /// YAML operations (generate, parse)
    Yaml(YamlCommand),
    /// Command-line JSON processor (jq-compatible)
    Jq(JqCommand),
    /// Find jq expression for a position in a JSON file
    JqLocate(jq_locate::JqLocateArgs),
    /// Developer tools (benchmarking, profiling)
    Dev(DevCommand),
}

#[derive(Debug, Parser)]
struct JsonCommand {
    #[command(subcommand)]
    command: JsonSubcommand,
}

#[derive(Debug, Subcommand)]
enum JsonSubcommand {
    /// Generate synthetic JSON files for benchmarking and testing
    Generate(GenerateJson),
    /// Generate a suite of JSON files with various sizes and patterns
    GenerateSuite(GenerateSuite),
}

#[derive(Debug, Parser)]
struct DsvCommand {
    #[command(subcommand)]
    command: DsvSubcommand,
}

#[derive(Debug, Subcommand)]
enum DsvSubcommand {
    /// Generate synthetic DSV (CSV/TSV) files for benchmarking and testing
    Generate(GenerateDsv),
    /// Generate a suite of DSV files with various sizes and patterns
    GenerateSuite(GenerateDsvSuite),
}

#[derive(Debug, Parser)]
struct YamlCommand {
    #[command(subcommand)]
    command: YamlSubcommand,
}

#[derive(Debug, Subcommand)]
enum YamlSubcommand {
    /// Generate synthetic YAML files for benchmarking and testing
    Generate(GenerateYaml),
    /// Generate a suite of YAML files with various sizes and patterns
    GenerateSuite(GenerateYamlSuite),
}

#[derive(Debug, Parser)]
struct DevCommand {
    #[command(subcommand)]
    command: DevSubcommand,
}

#[derive(Debug, Subcommand)]
enum DevSubcommand {
    /// Run benchmarks
    Bench(BenchCommand),
}

#[derive(Debug, Parser)]
struct BenchCommand {
    #[command(subcommand)]
    command: BenchSubcommand,
}

#[derive(Debug, Subcommand)]
enum BenchSubcommand {
    /// Benchmark succinctly jq vs system jq
    Jq(BenchJqArgs),
    /// Benchmark succinctly jq with DSV input
    Dsv(BenchDsvArgs),
}

/// Arguments for jq benchmark
#[derive(Debug, Parser)]
struct BenchJqArgs {
    /// Directory containing generated JSON files
    #[arg(short, long, default_value = "data/bench/generated")]
    data_dir: PathBuf,

    /// Output JSONL file for raw results
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Output markdown file for formatted tables
    #[arg(short, long)]
    markdown: Option<PathBuf>,

    /// Patterns to benchmark (comma-separated, or "all")
    #[arg(short, long, default_value = "all")]
    patterns: String,

    /// Sizes to benchmark (comma-separated, or "all")
    #[arg(short, long, default_value = "all")]
    sizes: String,

    /// Number of warmup runs before benchmarking
    #[arg(long, default_value = "1")]
    warmup: usize,

    /// Number of benchmark runs (median is taken)
    #[arg(long, default_value = "3")]
    runs: usize,

    /// Path to succinctly binary
    #[arg(long, default_value = "./target/release/succinctly")]
    binary: PathBuf,
}

/// Arguments for DSV benchmark
#[derive(Debug, Parser)]
struct BenchDsvArgs {
    /// Directory containing generated DSV files
    #[arg(short, long, default_value = "data/bench/generated/dsv")]
    data_dir: PathBuf,

    /// Output JSONL file for raw results
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Output markdown file for formatted tables
    #[arg(short, long)]
    markdown: Option<PathBuf>,

    /// Patterns to benchmark (comma-separated, or "all")
    #[arg(short, long, default_value = "all")]
    patterns: String,

    /// Sizes to benchmark (comma-separated, or "all")
    #[arg(short, long, default_value = "all")]
    sizes: String,

    /// Number of warmup runs before benchmarking
    #[arg(long, default_value = "1")]
    warmup: usize,

    /// Number of benchmark runs (median is taken)
    #[arg(long, default_value = "3")]
    runs: usize,

    /// Path to succinctly binary
    #[arg(long, default_value = "./target/release/succinctly")]
    binary: PathBuf,

    /// Delimiter character for DSV files (default: comma)
    #[arg(long, default_value = ",")]
    delimiter: char,

    /// Query to run (default: "." for full output, or ".[0]" for first column)
    #[arg(long, default_value = ".")]
    query: String,
}

/// Generate synthetic JSON files for benchmarking and testing
#[derive(Debug, Parser)]
struct GenerateJson {
    /// Size of JSON to generate (supports b, kb, mb, gb - case insensitive)
    /// Examples: 1024, 1kb, 512MB, 2Gb
    #[arg(value_parser = parse_size)]
    size: usize,

    /// Output file path (defaults to stdout)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// JSON pattern to generate
    #[arg(short, long, default_value = "comprehensive")]
    pattern: PatternArg,

    /// Random seed for reproducible generation
    #[arg(short, long)]
    seed: Option<u64>,

    /// Pretty print JSON (slower, larger output)
    #[arg(long)]
    pretty: bool,

    /// Verify generated JSON is valid
    #[arg(long)]
    verify: bool,

    /// Nesting depth for nested structures (default: 5)
    #[arg(long, default_value = "5")]
    depth: usize,

    /// Escape sequence density (0.0-1.0, default: 0.1)
    #[arg(long, default_value = "0.1")]
    escape_density: f64,
}

#[derive(Debug, Clone, ValueEnum)]
enum PatternArg {
    /// Comprehensive pattern testing all JSON features (default, best for benchmarking)
    Comprehensive,
    /// Array of user objects (realistic structure)
    Users,
    /// Deeply nested objects (tests nesting and BP operations)
    Nested,
    /// Array of arrays (tests array handling)
    Arrays,
    /// Mix of all types (balanced distribution)
    Mixed,
    /// String-heavy with escapes (tests string parsing and escape handling)
    Strings,
    /// Number-heavy documents (tests number parsing)
    Numbers,
    /// Boolean and null heavy (tests literal parsing)
    Literals,
    /// Unicode-heavy strings (tests UTF-8 handling)
    Unicode,
    /// Worst-case for parsing (maximum structural density)
    Pathological,
}

/// Generate a suite of JSON files with various sizes and patterns for benchmarking
#[derive(Debug, Parser)]
struct GenerateSuite {
    /// Output directory (defaults to data/bench/generated)
    #[arg(short, long, default_value = "data/bench/generated")]
    output_dir: PathBuf,

    /// Base seed for deterministic generation (each file uses seed + file_index)
    #[arg(short, long, default_value = "42")]
    seed: u64,

    /// Clean output directory before generating
    #[arg(long)]
    clean: bool,

    /// Verify all generated JSON files are valid
    #[arg(long)]
    verify: bool,

    /// Maximum file size to generate (files larger than this are skipped)
    /// Supports units: b, kb, mb, gb (e.g., "100mb", "1gb")
    #[arg(short, long, value_parser = parse_size, default_value = "1gb")]
    max_size: usize,
}

/// Generate synthetic DSV (CSV/TSV) files for benchmarking and testing
#[derive(Debug, Parser)]
struct GenerateDsv {
    /// Size of DSV to generate (supports b, kb, mb, gb - case insensitive)
    /// Examples: 1024, 1kb, 512MB, 2Gb
    #[arg(value_parser = parse_size)]
    size: usize,

    /// Output file path (defaults to stdout)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// DSV pattern to generate
    #[arg(short, long, default_value = "tabular")]
    pattern: DsvPatternArg,

    /// Random seed for reproducible generation
    #[arg(short, long)]
    seed: Option<u64>,

    /// Field delimiter character
    #[arg(short, long, default_value = ",")]
    delimiter: char,

    /// Include header row
    #[arg(long, default_value = "true")]
    header: bool,

    /// Verify generated DSV can be parsed
    #[arg(long)]
    verify: bool,
}

#[derive(Debug, Clone, ValueEnum)]
enum DsvPatternArg {
    /// Standard tabular data with mixed types (default)
    Tabular,
    /// User/person records (realistic structure)
    Users,
    /// Numeric-heavy data (financial, scientific)
    Numeric,
    /// String-heavy data with various lengths
    Strings,
    /// Data with quoted fields containing delimiters
    Quoted,
    /// Data with quoted fields containing newlines
    Multiline,
    /// Wide tables (many columns)
    Wide,
    /// Narrow but long tables (few columns, many rows)
    Long,
    /// Mixed data types per row
    Mixed,
    /// Worst case: every field is quoted with embedded delimiters
    Pathological,
}

impl From<DsvPatternArg> for dsv_generators::DsvPattern {
    fn from(arg: DsvPatternArg) -> Self {
        match arg {
            DsvPatternArg::Tabular => dsv_generators::DsvPattern::Tabular,
            DsvPatternArg::Users => dsv_generators::DsvPattern::Users,
            DsvPatternArg::Numeric => dsv_generators::DsvPattern::Numeric,
            DsvPatternArg::Strings => dsv_generators::DsvPattern::Strings,
            DsvPatternArg::Quoted => dsv_generators::DsvPattern::Quoted,
            DsvPatternArg::Multiline => dsv_generators::DsvPattern::Multiline,
            DsvPatternArg::Wide => dsv_generators::DsvPattern::Wide,
            DsvPatternArg::Long => dsv_generators::DsvPattern::Long,
            DsvPatternArg::Mixed => dsv_generators::DsvPattern::Mixed,
            DsvPatternArg::Pathological => dsv_generators::DsvPattern::Pathological,
        }
    }
}

/// Generate a suite of DSV files with various sizes and patterns for benchmarking
#[derive(Debug, Parser)]
struct GenerateDsvSuite {
    /// Output directory (defaults to data/bench/generated/dsv)
    #[arg(short, long, default_value = "data/bench/generated/dsv")]
    output_dir: PathBuf,

    /// Base seed for deterministic generation (each file uses seed + file_index)
    #[arg(short, long, default_value = "42")]
    seed: u64,

    /// Field delimiter character
    #[arg(short, long, default_value = ",")]
    delimiter: char,

    /// Clean output directory before generating
    #[arg(long)]
    clean: bool,

    /// Verify all generated DSV files can be parsed
    #[arg(long)]
    verify: bool,

    /// Maximum file size to generate (files larger than this are skipped)
    /// Supports units: b, kb, mb, gb (e.g., "100mb", "1gb")
    #[arg(short, long, value_parser = parse_size, default_value = "1gb")]
    max_size: usize,
}

/// Generate synthetic YAML files for benchmarking and testing
#[derive(Debug, Parser)]
struct GenerateYaml {
    /// Size of YAML to generate (supports b, kb, mb, gb - case insensitive)
    /// Examples: 1024, 1kb, 512MB, 2Gb
    #[arg(value_parser = parse_size)]
    size: usize,

    /// Output file path (defaults to stdout)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// YAML pattern to generate
    #[arg(short, long, default_value = "comprehensive")]
    pattern: YamlPatternArg,

    /// Random seed for reproducible generation
    #[arg(short, long)]
    seed: Option<u64>,

    /// Verify generated YAML can be parsed
    #[arg(long)]
    verify: bool,
}

#[derive(Debug, Clone, ValueEnum)]
enum YamlPatternArg {
    /// Comprehensive pattern testing various YAML features (default)
    Comprehensive,
    /// Array of user/person records (realistic structure)
    Users,
    /// Deeply nested mappings and sequences
    Nested,
    /// Sequences at various levels
    Sequences,
    /// Mixed mappings and sequences
    Mixed,
    /// String-heavy with quoted strings
    Strings,
    /// Numeric values (integers and floats)
    Numbers,
    /// Configuration file style (realistic config)
    Config,
    /// Unicode strings in various scripts
    Unicode,
    /// Worst case for parsing (maximum depth and density)
    Pathological,
}

impl From<YamlPatternArg> for yaml_generators::YamlPattern {
    fn from(arg: YamlPatternArg) -> Self {
        match arg {
            YamlPatternArg::Comprehensive => yaml_generators::YamlPattern::Comprehensive,
            YamlPatternArg::Users => yaml_generators::YamlPattern::Users,
            YamlPatternArg::Nested => yaml_generators::YamlPattern::Nested,
            YamlPatternArg::Sequences => yaml_generators::YamlPattern::Sequences,
            YamlPatternArg::Mixed => yaml_generators::YamlPattern::Mixed,
            YamlPatternArg::Strings => yaml_generators::YamlPattern::Strings,
            YamlPatternArg::Numbers => yaml_generators::YamlPattern::Numbers,
            YamlPatternArg::Config => yaml_generators::YamlPattern::Config,
            YamlPatternArg::Unicode => yaml_generators::YamlPattern::Unicode,
            YamlPatternArg::Pathological => yaml_generators::YamlPattern::Pathological,
        }
    }
}

/// Generate a suite of YAML files with various sizes and patterns for benchmarking
#[derive(Debug, Parser)]
struct GenerateYamlSuite {
    /// Output directory (defaults to data/bench/generated/yaml)
    #[arg(short, long, default_value = "data/bench/generated/yaml")]
    output_dir: PathBuf,

    /// Base seed for deterministic generation (each file uses seed + file_index)
    #[arg(short, long, default_value = "42")]
    seed: u64,

    /// Clean output directory before generating
    #[arg(long)]
    clean: bool,

    /// Verify all generated YAML files can be parsed
    #[arg(long)]
    verify: bool,

    /// Maximum file size to generate (files larger than this are skipped)
    /// Supports units: b, kb, mb, gb (e.g., "100mb", "1gb")
    #[arg(short, long, value_parser = parse_size, default_value = "1gb")]
    max_size: usize,
}

/// Command-line JSON processor (jq-compatible CLI)
#[derive(Debug, Parser)]
#[command(name = "jq")]
#[command(about = "Command-line JSON processor", long_about = None)]
struct JqCommand {
    /// jq filter expression (e.g., ".", ".foo", ".[]")
    /// If not provided, uses "." (identity)
    filter: Option<String>,

    /// Input files (reads from stdin if none provided)
    /// When using --args or --jsonargs, these become positional values instead.
    #[arg(trailing_var_arg = true)]
    files: Vec<String>,

    // === Input Options ===
    /// Don't read any input; use null as the single input value
    #[arg(short = 'n', long)]
    null_input: bool,

    /// Read each line as a string instead of JSON
    #[arg(short = 'R', long)]
    raw_input: bool,

    /// [Extension] Read input as DSV (delimiter-separated values).
    /// Each row becomes a JSON array of strings.
    /// Properly handles quoted fields with embedded delimiters and newlines.
    /// Use comma for CSV, tab for TSV, or any single ASCII character.
    /// Special CSV characters (quote " and newline) cannot be used as delimiters.
    #[arg(long, value_name = "DELIMITER")]
    input_dsv: Option<char>,

    /// Read all inputs into an array and use it as the single input value
    #[arg(short = 's', long)]
    slurp: bool,

    // === Output Options ===
    /// Compact output (no pretty printing)
    #[arg(short = 'c', long)]
    compact_output: bool,

    /// Output raw strings without quotes
    #[arg(short = 'r', long)]
    raw_output: bool,

    /// Like -r but don't print newline after each output
    #[arg(short = 'j', long)]
    join_output: bool,

    /// Like -r but print NUL instead of newline after each output
    #[arg(long)]
    raw_output0: bool,

    /// Output ASCII only, escaping non-ASCII as \uXXXX
    #[arg(short = 'a', long)]
    ascii_output: bool,

    /// Colorize output (default if stdout is a terminal)
    #[arg(short = 'C', long)]
    color_output: bool,

    /// Disable colorized output
    #[arg(short = 'M', long)]
    monochrome_output: bool,

    /// Sort keys of each object on output
    #[arg(short = 'S', long)]
    sort_keys: bool,

    /// Preserve original input formatting (numbers like 4e4, escape sequences)
    /// Can also be enabled via SUCCINCTLY_PRESERVE_INPUT=1 environment variable
    #[arg(long)]
    preserve_input: bool,

    /// Use tabs for indentation
    #[arg(long)]
    tab: bool,

    /// Use n spaces for indentation (max 7)
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=7))]
    indent: Option<u8>,

    // === Program Input ===
    /// Read filter from file instead of command line
    #[arg(short = 'f', long, value_name = "FILE")]
    from_file: Option<PathBuf>,

    // === Variables ===
    /// Set $name to the string value
    #[arg(long, value_names = ["NAME", "VALUE"], num_args = 2, action = clap::ArgAction::Append)]
    arg: Vec<String>,

    /// Set $name to the JSON value
    #[arg(long, value_names = ["NAME", "VALUE"], num_args = 2, action = clap::ArgAction::Append)]
    argjson: Vec<String>,

    /// Set $name to an array of JSON values read from file
    #[arg(long, value_names = ["NAME", "FILE"], num_args = 2, action = clap::ArgAction::Append)]
    slurpfile: Vec<String>,

    /// Set $name to the string contents of file
    #[arg(long, value_names = ["NAME", "FILE"], num_args = 2, action = clap::ArgAction::Append)]
    rawfile: Vec<String>,

    /// Consume remaining arguments as positional string values
    #[arg(long, num_args = 0.., value_name = "STRINGS")]
    args: Vec<String>,

    /// Consume remaining arguments as positional JSON values
    #[arg(long, num_args = 0.., value_name = "JSON_VALUES")]
    jsonargs: Vec<String>,

    // === Modules ===
    /// Prepend directory to module search path
    #[arg(short = 'L', value_name = "DIR", action = clap::ArgAction::Append)]
    library_path: Vec<PathBuf>,

    // === Formats ===
    /// Parse input/output as application/json-seq (RFC 7464)
    #[arg(long)]
    seq: bool,

    // === Exit Status ===
    /// Set exit status based on output (0 if last output != false/null)
    #[arg(short = 'e', long)]
    exit_status: bool,

    /// Flush output after each JSON value
    #[arg(long)]
    unbuffered: bool,

    // === Info ===
    /// Show version information
    #[arg(short = 'V', long)]
    version: bool,

    /// Show build configuration
    #[arg(long)]
    build_configuration: bool,
}

impl From<PatternArg> for generators::Pattern {
    fn from(arg: PatternArg) -> Self {
        match arg {
            PatternArg::Comprehensive => generators::Pattern::Comprehensive,
            PatternArg::Users => generators::Pattern::Users,
            PatternArg::Nested => generators::Pattern::Nested,
            PatternArg::Arrays => generators::Pattern::Arrays,
            PatternArg::Mixed => generators::Pattern::Mixed,
            PatternArg::Strings => generators::Pattern::Strings,
            PatternArg::Numbers => generators::Pattern::Numbers,
            PatternArg::Literals => generators::Pattern::Literals,
            PatternArg::Unicode => generators::Pattern::Unicode,
            PatternArg::Pathological => generators::Pattern::Pathological,
        }
    }
}

/// Parse size string like "1mb", "512KB", "2GB", "1024" (case insensitive)
fn parse_size(s: &str) -> Result<usize, String> {
    let s = s.trim().to_lowercase();

    // Try parsing as plain number first
    if let Ok(bytes) = s.parse::<usize>() {
        return Ok(bytes);
    }

    // Parse with unit suffix
    let (num_str, unit) = if s.ends_with("gb") {
        (s.trim_end_matches("gb"), 1024 * 1024 * 1024)
    } else if s.ends_with("mb") {
        (s.trim_end_matches("mb"), 1024 * 1024)
    } else if s.ends_with("kb") {
        (s.trim_end_matches("kb"), 1024)
    } else if s.ends_with('b') {
        (s.trim_end_matches('b'), 1)
    } else {
        return Err(format!(
            "Invalid size format: '{}'. Use format like '1mb', '512KB', or '1024'",
            s
        ));
    };

    num_str
        .trim()
        .parse::<usize>()
        .map(|n| n * unit)
        .map_err(|_| format!("Invalid number in size: '{}'", s))
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Jq(args) => {
            let exit_code = jq_runner::run_jq(args)?;
            std::process::exit(exit_code);
        }
        Command::JqLocate(args) => {
            let exit_code = jq_locate::run_jq_locate(args)?;
            std::process::exit(exit_code);
        }
        Command::Json(json_cmd) => match json_cmd.command {
            JsonSubcommand::Generate(args) => {
                let json = generate_json(
                    args.size,
                    args.pattern.into(),
                    args.seed,
                    args.depth,
                    args.escape_density,
                );

                if args.verify {
                    serde_json::from_str::<serde_json::Value>(&json)
                        .context("Generated invalid JSON")?;
                    eprintln!("✓ JSON validated successfully");
                }

                let output = if args.pretty {
                    let value: serde_json::Value = serde_json::from_str(&json)?;
                    serde_json::to_string_pretty(&value)?
                } else {
                    json
                };

                match args.output {
                    Some(path) => {
                        std::fs::write(&path, &output)?;
                        eprintln!("✓ Wrote {} bytes to {}", output.len(), path.display());
                    }
                    None => {
                        println!("{}", output);
                    }
                }

                Ok(())
            }
            JsonSubcommand::GenerateSuite(args) => generate_json_suite(args),
        },
        Command::Dsv(dsv_cmd) => match dsv_cmd.command {
            DsvSubcommand::Generate(args) => {
                let dsv = dsv_generators::generate_dsv(
                    args.size,
                    args.pattern.into(),
                    args.seed,
                    args.delimiter,
                    args.header,
                );

                if args.verify {
                    let config =
                        succinctly::DsvConfig::default().with_delimiter(args.delimiter as u8);
                    let parsed = succinctly::Dsv::parse_with_config(dsv.as_bytes(), &config);
                    eprintln!("✓ DSV validated successfully ({} rows)", parsed.row_count());
                }

                match args.output {
                    Some(path) => {
                        std::fs::write(&path, &dsv)?;
                        eprintln!("✓ Wrote {} bytes to {}", dsv.len(), path.display());
                    }
                    None => {
                        print!("{}", dsv);
                    }
                }

                Ok(())
            }
            DsvSubcommand::GenerateSuite(args) => generate_dsv_suite(args),
        },
        Command::Yaml(yaml_cmd) => match yaml_cmd.command {
            YamlSubcommand::Generate(args) => {
                let yaml =
                    yaml_generators::generate_yaml(args.size, args.pattern.into(), args.seed);

                if args.verify {
                    succinctly::yaml::YamlIndex::build(yaml.as_bytes())
                        .map_err(|e| anyhow::anyhow!("Generated invalid YAML: {}", e))?;
                    eprintln!("✓ YAML validated successfully");
                }

                match args.output {
                    Some(path) => {
                        std::fs::write(&path, &yaml)?;
                        eprintln!("✓ Wrote {} bytes to {}", yaml.len(), path.display());
                    }
                    None => {
                        print!("{}", yaml);
                    }
                }

                Ok(())
            }
            YamlSubcommand::GenerateSuite(args) => generate_yaml_suite(args),
        },
        Command::Dev(dev_cmd) => match dev_cmd.command {
            DevSubcommand::Bench(bench_cmd) => match bench_cmd.command {
                BenchSubcommand::Jq(args) => run_jq_benchmark(args),
                BenchSubcommand::Dsv(args) => run_dsv_benchmark(args),
            },
        },
    }
}

/// Run jq benchmark
fn run_jq_benchmark(args: BenchJqArgs) -> Result<()> {
    let all_patterns = vec![
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
    let all_sizes = vec!["1kb", "10kb", "100kb", "1mb", "10mb", "100mb"];

    let patterns = if args.patterns == "all" {
        all_patterns.into_iter().map(String::from).collect()
    } else {
        args.patterns
            .split(',')
            .map(|s| s.trim().to_string())
            .collect()
    };

    let sizes = if args.sizes == "all" {
        all_sizes.into_iter().map(String::from).collect()
    } else {
        args.sizes
            .split(',')
            .map(|s| s.trim().to_string())
            .collect()
    };

    let config = jq_bench::BenchConfig {
        data_dir: args.data_dir,
        patterns,
        sizes,
        succinctly_binary: args.binary,
        warmup_runs: args.warmup,
        benchmark_runs: args.runs,
    };

    // Use default output paths if not specified
    let results_dir = PathBuf::from("data/bench/results");
    let default_jsonl = results_dir.join("jq-bench.jsonl");
    let default_md = results_dir.join("jq-bench.md");

    let output_jsonl = args.output.unwrap_or(default_jsonl);
    let output_md = args.markdown.unwrap_or(default_md);

    // Ensure results directory exists
    if let Some(parent) = output_jsonl.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let _results = jq_bench::run_benchmark(
        &config,
        Some(output_jsonl.as_path()),
        Some(output_md.as_path()),
    )?;

    Ok(())
}

/// Run DSV benchmark
fn run_dsv_benchmark(args: BenchDsvArgs) -> Result<()> {
    let all_patterns = vec![
        "tabular",
        "users",
        "numeric",
        "strings",
        "quoted",
        "multiline",
        "wide",
        "long",
        "mixed",
        "pathological",
    ];
    let all_sizes = vec!["1kb", "10kb", "100kb", "1mb", "10mb", "100mb"];

    let patterns = if args.patterns == "all" {
        all_patterns.into_iter().map(String::from).collect()
    } else {
        args.patterns
            .split(',')
            .map(|s| s.trim().to_string())
            .collect()
    };

    let sizes = if args.sizes == "all" {
        all_sizes.into_iter().map(String::from).collect()
    } else {
        args.sizes
            .split(',')
            .map(|s| s.trim().to_string())
            .collect()
    };

    let config = dsv_bench::BenchConfig {
        data_dir: args.data_dir,
        patterns,
        sizes,
        succinctly_binary: args.binary,
        warmup_runs: args.warmup,
        benchmark_runs: args.runs,
        delimiter: args.delimiter,
        query: args.query,
    };

    // Use default output paths if not specified
    let results_dir = PathBuf::from("data/bench/results");
    let default_jsonl = results_dir.join("dsv-bench.jsonl");
    let default_md = results_dir.join("dsv-bench.md");

    let output_jsonl = args.output.unwrap_or(default_jsonl);
    let output_md = args.markdown.unwrap_or(default_md);

    // Ensure results directory exists
    if let Some(parent) = output_jsonl.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let _results = dsv_bench::run_benchmark(
        &config,
        Some(output_jsonl.as_path()),
        Some(output_md.as_path()),
    )?;

    Ok(())
}

/// Suite configuration: patterns and sizes to generate
const SUITE_PATTERNS: &[(&str, generators::Pattern)] = &[
    ("comprehensive", generators::Pattern::Comprehensive),
    ("users", generators::Pattern::Users),
    ("nested", generators::Pattern::Nested),
    ("arrays", generators::Pattern::Arrays),
    ("mixed", generators::Pattern::Mixed),
    ("strings", generators::Pattern::Strings),
    ("numbers", generators::Pattern::Numbers),
    ("literals", generators::Pattern::Literals),
    ("unicode", generators::Pattern::Unicode),
    ("pathological", generators::Pattern::Pathological),
];

/// Sizes to generate for each pattern (name, bytes)
const SUITE_SIZES: &[(&str, usize)] = &[
    ("1kb", 1024),
    ("10kb", 10 * 1024),
    ("100kb", 100 * 1024),
    ("1mb", 1024 * 1024),
    ("10mb", 10 * 1024 * 1024),
    ("100mb", 100 * 1024 * 1024),
    ("1gb", 1024 * 1024 * 1024),
];

fn generate_json_suite(args: GenerateSuite) -> Result<()> {
    let output_dir = &args.output_dir;

    // Clean directory if requested
    if args.clean && output_dir.exists() {
        std::fs::remove_dir_all(output_dir)
            .with_context(|| format!("Failed to clean directory: {}", output_dir.display()))?;
        eprintln!("Cleaned {}", output_dir.display());
    }

    // Create output directory
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create directory: {}", output_dir.display()))?;

    let mut file_index: u64 = 0;
    let mut total_bytes: usize = 0;
    let mut file_count: usize = 0;
    let mut skipped_count: usize = 0;

    eprintln!(
        "Generating JSON suite in {} (max size: {})...",
        output_dir.display(),
        format_bytes(args.max_size)
    );

    for (pattern_name, pattern) in SUITE_PATTERNS {
        // Create pattern subdirectory
        let pattern_dir = output_dir.join(pattern_name);
        std::fs::create_dir_all(&pattern_dir)?;

        for (size_name, size) in SUITE_SIZES {
            // Skip files that exceed max_size
            if *size > args.max_size {
                skipped_count += 1;
                continue;
            }

            let filename = format!("{}.json", size_name);
            let path = pattern_dir.join(&filename);

            // Deterministic seed: base_seed + file_index
            let seed = args.seed.wrapping_add(file_index);
            file_index += 1;

            let json = generate_json(*size, *pattern, Some(seed), 5, 0.1);

            if args.verify {
                serde_json::from_str::<serde_json::Value>(&json)
                    .with_context(|| format!("Generated invalid JSON for {}", path.display()))?;
            }

            std::fs::write(&path, &json)?;
            total_bytes += json.len();
            file_count += 1;

            eprintln!(
                "  {} ({}, seed={})",
                path.display(),
                format_bytes(json.len()),
                seed
            );
        }
    }

    eprintln!();
    eprintln!(
        "Generated {} files ({} total)",
        file_count,
        format_bytes(total_bytes)
    );

    if skipped_count > 0 {
        eprintln!("Skipped {} files exceeding max size", skipped_count);
    }

    if args.verify {
        eprintln!("All files validated successfully");
    }

    Ok(())
}

/// DSV Suite configuration: patterns and sizes to generate
const DSV_SUITE_PATTERNS: &[(&str, dsv_generators::DsvPattern)] = &[
    ("tabular", dsv_generators::DsvPattern::Tabular),
    ("users", dsv_generators::DsvPattern::Users),
    ("numeric", dsv_generators::DsvPattern::Numeric),
    ("strings", dsv_generators::DsvPattern::Strings),
    ("quoted", dsv_generators::DsvPattern::Quoted),
    ("multiline", dsv_generators::DsvPattern::Multiline),
    ("wide", dsv_generators::DsvPattern::Wide),
    ("long", dsv_generators::DsvPattern::Long),
    ("mixed", dsv_generators::DsvPattern::Mixed),
    ("pathological", dsv_generators::DsvPattern::Pathological),
];

fn generate_dsv_suite(args: GenerateDsvSuite) -> Result<()> {
    let output_dir = &args.output_dir;

    // Clean directory if requested
    if args.clean && output_dir.exists() {
        std::fs::remove_dir_all(output_dir)
            .with_context(|| format!("Failed to clean directory: {}", output_dir.display()))?;
        eprintln!("Cleaned {}", output_dir.display());
    }

    // Create output directory
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create directory: {}", output_dir.display()))?;

    let mut file_index: u64 = 0;
    let mut total_bytes: usize = 0;
    let mut file_count: usize = 0;
    let mut skipped_count: usize = 0;

    let extension = if args.delimiter == '\t' { "tsv" } else { "csv" };

    eprintln!(
        "Generating DSV suite in {} (max size: {}, delimiter: {:?})...",
        output_dir.display(),
        format_bytes(args.max_size),
        args.delimiter
    );

    for (pattern_name, pattern) in DSV_SUITE_PATTERNS {
        // Create pattern subdirectory
        let pattern_dir = output_dir.join(pattern_name);
        std::fs::create_dir_all(&pattern_dir)?;

        for (size_name, size) in SUITE_SIZES {
            // Skip files that exceed max_size
            if *size > args.max_size {
                skipped_count += 1;
                continue;
            }

            let filename = format!("{}.{}", size_name, extension);
            let path = pattern_dir.join(&filename);

            // Deterministic seed: base_seed + file_index
            let seed = args.seed.wrapping_add(file_index);
            file_index += 1;

            let dsv = dsv_generators::generate_dsv(
                *size,
                *pattern,
                Some(seed),
                args.delimiter,
                true, // include_header
            );

            if args.verify {
                let config = succinctly::DsvConfig::default().with_delimiter(args.delimiter as u8);
                let parsed = succinctly::Dsv::parse_with_config(dsv.as_bytes(), &config);
                if parsed.row_count() == 0 && !dsv.is_empty() {
                    anyhow::bail!("Generated invalid DSV for {}", path.display());
                }
            }

            std::fs::write(&path, &dsv)?;
            total_bytes += dsv.len();
            file_count += 1;

            eprintln!(
                "  {} ({}, seed={})",
                path.display(),
                format_bytes(dsv.len()),
                seed
            );
        }
    }

    eprintln!();
    eprintln!(
        "Generated {} files ({} total)",
        file_count,
        format_bytes(total_bytes)
    );

    if skipped_count > 0 {
        eprintln!("Skipped {} files exceeding max size", skipped_count);
    }

    if args.verify {
        eprintln!("All files validated successfully");
    }

    Ok(())
}

/// YAML Suite configuration: patterns and sizes to generate
const YAML_SUITE_PATTERNS: &[(&str, yaml_generators::YamlPattern)] = &[
    ("comprehensive", yaml_generators::YamlPattern::Comprehensive),
    ("users", yaml_generators::YamlPattern::Users),
    ("nested", yaml_generators::YamlPattern::Nested),
    ("sequences", yaml_generators::YamlPattern::Sequences),
    ("mixed", yaml_generators::YamlPattern::Mixed),
    ("strings", yaml_generators::YamlPattern::Strings),
    ("numbers", yaml_generators::YamlPattern::Numbers),
    ("config", yaml_generators::YamlPattern::Config),
    ("unicode", yaml_generators::YamlPattern::Unicode),
    ("pathological", yaml_generators::YamlPattern::Pathological),
];

fn generate_yaml_suite(args: GenerateYamlSuite) -> Result<()> {
    let output_dir = &args.output_dir;

    // Clean directory if requested
    if args.clean && output_dir.exists() {
        std::fs::remove_dir_all(output_dir)
            .with_context(|| format!("Failed to clean directory: {}", output_dir.display()))?;
        eprintln!("Cleaned {}", output_dir.display());
    }

    // Create output directory
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create directory: {}", output_dir.display()))?;

    let mut file_index: u64 = 0;
    let mut total_bytes: usize = 0;
    let mut file_count: usize = 0;
    let mut skipped_count: usize = 0;

    eprintln!(
        "Generating YAML suite in {} (max size: {})...",
        output_dir.display(),
        format_bytes(args.max_size)
    );

    for (pattern_name, pattern) in YAML_SUITE_PATTERNS {
        // Create pattern subdirectory
        let pattern_dir = output_dir.join(pattern_name);
        std::fs::create_dir_all(&pattern_dir)?;

        for (size_name, size) in SUITE_SIZES {
            // Skip files that exceed max_size
            if *size > args.max_size {
                skipped_count += 1;
                continue;
            }

            let filename = format!("{}.yaml", size_name);
            let path = pattern_dir.join(&filename);

            // Deterministic seed: base_seed + file_index
            let seed = args.seed.wrapping_add(file_index);
            file_index += 1;

            let yaml = yaml_generators::generate_yaml(*size, *pattern, Some(seed));

            if args.verify {
                if let Err(e) = succinctly::yaml::YamlIndex::build(yaml.as_bytes()) {
                    anyhow::bail!("Generated invalid YAML for {}: {}", path.display(), e);
                }
            }

            std::fs::write(&path, &yaml)?;
            total_bytes += yaml.len();
            file_count += 1;

            eprintln!(
                "  {} ({}, seed={})",
                path.display(),
                format_bytes(yaml.len()),
                seed
            );
        }
    }

    eprintln!();
    eprintln!(
        "Generated {} files ({} total)",
        file_count,
        format_bytes(total_bytes)
    );

    if skipped_count > 0 {
        eprintln!("Skipped {} files exceeding max size", skipped_count);
    }

    if args.verify {
        eprintln!("All files validated successfully");
    }

    Ok(())
}

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

mod dsv_bench;
mod dsv_generators;
mod generators;
mod jq_bench;
mod jq_locate;
mod jq_runner;
mod yaml_generators;
use generators::generate_json;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size() {
        // Plain numbers
        assert_eq!(parse_size("1024").unwrap(), 1024);

        // Bytes (case insensitive)
        assert_eq!(parse_size("100b").unwrap(), 100);
        assert_eq!(parse_size("100B").unwrap(), 100);

        // Kilobytes
        assert_eq!(parse_size("1kb").unwrap(), 1024);
        assert_eq!(parse_size("1KB").unwrap(), 1024);
        assert_eq!(parse_size("1Kb").unwrap(), 1024);
        assert_eq!(parse_size("512kb").unwrap(), 512 * 1024);

        // Megabytes
        assert_eq!(parse_size("1mb").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("1MB").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("1Mb").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("10mb").unwrap(), 10 * 1024 * 1024);

        // Gigabytes
        assert_eq!(parse_size("1gb").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("1GB").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("2Gb").unwrap(), 2 * 1024 * 1024 * 1024);

        // With whitespace
        assert_eq!(parse_size(" 1mb ").unwrap(), 1024 * 1024);

        // Errors
        assert!(parse_size("abc").is_err());
        assert!(parse_size("1tb").is_err());
        assert!(parse_size("").is_err());
    }
}
