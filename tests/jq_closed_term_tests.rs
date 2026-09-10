//! #2173: the soundness of `jq::walk::reads_ambient_value`, tested against
//! what it *means* rather than against a snapshot of what it currently says.
//!
//! The predicate decides whether an expression may be evaluated against
//! `OwnedValue::Null` instead of a materialized copy of the whole input
//! document (`bridge_ambient_input` in `src/jq/eval_generic.rs`). A wrong
//! `true` costs a materialization nobody needed. A wrong `false` silently
//! evaluates a user's filter against the wrong input — so that direction
//! needs a test that cannot pass by agreeing with the implementation.
//!
//! The invariant here is that test: **if the predicate calls a filter
//! closed, then the filter's output must not depend on the document.** Every
//! closed filter is run against several structurally unrelated documents and
//! every run must agree. A `.` that crept into the allowlist would make one
//! of them disagree, whatever the predicate believes about itself.
//!
//! The reverse direction is deliberately *not* asserted. A pipe whose tail
//! stage reads its own input — `now | type`, `[1,2,3] | length`,
//! `reduce (1,2,3) as $x (0; . + $x)` — is closed in truth and reported as
//! reading, because the walk sees an `Expr::Identity`-shaped read and does
//! not know that `.` there means the previous stage's output rather than the
//! document. So is `def f: 1; f` *at parse time*, where `f` is an unresolved
//! `Expr::FuncCall` with no body to inspect; by the time
//! `bridge_ambient_input` asks, the call is an `Expr::DefCall` whose body
//! `any_subexpr` descends into, and the answer is closed. Both imprecisions
//! are recorded in `reads_ambient_value`'s own doc comment and are safe by
//! construction — they cost a materialization, never an answer — so pinning
//! them here would only freeze a limitation in place.

use succinctly::jq::walk::reads_ambient_value;
use succinctly::jq::{eval_generic, parse};
use succinctly::json::JsonIndex;

/// Structurally unrelated, all well-formed: an object, an array, a scalar,
/// an empty container, and a nested mix. If an expression reads *anything*
/// from the input, one of these five will make it show.
const DOCS: &[&str] = &[
    r#"{"a":1,"b":{"c":[2,3]}}"#,
    r#"[10,20,30]"#,
    r#""a string""#,
    r#"{}"#,
    r#"{"a":[{"z":null},false],"q":"x"}"#,
];

/// Filters that read nothing. Every one of these must be reported closed
/// *and* answer identically on all five documents.
const CLOSED: &[&str] = &[
    "1",
    "1+1",
    "[1+1]",
    "[range(3)]",
    "range(3)",
    "$__loc__",
    "empty",
    "true and true",
    "false or true",
    "false // 1",
    "-3",
    r#"{"k":1}"#,
    "if 1==1 then 2 else 3 end",
    "1 as $x | $x",
    "[limit(2; 1,2,3)]",
    "first(4,5)",
    r#"try error("boom") catch "caught""#,
    "[1,2] as [$a,$b] | $a + $b",
    r#""x\(1+1)y""#,
    "label $out | (1, break $out)",
];

/// Filters that read the input. These must be reported as reading — a
/// regression here is the dangerous direction, so they are pinned directly
/// rather than only through the agreement property.
const READING: &[&str] = &[
    ".",
    ".a",
    ".[0]",
    ".[]",
    ".[1:2]",
    "..",
    "length",
    "keys",
    "not",
    "@json",
    "add",
    "map(1)",
    "select(true)",
    ". and true",
    "range(length; 3)",
    "error",
    r#"try error catch "caught""#,
    "[.[]]",
    "{k: .a}",
    "1 + .a",
    "if . then 1 else 2 end",
    ". as $x | 1",
    "def f: .a; f",
    "[while(. < 3; . + 1)]",
    "[.[] | 1]",
    "paths",
    "to_entries",
    "getpath([\"a\"])",
];

