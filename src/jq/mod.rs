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
mod eval;
pub mod eval_generic;
mod expr;
mod lazy;
mod parser;
pub mod stream;
mod value;

pub use eval::{
    eval, eval_lenient, substitute_vars, EvalError, EvalSemantics, JqSemantics, QueryResult,
    YqSemantics,
};
pub use expr::{
    ArithOp, AssignOp, Builtin, CompareOp, Expr, FormatType, Import, Include, Literal, MetaValue,
    ModuleMeta, ObjectEntry, ObjectKey, Pattern, PatternEntry, Program, StringPart,
};
pub use lazy::JqValue;
pub use parser::{
    parse, parse_program, parse_program_with_mode, parse_with_mode, ParseError, ParserMode,
};
pub use stream::{StreamStats, StreamableValue};
pub use value::OwnedValue;
