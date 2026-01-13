//! YAML semi-indexing for succinct YAML parsing (Phase 2: YAML with flow style).
//!
//! This module provides semi-indexing for YAML 1.2 documents, enabling efficient
//! navigation using rank/select operations on the balanced parentheses (BP) tree.
//!
//! # Phase 2 Scope
//!
//! - Block mappings and sequences
//! - **Flow mappings `{key: value}` and sequences `[a, b, c]`**
//! - Nested flow containers (e.g., `{users: [{name: Alice}]}`)
//! - Simple scalars (unquoted, double-quoted, single-quoted)
//! - Comments (ignored in block context)
//! - Single document only
//!
//! # Example
//!
//! ```ignore
//! use succinctly::yaml::{YamlIndex, YamlValue};
//!
//! // Block style
//! let yaml = b"name: Alice\nage: 30";
//! let index = YamlIndex::build(yaml)?;
//! let root = index.root(yaml);
//!
//! // Flow style also works
//! let yaml_flow = b"person: {name: Alice, age: 30}";
//! let index_flow = YamlIndex::build(yaml_flow)?;
//! ```
//!
//! # Architecture
//!
//! YAML parsing uses an oracle + index model:
//!
//! 1. **Oracle** (sequential): Resolves YAML's context-sensitive grammar,
//!    tracks indentation/flow context, and emits IB/BP/TY bits.
//!
//! 2. **Semi-Index** (O(1) queries): Once built, navigation uses only the
//!    BP tree structure without re-parsing.
//!
//! The oracle handles both block style (indentation-based) and flow style
//! (bracket-based like JSON) constructs uniformly.

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
