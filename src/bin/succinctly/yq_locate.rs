//! yq-locate command implementation.
//!
//! Finds the yq expression that navigates to a specific position in a YAML file.

use anyhow::{Context, Result};
use clap::ValueEnum;
use std::path::PathBuf;

use succinctly::text::LineIndex;
use succinctly::yaml::{locate_offset_detailed, YamlIndex};

/// Arguments for yq-locate command
#[derive(Debug, clap::Parser)]
pub struct YqLocateArgs {
    /// Input YAML file
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

/// Output format for yq-locate
#[derive(Debug, Clone, Default, ValueEnum)]
pub enum LocateFormat {
    /// Just the yq expression (default)
    #[default]
    Expression,
    /// JSON object with expression, type, and byte range
    Json,
}

/// Run the yq-locate command
pub fn run_yq_locate(args: YqLocateArgs) -> Result<i32> {
    // Read the file
    let text = std::fs::read(&args.file)
        .with_context(|| format!("Failed to read file: {}", args.file.display()))?;

    // Unlike `yq`/`yq --validate`/`at_offset` (which all reject invalid UTF-8
    // at the input boundary, #1391/#1242), `yq-locate` keeps answering on an
    // encoding-invalid document instead of refusing outright (#1627): asking
    // "what's at byte N" of a file a parser just rejected is the normal case
    // for a diagnostic tool, not an edge case, so the tool stays useful right
    // where it's needed most. A stderr warning stops the silent
    // self-disagreement with the rest of the yq surface without losing that.
    if let Err(err) = succinctly::text::utf8::validate_utf8(&text) {
        eprintln!("Warning: input is not valid UTF-8: {err}");
    }

    // Determine the byte offset
    let offset = match (args.offset, args.line, args.column) {
        (Some(off), None, None) => off,
        (None, Some(line), Some(column)) => {
            // Build the line index to convert line/column to offset
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

    // Validate offset
    if offset >= text.len() {
        anyhow::bail!(
            "Offset {} is out of bounds (file size: {} bytes)",
            offset,
            text.len()
        );
    }

    // Build the YAML index
    let index = YamlIndex::build(&text)
        .with_context(|| format!("Failed to parse YAML file: {}", args.file.display()))?;

    // Locate the position
    let result = locate_offset_detailed(&index, &text, offset)
        .with_context(|| format!("Could not locate position at offset {offset}"))?;

    // Output
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
