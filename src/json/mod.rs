//! JSON semi-indexing for succinct JSON parsing.
//!
//! This module provides semi-indexing algorithms that convert JSON text into
//! Interest Bits (IB) and Balanced Parentheses (BP) vectors, enabling efficient
//! navigation of JSON structure using rank/select operations.
//!
//! Two cursor types are provided:
//! - [`simple`]: 3-state machine, marks all structural characters
//! - [`standard`]: 4-state machine, marks structural characters and value starts
//!
//! The [`light`] module provides a lazy JSON navigation API that avoids full parsing.
//!
//! SIMD-accelerated versions are available on supported platforms:
//! - [`simd`]: ARM NEON acceleration (16 bytes at a time)

mod bit_writer;
pub mod light;
pub mod simple;
pub mod standard;

#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
pub mod simd;

pub use bit_writer::BitWriter;
pub use light::{JsonIndex, LightJson};
