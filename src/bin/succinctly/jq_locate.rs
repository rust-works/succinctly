//! jq-locate command implementation.
//!
//! Finds the jq expression that navigates to a specific position in a JSON file.

use anyhow::{Context, Result};
use clap::ValueEnum;
use std::path::PathBuf;

use succinctly::json::light::JsonIndex;
use succinctly::json::locate::locate_offset_detailed;
use succinctly::text::LineIndex;

/// Arguments for jq-locate command
#[derive(Debug, clap::Parser)]
pub struct JqLocateArgs {
    /// Input JSON file
    pub file: PathBuf,

    /// Byte offset in file (0-indexed), counted after a leading UTF-8 byte-order mark
    #[arg(long, conflicts_with_all = ["line", "column"])]
    pub offset: Option<usize>,

    /// Line number (1-indexed)
    #[arg(long, requires = "column")]
    pub line: Option<usize>,

    /// Column number (1-indexed, byte offset within line; line 1 starts after a leading BOM)
    #[arg(long, requires = "line")]
    pub column: Option<usize>,

    /// Output format
    #[arg(long, default_value = "expression")]
    pub format: LocateFormat,
}

/// Output format for jq-locate
#[derive(Debug, Clone, Default, ValueEnum)]
pub enum LocateFormat {
    /// Just the jq expression (default)
    #[default]
    Expression,
    /// JSON object with expression, type, and byte range
    Json,
}

/// Run the jq-locate command
pub fn run_jq_locate(args: JqLocateArgs) -> Result<i32> {
    // Read the file
    let file_bytes = std::fs::read(&args.file)
        .with_context(|| format!("Failed to read file: {}", args.file.display()))?;

    // #5313: a leading UTF-8 byte-order mark is not part of the document. `succinctly jq` drops it
    // before indexing (#5291), so `at_offset(n)` and `at_position` count from the first byte after
    // it, and so does this: the same `n` names the same node in both. Editors show the same text.
    let bom_len = crate::cli::front_matter::leading_bom_len(&file_bytes);
    let text = &file_bytes[bom_len..];
    // #7425: the line index and the JSON index each hold a text of at most `u32::MAX` bytes.
    if !JsonIndex::can_index(text.len()) {
        anyhow::bail!(
            "{} is {} bytes: jq-locate reads a document of at most {} bytes (4 GiB)",
            args.file.display(),
            text.len(),
            JsonIndex::MAX_TEXT_LEN
        );
    }

    // Determine the byte offset
    let offset = match (args.offset, args.line, args.column) {
        (Some(off), None, None) => off,
        (None, Some(line), Some(column)) => {
            // Build the line index to convert line/column to offset
            let line_index = LineIndex::build(text);
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
        let after_bom = if bom_len > 0 {
            format!(
                "; offsets count from after the {bom_len}-byte byte-order mark, so the document is {} bytes",
                text.len()
            )
        } else {
            String::new()
        };
        anyhow::bail!(
            "Offset {} is out of bounds (file size: {} bytes{after_bom})",
            offset,
            file_bytes.len()
        );
    }

    // Build the JSON index
    let index = JsonIndex::build(text);

    // Locate the position
    let result = locate_offset_detailed(&index, text, offset)
        .with_context(|| format!("Could not locate position at offset {offset}"))?;

    // Output
    match args.format {
        LocateFormat::Expression => {
            println!("{}", result.expression);
        }
        LocateFormat::Json => {
            let mut json = serde_json::json!({
                "expression": result.expression,
                "type": result.value_type,
                "byte_range": [result.byte_range.0, result.byte_range.1],
            });
            // `byte_range` counts from after the mark; a caller slicing the file adds this back.
            if bom_len > 0 {
                json["bom_length"] = serde_json::json!(bom_len);
            }
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
    }

    Ok(0)
}
