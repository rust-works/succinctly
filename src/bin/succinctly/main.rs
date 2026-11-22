//! Succinctly CLI tool for working with succinct data structures.

// First, so its `eprintln!`/`eprint!` (which ignore a failed write) shadow the standard ones in
// this file and in every module declared below (#7476).
#[macro_use]
mod diag;

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
    /// Run a jq filter through the #5223 path-mode interpreter (dev-only)
    #[command(hide = true)]
    JqPathMode(jq_path_mode::JqPathModeArgs),
    /// Reproducible comparison of jq and succinctly jq (#5409)
    #[cfg(feature = "bench-runner")]
    Compare(CompareCommand),
}

#[cfg(feature = "bench-runner")]
#[derive(Debug, Parser)]
struct CompareCommand {
    #[command(subcommand)]
    command: CompareSubcommand,
}

#[cfg(feature = "bench-runner")]
#[derive(Debug, Subcommand)]
enum CompareSubcommand {
    /// jq vs succinctly jq (pinned reference, interleaved timing, output gate)
    Jq(compare::CompareJqArgs),
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

    /// Query types to benchmark (comma-separated, "all", or "users_mix")
    /// Available: identity (.), keys_unsorted, keys_unsorted_length,
    /// keys_unsorted_map, keys_unsorted_select, map, select; and, for
    /// `--patterns users` only: users_navigate, users_length, users_select,
    /// users_map, users_sort_by, users_interpolate, users_group_by, users_index
    /// ("users_mix" is identity plus all of them)
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

    /// The jq to compare against (a bare name is looked up on PATH). The pinned
    /// jq 1.7.1 is target/compare/jq-1.7.1/bin/jq, built by `dev compare jq --build-jq`
    #[arg(long, default_value = "jq")]
    jq: PathBuf,

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

    /// The yq to compare against (a bare name is looked up on PATH; without this flag a missing
    /// yq runs succinctly alone). The reference is Homebrew yq v4.53.3
    #[arg(long)]
    yq: Option<PathBuf>,
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
    /// Wide flat object like `wide`, but a fraction of keys (per
    /// --escape-density) contain a JSON escape sequence (tests escaped-key
    /// decode cost, #2637)
    WideEscapedKeys,
    /// Newline-delimited JSON: one compact user record per line, no enclosing array
    /// (tests many small documents; not part of `generate-suite`, and `--pretty` has
    /// no single document to indent)
    Ndjson,
    /// Like `ndjson`, with each record introduced by an RS byte (RFC 7464 JSON text
    /// sequences, the input of `jq --seq`)
    JsonSeq,
    /// Like `json-seq`, with one truncated record before the first (a `--seq` stream with a
    /// diagnostic, the shape no clean stream exercises, #6619)
    JsonSeqBadFirst,
    /// Like `json-seq`, with one truncated record after the last (#6619)
    JsonSeqBadLast,
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

/// A boolean flag's value as Go's `strconv.ParseBool` reads it, which is what yq's flags use
/// (#7025): `1`, `t`, `T`, `true`, `TRUE`, `True` and the same for false.
fn parse_go_bool(text: &str) -> Result<bool, String> {
    match text {
        "1" | "t" | "T" | "true" | "TRUE" | "True" => Ok(true),
        "0" | "f" | "F" | "false" | "FALSE" | "False" => Ok(false),
        _ => Err(format!(
            "strconv.ParseBool: parsing {text:?}: invalid syntax"
        )),
    }
}

/// An argv word's bytes as the operating system passed them (#7426). Unix argv is bytes; anywhere
/// else a word is whatever text the platform made of it.
pub(crate) fn os_word_bytes(word: &std::ffi::OsStr) -> std::borrow::Cow<'_, [u8]> {
    #[cfg(unix)]
    {
        std::borrow::Cow::Borrowed(std::os::unix::ffi::OsStrExt::as_bytes(word))
    }
    #[cfg(not(unix))]
    {
        match word.to_string_lossy() {
            std::borrow::Cow::Borrowed(text) => std::borrow::Cow::Borrowed(text.as_bytes()),
            std::borrow::Cow::Owned(text) => std::borrow::Cow::Owned(text.into_bytes()),
        }
    }
}

/// An argv word as jq 1.7.1 reads it (#7426): its `jv_string` replaces each invalid UTF-8
/// sequence with U+FFFD by jq's own rule, so `--arg x $'a\xff'` binds `"a\u{FFFD}"` and the program
/// `$'"\xed\xa0\x80"'` is one U+FFFD, not three. Every text word goes through this: the program,
/// the `--arg`/`--argjson` names and values and the `--args`/`--jsonargs` words. A file name does
/// not: jq opens it by its bytes.
pub(crate) fn jq_word_text(word: &std::ffi::OsStr) -> String {
    match word.to_str() {
        Some(text) => text.to_owned(),
        None => succinctly::text::utf8::substitute_invalid_utf8_jq_style(&os_word_bytes(word)),
    }
}

/// [`jq_word_text`] as a clap value parser: a text option takes any bytes, where clap's own `String`
/// parser refuses a word that is not UTF-8.
fn jq_text() -> impl clap::builder::TypedValueParser<Value = String> {
    clap::builder::TypedValueParser::map(clap::builder::OsStringValueParser::new(), |word| {
        jq_word_text(&word)
    })
}

