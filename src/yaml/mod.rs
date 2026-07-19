//! YAML semi-indexing for succinct YAML parsing.
//!
//! This module provides semi-indexing for YAML 1.2 documents, enabling efficient
//! navigation using rank/select operations on the balanced parentheses (BP) tree.
//!
//! # Supported
//!
//! - Block mappings and sequences
//! - Flow mappings `{key: value}` and sequences `[a, b, c]`
//! - Nested flow containers (e.g., `{users: [{name: Alice}]}`)
//! - Simple scalars (unquoted, double-quoted, single-quoted)
//! - Block scalars: literal (`|`) and folded (`>`)
//! - Chomping modifiers: strip (`-`), keep (`+`), clip (default)
//! - Anchors (`&name`) and aliases (`*name`)
//! - Explicit keys (`?` / `:`)
//! - Multi-document streams (`---` / `...`), wrapped in an implicit root sequence
//! - Comments (ignored in block context)
//!
//! # Not supported
//!
//! - Tags (`!!str`, `!custom`, verbatim `!<...>`) — rejected in block context, absorbed
//!   as scalar text in flow context
//! - `%YAML` / `%TAG` directives — parsed as plain scalars
//! - Merge keys (`<<`) — parsed as an ordinary key
//!
//! # Validation
//!
//! `YamlIndex::build` performs minimal validation during indexing (structural recognition
//! only) and accepts many malformed documents. It is a non-validating loader: of the YAML
//! Test Suite's 94 invalid documents it rejects 11. Do not rely on a parse error to
//! detect malformed input. See `docs/compliance/yaml/limitations.md`.
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

mod advance_positions;
mod end_positions;
mod error;
mod index;
mod light;
mod locate;
mod parser;
mod scalar;
pub mod simd;

pub use error::YamlError;
pub use index::YamlIndex;
pub use light::{
    ChompingIndicator, YamlCursor, YamlElements, YamlField, YamlFields, YamlNumber, YamlString,
    YamlValue,
};
pub use locate::{locate_offset, locate_offset_detailed, LocateResult};
pub use scalar::{resolve_plain, ResolvedScalar};
