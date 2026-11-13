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

use succinctly::jq::eval_generic::{
    path_context_pipe_streams_cursors, recurse_cond_streams_cursors,
};
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

        // Chained navigation -- or a path-context pipe the cursor walk
        // takes whole (#2416 phase 2): `.a.b | parent`, `[.[] | key]`'s
        // inner pipe, `(.a, .b) | path`. Each output is then a live document
        // cursor or a small `key`/`path` value, and streaming the cursor's
        // own text is what keeps a float scalar's source spelling (#2419)
        // and a mapping's duplicate keys/comments, where the DOM path's
        // `to_owned()` re-spells the float as `1e+19`. A path-context pipe
        // the walk declines, or takes only the head of, stays on the DOM
        // path exactly as before.
        Expr::Pipe(exprs) => {
            path_context_pipe_streams_cursors(exprs) || exprs.iter().all(can_use_m2_streaming)
        }

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

        // `..` answers each node as its own live cursor (spine 2416, #2428),
        // the same shape `.[]` streams. Without an arm here the DOM path's
        // unconditional `to_owned()` flattens every node back into an
        // `IndexMap`-backed `OwnedValue` at output time, re-collapsing a
        // duplicate mapping key inside a node that yq prints intact (#1344).
        // `recurse`/`recurse_down` are the same walk (#3719).
        Expr::RecursiveDescent | Expr::Builtin(Builtin::Recurse | Builtin::RecurseDown) => true,

        // `recurse(.[]?; cond)` for a `cond` yielding at most one value is `..` pruned: each node
        // it keeps is its own live cursor (#3867, #4095), so it streams the same way. Without this
        // arm the DOM path copied every node's whole subtree again at output time -- 1.9 times
        // `recurse(.[]?; true)`'s time and 2.2 times its memory over a 1 MB document -- and
        // printed an alias or a flow collection expanded where `..` keeps its spelling.
        Expr::Builtin(Builtin::RecurseCond(f, cond)) => recurse_cond_streams_cursors(f, cond),

        // Everything else requires OwnedValue
        _ => false,
    }
}

/// Whether `expr` is an array that holds nothing but the document's own nodes (yq only, #3475):
/// `[.users[] | select(.age > 30)]`, `[.[]]`, `[.users]`, `.users | [.[]]`.
///
/// Such an array answers a `GenericResult::LazySeq` of cursors (`LazySeq::from_cursors`), which the
/// streaming writers render one live cursor at a time. The DOM route copied the whole sequence into
/// an owned value and a comment tree (`sequence_of_nodes`) first: 2,350 MB against
/// `map(select(...))`'s 260 MB over a 100 MB file, for the same output.
///
/// The gate has to know *before evaluating* that every element is a node, because an owned element
/// (a missing key's `null` beside real nodes, a `LazySeq` a `sort_by` or `map` hands on) makes the
/// whole array owned, and the streamers write an owned value in another spelling than the DOM
/// route's encoder: a block scalar as a quoted string, a `1e-7` float as `0.0000001`. So the body
/// is navigation to one node (a node, or a lone `null`, which both routes print alike), or
/// navigation that fans out with `.[]` and goes on only through stages that forward the node they
/// are given (`.`, `.[]`, `select(...)`): never a stage that computes, a `.a` after a fan-out that
/// can be missing, a slice, or a `,`, which is not jq's in yq (`[., .]` has one element). A prefix is
/// navigation to one node. `array_of_nodes_always_evaluates_to_cursors_3475` checks the claim
/// against the evaluator.
pub fn is_array_of_nodes(expr: &Expr) -> bool {
    match expr {
        Expr::Array(body) => body_yields_only_nodes(body),
        Expr::Paren(inner) => is_array_of_nodes(inner),
        // `.users | [.[]]`: the array last. An array in the middle feeds a later stage an owned
        // value and stays on the DOM route.
        Expr::Pipe(stages) => stages.split_last().is_some_and(|(last, prefix)| {
            is_array_of_nodes(last) && prefix.iter().all(is_navigation)
        }),
        _ => false,
    }
}

