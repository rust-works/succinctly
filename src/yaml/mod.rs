//! YAML semi-indexing for succinct YAML parsing (Phase 4: Anchors & Aliases).
//!
//! This module provides semi-indexing for YAML 1.2 documents, enabling efficient
//! navigation using rank/select operations on the balanced parentheses (BP) tree.
//!
//! # Phase 4 Scope
//!
//! - Block mappings and sequences
//! - Flow mappings `{key: value}` and sequences `[a, b, c]`
//! - Nested flow containers (e.g., `{users: [{name: Alice}]}`)
//! - Simple scalars (unquoted, double-quoted, single-quoted)
//! - Block scalars: literal (`|`) and folded (`>`)
//! - Chomping modifiers: strip (`-`), keep (`+`), clip (default)
//! - **Anchors (`&name`) and aliases (`*name`)**
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
//! // Flow style
//! let yaml_flow = b"person: {name: Alice, age: 30}";
//! let index_flow = YamlIndex::build(yaml_flow)?;
//!
//! // Anchor and alias
//! let yaml_anchor = b"default: &def value\nref: *def";
//! let index_anchor = YamlIndex::build(yaml_anchor)?;
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
//! The oracle handles block style (indentation-based), flow style
//! (bracket-based like JSON), anchors, aliases, and block scalars uniformly.

mod comment;
mod error;
mod index;
mod light;
mod locate;
mod parser;
mod simd;

pub use comment::{CommentEntry, CommentIndex, CommentKind, TagIndex};
pub use error::YamlError;
pub use index::YamlIndex;
pub use light::{
    ChompingIndicator, YamlCursor, YamlElements, YamlField, YamlFields, YamlNumber, YamlString,
    YamlValue,
};
pub use locate::locate_offset;
