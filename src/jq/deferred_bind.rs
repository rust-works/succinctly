//! When `EXPR as $x | body` may leave `$x` undecoded (#3856, phase 3).
//!
//! A bind used to decode the bound node into an `OwnedValue` whether or not
//! `$x` was ever read: `. as $root | .users[] | .name` paid a full decode of
//! the document (6.5x the memory, 4-5x the time of the same filter without the
//! bind). The substitution itself is nearly free; the decode is the cost.
//!
//! A deferred bind substitutes an [`Expr::DeferredVar`] naming the node
//! instead. The use site re-resolves it to a cursor (`DocumentCursor::at_node_id`)
//! from the ambient cursor, so `$x` is validated and decoded only where
//! something reads it, like any other cursor. It carries **no value**: an
//! `Expr` is `'static` and cannot hold a cursor, so a read with no cursor of
//! the bind's own document has nothing to fall back on. [`deferred_bind_is_sound`]
//! is what keeps that from happening, and the use site raises an internal
//! error rather than guess if it ever does.
//!
//! # The predicate
//!
//! It answers: is every `$x` in `body` read where the ambient input is
//! *definitely* a cursor of the document `$x` was bound from? It tracks the
//! ambient input through the body as an [`Ambient`]:
//!
//! - [`Ambient::Cursor`]: a node of the document. The bind's own input starts
//!   here (the caller checks that the bound node belongs to the ambient
//!   cursor's document), and so does every element `.[]` yields.
//! - [`Ambient::Nav`]: a cursor, **or** an owned `null`. A field or index read
//!   of a node answers a cursor when the member exists and an owned `null`
//!   (no cursor) when it does not, so `.missing | $x` has nothing to resolve
//!   from. `.[]` of a `Nav` is back to `Cursor`, because an owned `null` has no
//!   elements to yield.
//! - [`Ambient::Opaque`]: anything else -- a literal, a construction, a
//!   computed value.
//!
//! `$x` is admitted only under `Cursor`. The body must be built from the forms
//! listed in [`walk`]; a form outside the list is fine when it does not mention
//! `$x` (it only degrades the ambient to `Opaque` for what follows) and makes
//! the bind eager when it does. That is a whitelist on purpose: the evaluator
//! hands some forms to the owned evaluator, which has no cursor to resolve a
//! deferred variable against, and a form not known to stay native must not
//! carry one. It is kept separate from `array_route_stage_is_pure_navigation`
//! and `path_expr_is_cursor_navigable` (#3501): three predicates that look
//! alike and answer different questions must not widen together.
//!
//! # What a read costs
//!
//! Nothing more than before. The eager bind already resolved `$x` back to its
//! document node wherever the use site held a cursor of the same document
//! (#2072), and only fell back to its decoded value where there was none --
//! which is exactly the position this predicate refuses. So a read it admits
//! navigates the document as it always did, and deferring removes only the
//! decode at the bind. (A read that walks a large container per element, such
//! as `$root.nodes[.from]` over a big array, was O(n) per read before and still
//! is: that is the array index walking its length, not this change.)
//!
//! No `path(...)`, assignment, `del`, `reduce`, `foreach`, `def` or call is on
//! the list, so a deferred variable never reaches the path resolver and its
//! many readers of `Tracked::value`, the embed table or storage identity
//! (#2889, #3134, #3177).

use alloc::rc::Rc;

use super::document::DocumentCursor;
use super::expr::{BindOrigin, Builtin, Expr, ObjectKey};
use super::walk::any_subexpr;

/// What is known about the ambient input at a point in the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Ambient {
    /// Not known to be a node of the document.
    Opaque,
    /// A node of the document, or an owned `null` standing for an absent one.
    Nav,
    /// A node of the document.
    Cursor,
}

impl Ambient {
    /// What is known of a value that is one of `self` or one of `other`.
    fn join(self, other: Self) -> Self {
        self.min(other)
    }
}

/// Whether `body` may read `$var` through a deferred binding -- see the
/// module documentation. `false` keeps the bind eager, exactly as before.
pub(crate) fn deferred_bind_is_sound(body: &Expr, var: &str) -> bool {
    // A body that never names the variable is trivially sound (the cost
    // `. as $x | 1` used to pay), and the common case -- one scan, no walk.
    !mentions_var(body, var) || walk(body, var, Ambient::Cursor).is_some()
}

