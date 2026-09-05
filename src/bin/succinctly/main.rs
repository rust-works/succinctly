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
    /// Command-line YAML processor (jq-compatible syntax)
    Yq(YqCommand),
    /// Find jq expression for a position in a JSON file
    JqLocate(jq_locate::JqLocateArgs),
    /// Find yq expression for a position in a YAML file
    YqLocate(yq_locate::YqLocateArgs),
    /// Developer tools (benchmarking, profiling)
    Dev(DevCommand),
    /// Install short alias symlinks (sjq, syq, sjq-locate, syq-locate)
    InstallAliases(InstallAliasesArgs),
    /// Unified benchmark runner (list and run all benchmarks)
    #[cfg(feature = "bench-runner")]
    Bench(BenchRunnerCommand),
    /// Text processing operations (UTF-8 validation, etc.)
    Text(TextCommand),
}

#[derive(Debug, Parser)]
struct TextCommand {
    #[command(subcommand)]
    command: TextSubcommand,
}

#[derive(Debug, Subcommand)]
enum TextSubcommand {
    /// Text validation operations
    Validate(TextValidateCommand),
    /// Generate UTF-8 text files for benchmarking and testing
    Generate(GenerateUtf8),
    /// Generate a suite of UTF-8 files with various sizes and patterns
    GenerateSuite(GenerateUtf8Suite),
}

#[derive(Debug, Parser)]
struct TextValidateCommand {
    #[command(subcommand)]
    command: TextValidateSubcommand,
}

#[derive(Debug, Subcommand)]
enum TextValidateSubcommand {
    /// Validate UTF-8 encoding
    Utf8(text_validate::ValidateUtf8Args),
}

#[derive(Debug, Parser)]
struct GenerateUtf8 {
    /// Size of text to generate (supports b, kb, mb, gb - case insensitive)
    /// Examples: 1024, 1kb, 512MB, 2Gb
    #[arg(value_parser = parse_size)]
    size: usize,

    /// Output file path (defaults to stdout)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// UTF-8 pattern to generate
    #[arg(short, long, default_value = "mixed")]
    pattern: Utf8PatternArg,

    /// Random seed for reproducible generation
    #[arg(short, long)]
    seed: Option<u64>,

    /// Verify generated content is valid UTF-8
    #[arg(long)]
    verify: bool,
}

#[derive(Debug, Parser)]
struct GenerateUtf8Suite {
    /// Output directory (defaults to data/bench/generated/utf8)
    #[arg(short, long, default_value = "data/bench/generated/utf8")]
    output_dir: PathBuf,

    /// Base seed for deterministic generation (each file uses seed + file_index)
    #[arg(short, long, default_value = "42")]
    seed: u64,

    /// Clean output directory before generating
    #[arg(long)]
    clean: bool,

    /// Verify all generated files are valid UTF-8
    #[arg(long)]
    verify: bool,

    /// Maximum file size to generate (files larger than this are skipped)
    /// Supports units: b, kb, mb, gb (e.g., "100mb", "1gb")
    #[arg(short, long, value_parser = parse_size, default_value = "1gb")]
    max_size: usize,
}

#[derive(Debug, Clone, ValueEnum)]
enum Utf8PatternArg {
    /// Pure ASCII (7-bit, single-byte sequences)
    Ascii,
    /// Latin Extended characters (2-byte sequences: accents, diacritics)
    Latin,
    /// Greek and Cyrillic (2-byte sequences)
    GreekCyrillic,
    /// Chinese/Japanese/Korean (3-byte sequences)
    Cjk,
    /// Emoji and symbols (4-byte sequences)
    Emoji,
    /// Mixed realistic content (prose with occasional non-ASCII)
    Mixed,
    /// Uniform mix of all sequence lengths (1-4 bytes)
    AllLengths,
    /// Log file style (mostly ASCII with timestamps)
    LogFile,
    /// Source code style (ASCII with unicode in strings/comments)
    SourceCode,
    /// JSON-like structure with unicode strings
    JsonLike,
    /// Pathological: maximum multi-byte density
    Pathological,
}

impl From<Utf8PatternArg> for text_generators::Utf8Pattern {
    fn from(arg: Utf8PatternArg) -> Self {
        match arg {
            Utf8PatternArg::Ascii => Self::Ascii,
            Utf8PatternArg::Latin => Self::Latin,
            Utf8PatternArg::GreekCyrillic => Self::GreekCyrillic,
            Utf8PatternArg::Cjk => Self::Cjk,
            Utf8PatternArg::Emoji => Self::Emoji,
            Utf8PatternArg::Mixed => Self::Mixed,
            Utf8PatternArg::AllLengths => Self::AllLengths,
            Utf8PatternArg::LogFile => Self::LogFile,
            Utf8PatternArg::SourceCode => Self::SourceCode,
            Utf8PatternArg::JsonLike => Self::JsonLike,
            Utf8PatternArg::Pathological => Self::Pathological,
        }
    }
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
    /// Validate JSON files strictly according to RFC 8259
    Validate(json_validate::ValidateArgs),
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
    /// Validate YAML files strictly (opt-in; mirrors `json validate`)
    Validate(yaml_validate::ValidateArgs),
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
    /// Report how many words the select word scans traverse (#40)
    SelectStats(SelectStatsArgs),
}

/// Arguments for the select scan-length report.
///
/// Needs a binary built with the `select-stats` feature; without it the
/// counters are compiled out and the command exits non-zero rather than
/// printing zeros that look like a finding.
#[derive(Debug, Parser)]
struct SelectStatsArgs {
    /// Corpus root directory to traverse (recursively, by file extension)
    #[arg(short, long, default_value = "data/bench/corpus")]
    data_dir: PathBuf,

    /// Markdown report to write (default: print to stdout)
    #[arg(short, long)]
    markdown: Option<PathBuf>,
}

#[derive(Debug, Parser)]
struct InstallAliasesArgs {
    /// Directory to create symlinks in (default: same directory as the binary)
    #[arg(long, value_name = "DIR")]
    dir: Option<PathBuf>,
}

/// Unified benchmark runner command.
#[cfg(feature = "bench-runner")]
#[derive(Debug, Parser)]
struct BenchRunnerCommand {
    #[command(subcommand)]
    command: BenchRunnerSubcommand,
}

/// Unified benchmark runner subcommands.
#[cfg(feature = "bench-runner")]
#[derive(Debug, Subcommand)]
enum BenchRunnerSubcommand {
    /// List all available benchmarks
    List(bench_runner::ListArgs),
    /// Run one or more benchmarks
    Run(bench_runner::RunArgs),
    /// Run benchmarks across configured SSH nodes (issue #98)
    Orchestrate(bench_runner::OrchestrateArgs),
    /// Cross-compile and deploy the release binary to configured nodes (issue #98)
    Sync(bench_runner::SyncArgs),
    /// Report node status, or start/stop EC2 instances (issue #98)
    Nodes(bench_runner::NodesArgs),
    /// Compare orchestrated results across nodes/architectures (issue #98)
    Report(bench_runner::ReportArgs),
}

/// Default alias names installed by `install-aliases`.
const MULTICALL_ALIASES: &[&str] = &["sjq", "syq", "sjq-locate", "syq-locate"];

#[derive(Debug, Parser)]
struct BenchCommand {
    #[command(subcommand)]
    command: BenchSubcommand,
}

