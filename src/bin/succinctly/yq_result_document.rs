//! Whether every result of a yq filter is a node yq builds with no document (#7019).
//!
//! yq's printer writes `---` before a result whose document index differs from the previous
//! result's (`printer.go`, v4.53.3). A node reads its index from the root of its `Parent` chain
//! (`CandidateNode.GetDocument`), so what decides it is how the operator that made the result
//! built it:
//!
//! - a navigation step returns a child of its input, which reads its document's index;
//! - `CreateReplacement` (behind `length`, `keys`, `tag`, `[...]`, a comparison, `@json`, ...)
//!   takes its input's parent, so a replacement of a document's root has no parent and index 0,
//!   while a replacement of a child keeps the child's document;
//! - a literal is a fresh node, index 0;
//! - `{...}` copies its input's index onto a node with no parent.
//!
//! A result with index 0 compares equal to every other one, and to the first document's, so a
//! filter whose results all have index 0 prints no `---` at all. That is the one case decided
//! here, statically and conservatively: [`results_have_no_document`] answers `true` only when every
//! stage is one whose node yq builds is known, and every other filter keeps the separator before
//! each document's first result. Results whose index depends on the data (`map` over a document's
//! node, which copies an empty input but builds a fresh sequence otherwise), or that mix the two
//! within one document (`.a, "x"`, `..`), are not modelled; they are recorded in
//! `docs/compliance/yq/limitations.md`.

use succinctly::jq::{self, Builtin, Expr, FormatType, Literal};

/// The two facts about a yq node that decide the document index of what is built from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Node {
    /// Its parent chain ends at an input document, so it reads that document's index.
    document: bool,
    /// It has a parent, which a replacement of it takes over.
    parent: bool,
}

/// A document's root: it carries the document's index and has no parent.
const ROOT: Node = Node {
    document: true,
    parent: false,
};

/// A node built from nothing: index 0, no parent.
const FRESH: Node = Node {
    document: false,
    parent: false,
};

/// Whether every result `expr` yields over a document has yq's document index 0, so that yq writes
/// no `---` between them.
///
/// A filter with `split_doc` anywhere is never one: that operator gives each result a document of
/// its own, and its separators are written by `SplitDocState`, which `no_doc` would silence.
pub(crate) fn results_have_no_document(expr: &Expr) -> bool {
    !jq::walk::contains_builtin(expr, |b| matches!(b, Builtin::SplitDoc))
        && matches!(
            result_node(expr, ROOT, Cx { scope: None, budget: &core::cell::Cell::new(BUDGET) }),
            Some(node) if !node.document
        )
}

/// What `CreateReplacement` builds from `input`: its parent, and through it its document, or a
/// fresh node where it has none.
fn replacement(input: Node) -> Node {
    if input.parent {
        input
    } else {
        FRESH
    }
}

/// A child of `input`, as a navigation step returns.
fn child(input: Node) -> Node {
    Node {
        document: input.document,
        parent: true,
    }
}

/// The node of results that may come from either of two nodes: a node with no document stays one
/// whatever is built from it, so two of those agree whatever their parents.
fn either(a: Node, b: Node) -> Option<Node> {
    if !a.document && !b.document {
        Some(FRESH)
    } else {
        (a == b).then_some(a)
    }
}

/// Whether `expr` yields exactly one result over any input: a literal, `.`, or a field or index
/// step (an absent one is `null`), alone or in a pipe of them. Narrower than the evaluator's own
/// single-value predicates on purpose: it is asked of the filter's text, before any value exists.
fn yields_one(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(_) | Expr::Identity | Expr::Field(_) | Expr::Index { .. } => true,
        Expr::Paren(inner) => yields_one(inner),
        Expr::Pipe(stages) => stages.iter().all(yields_one),
        _ => false,
    }
}

/// The `$name`s bound around a subexpression, each with the node its value has (`None` where that
/// is not known).
struct Scope<'a> {
    name: &'a str,
    node: Option<Node>,
    outer: Option<&'a Self>,
}

fn lookup(scope: Option<&Scope<'_>>, name: &str) -> Option<Node> {
    let mut scope = scope;
    while let Some(binding) = scope {
        if binding.name == name {
            return binding.node;
        }
        scope = binding.outer;
    }
    None
}