/// An argv word as yq v4.53.3 reads it (#7426): Go decodes a string a byte at a time, so each byte
/// of an invalid sequence is one U+FFFD (`$'"a\xed\xa0\x80"' | length` is 10 in yq, `a` and three
/// three-byte replacements).
pub(crate) fn yq_word_text(word: std::ffi::OsString) -> String {
    word.into_string().unwrap_or_else(|word| {
        let bytes = succinctly::text::utf8::substitute_invalid_utf8_per_byte(&os_word_bytes(&word));
        // Valid UTF-8 by construction, so this borrows and copies once.
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// `--indent`'s value as jq 1.8 reads it (#7501, upstream jqlang/jq#3195): an optional `+` or `-`
/// and one or more ASCII digits, with nothing before, between or after them. White space, `3x`,
/// `3.0`, `0x3`, `1e1`, an empty value and a second sign are malformed, and so is a number outside
/// -1 to 7 (a value past any integer is outside it); jq exits 2 with `--indent takes a number
/// between -1 and 7` for all of them. Leading zeros are digits (`007` is 7, `-00` is 0); `-1` is a
/// tab. jq 1.7.1 read the value with C's `atoi` instead (#6017): the text up to its first non-digit,
/// `0` when there is none, so it ran every malformed value above.
fn parse_jq_indent(text: &str) -> Result<i8, String> {
    let refused = || "--indent takes a number between -1 and 7".to_string();
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(refused());
    }
    let mut value: i8 = 0;
    for digit in digits.bytes().map(|b| (b - b'0') as i8) {
        // Past 7 the value is out of range whatever follows, so it never overflows.
        value = value * 10 + digit;
        if value > 7 {
            return Err(refused());
        }
    }
    match (negative, value) {
        (false, n) => Ok(n),
        (true, 0 | 1) => Ok(-value),
        (true, _) => Err(refused()),
    }
}

/// Command-line JSON processor (jq-compatible CLI)
#[derive(Debug, Parser)]
#[command(name = "jq")]
#[command(about = "Command-line JSON processor", long_about = None)]
struct JqCommand {
    /// jq filter expression (e.g., ".", ".foo", ".[]")
    /// If not provided, uses "." (identity)
    ///
    /// The word as given (#7426): under `-f` it becomes the program file (`program_file`, #7439),
    /// which jq opens by its bytes, and as the program text it is read by [`jq_word_text`].
    filter: Option<std::ffi::OsString>,

    /// Input files (reads from stdin if none provided)
    /// Words after --args or --jsonargs are positional values, not files.
    ///
    /// Not `trailing_var_arg`: that made every word after the first file a
    /// file, flags included, so `jq . data.json -c` and `jq . data.json
    /// --args a` failed with "Could not open file -c". jq reads an option
    /// wherever it appears (#3447).
    ///
    /// Not `String`: jq opens a file by the bytes of its name, which need not be UTF-8 (#7426).
    files: Vec<std::ffi::OsString>,

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
    /// `ArgAction::Set` value options like `--input-dsv`, whose repeats
    /// were never verified against jq, so the blanket command-level
    /// version would have silently introduced new, unverified behaviour
    /// there. (`-f` is a flag too since #7439, with its own
    /// `overrides_with`: jq's `-f a.jq -f b.jq` runs `a.jq` over the
    /// input file `b.jq`.)
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
    ///
    /// The value is read as jq 1.8 reads it (#7501): `-1` is a tab, an optional sign and digits are
    /// allowed, and anything else is refused with exit 2.
    #[arg(long, value_name = "N", value_parser = clap::builder::TypedValueParser::try_map(clap::builder::OsStringValueParser::new(), |word| parse_jq_indent(&jq_word_text(&word))), allow_hyphen_values = true, overrides_with_all = JQ_OUTPUT_FORMAT_FLAGS)]
    indent: Option<i8>,

    // === Program Input ===
    /// Read the filter from the file the first positional argument names
    ///
    /// A flag, not an option with a value (#7439): jq 1.7.1's `-f` only marks the program as a
    /// file, and the first non-option word, wherever it sits in argv, is that file (`jq -f -r
    /// prog.jq`, `jq -rf prog.jq`, `jq prog.jq in.json -f`). `resolve_positional_args` moves that
    /// word into `program_file`. A repeat is tolerated, as jq tolerates it: `-f a.jq -f b.jq` runs
    /// `a.jq` over the input file `b.jq`.
    #[arg(short = 'f', long, overrides_with = "from_file")]
    from_file: bool,

    /// The program file under `-f`, set by `resolve_positional_args` from the first positional
    /// word (#7439); `None` when there is none, which is jq 1.8.2's usage error.
    #[arg(skip)]
    program_file: Option<PathBuf>,

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
    // A text word that is not UTF-8 is read as jq reads it ([`jq_word_text`], #7426); a FILE keeps
    // its bytes, as jq opens it by them.
    /// Set $name to the string value
    #[arg(long, value_names = ["NAME", "VALUE"], num_args = 2, action = clap::ArgAction::Append, allow_hyphen_values = true, value_parser = jq_text())]
    arg: Vec<String>,

    /// Set $name to the JSON value
    #[arg(long, value_names = ["NAME", "VALUE"], num_args = 2, action = clap::ArgAction::Append, allow_hyphen_values = true, value_parser = jq_text())]
    argjson: Vec<String>,

    /// Set $name to an array of JSON values read from file
    #[arg(long, value_names = ["NAME", "FILE"], num_args = 2, action = clap::ArgAction::Append, allow_hyphen_values = true)]
    slurpfile: Vec<std::ffi::OsString>,

    /// Set $name to the string contents of file
    #[arg(long, value_names = ["NAME", "FILE"], num_args = 2, action = clap::ArgAction::Append, allow_hyphen_values = true)]
    rawfile: Vec<std::ffi::OsString>,

    /// Consume remaining arguments as positional string values
    #[arg(long, num_args = 0.., value_name = "STRINGS", allow_hyphen_values = true, value_parser = jq_text())]
    args: Vec<String>,

    /// Consume remaining arguments as positional JSON values
    #[arg(long, num_args = 0.., value_name = "JSON_VALUES", allow_hyphen_values = true, value_parser = jq_text())]
    jsonargs: Vec<String>,

    /// `--args`/`--jsonargs`' words in argv order, filled by `run_jq`
    /// (#3412). clap's greedy flags above capture every word after them,
    /// the filter and any later `--args`/`--jsonargs` included;
    /// `JqCommand::resolve_positional_args` sorts them back out.
    #[arg(skip)]
    positional_words: Vec<jq_runner::PositionalWord>,

    /// The kinds of `--arg`/`--argjson`/`--slurpfile`/`--rawfile` in argv order, filled by `run_jq` (#5154). clap keeps
    /// each flag's values in a `Vec` of its own, and jq applies the four in the order given, keeping the first value of a
    /// name.
    #[arg(skip)]
    named_order: Vec<jq_runner::NamedFlag>,

    /// Whether `--args`/`--jsonargs` captured anything, so a later input
    /// file is a positional word rather than a file (#3412).
    #[arg(skip)]
    positional_mode: bool,

    // === Modules ===
    /// Prepend directory to module search path
    ///
    /// An empty directory is a search-path element like any other (jq accepts `-L ""`, #6017).
    #[arg(short = 'L', value_name = "DIR", action = clap::ArgAction::Append, allow_hyphen_values = true, value_parser = clap::builder::TypedValueParser::map(clap::builder::OsStringValueParser::new(), PathBuf::from))]
    library_path: Vec<PathBuf>,

    // === Formats ===
    /// Parse input/output as application/json-seq (RFC 7464)
    #[arg(long, overrides_with = "seq")]
    seq: bool,

    /// Parse the input value in streaming fashion: the program reads `tostream` events instead of each document
    #[arg(long, overrides_with = "stream")]
    stream: bool,

    /// Implies --stream, and reports a parse error as an input of the form [message, path] instead of stopping
    #[arg(long, overrides_with = "stream_errors")]
    stream_errors: bool,

    /// Accepted and ignored, as jq does outside Windows. jq 1.7.1 refused the `--binary` its own
    /// help lists; jq 1.8 fixed that (jqlang/jq#3131), and this follows it (#7438).
    #[arg(short = 'b', long, hide = true, overrides_with = "binary")]
    binary: bool,

    // === Exit Status ===
    /// Set exit status based on output (0 if last output != false/null)
    #[arg(short = 'e', long, overrides_with = "exit_status")]
    exit_status: bool,

    /// What an uncaught error does to the run (succinctly extension, not a jq flag). Without
    /// it the exit status is the last input's outcome, as in jq: `continue` exits 5 if any
    /// input failed, `stop` stops at the first failure and exits 5 (#5471)
    #[arg(long, value_name = "MODE", overrides_with = "on_error")]
    on_error: Option<OnError>,

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

    /// jq's `--run-tests [file...]`, which runs jq's own test files and is not supported here
    /// (#7715): `run_jq` refuses it, and the words after it are not read, as jq hands them to its
    /// test runner. Hidden, as jq's own `--help` does not list it.
    #[arg(long, hide = true)]
    run_tests: bool,
}

/// `--on-error` of `succinctly jq` (#5471) and `succinctly yq` (#5507): how an uncaught error
/// affects the run. The exit code is the subcommand's own error code (jq 5, yq 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OnError {
    /// Process every input, reporting every error; the run fails if any of them did.
    #[value(name = "continue")]
    Continue,
    /// Evaluate no input after the first one that failed.
    #[value(name = "stop")]
    Stop,
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
    /// Java properties output (`a.b = c`), as `@props` writes it (#6850)
    #[value(name = "props", alias = "p")]
    Props,
    /// Shell variable assignments (`a_b=c`) (#6850)
    #[value(name = "shell", alias = "s")]
    Shell,
    /// CSV of an array, as `@csv` writes it (#6850)
    #[value(name = "csv", alias = "c")]
    Csv,
    /// TSV of an array, as `@tsv` writes it (#6850)
    #[value(name = "tsv", alias = "t")]
    Tsv,
    /// Lua table literal (#6924)
    #[value(name = "lua", alias = "l")]
    Lua,
    /// XML elements, attributes and text (#6924)
    #[value(name = "xml", alias = "x")]
    Xml,
    /// KYAML, YAML's flow subset with explicit commas (#6924)
    #[value(name = "kyaml", alias = "ky")]
    Kyaml,
    /// TOML tables and arrays of tables (#6924)
    #[value(name = "toml")]
    Toml,
    /// INI, sections and their scalars (#6924)
    #[value(name = "ini", alias = "i")]
    Ini,
    /// Base64 of a string, as `@base64` writes it, with no line break (#6850)
    #[value(name = "base64")]
    Base64,
    /// URI percent-encoding of a string, as `@uri` writes it, with no line break (#6850)
    #[value(name = "uri")]
    Uri,
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
    /// Base64: the whole input decoded to one string, as yq's decoder reads it (#6925)
    #[value(name = "base64")]
    Base64,
    /// URI percent-encoding: the whole input unescaped to one string, `+` a space (#6925)
    #[value(name = "uri")]
    Uri,
    /// Java properties (`a.b = c`), as yq's decoder reads them: each key a path, each value a
    /// string, comments kept (#6925)
    #[value(name = "props", alias = "p", alias = "properties")]
    Props,
    /// Comma-separated values: a record is a mapping of the header's names, a cell a snippet of
    /// YAML (#6925)
    #[value(name = "csv", alias = "c")]
    Csv,
    /// Tab-separated values, read as `csv` is with a tab for its separator (#6925)
    #[value(name = "tsv", alias = "t")]
    Tsv,
    /// INI as go-ini reads it: a mapping per section, the keys before the first section at the
    /// root, every value a string (#6925)
    #[value(name = "ini", alias = "i")]
    Ini,
    /// KYAML, which yq reads with its YAML decoder and prints, by extension, as KYAML (#6925)
    #[value(name = "kyaml", alias = "ky")]
    Kyaml,
    /// XML as yq's decoder reads it over Go's `encoding/xml`: an element a mapping, attributes under
    /// `--xml-attribute-prefix`, text under `--xml-content-name`, every value a string (#6925)
    #[value(name = "xml", alias = "x")]
    Xml,
    /// TOML as yq reads it: a port of go-toml's parser and of yq's decoder over it (#6925)
    #[value(name = "toml")]
    Toml,
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
    ///
    /// Not `trailing_var_arg`: that made every word after the first file a file, flags included,
    /// so `yq '.a' f.yaml -o=json` and `yq '.a = 1' f.yaml -i` failed with "failed to read file:
    /// -o=json". yq reads a flag wherever it appears (#6705, as `jq` does, #3447).
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

    /// Accept jq-only builtins and generic postfix `?` that real yq's lexer
    /// rejects as succinctly extensions. Off by default so `succinctly yq`
    /// matches real yq's syntax surface (#1512, #3378).
    #[arg(long, overrides_with = "jq_extensions")]
    pub jq_extensions: bool,

    /// Input format type [auto, yaml, json, props, csv, tsv, ini, kyaml, xml, toml, base64, uri] (default: auto)
    #[arg(
        short = 'p',
        long = "input-format",
        value_name = "FORMAT",
        default_value = "auto",
        overrides_with = "input_format"
    )]
    pub input_format: InputFormat,

    /// Treat input as text with a `---`-fenced YAML front matter header
    /// (e.g. Markdown). `extract` evaluates the expression against just the
    /// front matter and discards the body; `process` re-emits the
    /// transformed front matter followed by the untouched body.
    ///
    /// `-f` is yq's short spelling (#7025).
    #[arg(
        short = 'f',
        long = "front-matter",
        value_name = "MODE",
        overrides_with = "front_matter"
    )]
    pub front_matter: Option<FrontMatterMode>,

    // === Output Options ===
    /// Output format type [yaml, json, props, shell, csv, tsv, base64, uri, auto] (default: auto,
    /// which is the input's own format: a `.json` file prints JSON, anything else YAML, #6703)
    #[arg(
        short = 'o',
        long,
        value_name = "FORMAT",
        default_value = "auto",
        overrides_with = "output_format"
    )]
    pub output_format: OutputFormat,

    /// Unwrap scalar values, print without quotes. Takes its value after `=` like yq's other
    /// boolean flags; not given, it is true for YAML and properties output and false for the rest
    /// (#7093)
    #[arg(
        short = 'r',
        long = "unwrapScalar",
        overrides_with = "unwrap_scalar",
        value_parser = parse_go_bool,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true"
    )]
    pub unwrap_scalar: Option<bool>,

    /// `--unwrapScalar` given as true, which the output routes read as `-r` (set from
    /// `unwrap_scalar` when the run starts)
    #[arg(skip)]
    pub raw_output: bool,

    /// (deprecated) Output as JSON, same as `-o=json`
    ///
    /// Real yq's own `-j`: a deprecated alias for `--tojson` that prints a
    /// warning to stderr and wins over `-o` in either order. jq's `-j`
    /// (join output) is `succinctly jq`'s, not this (#1731).
    #[arg(short = 'j', long = "tojson", overrides_with = "tojson")]
    pub tojson: bool,

    /// Set by the yq runner when `-o` names an encoder that writes its own line ends (#6850).
    #[arg(skip)]
    pub output_encoder_joins: bool,

    /// Separator between a key and its value for `-o=props` (#6922)
    #[arg(
        long = "properties-separator",
        value_name = "SEPARATOR",
        default_value = " = ",
        overrides_with = "properties_separator"
    )]
    pub properties_separator: String,

    /// Write `a[0]` rather than `a.0` for an array element in `-o=props` (#6922)
    #[arg(
        long = "properties-array-brackets",
        overrides_with = "properties_array_brackets"
    )]
    pub properties_array_brackets: bool,

    /// The field separator of `-p csv` and `-o=csv`, one byte (#6922, #6925)
    #[arg(
        long = "csv-separator",
        value_name = "CHAR",
        default_value = ",",
        overrides_with = "csv_separator"
    )]
    pub csv_separator: String,

    /// Whether `-p csv` reads a cell as a snippet of YAML, as yq's `--csv-auto-parse` (default
    /// true); false reads it as a plain scalar (#6925)
    #[arg(
        long = "csv-auto-parse",
        overrides_with = "csv_auto_parse",
        value_parser = parse_go_bool,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true",
        default_value = "true"
    )]
    pub csv_auto_parse: bool,

    /// Whether `-p tsv` reads a cell as a snippet of YAML, as yq's `--tsv-auto-parse` (default
    /// true) (#6925)
    #[arg(
        long = "tsv-auto-parse",
        overrides_with = "tsv_auto_parse",
        value_parser = parse_go_bool,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true",
        default_value = "true"
    )]
    pub tsv_auto_parse: bool,

    /// Keep the quotes that surround an INI value when `-p ini` reads it, as yq's
    /// `--ini-preserve-quotes` (#6925)
    #[arg(long = "ini-preserve-quotes", overrides_with = "ini_preserve_quotes")]
    pub ini_preserve_quotes: bool,

    /// Text before the table of `-o=lua`; `\n`, `\r`, `\t`, `\\`, `\'` and `\"` are read (#6924)
    #[arg(
        long = "lua-prefix",
        value_name = "PREFIX",
        default_value = "return ",
        overrides_with = "lua_prefix"
    )]
    pub lua_prefix: String,

    /// Text after the table of `-o=lua`, escapes as for `--lua-prefix` (#6924)
    #[arg(
        long = "lua-suffix",
        value_name = "SUFFIX",
        default_value = ";\\n",
        overrides_with = "lua_suffix"
    )]
    pub lua_suffix: String,

    /// Write the top-level keys of `-o=lua` as global assignments (#6924)
    #[arg(long = "lua-globals", overrides_with = "lua_globals")]
    pub lua_globals: bool,

    /// Write a string key that is a Lua name unquoted in `-o=lua` (#6924)
    #[arg(long = "lua-unquoted", overrides_with = "lua_unquoted")]
    pub lua_unquoted: bool,

    /// Keep the namespace prefix of an element and an attribute name when `-p xml` reads them, as
    /// yq's `--xml-keep-namespace` (default true) (#6925)
    #[arg(
        long = "xml-keep-namespace",
        overrides_with = "xml_keep_namespace",
        value_parser = parse_go_bool,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true",
        default_value = "true"
    )]
    pub xml_keep_namespace: bool,

    /// Read `-p xml` with Go's `RawToken` (no end-tag matching, no namespace translation), as yq's
    /// `--xml-raw-token` (default true) (#6925)
    #[arg(
        long = "xml-raw-token",
        overrides_with = "xml_raw_token",
        value_parser = parse_go_bool,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true",
        default_value = "true"
    )]
    pub xml_raw_token: bool,

    /// Skip the directives (`<!DOCTYPE ...>`) when `-p xml` reads (#6925)
    #[arg(
        long = "xml-skip-directives",
        overrides_with = "xml_skip_directives",
        value_parser = parse_go_bool,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true",
        default_value = "false"
    )]
    pub xml_skip_directives: bool,

    /// Skip the processing instructions (`<?xml ...?>`) when `-p xml` reads (#6925)
    #[arg(
        long = "xml-skip-proc-inst",
        overrides_with = "xml_skip_proc_inst",
        value_parser = parse_go_bool,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true",
        default_value = "false"
    )]
    pub xml_skip_proc_inst: bool,

    /// Read `-p xml` as Go's strict decoder does: an unknown entity, an unquoted attribute value
    /// and a mismatched end tag are errors (#6925)
    #[arg(
        long = "xml-strict-mode",
        overrides_with = "xml_strict_mode",
        value_parser = parse_go_bool,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true",
        default_value = "false"
    )]
    pub xml_strict_mode: bool,

    /// Prefix of a key that is an attribute in `-o=xml` (#6924)
    #[arg(
        long = "xml-attribute-prefix",
        value_name = "PREFIX",
        default_value = "+@",
        overrides_with = "xml_attribute_prefix"
    )]
    pub xml_attribute_prefix: String,

    /// Key that holds an element's text in `-o=xml` (#6924)
    #[arg(
        long = "xml-content-name",
        value_name = "NAME",
        default_value = "+content",
        overrides_with = "xml_content_name"
    )]
    pub xml_content_name: String,

    /// Prefix of a key that is a processing instruction in `-o=xml` (#6924)
    #[arg(
        long = "xml-proc-inst-prefix",
        value_name = "PREFIX",
        default_value = "+p_",
        overrides_with = "xml_proc_inst_prefix"
    )]
    pub xml_proc_inst_prefix: String,

    /// Key that holds a directive in `-o=xml` (#6924)
    #[arg(
        long = "xml-directive-name",
        value_name = "NAME",
        default_value = "+directive",
        overrides_with = "xml_directive_name"
    )]
    pub xml_directive_name: String,

    /// Separator between the parts of a variable name for `-o=shell` (#6850)
    #[arg(
        long = "shell-key-separator",
        value_name = "SEPARATOR",
        default_value = "_",
        overrides_with = "shell_key_separator"
    )]
    pub shell_key_separator: String,

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
    #[arg(long = "doc", value_name = "N", overrides_with = "document")]
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
    /// the same way) means 4 instead, matching real yq's own go-yaml
    /// dependency (#1575, #2606). Unlike jq's own `--indent` (capped at 7,
    /// jq's own real limit), real yq accepts any non-negative width for
    /// JSON output -- unbounded above, only clamped for YAML (see
    /// `IndentSpec::for_yaml`) -- so this has no upper-bound range.
    #[arg(
        short = 'I',
        long,
        value_name = "N",
        default_value = "2",
        overrides_with = "indent"
    )]
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
    #[arg(long = "split-exp", value_name = "EXPR", overrides_with = "split_exp")]
    pub split_exp: Option<String>,

    // === Program Input ===
    /// Read filter from file instead of command line
    #[arg(long = "from-file", value_name = "FILE", overrides_with = "from_file")]
    pub from_file: Option<PathBuf>,

    /// Take the filter from this flag, so every positional argument is an input file
    /// (yq's `--expression`, for a filter that looks like a file name, #7025).
    ///
    /// `--from-file` wins over it, in either order, as in yq.
    #[arg(
        long = "expression",
        value_name = "EXPR",
        overrides_with = "expression"
    )]
    pub expression: Option<String>,

    /// Read the `--split-exp` expression from a file (yq's `--split-exp-file`, #7025).
    ///
    /// It wins over `--split-exp`, and an empty file means no split at all, as in yq.
    #[arg(
        long = "split-exp-file",
        value_name = "FILE",
        overrides_with = "split_exp_file"
    )]
    pub split_exp_file: Option<PathBuf>,

    // === Security ===
    /// Refuse `env()`, `strenv()` and `envsubst` (yq's `--security-disable-env-ops`, #7025)
    #[arg(
        long = "security-disable-env-ops",
        overrides_with = "security_disable_env_ops"
    )]
    pub security_disable_env_ops: bool,

    /// Refuse `load()`, `load_str()` and `load_base64()` (yq's `--security-disable-file-ops`,
    /// #7025). Reading the input files, `-i` and `--split-exp` still work, as in yq.
    #[arg(
        long = "security-disable-file-ops",
        overrides_with = "security_disable_file_ops"
    )]
    pub security_disable_file_ops: bool,

    /// Let `system(command)` and `system(command; args)` run a program (yq's
    /// `--security-enable-system-operator`, #7025). Without it the operator is an error.
    #[arg(
        long = "security-enable-system-operator",
        overrides_with = "security_enable_system_operator"
    )]
    pub security_enable_system_operator: bool,

    /// yq's `-c`/`--yaml-compact-seq-indent`: a block sequence under a mapping key is not
    /// indented past the key by a whole step, but by the step less two columns (`-I 2`: the items
    /// sit at the key's column, `-I 4`: two columns in). YAML output only; JSON is unchanged
    /// (#7025). Like every boolean flag of yq's it takes its value after `=` only. Not jq's `-c`
    /// (`--compact-output`): it does not compact anything.
    #[arg(
        short = 'c',
        long = "yaml-compact-seq-indent",
        overrides_with = "yaml_compact_seq_indent",
        value_parser = parse_go_bool,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true",
        default_value = "false"
    )]
    pub yaml_compact_seq_indent: bool,

    /// yq's `--yaml-fix-merge-anchor-to-spec`: read a `<<` by the YAML merge-key spec. An own key
    /// beats a merged one wherever the `<<` is written, and of the sources of `<<: [*a, *b]` the
    /// earlier-listed wins; without the flag yq (and so `succinctly yq`) lets the last write win
    /// (#7025). Like every boolean flag of yq's it takes its value after `=` only.
    #[arg(
        long = "yaml-fix-merge-anchor-to-spec",
        overrides_with = "yaml_fix_merge_anchor_to_spec",
        value_parser = parse_go_bool,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true",
        default_value = "false"
    )]
    pub yaml_fix_merge_anchor_to_spec: bool,

    // === Diagnostics ===
    /// Accepted for yq's `-v`/`--verbose`; the debug log yq writes to stderr is not produced
    /// (#7025). Standard output is unchanged either way.
    #[arg(
        short = 'v',
        long,
        overrides_with = "verbose",
        value_parser = parse_go_bool,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true",
        default_value = "false"
    )]
    pub verbose: bool,

    /// Whether a string literal reads `\(expr)` as an interpolation (yq's
    /// `--string-interpolation`, default true, #7025). `--string-interpolation=false` keeps
    /// `\(` as text, in the program and in an `eval(..)` text alike. Like every boolean flag of
    /// yq's, it takes its value after `=` only (`--string-interpolation false` is a file name).
    #[arg(
        long = "string-interpolation",
        overrides_with = "string_interpolation",
        value_parser = parse_go_bool,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true",
        default_value = "true"
    )]
    pub string_interpolation: bool,

    /// Whether a YAML input's leading blank, comment, `%YAML` and `---` lines are slurped as a
    /// header (yq's `--header-preprocess`, default true, #7025). `--header-preprocess=false`
    /// reads the input as an ordinary stream: the leading comments are the first key's head
    /// comment, an explicit first `---` is read and not echoed, and the break after a leading
    /// `--- text` line is kept. Like every boolean flag of yq's, it takes its value after `=` only.
    #[arg(
        long = "header-preprocess",
        overrides_with = "header_preprocess",
        value_parser = parse_go_bool,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true",
        default_value = "true"
    )]
    pub header_preprocess: bool,

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

    /// What an uncaught error does to the run (succinctly extension, not a yq flag). Without it
    /// yq's rule holds: stop at the first error the program raises, exit 1. `continue` evaluates
    /// every document and file, reports every error and exits 1 if any failed; `stop` is the
    /// default spelled out, and under `--inplace` it also edits no file after a failed one (#5507)
    #[arg(long, value_name = "MODE", overrides_with = "on_error")]
    pub on_error: Option<OnError>,

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
            PatternArg::WideEscapedKeys => Self::WideEscapedKeys,
            PatternArg::Ndjson => Self::Ndjson,
            PatternArg::JsonSeq => Self::JsonSeq,
            PatternArg::JsonSeqBadFirst => Self::JsonSeqBadFirst,
            PatternArg::JsonSeqBadLast => Self::JsonSeqBadLast,
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
/// Whether `token` is one of jq's own "not actually an option" spellings
/// for a `-`-leading FILTER argument (#3389). jq 1.7.1's own CLI parser
/// decides this from the character right after the leading `-`: a letter
/// or another `-` still reads as an option (`-n`, `-c`, `--slurp`, and a
/// genuine unknown flag like `-x`, which must keep erroring rather than
/// silently becoming the filter), anything else (a digit, `.`, `(`, `$`,
/// `[`, `{`, `"`, or a lone `-`) is the start of a filter expression jq
/// goes on to parse (`-1`, `-.[0]`, `-(1)`, `-$__loc__.line`). Confirmed
/// live against jq 1.7.1 that this rule, not `allow_hyphen_values`-style
/// blanket acceptance, is the actual boundary.
///
/// `succinctly yq`'s own FILTER needs no such rule: real yq's cobra parser
/// always treats a `-`-leading argument's remaining bytes as shorthand
/// flags and errors on anything it can't resolve that way (`yq '-1'` fails
/// with "unknown shorthand flag: '1' in -1", confirmed live against yq
/// v4.53.3) -- `succinctly yq` already matches that (rejecting without
/// `--`), so this helper and its callers below are jq-only.
fn looks_like_negative_filter(token: &str) -> bool {
    let mut chars = token.chars();
    if chars.next() != Some('-') {
        return false;
    }
    !matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '-')
}

