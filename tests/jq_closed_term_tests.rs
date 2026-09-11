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
use succinctly::jq::{eval_generic, parse, parse_with_mode, ParserMode};
use succinctly::json::JsonIndex;

/// Structurally unrelated, all well-formed: an object, an array, a scalar,
/// an empty container, and a nested mix. If an expression reads *anything*
/// from the input, one of these five will make it show.
const DOCS: &[&str] = &[
    r#"{"a":1,"b":{"c":[2,3]}}"#,
    "[10,20,30]",
    r#""a string""#,
    "{}",
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
    // `repeat`'s native arm materialized unconditionally, bypassing the
    // predicate entirely -- caught only by a direct malformed-document
    // probe (`test_closed_terms_do_not_validate_2173`), not this corpus,
    // but still closed at the predicate level and belongs here too.
    "[limit(1; repeat(1))]",
    // `Builtin::EnvObject`/`Builtin::StrEnv` take no value parameter but
    // were missing from `node_reads_ambient`'s allowlist. A var guaranteed
    // absent keeps the `?` outcome document-independent.
    "[env(NONEXISTENT_VAR_XYZ_2173)?]",
    "[strenv(NONEXISTENT_VAR_XYZ_2173)?]",
    // #2699: a pipe stage's ambient value is the previous stage's output,
    // so a closed first stage closes the whole pipe.
    "1 | .",
    "now | type",
    "[1,2,3] | length",
    r#""abc" | ascii_upcase"#,
    "2 | . + 1",
    "[1] | .[0]",
    "1 | length",
    "[1,2,3] | .[1:2]",
    "[1,2] | add",
    "{} | keys",
    "1 | if . then 2 else 3 end",
    "[1,2] | map(. + 1)",
    "1 | 2 | 3 | .",
    "[1,2,3] | .[] | . + 1",
    // `reduce`/`foreach` rebind `.` to the accumulator in `update`.
    "reduce (1,2,3) as $x (0; . + $x)",
    "[foreach (1,2) as $x (0; . + $x)]",
    "[foreach (1,2) as $x (0; . + $x; . * 10)]",
    // The ambient-transparent wrappers carry the refinement through.
    "(1 | .) | .a?",
    "[1 | .]",
    "[[1,2] | length]",
    "((1 | .))",
    // The `Expr::Optional` arm's only witness. Every other `?` in this
    // corpus sits in a *later* stage, which `stage_escapes_own_input`
    // judges -- so removing the `Optional` arm broke no test at all until
    // this row existed (found by the #2790 review's arm-by-arm mutation).
    "(1 | .)?",
    "[(1 | .), (2 | .)]",
    // Closed *and* ill-typed: `.a` applies to the previous stage's `1`, so
    // the outcome is the same whatever the document is -- which is the
    // property under test. Note these raise on the CLI ("Cannot index
    // number with string \"a\"", matching jq 1.7.1) but reach this
    // library harness as no output, because `collect_owned` stops at the
    // failure rather than propagating it. Either way: document-independent.
    // `[foreach ...; .a]` is here for the same reason, and is the row that
    // proves `extract` sees the accumulator rather than the document.
    "1 | .a",
    "1 | .[0]",
    r#""abc" | .a"#,
    "1 | keys",
    "[1,2] | .a",
    "[foreach (1,2) as $x (0; . + $x; .a)]",
    // #2794: `isempty(f)`'s argument is evaluated against the true ambient,
    // so it is closed exactly when `f` is.
    "isempty(1)",
    "isempty(empty)",
    "isempty(range(9))",
    // #2794: `any(gen; cond)`/`all(gen; cond)` desugar to `gen | cond`, so
    // `cond` sees gen's *output*, not the document -- closed when `gen` is,
    // whatever `cond` looks like.
    "any(range(3); . > 1)",
    "any(range(3); . > 5)",
    "all(range(3); . >= 0)",
    "all(range(3); . > 5)",
    "any(1,2,3; . == 2)",
    "all(1,2,3; . > 0)",
    // Closed *and* ill-typed, mirroring `[foreach ...; .a]` above: `.a`
    // applies to gen's own `1`, not the document.
    "any(1; .a)",
    "all(1; .a)",
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
    // Assignments: the output is the document with a modification, so they
    // read `.` even when path and value are both closed. `(1) = 5` is the
    // row that caught the missing arm (#1764's no-op turned into `null`).
    "(1) = 5",
    "1 = 5",
    "(1+1) = 5",
    "(1, 2) = 5",
    ".a = 5",
    "1 |= . + 1",
    // #2699: the refinement must NOT reach these.
    //
    // `as` binds a variable but leaves `.` on the OUTER ambient, so the
    // body still reads the document. This is the row that makes the pipe
    // rule safe: it parses as `Expr::As`, never `Expr::Pipe`.
    "1 as $x | .",
    "1 as $x | .a",
    "[1,2] as [$a,$b] | .",
    "{} as {a:$a} | .",
    // A closed first stage does not help if a later stage reaches the
    // document through a channel that bypasses the ambient value.
    "1 | input",
    "1 | [inputs]",
    "1 | key",
    "1 | parent",
    "1 | path(.)",
    "1 | [paths]",
    "1 | [leaf_paths]",
    "1 | getpath([\"a\"])",
    "1 | line",
    "1 | column",
    // The five siblings missing from `stage_escapes_own_input` on day one
    // (#2790 review). None could be turned into a wrong answer, but the
    // omissions are what a fail-open list looks like in practice.
    "1 | path",
    "1 | [paths(numbers)]",
    "1 | file_index",
    "1 | tag",
    "1 | kind",
    // An unresolved call carries no body to inspect.
    "1 | f",
    // A reading first stage keeps the whole pipe reading, at any depth.
    ". | 1",
    ".a | 1",
    "(. | 1) | 2",
    "[. | 1]",
    ". | . | .",
    "(.a, 1) | 1",
    // `reduce`/`foreach` read the document through `input` and `init`.
    "reduce .[] as $x (0; . + $x)",
    "reduce (1,2) as $x (.; . + $x)",
    "[foreach .[] as $x (0; . + $x)]",
    "[foreach (1,2) as $x (.; . + $x)]",
    ".a += 1",
    ".a //= 1",
    "del(.a)",
    "setpath([\"a\"]; 1)",
    // #2794: a reading `isempty`/`any`/`all` argument keeps the whole node
    // reading.
    "isempty(.[])",
    "isempty(.a)",
    "any(.[]; . > 1)",
    "all(.[]; . > 0)",
    // #2794: `gen` closed but `cond` escapes through a channel that
    // bypasses gen's rebound value entirely -- the same rule
    // `stage_escapes_own_input` already enforces for `reduce`/`foreach`.
    "any(1; key)",
    "any(1; input)",
    "any(1; path(.))",
    "all(1; key)",
];

