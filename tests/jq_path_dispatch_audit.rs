//! #2697's compiler nudge: a wildcard-free witness over `Expr`, so adding a
//! variant cannot silently land on `path()`'s eager fallback.
//!
//! # The gap this closes
//!
//! `resolve_node_sink` and `resolve_node_eager` (`src/jq/eval.rs`) are the two
//! dispatch points #2235's demand-driven migration moves constructs through,
//! one at a time. Both end in an unchecked `other => ...` arm, so a new
//! `Expr` variant compiles cleanly and lands on the eager, non-streaming
//! path. Nothing makes a human notice that a new bounded-generator-shaped
//! construct needs a `_sink` arm to stream under `limit`/fold-source
//! contexts.
//!
//! #2697 assumed no compiler mechanism could help "without a larger
//! restructuring", and proposed a lint or a naming convention instead. Both
//! are weaker than what this repo already does one module over:
//! `walk.rs`'s `builtin_kids` names every `Builtin` variant with no wildcard,
//! precisely so "adding a variant fails to compile here until its
//! sub-expressions are declared". Its doc comment is worth re-reading before
//! touching this file:
//!
//! > Resist the temptation to shorten this with a catch-all: the cost of the
//! > long match is paid once, by whoever adds a variant; the cost of a
//! > wildcard is paid silently and repeatedly by everyone downstream.
//!
//! The same technique works here **without restructuring the real dispatch**,
//! because the witness does not have to *be* the dispatch. `classify` below
//! is a separate, wildcard-free match whose only job is to fail compilation
//! when `Expr` grows. The hundreds-of-arms match in `eval.rs` keeps its
//! catch-all; what it loses is the ability to absorb a new variant unnoticed.
//!
//! # Why the classification is cross-checked and not just written down
//!
//! A hand-maintained list of "which variants stream" is exactly the artefact
//! that rots -- it would still compile after someone migrates a construct
//! from the eager path to a `_sink` arm, and would then describe the code as
//! it was, not as it is. So `sink_classification_matches_the_dispatch`
//! re-derives the truth from `eval.rs`'s own source (`include_str!`, the same
//! shape `jq_member_validation_audit.rs` and `jq_optional_suppression_audit.rs`
//! use) and asserts the witness agrees.
//!
//! That makes the file self-correcting in both directions: adding an `Expr`
//! variant breaks the build here, and migrating a construct between the two
//! dispatch paths breaks this test until the witness is updated to match.

use succinctly::jq::{Builtin, Expr};

/// Which of `path()`'s two dispatch paths a variant takes in
/// `resolve_node_sink` (`src/jq/eval.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathDispatch {
    /// Has a native demand-driven arm: it can stop a generator early, so a
    /// bounded consumer (`first`, `limit`, a fold source) never pays for
    /// outputs it will not read.
    Sink,
    /// Reaches the sink through `drain_path_result` after resolving eagerly.
    /// Byte-identical to the sink path for an always-`Continue` consumer, so
    /// this is a missed optimization, never a behaviour difference -- see
    /// `resolve_node_sink`'s own catch-all comment.
    ///
    /// A variant here is a *candidate* for migration, not a bug. What #2697
    /// asks is that arriving here be a decision someone made, not a default
    /// a new variant fell into.
    Eager,
}