#[derive(Debug, Subcommand)]
enum BenchSubcommand {
    /// Benchmark succinctly jq vs system jq
    Jq(BenchJqArgs),
    /// Benchmark succinctly yq vs system yq
    Yq(BenchYqArgs),
    /// Benchmark succinctly jq with DSV input
    Dsv(BenchDsvArgs),
    /// Benchmark succinctly UTF-8 validation vs std::str::from_utf8
    Utf8(BenchUtf8Args),
    /// Report shape statistics for the real-workload corpus (#301)
    CorpusStats(BenchCorpusStatsArgs),
}

/// Arguments for the real-workload corpus shape-statistics report.
#[derive(Debug, Parser)]
struct BenchCorpusStatsArgs {
    /// Corpus root directory to scan (recursively, by file extension)
    #[arg(short, long, default_value = "data/bench/corpus")]
    data_dir: PathBuf,

    /// Markdown report to write (or, with --check, the golden to compare against)
    #[arg(short, long)]
    markdown: Option<PathBuf>,

    /// Optional JSONL file of per-file inventory records
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Verify the generated report matches --markdown instead of writing it
    #[arg(long)]
    check: bool,
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

    /// Query types to benchmark (comma-separated, or "all")
    /// Available: identity (.), keys_unsorted, keys_unsorted_length,
    /// keys_unsorted_map, keys_unsorted_select, map, select
    #[arg(short, long, default_value = "identity")]
    queries: String,

    /// Number of warmup runs before benchmarking
    #[arg(long, default_value = "1")]
    warmup: usize,

    /// Number of benchmark runs (median is taken)
    #[arg(long, default_value = "3")]
    runs: usize,

    /// Path to succinctly binary
    #[arg(long, default_value = "./target/release/succinctly")]
    binary: PathBuf,

    /// Skip memory measurement (memory is collected by default)
    #[arg(long)]
    no_memory: bool,
}

/// Arguments for yq benchmark
#[derive(Debug, Parser)]
struct BenchYqArgs {
    /// Directory containing generated YAML files
    #[arg(short, long, default_value = "data/bench/generated/yaml")]
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

    /// Query types to benchmark (comma-separated, or "all")
    /// Available: identity (.), first_element (.[0]), iteration (.[]), length
    #[arg(short, long, default_value = "identity")]
    queries: String,

    /// Skip memory measurement (memory is collected by default)
    #[arg(long)]
    no_memory: bool,

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

    /// Skip memory measurement (memory is collected by default)
    #[arg(long)]
    no_memory: bool,
}

/// Arguments for UTF-8 validation benchmark
#[derive(Debug, Parser)]
struct BenchUtf8Args {
    /// Directory containing generated UTF-8 files
    #[arg(short, long, default_value = "data/bench/generated/utf8")]
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

    /// Validation engine to time: auto, scalar, broadword or std
    #[arg(long, default_value = "auto")]
    engine: String,
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
    /// Indented, pretty-printed documents (tests whitespace skipping)
    Pretty,
    /// Wide flat object: many distinct top-level keys, no nesting (tests keys_unsorted)
    Wide,
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

    /// Include header row (default; --header and --no-header override each other)
    #[arg(long, overrides_with = "no_header")]
    header: bool,

    /// Omit the header row
    #[arg(long)]
    no_header: bool,

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
            DsvPatternArg::Tabular => Self::Tabular,
            DsvPatternArg::Users => Self::Users,
            DsvPatternArg::Numeric => Self::Numeric,
            DsvPatternArg::Strings => Self::Strings,
            DsvPatternArg::Quoted => Self::Quoted,
            DsvPatternArg::Multiline => Self::Multiline,
            DsvPatternArg::Wide => Self::Wide,
            DsvPatternArg::Long => Self::Long,
            DsvPatternArg::Mixed => Self::Mixed,
            DsvPatternArg::Pathological => Self::Pathological,
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
    /// Top-level array for M2 navigation benchmarks (.[0], .[], .[0].name)
    Navigation,
    /// Flow mappings `{...}` and sequences `[...]`
    Flow,
    /// Anchors `&name` and aliases `*name`
    Anchors,
    /// Literal `|` and folded `>` block scalars, with chomping modifiers
    BlockScalars,
    /// Explicit keys (`? ` / `: `)
    ExplicitKeys,
    /// Multi-document streams (`---` / `...`)
    MultiDoc,
    /// Top-level sequence of childless bare-dash items (`-` alone on its own
    /// line, reads as `null`) — the shape #337 found quadratic
    EmptyItems,
    /// Childless bare-dash items interleaved with items carrying inline
    /// scalar content (`- x`)
    HalfEmptyItems,
}

