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

    /// Alias reference that would make an anchor's value contain itself.
    AliasCycle {
        /// Byte offset of the alias (`*name`) that closes the cycle
        offset: usize,
        /// The referenced anchor name
        name: String,
    },

    /// Alias referencing an anchor that is not in scope, including a forward
    /// reference (YAML 1.2 §7.1 requires an alias to name a *previous* anchor).
    UnknownAnchor {
        /// Byte offset of the alias (`*name`)
        offset: usize,
        /// The referenced anchor name
        name: String,
    },

    /// An alias node (`*name`) was itself decorated with an anchor or tag
    /// (`&other *name` / `!!str *name`). Per the YAML 1.2 grammar an alias
    /// node carries no node properties of its own - real yq and PyYAML both
    /// reject this (#1374).
    PropertyOnAlias {
        /// Byte offset of the alias (`*name`)
        offset: usize,
        /// The referenced anchor name
        name: String,
    },

    /// Tag not supported - kept for backwards compatibility but no longer used.
    #[deprecated(note = "Tags are now supported (#224)")]
    TagNotSupported {
        /// Byte offset of the `!`
        offset: usize,
    },

    /// Malformed tag syntax (e.g. an unterminated verbatim `!<...>` tag).
    ///
    /// A bare `!` alone is the YAML 1.2 non-specific tag and is valid, not an
    /// error - this variant is only for syntax the tag lexer cannot make
    /// sense of at all.
    InvalidTag {
        /// Byte offset of the `!`
        offset: usize,
        /// Reason for invalidity
        reason: &'static str,
    },

    /// Empty input.
    EmptyInput,

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

    /// Input exceeds the maximum indexable size (`u32::MAX` bytes).
    ///
    /// Text positions are stored as `u32` in the semi-index (#188), so larger
    /// inputs would silently truncate offsets instead of failing cleanly.
    InputTooLarge {
        /// Actual input length in bytes
        len: usize,
    },

    /// A line's indentation falls in the ambiguous gap between an open
    /// container's own indent and its immediate parent's indent, with no
    /// unambiguous interpretation for which container it belongs to (#900,
    /// #901). Real yq rejects the same inputs (`did not find expected key`
    /// / `did not find expected '-' indicator`) rather than guessing.
    InconsistentIndentation {
        /// Byte offset where the misindented line starts
        offset: usize,
        /// Line number
        line: usize,
    },
}

impl fmt::Display for YamlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
            Self::AliasCycle { offset, name } => {
                write!(
                    f,
                    "cyclic alias '{name}' at offset {offset} (anchor value contains itself)"
                )
            }
            Self::UnknownAnchor { offset, name } => {
                write!(f, "unknown anchor '{name}' referenced at offset {offset}")
            }
            Self::PropertyOnAlias { offset, name } => {
                write!(
                    f,
                    "alias '*{name}' at offset {offset} cannot carry an anchor or tag (an alias node has no node properties)"
                )
            }
            #[allow(deprecated)]
            // STYLE-0004: Display arm for a deprecated error variant kept for back-compat
            Self::TagNotSupported { offset } => {
                write!(f, "tags (!) not supported at offset {offset}")
            }
            Self::InvalidTag { offset, reason } => {
                write!(f, "invalid tag at offset {offset}: {reason}")
            }
            Self::EmptyInput => {
                write!(f, "empty input")
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
            Self::InputTooLarge { len } => {
                write!(
                    f,
                    "input too large: {len} bytes exceeds the u32::MAX-byte (4 GiB) indexing limit"
                )
            }
            Self::InconsistentIndentation { offset, line } => {
                write!(
                    f,
                    "inconsistent indentation at line {line} (offset {offset})"
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
    fn test_alias_cycle_display() {
        let err = YamlError::AliasCycle {
            offset: 20,
            name: "anchor".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "cyclic alias 'anchor' at offset 20 (anchor value contains itself)"
        );
    }

    #[test]
    fn test_property_on_alias_display() {
        let err = YamlError::PropertyOnAlias {
            offset: 11,
            name: "a0".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "alias '*a0' at offset 11 cannot carry an anchor or tag (an alias node has no node properties)"
        );
    }

    #[test]
    fn test_unknown_anchor_display() {
        let err = YamlError::UnknownAnchor {
            offset: 3,
            name: "nope".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "unknown anchor 'nope' referenced at offset 3"
        );
    }

    #[test]
    #[allow(deprecated)] // STYLE-0004: test intentionally exercises a deprecated variant's Display arm
    fn test_tag_not_supported_display() {
        let err = YamlError::TagNotSupported { offset: 8 };
        assert_eq!(err.to_string(), "tags (!) not supported at offset 8");
    }

    #[test]
    fn test_invalid_tag_display() {
        let err = YamlError::InvalidTag {
            offset: 4,
            reason: "unterminated verbatim tag",
        };
        assert_eq!(
            err.to_string(),
            "invalid tag at offset 4: unterminated verbatim tag"
        );
    }

    #[test]
    fn test_empty_input_display() {
        assert_eq!(YamlError::EmptyInput.to_string(), "empty input");
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
    fn test_input_too_large_display() {
        let err = YamlError::InputTooLarge { len: 5_000_000_000 };
        assert_eq!(
            err.to_string(),
            "input too large: 5000000000 bytes exceeds the u32::MAX-byte (4 GiB) indexing limit"
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
    fn test_inconsistent_indentation_display() {
        let err = YamlError::InconsistentIndentation {
            offset: 17,
            line: 2,
        };
        assert_eq!(
            err.to_string(),
            "inconsistent indentation at line 2 (offset 17)"
        );
    }

    #[test]
    fn test_error_is_clone_and_eq() {
        // Exercises the derived Clone/PartialEq used throughout the parser.
        let err = YamlError::KeyWithoutValue { offset: 5, line: 2 };
        assert_eq!(err.clone(), err);
        assert_ne!(err, YamlError::EmptyInput);
    }
}
