//! Text processing utilities.
//!
//! This module provides utilities for text processing, including UTF-8
//! validation and line/column lookup.
//!
//! ## Line/column lookup
//!
//! [`LineIndex`](crate::text::LineIndex) maps byte offsets to 1-indexed
//! line/column positions and back,
//! storing line starts Elias-Fano encoded so the index scales with the number
//! of lines rather than the size of the text.
//!
//! ```
//! use succinctly::text::LineIndex;
//!
//! let index = LineIndex::build(b"line1\nline2");
//! assert_eq!(index.to_line_column(6), (2, 1));
//! assert_eq!(index.to_offset(2, 1), Some(6));
//! ```
//!
//! ## UTF-8 Validation
//!
//! The [`utf8`](crate::text::utf8) module provides high-performance UTF-8 validation with detailed
//! error reporting including byte offset, line number, and column position.
//!
//! [`validate_utf8`](crate::text::validate_utf8) picks the fastest engine for
//! the target: an AVX2 kernel on x86_64 with runtime feature detection, and a
//! portable broadword (SWAR) scan everywhere else. Both are accept scans that
//! defer to [`validate_utf8_scalar`](crate::text::validate_utf8_scalar) — the
//! reference implementation — for the exact error, so diagnostics do not
//! depend on which engine ran.
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

pub(crate) mod line_break;
pub mod lines;
pub mod utf8;

// Re-export commonly used types. The broadword engine is portable, so unlike
// the AVX2 path below it needs no `cfg`.
pub use lines::LineIndex;
pub use utf8::{
    validate_utf8, validate_utf8_broadword, validate_utf8_scalar, Utf8Error, Utf8ErrorKind,
};

// The AVX2 fast path is only present on x86_64 when runtime feature detection
// is available (the `std` feature, or under test).
#[cfg(all(target_arch = "x86_64", any(test, feature = "std")))]
pub use utf8::validate_utf8_simd;
