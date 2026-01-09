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
    /// Command-line JSON processor (jq-compatible)
    Jq(JqCommand),
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
    /// Query JSON using jq-like expressions
    Query(QueryJson),
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

/// Query JSON using jq-like expressions
#[derive(Debug, Parser)]
struct QueryJson {
    /// jq expression to evaluate (e.g., ".foo.bar[0]", ".users[].name")
    expression: String,

    /// Input JSON file (reads from stdin if not provided)
    #[arg(short, long)]
    input: Option<PathBuf>,

    /// Output format
    #[arg(short, long, default_value = "json")]
    format: OutputFormat,

    /// Compact output (no pretty printing)
    #[arg(short, long)]
    compact: bool,

    /// Show raw strings without quotes
    #[arg(short, long)]
    raw: bool,

    /// Use memory-mapped I/O for reading files (more efficient for large files)
    #[arg(long)]
    mmap: bool,
}

#[derive(Debug, Clone, ValueEnum)]
enum OutputFormat {
    /// Standard JSON output
    Json,
    /// One value per line (newline-delimited)
    Lines,
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
            JsonSubcommand::GenerateSuite(args) => generate_suite(args),
            JsonSubcommand::Query(args) => query_json(args),
        },
    }
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

fn generate_suite(args: GenerateSuite) -> Result<()> {
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

/// Wrapper enum to hold either owned bytes or memory-mapped file
enum JsonInput {
    Owned(Vec<u8>),
    Mmap(memmap2::Mmap),
}

impl AsRef<[u8]> for JsonInput {
    fn as_ref(&self) -> &[u8] {
        match self {
            JsonInput::Owned(v) => v,
            JsonInput::Mmap(m) => m,
        }
    }
}

fn query_json(args: QueryJson) -> Result<()> {
    use std::io::{BufWriter, Read, Write};
    use succinctly::jq;
    use succinctly::json::light::StandardJson;
    use succinctly::json::JsonIndex;

    // Read input JSON
    let json_input: JsonInput = match &args.input {
        Some(path) => {
            if args.mmap {
                let file = std::fs::File::open(path)
                    .with_context(|| format!("Failed to open {}", path.display()))?;
                // SAFETY: We assume the file won't be modified while we're reading it
                let mmap = unsafe { memmap2::Mmap::map(&file) }
                    .with_context(|| format!("Failed to memory-map {}", path.display()))?;
                JsonInput::Mmap(mmap)
            } else {
                JsonInput::Owned(
                    std::fs::read(path)
                        .with_context(|| format!("Failed to read {}", path.display()))?,
                )
            }
        }
        None => {
            if args.mmap {
                anyhow::bail!("--mmap requires an input file (-i)");
            }
            let mut buf = Vec::new();
            std::io::stdin()
                .read_to_end(&mut buf)
                .context("Failed to read from stdin")?;
            JsonInput::Owned(buf)
        }
    };

    let json_bytes = json_input.as_ref();

    // Parse the jq expression
    let expr = jq::parse(&args.expression).map_err(|e| anyhow::anyhow!("Parse error: {}", e))?;

    // Build the index and run the query
    let index = JsonIndex::build(json_bytes);
    let cursor = index.root(json_bytes);
    let result = jq::eval(&expr, cursor);

    // Use buffered output for better performance
    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    // Write a single value to output - use raw bytes when possible for performance
    let write_value = |out: &mut BufWriter<_>,
                       value: &StandardJson<'_, Vec<u64>>,
                       raw: bool|
     -> std::io::Result<()> {
        match value {
            StandardJson::String(s) => {
                if raw {
                    // Raw mode: output string content without quotes
                    if let Ok(str_val) = s.as_str() {
                        out.write_all(str_val.as_bytes())?;
                    }
                } else {
                    // Use raw_bytes() which includes quotes - zero-copy!
                    out.write_all(s.raw_bytes())?;
                }
            }
            StandardJson::Number(n) => {
                // Use raw_bytes() - zero-copy!
                out.write_all(n.raw_bytes())?;
            }
            StandardJson::Bool(b) => {
                out.write_all(if *b { b"true" } else { b"false" })?;
            }
            StandardJson::Null => {
                out.write_all(b"null")?;
            }
            StandardJson::Object(_) | StandardJson::Array(_) => {
                // For complex types, we still need to reconstruct JSON
                // TODO: optimize by finding byte range via BP find_close
                let json_str = reconstruct_json(value, false);
                out.write_all(json_str.as_bytes())?;
            }
            StandardJson::Error(e) => {
                write!(out, "<error: {}>", e)?;
            }
        }
        Ok(())
    };

    // Output results - default to jq-style one-value-per-line
    match result {
        jq::QueryResult::One(value) => {
            write_value(&mut out, &value, args.raw)?;
            writeln!(out)?;
        }
        jq::QueryResult::Many(values) => match args.format {
            OutputFormat::Json => {
                if args.compact {
                    out.write_all(b"[")?;
                    for (i, value) in values.iter().enumerate() {
                        if i > 0 {
                            out.write_all(b",")?;
                        }
                        write_value(&mut out, value, args.raw)?;
                    }
                    writeln!(out, "]")?;
                } else {
                    writeln!(out, "[")?;
                    for (i, value) in values.iter().enumerate() {
                        out.write_all(b"  ")?;
                        write_value(&mut out, value, args.raw)?;
                        if i < values.len() - 1 {
                            writeln!(out, ",")?;
                        } else {
                            writeln!(out)?;
                        }
                    }
                    writeln!(out, "]")?;
                }
            }
            OutputFormat::Lines => {
                for value in values.iter() {
                    write_value(&mut out, value, args.raw)?;
                    writeln!(out)?;
                }
            }
        },
        jq::QueryResult::None => {
            // Print nothing for None (matches jq behavior for optional that didn't find anything)
        }
        jq::QueryResult::Error(e) => {
            anyhow::bail!("Query error: {}", e);
        }
        jq::QueryResult::Owned(value) => {
            writeln!(out, "{}", value.to_json())?;
        }
        jq::QueryResult::ManyOwned(values) => match args.format {
            OutputFormat::Json => {
                if args.compact {
                    out.write_all(b"[")?;
                    for (i, value) in values.iter().enumerate() {
                        if i > 0 {
                            out.write_all(b",")?;
                        }
                        out.write_all(value.to_json().as_bytes())?;
                    }
                    writeln!(out, "]")?;
                } else {
                    writeln!(out, "[")?;
                    for (i, value) in values.iter().enumerate() {
                        out.write_all(b"  ")?;
                        out.write_all(value.to_json().as_bytes())?;
                        if i < values.len() - 1 {
                            writeln!(out, ",")?;
                        } else {
                            writeln!(out)?;
                        }
                    }
                    writeln!(out, "]")?;
                }
            }
            OutputFormat::Lines => {
                for value in values.iter() {
                    writeln!(out, "{}", value.to_json())?;
                }
            }
        },
    }

    out.flush()?;
    Ok(())
}

/// Reconstruct JSON string from a StandardJson value
fn reconstruct_json<W: Clone + AsRef<[u64]>>(
    value: &succinctly::json::light::StandardJson<'_, W>,
    pretty: bool,
) -> String {
    use succinctly::json::light::StandardJson;

    fn inner<W: Clone + AsRef<[u64]>>(
        value: &StandardJson<'_, W>,
        pretty: bool,
        indent: usize,
    ) -> String {
        let indent_str = if pretty {
            "  ".repeat(indent)
        } else {
            String::new()
        };
        let newline = if pretty { "\n" } else { "" };

        match value {
            StandardJson::String(s) => {
                format!("\"{}\"", s.as_str().unwrap_or_default())
            }
            StandardJson::Number(n) => {
                if let Ok(i) = n.as_i64() {
                    i.to_string()
                } else if let Ok(f) = n.as_f64() {
                    f.to_string()
                } else {
                    "null".to_string()
                }
            }
            StandardJson::Bool(b) => b.to_string(),
            StandardJson::Null => "null".to_string(),
            StandardJson::Object(fields) => {
                let items: Vec<_> = (*fields)
                    .map(|f| {
                        let key = match f.key() {
                            StandardJson::String(s) => s.as_str().unwrap_or_default().to_string(),
                            _ => String::new(),
                        };
                        let val = inner(&f.value(), pretty, indent + 1);
                        if pretty {
                            format!("{}  \"{}\": {}", indent_str, key, val)
                        } else {
                            format!("\"{}\":{}", key, val)
                        }
                    })
                    .collect();

                if items.is_empty() {
                    "{}".to_string()
                } else if pretty {
                    format!(
                        "{{{}{}{}{}}}",
                        newline,
                        items.join(&format!(",{}", newline)),
                        newline,
                        indent_str
                    )
                } else {
                    format!("{{{}}}", items.join(","))
                }
            }
            StandardJson::Array(elements) => {
                let items: Vec<_> = (*elements)
                    .map(|e| {
                        let val = inner(&e, pretty, indent + 1);
                        if pretty {
                            format!("{}  {}", indent_str, val)
                        } else {
                            val
                        }
                    })
                    .collect();

                if items.is_empty() {
                    "[]".to_string()
                } else if pretty {
                    format!(
                        "[{}{}{}{}]",
                        newline,
                        items.join(&format!(",{}", newline)),
                        newline,
                        indent_str
                    )
                } else {
                    format!("[{}]", items.join(","))
                }
            }
            StandardJson::Error(e) => format!("<error: {}>", e),
        }
    }

    inner(value, pretty, 0)
}

mod generators;
mod jq_runner;
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
