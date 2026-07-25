//! CLI handler for the `yaml validate` command.
//!
//! Mirrors `json validate` (see `json_validate.rs`) but for the opt-in YAML
//! validator. Unlike the JSON handler, the error header is rendered straight
//! from the error kind's `Display` impl rather than a hand-copied `match`, so
//! there is a single source of truth for the message text.

use anyhow::{Context, Result};
use clap::Parser;
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;
use succinctly::yaml::validate::{validate, YamlValidationError, YamlValidationErrorKind};

/// Validate YAML files strictly (opt-in; the default loader does not validate).
#[derive(Debug, Parser)]
pub struct ValidateArgs {
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
    /// YAML is valid.
    pub const SUCCESS: i32 = 0;
    /// YAML is invalid (validation error).
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

/// Run the validate command.
pub fn run(args: ValidateArgs) -> Result<i32> {
    let use_color = if args.no_color {
        false
    } else if args.color {
        true
    } else {
        io::stderr().is_terminal()
    };

    let scheme = ColorScheme::new(use_color);

    if args.files.is_empty() {
        let mut input = Vec::new();
        io::stdin()
            .read_to_end(&mut input)
            .context("failed to read from stdin")?;

        validate_input(&input, None, &args, &scheme)
    } else {
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
    args: &ValidateArgs,
    scheme: &ColorScheme,
) -> Result<i32> {
    match validate(input) {
        Ok(()) => Ok(exit_codes::SUCCESS),
        Err(err) => {
            if !args.quiet {
                print_error(&err, input, filename, scheme);
            }
            Ok(exit_codes::INVALID)
        }
    }
}

/// Print a formatted error message with a rustc-style context snippet.
fn print_error(
    err: &YamlValidationError,
    input: &[u8],
    filename: Option<&str>,
    scheme: &ColorScheme,
) {
    let pos = &err.position;

    // Single source of truth for the message: the error kind's Display impl.
    eprintln!("{}error{}: {}", scheme.error, scheme.reset, err.kind);

    let location = match filename {
        Some(f) => format!("{}:{}:{}", f, pos.line, pos.column),
        None => format!("<stdin>:{}:{}", pos.line, pos.column),
    };
    eprintln!("  {}--> {}{}", scheme.location, location, scheme.reset);

    if let Some(snippet) = get_error_snippet(input, pos.line, pos.column) {
        let line_num_width = pos.line.to_string().len().max(3);
        let blank_padding = " ".repeat(line_num_width + 2);

        eprintln!("{}{}|{}", blank_padding, scheme.line_num, scheme.reset);
        eprintln!(
            " {}{:>width$}{} {}|{} {}",
            scheme.line_num,
            pos.line,
            scheme.reset,
            scheme.line_num,
            scheme.reset,
            snippet.line_content,
            width = line_num_width
        );

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

/// A short, actionable hint for certain error kinds, shown after the caret.
/// This is a hint only — the message itself comes from `Display` (see above).
fn format_error_hint(kind: &YamlValidationErrorKind, scheme: &ColorScheme) -> String {
    let hint = match kind {
        YamlValidationErrorKind::InvalidEscape { .. } => Some("not a valid YAML escape"),
        YamlValidationErrorKind::TabInIndentation => Some("use spaces, not tabs, to indent"),
        YamlValidationErrorKind::NestedMappingKey => {
            Some("a mapping value cannot itself be a mapping on one line")
        }
        YamlValidationErrorKind::MultilineImplicitKey => {
            Some("an implicit key must fit on a single line")
        }
        YamlValidationErrorKind::UnexpectedFlowComma => Some("remove the extra comma"),
        YamlValidationErrorKind::MisplacedDirective => {
            Some("a directive must directly precede a `---` document start")
        }
        YamlValidationErrorKind::CommentNotSeparated => Some("put a space before `#`"),
        _ => None,
    };

    match hint {
        Some(h) => format!(" {}{}{}", scheme.message, h, scheme.reset),
        None => String::new(),
    }
}

/// Information about an error snippet.
struct ErrorSnippet {
    line_content: String,
    caret_offset: usize,
    caret_width: usize,
}

/// Extract a snippet of context around an error position.
fn get_error_snippet(input: &[u8], line: usize, column: usize) -> Option<ErrorSnippet> {
    let text = String::from_utf8_lossy(input);

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

    if current_line != line && line > 1 {
        return None;
    }

    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |i| line_start + i);
    let line_content = &text[line_start..line_end];

    let max_width = 80;
    let (display_content, caret_offset) = if line_content.len() > max_width {
        let error_col = column.saturating_sub(1);
        if error_col < max_width / 2 {
            let truncated = &line_content[..max_width.min(line_content.len())];
            (format!("{truncated}..."), error_col)
        } else if error_col >= line_content.len().saturating_sub(max_width / 2) {
            let start = line_content.len().saturating_sub(max_width);
            let truncated = &line_content[start..];
            let pos_in_truncated = error_col.saturating_sub(start);
            (format!("...{truncated}"), pos_in_truncated + 3)
        } else {
            let start = error_col.saturating_sub(max_width / 2);
            let end = (start + max_width).min(line_content.len());
            let truncated = &line_content[start..end];
            let pos_in_truncated = error_col.saturating_sub(start);
            (format!("...{truncated}..."), pos_in_truncated + 3)
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