impl From<YamlPatternArg> for yaml_generators::YamlPattern {
    fn from(arg: YamlPatternArg) -> Self {
        match arg {
            YamlPatternArg::Comprehensive => Self::Comprehensive,
            YamlPatternArg::Users => Self::Users,
            YamlPatternArg::Nested => Self::Nested,
            YamlPatternArg::Sequences => Self::Sequences,
            YamlPatternArg::Mixed => Self::Mixed,
            YamlPatternArg::Strings => Self::Strings,
            YamlPatternArg::Numbers => Self::Numbers,
            YamlPatternArg::Config => Self::Config,
            YamlPatternArg::Unicode => Self::Unicode,
            YamlPatternArg::Pathological => Self::Pathological,
            YamlPatternArg::Navigation => Self::Navigation,
            YamlPatternArg::Flow => Self::Flow,
            YamlPatternArg::Anchors => Self::Anchors,
            YamlPatternArg::BlockScalars => Self::BlockScalars,
            YamlPatternArg::ExplicitKeys => Self::ExplicitKeys,
            YamlPatternArg::MultiDoc => Self::MultiDoc,
            YamlPatternArg::EmptyItems => Self::EmptyItems,
            YamlPatternArg::HalfEmptyItems => Self::HalfEmptyItems,
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

/// The three jq flags that together form one output-format knob (#2009) --
/// shared so `overrides_with_all` on each of `JqCommand`'s three fields
/// below names one definition instead of three hand-copied literals that
/// could silently drift out of sync if a fourth output-format flag is ever
/// added (CLAUDE.md: "duplicated predicates diverge silently"). A simpler
/// single-pair version of this same clap idiom already exists at
/// `header`'s own `overrides_with = "no_header"` (DSV generation, below) --
/// that one is one-directional and needs no command-level
/// `args_override_self` since it's a two-flag pair with no repeat-tolerance
/// requirement, unlike this three-way mutual group.
const JQ_OUTPUT_FORMAT_FLAGS: &[&str] = &["compact_output", "tab", "indent"];

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
    ///
    /// `overrides_with = "null_input"` (#2009): clap's derive default
    /// hard-errors on a repeated flag ("cannot be used multiple times") for
    /// every plain bool field, unlike real jq, which tolerates any flag
    /// given twice (a wrapper script or alias that already passes a flag
    /// should not make a caller's own repeat of it a hard failure) --
    /// live-verified against jq 1.7.1 for `-n -n` specifically, and for
    /// `-r -j`/`-j -r` (see those fields' own attributes below). Applied
    /// per-bool-field rather than as one command-level
    /// `args_override_self` (code review, #2009): that setting also covers
    /// `ArgAction::Set` value options like `--from-file`/`--input-dsv`,
    /// and real jq's own `-f a.jq -f b.jq` does *not* behave as simple
    /// last-wins (live-verified: it errors trying to concatenate both
    /// files into one program) -- so the blanket command-level version
    /// would have silently introduced a new, unverified divergence there.
    #[arg(short = 'n', long, overrides_with = "null_input")]
    null_input: bool,

    /// Read each line as a string instead of JSON
    #[arg(short = 'R', long, overrides_with = "raw_input")]
    raw_input: bool,

    /// [Extension] Read input as DSV (delimiter-separated values).
    /// Each row becomes a JSON array of strings.
    /// Properly handles quoted fields with embedded delimiters and newlines.
    /// Use comma for CSV, tab for TSV, or any single ASCII character.
    /// Special CSV characters (quote " and newline) cannot be used as delimiters.
    #[arg(long, value_name = "DELIMITER")]
    input_dsv: Option<char>,

    /// Read all inputs into an array and use it as the single input value
    #[arg(short = 's', long, overrides_with = "slurp")]
    slurp: bool,

    /// Validate JSON strictly according to RFC 8259 before processing.
    /// Reports detailed validation errors with line:column positions.
    #[arg(long, overrides_with = "validate")]
    validate: bool,

    // === Output Options ===
    /// Compact output (no pretty printing)
    ///
    /// `-c`/`--tab`/`--indent` are one output-format knob in real jq --
    /// whichever is given last wins (#2009, live-verified against jq 1.7.1:
    /// `jq -c --tab '.'` pretty-prints with tabs, `jq --tab -c '.'` stays
    /// compact). `overrides_with_all` on each of the three, naming all
    /// three (itself included, so a later repeat of the same flag still
    /// wins over an earlier different one, with no need for a separate
    /// self-only `overrides_with` alongside it), reproduces that ordering.
    #[arg(short = 'c', long, overrides_with_all = JQ_OUTPUT_FORMAT_FLAGS)]
    compact_output: bool,

    /// Output raw strings without quotes
    ///
    /// See `null_input`'s own doc comment for the shared repeat-tolerance
    /// rationale (#2009) -- `-r -j`/`-j -r` specifically live-verified
    /// against jq 1.7.1.
    #[arg(short = 'r', long, overrides_with = "raw_output")]
    raw_output: bool,

    /// Like -r but don't print newline after each output
    #[arg(short = 'j', long, overrides_with = "join_output")]
    join_output: bool,

    /// Like -r but print NUL instead of newline after each output
    #[arg(long, overrides_with = "raw_output0")]
    raw_output0: bool,

    /// Output ASCII only, escaping non-ASCII as \uXXXX
    #[arg(short = 'a', long, overrides_with = "ascii_output")]
    ascii_output: bool,

    /// Colorize output (default if stdout is a terminal)
    #[arg(short = 'C', long, overrides_with = "color_output")]
    color_output: bool,

    /// Disable colorized output
    #[arg(short = 'M', long, overrides_with = "monochrome_output")]
    monochrome_output: bool,

    /// Sort keys of each object on output
    #[arg(short = 'S', long, overrides_with = "sort_keys")]
    sort_keys: bool,

    /// Preserve original input formatting (numbers like 4e4, escape sequences)
    /// Can also be enabled via SUCCINCTLY_PRESERVE_INPUT=1 environment variable
    #[arg(long, overrides_with = "preserve_input")]
    preserve_input: bool,

    /// Use tabs for indentation
    ///
    /// See `compact_output`'s own doc comment for the shared
    /// last-flag-wins rationale (#2009).
    #[arg(long, overrides_with_all = JQ_OUTPUT_FORMAT_FLAGS)]
    tab: bool,

    /// Use n spaces for indentation (max 7)
    ///
    /// See `compact_output`'s own doc comment for the shared
    /// last-flag-wins rationale (#2009).
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=7), overrides_with_all = JQ_OUTPUT_FORMAT_FLAGS)]
    indent: Option<u8>,

    // === Program Input ===
    /// Read filter from file instead of command line
    #[arg(short = 'f', long, value_name = "FILE")]
    from_file: Option<PathBuf>,

    // === Variables ===
    // `allow_hyphen_values = true` below (#1150) lets a VALUE/FILE that
    // legitimately starts with `-` (a negative number, a hyphen-prefixed
    // filename) through clap, matching real jq. The tradeoff -- clap can
    // no longer tell "the user forgot VALUE" from "VALUE legitimately
    // starts with -" -- is real (a forgotten VALUE now silently swallows
    // the next real flag as its value instead of erroring), but it's not
    // a regression versus the oracle: confirmed live that real jq has the
    // identical footgun (`jq --arg n -c '.'` silently sets $n="-c" and
    // drops -c/compact-output there too). yq mode's `arg`/`argjson` below
    // have no such oracle precedent (real yq has no equivalent flags at
    // all, #284) but carry the same tradeoff for consistency with jq mode.
    /// Set $name to the string value
    #[arg(long, value_names = ["NAME", "VALUE"], num_args = 2, action = clap::ArgAction::Append, allow_hyphen_values = true)]
    arg: Vec<String>,

    /// Set $name to the JSON value
    #[arg(long, value_names = ["NAME", "VALUE"], num_args = 2, action = clap::ArgAction::Append, allow_hyphen_values = true)]
    argjson: Vec<String>,

    /// Set $name to an array of JSON values read from file
    #[arg(long, value_names = ["NAME", "FILE"], num_args = 2, action = clap::ArgAction::Append, allow_hyphen_values = true)]
    slurpfile: Vec<String>,

    /// Set $name to the string contents of file
    #[arg(long, value_names = ["NAME", "FILE"], num_args = 2, action = clap::ArgAction::Append, allow_hyphen_values = true)]
    rawfile: Vec<String>,

    /// Consume remaining arguments as positional string values
    #[arg(long, num_args = 0.., value_name = "STRINGS", allow_hyphen_values = true)]
    args: Vec<String>,

    /// Consume remaining arguments as positional JSON values
    #[arg(long, num_args = 0.., value_name = "JSON_VALUES", allow_hyphen_values = true)]
    jsonargs: Vec<String>,

    // === Modules ===
    /// Prepend directory to module search path
    #[arg(short = 'L', value_name = "DIR", action = clap::ArgAction::Append, allow_hyphen_values = true)]
    library_path: Vec<PathBuf>,

    // === Formats ===
    /// Parse input/output as application/json-seq (RFC 7464)
    #[arg(long, overrides_with = "seq")]
    seq: bool,

    // === Exit Status ===
    /// Set exit status based on output (0 if last output != false/null)
    #[arg(short = 'e', long, overrides_with = "exit_status")]
    exit_status: bool,

    /// Flush output after each JSON value
    #[arg(long, overrides_with = "unbuffered")]
    unbuffered: bool,

    // === Info ===
    /// Show version information
    #[arg(short = 'V', long, overrides_with = "version")]
    version: bool,

    /// Show build configuration
    #[arg(long, overrides_with = "build_configuration")]
    build_configuration: bool,
}

/// Output format for yq command
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// YAML output (default)
    #[default]
    #[value(name = "yaml", alias = "y")]
    Yaml,
    /// JSON output
    #[value(name = "json", alias = "j")]
    Json,
    /// Auto-detect based on input
    #[value(name = "auto", alias = "a")]
    Auto,
}

