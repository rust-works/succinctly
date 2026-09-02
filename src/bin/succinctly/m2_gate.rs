//! The M2 streaming AST-shape gate, shared between `jq_runner.rs` and
//! `yq_runner.rs` (#1576).
//!
//! `can_use_m2_streaming` originated in `yq_runner.rs` (#757 and its many
//! follow-ups) but is pure `Expr`/`Builtin` matching with no yq-specific
//! dependency -- confirmed by reading it end to end before moving it here.
//! jq's own M2 fast path (`jq_runner.rs`) needs the identical predicate, and
//! duplicating it there would recreate exactly the failure mode this
//! codebase's own CLAUDE.md documents as a past bug: "Duplicated predicates
//! diverge silently -- one definition, plus a test that the call sites
//! agree" (#106 had three copies of one predicate, one of them wrong). A
//! single shared definition, used by both runners, closes that off by
//! construction rather than by convention.

use succinctly::jq::{Builtin, Expr};

/// Check if an expression can use the M2 streaming path.
///
/// M2 streaming is used for simple navigation expressions that produce
/// cursor results without requiring OwnedValue construction:
/// - Identity: `.`
/// - Field access: `.field`
/// - Index access: `.[0]`, `.[-1]`
/// - Iteration: `.[]`
/// - Chained navigation: `.field[0].name`
/// - Optional variants: `.field?`, `.[0]?`, `.[]?`
/// - `keys_unsorted` (streams lazily via `GenericResult::LazyKeys { sorted: false, .. }`, #685)
/// - `select(...)`, `first`/`last`/`limit`/`nth` (when their body does)
/// - `sort`/`sort_by`/`unique`/`unique_by`/`min`/`min_by`/`max`/`max_by`/`reverse`
/// - `map(f)` (when `f` does, #724/#725/#757/#1576)
///
/// Expressions that require OwnedValue construction cannot use M2:
/// - Builtins like `length`, `keys` (sorted)
/// - Array/object construction: `[...]`, `{...}`
/// - Arithmetic, comparison, and logic operators
/// - String interpolation
/// - Variables and function calls
pub fn can_use_m2_streaming(expr: &Expr) -> bool {
    match expr {
        // Core M2 expressions
        Expr::Identity => true,
        Expr::Field(_) => true,
        Expr::Index { .. } => true,
        Expr::Iterate => true,

        // Chained navigation
        Expr::Pipe(exprs) => exprs.iter().all(can_use_m2_streaming),

        // Optional variants
        Expr::Optional(inner) => can_use_m2_streaming(inner),

        // Parentheses don't affect streamability
        Expr::Paren(inner) => can_use_m2_streaming(inner),

        // first(f)/last(f) (both AST spellings the parser produces, see
        // `Expr::FirstExpr`/`LastExpr` doc comments) thread a cursor through
        // natively in `eval_generic.rs` (#607) *only when `f` itself does* --
        // `first(.[])` streams a `GenericResult` cursor exactly like plain
        // navigation, but `first(.a * 1e100)` still has to materialize an
        // `OwnedValue::Float` from the arithmetic, which then needs the DOM
        // path's yq-mode scientific-notation formatting (#997) rather than
        // the M2 fast writers in `src/jq/stream.rs`, which don't have it.
        // Recursing here (like `Pipe`/`Optional`/`Paren` above) restricts the
        // fast path to exactly the inner shapes it can actually stream a
        // cursor for. Streaming through `eval_with_cursor_using` for the
        // eligible cases (rather than the DOM path's unconditional
        // `to_owned()`) is also what keeps duplicate mapping keys intact for
        // these shapes, matching `.[0]` on the same input (#631).
        Expr::FirstExpr(inner) | Expr::LastExpr(inner) => can_use_m2_streaming(inner),
        Expr::Builtin(Builtin::FirstStream(inner) | Builtin::LastStream(inner)) => {
            can_use_m2_streaming(inner)
        }

        // Same reasoning as `FirstExpr`/`LastExpr` above, now that #1607
        // gave `Expr::Limit`/`Builtin::NthStream` (the arm real
        // `nth(n; expr)` calls reach) their own native, cursor-threading
        // arms in `eval_generic.rs`: `limit(3; .[])`/`nth(0; .[])` stream a
        // `GenericResult` cursor exactly like plain navigation when `expr`
        // itself does, so route them through `eval_with_cursor_using`
        // here too rather than the DOM path's unconditional `to_owned()`
        // -- otherwise #1607's own fix is discarded one layer up: a
        // correctly cursor-preserving `GenericResult` still gets flattened
        // into an `IndexMap`-backed `OwnedValue` the moment the DOM path
        // materializes it for output, silently re-losing a duplicate key
        // *inside* the captured item (not the `limit`/`nth` walk itself,
        // which #1607 already fixed regardless of this gate). `n` is never
        // recursed into: it's always evaluated as a single control value,
        // never streamed.
        Expr::Limit { n: _, expr }
        | Expr::NthExpr { n: _, expr }
        | Expr::Builtin(Builtin::NthStream(_, expr)) => can_use_m2_streaming(expr),
        Expr::IndexExpr { .. } => true,

        // `select(...)` never changes position - a truthy output is always
        // the input node unchanged - and `eval_generic.rs`'s own
        // `Builtin::Select` arm already forwards the incoming cursor as-is
        // (`OneCursor`/`ManyCursor`) rather than rebuilding a value. Routing
        // it here rather than through the DOM path's unconditional
        // `to_owned()` is what keeps duplicate mapping keys (and their
        // comments, in yq mode) intact, matching `FirstExpr`/`LastExpr`
        // above (#631) and `-S`/`--tab` (#733) - `select()` had the same
        // latent gap (#796).
        Expr::Builtin(Builtin::Select(_)) => true,

        // #1687: `sort`/`sort_by`/`unique`/`unique_by`/`reverse` answer a
        // `GenericResult::LazySeq` over their input's own element cursors,
        // and `min`/`min_by`/`max`/`max_by` a bare `OneCursor` -- the same
        // shapes `map`/`first` above already stream. Without an arm here the
        // fix is discarded one layer up, exactly as #1607's was before its
        // review caught it: the DOM path's unconditional `to_owned()`
        // flattens the cursor-preserving result back into an
        // `IndexMap`-backed `OwnedValue` at output time, silently
        // re-collapsing a duplicate mapping key inside a moved element (yq
        // mode) or discarding cursor-level throughput (jq mode, #1576).
        //
        // The key filter is never recursed into: it only ever produces a
        // comparison key, which is an `OwnedValue` regardless and never
        // reaches the output. What *is* streamed is the element the key
        // selected, and that is always a document node.
        Expr::Builtin(
            Builtin::Sort
            | Builtin::SortBy(_)
            | Builtin::Unique
            | Builtin::UniqueBy(_)
            | Builtin::Min
            | Builtin::MinBy(_)
            | Builtin::Max
            | Builtin::MaxBy(_)
            | Builtin::Reverse,
        ) => true,

        // `keys_unsorted` on a mapping/object produces
        // `GenericResult::LazyKeys { sorted: false, .. }`, which
        // `GenericResult::stream_json`/`stream_yaml` stream directly from
        // the field cursor (#685) instead of materializing a `Vec<String>`
        // first. On an array input it already returns `GenericResult::Owned`
        // cheaply, so this only changes routing for the mapping case.
        Expr::Builtin(Builtin::KeysUnsorted) => true,

        // `map(f)` on a container produces `GenericResult::LazySeq` (#724,
        // #725), which `GenericResult::stream_json`/`stream_yaml` render one
        // element at a time from each element's own live cursor (#757,
        // #1576) rather than through an `OwnedValue::Array`. For yq that is
        // both a throughput win and a fidelity fix (duplicate mapping keys,
        // comments, anchors/aliases and flow style, all of which real yq
        // keeps and the DOM path drops); for jq it's throughput only (#1576
        // verified no jq-side correctness gap exists here).
        //
        // Recursing into `f` rather than answering a flat `true` follows
        // `FirstExpr`/`LastExpr` above, for the same reason: a *computing*
        // body materializes an `OwnedValue::Float`/`NumberLiteral` that
        // needs a mode-specific DOM-path formatting rule the M2 streamers
        // don't have (yq's scientific-notation/decimal-point formatting,
        // #997/#949/#1090; jq's own `format_number_jq_compat` reformatting
        // for a *computed*, not source-literal, number). So `map(.)`,
        // `map(.name)`, `map(.a.b)` and `map(select(...))` stream;
        // `map(.+1)` and `map(length)` keep the DOM path exactly as before.
        Expr::Builtin(Builtin::Map(f)) => can_use_m2_streaming(f),

        // Everything else requires OwnedValue
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Expr::NthExpr` shares its gate arm with `Expr::Limit` and
    /// `Builtin::NthStream`, but is never constructed by the parser -- a CLI
    /// `nth(n; expr)` always parses through `Builtin::NthStream` (see
    /// `install_def_calls_descends_into_nth_expr_1371` in `src/jq/eval.rs`
    /// for the same constraint on the evaluator side). Sharing one arm is
    /// only correct while all three answer the same way, so the arm is
    /// exercised directly here rather than left to whichever alternative the
    /// CLI happens to reach: the gate must recurse into `expr` (streamable
    /// body streams, computing body doesn't) and must ignore `n` entirely,
    /// which is always evaluated as a single control value and never
    /// streamed.
    #[test]
    fn nth_expr_gate_arm_recurses_into_expr_only_1576() {
        let streamable = || Expr::NthExpr {
            n: Box::new(Expr::Identity),
            expr: Box::new(Expr::Iterate),
        };
        assert!(
            can_use_m2_streaming(&streamable()),
            "a streamable body must stream through NthExpr"
        );

        assert!(
            !can_use_m2_streaming(&Expr::NthExpr {
                n: Box::new(Expr::Identity),
                expr: Box::new(Expr::Builtin(Builtin::Length)),
            }),
            "a computing body must keep the DOM path"
        );

        // `n` is never recursed into: a non-streamable `n` alongside a
        // streamable `expr` still streams.
        assert!(
            can_use_m2_streaming(&Expr::NthExpr {
                n: Box::new(Expr::Builtin(Builtin::Length)),
                expr: Box::new(Expr::Iterate),
            }),
            "n is a control value, not part of the streamed shape"
        );

        // The two alternatives sharing this arm must agree with it.
        assert_eq!(
            can_use_m2_streaming(&streamable()),
            can_use_m2_streaming(&Expr::Limit {
                n: Box::new(Expr::Identity),
                expr: Box::new(Expr::Iterate),
            }),
            "Expr::Limit shares NthExpr's arm and must answer the same"
        );
        assert_eq!(
            can_use_m2_streaming(&streamable()),
            can_use_m2_streaming(&Expr::Builtin(Builtin::NthStream(
                Box::new(Expr::Identity),
                Box::new(Expr::Iterate),
            ))),
            "Builtin::NthStream shares NthExpr's arm and must answer the same"
        );
    }
}
