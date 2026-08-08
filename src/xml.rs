//! XML semi-indexing.
//!
//! Milestone 1 (issue #667) of the `xq` XML query tool (full vision: issue
//! #85): a semi-index over XML text, exposing `first_child`/`next_sibling`/
//! `parent`/`value` navigation via this crate's format-agnostic
//! [`DocumentValue`](crate::jq::document::DocumentValue)/
//! [`DocumentCursor`](crate::jq::document::DocumentCursor) traits, the same
//! way [`json`](crate::json) and [`yaml`](crate::yaml) already do.
//!
//! Scope is intentionally narrow: element/attribute/text navigation plus
//! `at_offset`/`at_position`. No XML-specific jq combinators, no `@xml`
//! format function, no namespace resolution, and no SIMD acceleration (see
//! `docs/STYLE_GUIDE.md` and this repo's P5-P8 rejected-SIMD history) — all
//! left to #85.
//!
//! An XML element projects to [`DocumentValue::as_object`] only (never
//! `as_array`): its attributes and children become object fields, keyed as
//! `"+@name"` for an attribute, the plain tag name for a child element, and
//! `"+content"` for the element's text content. The `+@` attribute prefix
//! matches the convention already recorded for the future `@xml`/`to_xml`
//! encoder (`docs/plan/yq.md`), so this milestone is forward-compatible with
//! it. Repeated same-name children resolve to the first occurrence.

pub mod light;
pub mod locate;
mod scan;

pub use light::{XmlCursor, XmlElements, XmlFields, XmlIndex, XmlValue};
pub use locate::{locate_offset, locate_offset_detailed, LocateResult};
pub use scan::XmlScanError;