/// Input format for yq command
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum InputFormat {
    /// Auto-detect based on file extension (default)
    #[default]
    #[value(name = "auto", alias = "a")]
    Auto,
    /// YAML input
    #[value(name = "yaml", alias = "y")]
    Yaml,
    /// JSON input
    #[value(name = "json", alias = "j")]
    Json,
}

/// Front matter handling mode for `--front-matter`
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum FrontMatterMode {
    /// Evaluate the expression against only the YAML front matter, discard
    /// the trailing body content.
    #[value(name = "extract")]
    Extract,
    /// Evaluate the expression against the YAML front matter, then re-emit
    /// the transformed front matter followed by the original trailing body,
    /// unchanged.
    #[value(name = "process")]
    Process,
}

/// Command-line YAML processor (yq-compatible)
///
/// Bool flags below carry `overrides_with = "self"` (#2009): same
/// clap-derive default gap `JqCommand` had (see `null_input`'s own doc
/// comment there) -- `syq -n -n`/`--tab --tab` etc. used to hard-error,
/// where the pinned yq oracle (v4.53.3) tolerates any repeated flag
/// (live-verified: `yq -n -n '.'` succeeds there). Applied per-field, not
/// as one command-level `args_override_self`, for the same reason
/// `JqCommand` uses per-field attributes -- that setting would also cover
/// `--from-file`/`--split-exp`/etc., unverified against the oracle here.
/// yq's own `--tab`/`--indent`/output-format flags have a different shape
/// than jq's (`-I` takes a default value, not `Option`; no `-c` at all)
/// and would need their own oracle verification before adding an
/// `overrides_with_all` last-wins group, not attempted here.
#[derive(Debug, Parser)]
#[command(name = "yq")]
#[command(about = "Command-line YAML processor (yq-compatible)", long_about = None)]
pub struct YqCommand {
    /// jq filter expression (e.g., ".", ".foo", ".[]")
    /// If not provided, uses "." (identity)
    pub filter: Option<String>,

    /// Input files (reads from stdin if none provided)
    #[arg(trailing_var_arg = true)]
    pub files: Vec<String>,

    // === Input Options ===
    /// Don't read any input; use null as the single input value
    #[arg(short = 'n', long, overrides_with = "null_input")]
    pub null_input: bool,

    /// Read each line as a string instead of parsing as YAML/JSON
    #[arg(short = 'R', long, overrides_with = "raw_input")]
    pub raw_input: bool,

    /// Read all inputs into an array and use it as the single input value
    #[arg(short = 's', long, overrides_with = "slurp")]
    pub slurp: bool,

    /// Evaluate the filter once over the list of every document of every
    /// file (yq-compatible name: `eval-all`/`ea`), exposing `file_index`/
    /// `fileIndex`/`fi` for cross-file merges. The filter never sees a
    /// containing array, so it applies per document (`select(file_index ==
    /// 0)`, not `.[] | select(...)`); `[E]` collects across the whole list
    /// and a binary operator pairs both operands over it, matching real yq
    /// (#715, #2427). See docs/reference/yq-language.md.
    #[arg(long = "eval-all", alias = "ea", overrides_with = "eval_all")]
    pub eval_all: bool,

    /// Validate YAML strictly (opt-in) before processing. Reports line:column
    /// errors and exits without producing output on the first violation.
    #[arg(long, overrides_with = "validate")]
    pub validate: bool,

    /// Accept jq-only builtins real yq's lexer rejects (`paths`, `getpath`,
    /// `limit`, `gsub`/`scan`/`splits`, `leaf_paths`, etc.) as a succinctly
    /// extension. Off by default so `succinctly yq` matches real yq's
    /// syntax surface (#1512).
    #[arg(long, overrides_with = "jq_extensions")]
    pub jq_extensions: bool,

    /// Input format type [auto, yaml, json] (default: auto)
    #[arg(
        short = 'p',
        long = "input-format",
        value_name = "FORMAT",
        default_value = "auto"
    )]
    pub input_format: InputFormat,

    /// Treat input as text with a `---`-fenced YAML front matter header
    /// (e.g. Markdown). `extract` evaluates the expression against just the
    /// front matter and discards the body; `process` re-emits the
    /// transformed front matter followed by the untouched body.
    #[arg(long = "front-matter", value_name = "MODE")]
    pub front_matter: Option<FrontMatterMode>,

    // === Output Options ===
    /// Output format type [yaml, json, auto] (default: yaml)
    #[arg(short = 'o', long, value_name = "FORMAT", default_value = "yaml")]
    pub output_format: OutputFormat,

    /// Unwrap scalar values, print without quotes (default for YAML)
    #[arg(short = 'r', long = "unwrapScalar", overrides_with = "raw_output")]
    pub raw_output: bool,

    /// Like -r but don't print newline after each output
    #[arg(short = 'j', long, overrides_with = "join_output")]
    pub join_output: bool,

    /// Use NUL char to separate values instead of newline
    #[arg(short = '0', long = "nul-output", overrides_with = "nul_output")]
    pub nul_output: bool,

    /// Output ASCII only, escaping non-ASCII as \uXXXX
    #[arg(short = 'a', long, overrides_with = "ascii_output")]
    pub ascii_output: bool,

    /// Force colorized output
    #[arg(short = 'C', long = "colors", overrides_with = "color_output")]
    pub color_output: bool,

    /// Disable colorized output
    #[arg(short = 'M', long = "no-colors", overrides_with = "monochrome_output")]
    pub monochrome_output: bool,

    /// Sort keys of each object on output
    #[arg(short = 'S', long, overrides_with = "sort_keys")]
    pub sort_keys: bool,

    /// Don't print document separators (---)
    #[arg(short = 'N', long = "no-doc", overrides_with = "no_doc")]
    pub no_doc: bool,

    /// Select specific document by 0-based index from multi-document stream
    #[arg(long = "doc", value_name = "N")]
    pub document: Option<usize>,

    /// Pretty print, expand flow styles to block style (currently a no-op:
    /// output is already always block-style pending style preservation, #707)
    #[arg(short = 'P', long = "prettyPrint", overrides_with = "pretty_print")]
    pub pretty_print: bool,

    /// Use tabs for indentation. Write-only: succinctly's own YAML reader (like
    /// the wider YAML 1.1/1.2 spec) forbids tab characters in indentation, so
    /// this flag's YAML output cannot be read back by `succinctly yq` itself,
    /// or by other spec-strict YAML parsers (#1684).
    #[arg(long, overrides_with = "tab")]
    pub tab: bool,

    /// Sets indent level for output (default 2). 0 means compact/flow for
    /// JSON output, but for YAML (whose block style can't go flow-compact
    /// the same way) means "use a small default width" instead (#1575).
    #[arg(short = 'I', long, value_name = "N", default_value = "2", value_parser = clap::value_parser!(u8).range(0..=7))]
    pub indent: u8,

    /// Update the file in place
    #[arg(short = 'i', long, overrides_with = "inplace")]
    pub inplace: bool,

    /// Split output into multiple files, one per result, named by evaluating
    /// EXPR against each result (`.` is the result; `$index` is that
    /// result's zero-based output index across the whole run; --arg/
    /// --argjson values and $ARGS are also available, same as the main
    /// filter). Suppresses normal stdout output.
    ///
    /// Deliberately long-only, unlike real yq's `-s`/`--split-exp`:
    /// succinctly's `-s` is already `--slurp` (#715).
    #[arg(long = "split-exp", value_name = "EXPR")]
    pub split_exp: Option<String>,

    // === Program Input ===
    /// Read filter from file instead of command line
    #[arg(long = "from-file", value_name = "FILE")]
    pub from_file: Option<PathBuf>,

    // === Variables ===
    /// Set $name to the string value
    #[arg(long, value_names = ["NAME", "VALUE"], num_args = 2, action = clap::ArgAction::Append, allow_hyphen_values = true)]
    pub arg: Vec<String>,

    /// Set $name to the JSON value
    #[arg(long, value_names = ["NAME", "VALUE"], num_args = 2, action = clap::ArgAction::Append, allow_hyphen_values = true)]
    pub argjson: Vec<String>,

    // === Exit Status ===
    /// Set exit status based on output (0 if last output != false/null)
    #[arg(short = 'e', long, overrides_with = "exit_status")]
    pub exit_status: bool,

    // === Info ===
    /// Show version information
    #[arg(short = 'V', long, overrides_with = "version")]
    pub version: bool,

    /// Show build configuration
    #[arg(long, overrides_with = "build_configuration")]
    pub build_configuration: bool,
}