/// `.`, `.a`, `.[0]`, `.a.b`: steps that answer at most one node.
fn is_navigation(stage: &Expr) -> bool {
    match stage {
        Expr::Identity | Expr::Field(_) | Expr::Index { .. } => true,
        Expr::Optional(inner) | Expr::Paren(inner) => is_navigation(inner),
        Expr::Pipe(stages) => stages.iter().all(is_navigation),
        _ => false,
    }
}

/// A stage that hands on the node it was given, or the nodes inside it: `.`, `.[]`, `select(...)`.
fn forwards_nodes(stage: &Expr) -> bool {
    match stage {
        Expr::Identity | Expr::Iterate | Expr::Builtin(Builtin::Select(_)) => true,
        Expr::Optional(inner) | Expr::Paren(inner) => forwards_nodes(inner),
        _ => false,
    }
}

/// An array body whose every output is a node of the document (see [`is_array_of_nodes`]).
fn body_yields_only_nodes(body: &Expr) -> bool {
    // The parser nests a pipe in a pipe (`.users[] | select(.a)` is `Pipe([Pipe([users, .[]]), select])`).
    fn flatten<'a>(expr: &'a Expr, stages: &mut Vec<&'a Expr>) {
        match expr {
            Expr::Pipe(inner) => inner.iter().for_each(|stage| flatten(stage, stages)),
            Expr::Paren(inner) => flatten(inner, stages),
            other => stages.push(other),
        }
    }
    let mut stages = Vec::new();
    flatten(body, &mut stages);
    // Navigation up to the first stage that fans out, then only stages that forward nodes.
    match stages.iter().position(|stage| !is_navigation(stage)) {
        None => true,
        Some(fan_out) => stages[fan_out..].iter().all(|stage| forwards_nodes(stage)),
    }
}

/// Whether `yq --eval-all` answers `expr` the way it would answer each document
/// on its own (#4380).
///
/// Real yq's `ea` is the ordinary evaluator with the context set to *every*
/// document of every file at once; the plain mode evaluates the same filter
/// with the context set to one file's documents. Measured against yq v4.53.3,
/// `[.]` collects `ea`'s whole list but only a document in plain mode, `,` runs
/// its left operand over every document before its right, and `keys`,
/// `|= . + 1` and `.a = .b` all see the list. Navigation, and a write whose right
/// side is a literal, touches each document independently -- and the plain route
/// is the one that keeps comments, anchors and flow/quote style, which the
/// owned-value route `--eval-all` takes carries none of.
///
/// The set is a deliberate whitelist: anything not named here (a builtin, a
/// variable, `,`, a collector, an operator, a right side that reads the
/// document) stays on the `--eval-all` route, where a wrong answer is the
/// presentation loss this issue already records and not a changed value.
///
/// It claims "no worse than the owned-value route", not "byte for byte": `ea`
/// has quirks neither route reproduces (it doubles every result of `.[0]` over a
/// stream of arrays, and separates the results of `.[]` across files differently),
/// and the plain route has its own gaps (see the limitations doc).
pub fn eval_all_matches_per_document(expr: &Expr) -> bool {
    // #4380: `select(fi == 1) | .b`: a test of where the document came from keeps or drops each one
    // on its own, so what follows it is the per-document answer of the documents it keeps (the
    // plain route reads `fi`, `di` and `filename` per document, as `ea` does).
    if is_origin_select(expr) {
        return true;
    }
    if let Expr::Pipe(stages) = expr {
        let rest = stages
            .iter()
            .position(|stage| !is_origin_select(stage))
            .unwrap_or(stages.len());
        if rest > 0 && rest < stages.len() {
            return match &stages[rest..] {
                [only] => eval_all_matches_per_document(only),
                tail => tail.iter().all(navigates_only),
            };
        }
    }
    match expr {
        Expr::Builtin(Builtin::Del(path)) => navigates_only(path),
        Expr::Assign { path, value } | Expr::AlternativeAssign { path, value } => {
            navigates_only(path) && matches!(**value, Expr::Literal(_))
        }
        Expr::Update { path, filter } => {
            navigates_only(path) && matches!(**filter, Expr::Literal(_))
        }
        Expr::CompoundAssign { path, value, .. } => {
            navigates_only(path) && matches!(**value, Expr::Literal(_))
        }
        read => navigates_only(read),
    }
}

