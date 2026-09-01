//! jq-like query expressions for JSON navigation.
//!
//! This module provides a subset of jq syntax for querying JSON documents
//! using the fast cursor-based navigation provided by the JSON semi-indexing.
//!
//! # Supported Syntax
//!
//! | Expression | Meaning |
//! |------------|---------|
//! | `.` | Identity (return the whole document) |
//! | `.foo` | Access field "foo" of an object |
//! | `.[0]` | Access first element of an array |
//! | `.[-1]` | Access last element of an array |
//! | `.[]` | Iterate all elements of array/object |
//! | `.[2:5]` | Slice array from index 2 to 5 |
//! | `.[$k]`, `.[.k]`, `.[1,2]` | Index by a computed key (string key selects a field, number an element) |
//! | `.foo.bar` | Chained field access |
//! | `.foo[0].bar` | Mixed field and index access |
//! | `.foo?` | Optional access (null if missing) |
//! | `.foo, .bar` | Comma - output from both expressions |
//! | `[.foo, .bar]` | Array construction |
//! | `{foo: .bar}` | Object construction |
//! | `(.foo)` | Parentheses for grouping |
//! | `..` | Recursive descent |
//! | `null`, `true`, `false` | Literal values |
//! | `"string"` | String literal |
//! | `123`, `3.14` | Number literal |
//! | `.a + .b` | Arithmetic (+, -, *, /, %) |
//! | `.a * .b` | Recursive object merge (both jq and yq); yq also replaces on array `*`, with flag suffixes `*+`/`*?`/`*n`/`*d`/`*c` controlling merge semantics (yq mode only) |
//! | `.a == .b` | Comparison (==, !=, <, <=, >, >=) |
//! | `.a and .b` | Boolean AND |
//! | `.a or .b` | Boolean OR |
//! | `not` | Boolean NOT |
//! | `.a // .b` | Alternative (default if falsy) |
//! | `if .a then .b else .c end` | Conditional |
//! | `try .a catch .b` | Error handling |
//! | `error("msg")` | Raise error |
//! | `.a = value` | Simple assignment |
//! | `.a \|= filter` | Update assignment |
//! | `.a += value` | Compound assignment (+=, -=, *=, /=, %=) |
//! | `.a //= value` | Alternative assignment (set if null/false) |
//! | `del(.a)` | Delete field/element |
//!
//! # Example
//!
//! ```
//! use succinctly::jq::{parse, eval, JqSemantics, QueryResult};
//! use succinctly::json::{JsonIndex, StandardJson};
//!
//! let json = br#"{"users": [{"name": "Alice"}, {"name": "Bob"}]}"#;
//! let index = JsonIndex::build(json);
//! let cursor = index.root(json);
//!
//! // Get first user's name
//! let expr = parse(".users[0].name").unwrap();
//! match eval::<Vec<u64>, JqSemantics>(&expr, cursor) {
//!     QueryResult::One(StandardJson::String(s)) => {
//!         assert_eq!(s.as_str().unwrap().as_ref(), "Alice");
//!     }
//!     _ => panic!("unexpected result"),
//! }
//!
//! // Get all user names
//! let expr = parse(".users[].name").unwrap();
//! match eval::<Vec<u64>, JqSemantics>(&expr, cursor) {
//!     QueryResult::Many(names) => {
//!         assert_eq!(names.len(), 2);
//!     }
//!     _ => panic!("unexpected result"),
//! }
//! ```

pub mod document;
mod error;
pub mod escape;
mod eval;
pub mod eval_generic;
mod expr;
mod lazy;
mod parser;
pub mod resolve;
mod slice;
pub mod stream;
pub mod utf8_document;
mod value;
pub mod walk;

pub use error::{BinOp, Control, ErrorKind, EvalError, EvalErrorPayload};
pub use eval::{
    eval, eval_lenient, eval_owned_with_file_index, nonfinite_display_string, substitute_vars,
    sync_aliased_paths, EvalSemantics, JqSemantics, QueryResult, YqSemantics,
};
// `input`/`inputs`/`input_line_number`'s CLI-facing seam (#723) -- only
// exists under `eval.rs`'s own `#[cfg(feature = "std")]` gate (a `thread_local!`
// backing store needs `std`, and there is no meaningful "remaining input
// stream" to seed under a `no_std` embedding with no CLI driver anyway).
#[cfg(feature = "std")]
pub use eval::{
    current_input_location, pop_remaining_input, seed_remaining_inputs, UNKNOWN_INPUT_LINE,
};
// `input_queue_is_active` exists under both gates (a `const fn` returning
// `false` without `std`), so `eval_generic` can consult it unconditionally.
pub use eval::input_queue_is_active;
pub use expr::{
    ArithOp, AssignOp, Builtin, CompareOp, Expr, FormatType, FuncDefData, Import, Include, Literal,
    MetaValue, ModuleMeta, NumberKey, ObjectEntry, ObjectKey, Pattern, PatternEntry, Program,
    StringPart,
};
pub use lazy::JqValue;
pub use parser::{
    parse, parse_program, parse_program_with_mode, parse_program_with_mode_and_extensions,
    parse_with_mode, parse_with_mode_and_extensions, ParseError, ParserMode,
};
pub use resolve::{resolve_func_calls, resolve_func_calls_all, UnresolvedCall};
pub use stream::{StreamError, StreamStats, StreamableValue};
pub use value::{
    assert_value_tree_depth, format_number_jq_compat, nesting_depth_exceeded_message, NumberRepr,
    OwnedValue, MAX_VALUE_TREE_DEPTH,
};
// `pub(crate)`, not `pub`: only `yaml::light`'s alias-chain resolvers (#1191
// code review, #1193/PR #1314) need this from outside `jq::value` --
// re-exporting it as a real public API item would put a crate-internal
// depth-guard helper on docs.rs with no external caller to justify the
// exposure.
pub(crate) use value::assert_depth;
