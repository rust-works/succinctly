//! JSON semi-indexing for succinct JSON parsing.
//!
//! This module provides semi-indexing algorithms that convert JSON text into
//! Interest Bits (IB) and Balanced Parentheses (BP) vectors, enabling efficient
//! navigation of JSON structure using rank/select operations.
//!
//! Two cursor types are provided:
//! - [`simple`]: 3-state machine, marks all structural characters
//! - [`standard`]: 4-state machine, marks structural characters and value starts

mod bit_writer;
pub mod simple;
pub mod standard;

pub use bit_writer::BitWriter;