/// Every `Expr` variant, and which dispatch path it takes.
///
/// **Deliberately has no wildcard arm** (see this file's own module comment,
/// and `walk.rs`'s `builtin_kids`, which this mirrors). If you are here
/// because a new variant broke the build: decide whether it needs to stream
/// under a bounded consumer. If it does, give it an arm in
/// `resolve_node_sink` and classify it `Sink`; if it does not, classify it
/// `Eager` and say why in a comment. Do not add a catch-all.
fn classify(expr: &Expr) -> PathDispatch {
    match expr {
        // --- Native demand-driven arms in `resolve_node_sink` ------------
        //
        // Structural constructs that thread a consumer's demand through to
        // their own sub-expressions, plus the bounded generators #2235
        // migrated one at a time.
        Expr::Identity
        | Expr::Iterate
        | Expr::Optional(_)
        | Expr::Pipe(_)
        | Expr::Comma(_)
        | Expr::Array(_)
        | Expr::RecursiveDescent
        | Expr::Paren(_)
        | Expr::Alternative(_, _)
        | Expr::If { .. }
        | Expr::Try { .. }
        | Expr::Builtin(_)
        | Expr::As { .. }
        | Expr::TrackedVar(_)
        | Expr::Reduce { .. }
        | Expr::Foreach { .. }
        | Expr::Limit { .. }
        | Expr::FirstExpr(_)
        | Expr::NthExpr { .. }
        | Expr::Repeat(_)
        | Expr::Label { .. }
        | Expr::AsPattern { .. }
        | Expr::Shared(_)
        | Expr::DefCall { .. } => PathDispatch::Sink,

        // --- Eager, and deliberately so ----------------------------------
        //
        // Single-valued navigation. A generator bound cannot save work on a
        // step that produces exactly one output, so there is nothing for a
        // `_sink` arm to buy -- `resolve_leaf` handles these and the
        // static-tail fast path skips even that.
        Expr::Field(_) | Expr::Index { .. } | Expr::Slice { .. } => PathDispatch::Eager,

        // The computed-key/slice siblings. `resolve_node_eager` names these
        // explicitly rather than letting them fall through, because their
        // key expression is itself resolved. #2546 tracks making their
        // computed bounds lazy; until then they are eager by decision.
        Expr::IndexExpr { .. } | Expr::SliceExpr { .. } => PathDispatch::Eager,

        // Value-producing constructs. None of these can *be* a path, so
        // whatever they compute is untracked and a bounded consumer has
        // nothing to truncate: the whole value is produced or none of it is.
        // They reach the sink through `drain_path_result` and raise jq's
        // "Invalid path expression" at the terminal.
        Expr::Object(_)
        | Expr::Literal(_)
        | Expr::Arithmetic { .. }
        | Expr::Negate(_)
        | Expr::Compare { .. }
        | Expr::And(_, _)
        | Expr::Or(_, _)
        | Expr::Not
        | Expr::Error(_)
        | Expr::StringInterpolation(_)
        | Expr::Format(_)
        | Expr::Var(_)
        | Expr::Loc { .. }
        | Expr::Env => PathDispatch::Eager,

        // Generators that are not *bounded* by their own shape: each runs to
        // its own completion, so a `_sink` arm would forward demand without
        // being able to stop earlier than the eager path already does.
        // `LastExpr` is the pointed case -- `last(f)` must drain `f` to know
        // its final output, which is why it has no arm beside `FirstExpr`'s.
        Expr::LastExpr(_)
        | Expr::Until { .. }
        | Expr::While { .. }
        | Expr::Range { .. }
        | Expr::Break(_) => PathDispatch::Eager,

        // Definition and call machinery. `FuncDef`/`FuncCall`/`NamespacedCall`
        // are resolved away before path dispatch sees them -- a bound call
        // arrives as `DefCall`, which is `Sink` above -- so these reach the
        // eager fallback only on a shape that never resolved.
        Expr::FuncDef { .. } | Expr::FuncCall { .. } | Expr::NamespacedCall { .. } => {
            PathDispatch::Eager
        }

        // Assignment forms. These are *consumers* of `path()`, not path
        // expressions: `.a = 1` resolves `.a` through this machinery and then
        // writes. Reaching path dispatch with an assignment as the path is a
        // malformed shape, not a streaming opportunity.
        Expr::Assign { .. }
        | Expr::Update { .. }
        | Expr::CompoundAssign { .. }
        | Expr::AlternativeAssign { .. }
        | Expr::MetaAssign { .. } => PathDispatch::Eager,
    }
}

