//! JSON semi-indexing for succinct JSON parsing.
//!
//! This module provides semi-indexing algorithms that convert JSON text into
//! Interest Bits (IB) and Balanced Parentheses (BP) vectors, enabling efficient
//! navigation of JSON structure using rank/select operations.
//!
//! Two cursor types are provided:
//! - `simple`: 3-state machine, marks all structural characters
//! - `standard`: 4-state machine, marks structural characters and value starts
//!
//! The `light` module provides a lazy JSON navigation API using the standard cursor.
//!
//! SIMD-accelerated versions are available on supported platforms (x86_64, aarch64):
//! - `simd`: Platform-specific SIMD acceleration (AVX2, NEON, etc.)

mod bit_writer;
pub mod light;
mod pfsm_optimized;
pub mod pfsm_tables;
pub mod simple;
pub mod simple_light;
pub mod standard;

#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
pub mod simd;

pub use bit_writer::BitWriter;
pub use light::{JsonIndex, StandardJson};
pub use simple_light::SimpleJsonIndex;
