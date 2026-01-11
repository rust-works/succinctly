//! Configuration for DSV parsing.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Configuration for DSV parsing.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DsvConfig {
    /// Field delimiter (default: b',')
    pub delimiter: u8,
    /// Quote character (default: b'"')
    pub quote_char: u8,
    /// Record delimiter (default: b'\n')
    pub newline: u8,
    /// Select sample rate for BitVec (default: 256)
    pub select_sample_rate: u32,
}

impl Default for DsvConfig {
    fn default() -> Self {
        Self {
            delimiter: b',',
            quote_char: b'"',
            newline: b'\n',
            select_sample_rate: 256,
        }
    }
}

impl DsvConfig {
    /// Create a CSV configuration (comma-separated).
    pub fn csv() -> Self {
        Self::default()
    }

    /// Create a TSV configuration (tab-separated).
    pub fn tsv() -> Self {
        Self {
            delimiter: b'\t',
            ..Self::default()
        }
    }

    /// Create a PSV configuration (pipe-separated).
    pub fn psv() -> Self {
        Self {
            delimiter: b'|',
            ..Self::default()
        }
    }

    /// Set the field delimiter.
    pub fn with_delimiter(mut self, delimiter: u8) -> Self {
        self.delimiter = delimiter;
        self
    }

    /// Set the quote character.
    pub fn with_quote_char(mut self, quote_char: u8) -> Self {
        self.quote_char = quote_char;
        self
    }

    /// Set the select sample rate for the index.
    pub fn with_select_sample_rate(mut self, rate: u32) -> Self {
        self.select_sample_rate = rate;
        self
    }
}