/// How many words after `flag` the jq option `flag` consumes as its values
/// (`--arg NAME VALUE` is 2, `-L DIR` is 1, `-c` and `-f` are 0), read off clap's own
/// definition so a new option cannot drift from this table. `flag` is one
/// argv word: a `--long`, `--long=value`, or a short bundle (`-nc`, `-nL`).
///
/// A bundle ends at the first short that takes a value: when text follows it
/// in the same word, that text is the value (`-Llib`) and nothing more is
/// consumed; when it is the last character, the next word(s) are. An unknown
/// option consumes nothing, so clap reports it in its own words.
fn jq_option_arity(cmd: &clap::Command, flag: &str) -> usize {
    let takes = |arg: &clap::Arg| arg.get_num_args().map_or(0, |range| range.min_values());
    if let Some(long) = flag.strip_prefix("--") {
        if long.contains('=') {
            return 0;
        }
        return cmd
            .get_arguments()
            .find(|arg| {
                arg.get_long() == Some(long)
                    || arg
                        .get_all_aliases()
                        .is_some_and(|aliases| aliases.contains(&long))
            })
            .map_or(0, takes);
    }
    let shorts: Vec<char> = flag.chars().skip(1).collect();
    for (i, c) in shorts.iter().enumerate() {
        let Some(arg) = cmd.get_arguments().find(|arg| {
            arg.get_short() == Some(*c)
                || arg
                    .get_all_short_aliases()
                    .is_some_and(|aliases| aliases.contains(c))
        }) else {
            return 0;
        };
        let n = takes(arg);
        if n > 0 {
            return if i + 1 < shorts.len() { 0 } else { n };
        }
    }
    0
}

/// The inverse of [`os_word_bytes`], for the part of a word after a letter (`-Llib`'s `lib`).
fn os_word_from_bytes(bytes: &[u8]) -> std::ffi::OsString {
    #[cfg(unix)]
    {
        <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(bytes).to_owned()
    }
    #[cfg(not(unix))]
    {
        String::from_utf8_lossy(bytes).into_owned().into()
    }
}

/// The long options of jq 1.7.1's `main.c`. jq has no `--name=value` syntax, so a word of that
/// shape whose name is one of these is jq's `Unknown option` (#7438); a succinctly-only option
/// (`--on-error=stop`) keeps clap's `=` form, which its documentation uses.
const JQ_LONG_OPTIONS: &[&str] = &[
    "slurp",
    "raw-output",
    "raw-output0",
    "join-output",
    "compact-output",
    "color-output",
    "monochrome-output",
    "ascii-output",
    "unbuffered",
    "sort-keys",
    "raw-input",
    "null-input",
    "from-file",
    "binary",
    "tab",
    "indent",
    "seq",
    "stream",
    "stream-errors",
    "exit-status",
    "args",
    "jsonargs",
    "arg",
    "argjson",
    "rawfile",
    "slurpfile",
    "debug-dump-disasm",
    "debug-trace",
    "help",
    "version",
    "build-configuration",
    "run-tests",
];

/// The mode a positional word was read in: jq's `further_args_are_strings` and
/// `further_args_are_json`, set by the latest `--args` or `--jsonargs` before the word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WordMode {
    File,
    Str,
    Json,
}

/// `succinctly jq`'s argv as jq's own option loop reads it (#7438, #7494).
#[derive(Debug, PartialEq, Eq)]
struct JqArgv {
    /// The program name, then every option with the words it takes, one option per word and in
    /// argv order (`-nL lib` is `-n`, `-L`, `lib`): what clap parses. `--args`, `--jsonargs` and
    /// `--` are not here; their effect is in `positional`.
    options: Vec<std::ffi::OsString>,
    /// Every positional word with the mode it was read in. The first is the program, or the
    /// program file under `-f`, whatever its mode.
    positional: Vec<(WordMode, std::ffi::OsString)>,
    /// Where jq's option loop stops before the end of argv, if it does (#7712).
    stop: Option<JqStop>,
}