fn outputs(doc: &str, filter: &str) -> Result<Vec<String>, String> {
    let bytes = doc.as_bytes();
    let index = JsonIndex::build(bytes);
    let cursor = index.root(bytes);
    let expr = parse(filter).map_err(|e| format!("parse {filter:?}: {e:?}"))?;
    let values = eval_generic::eval_with_cursor(&expr, cursor)
        .collect_owned()
        .map_err(|e| format!("eval {filter:?}: {e:?}"))?;
    Ok(values.iter().map(|v| v.to_json()).collect())
}

/// The soundness property: a filter the predicate calls closed must produce
/// the same outputs whatever the document is.
#[test]
fn closed_terms_ignore_the_document_2173() {
    for filter in CLOSED {
        let expr = parse(filter).expect("filter should parse");
        assert!(
            !reads_ambient_value(&expr),
            "`{filter}` should be recognised as closed"
        );

        let first =
            outputs(DOCS[0], filter).unwrap_or_else(|e| panic!("`{filter}` on {:?}: {e}", DOCS[0]));
        for doc in &DOCS[1..] {
            let got = outputs(doc, filter).unwrap_or_else(|e| panic!("`{filter}` on {doc:?}: {e}"));
            assert_eq!(
                got, first,
                "`{filter}` is reported closed but its output depends on the document \
                 ({:?} vs {doc:?})",
                DOCS[0]
            );
        }
    }
}

/// The dangerous direction, pinned directly: anything that reads the input
/// must say so, or `bridge_ambient_input` will hand it `null`.
#[test]
fn reading_terms_are_reported_as_reading_2173() {
    for filter in READING {
        let expr = parse(filter).expect("filter should parse");
        assert!(
            reads_ambient_value(&expr),
            "`{filter}` reads the input but was reported closed -- \
             `bridge_ambient_input` would evaluate it against null"
        );
    }
}

/// Bare `error` raises `.` itself, and `any_subexpr` gives `Expr::Error(None)`
/// no child to catch that on — the one case in `node_reads_ambient` that
/// needed its own arm rather than falling out of the recursion. Pinned
/// separately because the corpus above would keep passing if the arm were
/// deleted and the filter merely reported closed by accident of some other
/// node.
#[test]
fn bare_error_reads_the_document_2173() {
    assert!(reads_ambient_value(&parse("error").expect("parses")));
    assert!(reads_ambient_value(
        &parse("try error catch .").expect("parses")
    ));
    // `error(f)` has a child, so it is closed exactly when `f` is.
    assert!(!reads_ambient_value(
        &parse(r#"error("boom")"#).expect("parses")
    ));
    assert!(reads_ambient_value(&parse("error(.a)").expect("parses")));
}

/// The `Builtin` allowlist in `node_reads_ambient`, asserted at the
/// predicate level only.
///
/// These answer from the clock, the environment or the language rather than
/// from `.`, so they are closed — but `now` and `env` are not *constant*,
/// which is why they cannot ride in the agreement corpus above. Every other
/// builtin must be reported as reading, including ones whose arguments are
/// themselves closed: `map(1)` and `select(true)` both iterate or test `.`
/// however innocent their argument looks, and that distinction is the whole
/// reason the allowlist is an allowlist rather than "has no children".
#[test]
fn input_independent_builtins_are_closed_2173() {
    for filter in ["now", "empty", "env", "builtins", "nan", "infinite"] {
        let expr = parse(filter).expect("filter should parse");
        assert!(
            !reads_ambient_value(&expr),
            "`{filter}` answers from outside the document and should be closed"
        );
    }
    for filter in [
        "map(1)",
        "select(true)",
        "has(\"a\")",
        "length",
        "type",
        "tostring",
    ] {
        let expr = parse(filter).expect("filter should parse");
        assert!(
            reads_ambient_value(&expr),
            "`{filter}` consults `.` however closed its argument is"
        );
    }
}