impl From<PatternArg> for generators::Pattern {
    fn from(arg: PatternArg) -> Self {
        match arg {
            PatternArg::Comprehensive => Self::Comprehensive,
            PatternArg::Users => Self::Users,
            PatternArg::Nested => Self::Nested,
            PatternArg::Arrays => Self::Arrays,
            PatternArg::Mixed => Self::Mixed,
            PatternArg::Strings => Self::Strings,
            PatternArg::Numbers => Self::Numbers,
            PatternArg::Literals => Self::Literals,
            PatternArg::Unicode => Self::Unicode,
            PatternArg::Pathological => Self::Pathological,
            PatternArg::Pretty => Self::Pretty,
            PatternArg::Wide => Self::Wide,
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
            "Invalid size format: '{s}'. Use format like '1mb', '512KB', or '1024'"
        ));
    };

    num_str
        .trim()
        .parse::<usize>()
        .map_err(|_| format!("Invalid number in size: '{s}'"))
        .and_then(|n| {
            n.checked_mul(unit)
                .ok_or_else(|| format!("Size too large: '{s}'"))
        })
}

/// Multi-call binary support: detect if invoked via a known alias name.
///
/// When the binary is symlinked as `sjq`, `syq`, etc., this function
/// detects the alias from argv[0] and dispatches directly to the
/// appropriate subcommand, bypassing the top-level Cli parser.
///
/// Returns `Some(exit_code)` if a multi-call alias was detected,
/// or `None` to fall through to normal `Cli::parse()`.
fn try_multicall() -> Result<Option<i32>> {
    let binary_name = std::env::args()
        .next()
        .as_deref()
        .and_then(|s| std::path::Path::new(s).file_name())
        .and_then(|n| n.to_str())
        .map(std::string::ToString::to_string);

    let name = match binary_name {
        Some(ref n) => n.as_str(),
        None => return Ok(None),
    };

    match name {
        "sjq" | "jq" => {
            let cmd = JqCommand::parse_from(
                std::iter::once(name.to_string()).chain(std::env::args().skip(1)),
            );
            Ok(Some(jq_runner::run_jq(cmd)?))
        }
        "syq" | "yq" => {
            let cmd = YqCommand::parse_from(
                std::iter::once(name.to_string()).chain(std::env::args().skip(1)),
            );
            Ok(Some(yq_runner::run_yq(cmd)?))
        }
        "sjq-locate" | "jq-locate" => {
            let cmd = jq_locate::JqLocateArgs::parse_from(
                std::iter::once(name.to_string()).chain(std::env::args().skip(1)),
            );
            Ok(Some(jq_locate::run_jq_locate(cmd)?))
        }
        "syq-locate" | "yq-locate" => {
            let cmd = yq_locate::YqLocateArgs::parse_from(
                std::iter::once(name.to_string()).chain(std::env::args().skip(1)),
            );
            Ok(Some(yq_locate::run_yq_locate(cmd)?))
        }
        _ => Ok(None),
    }
}

/// Native stack given to the thread every command actually runs on
/// (#1371).
///
/// A user-defined `def` now recurses by real evaluation rather than by
/// pre-substituting its own body, so recursion depth is bounded by native
/// stack instead of by a substitution budget. One live call level costs
/// ~4.4 KB, measured (#2080) by bisecting the deepest working recursion at
/// two stack sizes an 8x multiple apart -- 4406 B/level at 8 MB and 4381
/// B/level at 64 MB, agreeing to within 0.6%, so the cost is linear in depth
/// and this size converts directly into a depth.
///
/// 256 MB carries `MAX_EVAL_FRAMES`'s ~13,000 levels for a `sum_to`-shaped
/// body (~57 MB at this per-level cost) with room for bodies far heavier
/// than that one, on a platform
/// whose *default* main-thread stack is 8 MB and would cap the same
/// recursion below 1,000. It is reserved address space, not committed
/// memory: pages are faulted in only as the stack is actually used, so a
/// filter that never recurses pays nothing for it.
///
/// Applied here, at the single point every subcommand passes through, rather
/// than around `run_jq` alone -- `yq` shares the same evaluator, and the
/// alternative is one wrapper per command that each has to remember.
///
/// **Bigger in a debug build, deliberately.** An unoptimized frame for the
/// same evaluator is ~9x the size of an optimized one -- measured at the same
/// stack, a recursion that survives ~57,500 levels in release dies at ~6,250
/// in debug. A single stack size would therefore mean `MAX_EVAL_FRAMES` had
/// to be calibrated for debug and would then cap release far below what its
/// stack could carry, or be calibrated for release and abort the debug
/// binary -- which is what `cargo test` and CI actually run. Scaling the
/// stack with the profile instead keeps *one* frame ceiling correct for both,
/// so the tests exercise the same limit the shipped binary enforces. Reserved
/// address space either way: pages are faulted in only as used.
const EVAL_STACK_SIZE: usize = if cfg!(debug_assertions) {
    2 * 1024 * 1024 * 1024
} else {
    256 * 1024 * 1024
};