/// Whether `expr` reads `$var` anywhere. Over-approximate: a nested binder
/// of the same name still counts, which only keeps a bind eager.
pub(crate) fn mentions_var(expr: &Expr, var: &str) -> bool {
    any_subexpr(expr, &mut |e| matches!(e, Expr::Var(name) if name == var))
}

/// The ambient after `expr` runs against `at`, or `None` when `expr` reads
/// `$var` somewhere a deferred binding cannot be resolved.
fn walk(expr: &Expr, var: &str, at: Ambient) -> Option<Ambient> {
    // Everything outside the whitelist: harmless without the variable, which
    // then leaves the ambient unknown for any later stage.
    let opaque = |e: &Expr| (!mentions_var(e, var)).then_some(Ambient::Opaque);
    match expr {
        Expr::Var(name) if name == var => (at == Ambient::Cursor).then_some(Ambient::Cursor),
        Expr::Identity => Some(at),
        // A member that exists is a cursor; one that does not is an owned
        // `null` -- so the result is `Nav` whatever the input was, unless the
        // input is already opaque.
        Expr::Field(_) | Expr::Index { .. } => Some(match at {
            Ambient::Opaque => Ambient::Opaque,
            _ => Ambient::Nav,
        }),
        // Only an actual container has elements to yield, and a container
        // reached from a node is a node: `.[]` always yields cursors.
        Expr::Iterate => Some(match at {
            Ambient::Opaque => Ambient::Opaque,
            _ => Ambient::Cursor,
        }),
        Expr::Paren(inner) | Expr::Optional(inner) => walk(inner, var, at),
        Expr::Pipe(stages) => stages
            .iter()
            .try_fold(at, |ambient, stage| walk(stage, var, ambient)),
        Expr::Comma(branches) => branches.iter().try_fold(Ambient::Cursor, |joined, branch| {
            Some(joined.join(walk(branch, var, at)?))
        }),
        Expr::Compare { left, right, .. } | Expr::Arithmetic { left, right, .. } => {
            walk(left, var, at)?;
            walk(right, var, at)?;
            Some(Ambient::Opaque)
        }
        Expr::And(left, right) | Expr::Or(left, right) => {
            walk(left, var, at)?;
            walk(right, var, at)?;
            Some(Ambient::Opaque)
        }
        Expr::Negate(inner) => {
            walk(inner, var, at)?;
            Some(Ambient::Opaque)
        }
        Expr::Alternative(left, right) => Some(walk(left, var, at)?.join(walk(right, var, at)?)),
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            walk(cond, var, at)?;
            Some(walk(then_branch, var, at)?.join(walk(else_branch, var, at)?))
        }
        // The handler runs on the error value, not on the ambient.
        Expr::Try { expr, catch } => {
            let body = walk(expr, var, at)?;
            match catch {
                Some(handler) => walk(handler, var, Ambient::Opaque).map(|_| Ambient::Opaque),
                None => Some(body),
            }
        }
        Expr::Array(inner) => {
            walk(inner, var, at)?;
            Some(Ambient::Opaque)
        }
        Expr::Object(entries) => {
            for entry in entries {
                if let ObjectKey::Expr(key) = &entry.key {
                    walk(key, var, at)?;
                }
                walk(&entry.value, var, at)?;
            }
            Some(Ambient::Opaque)
        }
        // `select` hands its input on unchanged.
        Expr::Builtin(Builtin::Select(cond)) => {
            walk(cond, var, at)?;
            Some(at)
        }
        // A nested bind runs its body on the same ambient the bind sees, and
        // its source is read there too. The same name shadows ours.
        Expr::As {
            expr,
            var: inner,
            body,
        } => {
            walk(expr, var, at)?;
            if inner == var {
                Some(Ambient::Opaque)
            } else {
                walk(body, var, at)
            }
        }
        other => opaque(other),
    }
}