/// `select(T)` where `T` tests only the origin of the document (`fi`, `di`, `filename`) against a
/// literal, alone or under `and`, `or` and `not` (#4380).
fn is_origin_select(stage: &Expr) -> bool {
    fn origin(expr: &Expr) -> bool {
        matches!(
            expr,
            Expr::Builtin(Builtin::FileIndex | Builtin::DocumentIndex | Builtin::Filename)
        )
    }
    fn test(expr: &Expr) -> bool {
        match expr {
            Expr::Paren(inner) => test(inner),
            Expr::And(l, r) | Expr::Or(l, r) => test(l) && test(r),
            Expr::Compare { left, right, .. } => {
                (origin(left) && matches!(**right, Expr::Literal(_)))
                    || (origin(right) && matches!(**left, Expr::Literal(_)))
            }
            _ => false,
        }
    }
    match stage {
        Expr::Paren(inner) => is_origin_select(inner),
        Expr::Builtin(Builtin::Select(cond)) => test(cond),
        _ => false,
    }
}

/// `.`, fields, indexes, slices, `.[]` and `..`, however they are chained.
fn navigates_only(expr: &Expr) -> bool {
    match expr {
        Expr::Identity
        | Expr::Field(_)
        | Expr::Index { .. }
        | Expr::Slice { .. }
        | Expr::Iterate
        | Expr::RecursiveDescent => true,
        Expr::Optional(inner) | Expr::Paren(inner) => navigates_only(inner),
        Expr::Pipe(stages) => stages.iter().all(navigates_only),
        // #2126: a `del()` argument keeps `.["a"]` as a bracket on its target: navigation too.
        Expr::IndexExpr { .. } => expr
            .literal_bracket_key()
            .is_some_and(|(target, _)| navigates_only(target)),
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

    /// The arrays [`is_array_of_nodes`] admits: the gate test asserts each streams, and the
    /// agreement test that each evaluates to cursors, so the two cannot drift apart.
    const ARRAYS_OF_NODES: &[&str] = &[
        "[.]",
        "[.[]]",
        "[.[]?]",
        "[.users]",
        "[.a.b]",
        "[.[0]]",
        "[.users[]]",
        "[.users[] | select(.age > 30)]",
        "[.a.b[] | select(.x) | select(.y)]",
        "[.[][]]",
        "[select(.a)]",
        "([.[]])",
        ".users | [.[]]",
        ".a.b | [.[] | select(.x)]",
        ". | ([.[]])",
        ".missing | [.[]]",
        ".missing | [.[]?]",
        "[.missing]",
        "[.users[] | select(.name)]",
    ];

    /// #3475: yq streams an array whose every element is a node of the document, and keeps every
    /// other array on the DOM route. jq's own gate is unchanged: it never sees these shapes.
    #[test]
    fn test_array_of_nodes_gate_admits_only_provable_nodes_3475() {
        let parse = |f: &str| {
            let program = succinctly::jq::parse_program_with_mode_and_extensions(
                f,
                succinctly::jq::ParserMode::Yq,
                false,
            )
            .expect("parses");
            program.expr
        };
        for streams in ARRAYS_OF_NODES
            .iter()
            .copied()
            .filter(|f| !f.contains("missing"))
        {
            let expr = parse(streams);
            assert!(is_array_of_nodes(&expr), "{streams} should stream");
            assert!(
                !can_use_m2_streaming(&expr),
                "{streams}: the shared gate (jq's too) stays as it was"
            );
        }
        for stays in [
            // A field or index after a fan-out can be missing: an owned `null` among the nodes.
            "[.[] | .a]",
            "[.users[].name]",
            "[.users[] | select(.a) | .b]",
            "[.users[] | .a | select(.b)]",
            // A stage that computes, a slice, and `,` (not jq's in yq: `[., .]` has one element).
            "[.[] | .a + 1]",
            "[.[] | length]",
            "[.[] | keys]",
            "[.[1:]]",
            "[., .]",
            "[.a, .b]",
            // A generator index answers several nodes.
            ".[0,1] | [.[]]",
            ".[\"a\",\"b\"] | [.[]]",
            "[.[0,1]]",
            // A prefix that is not one node, or an array that feeds a later stage.
            ".[] | [.a]",
            ".[] | [.[]]",
            "sort_by(.id) | [.[]]",
            "reverse | [.[]]",
            "map(.) | [.[]]",
            "[.[]] | .[0]",
            "[.[]] | length",
            "{a: [.[]]}",
        ] {
            assert!(
                !is_array_of_nodes(&parse(stays)),
                "{stays} should keep the DOM route"
            );
        }
        // What already streams is the shared gate's, not this one's.
        assert!(!is_array_of_nodes(&parse(".a | map(.)")));
        assert!(!is_array_of_nodes(&parse(".[]")));
    }

    /// #3475: the claim [`is_array_of_nodes`] rests on, checked against the evaluator: an admitted
    /// shape never answers an owned element, over documents where a key is missing, a value is
    /// `null`, a block scalar, an alias or a scalar root.
    #[test]
    fn array_of_nodes_always_evaluates_to_cursors_3475() {
        use succinctly::jq::eval_generic::{eval_with_cursor_using, GenericResult, LazyElem};
        use succinctly::jq::{OwnedValue, YqSemantics};
        use succinctly::yaml::YamlIndex;
        let shapes = ARRAYS_OF_NODES;
        let docs = [
            "users:\n  - {name: a, age: 31}\n  - {age: 20}\n  - ~\n  - |\n    block\n",
            "a: {b: [{x: 1, y: 2}, {x: 1}, 3, null]}\n",
            "- {a: 1}\n- {b: 2}\n- &n {a: 3}\n- *n\n",
            "42\n",
            "null\n",
            "",
            "{}\n",
            "[]\n",
        ];
        for doc in docs {
            // The empty document has no root to evaluate against.
            let Ok(index) = YamlIndex::build_yq(doc.as_bytes()) else {
                continue;
            };
            let root = index.root(doc.as_bytes());
            let Some(cursor) = root.first_child() else {
                continue;
            };
            for shape in shapes {
                let program = succinctly::jq::parse_program_with_mode_and_extensions(
                    shape,
                    succinctly::jq::ParserMode::Yq,
                    false,
                )
                .expect("parses");
                assert!(is_array_of_nodes(&program.expr), "{shape}");
                match eval_with_cursor_using::<YqSemantics, _>(&program.expr, cursor) {
                    GenericResult::LazySeq(seq) => {
                        let items = seq.as_ref().clone().drain_atomic().expect("drains");
                        assert!(
                            items.iter().all(|item| matches!(item, LazyElem::Cursor(_))),
                            "{shape} over {doc:?} holds an owned element"
                        );
                    }
                    // An empty array has no element to be owned, a lone `null` is a missing path
                    // (both routes print it alike), and a failure prints nothing.
                    GenericResult::Owned(OwnedValue::Array(items)) => {
                        assert!(
                            items.is_empty() || matches!(&items[..], [OwnedValue::Null]),
                            "{shape} over {doc:?}: {items:?}"
                        );
                    }
                    GenericResult::Error(_) | GenericResult::None => {}
                    _ => panic!("{shape} over {doc:?} did not answer cursors"),
                }
            }
        }
    }

    /// #2416 phase 2: a path-context pipe the cursor walk takes whole streams
    /// through M2 (its outputs are live cursors or small `key`/`path`
    /// values), so a float scalar keeps its source spelling (#2419) instead
    /// of the DOM path's `1e+19`. A pipe the walk declines, or only takes
    /// the head of, stays on the DOM path; so does a bare `key`/`parent`
    /// with no pipe, which never reaches the walk at all.
    #[test]
    fn test_path_context_pipes_stream_when_the_walk_takes_them_whole_2416() {
        let parse = |f: &str| succinctly::jq::parse(f).expect("parses");
        assert!(can_use_m2_streaming(&parse(".outer.big | parent")));
        assert!(can_use_m2_streaming(&parse(".a | key")));
        assert!(can_use_m2_streaming(&parse("(.a, .b) | path")));
        assert!(can_use_m2_streaming(&parse(".a.b | parent(2)")));
        assert!(!can_use_m2_streaming(&parse(".a | key | tostring")));
        assert!(!can_use_m2_streaming(&parse("(.a | parent) | .x + 0")));
        assert!(!can_use_m2_streaming(&parse("[.[] | key]")));
        assert!(!can_use_m2_streaming(&parse("parent")));
        assert!(!can_use_m2_streaming(&parse("key")));
    }

    /// #1344: `..` and the `recurse` spellings that are the same walk stream
    /// their node cursors, so a node prints with its duplicate keys instead of
    /// being re-collapsed by the DOM path; a computing stage after it does not.
    #[test]
    fn recursive_descent_streams_cursors_1344() {
        let parse = |f: &str| succinctly::jq::parse(f).expect("parses");
        assert!(can_use_m2_streaming(&parse("..")));
        assert!(can_use_m2_streaming(&parse(
            ".. | select(tag == \"!!map\")"
        )));
        assert!(can_use_m2_streaming(&parse("recurse")));
        assert!(!can_use_m2_streaming(&parse(".. | length")));
        assert!(!can_use_m2_streaming(&parse("[..]")));
    }

    /// #4380: `--eval-all` takes the per-document route only for a filter real
    /// yq answers the same way in both modes. Each `false` row is one measured
    /// against yq v4.53.3 to differ between `ea` and plain evaluation over
    /// two documents: `[.]` collects the whole list, `,` runs its left operand
    /// over every document before its right, `.a = .b` and `|= . + 1` read the
    /// list, and `keys`/`$x`/`select` are builtins or bindings whose list
    /// handling this gate does not claim.
    #[test]
    fn eval_all_per_document_gate_admits_only_navigation_and_literal_writes_4380() {
        let parse = |f: &str| {
            succinctly::jq::parse_program_with_mode_and_extensions(
                f,
                succinctly::jq::ParserMode::Yq,
                true,
            )
            .expect("parses")
            .expr
        };
        for f in [
            ".",
            ".a",
            ".a.b",
            ".a?",
            "(.a)",
            ".a | .b",
            ".[]",
            ".[0]",
            ".[1:]",
            "..",
            ".a[]",
            ".a[0]",
            ".a = 9",
            ".a = \"s\"",
            ".a = null",
            ".a.b |= 5",
            ".a //= 7",
            ".n += 1",
            ".n -= 1",
            ".[] = 1",
            ".[0] = 5",
            ".l[1:] = 5",
            ".. = 1",
            ".a[].b |= 5",
            "del(.a)",
            "del(.a.b)",
            "del(.[0])",
            // #4380: a test of the document's origin in front of the same stages.
            "select(fi == 1)",
            "select(di == 0) | .a",
            "select(fi == 0 or fi == 1) | .a.b | .c",
            "select(fi == 1) | .a = 5",
            "select(fi != 0) | select(di == 0) | del(.a)",
            "select(1 < fi)",
        ] {
            assert!(eval_all_matches_per_document(&parse(f)), "{f}");
        }
        for f in [
            "[.]",
            ".a, .b",
            "(.a, .b)",
            ".a = .b",
            ".a |= . + 1",
            ".a |= .b",
            ".n += .m",
            ".a //= .b",
            "keys",
            "length",
            ".a | length",
            "select(.a == 1)",
            "di",
            "file_index",
            ". as $x | $x",
            "$x",
            "{a: .a}",
            ".a + 1",
            "del(.a, .b)",
            "del(.a | length)",
            ".a style = \"flow\"",
            // #4380: a test that reads the document, or a stage that is not per document, after one.
            "select(fi == .a)",
            "select(.a == 1) | .b",
            "select(fi == 0) | keys",
            "select(fi == 0) | .a = .b",
            "select(fi == 0), select(fi == 1)",
            ".a | select(fi == 0)",
        ] {
            assert!(!eval_all_matches_per_document(&parse(f)), "{f}");
        }
    }
}