/// yq's metadata-assignment grammar (#798), which only the yq-mode parser
/// accepts. `PATH <slot> = value` leaves the value alone and answers with
/// the document, metadata written -- so like every other assignment shape
/// it reads `.` however closed its operands are. Added when a rebase
/// brought `Expr::MetaAssign` onto `main` and `node_reads_ambient`'s
/// exhaustive match refused to compile until it was classified.
const READING_YQ: &[&str] = &[
    r#".a line_comment = "hi""#,
    r#".a style = "flow""#,
    r#".a anchor = "z""#,
    r#".a line_comment |= "hi""#,
];

fn outputs(doc: &str, filter: &str) -> Result<Vec<String>, String> {
    let bytes = doc.as_bytes();
    let index = JsonIndex::build(bytes);
    let cursor = index.root(bytes);
    let expr = parse(filter).map_err(|e| format!("parse {filter:?}: {e:?}"))?;
    let values = eval_generic::eval_with_cursor(&expr, cursor)
        .collect_owned()
        .map_err(|e| format!("eval {filter:?}: {e:?}"))?;
    Ok(values
        .iter()
        .map(succinctly::jq::OwnedValue::to_json)
        .collect())
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
    // yq's metadata-assignment grammar (#798) needs the yq-mode parser.
    for filter in READING_YQ {
        let expr =
            parse_with_mode(filter, ParserMode::Yq).expect("yq filter should parse in yq mode");
        assert!(
            reads_ambient_value(&expr),
            "`{filter}` answers with the document and was reported closed"
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

/// The defensive arm: an empty `Expr::Pipe` answers `true` (#2699).
///
/// The parser never builds one — a pipe always has at least the stage before
/// its `|` — but `reads_ambient_value` is `pub`, so a library caller can
/// construct one directly, which is what this test does. `true` is the only
/// safe guess: `false` would tell `bridge_ambient_input` to hand `null` to an
/// expression nobody has inspected.
#[test]
fn an_empty_pipe_is_conservatively_reading_2699() {
    assert!(
        reads_ambient_value(&succinctly::jq::Expr::Pipe(vec![])),
        "an empty pipe has no first stage to judge, so it must not be called closed"
    );
}