fn main() -> Result<()> {
    // Run everything on a thread with an explicitly sized stack, then
    // propagate its result. A panic in the child is resumed here rather than
    // swallowed, so panic behaviour (and the abort/backtrace a panic produces)
    // is exactly what it was when this ran on the main thread.
    let child = std::thread::Builder::new()
        .stack_size(EVAL_STACK_SIZE)
        .spawn(run_main)
        .context("failed to spawn the evaluation thread")?;
    match child.join() {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn run_main() -> Result<()> {
    // Multi-call binary: check if invoked via a known alias name (e.g., sjq, syq)
    if let Some(exit_code) = try_multicall()? {
        std::process::exit(exit_code);
    }

    let cli = Cli::parse();

    match cli.command {
        Command::Jq(args) => {
            let exit_code = jq_runner::run_jq(args)?;
            std::process::exit(exit_code);
        }
        Command::Yq(args) => {
            let exit_code = yq_runner::run_yq(args)?;
            std::process::exit(exit_code);
        }
        Command::JqLocate(args) => {
            let exit_code = jq_locate::run_jq_locate(args)?;
            std::process::exit(exit_code);
        }
        Command::YqLocate(args) => {
            let exit_code = yq_locate::run_yq_locate(args)?;
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

                let verified_parse = if args.verify {
                    let parsed = validate_generated_json(&json, args.pretty)
                        .context("Generated invalid JSON")?;
                    eprintln!("✓ JSON validated successfully");
                    parsed
                } else {
                    None
                };

                let output = if args.pretty {
                    let value = match verified_parse {
                        Some(v) => v,
                        None => serde_json::from_str(&json)?,
                    };
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
                        println!("{output}");
                    }
                }

                Ok(())
            }
            JsonSubcommand::GenerateSuite(args) => generate_json_suite(args),
            JsonSubcommand::Validate(args) => {
                let exit_code = json_validate::run(args)?;
                std::process::exit(exit_code);
            }
        },
        Command::Dsv(dsv_cmd) => match dsv_cmd.command {
            DsvSubcommand::Generate(args) => {
                let dsv = dsv_generators::generate_dsv(
                    args.size,
                    args.pattern.into(),
                    args.seed,
                    args.delimiter,
                    args.header || !args.no_header,
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
                        print!("{dsv}");
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
                        .map_err(|e| anyhow::anyhow!("Generated invalid YAML: {e}"))?;
                    eprintln!("✓ YAML validated successfully");
                }

                match args.output {
                    Some(path) => {
                        std::fs::write(&path, &yaml)?;
                        eprintln!("✓ Wrote {} bytes to {}", yaml.len(), path.display());
                    }
                    None => {
                        print!("{yaml}");
                    }
                }

                Ok(())
            }
            YamlSubcommand::GenerateSuite(args) => generate_yaml_suite(args),
            YamlSubcommand::Validate(args) => {
                let exit_code = yaml_validate::run(args)?;
                std::process::exit(exit_code);
            }
        },
        Command::Dev(dev_cmd) => match dev_cmd.command {
            DevSubcommand::Bench(bench_cmd) => match bench_cmd.command {
                BenchSubcommand::Jq(args) => run_jq_benchmark(args),
                BenchSubcommand::Yq(args) => run_yq_benchmark(args),
                BenchSubcommand::Dsv(args) => run_dsv_benchmark(args),
                BenchSubcommand::Utf8(args) => run_utf8_benchmark(args),
                BenchSubcommand::CorpusStats(args) => {
                    let exit_code = corpus_stats::run(
                        &args.data_dir,
                        args.markdown.as_deref(),
                        args.output.as_deref(),
                        args.check,
                    )?;
                    std::process::exit(exit_code);
                }
            },
            DevSubcommand::SelectStats(args) => {
                let exit_code = select_stats_report::run(&args.data_dir, args.markdown.as_ref())?;
                std::process::exit(exit_code);
            }
        },
        Command::InstallAliases(args) => install_aliases(args),
        #[cfg(feature = "bench-runner")]
        Command::Bench(bench_cmd) => match bench_cmd.command {
            BenchRunnerSubcommand::List(args) => bench_runner::run_list(args),
            BenchRunnerSubcommand::Run(args) => bench_runner::run_benchmarks(args),
            BenchRunnerSubcommand::Orchestrate(args) => bench_runner::run_orchestrate(args),
            BenchRunnerSubcommand::Sync(args) => bench_runner::run_sync(args),
            BenchRunnerSubcommand::Nodes(args) => bench_runner::run_nodes(args),
            BenchRunnerSubcommand::Report(args) => bench_runner::run_report(args),
        },
        Command::Text(text_cmd) => match text_cmd.command {
            TextSubcommand::Validate(validate_cmd) => match validate_cmd.command {
                TextValidateSubcommand::Utf8(args) => {
                    let exit_code = text_validate::run(args)?;
                    std::process::exit(exit_code);
                }
            },
            TextSubcommand::Generate(args) => {
                let data =
                    text_generators::generate_utf8(args.size, args.pattern.into(), args.seed);

                if args.verify {
                    succinctly::text::utf8::validate_utf8(&data)
                        .map_err(|e| anyhow::anyhow!("Generated invalid UTF-8: {e}"))?;
                    eprintln!("✓ UTF-8 validated successfully");
                }

                match args.output {
                    Some(path) => {
                        std::fs::write(&path, &data)?;
                        eprintln!("✓ Wrote {} bytes to {}", data.len(), path.display());
                    }
                    None => {
                        use std::io::Write;
                        std::io::stdout().write_all(&data)?;
                    }
                }

                Ok(())
            }
            TextSubcommand::GenerateSuite(args) => generate_utf8_suite(args),
        },
    }
}