/// A word jq's option loop stops at (#7712): jq reads argv left to right and exits at the first
/// word it refuses, or at the first help, version or build-configuration option, so everything
/// it checks on the way (an `--argjson` text, a `--slurpfile` file) is checked before it.
#[derive(Debug, PartialEq, Eq)]
struct JqStop {
    /// How many of [`JqArgv::options`] come before the option that stops the loop: the words jq
    /// read before it.
    at: usize,
    kind: JqStopKind,
}

#[derive(Debug, PartialEq, Eq)]
enum JqStopKind {
    /// An option jq does not define.
    Unknown(Vec<u8>),
    /// `-h`, `-V` or `--build-configuration`, which clap prints, or an `--indent` jq refuses or a
    /// succinctly-only option without its value, which clap reports.
    Clap,
    /// A jq option whose values argv does not hold (`--arg x` at the end), with jq's message.
    MissingValue(&'static str),
    /// An `--argjson` text or a `--jsonargs` word that is not JSON; the flag is named.
    InvalidJson(&'static str),
}

/// The word jq names in `jq: Unknown option <word>`.
#[derive(Debug, PartialEq, Eq)]
struct UnknownJqOption(Vec<u8>);

/// [`read_jq_argv`] with an unknown option as the error, for the tests that only need to know
/// what clap is given.
#[cfg(test)]
fn walk_jq_argv<W: AsRef<std::ffi::OsStr>>(
    cmd: &clap::Command,
    argv: &[W],
) -> Result<JqArgv, UnknownJqOption> {
    let walked = read_jq_argv(cmd, argv);
    match walked.stop {
        Some(JqStop {
            kind: JqStopKind::Unknown(word),
            ..
        }) => Err(UnknownJqOption(word)),
        _ => Ok(walked),
    }
}

/// Read `argv` (program name first) as jq 1.8.2's `main.c` does, against `cmd`, the built jq
/// definition (#7438, #7494).
///
/// A word is an option when it is `-` followed by a letter or another `-` (jq's `isoptish`;
/// [`looks_like_negative_filter`] is the complement), until a `--`. Any other word is positional
/// wherever it sits: `jq -1 -n` runs the program `-1` with `-n`, and `jq . a.json -1 -c` reads the
/// file `-1` and still applies `-c`. The first positional word is the program; each later one is a
/// file, or an `--args`/`--jsonargs` word when one of those came before it.
///
/// A long option must be a name clap defines, exactly: `--indent=3`, `--args=` and `--from-file=p`
/// are jq's `Unknown option`. A bundle is read a letter at a time, in order (jq 1.8's in-order
/// parser, jqlang/jq#3194): a letter may repeat (`-nn`), `-L` may end a bundle and takes the rest of
/// the word or the next word (`-nLlib`, `-nL lib`), and the first letter jq does not define is the
/// one named (`-fprog.jq` is `Unknown option -p`). A help, version or build-configuration option
/// ends the walk, as jq exits there: `-V1` and `-Vx` print the version.
///
/// Values are kept with their option by arity ([`jq_option_arity`]'s rule), whatever they look like:
/// `--arg x -c` binds `-c`, `-L --` adds the directory `--`.
///
/// The walk stops where jq's loop stops (#7712, [`JqStop`]): at an unknown option, at a help,
/// version or build-configuration option, at an option whose values argv does not hold, at an
/// `--indent` value jq refuses, and at an `--argjson` text or a `--jsonargs` word that is not JSON.
/// An `--argjson` whose name an earlier named argument bound is skipped unread, as jq skips it.
/// A `--slurpfile` or `--rawfile` file is not read here: [`parse_jq`] reads the ones before a stop.
fn read_jq_argv<W: AsRef<std::ffi::OsStr>>(cmd: &clap::Command, argv: &[W]) -> JqArgv {
    let takes = |arg: &clap::Arg| arg.get_num_args().map_or(0, |range| range.min_values());
    let ends_the_walk = |arg: &clap::Arg| {
        matches!(
            arg.get_id().as_str(),
            "help" | "version" | "build_configuration" | "run_tests"
        )
    };
    let long_arg = |name: &str| {
        cmd.get_arguments().find(|arg| {
            arg.get_long() == Some(name)
                || arg
                    .get_all_aliases()
                    .is_some_and(|aliases| aliases.contains(&name))
        })
    };
    let short_arg = |letter: u8| {
        let letter = char::from(letter);
        cmd.get_arguments().find(|arg| {
            letter.is_ascii_alphabetic()
                && (arg.get_short() == Some(letter)
                    || arg
                        .get_all_short_aliases()
                        .is_some_and(|aliases| aliases.contains(&letter)))
        })
    };

    let mut walked = JqArgv {
        options: argv
            .first()
            .map(|name| name.as_ref().to_owned())
            .into_iter()
            .collect(),
        positional: Vec::new(),
        stop: None,
    };
    // The names `--arg`, `--argjson`, `--slurpfile` and `--rawfile` have bound: jq reads the value
    // of a name only the first time (#5154).
    let mut bound: std::collections::HashSet<String> = std::collections::HashSet::new();
    // The values an option takes follow it, whatever they look like. `arg` is the option, `at`
    // where it starts in `options`; a value jq refuses there stops the walk.
    let take_values = |walked: &mut JqArgv,
                       bound: &mut std::collections::HashSet<String>,
                       arg: &clap::Arg,
                       at: usize,
                       i: &mut usize|
     -> Option<JqStop> {
        let n = takes(arg);
        let end = (*i + n).min(argv.len());
        let values = &argv[*i..end];
        walked
            .options
            .extend(values.iter().map(|w| w.as_ref().to_owned()));
        *i = end;
        let stop = |kind| Some(JqStop { at, kind });
        if values.len() < n {
            return stop(
                jq_missing_value_message(arg.get_id().as_str())
                    .map_or(JqStopKind::Clap, JqStopKind::MissingValue),
            );
        }
        let text = |k: usize| jq_word_text(values[k].as_ref());
        match arg.get_id().as_str() {
            "indent" if parse_jq_indent(&text(0)).is_err() => stop(JqStopKind::Clap),
            // succinctly's own `--on-error`, whose value clap checks: refused where it stands too.
            "on_error" if <OnError as clap::ValueEnum>::from_str(&text(0), false).is_err() => {
                stop(JqStopKind::Clap)
            }
            id @ ("arg" | "argjson" | "slurpfile" | "rawfile") => {
                let name = text(0);
                if bound.contains(&name) {
                    return None;
                }
                if id == "argjson" && !jq_runner::is_json_text(&text(1)) {
                    return stop(JqStopKind::InvalidJson("argjson"));
                }
                bound.insert(name);
                None
            }
            _ => None,
        }
    };
    let mut mode = WordMode::File;
    let mut options_ended = false;
    let mut i = 1;
    while i < argv.len() {
        let word = argv[i].as_ref();
        let bytes = os_word_bytes(word);
        i += 1;
        let is_option = bytes.len() > 1
            && bytes[0] == b'-'
            && (bytes[1] == b'-' || bytes[1].is_ascii_alphabetic());
        if options_ended || !is_option {
            // The first positional word is the program, whatever the mode.
            if mode == WordMode::Json
                && !walked.positional.is_empty()
                && !jq_runner::is_json_text(&jq_word_text(word))
            {
                walked.stop = Some(JqStop {
                    at: walked.options.len(),
                    kind: JqStopKind::InvalidJson("jsonargs"),
                });
                return walked;
            }
            walked.positional.push((mode, word.to_owned()));
            continue;
        }
        if *bytes == *b"--" {
            options_ended = true;
            continue;
        }
        let at = walked.options.len();
        if bytes[1] == b'-' {
            let name = std::str::from_utf8(&bytes[2..]).ok();
            let Some(arg) = name.and_then(long_arg) else {
                // `--on-error=stop`: a succinctly-only option keeps clap's `=` form.
                let extension =
                    name.and_then(|name| name.split_once('='))
                        .is_some_and(|(name, _)| {
                            !JQ_LONG_OPTIONS.contains(&name) && long_arg(name).is_some()
                        });
                if extension {
                    walked.options.push(word.to_owned());
                    continue;
                }
                walked.stop = Some(JqStop {
                    at,
                    kind: JqStopKind::Unknown(bytes.into_owned()),
                });
                return walked;
            };
            match arg.get_id().as_str() {
                "args" => mode = WordMode::Str,
                "jsonargs" => mode = WordMode::Json,
                _ => {
                    walked.options.push(word.to_owned());
                    if ends_the_walk(arg) {
                        walked.stop = Some(JqStop {
                            at,
                            kind: JqStopKind::Clap,
                        });
                        return walked;
                    }
                    if let Some(stop) = take_values(&mut walked, &mut bound, arg, at, &mut i) {
                        walked.stop = Some(stop);
                        return walked;
                    }
                }
            }
            continue;
        }
        for (k, &letter) in bytes.iter().enumerate().skip(1) {
            let at = walked.options.len();
            let Some(arg) = short_arg(letter) else {
                walked.stop = Some(JqStop {
                    at,
                    kind: JqStopKind::Unknown(vec![b'-', letter]),
                });
                return walked;
            };
            walked
                .options
                .push(format!("-{}", char::from(letter)).into());
            if ends_the_walk(arg) {
                walked.stop = Some(JqStop {
                    at,
                    kind: JqStopKind::Clap,
                });
                return walked;
            }
            if takes(arg) > 0 {
                // `-Llib`: the rest of the word is the value; `-nL lib`: the next word is.
                if k + 1 < bytes.len() {
                    walked.options.push(os_word_from_bytes(&bytes[k + 1..]));
                } else if let Some(stop) = take_values(&mut walked, &mut bound, arg, at, &mut i) {
                    walked.stop = Some(stop);
                    return walked;
                }
                break;
            }
        }
    }
    walked
}

/// jq's refusal of an option it does not define (#7438): `jq: Unknown option <word>` and its usage
/// hint on standard error, exit 2. The word is printed as given, as jq's `fprintf` prints it.
fn exit_unknown_jq_option(UnknownJqOption(word): UnknownJqOption) -> ! {
    let mut line = b"jq: Unknown option ".to_vec();
    line.extend_from_slice(&word);
    exit_jq_usage_error(&line)
}

/// One line of jq's usage error, then the usage hint jq's `die()` prints, on standard error, and
/// exit 2.
fn exit_jq_usage_error(line: &[u8]) -> ! {
    use std::io::Write;
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(line);
    let _ = stderr.write_all(
        b"\nUse jq --help for help with command-line options,\n\
          or see the jq manpage, or online docs  at https://jqlang.github.io/jq\n",
    );
    let _ = stderr.flush();
    std::process::exit(output::exit_codes::USAGE_ERROR)
}

/// jq's report of an option at the end of argv without the values it takes (#7712), for the jq
/// options that take values; `None` for a succinctly-only option, which clap reports.
fn jq_missing_value_message(id: &str) -> Option<&'static str> {
    Some(match id {
        "arg" => "jq: --arg takes two parameters (e.g. --arg varname value)",
        "argjson" => "jq: --argjson takes two parameters (e.g. --argjson varname text)",
        "slurpfile" => "jq: --slurpfile takes two parameters (e.g. --slurpfile varname filename)",
        "rawfile" => "jq: --rawfile takes two parameters (e.g. --rawfile varname filename)",
        "indent" => "jq: --indent takes one parameter",
        "library_path" => "-L takes a parameter: (e.g. -L /search/path or -L/search/path)",
        _ => return None,
    })
}

/// Returns `Some(exit_code)` if a multi-call alias was detected,
/// or `None` to fall through to normal `Cli::parse()`.
///
/// argv is read as the operating system's words (`args_os`), never `std::env::args`, which panics
/// on the first one that is not UTF-8 (#7426).
fn try_multicall() -> Result<Option<i32>> {
    let binary_name = std::env::args_os()
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
            let argv: Vec<std::ffi::OsString> = std::iter::once(name.into())
                .chain(std::env::args_os().skip(1))
                .collect();
            Ok(Some(jq_runner::run_jq(parse_jq(argv, None))?))
        }
        "syq" | "yq" => {
            let argv: Vec<std::ffi::OsString> = std::iter::once(name.into())
                .chain(std::env::args_os().skip(1))
                .collect();
            Ok(Some(yq_runner::run_yq(parse_yq(argv, None))?))
        }
        "sjq-locate" | "jq-locate" => {
            let cmd = jq_locate::JqLocateArgs::parse_from(
                std::iter::once(name.into()).chain(std::env::args_os().skip(1)),
            );
            Ok(Some(jq_locate::run_jq_locate(cmd)?))
        }
        "syq-locate" | "yq-locate" => {
            let cmd = yq_locate::YqLocateArgs::parse_from(
                std::iter::once(std::ffi::OsString::from(name)).chain(std::env::args_os().skip(1)),
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
///
/// **Registered with the evaluator (ADR-0025, #3262).** `main` arms
/// `succinctly::jq::with_stack_budget` with this size, so a recursion of any
/// shape refuses once it has used half of it -- the guarantee the frame count
/// above could only approximate, since the stack a level costs ranges from
/// ~1.4 KB to ~1 MB across ordinary programs. Changing this size therefore
/// moves where deep recursion refuses, in proportion.
const EVAL_STACK_SIZE: usize = if cfg!(debug_assertions) {
    2 * 1024 * 1024 * 1024
} else {
    256 * 1024 * 1024
};

/// What to say, and where to go, when a binary built with the `popcnt` target feature (the x86_64 release
/// archives, #5035) starts on a CPU without the instruction.
#[cfg(any(test, all(target_arch = "x86_64", target_feature = "popcnt")))]
fn popcnt_required_message(program: &str) -> String {
    format!(
        "{program}: this build needs an x86-64 CPU with the POPCNT instruction (Intel Nehalem, AMD K10 \
         or newer), and this CPU does not report it.\n\
         Build a baseline binary instead: cargo install succinctly --features cli"
    )
}

/// Whether CPUID leaf 1's `ecx` reports `POPCNT` (bit 23).
#[cfg(any(test, all(target_arch = "x86_64", target_feature = "popcnt")))]
fn cpuid_ecx_reports_popcnt(ecx: u32) -> bool {
    ecx & (1 << 23) != 0
}

/// Refuse to run, with a message that names the requirement, rather than die on `Illegal instruction`.
///
/// Asks CPUID itself: `is_x86_feature_detected!("popcnt")` expands to `cfg!(target_feature = "popcnt") || <runtime
/// check>`, which is `true` by construction in the only build that compiles this function. `main` and the runtime
/// start-up code carry no `POPCNT` of their own (checked in the release binary: the instruction sits in the query
/// code, not in `main`, `lang_start` or `_start`), and the standard library is prebuilt for the baseline target.
#[cfg(all(target_arch = "x86_64", target_feature = "popcnt"))]
fn require_popcnt() {
    let leaf1 = core::arch::x86_64::__cpuid(1);
    if !cpuid_ecx_reports_popcnt(leaf1.ecx) {
        eprintln!("{}", popcnt_required_message("succinctly"));
        std::process::exit(1);
    }
}

/// The least stack the evaluation thread settles for when the address space is capped (#5634).
///
/// Half of it is the native-stack floor's reach (ADR-0025): 8 MiB still holds the walks the floor
/// does not charge for, a built value at `MAX_VALUE_TREE_DEPTH` and a native `recurse`'s own
/// 512 KiB budget, with room to spare. In `succinctly jq` (#5554) and `succinctly yq` (#5876) a walk
/// over a built value past 384 levels is charged to the floor itself, so a smaller stack refuses it
/// shallower; the recursive `Drop` of what it built is the walk left uncharged (#5782).
const MIN_EVAL_STACK_SIZE: usize = 16 * 1024 * 1024;

const _: () = assert!(EVAL_STACK_SIZE >= MIN_EVAL_STACK_SIZE);

/// The stack sizes [`spawn_eval_thread`] tries, largest first: `start`, then each half of the last
/// while it stays at or above `min`. `start` itself is always tried.
fn eval_stack_sizes(start: usize, min: usize) -> impl Iterator<Item = usize> {
    std::iter::successors(Some(start), move |size| {
        Some(size / 2).filter(|half| *half >= min)
    })
}

/// The stack to try first under a cap of `cap` bytes of address space: a quarter of it, so the
/// document and its index keep the rest, within [`MIN_EVAL_STACK_SIZE`] and `max`. No cap is `max`.
fn eval_stack_start(max: usize, cap: Option<usize>) -> usize {
    match cap {
        Some(cap) => (cap / 4).clamp(MIN_EVAL_STACK_SIZE.min(max), max),
        None => max,
    }
}

/// Spawn the evaluation thread on [`EVAL_STACK_SIZE`], or on a smaller stack when the address
/// space is capped (#5634).
///
/// The stack is address space, not memory, but `ulimit -v` (`RLIMIT_AS`) counts it: under a cap
/// below the stack size `pthread_create` fails `EAGAIN`, and every run died with `failed to spawn
/// the evaluation thread` however small its input -- where jq reads the same document in the same
/// cap. A cap also has to hold the document, so the stack starts at a quarter of it
/// ([`eval_stack_start`]) and halves from there if the thread still cannot be mapped. Each size
/// registers its own native-stack floor (ADR-0025), which sits at half of what the thread really
/// has, so a smaller stack moves where deep recursion is refused, in proportion, and never to an
/// overflow.
fn spawn_eval_thread() -> Result<std::thread::JoinHandle<Result<()>>> {
    let mut last_error = None;
    let start = eval_stack_start(EVAL_STACK_SIZE, address_space::cap());
    for size in eval_stack_sizes(start, MIN_EVAL_STACK_SIZE) {
        let spawned = std::thread::Builder::new()
            .stack_size(size)
            .spawn(move || succinctly::jq::with_stack_budget(size, run_main));
        match spawned {
            Ok(child) => return Ok(child),
            Err(e) => last_error = Some(e),
        }
    }
    Err(last_error.expect("`start` is always tried"))
        .context("failed to spawn the evaluation thread")
}

pub fn main() -> Result<()> {
    #[cfg(all(target_arch = "x86_64", target_feature = "popcnt"))]
    require_popcnt();
    // #7412: the loader is done with the relocation table; its pages need not stay in the RSS.
    relocation_pages::release();
    malloc_arena::use_one();
    // Run everything on a thread with an explicitly sized stack, then
    // propagate its result. A panic in the child is resumed here rather than
    // swallowed, so panic behaviour (and the abort/backtrace a panic produces)
    // is exactly what it was when this ran on the main thread.
    //
    // ADR-0025: the thread's stack is registered with the evaluator, so a
    // recursion of any shape refuses once it has spent half of it instead of
    // overflowing -- registered first thing, where the stack still available
    // and the thread's size agree.
    let child = match spawn_eval_thread() {
        Ok(child) => child,
        Err(failed) => return run_main_on_this_thread(failed),
    };
    match child.join() {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// Run on the main thread when no evaluation thread can be created (#7733): at the user's process
/// or thread limit (`ulimit -u`, a container's pids limit) every size fails `EAGAIN`, where jq,
/// which is single-threaded, runs.
///
/// The floor is registered at the stack this thread really has ([`main_thread_stack::available`]),
/// so deep recursion is refused, sooner than on the evaluation thread, instead of overflowing; under
/// a cap on the address space no more than a quarter of the cap is counted, as
/// [`eval_stack_start`] does. Where the stack's bounds cannot be read the spawn failure stands.
fn run_main_on_this_thread(failed: anyhow::Error) -> Result<()> {
    let Some(stack) = main_thread_stack::available() else {
        return Err(failed);
    };
    let stack = match address_space::cap() {
        Some(cap) => stack.min(cap / 4),
        None => stack,
    }
    .min(EVAL_STACK_SIZE);
    succinctly::jq::with_stack_budget(stack, run_main)
}

/// Exit after a `jq`/`yq` run.
///
/// With the `share-stats` feature and `SUCCINCTLY_SHARE_STATS` set, first
/// print #2999's copy-on-write audit -- every shared container the run copied
/// on write or by value, with the call site that forced it -- to stderr.
/// Nothing is printed otherwise, and the default build has no counters at
/// all. See `succinctly::jq::share_stats`.
fn exit_after_run(exit_code: i32) -> ! {
    #[cfg(feature = "share-stats")]
    if std::env::var_os("SUCCINCTLY_SHARE_STATS").is_some() {
        let report = succinctly::jq::share_stats::report();
        if report.is_empty() {
            eprintln!("share-stats: no forced copies");
        } else {
            eprint!("share-stats: forced copies (count  kind  site)\n{report}");
        }
    }
    std::process::exit(exit_code)
}

/// What `succinctly jq --help` and `succinctly yq --help` say about themselves: the doc comments of
/// [`Command::Jq`] and [`Command::Yq`], which the direct path of [`try_direct_subcommand`] has to repeat
/// (`direct_subcommands_render_the_trees_help_7149` pins the rendered help to the tree's).
const JQ_SUBCOMMAND_ABOUT: &str = "Command-line JSON processor (jq-compatible)";
const YQ_SUBCOMMAND_ABOUT: &str = "Command-line YAML processor (jq-compatible syntax)";

/// `argv` (program name first) parsed as `jq`, for the `sjq` multi-call path (`about` is `None`: its
/// own text) and for `succinctly jq` (`about` is the one the tree shows, #7149).
///
/// jq's grammar is not clap's (#7438, #7494): [`read_jq_argv`] reads argv as jq does, refusing what
/// jq refuses, and clap then parses only the options it found, so no positional word (`-1`, `-.a`)
/// ever reaches clap. One definition serves both, built once.
fn parse_jq(argv: Vec<std::ffi::OsString>, about: Option<&'static str>) -> JqCommand {
    use clap::{CommandFactory, FromArgMatches};
    let mut command = JqCommand::command();
    if let Some(about) = about {
        command = command.about(about);
    }
    // `read_jq_argv` reads each option's arity, which is only filled in once the command is built.
    command.build();
    let walked = read_jq_argv(&command, &argv);
    if let Some(stop) = &walked.stop {
        // jq checks every option before the one it stops at first (#7712).
        report_jq_errors_before(&mut command, &walked.options[..stop.at], &walked.positional);
        match &stop.kind {
            JqStopKind::Unknown(word) => exit_unknown_jq_option(UnknownJqOption(word.clone())),
            JqStopKind::InvalidJson(flag) => {
                jq_runner::exit_invalid_json_text(flag);
            }
            JqStopKind::MissingValue(message) => exit_jq_usage_error(message.as_bytes()),
            // clap prints the help and reports the `--indent` it refuses.
            JqStopKind::Clap => {}
        }
    }
    let matches = command
        .try_get_matches_from_mut(walked.options)
        .unwrap_or_else(|e| e.exit());
    let mut cmd = JqCommand::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    cmd.named_order = jq_runner::named_flag_order_of(&matches);
    cmd.set_positional_words(walked.positional);
    cmd
}

/// Report the first error jq would find in the words before the one its option loop stops at
/// (#7712), and exit with jq's status for it; return when there is none. `options` are the options
/// [`read_jq_argv`] read before the stop and `positional` the positional words.
///
/// clap parses `options` first, so a bad value of a succinctly-only option there is reported in
/// clap's words. The `--slurpfile` and `--rawfile` files are then read and every named argument
/// bound, as `run_jq` does: the walk has already checked each `--argjson` text and `--jsonargs` word.
/// With nothing before the stop but the program name there is nothing to check.
fn report_jq_errors_before(
    command: &mut clap::Command,
    options: &[std::ffi::OsString],
    positional: &[(WordMode, std::ffi::OsString)],
) {
    use clap::FromArgMatches;
    if options.len() <= 1 {
        return;
    }
    let matches = command
        .try_get_matches_from_mut(options)
        .unwrap_or_else(|e| e.exit());
    let mut cmd = JqCommand::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    cmd.named_order = jq_runner::named_flag_order_of(&matches);
    cmd.set_positional_words(positional.to_vec());
    if let Some(code) = jq_runner::report_named_argument_error(&cmd) {
        std::process::exit(code);
    }
}

impl JqCommand {
    /// Take the positional words [`read_jq_argv`] read (#7494): the first is the program (under
    /// `-f`, `resolve_positional_args` moves it to `program_file`), and each later one goes where
    /// the mode it was read in sends it: an input file, or an `--args`/`--jsonargs` word.
    fn set_positional_words(&mut self, words: Vec<(WordMode, std::ffi::OsString)>) {
        let mut words = words.into_iter();
        // The program and the files keep their bytes; an `--args`/`--jsonargs` word is text, read
        // as jq reads it (#7426).
        self.filter = words.next().map(|(_, word)| word);
        for (mode, word) in words {
            match mode {
                WordMode::File => self.files.push(word),
                WordMode::Str => self
                    .positional_words
                    .push(jq_runner::PositionalWord::Str(jq_word_text(&word))),
                WordMode::Json => self
                    .positional_words
                    .push(jq_runner::PositionalWord::Json(jq_word_text(&word))),
            }
        }
    }
}

/// `argv` (program name first) parsed as `yq`; see [`parse_jq`] for `about`.
///
/// A word that is not UTF-8 is read as yq reads it ([`yq_word_text`], #7426): the program and
/// every value as text, a file name included, where yq opens a file by its bytes.
fn parse_yq(argv: Vec<std::ffi::OsString>, about: Option<&'static str>) -> YqCommand {
    use clap::{CommandFactory, FromArgMatches};
    let argv: Vec<String> = argv.into_iter().map(yq_word_text).collect();
    let mut command = YqCommand::command();
    if let Some(about) = about {
        command = command.about(about);
    }
    let matches = command.get_matches_from(argv);
    YqCommand::from_arg_matches(&matches).unwrap_or_else(|e| e.exit())
}

/// `succinctly jq ...` and `succinctly yq ...` parse their own struct, as the `sjq`/`syq` multi-call
/// path does, instead of the whole [`Cli`] tree (#7149): clap's derive builds the argument definitions
/// of every subcommand before it matches one, which was 30% of the instructions of `jq -n 1` and
/// 8% more to drop the tree at exit. The binary name is `succinctly jq` (`succinctly yq`), so the
/// usage line and every clap error read as they did when the subcommand was matched in the tree.
/// Everything else, a bare `succinctly`, a global flag before the subcommand and every other
/// subcommand, goes through [`Cli`] unchanged.
fn try_direct_subcommand() -> Result<Option<i32>> {
    // `args_os`, as in [`try_multicall`] (#7426).
    let mut args = std::env::args_os();
    let argv0 = args.next().unwrap_or_default();
    // clap names the binary by its argv[0]'s file name, so `<argv0> jq` reads as `jq` does in the tree.
    let argv = |subcommand: &str| -> Vec<std::ffi::OsString> {
        let mut name = argv0.clone();
        name.push(format!(" {subcommand}"));
        std::iter::once(name)
            .chain(std::env::args_os().skip(2))
            .collect()
    };
    match args.next().as_ref().and_then(|word| word.to_str()) {
        Some("jq") => Ok(Some(jq_runner::run_jq(parse_jq(
            argv("jq"),
            Some(JQ_SUBCOMMAND_ABOUT),
        ))?)),
        Some("yq") => Ok(Some(yq_runner::run_yq(parse_yq(
            argv("yq"),
            Some(YQ_SUBCOMMAND_ABOUT),
        ))?)),
        _ => Ok(None),
    }
}

fn run_main() -> Result<()> {
    // #6560: `$ENV` is read once, as jq reads it, not built again for every record.
    succinctly::jq::snapshot_environment();
    // Multi-call binary: check if invoked via a known alias name (e.g., sjq, syq)
    if let Some(exit_code) = try_multicall()? {
        exit_after_run(exit_code);
    }
    if let Some(exit_code) = try_direct_subcommand()? {
        exit_after_run(exit_code);
    }

    let cli = Cli::parse();

    match cli.command {
        Command::Jq(args) => {
            let exit_code = jq_runner::run_jq(args)?;
            exit_after_run(exit_code);
        }
        Command::Yq(args) => {
            let exit_code = yq_runner::run_yq(args)?;
            exit_after_run(exit_code); // patchcov: coverage tolerate-line reason="the CLI suites drive yq through the `syq` multi-call arm above; this arm is the same call reached only when spelled `succinctly yq` (#2999)"
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
                let pattern: generators::Pattern = args.pattern.into();
                if pattern.is_line_delimited() && args.pretty {
                    anyhow::bail!("--pretty needs a single document; the ndjson and json-seq patterns (json-seq-bad-first, json-seq-bad-last) have one record per line");
                }
                let json = generate_json(
                    args.size,
                    pattern,
                    args.seed,
                    args.depth,
                    args.escape_density,
                );

                let verified_parse = if args.verify {
                    let parsed = validate_generated_json(&json, args.pretty, pattern)
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
            DevSubcommand::JqPathMode(args) => std::process::exit(jq_path_mode::run(args)),
            #[cfg(feature = "bench-runner")]
            DevSubcommand::Compare(c) => match c.command {
                CompareSubcommand::Jq(args) => std::process::exit(compare::run(args)?),
            },
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

    let patterns: Vec<String> = if args.patterns == "all" {
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
        // `users_mix` expands in place, so it combines with other names; an unknown name is an
        // error rather than silently dropped (a typo must not leave a partial run).
        let mut queries = Vec::new();
        let mut unknown = Vec::new();
        for name in args.queries.split(',').map(str::trim) {
            if name == "users_mix" {
                queries.extend_from_slice(jq_bench::QueryType::users_mix());
            } else if let Some(q) = jq_bench::QueryType::from_str(name) {
                queries.push(q);
            } else {
                unknown.push(name);
            }
        }
        if !queries.is_empty() && !unknown.is_empty() {
            anyhow::bail!("Unknown query type(s): {}", unknown.join(", "));
        }
        queries
    };

    if queries.is_empty() {
        anyhow::bail!(
            "No valid query types specified. Available: identity, keys_unsorted, keys_unsorted_length, keys_unsorted_map, keys_unsorted_select, map, select, users_navigate, users_length, users_select, users_map, users_sort_by, users_interpolate, users_group_by, users_index (or users_mix for the users-pattern set)"
        );
    }

    // The `users_*` queries read `.users`; on another pattern both tools would error or answer
    // `null`, hash-match, and report a meaningless speedup.
    if queries.iter().any(jq_bench::QueryType::is_users_shaped) {
        if let Some(p) = patterns.iter().find(|p| p.as_str() != "users") {
            anyhow::bail!(
                "the users_* queries need `--patterns users` (got pattern `{p}`): they read `.users`"
            );
        }
    }

    if args.runs == 0 {
        anyhow::bail!("--runs must be at least 1: the median of no runs is undefined");
    }

    let config = jq_bench::BenchConfig {
        data_dir: args.data_dir,
        patterns,
        sizes,
        queries,
        succinctly_binary: args.binary,
        jq_binary: args.jq,
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

    if args.runs == 0 {
        anyhow::bail!("--runs must be at least 1: the median of no runs is undefined");
    }

    let config = yq_bench::BenchConfig {
        data_dir: args.data_dir,
        patterns,
        sizes,
        queries,
        succinctly_binary: args.binary,
        yq_required: args.yq.is_some(),
        yq_binary: args.yq.unwrap_or_else(|| PathBuf::from("yq")),
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
    ("wide-escaped-keys", generators::Pattern::WideEscapedKeys),
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
///
/// A line-delimited `pattern` (`ndjson`, `json-seq`) is checked line by line, and there is no
/// single value to keep: every `json-seq` line must open with the RS byte (0x1e) and no
/// `ndjson` line may. The faulty `json-seq` patterns (#6619) must have exactly one line that is
/// not JSON, and it must be the one the pattern puts at its end.
fn validate_generated_json(
    json: &str,
    keep_parsed: bool,
    pattern: generators::Pattern,
) -> Result<Option<serde_json::Value>> {
    if pattern.is_line_delimited() {
        let framed = pattern.is_framed();
        let lines = json.lines().count();
        let fault = pattern.fault_line(lines);
        // A faulty pattern with no line to put the fault on (an empty file) promised one.
        if pattern.has_fault() && fault.is_none() {
            anyhow::bail!("the {pattern:?} pattern has no line for its fault record");
        }
        for (n, line) in json.lines().enumerate() {
            let record = match (framed, line.strip_prefix('\u{1e}')) {
                (true, Some(rest)) => rest,
                (true, None) => anyhow::bail!("line {}: no RS byte", n + 1),
                (false, _) => line,
            };
            let parsed = serde_json::from_str::<serde_json::Value>(record);
            match (fault == Some(n), parsed) {
                (false, parsed) => {
                    parsed.with_context(|| format!("line {}", n + 1))?;
                }
                (true, Ok(_)) => anyhow::bail!("line {}: the fault record parses", n + 1),
                (true, Err(_)) => {}
            }
        }
        return Ok(None);
    }
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
                validate_generated_json(&json, false, *pattern)
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

mod address_space;
mod bench_reps;
#[cfg(feature = "bench-runner")]
mod bench_runner;
#[cfg(feature = "bench-runner")]
mod compare;
mod corpus_stats;
mod dsv_bench;
mod dsv_generators;
mod env_config;
mod error_snippet;
#[cfg(feature = "bench-runner")]
mod exit_status;
mod front_matter;
mod generators;
mod interrupt;
mod jq_bench;
mod jq_locate;
mod jq_path_mode;
mod jq_runner;
mod jq_seq_reader;
mod jq_stream_errors;
mod json_validate;
mod m2_gate;
mod main_thread_stack;
mod malloc_arena;
mod output;
mod relocation_pages;
mod select_stats_report;
mod sigpipe;
mod stdin_access;
mod stdin_offset;
mod text_generators;
mod text_validate;
mod utf8_bench;
mod yaml_generators;
mod yaml_pattern_registry;
mod yaml_validate;
mod yq_bench;
mod yq_locate;
mod yq_result_document;
mod yq_runner;
use generators::generate_json;

#[cfg(test)]
mod tests {
    use super::*;

    /// #5634: the evaluation thread tries its full stack first, then each half down to the floor,
    /// and never a size below it.
    #[test]
    fn test_eval_stack_sizes_halve_from_the_full_stack_to_the_floor_5634() {
        const MIB: usize = 1024 * 1024;
        assert_eq!(
            eval_stack_sizes(256 * MIB, MIN_EVAL_STACK_SIZE).collect::<Vec<_>>(),
            [256 * MIB, 128 * MIB, 64 * MIB, 32 * MIB, 16 * MIB]
        );
        // Not a power-of-two multiple of the floor: the last size is the last one at or above it.
        assert_eq!(
            eval_stack_sizes(100 * MIB, 16 * MIB).collect::<Vec<_>>(),
            [100 * MIB, 50 * MIB, 25 * MIB]
        );
        // A start below the floor is still tried, once.
        assert_eq!(
            eval_stack_sizes(8 * MIB, 16 * MIB).collect::<Vec<_>>(),
            [8 * MIB]
        );
    }

    /// #5634: a capped address space starts the stack at a quarter of the cap, never above the
    /// full stack nor below the floor; an uncapped one starts at the full stack.
    #[test]
    fn test_eval_stack_start_follows_the_address_space_cap_5634() {
        const MIB: usize = 1024 * 1024;
        let max = 256 * MIB;
        assert_eq!(eval_stack_start(max, None), max);
        assert_eq!(eval_stack_start(max, Some(4096 * MIB)), max);
        assert_eq!(eval_stack_start(max, Some(512 * MIB)), 128 * MIB);
        assert_eq!(eval_stack_start(max, Some(64 * MIB)), MIN_EVAL_STACK_SIZE);
        assert_eq!(eval_stack_start(max, Some(MIB)), MIN_EVAL_STACK_SIZE);
        // A full stack already below the floor is never raised above itself.
        assert_eq!(eval_stack_start(8 * MIB, Some(MIB)), 8 * MIB);
    }

    /// The cap the test process itself runs under is read without failing.
    #[test]
    fn test_address_space_cap_reads_the_process_limit_5634() {
        let _ = address_space::cap();
    }

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

    /// #7051: yq's flags are Go `pflag` ones, so a repeated value flag takes the last value
    /// (`yq -o json -o yaml .` prints YAML), where a clap argument refuses a second occurrence unless
    /// it overrides itself. Introspects the live `YqCommand`, so a new value flag that forgets
    /// `overrides_with` fails here instead of reintroducing the refusal for that one flag. The
    /// `Append` flags (`--arg`, `--argjson`) are repeatable by design and skipped.
    #[test]
    fn test_every_single_value_yq_flag_takes_its_last_occurrence_7051() {
        use clap::CommandFactory;

        let mut cmd = YqCommand::command();
        // `get_num_args` is unset until the command is built.
        cmd.build();
        let mut checked = 0;
        for arg in cmd.get_arguments() {
            let takes_one_value = arg
                .get_num_args()
                .is_some_and(|range| range.min_values() == 1 && range.max_values() == 1);
            let sets = matches!(arg.get_action(), clap::ArgAction::Set);
            if arg.is_positional() || !takes_one_value || !sets {
                continue;
            }
            let flag = format!(
                "--{}",
                arg.get_long().expect("a yq value flag has a long name")
            );
            // Two different values the argument's own parser accepts, so the only error left is a
            // refused repeat and the value kept shows which occurrence won.
            let possible: Vec<String> = arg
                .get_possible_values()
                .iter()
                .map(|v| v.get_name().to_string())
                .collect();
            let (first, last) = match possible.as_slice() {
                [a, .., b] => (a.clone(), b.clone()),
                _ => ("1".to_string(), "2".to_string()),
            };
            let argv = ["yq", &flag, &first, &flag, &last];
            let parsed = cmd.clone().try_get_matches_from(argv);
            assert!(
                parsed.is_ok(),
                "repeating {flag} is refused (give it overrides_with): {:?}",
                parsed.as_ref().err()
            );
            let matches = parsed.expect("checked above");
            let kept: Vec<_> = matches
                .get_raw(arg.get_id().as_str())
                .expect("the flag was given")
                .collect();
            assert_eq!(kept, [last.as_str()], "{flag} keeps its first occurrence");
            checked += 1;
        }
        // Only the single-value flags that set their field are matched: a flag declared with
        // `Append`, a delimiter or a `num_args` range is not covered here. There are nineteen today;
        // an introspection that matched none would pass vacuously.
        assert!(
            checked >= 15,
            "only {checked} single-value flags were checked"
        );
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

    /// #3389: every character named in the issue's own boundary table, plus
    /// the edge cases the CLI-level tests in `tests/jq_cli_tests.rs` can't
    /// isolate as cleanly as a direct unit test can (a token that doesn't
    /// start with `-` at all, and an empty string).
    #[test]
    fn test_looks_like_negative_filter_3389() {
        for filter_like in [
            "-1",
            "-1.5",
            "-.[0]",
            "-(1)",
            "-$__loc__.line",
            "-[]",
            "-{}",
            "-\"a\"",
            "-",
        ] {
            assert!(
                looks_like_negative_filter(filter_like),
                "{filter_like:?} should read as a filter"
            );
        }
        for option_like in ["-x", "-n1", "-c", "--bogus", "--", "abc", ""] {
            assert!(
                !looks_like_negative_filter(option_like),
                "{option_like:?} should read as an option"
            );
        }
    }

    /// #7149: `succinctly jq` and `succinctly yq` parse their own struct, and what they render (the short
    /// and the long help, the usage line) is what the tree renders for the same subcommand.
    #[test]
    fn direct_subcommands_render_the_trees_help_7149() {
        use clap::CommandFactory;
        for (name, about, mut direct) in [
            ("jq", JQ_SUBCOMMAND_ABOUT, JqCommand::command()),
            ("yq", YQ_SUBCOMMAND_ABOUT, YqCommand::command()),
        ] {
            let mut tree = Cli::command().bin_name("succinctly");
            tree.build();
            let in_tree = tree.find_subcommand_mut(name).expect("a subcommand");
            direct = direct.about(about).bin_name(format!("succinctly {name}"));
            assert_eq!(
                in_tree.render_help().to_string(),
                direct.render_help().to_string(),
                "{name}: short help"
            );
            assert_eq!(
                in_tree.render_long_help().to_string(),
                direct.render_long_help().to_string(),
                "{name}: long help"
            );
            assert_eq!(
                in_tree.render_usage().to_string(),
                direct.render_usage().to_string(),
                "{name}: usage"
            );
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

    #[test]
    fn test_pattern_arg_line_delimited_patterns_map_to_generator_patterns() {
        assert!(matches!(
            generators::Pattern::from(PatternArg::Ndjson),
            generators::Pattern::Ndjson
        ));
        assert!(matches!(
            generators::Pattern::from(PatternArg::JsonSeq),
            generators::Pattern::JsonSeq
        ));
    }

    /// #6619: the faulty `json-seq` patterns have one line that is not JSON, at the end the
    /// pattern names; a clean stream, or the fault at the other end, is not what they promise.
    #[test]
    fn test_validate_generated_json_expects_the_fault_where_the_pattern_puts_it() {
        use generators::Pattern;
        let good = "\u{1e}{\"a\":1}\n";
        let fault = "\u{1e}{\"a\":\n";
        let first = format!("{fault}{good}{good}");
        let last = format!("{good}{good}{fault}");
        let clean = format!("{good}{good}");
        assert!(validate_generated_json(&first, false, Pattern::JsonSeqBadFirst).is_ok());
        assert!(validate_generated_json(&last, false, Pattern::JsonSeqBadLast).is_ok());
        // The fault at the wrong end, a stream without one, and one in the clean pattern.
        assert!(validate_generated_json(&last, false, Pattern::JsonSeqBadFirst).is_err());
        assert!(validate_generated_json(&first, false, Pattern::JsonSeqBadLast).is_err());
        assert!(validate_generated_json(&clean, false, Pattern::JsonSeqBadLast).is_err());
        assert!(validate_generated_json(&last, false, Pattern::JsonSeq).is_err());
        // A second bad line is a generator bug, not the fault.
        let two = format!("{fault}{good}{fault}");
        assert!(validate_generated_json(&two, false, Pattern::JsonSeqBadLast).is_err());
        // An empty file has no line for the fault either.
        assert!(validate_generated_json("", false, Pattern::JsonSeqBadLast).is_err());
        assert!(validate_generated_json("", false, Pattern::JsonSeqBadFirst).is_err());
        assert!(validate_generated_json("", false, Pattern::JsonSeq).is_ok());
    }

    #[test]
    fn test_validate_generated_json_per_line_checks_every_record() {
        use generators::Pattern;
        let plain = "{\"a\":1}\n{\"a\":2}\n";
        let framed = "\u{1e}{\"a\":1}\n\u{1e}{\"a\":2}\n";
        assert!(validate_generated_json(plain, false, Pattern::Ndjson)
            .unwrap()
            .is_none());
        assert!(validate_generated_json(framed, false, Pattern::JsonSeq)
            .unwrap()
            .is_none());
        // The format is part of the check: RS belongs to json-seq and only to it.
        assert!(validate_generated_json(framed, false, Pattern::Ndjson).is_err());
        let err = validate_generated_json(plain, false, Pattern::JsonSeq).unwrap_err();
        assert!(format!("{err:#}").contains("line 1: no RS byte"), "{err:#}");
        // A single-document pattern is still parsed whole.
        assert!(validate_generated_json(plain, false, Pattern::Users).is_err());
        let err =
            validate_generated_json("{\"a\":1}\n{\"a\":\n", false, Pattern::Ndjson).unwrap_err();
        assert!(format!("{err:#}").contains("line 2"), "{err:#}");
    }

    #[test]
    fn test_pattern_arg_wide_escaped_keys_maps_to_generator_pattern() {
        assert!(matches!(
            generators::Pattern::from(PatternArg::WideEscapedKeys),
            generators::Pattern::WideEscapedKeys
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
            jq: PathBuf::from("jq"),
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
    fn test_run_jq_benchmark_rejects_a_partly_unknown_query_list() {
        let args = bench_jq_args("users", "1kb", "identity,users_sortby", "./nonexistent");
        let err = run_jq_benchmark(args).unwrap_err();
        assert!(
            err.to_string()
                .contains("Unknown query type(s): users_sortby"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_run_jq_benchmark_rejects_users_queries_on_other_patterns() {
        // `users_mix` combines with other names, and is refused off the `users` pattern
        let args = bench_jq_args("arrays", "1kb", "identity,users_mix", "./nonexistent");
        let err = run_jq_benchmark(args).unwrap_err();
        assert!(
            err.to_string().contains("need `--patterns users`"),
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

    #[test]
    fn test_run_jq_benchmark_rejects_zero_runs() {
        let mut args = bench_jq_args("arrays", "1kb", "identity", "./nonexistent");
        args.runs = 0;
        let err = run_jq_benchmark(args).unwrap_err();
        assert!(
            err.to_string().contains("--runs must be at least 1"),
            "{err}"
        );
    }

    /// #5409: `--jq` reaches the benchmark, which refuses a jq it cannot run before timing
    /// anything (the old runner always ran the `jq` on `PATH`).
    #[test]
    fn test_run_jq_benchmark_uses_the_given_jq() {
        let mut args = bench_jq_args("arrays", "1kb", "identity", "./nonexistent");
        args.jq = PathBuf::from("/nonexistent/jq-for-bench-test");
        let err = run_jq_benchmark(args).unwrap_err();
        assert!(
            err.to_string()
                .contains("jq not found at `/nonexistent/jq-for-bench-test`"),
            "unexpected error: {err}"
        );
    }

    fn bench_yq_args(binary: &str) -> BenchYqArgs {
        BenchYqArgs {
            data_dir: PathBuf::from("data/bench/generated/yaml"),
            output: None,
            markdown: None,
            patterns: "arrays".to_string(),
            sizes: "1kb".to_string(),
            queries: "identity".to_string(),
            no_memory: true,
            warmup: 0,
            runs: 1,
            binary: PathBuf::from(binary),
            yq: None,
        }
    }

    /// #6417: `--yq` reaches the benchmark, which refuses a yq it cannot run before timing anything
    /// (the old runner always ran the `yq` on `PATH`), and `--runs 0` is a usage error.
    #[test]
    fn test_run_yq_benchmark_uses_the_given_yq() {
        let mut args = bench_yq_args("./nonexistent");
        args.yq = Some(PathBuf::from("/nonexistent/yq-for-bench-test"));
        let err = run_yq_benchmark(args).unwrap_err();
        assert!(
            err.to_string()
                .contains("yq not found at `/nonexistent/yq-for-bench-test`"),
            "unexpected error: {err}"
        );
        let mut args = bench_yq_args("./nonexistent");
        args.runs = 0;
        let err = run_yq_benchmark(args).unwrap_err();
        assert!(
            err.to_string().contains("--runs must be at least 1"),
            "unexpected error: {err}"
        );
    }

    /// #7501: `--indent` reads its value as jq 1.8 does: a sign and ASCII digits, nothing else,
    /// in -1 to 7 (rows captured from `/opt/homebrew/bin/jq` 1.8.2; `tests/jq_cli/i6017.rs` runs
    /// them end to end).
    #[test]
    fn the_indent_value_is_a_signed_number_and_nothing_else() {
        let ok = |text: &str| parse_jq_indent(text).unwrap();
        for (text, want) in [
            ("0", 0),
            ("7", 7),
            ("-1", -1),
            ("+2", 2),
            ("+0", 0),
            ("-0", 0),
            ("-00", 0),
            ("007", 7),
            ("0003", 3),
            ("+7", 7),
            ("-01", -1),
            ("0000000000000000000000000000000000000003", 3),
        ] {
            assert_eq!(ok(text), want, "{text:?}");
        }
        for text in [
            "",
            " ",
            " 2",
            "2 ",
            "\t3",
            "\n3",
            "3\n",
            "3x",
            "x3",
            "abc",
            "2.5",
            "3.0",
            "-1.0",
            "0.5",
            "1e1",
            "3e0",
            "0x2",
            "0b1",
            "+-3",
            "-+3",
            "--3",
            "+",
            "-",
            "--",
            "1_0",
            "3,",
            "1 2",
            "8",
            "-2",
            "+8",
            "99999999999",
            "-99999999999",
            "99999999999999999999",
            "2147483648",
            "4294967297",
            "-4294967295",
            "\u{663}",
            "\u{0661}",
            "-c",
            "--tab",
        ] {
            assert!(
                parse_jq_indent(text)
                    .unwrap_err()
                    .contains("between -1 and 7"),
                "{text:?}"
            );
        }
    }
}

/// #7438, #7494: [`walk_jq_argv`] on argv alone, without running a command.
#[cfg(test)]
mod jq_argv_7438 {
    use super::*;
    use std::ffi::OsString;

    fn walk(words: &[&str]) -> Result<JqArgv, UnknownJqOption> {
        use clap::CommandFactory;
        let mut cmd = JqCommand::command();
        cmd.build();
        walk_jq_argv(&cmd, words)
    }

    fn os(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    /// The options clap is given and the positional words, each with its mode.
    fn read(words: &[&str]) -> (Vec<OsString>, Vec<(WordMode, OsString)>) {
        let walked = walk(words).unwrap_or_else(|e| panic!("{words:?}: {e:?}"));
        (walked.options, walked.positional)
    }

    fn positional(words: &[(WordMode, &str)]) -> Vec<(WordMode, OsString)> {
        words.iter().map(|(m, w)| (*m, OsString::from(w))).collect()
    }

    fn unknown(words: &[&str]) -> String {
        match walk(words) {
            Err(UnknownJqOption(word)) => String::from_utf8(word).unwrap(),
            Ok(walked) => panic!("{words:?} was accepted: {walked:?}"),
        }
    }

    use WordMode::{File, Json, Str};

    #[test]
    fn a_bundle_is_read_a_letter_at_a_time_and_a_letter_may_repeat() {
        assert_eq!(read(&["jq", "-cc", "."]).0, os(&["jq", "-c", "-c"]));
        assert_eq!(read(&["jq", "-nre", "."]).0, os(&["jq", "-n", "-r", "-e"]));
    }

    #[test]
    fn l_ends_a_bundle_and_takes_the_rest_of_the_word_or_the_next_word() {
        assert_eq!(
            read(&["jq", "-nL", "lib", "."]),
            (os(&["jq", "-n", "-L", "lib"]), positional(&[(File, ".")]))
        );
        assert_eq!(
            read(&["jq", "-RsjLlib", "."]).0,
            os(&["jq", "-R", "-s", "-j", "-L", "lib"])
        );
        // The rest of the word is the value as it stands: no `=` is taken off it, unlike clap's.
        assert_eq!(read(&["jq", "-L=x", "."]).0, os(&["jq", "-L", "=x"]));
        // Nothing left to take: clap reports the missing value.
        assert_eq!(read(&["jq", "-nL"]).0, os(&["jq", "-n", "-L"]));
    }

    #[test]
    fn values_stay_with_their_option_whatever_they_look_like() {
        assert_eq!(
            read(&["jq", "--arg", "x", "-c", "--args", "-L", "--", "."]),
            (
                os(&["jq", "--arg", "x", "-c", "-L", "--"]),
                positional(&[(Str, ".")])
            )
        );
        assert_eq!(
            read(&["jq", "--indent", "-1", "--slurpfile", "a", "-1", "."]).0,
            os(&["jq", "--indent", "-1", "--slurpfile", "a", "-1"])
        );
        // `--arg a --args` hands `--args` to `--arg` as its value: there is no mode flag here.
        assert_eq!(
            read(&["jq", "-n", "--arg", "a", "--args", "$P", "b"]),
            (
                os(&["jq", "-n", "--arg", "a", "--args"]),
                positional(&[(File, "$P"), (File, "b")])
            )
        );
    }

    /// #7494: a word that is `-` and then neither a letter nor a `-` is positional, and options
    /// are still read after it.
    #[test]
    fn options_are_read_after_a_hyphen_led_positional_word() {
        assert_eq!(
            read(&["jq", "-1", "-n"]),
            (os(&["jq", "-n"]), positional(&[(File, "-1")]))
        );
        assert_eq!(
            read(&["jq", ".", "a.json", "-1", "-c", "-.a", "-", "-(1)"]),
            (
                os(&["jq", "-c"]),
                positional(&[
                    (File, "."),
                    (File, "a.json"),
                    (File, "-1"),
                    (File, "-.a"),
                    (File, "-"),
                    (File, "-(1)"),
                ])
            )
        );
        assert_eq!(
            read(&["jq", "$x", "-1", "--arg", "x", "2", "-n"]).0,
            os(&["jq", "--arg", "x", "2", "-n"])
        );
    }

    #[test]
    fn a_double_dash_ends_the_options_and_is_consumed() {
        assert_eq!(
            read(&["jq", ".", "a.json", "-1", "--", "-c", "--"]),
            (
                os(&["jq"]),
                positional(&[
                    (File, "."),
                    (File, "a.json"),
                    (File, "-1"),
                    (File, "-c"),
                    (File, "--"),
                ])
            )
        );
        // After `--args --`, an option-shaped word is the program, as in jq (#7494).
        assert_eq!(
            read(&["jq", "-n", "--args", "--", "-c", "."]),
            (os(&["jq", "-n"]), positional(&[(Str, "-c"), (Str, ".")]))
        );
    }

    #[test]
    fn args_and_jsonargs_set_the_mode_of_the_words_after_them() {
        assert_eq!(
            read(&[
                "jq",
                ".",
                "a.json",
                "--args",
                "s",
                "-c",
                "--jsonargs",
                "1",
                "--args",
                "t"
            ]),
            (
                os(&["jq", "-c"]),
                positional(&[
                    (File, "."),
                    (File, "a.json"),
                    (Str, "s"),
                    (Json, "1"),
                    (Str, "t"),
                ])
            )
        );
    }

    #[test]
    fn a_long_option_is_an_exact_name() {
        for word in [
            "--indent=3",
            "--args=",
            "--jsonargs=1",
            "--from-file=p",
            "--compac",
            "--Null-input",
            "--library-path",
            "--debug-trace",
            "--",
        ] {
            let argv = ["jq", "-n", word, "."];
            if word == "--" {
                assert_eq!(read(&argv).0, os(&["jq", "-n"]));
            } else {
                assert_eq!(unknown(&argv), word, "{word}");
            }
        }
        // `--binary` is accepted as in jq 1.8 (jqlang/jq#3131).
        assert_eq!(read(&["jq", "--binary", "."]).0, os(&["jq", "--binary"]));
    }

    /// A succinctly-only option keeps clap's `--name=value`, which its documentation uses; a jq
    /// option's name never does.
    #[test]
    fn an_extension_option_keeps_its_equals_form() {
        assert_eq!(
            read(&["jq", "--on-error=stop", "--input-dsv=,", "."]).0,
            os(&["jq", "--on-error=stop", "--input-dsv=,"])
        );
        assert_eq!(unknown(&["jq", "--tab=1", "."]), "--tab=1");
        assert_eq!(unknown(&["jq", "--bogus=1", "."]), "--bogus=1");
        for name in JQ_LONG_OPTIONS {
            assert!(
                !matches!(
                    *name,
                    "on-error" | "input-dsv" | "validate" | "preserve-input"
                ),
                "{name}"
            );
        }
    }

    /// jq 1.8.2 names the first letter it does not define, where 1.7.1 named the whole word
    /// (jqlang/jq#3194, the same change that lets a letter repeat).
    #[test]
    fn an_unknown_letter_is_named_alone() {
        assert_eq!(unknown(&["jq", "-fprog.jq", "a.json"]), "-p");
        assert_eq!(unknown(&["jq", "-n", "-library-path"]), "-l");
        assert_eq!(unknown(&["jq", "-xV"]), "-x");
        assert_eq!(unknown(&["jq", "-n-"]), "--");
        assert_eq!(unknown(&["jq", "-c1"]), "-1");
    }

    /// jq exits at a help, version or build-configuration option, so nothing after it is read.
    #[test]
    fn help_and_version_end_the_walk() {
        assert_eq!(read(&["jq", "-V1"]).0, os(&["jq", "-V"]));
        assert_eq!(read(&["jq", "-nVx", "--bogus"]).0, os(&["jq", "-n", "-V"]));
        assert_eq!(read(&["jq", "-hV"]).0, os(&["jq", "-h"]));
        assert_eq!(read(&["jq", "-Vh"]).0, os(&["jq", "-V"]));
        assert_eq!(
            read(&["jq", "--build-configuration", "-x"]).0,
            os(&["jq", "--build-configuration"])
        );
        assert_eq!(read(&["jq", "--help", "--bogus"]).0, os(&["jq", "--help"]));
    }

    /// Every word is read by its bytes: one that is not UTF-8 is positional, or the value of an
    /// option, as its bytes say (#7426).
    #[cfg(unix)]
    #[test]
    fn a_word_that_is_not_utf8_is_read_by_its_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let bad = |bytes: &[u8]| std::ffi::OsStr::from_bytes(bytes).to_owned();
        let mut cmd = <JqCommand as clap::CommandFactory>::command();
        cmd.build();
        let argv = [
            OsString::from("jq"),
            OsString::from("-L"),
            bad(b"l\xff"),
            bad(b"-\xff"),
            bad(b"x\xff"),
        ];
        let walked = walk_jq_argv(&cmd, &argv).unwrap();
        assert_eq!(walked.options[1..3], argv[1..3]);
        assert_eq!(walked.positional.len(), 2);
        assert_eq!(walked.positional[0].1, argv[3]);
        assert_eq!(
            walk_jq_argv(&cmd, &[OsString::from("jq"), bad(b"-n\xff")]),
            Err(UnknownJqOption(b"-\xff".to_vec()))
        );
        assert_eq!(
            walk_jq_argv(&cmd, &[OsString::from("jq"), bad(b"--a\xff")]),
            Err(UnknownJqOption(b"--a\xff".to_vec()))
        );
    }
}

/// #7712: where [`read_jq_argv`] stops, and how many options come before the stop.
#[cfg(test)]
mod jq_argv_stop_7712 {
    use super::*;

    fn stop(words: &[&str]) -> Option<(usize, JqStopKind)> {
        use clap::CommandFactory;
        let mut cmd = JqCommand::command();
        cmd.build();
        read_jq_argv(&cmd, words).stop.map(|s| (s.at, s.kind))
    }

    #[test]
    fn the_walk_stops_where_jq_s_option_loop_stops() {
        use JqStopKind::{Clap, InvalidJson, MissingValue, Unknown};
        assert_eq!(stop(&["jq", "-n", "--argjson", "a", "1", "."]), None);
        // The options before the stop: `jq`, `-n`, `--slurpfile a f`.
        assert_eq!(
            stop(&["jq", "-n", "--slurpfile", "a", "f", "-h"]),
            Some((5, Clap))
        );
        assert_eq!(stop(&["jq", "-ch", "-x"]), Some((2, Clap)));
        assert_eq!(
            stop(&["jq", "--argjson", "a", "{", "-x"]),
            Some((1, InvalidJson("argjson")))
        );
        // A bound name's value is not read, so it cannot stop the walk.
        assert_eq!(
            stop(&["jq", "--arg", "a", "1", "--argjson", "a", "{", "-x"]),
            Some((7, Unknown(b"-x".to_vec())))
        );
        assert_eq!(
            stop(&["jq", "--jsonargs", "{", "1", "{", "-x"]),
            Some((1, InvalidJson("jsonargs")))
        );
        assert_eq!(stop(&["jq", "--indent", "9", "-x"]), Some((1, Clap)));
        assert_eq!(
            stop(&["jq", "--on-error", "stop", "-x"]),
            Some((3, Unknown(b"-x".to_vec())))
        );
        assert_eq!(stop(&["jq", "--on-error", "bogus", "-x"]), Some((1, Clap)));
        assert_eq!(
            stop(&["jq", ".", "--arg", "x"]),
            Some((
                1,
                MissingValue("jq: --arg takes two parameters (e.g. --arg varname value)")
            ))
        );
        assert!(
            matches!(stop(&["jq", "-nL"]), Some((2, MissingValue(m))) if m.starts_with("-L takes"))
        );
        assert_eq!(stop(&["jq", "-cq"]), Some((2, Unknown(b"-q".to_vec()))));
    }
}

#[cfg(test)]
mod popcnt_required_tests_5035 {
    use super::{cpuid_ecx_reports_popcnt, popcnt_required_message};

    /// #5035: the refusal names the program, the instruction, the CPUs that have it and the way to a build that
    /// does not need it; it is the whole of what a user on an older CPU sees.
    #[test]
    fn the_message_names_the_requirement_and_the_way_out() {
        let message = popcnt_required_message("succinctly");
        assert!(message.starts_with("succinctly: "), "{message}");
        for needle in ["POPCNT", "Nehalem", "K10", "cargo install succinctly"] {
            assert!(message.contains(needle), "{needle}: {message}");
        }
        assert_eq!(message.lines().count(), 2, "{message}");
    }

    /// CPUID.01H:ECX bit 23 is POPCNT. The values are real leaf-1 `ecx` words: a Core 2 (no POPCNT, SSE4.1 only)
    /// and a Nehalem-class CPU (POPCNT and SSE4.2, bits 23 and 20), and the bit alone.
    #[test]
    fn the_guard_reads_the_popcnt_bit_of_cpuid_leaf_1() {
        assert!(
            !cpuid_ecx_reports_popcnt(0x0408_E3FD),
            "Core 2: SSE4.1, no POPCNT"
        );
        assert!(!cpuid_ecx_reports_popcnt(0), "an empty word");
        assert!(
            cpuid_ecx_reports_popcnt(0x029E_E3FF),
            "Nehalem: SSE4.2 and POPCNT"
        );
        assert!(cpuid_ecx_reports_popcnt(1 << 23), "the bit alone");
        assert!(
            !cpuid_ecx_reports_popcnt(!(1u32 << 23)),
            "every other bit set"
        );
    }
}
