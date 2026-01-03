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
        },
    }
}

mod generators;
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