/// The classification's state: the `$name`s in scope, and a work budget, so that a filter of
/// deeply nested pipes is given up on (and keeps its separators) rather than walked without bound.
#[derive(Clone, Copy)]
struct Cx<'a> {
    scope: Option<&'a Scope<'a>>,
    budget: &'a core::cell::Cell<u32>,
}

impl<'a> Cx<'a> {
    /// Takes one step from the budget, or answers `false` when it is spent.
    fn step(self) -> bool {
        let left = self.budget.get();
        self.budget.set(left.saturating_sub(1));
        left > 0
    }

    fn with(self, scope: &'a Scope<'a>) -> Self {
        Cx {
            scope: Some(scope),
            budget: self.budget,
        }
    }
}

/// Steps one classification may take; a filter needing more keeps its separators.
const BUDGET: u32 = 10_000;

/// Whether `stage` builds results with no document whatever node it is handed (a literal, `keys`,
/// interpolation), so what came before it in a pipe does not matter.
fn fresh_regardless(stage: &Expr, cx: Cx<'_>) -> bool {
    if !cx.step() {
        return false;
    }
    match stage {
        Expr::Literal(_) | Expr::StringInterpolation(_) => true,
        Expr::Object(entries) => entries.is_empty(),
        Expr::Builtin(Builtin::Keys | Builtin::KeysUnsorted | Builtin::WithEntries(_)) => true,
        Expr::Paren(inner) => fresh_regardless(inner, cx),
        Expr::Comma(items) => items.iter().all(|item| fresh_regardless(item, cx)),
        Expr::Alternative(left, _) => {
            matches!(left.as_ref(), Expr::Literal(literal) if truthy_literal(literal))
        }
        // After the last stage that is fresh whatever its input, the rest runs over a fresh node.
        Expr::Pipe(stages) => match stages.iter().rposition(|stage| fresh_regardless(stage, cx)) {
            Some(last) => matches!(
                pipe_node(&stages[last + 1..], FRESH, cx),
                Some(node) if !node.document
            ),
            None => false,
        },
        // The source's node is not known without an input, so `$var` reads as unknown here.
        Expr::As { var, body, .. } => {
            let bound = Scope {
                name: var,
                node: None,
                outer: cx.scope,
            };
            fresh_regardless(body, cx.with(&bound))
        }
        Expr::Var(name) => matches!(lookup(cx.scope, name), Some(node) if !node.document),
        _ => false,
    }
}

/// The node of what a pipe's `stages` yield over `input`.
fn pipe_node(stages: &[Expr], input: Node, cx: Cx<'_>) -> Option<Node> {
    // Not a `try_fold`: an unknown stage does not end the walk, since a later stage that is fresh
    // whatever its input still decides.
    let mut node = Some(input);
    for stage in stages {
        node = if fresh_regardless(stage, cx) {
            Some(FRESH)
        } else {
            node.and_then(|node| result_node(stage, node, cx))
        };
    }
    node
}

/// Whether a literal is truthy in yq: anything but `null` and `false`.
fn truthy_literal(literal: &Literal) -> bool {
    !matches!(literal, Literal::Null | Literal::Bool(false))
}

