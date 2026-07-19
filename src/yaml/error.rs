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

    /// Nesting depth of flow collections / inline sequence items exceeded the cap.
    NestingTooDeep {
        /// Byte offset where the limit was exceeded
        offset: usize,
        /// The configured depth limit
        limit: usize,
    },
}

impl fmt::Display for YamlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIndentation {
                line,
                expected,
                found,
            } => {
                write!(
                    f,
                    "invalid indentation at line {line}: expected {expected} spaces, found {found}"
                )
            }
            Self::TabIndentation { line, offset } => {
                write!(
                    f,
                    "tab character used for indentation at line {line} (offset {offset})"
                )
            }
            Self::UnexpectedCharacter {
                offset,
                char,
                context,
            } => {
                write!(
                    f,
                    "unexpected character '{char}' at offset {offset}: {context}"
                )
            }
            Self::UnclosedQuote {
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
            Self::InvalidEscape { offset, sequence } => {
                write!(f, "invalid escape sequence '{sequence}' at offset {offset}")
            }
            Self::InvalidUtf8 { offset } => {
                write!(f, "invalid UTF-8 sequence at offset {offset}")
            }
            #[allow(deprecated)]
            // STYLE-0004: Display arm for a deprecated error variant kept for back-compat
            Self::MultiDocumentNotSupported { offset } => {
                write!(
                    f,
                    "multi-document YAML not supported (found `---` at offset {offset})"
                )
            }
            #[allow(deprecated)]
            // STYLE-0004: Display arm for a deprecated error variant kept for back-compat
            Self::FlowStyleNotSupported { offset, char } => {
                write!(f, "flow style '{char}' not supported at offset {offset}")
            }
            Self::InvalidAnchorName { offset, reason } => {
                write!(f, "invalid anchor name at offset {offset}: {reason}")
            }
            Self::DuplicateAnchor { offset, name } => {
                write!(
                    f,
                    "duplicate anchor '{name}' at offset {offset} (previously defined)"
                )
            }
            Self::ExplicitKeyNotSupported { offset } => {
                write!(f, "explicit keys (?) not supported at offset {offset}")
            }
            Self::TagNotSupported { offset } => {
                write!(f, "tags (!) not supported at offset {offset}")
            }
            Self::EmptyInput => {
                write!(f, "empty input")
            }
            Self::ColonWithoutSpace { offset } => {
                write!(
                    f,
                    "colon at offset {offset} must be followed by space or newline"
                )
            }
            Self::KeyWithoutValue { offset, line } => {
                write!(f, "key without value at line {line} (offset {offset})")
            }
            Self::UnexpectedEof { context } => {
                write!(f, "unexpected end of input: {context}")
            }
            Self::NestingTooDeep { offset, limit } => {
                write!(
                    f,
                    "nesting depth exceeds limit of {limit} at offset {offset}"
                )
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

    #[test]
    fn test_unclosed_single_quote_display() {
        // The non-double branch of UnclosedQuote reports "single".
        let err = YamlError::UnclosedQuote {
            start_offset: 7,
            quote_type: '\'',
        };
        assert_eq!(
            err.to_string(),
            "unclosed single quote starting at offset 7"
        );
    }

    #[test]
    fn test_unexpected_character_display() {
        let err = YamlError::UnexpectedCharacter {
            offset: 12,
            char: '@',
            context: "in mapping value",
        };
        assert_eq!(
            err.to_string(),
            "unexpected character '@' at offset 12: in mapping value"
        );
    }

    #[test]
    fn test_invalid_escape_display() {
        let err = YamlError::InvalidEscape {
            offset: 4,
            sequence: "\\q".to_string(),
        };
        assert_eq!(err.to_string(), "invalid escape sequence '\\q' at offset 4");
    }

    #[test]
    fn test_invalid_utf8_display() {
        let err = YamlError::InvalidUtf8 { offset: 9 };
        assert_eq!(err.to_string(), "invalid UTF-8 sequence at offset 9");
    }

    #[test]
    fn test_invalid_anchor_name_display() {
        let err = YamlError::InvalidAnchorName {
            offset: 2,
            reason: "empty name",
        };
        assert_eq!(
            err.to_string(),
            "invalid anchor name at offset 2: empty name"
        );
    }

    #[test]
    fn test_duplicate_anchor_display() {
        let err = YamlError::DuplicateAnchor {
            offset: 15,
            name: "base".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "duplicate anchor 'base' at offset 15 (previously defined)"
        );
    }

    #[test]
    fn test_explicit_key_not_supported_display() {
        let err = YamlError::ExplicitKeyNotSupported { offset: 6 };
        assert_eq!(
            err.to_string(),
            "explicit keys (?) not supported at offset 6"
        );
    }

    #[test]
    fn test_tag_not_supported_display() {
        let err = YamlError::TagNotSupported { offset: 8 };
        assert_eq!(err.to_string(), "tags (!) not supported at offset 8");
    }

    #[test]
    fn test_empty_input_display() {
        assert_eq!(YamlError::EmptyInput.to_string(), "empty input");
    }

    #[test]
    fn test_colon_without_space_display() {
        let err = YamlError::ColonWithoutSpace { offset: 11 };
        assert_eq!(
            err.to_string(),
            "colon at offset 11 must be followed by space or newline"
        );
    }

    #[test]
    fn test_key_without_value_display() {
        let err = YamlError::KeyWithoutValue { offset: 5, line: 2 };
        assert_eq!(err.to_string(), "key without value at line 2 (offset 5)");
    }

    #[test]
    fn test_unexpected_eof_display() {
        let err = YamlError::UnexpectedEof {
            context: "while parsing flow sequence",
        };
        assert_eq!(
            err.to_string(),
            "unexpected end of input: while parsing flow sequence"
        );
    }

    #[test]
    #[allow(deprecated)] // STYLE-0004: test intentionally exercises deprecated variants' Display arms
    fn test_deprecated_variant_display() {
        // Deprecated but still part of the enum and its Display arms.
        let err = YamlError::MultiDocumentNotSupported { offset: 0 };
        assert_eq!(
            err.to_string(),
            "multi-document YAML not supported (found `---` at offset 0)"
        );

        let err = YamlError::FlowStyleNotSupported {
            offset: 3,
            char: '[',
        };
        assert_eq!(err.to_string(), "flow style '[' not supported at offset 3");
    }

    #[test]
    fn test_error_is_clone_and_eq() {
        // Exercises the derived Clone/PartialEq used throughout the parser.
        let err = YamlError::KeyWithoutValue { offset: 5, line: 2 };
        assert_eq!(err.clone(), err);
        assert_ne!(err, YamlError::EmptyInput);
    }
}
