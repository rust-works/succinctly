//! xq-locate command implementation.
//!
//! Finds the xq expression that navigates to a specific position in an XML
//! file. Mirrors `yq_locate.rs`/`jq_locate.rs`.

use anyhow::{Context, Result};
use clap::ValueEnum;
use std::path::PathBuf;

use succinctly::text::LineIndex;
use succinctly::xml::{locate_offset_detailed, XmlIndex};

/// Arguments for xq-locate command
#[derive(Debug, clap::Parser)]
pub struct XqLocateArgs {
    /// Input XML file
    pub file: PathBuf,

    /// Byte offset in file (0-indexed)
    #[arg(long, conflicts_with_all = ["line", "column"])]
    pub offset: Option<usize>,

    /// Line number (1-indexed)
    #[arg(long, requires = "column")]
    pub line: Option<usize>,

    /// Column number (1-indexed, byte offset within line)
    #[arg(long, requires = "line")]
    pub column: Option<usize>,

    /// Output format
    #[arg(long, default_value = "expression")]
    pub format: LocateFormat,
}

/// Output format for xq-locate
#[derive(Debug, Clone, Default, ValueEnum)]
pub enum LocateFormat {
    /// Just the xq expression (default)
    #[default]
    Expression,
    /// JSON object with expression, type, and byte range
    Json,
}

/// Run the xq-locate command
pub fn run_xq_locate(args: XqLocateArgs) -> Result<i32> {
    let text = std::fs::read(&args.file)
        .with_context(|| format!("Failed to read file: {}", args.file.display()))?;

    let offset = match (args.offset, args.line, args.column) {
        (Some(off), None, None) => off,
        (None, Some(line), Some(column)) => {
            let line_index = LineIndex::build(&text);
            line_index
                .to_offset(line, column)
                .with_context(|| format!("Invalid position: line {line} column {column}"))?
        }
        (None, None, None) => {
            anyhow::bail!("Either --offset or --line/--column must be specified");
        }
        _ => unreachable!(), // clap handles the conflicts
    };

    if offset >= text.len() {
        anyhow::bail!(
            "Offset {} is out of bounds (file size: {} bytes)",
            offset,
            text.len()
        );
    }

    let index = XmlIndex::build(&text)
        .with_context(|| format!("Failed to parse XML file: {}", args.file.display()))?;

    let result = locate_offset_detailed(&index, &text, offset)
        .with_context(|| format!("Could not locate position at offset {offset}"))?;

    match args.format {
        LocateFormat::Expression => {
            println!("{}", result.expression);
        }
        LocateFormat::Json => {
            let json = serde_json::json!({
                "expression": result.expression,
                "type": result.value_type,
                "byte_range": [result.byte_range.0, result.byte_range.1],
            });
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
    }

    Ok(0)
}
