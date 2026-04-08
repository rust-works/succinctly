//! CLI handler for the `text validate utf8` command.

use anyhow::{Context, Result};
use clap::Parser;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use succinctly::text::utf8::{self, Utf8Error, Utf8ErrorKind};

/// Validate text files for UTF-8 compliance.
#[derive(Debug, Parser)]
pub struct ValidateUtf8Args {
    /// Input files to validate (reads from stdin if none provided)
    #[arg(trailing_var_arg = true)]
    pub files: Vec<PathBuf>,

    /// Quiet mode: exit code only, no output
    #[arg(short, long)]
    pub quiet: bool,

    /// Force color output even when not a TTY
    #[arg(short = 'C', long = "color")]
    pub color: bool,

    /// Disable color output
    #[arg(short = 'M', long = "no-color")]
    pub no_color: bool,
}

/// Exit codes for the validate command.
pub mod exit_codes {
    /// UTF-8 is valid.
    pub const SUCCESS: i32 = 0;
    /// UTF-8 is invalid (validation error).
    pub const INVALID: i32 = 1;
    /// I/O error (file not found, permission denied, etc.).
    pub const IO_ERROR: i32 = 2;
}

/// ANSI color codes for error output.
mod colors {
    pub const RESET: &str = "\x1b[0m";
    pub const ERROR: &str = "\x1b[1;31m"; // Bold red
    pub const LOCATION: &str = "\x1b[1;34m"; // Bold blue
    pub const LINE_NUM: &str = "\x1b[0;34m"; // Blue
    pub const CARET: &str = "\x1b[1;32m"; // Bold green
    pub const MESSAGE: &str = "\x1b[0;33m"; // Yellow
}

/// Color scheme that can be disabled.
struct ColorScheme {
    error: &'static str,
    location: &'static str,
    line_num: &'static str,
    caret: &'static str,
    message: &'static str,
    reset: &'static str,
}

impl ColorScheme {
    fn new(use_color: bool) -> Self {
        if use_color {
            Self {
                error: colors::ERROR,
                location: colors::LOCATION,
                line_num: colors::LINE_NUM,
                caret: colors::CARET,
                message: colors::MESSAGE,
                reset: colors::RESET,
            }
        } else {
            Self {
                error: "",
                location: "",
                line_num: "",
                caret: "",
                message: "",
                reset: "",
            }
        }
    }
}

/// Run the validate utf8 command.
pub fn run(args: ValidateUtf8Args) -> Result<i32> {
    // Determine color output
    let use_color = if args.no_color {
        false
    } else if args.color {
        true
    } else {
        atty::is(atty::Stream::Stderr)
    };

    let scheme = ColorScheme::new(use_color);

    if args.files.is_empty() {
        // Read from stdin
        let mut input = Vec::new();
        io::stdin()
            .read_to_end(&mut input)
            .context("failed to read from stdin")?;

        validate_input(&input, None, &args, &scheme)
    } else {
        // Validate each file
        let mut any_invalid = false;
        let mut any_io_error = false;

        for path in &args.files {
            match fs::read(path) {
                Ok(input) => {
                    let filename = path.to_string_lossy();
                    let result = validate_input(&input, Some(&filename), &args, &scheme)?;
                    if result == exit_codes::INVALID {
                        any_invalid = true;
                    }
                }
                Err(e) => {
                    any_io_error = true;
                    if !args.quiet {
                        eprintln!(
                            "{}error{}: {}: {}",
                            scheme.error,
                            scheme.reset,
                            path.display(),
                            e
                        );
                    }
                }
            }
        }

        if any_io_error {
            Ok(exit_codes::IO_ERROR)
        } else if any_invalid {
            Ok(exit_codes::INVALID)
        } else {
            Ok(exit_codes::SUCCESS)
        }
    }
}

/// Validate a single input and print errors.
fn validate_input(
    input: &[u8],
    filename: Option<&str>,
    args: &ValidateUtf8Args,
    scheme: &ColorScheme,
) -> Result<i32> {
    match utf8::validate_utf8(input) {
        Ok(()) => Ok(exit_codes::SUCCESS),
        Err(err) => {
            if !args.quiet {
                print_error(&err, input, filename, scheme);
            }
            Ok(exit_codes::INVALID)
        }
    }
}