/// Install short alias symlinks for the multi-call binary.
fn install_aliases(args: InstallAliasesArgs) -> Result<()> {
    let binary = std::env::current_exe().context("Cannot determine binary path")?;
    let binary = std::fs::canonicalize(&binary).context("Cannot resolve binary path")?;

    let dir = match args.dir {
        Some(d) => d,
        None => binary
            .parent()
            .map(std::path::Path::to_path_buf)
            .context("Cannot determine binary directory")?,
    };

    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("Cannot create directory: {}", dir.display()))?;
    }

    for alias in MULTICALL_ALIASES {
        let target = dir.join(alias);

        // Skip if symlink already points to the right binary
        if target.is_symlink() {
            if let Ok(existing) = std::fs::read_link(&target) {
                if existing == binary {
                    eprintln!("  skip {alias} (already exists)");
                    continue;
                }
            }
            // Remove stale symlink
            std::fs::remove_file(&target)
                .with_context(|| format!("Cannot remove existing symlink: {}", target.display()))?;
        } else if target.exists() {
            eprintln!("  skip {alias} (non-symlink file exists)");
            continue;
        }

        #[cfg(unix)]
        std::os::unix::fs::symlink(&binary, &target)
            .with_context(|| format!("Cannot create symlink: {}", target.display()))?;

        #[cfg(not(unix))]
        std::fs::hard_link(&binary, &target)
            .with_context(|| format!("Cannot create hard link: {}", target.display()))?;

        eprintln!("  created {} -> {}", alias, binary.display());
    }

    eprintln!(
        "\nInstalled {} aliases in {}",
        MULTICALL_ALIASES.len(),
        dir.display()
    );
    Ok(())
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
        "wide",
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

    // Parse query types
    let queries: Vec<jq_bench::QueryType> = if args.queries == "all" {
        jq_bench::QueryType::all().to_vec()
    } else {
        args.queries
            .split(',')
            .filter_map(|s| jq_bench::QueryType::from_str(s.trim()))
            .collect()
    };

    if queries.is_empty() {
        anyhow::bail!(
            "No valid query types specified. Available: identity, keys_unsorted, keys_unsorted_length, keys_unsorted_map, keys_unsorted_select, map, select"
        );
    }

    let config = jq_bench::BenchConfig {
        data_dir: args.data_dir,
        patterns,
        sizes,
        queries,
        succinctly_binary: args.binary,
        warmup_runs: args.warmup,
        benchmark_runs: args.runs,
        memory_mode: !args.no_memory,
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

/// Run yq benchmark
fn run_yq_benchmark(args: BenchYqArgs) -> Result<()> {
    let all_patterns = vec![
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

    // Parse query types
    let queries: Vec<yq_bench::QueryType> = if args.queries == "all" {
        yq_bench::QueryType::all().to_vec()
    } else {
        args.queries
            .split(',')
            .filter_map(|s| yq_bench::QueryType::from_str(s.trim()))
            .collect()
    };

    if queries.is_empty() {
        anyhow::bail!(
            "No valid query types specified. Available: identity, first_element, iteration, length"
        );
    }

    let config = yq_bench::BenchConfig {
        data_dir: args.data_dir,
        patterns,
        sizes,
        queries,
        succinctly_binary: args.binary,
        warmup_runs: args.warmup,
        benchmark_runs: args.runs,
        memory_mode: !args.no_memory,
    };

    // Use default output paths if not specified
    let results_dir = PathBuf::from("data/bench/results");
    let default_jsonl = results_dir.join("yq-bench.jsonl");
    let default_md = results_dir.join("yq-bench.md");

    let output_jsonl = args.output.unwrap_or(default_jsonl);
    let output_md = args.markdown.unwrap_or(default_md);

    // Ensure results directory exists
    if let Some(parent) = output_jsonl.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let _results = yq_bench::run_benchmark(
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
        memory_mode: !args.no_memory,
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

fn run_utf8_benchmark(args: BenchUtf8Args) -> Result<()> {
    let all_patterns = vec![
        "ascii",
        "latin",
        "greek_cyrillic",
        "cjk",
        "emoji",
        "mixed",
        "all_lengths",
        "log_file",
        "source_code",
        "json_like",
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

    let config = utf8_bench::BenchConfig {
        data_dir: args.data_dir,
        patterns,
        sizes,
        warmup_runs: args.warmup,
        benchmark_runs: args.runs,
        engine: utf8_bench::Engine::parse(&args.engine)?,
    };

    // Use default output paths if not specified
    let results_dir = PathBuf::from("data/bench/results");
    let default_jsonl = results_dir.join("utf8-bench.jsonl");
    let default_md = results_dir.join("utf8-bench.md");

    let output_jsonl = args.output.unwrap_or(default_jsonl);
    let output_md = args.markdown.unwrap_or(default_md);

    // Ensure results directory exists
    if let Some(parent) = output_jsonl.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let _results = utf8_bench::run_benchmark(
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
    ("pretty", generators::Pattern::Pretty),
    ("wide", generators::Pattern::Wide),
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

/// Validates that self-generated `json` is well-formed, via `serde_json::Value` --
/// shared by `json generate --verify` and `json generate-suite --verify`'s own
/// generator-self-check (#1212, following #1163's identical consolidation for
/// `jq_runner.rs`'s CLI-arg validation). Neither caller wants the parsed tree for its
/// own sake (both are checking "did the generator itself produce valid JSON", not
/// materializing a value) -- but `generate --verify --pretty` needs to immediately
/// re-parse the same string to pretty-print it, so `keep_parsed` lets that one caller
/// reuse this validation pass's own parse instead of paying for a second one.
fn validate_generated_json(json: &str, keep_parsed: bool) -> Result<Option<serde_json::Value>> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    Ok(keep_parsed.then_some(value))
}

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

            let filename = format!("{size_name}.json");
            let path = pattern_dir.join(&filename);

            // Deterministic seed: base_seed + file_index
            let seed = args.seed.wrapping_add(file_index);
            file_index += 1;

            let json = generate_json(*size, *pattern, Some(seed), 5, 0.1);

            if args.verify {
                validate_generated_json(&json, false)
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
        eprintln!("Skipped {skipped_count} files exceeding max size");
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

            let filename = format!("{size_name}.{extension}");
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
        eprintln!("Skipped {skipped_count} files exceeding max size");
    }

    if args.verify {
        eprintln!("All files validated successfully");
    }

    Ok(())
}

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

    // One pass over the registry, in registration order: the seed of every file
    // is `base_seed + file_index`, so counting in a stable order is what keeps
    // previously generated patterns byte-identical when a pattern is appended.
    for (pattern_name, pattern, scale) in yaml_generators::ALL_PATTERNS {
        // Create pattern subdirectory
        let pattern_dir = output_dir.join(pattern_name);
        std::fs::create_dir_all(&pattern_dir)?;

        // Fixed-size patterns are realistic templates: one file at the size the
        // generator stops at, rather than one file per size in the ladder.
        let targets: Vec<(String, usize)> = match scale {
            yaml_generators::PatternScale::Scalable => SUITE_SIZES
                .iter()
                .map(|(size_name, size)| (format!("{size_name}.yaml"), *size))
                .collect(),
            yaml_generators::PatternScale::Fixed => {
                vec![(format!("{pattern_name}.yaml"), 1024 * 1024)]
            }
        };

        for (filename, size) in targets {
            // Skip files that exceed max_size (fixed-size templates always run:
            // they stop at their natural size, which max_size cannot predict)
            if *scale == yaml_generators::PatternScale::Scalable && size > args.max_size {
                skipped_count += 1;
                continue;
            }

            let path = pattern_dir.join(&filename);

            // Deterministic seed: base_seed + file_index
            let seed = args.seed.wrapping_add(file_index);
            file_index += 1;

            let yaml = yaml_generators::generate_yaml(size, *pattern, Some(seed));

            if args.verify {
                if let Err(e) = succinctly::yaml::YamlIndex::build(yaml.as_bytes()) {
                    anyhow::bail!("Generated invalid YAML for {}: {}", path.display(), e);
                }
            }

            std::fs::write(&path, &yaml)?;
            total_bytes += yaml.len();
            file_count += 1;

            let fixed = match scale {
                yaml_generators::PatternScale::Fixed => " [fixed-size]",
                yaml_generators::PatternScale::Scalable => "",
            };
            eprintln!(
                "  {} ({}, seed={}){fixed}",
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
        eprintln!("Skipped {skipped_count} files exceeding max size");
    }

    if args.verify {
        eprintln!("All files validated successfully");
    }

    Ok(())
}

/// UTF-8 patterns for suite generation
const UTF8_SUITE_PATTERNS: &[(&str, text_generators::Utf8Pattern)] = &[
    ("ascii", text_generators::Utf8Pattern::Ascii),
    ("latin", text_generators::Utf8Pattern::Latin),
    (
        "greek_cyrillic",
        text_generators::Utf8Pattern::GreekCyrillic,
    ),
    ("cjk", text_generators::Utf8Pattern::Cjk),
    ("emoji", text_generators::Utf8Pattern::Emoji),
    ("mixed", text_generators::Utf8Pattern::Mixed),
    ("all_lengths", text_generators::Utf8Pattern::AllLengths),
    ("log_file", text_generators::Utf8Pattern::LogFile),
    ("source_code", text_generators::Utf8Pattern::SourceCode),
    ("json_like", text_generators::Utf8Pattern::JsonLike),
    ("pathological", text_generators::Utf8Pattern::Pathological),
];

fn generate_utf8_suite(args: GenerateUtf8Suite) -> Result<()> {
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
        "Generating UTF-8 suite in {} (max size: {})...",
        output_dir.display(),
        format_bytes(args.max_size)
    );

    // Generate patterns at various sizes
    for (pattern_name, pattern) in UTF8_SUITE_PATTERNS {
        // Create pattern subdirectory
        let pattern_dir = output_dir.join(pattern_name);
        std::fs::create_dir_all(&pattern_dir)?;

        for (size_name, size) in SUITE_SIZES {
            // Skip files that exceed max_size
            if *size > args.max_size {
                skipped_count += 1;
                continue;
            }

            let filename = format!("{size_name}.txt");
            let path = pattern_dir.join(&filename);

            // Deterministic seed: base_seed + file_index
            let seed = args.seed.wrapping_add(file_index);
            file_index += 1;

            let data = text_generators::generate_utf8(*size, *pattern, Some(seed));

            if args.verify {
                if let Err(e) = succinctly::text::utf8::validate_utf8(&data) {
                    anyhow::bail!("Generated invalid UTF-8 for {}: {}", path.display(), e);
                }
            }

            std::fs::write(&path, &data)?;
            total_bytes += data.len();
            file_count += 1;

            eprintln!(
                "  {} ({}, seed={})",
                path.display(),
                format_bytes(data.len()),
                seed
            );
        }
    }

    eprintln!(
        "\n✓ Generated {} files ({} total)",
        file_count,
        format_bytes(total_bytes)
    );
    if skipped_count > 0 {
        eprintln!(
            "  (skipped {} files exceeding max size {})",
            skipped_count,
            format_bytes(args.max_size)
        );
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
        format!("{bytes} bytes")
    }
}

#[cfg(feature = "bench-runner")]
mod bench_runner;
mod corpus_stats;
mod dsv_bench;
mod dsv_generators;
mod env_config;
#[cfg(feature = "bench-runner")]
mod exit_status;
mod front_matter;
mod generators;
mod jq_bench;
mod jq_locate;
mod jq_runner;
mod json_validate;
mod m2_gate;
mod output;
mod select_stats_report;
mod text_generators;
mod text_validate;
mod utf8_bench;
mod yaml_generators;
mod yaml_pattern_registry;
mod yaml_validate;
mod yq_bench;
mod yq_locate;
mod yq_runner;
use generators::generate_json;

#[cfg(test)]
mod tests {
    use super::*;

    /// Guardrail for #1150: every `JqCommand`/`YqCommand` flag that can
    /// take more than one value per occurrence (`--arg NAME VALUE`-style
    /// pairs, or `--args`/`--jsonargs`' greedy remainder) must also allow
    /// hyphen-prefixed values, or clap rejects a legitimately negative
    /// number/hyphen-prefixed string/filename before it ever reaches this
    /// crate's own validation -- the exact bug #1150 fixed for the six
    /// (now eight) sites that existed at the time. Introspects the live
    /// clap `Command` definitions rather than re-listing the field names,
    /// so a future multi-value arg that forgets `allow_hyphen_values`
    /// fails this test immediately instead of silently reintroducing the
    /// bug for just that one flag.
    #[test]
    fn test_multi_value_variable_args_allow_hyphen_values_1150() {
        use clap::CommandFactory;

        for cmd in [JqCommand::command(), YqCommand::command()] {
            for arg in cmd.get_arguments() {
                if arg.is_positional() {
                    continue;
                }
                let takes_multiple_per_occurrence = arg
                    .get_num_args()
                    .is_some_and(|range| range.max_values() > 1);
                if takes_multiple_per_occurrence {
                    assert!(
                        arg.is_allow_hyphen_values_set(),
                        "--{} in `{}` accepts multiple values per occurrence but doesn't set \
                         allow_hyphen_values -- see #1150",
                        arg.get_id(),
                        cmd.get_name()
                    );
                }
            }
        }
    }

    /// Sibling guardrail for #1203: `-L` takes exactly one value *per
    /// occurrence* (so #1150's `max_values() > 1` check above doesn't cover
    /// it) but is repeatable across occurrences via `ArgAction::Append`, and
    /// had the identical hyphen-value bug on that different clap shape -- a
    /// value starting with `-` (a legitimately hyphen-prefixed directory
    /// name) was rejected before reaching this crate's own logic.
    /// Introspects live `Command` definitions the same way #1150's
    /// guardrail does, so any future `Append`-action,
    /// single-value-per-occurrence arg is caught immediately.
    ///
    /// Deliberately excludes anything #1150's guardrail above already
    /// covers (`max_values() > 1`), even though every current multi-value
    /// arg happens to also be `Append`-action -- kept the two predicates
    /// non-overlapping so each flag is checked, and reported, by exactly
    /// one of them.
    #[test]
    fn test_append_action_args_allow_hyphen_values_1203() {
        use clap::CommandFactory;

        for cmd in [JqCommand::command(), YqCommand::command()] {
            for arg in cmd.get_arguments() {
                if arg.is_positional() {
                    continue;
                }
                let takes_multiple_per_occurrence = arg
                    .get_num_args()
                    .is_some_and(|range| range.max_values() > 1);
                if matches!(arg.get_action(), clap::ArgAction::Append)
                    && !takes_multiple_per_occurrence
                {
                    assert!(
                        arg.is_allow_hyphen_values_set(),
                        "{} in `{}` is repeatable (ArgAction::Append) but doesn't set \
                         allow_hyphen_values -- see #1203",
                        invocable_flag(arg),
                        cmd.get_name()
                    );
                }
            }
        }
    }

    /// The flag spelling a user would actually type, for a test-failure
    /// message -- `arg.get_id()` is the Rust field name (e.g. `library_path`
    /// for `-L`, which has no `long` at all), not necessarily anything
    /// invocable on the command line.
    fn invocable_flag(arg: &clap::Arg) -> String {
        match (arg.get_short(), arg.get_long()) {
            (Some(s), Some(l)) => format!("-{s}/--{l}"),
            (Some(s), None) => format!("-{s}"),
            (None, Some(l)) => format!("--{l}"),
            (None, None) => format!("(positional?) {}", arg.get_id()),
        }
    }

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

        // Overflow: rejected cleanly instead of wrapping (issue #179)
        assert!(parse_size("99999999999gb").is_err());
        assert!(parse_size(&format!("{}kb", usize::MAX)).is_err());
        assert_eq!(parse_size(&format!("{}b", usize::MAX)).unwrap(), usize::MAX);
    }

    #[test]
    fn test_pattern_arg_wide_maps_to_generator_pattern() {
        assert!(matches!(
            generators::Pattern::from(PatternArg::Wide),
            generators::Pattern::Wide
        ));
    }

    fn bench_jq_args(patterns: &str, sizes: &str, queries: &str, binary: &str) -> BenchJqArgs {
        BenchJqArgs {
            data_dir: PathBuf::from("data/bench/generated"),
            output: None,
            markdown: None,
            patterns: patterns.to_string(),
            sizes: sizes.to_string(),
            queries: queries.to_string(),
            warmup: 0,
            runs: 1,
            binary: PathBuf::from(binary),
            no_memory: true,
        }
    }

    #[test]
    fn test_run_jq_benchmark_rejects_invalid_queries() {
        // Fails during argument parsing, before any jq/succinctly subprocess
        // or file I/O - deterministic regardless of the test environment.
        let args = bench_jq_args(
            "arrays",
            "1kb",
            "not_a_real_query",
            "./target/release/succinctly",
        );
        let err = run_jq_benchmark(args).unwrap_err();
        assert!(
            err.to_string().contains("No valid query types specified"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_run_jq_benchmark_builds_config_for_all_queries() {
        // "all" must resolve to a non-empty query list and reach the
        // jq/binary availability checks - it can't succeed without a real jq
        // and succinctly binary, but a nonexistent binary path still
        // guarantees a clean, specific error rather than a panic.
        let args = bench_jq_args(
            "arrays",
            "1kb",
            "all",
            "/nonexistent/succinctly-coverage-test-binary",
        );
        let err = run_jq_benchmark(args).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("jq not found") || msg.contains("binary not found"),
            "unexpected error: {msg}"
        );
    }
}
