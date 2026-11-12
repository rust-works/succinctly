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

use succinctly::jq::{Builtin, Expr, FormatType, Literal};

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
pub(crate) fn results_have_no_document(expr: &Expr) -> bool {
    matches!(result_node(expr, ROOT), Some(node) if !node.document)
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
/// step (an absent one is `null`), alone or in a pipe of them.
fn yields_one(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(_) | Expr::Identity | Expr::Field(_) | Expr::Index { .. } => true,
        Expr::Paren(inner) => yields_one(inner),
        Expr::Pipe(stages) => stages.iter().all(yields_one),
        _ => false,
    }
}

/// The node of every result `expr` yields over a node `input`, or `None` where it is not known (or
/// differs between results).
fn result_node(expr: &Expr, input: Node) -> Option<Node> {
    match expr {
        Expr::Identity => Some(input),
        Expr::Paren(inner) | Expr::Optional(inner) => result_node(inner, input),
        Expr::Field(_) | Expr::Index { .. } | Expr::Iterate => Some(child(input)),
        Expr::IndexExpr { target, .. } => result_node(target, input).map(child),
        Expr::Pipe(stages) => stages
            .iter()
            .try_fold(input, |node, stage| result_node(stage, node)),
        Expr::Comma(items) => {
            let mut nodes = items.iter().map(|item| result_node(item, input));
            let first = nodes.next()??;
            nodes.try_fold(first, |acc, node| either(acc, node?))
        }
        Expr::Literal(_) => Some(FRESH),
        Expr::Array(_) | Expr::Not => Some(replacement(input)),
        // Interpolation builds a new string node whatever its input.
        Expr::StringInterpolation(_) => Some(FRESH),
        Expr::Object(_) => Some(Node {
            document: input.document,
            parent: false,
        }),
        // A comparison and `and`/`or` build their boolean as a replacement of the left operand
        // (`createBooleanCandidate(lhs)`), or of the right one where the left yields nothing, so a
        // left operand that always yields one result decides.
        Expr::Compare { left, right, .. } | Expr::And(left, right) | Expr::Or(left, right) => {
            let left_node = replacement(result_node(left, input)?);
            if yields_one(left) {
                Some(left_node)
            } else {
                either(left_node, replacement(result_node(right, input)?))
            }
        }
        // Arithmetic copies the right operand where the left is null and the left one otherwise, so a
        // left literal that is not `null` decides; elsewhere both must agree. `//` is its left
        // operand where that is truthy, which depends on the values.
        Expr::Arithmetic { left, right, .. } => match left.as_ref() {
            Expr::Literal(literal) if !matches!(literal, Literal::Null) => Some(FRESH),
            _ => either(result_node(left, input)?, result_node(right, input)?),
        },
        Expr::Alternative(left, right) => {
            either(result_node(left, input)?, result_node(right, input)?)
        }
        Expr::If {
            then_branch,
            else_branch,
            ..
        } => either(
            result_node(then_branch, input)?,
            result_node(else_branch, input)?,
        ),
        Expr::Try { expr, catch: None } => result_node(expr, input),
        // The body runs over the input; a `$name` it reads is not modelled (`Var` is unknown).
        Expr::As { body, .. } => result_node(body, input),
        Expr::Format(
            FormatType::Text
            | FormatType::Json
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
    use succinctly::jq;

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
            "del(.a) | \"x\"",
        ] {
            assert!(!fresh(filter), "{filter}");
        }
    }
}