/// Print a formatted error message with context snippet.
fn print_error(err: &Utf8Error, input: &[u8], filename: Option<&str>, scheme: &ColorScheme) {
    // Print error header
    eprintln!(
        "{}error{}: {}",
        scheme.error,
        scheme.reset,
        format_error_kind(&err.kind, err.offset, input)
    );

    // Print location
    let location = match filename {
        Some(f) => format!("{}:{}:{}", f, err.line, err.column),
        None => format!("<stdin>:{}:{}", err.line, err.column),
    };
    eprintln!("  {}--> {}{}", scheme.location, location, scheme.reset);

    // Print context snippet
    if let Some(snippet) = get_error_snippet(input, err.line, err.column) {
        // Calculate line number width (minimum 3 chars for alignment)
        let line_num_width = err.line.to_string().len().max(3);
        let blank_padding = " ".repeat(line_num_width + 2);

        // Print blank line with pipe
        eprintln!("{}{}|{}", blank_padding, scheme.line_num, scheme.reset);

        // Print the line with line number
        eprintln!(
            " {}{:>width$}{} {}|{} {}",
            scheme.line_num,
            err.line,
            scheme.reset,
            scheme.line_num,
            scheme.reset,
            snippet.line_content,
            width = line_num_width
        );

        // Print the caret pointer
        let padding = " ".repeat(snippet.caret_offset);
        let carets = "^".repeat(snippet.caret_width.max(1));
        eprintln!(
            "{}{}|{} {}{}{}{}{}",
            blank_padding,
            scheme.line_num,
            scheme.reset,
            padding,
            scheme.caret,
            carets,
            scheme.reset,
            format_error_hint(&err.kind, scheme)
        );
    }

    eprintln!();
}

/// Format the error kind as a human-readable message.
fn format_error_kind(kind: &Utf8ErrorKind, offset: usize, input: &[u8]) -> String {
    let byte_info = if offset < input.len() {
        format!(" (byte 0x{:02X})", input[offset])
    } else {
        String::new()
    };

    match kind {
        Utf8ErrorKind::InvalidLeadByte => format!("invalid UTF-8 lead byte{}", byte_info),
        Utf8ErrorKind::InvalidContinuationByte => {
            format!("invalid UTF-8 continuation byte{}", byte_info)
        }
        Utf8ErrorKind::OverlongEncoding => "overlong UTF-8 encoding".to_string(),
        Utf8ErrorKind::SurrogateCodepoint => "surrogate codepoint in UTF-8".to_string(),
        Utf8ErrorKind::OutOfRangeCodepoint => "codepoint above U+10FFFF".to_string(),
        Utf8ErrorKind::TruncatedSequence => "truncated UTF-8 sequence at end of input".to_string(),
    }
}

/// Format an additional hint for certain error types.
fn format_error_hint(kind: &Utf8ErrorKind, scheme: &ColorScheme) -> String {
    let hint = match kind {
        Utf8ErrorKind::InvalidLeadByte => Some("bytes 0x80-0xBF are continuation bytes"),
        Utf8ErrorKind::InvalidContinuationByte => Some("expected byte 0x80-0xBF"),
        Utf8ErrorKind::OverlongEncoding => Some("use shortest possible encoding"),
        Utf8ErrorKind::SurrogateCodepoint => Some("U+D800-U+DFFF are reserved for UTF-16"),
        Utf8ErrorKind::OutOfRangeCodepoint => Some("maximum is U+10FFFF"),
        Utf8ErrorKind::TruncatedSequence => None,
    };

    match hint {
        Some(h) => format!(" {}{}{}", scheme.message, h, scheme.reset),
        None => String::new(),
    }
}

/// Information about an error snippet.
struct ErrorSnippet {
    /// The content of the line containing the error.
    line_content: String,
    /// Number of spaces before the caret.
    caret_offset: usize,
    /// Width of the caret (number of ^ characters).
    caret_width: usize,
}

/// Extract a snippet of context around an error position.
fn get_error_snippet(input: &[u8], line: usize, column: usize) -> Option<ErrorSnippet> {
    // Convert input to string (lossy for display)
    let text = String::from_utf8_lossy(input);

    // Find the line containing the error
    let mut current_line = 1;
    let mut line_start = 0;

    for (i, ch) in text.char_indices() {
        if current_line == line {
            line_start = i;
            break;
        }
        if ch == '\n' {
            current_line += 1;
        }
    }

    // If we didn't find the line
    if current_line != line && line > 1 {
        return None;
    }

    // Find the end of this line
    let line_end = text[line_start..]
        .find('\n')
        .map(|i| line_start + i)
        .unwrap_or(text.len());

    let line_content = &text[line_start..line_end];

    // Truncate long lines
    let max_width = 80;
    let (display_content, caret_offset) = if line_content.len() > max_width {
        let error_col = column.saturating_sub(1);
        if error_col < max_width / 2 {
            let truncated = &line_content[..max_width.min(line_content.len())];
            (format!("{}...", truncated), error_col)
        } else if error_col >= line_content.len().saturating_sub(max_width / 2) {
            let start = line_content.len().saturating_sub(max_width);
            let truncated = &line_content[start..];
            let pos_in_truncated = error_col.saturating_sub(start);
            (format!("...{}", truncated), pos_in_truncated + 3)
        } else {
            let start = error_col.saturating_sub(max_width / 2);
            let end = (start + max_width).min(line_content.len());
            let truncated = &line_content[start..end];
            let pos_in_truncated = error_col.saturating_sub(start);
            (format!("...{}...", truncated), pos_in_truncated + 3)
        }
    } else {
        (line_content.to_string(), column.saturating_sub(1))
    };

    Some(ErrorSnippet {
        line_content: display_content,
        caret_offset,
        caret_width: 1,
    })
}
