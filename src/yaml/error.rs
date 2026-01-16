//! YAML parsing errors.
//!
//! Provides detailed error information with byte offsets and line numbers
//! for IDE integration and debugging.

#[cfg(not(test))]
use alloc::string::String;

use core::fmt;

/// Errors that can occur during YAML parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YamlError {
    /// Inconsistent indentation (e.g., mixing 2-space and 4-space).
    InvalidIndentation {
        /// Line number (1-indexed)
        line: usize,
        /// Expected indentation level
        expected: usize,
        /// Actual indentation level found
        found: usize,
    },

    /// Tab character used for indentation (YAML forbids tabs).
    TabIndentation {
        /// Line number where tab was found
        line: usize,
        /// Byte offset in input
        offset: usize,
    },

    /// Unexpected character in the given context.
    UnexpectedCharacter {
        /// Byte offset in input
        offset: usize,
        /// The unexpected character
        char: char,
        /// Description of what was expected
        context: &'static str,
    },

    /// Unclosed quote in a string.
    UnclosedQuote {
        /// Byte offset where the quote started
        start_offset: usize,
        /// The quote character (" or ')
        quote_type: char,
    },

    /// Invalid escape sequence in a double-quoted string.
    InvalidEscape {
        /// Byte offset of the backslash
        offset: usize,
        /// The invalid escape sequence
        sequence: String,
    },

    /// Invalid UTF-8 sequence.
    InvalidUtf8 {
        /// Byte offset where invalid UTF-8 starts
        offset: usize,
    },

    /// Document marker found but multi-document not supported.
    /// Note: Multi-document streams are now supported in Phase 5+.
    #[deprecated(note = "Multi-document streams are now supported in Phase 5+")]
    MultiDocumentNotSupported {
        /// Byte offset of the `---` marker
        offset: usize,
    },

    /// Flow style (`{` or `[`) - kept for backwards compatibility but no longer used.
    #[deprecated(note = "Flow style is now supported in Phase 2+")]
    FlowStyleNotSupported {
        /// Byte offset of the flow character
        offset: usize,
        /// The flow character found
        char: char,
    },

    /// Invalid anchor name (empty or contains invalid characters).
    InvalidAnchorName {
        /// Byte offset of the anchor
        offset: usize,
        /// Reason for invalidity
        reason: &'static str,
    },

    /// Duplicate anchor definition.
    DuplicateAnchor {
        /// Byte offset of the duplicate anchor
        offset: usize,
        /// The anchor name
        name: String,
    },

    /// Explicit key (`?`) not supported.
    ExplicitKeyNotSupported {
        /// Byte offset of the `?`
        offset: usize,
    },

    /// Tag not supported.
    TagNotSupported {
        /// Byte offset of the `!`
        offset: usize,
    },

    /// Empty input.
    EmptyInput,

    /// Colon found without following space (ambiguous).
    ColonWithoutSpace {
        /// Byte offset of the colon
        offset: usize,
    },

    /// Key without value in mapping.
    KeyWithoutValue {
        /// Byte offset where key starts
        offset: usize,
        /// Line number
        line: usize,
    },

    /// Unexpected end of input.
    UnexpectedEof {
        /// What was expected
        context: &'static str,
    },
}

impl fmt::Display for YamlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            YamlError::InvalidIndentation {
                line,
                expected,
                found,
            } => {
                write!(
                    f,
                    "invalid indentation at line {}: expected {} spaces, found {}",
                    line, expected, found
                )
            }
            YamlError::TabIndentation { line, offset } => {
                write!(
                    f,
                    "tab character used for indentation at line {} (offset {})",
                    line, offset
                )
            }
            YamlError::UnexpectedCharacter {
                offset,
                char,
                context,
            } => {
                write!(
                    f,
                    "unexpected character '{}' at offset {}: {}",
                    char, offset, context
                )
            }
            YamlError::UnclosedQuote {
                start_offset,
                quote_type,
            } => {
                write!(
                    f,
                    "unclosed {} quote starting at offset {}",
                    if *quote_type == '"' {
                        "double"
                    } else {
                        "single"
                    },
                    start_offset
                )
            }
            YamlError::InvalidEscape { offset, sequence } => {
                write!(
                    f,
                    "invalid escape sequence '{}' at offset {}",
                    sequence, offset
                )
            }
            YamlError::InvalidUtf8 { offset } => {
                write!(f, "invalid UTF-8 sequence at offset {}", offset)
            }
            #[allow(deprecated)]
            YamlError::MultiDocumentNotSupported { offset } => {
                write!(
                    f,
                    "multi-document YAML not supported (found `---` at offset {})",
                    offset
                )
            }
            #[allow(deprecated)]
            YamlError::FlowStyleNotSupported { offset, char } => {
                write!(
                    f,
                    "flow style '{}' not supported at offset {}",
                    char, offset
                )
            }
            YamlError::InvalidAnchorName { offset, reason } => {
                write!(f, "invalid anchor name at offset {}: {}", offset, reason)
            }
            YamlError::DuplicateAnchor { offset, name } => {
                write!(
                    f,
                    "duplicate anchor '{}' at offset {} (previously defined)",
                    name, offset
                )
            }
            YamlError::ExplicitKeyNotSupported { offset } => {
                write!(f, "explicit keys (?) not supported at offset {}", offset)
            }
            YamlError::TagNotSupported { offset } => {
                write!(f, "tags (!) not supported at offset {}", offset)
            }
            YamlError::EmptyInput => {
                write!(f, "empty input")
            }
            YamlError::ColonWithoutSpace { offset } => {
                write!(
                    f,
                    "colon at offset {} must be followed by space or newline",
                    offset
                )
            }
            YamlError::KeyWithoutValue { offset, line } => {
                write!(f, "key without value at line {} (offset {})", line, offset)
            }
            YamlError::UnexpectedEof { context } => {
                write!(f, "unexpected end of input: {}", context)
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for YamlError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = YamlError::InvalidIndentation {
            line: 5,
            expected: 2,
            found: 4,
        };
        assert_eq!(
            err.to_string(),
            "invalid indentation at line 5: expected 2 spaces, found 4"
        );

        let err = YamlError::TabIndentation {
            line: 3,
            offset: 20,
        };
        assert_eq!(
            err.to_string(),
            "tab character used for indentation at line 3 (offset 20)"
        );

        let err = YamlError::UnclosedQuote {
            start_offset: 10,
            quote_type: '"',
        };
        assert_eq!(
            err.to_string(),
            "unclosed double quote starting at offset 10"
        );
    }
}
