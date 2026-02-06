//! Text processing utilities.
//!
//! This module provides utilities for text processing, including UTF-8 validation.
//!
//! ## UTF-8 Validation
//!
//! The [`utf8`] module provides high-performance UTF-8 validation with detailed
//! error reporting including byte offset, line number, and column position.
//!
//! ```
//! use succinctly::text::utf8::{validate_utf8, Utf8Error, Utf8ErrorKind};
//!
//! // Valid UTF-8
//! assert!(validate_utf8(b"Hello, world!").is_ok());
//! assert!(validate_utf8("日本語".as_bytes()).is_ok());
//!
//! // Invalid UTF-8 (bare continuation byte)
//! let result = validate_utf8(&[0x80]);
//! assert!(result.is_err());
//! let err = result.unwrap_err();
//! assert_eq!(err.kind, Utf8ErrorKind::InvalidLeadByte);
//! assert_eq!(err.offset, 0);
//! ```

pub mod utf8;

// Re-export commonly used types
pub use utf8::{validate_utf8, Utf8Error, Utf8ErrorKind};