/// The witness compiles, which is the whole mechanism -- plus a sanity check
/// that it actually discriminates rather than answering one value.
#[test]
fn every_expr_variant_is_classified() {
    // Representative of each arm group, so this fails loudly if a future
    // edit collapses the match into something uniform.
    assert_eq!(classify(&Expr::Identity), PathDispatch::Sink);
    assert_eq!(classify(&Expr::Pipe(Vec::new())), PathDispatch::Sink);
    assert_eq!(classify(&Expr::Builtin(Builtin::Type)), PathDispatch::Sink);
    assert_eq!(classify(&Expr::Field("a".to_string())), PathDispatch::Eager);
    assert_eq!(classify(&Expr::Not), PathDispatch::Eager);
    assert_eq!(
        classify(&Expr::LastExpr(Box::new(Expr::Identity))),
        PathDispatch::Eager
    );
}

/// The classification above is cross-checked against `eval.rs` itself, so it
/// cannot quietly describe a migration that has not happened (or miss one
/// that has).
///
/// The check is **set equality, both directions**: the variants the witness
/// calls `Sink` are exactly the variants `resolve_node_sink`'s body names.
/// That held on the nose when this was written (24 and 24, no stragglers in
/// either direction), which is what makes the strong form worth asserting --
/// a one-directional check would still pass after a construct was migrated
/// *back* to the eager path.
///
/// If this fails, one of three things happened, and the message says which:
/// a construct gained a `_sink` arm (classify it `Sink`), lost one (classify
/// it `Eager`), or the arm was renamed.
#[test]
fn sink_classification_matches_the_dispatch() {
    const EVAL_RS: &str = include_str!("../src/jq/eval.rs");

    let body = fn_body(EVAL_RS, "resolve_node_sink").expect("resolve_node_sink should be found");

    // The `Sink` half of `classify`, spelled out so the check has something
    // to compare against -- keep in step with the match above. The test that
    // guards *this* list against drift is the compile error: a variant added
    // to `Expr` breaks `classify`, and whoever fixes it lands here next.
    let sink_variants = [
        "Identity",
        "Iterate",
        "Optional",
        "Pipe",
        "Comma",
        "Array",
        "RecursiveDescent",
        "Paren",
        "Alternative",
        "If",
        "Try",
        "Builtin",
        "As",
        "TrackedVar",
        "Reduce",
        "Foreach",
        "Limit",
        "FirstExpr",
        "NthExpr",
        "Repeat",
        "Label",
        "AsPattern",
        "Shared",
        "DefCall",
    ];

    let missing: Vec<&str> = sink_variants
        .iter()
        .copied()
        .filter(|v| !body.contains(&format!("Expr::{v}")))
        .collect();

    assert!(
        missing.is_empty(),
        "classified `Sink` but not named in `resolve_node_sink`: {missing:?} -- either \
         the construct was migrated back to the eager path (classify it `Eager`), or \
         its arm was renamed (update this list)"
    );

    // The other direction: nothing `resolve_node_sink` names may be missing
    // from the list. This is what catches a *new* `_sink` arm added without
    // the witness being updated -- the migration direction #2235 is actually
    // travelling.
    let mut named: Vec<String> = Vec::new();
    let mut rest = body;
    while let Some(at) = rest.find("Expr::") {
        rest = &rest[at + "Expr::".len()..];
        let end = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(rest.len());
        let variant = &rest[..end];
        if !variant.is_empty() && !named.iter().any(|n| n == variant) {
            named.push(variant.to_string());
        }
    }
    let unclassified: Vec<&String> = named
        .iter()
        .filter(|n| !sink_variants.contains(&n.as_str()))
        .collect();

    assert!(
        unclassified.is_empty(),
        "named in `resolve_node_sink` but not classified `Sink`: {unclassified:?} -- a \
         construct gained a native demand-driven arm; classify it `Sink` in `classify` \
         and add it here"
    );
}

/// The body of `fn <name>`, brace-matched from its signature.
///
/// Deliberately naive: it finds the first `fn <name>` and matches braces. The
/// audited functions are ordinary free functions with no `fn` in a string
/// literal before their opening brace, and the test above fails loudly if
/// this ever returns the wrong span.
fn fn_body<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    let at = source.find(&format!("\nfn {name}"))?;
    let open = at + source[at..].find('{')?;
    let mut depth = 0usize;
    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[open..open + offset]);
                }
            }
            _ => {}
        }
    }
    None
}