/// [`DocumentCursor::at_node_id`] for a deferred binding's node, against any
/// cursor of the document it was bound from. `None` for a cursor of another
/// document: the token is checked before the id is trusted, since an id from
/// one document is usually in range in another.
pub(crate) fn deferred_bind_cursor<C: DocumentCursor>(
    node: &Rc<BindOrigin>,
    anchor: &C,
) -> Option<C> {
    match node.as_ref() {
        BindOrigin::Node { node, document } if *document == anchor.document_token() => {
            anchor.at_node_id(*node)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jq::parse;

    fn sound(filter: &str) -> bool {
        // `. as $x | body`: split off the body the way the parser does.
        match parse(filter).expect("parses") {
            Expr::As { var, body, .. } => deferred_bind_is_sound(&body, &var),
            other => panic!("expected an `as` bind, got {other:?}"),
        }
    }

    #[test]
    fn unused_binding_is_sound() {
        assert!(sound(". as $x | 1"));
        assert!(sound(". as $x | .a.b | length"));
    }

    #[test]
    fn reads_at_the_binds_own_input_are_sound() {
        assert!(sound(". as $x | $x"));
        assert!(sound(". as $x | $x.a.b"));
        assert!(sound(". as $x | [$x, 1]"));
        assert!(sound(r". as $x | {k: $x.a, n: .b}"));
        assert!(sound(". as $x | $x.a == .b"));
        assert!(sound(". as $x | if .a then $x else . end"));
        assert!(sound(". as $x | .a // $x"));
        assert!(sound(". as $x | try $x.a catch 1"));
    }

    #[test]
    fn reads_under_an_iteration_are_sound() {
        assert!(sound(". as $x | .users[] | $x.meta"));
        assert!(sound(". as $x | .users[] | select(.id == $x.limit)"));
        assert!(sound(". as $x | .users[] | {a: $x.a}"));
        assert!(sound(". as $x | .[] | .[] | $x"));
    }

    #[test]
    fn nested_bind_inherits_the_ambient() {
        assert!(sound(". as $x | .a as $y | [$x, $y]"));
        // A same-named inner binder shadows: `$x` below it is not ours.
        assert!(sound(". as $x | 1 as $x | $x"));
    }

    #[test]
    fn a_read_after_a_stage_that_may_lose_the_cursor_is_refused() {
        // A missing member answers an owned `null`: no cursor to resolve.
        assert!(!sound(". as $x | .missing | $x"));
        assert!(!sound(". as $x | .a.b | $x"));
        assert!(!sound(". as $x | .[0] | $x"));
        // Computed values have no cursor at all.
        assert!(!sound(". as $x | 1 | $x"));
        assert!(!sound(". as $x | length | $x"));
        assert!(!sound(r#". as $x | "a" | $x"#));
        assert!(!sound(". as $x | [.[]] | $x"));
        assert!(!sound(". as $x | {a: 1} | $x"));
        assert!(!sound(". as $x | (.a, 1) | $x"));
        assert!(!sound(". as $x | .a[] | .b | $x"));
    }

    #[test]
    fn a_catch_handler_runs_on_the_error_value() {
        assert!(!sound(". as $x | try error(1) catch $x"));
    }

    #[test]
    fn forms_outside_the_whitelist_that_read_the_variable_are_refused() {
        assert!(!sound(". as $x | reduce .[] as $i (0; . + $x)"));
        assert!(!sound(". as $x | map($x)"));
        assert!(!sound(". as $x | path($x)"));
        assert!(!sound(". as $x | $x |= 1"));
        assert!(!sound(". as $x | def f: $x; f"));
        assert!(!sound(". as $x | $x as [$a] | $a"));
        assert!(!sound(". as $x | foreach .[] as $i (0; . + 1; $x)"));
        assert!(!sound(". as $x | to_entries | $x"));
    }

    #[test]
    fn forms_outside_the_whitelist_are_fine_without_the_variable() {
        assert!(sound(". as $x | reduce .[] as $i (0; . + $i)"));
        assert!(sound(". as $x | map(.a) | length"));
        assert!(sound(". as $x | path(.a)"));
    }

    #[test]
    fn join_prefers_the_weaker_ambient() {
        assert_eq!(Ambient::Cursor.join(Ambient::Nav), Ambient::Nav);
        assert_eq!(Ambient::Nav.join(Ambient::Opaque), Ambient::Opaque);
        assert_eq!(Ambient::Cursor.join(Ambient::Cursor), Ambient::Cursor);
    }
}
