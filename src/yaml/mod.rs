//! YAML semi-indexing for succinct YAML parsing (Phase 1: YAML-lite).
//!
//! This module provides semi-indexing for YAML 1.2 documents, enabling efficient
//! navigation using rank/select operations on the balanced parentheses (BP) tree.
//!
//! # Phase 1 Scope (YAML-lite)
//!
//! - Block mappings and sequences only (no flow style `{}` `[]`)
//! - Simple scalars (unquoted, double-quoted, single-quoted)
//! - Comments (ignored)
//! - Single document only
//!
//! # Example
//!
//! ```ignore
//! use succinctly::yaml::{YamlIndex, YamlCursor};
//!
//! let yaml = b"name: Alice\nage: 30";
//! let index = YamlIndex::build(yaml)?;
//! let root = index.root(yaml);
//!
//! // Navigate to first child (the key "name")
//! if let Some(child) = root.first_child() {
//!     // ...
//! }
//! ```
//!
//! # Architecture
//!
//! YAML parsing uses an oracle + index model:
//!
//! 1. **Oracle** (sequential): Resolves YAML's context-sensitive grammar,
//!    tracks indentation, and emits IB/BP/TY bits.
//!
//! 2. **Semi-Index** (O(1) queries): Once built, navigation uses only the
//!    BP tree structure without re-parsing.
//!
//! Unlike JSON where brackets define structure explicitly, YAML uses indentation.
//! The oracle converts indentation changes to virtual brackets in the BP index.

mod error;
mod index;
mod light;
mod locate;
mod parser;

pub use error::YamlError;
pub use index::YamlIndex;
pub use light::{
    YamlCursor, YamlElements, YamlField, YamlFields, YamlNumber, YamlString, YamlValue,
};
pub use locate::locate_offset;