/// The node of every result `expr` yields over a node `input`, or `None` where it is not known (or
/// differs between results).
fn result_node(expr: &Expr, input: Node, cx: Cx<'_>) -> Option<Node> {
    if !cx.step() {
        return None;
    }
    match expr {
        Expr::Identity => Some(input),
        Expr::Paren(inner) | Expr::Optional(inner) => result_node(inner, input, cx),
        Expr::Field(_) | Expr::Index { .. } | Expr::Iterate => Some(child(input)),
        Expr::IndexExpr { target, .. } => result_node(target, input, cx).map(child),
        // A stage that builds fresh results over any input decides alone, so an unknown stage
        // before it does not matter (`del(.a) | "x"`).
        Expr::Pipe(stages) => pipe_node(stages, input, cx),
        Expr::Comma(items) => {
            let mut nodes = items.iter().map(|item| result_node(item, input, cx));
            let first = nodes.next()??;
            nodes.try_fold(first, |acc, node| either(acc, node?))
        }
        // `{}` is a literal in yq; any other `{...}` takes its input's document.
        Expr::Literal(_) => Some(FRESH),
        Expr::Object(entries) if entries.is_empty() => Some(FRESH),
        Expr::Object(_) => Some(Node {
            document: input.document,
            parent: false,
        }),
        Expr::Array(_) | Expr::Not => Some(replacement(input)),
        // Interpolation builds a new string node whatever its input.
        Expr::StringInterpolation(_) => Some(FRESH),
        // A comparison and `and`/`or` build their boolean as a replacement of the left operand
        // (`createBooleanCandidate(lhs)`), or of the right one where the left yields nothing, so a
        // left operand that always yields one result decides.
        Expr::Compare { left, right, .. } | Expr::And(left, right) | Expr::Or(left, right) => {
            let left_node = replacement(result_node(left, input, cx)?);
            if yields_one(left) {
                Some(left_node)
            } else {
                either(left_node, replacement(result_node(right, input, cx)?))
            }
        }
        // Arithmetic copies the right operand where the left is null and the left one otherwise, so a
        // left literal that is not `null` decides; elsewhere both must agree.
        Expr::Arithmetic { left, right, .. } => match left.as_ref() {
            Expr::Literal(literal) if !matches!(literal, Literal::Null) => Some(FRESH),
            _ => either(
                result_node(left, input, cx)?,
                result_node(right, input, cx)?,
            ),
        },
        // `//` is its left operand where that is truthy and its right one otherwise: a literal on
        // the left decides which; elsewhere both must agree.
        Expr::Alternative(left, right) => match left.as_ref() {
            Expr::Literal(literal) if truthy_literal(literal) => Some(FRESH),
            Expr::Literal(_) => result_node(right, input, cx),
            _ => either(
                result_node(left, input, cx)?,
                result_node(right, input, cx)?,
            ),
        },
        Expr::If {
            then_branch,
            else_branch,
            ..
        } => either(
            result_node(then_branch, input, cx)?,
            result_node(else_branch, input, cx)?,
        ),
        Expr::Try { expr, catch: None } => result_node(expr, input, cx),
        // The body runs over the input, with `$var` bound to the source's results.
        Expr::As { expr, var, body } => {
            let bound = Scope {
                name: var,
                node: result_node(expr, input, cx),
                outer: cx.scope,
            };
            result_node(body, input, cx.with(&bound))
        }
        Expr::Var(name) => lookup(cx.scope, name),
        Expr::Format(
            FormatType::Json
            | FormatType::Yaml
            | FormatType::Props
            | FormatType::YqJson(_)
            | FormatType::YqYaml(_),
        ) => Some(replacement(input)),
        Expr::Builtin(builtin) => builtin_node(builtin, input),
        _ => None,
    }
}

