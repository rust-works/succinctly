//! Succinct tree representations.
//!
//! This module provides succinct encodings for tree structures using
//! balanced parentheses and related representations.
//!
//! # Data Structures
//!
//! - [`BalancedParens`] - Tree encoded as balanced parentheses with O(1) navigation
//!
//! # Example
//!
//! ```
//! use succinctly::trees::BalancedParens;
//!
//! // Tree: ((()())())  encoded as 1110100100
//! let bp = BalancedParens::new(vec![0b0010010111], 10);
//! assert_eq!(bp.find_close(0), Some(9));
//! ```

mod bp;

pub use bp::BalancedParens;
pub use bp::{enclose, find_close, find_close_in_word, find_open, find_unmatched_close_in_word};
pub use bp::{NoSelect, SelectSupport, WithSelect};