fn builtin_node(builtin: &Builtin, input: Node) -> Option<Node> {
    match builtin {
        // `keys` (which yq mode parses as `KeysUnsorted`, document order) and `with_entries` build
        // a new collection whatever their input.
        Builtin::Keys | Builtin::KeysUnsorted | Builtin::WithEntries(_) => Some(FRESH),
        Builtin::Length
        | Builtin::Has(_)
        | Builtin::Contains(_)
        | Builtin::ToEntries
        | Builtin::ToString
        | Builtin::ToJson
        | Builtin::Type
        | Builtin::Tag
        | Builtin::Anchor
        | Builtin::Style
        | Builtin::Kind
        | Builtin::Key
        | Builtin::Line
        | Builtin::Column
        | Builtin::FileIndex
        | Builtin::DocumentIndex
        | Builtin::Filename
        | Builtin::PathNoArg => Some(replacement(input)),
        Builtin::Select(_) => Some(input),
        // `map` copies an empty input and builds a fresh sequence from any other, so its result
        // has no document whatever the input when the input has none.
        Builtin::Map(_) if !input.document => Some(FRESH),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(filter: &str) -> bool {
        let mut program =
            jq::parse_program_with_mode_and_extensions(filter, jq::ParserMode::Yq, false)
                .expect("filter parses");
        // As `run_yq` does before it classifies.
        jq::resolve_func_calls(&mut program.expr).expect("calls resolve");
        results_have_no_document(&program.expr)
    }

    #[test]
    fn results_built_from_the_root_or_nothing_have_no_document_7019() {
        for filter in [
            "\"x\"",
            "[.a]",
            "length",
            "keys",
            "keys | .[0]",
            "tag",
            "to_json",
            "@props",
            "\"\\(.a)\"",
            "{\"k\": .a} | length",
            "1 + 1",
            ". == 1",
            "\"x\" // 1",
            "select(.a > 1) | \"x\"",
            "[.a] | map(.)",
            "\"x\", 1",
            ". as $x | \"x\"",
            "file_index",
            "document_index",
            ".a | keys",
            ".a | with_entries(.)",
            ".a | \"\\(.)\"",
            "1 == .n",
            "\"x\" and .a",
            "\"x\" + .s",
            "2 * .n",
            "{}",
            "1 // .a",
            "false // \"x\"",
            "\"x\" as $y | $y",
            "[.a] as $x | $x",
            ".a as $x | $x | \"y\"",
            "del(.a) | \"x\"",
            "(.a // 1) | \"x\"",
            "sort_keys(.) | keys",
        ] {
            assert!(fresh(filter), "{filter}");
        }
    }

    #[test]
    fn results_of_a_document_node_or_unknown_ones_keep_it_7019() {
        for filter in [
            ".",
            ".a",
            ".a | length",
            ".l | [.[]]",
            "{\"k\": .a}",
            ".a + 1",
            ".a | document_index",
            ".a == 1",
            "null + .a",
            ".[] == 1",
            ".z // \"x\"",
            ".a // 1",
            "map(.)",
            ".a, \"x\"",
            ".. | tag",
            "sort_keys(.)",
            "split_doc | \"x\"",
            ". as $x | $x",
            ".a as $x | $x",
            ".a as $x | \"y\" | $x",
            "false // .a",
            "(.[] | split_doc) as $x | \"x\"",
            "{\"a\": 1}",
        ] {
            assert!(!fresh(filter), "{filter}");
        }
    }

    /// Nested pipes each ask whether a stage is fresh before classifying it; the budget keeps a
    /// deep filter (up to the parser's nesting limit of 256) from taking time exponential in its
    /// depth, and only gives up (keeping the separators) once spent. On a thread of its own because
    /// parsing that depth needs more than a test thread's stack in a debug build.
    #[test]
    fn a_deeply_nested_filter_is_classified_in_bounded_work_7019() {
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(|| {
                let deep = |stage: &str| {
                    format!(
                        "{}\"x\"{}",
                        format!("({stage} | ").repeat(250),
                        ")".repeat(250)
                    )
                };
                let started = std::time::Instant::now();
                for stage in [".a", "del(.a)", ". as $v | $v"] {
                    let _ = fresh(&deep(stage));
                }
                assert!(started.elapsed() < std::time::Duration::from_secs(5));
                assert!(fresh(&deep(".a")));
            })
            .expect("spawn")
            .join()
            .expect("classified");
    }

    /// `try` parses under `--jq-extensions` only; without a `catch` its results are its body's.
    #[test]
    fn try_without_catch_is_its_body_7019() {
        let classify = |filter: &str| {
            let mut program =
                jq::parse_program_with_mode_and_extensions(filter, jq::ParserMode::Yq, true)
                    .expect("filter parses");
            jq::resolve_func_calls(&mut program.expr).expect("calls resolve");
            results_have_no_document(&program.expr)
        };
        assert!(classify("try \"x\""));
        assert!(!classify("try .a"));
        assert!(!classify("try \"x\" catch ."));
    }

    /// A spent budget answers "not known", so the filter keeps its separators.
    #[test]
    fn a_spent_budget_keeps_the_separators_7019() {
        let program =
            jq::parse_program_with_mode_and_extensions("\"x\"", jq::ParserMode::Yq, false)
                .expect("filter parses");
        let spent = core::cell::Cell::new(0);
        let cx = Cx {
            scope: None,
            budget: &spent,
        };
        assert_eq!(result_node(&program.expr, ROOT, cx), None);
        assert!(!fresh_regardless(&program.expr, cx));
        let ample = core::cell::Cell::new(BUDGET);
        let cx = Cx {
            scope: None,
            budget: &ample,
        };
        assert_eq!(result_node(&program.expr, ROOT, cx), Some(FRESH));
        assert!(fresh_regardless(&program.expr, cx));
    }
}
