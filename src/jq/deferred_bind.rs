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
//! It answers two questions about every `$x` in `body`.
//!
//! **Is there a cursor to resolve it from?** It tracks the ambient input
//! through the body as an [`Ambient`]:
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
//! `$x` is admitted only under `Cursor`.
//!
//! **Does the read share what the eager bind shared?** The eager bind decoded
//! the node once and registered the result in the embed table (#2889), so every
//! later materialization of *that node* -- `{r: $x}`, `$x == .`, `[$x]`, `$x |
//! tojson` -- was handed the one `Rc` instead of decoding again. A deferred bind
//! registers a *lazy* entry for its node (#4036, `eval_generic::embed_lazy_push`):
//! the first materialization builds the value and fills the entry, and every
//! later one shares it. So a bare read of `$x` costs the one decode the eager
//! bind paid, moved to where it is needed, however many times it runs: once per
//! element of a collection is no different from once. That is what lets
//! `. as $x | .users[] | select(.id < 300) | {r: $x}` defer (it decoded the whole
//! document per element before the entry, 0.18 s against 6.7 s on a 1.4 MB
//! document, which is why a bare read under a repeated ambient was refused).
//!
//! What is still refused under a repeated ambient is a *member chain* something
//! goes on to consume (`$x.users | length`, `$x.users[0]`, `$x.nodes[.from]`):
//! that navigates a cursor per element where the eager bind walked an owned
//! value, and a cursor walks an array to its index (#4035). A chain nothing
//! consumes as a pipe stage (`$x.limit` as an operand, an object value, an array
//! element) reads a member, never the node, and is admitted.
//!
//! The entry needs a thread-local table, so without `std` ([`LAZY_SHARES`]) a
//! bare read under a repeated ambient is refused as before. A body that repeats
//! such a read is [`DeferredReads::Sharing`]; every other sound body is
//! [`DeferredReads::Navigating`].
//!
//! What is left is a constant factor, not a growth with the document: a body
//! that names the node bare `k` times decodes it once and shares it `k - 1`
//! times, and a read that only navigates (`$x.a`, `$x | keys`, `[$x]` counted)
//! skips the decode altogether.
//!
//! The body must be built from the forms listed in [`walk`]; a form outside the
//! list is fine when it does not mention `$x` (it only degrades the ambient to
//! `Opaque` for what follows) and makes the bind eager when it does. That is a
//! whitelist on purpose: the evaluator hands some forms to the owned evaluator,
//! which has no cursor to resolve a deferred variable against, and a form not
//! known to stay native must not carry one. It is kept separate from
//! `array_route_stage_is_pure_navigation` and `path_expr_is_cursor_navigable`
//! (#3501): three predicates that look alike and answer different questions must
//! not widen together.
//!
//! No `path(...)`, assignment, `del`, `reduce`, `foreach`, `def` or call is on
//! the list, so a deferred variable never reaches the path resolver and its
//! many readers of `Tracked::value`, the embed table or storage identity
//! (#2889, #3134, #3177).

use alloc::rc::Rc;

use super::document::DocumentCursor;
use super::expr::{BindOrigin, Builtin, Expr, ObjectKey};
use super::walk::any_subexpr;

/// Whether a deferred bind's whole-node reads can share one decoded copy
/// (#4036): the first materialization of the node fills a lazy embed-table
/// entry and every later one is handed the same `Rc`, as the eager bind's entry
/// did. That needs a thread-local table, so without `std` a repeated whole-node
/// read is still refused and the bind stays eager.
const LAZY_SHARES: bool = cfg!(feature = "std");

/// How a sound body reads `$var` (#4036).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeferredReads {
    /// Every read navigates the node, or runs once per bind: nothing is
    /// decoded more than the eager bind decoded it, bare reads included.
    Navigating,
    /// Some whole-node read runs once per element of a collection. It decodes
    /// the node once, in the first run, and every later one shares that copy
    /// through the lazy embed entry.
    Sharing,
}

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

/// The ambient input at a point in the body, and whether the stages that
/// produced it may have run it more than once per bind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct State {
    at: Ambient,
    /// A generator upstream (`.[]`) hands this point one input per element, so
    /// a read here runs once per element, not once per bind.
    repeated: bool,
    /// Whether a repeated whole-node read is admitted: the walk of a body that
    /// has the shared copy to lean on ([`LAZY_SHARES`]).
    shares: bool,
}

impl State {
    const START: Self = Self {
        at: Ambient::Cursor,
        repeated: false,
        shares: false,
    };

    fn join(self, other: Self) -> Self {
        Self {
            at: self.at.join(other.at),
            repeated: self.repeated || other.repeated,
            shares: self.shares,
        }
    }

    fn with(self, at: Ambient) -> Self {
        Self { at, ..self }
    }

    fn repeating(self, many: bool) -> Self {
        Self {
            repeated: self.repeated || many,
            ..self
        }
    }
}

/// Whether `body` may read `$var` through a deferred binding -- see the
/// module documentation. `false` keeps the bind eager, exactly as before.
#[cfg(test)]
pub(crate) fn deferred_bind_is_sound(body: &Expr, var: &str) -> bool {
    // A body that never names the variable is trivially sound (the cost
    // `. as $x | 1` used to pay).
    !mentions_var(body, var) || deferred_bind_reads(body, var).is_some()
}

/// How `body`, already known to name `$var`, reads it through a deferred
/// binding, or `None` when a read cannot be resolved or is not cheap: the walk
/// alone, so the caller's one scan for the name is not repeated.
///
/// The strict walk comes first: a body it admits never repeats a whole-node
/// read and needs no shared copy. Only one it refuses is walked again with
/// repeated whole-node reads admitted, where they are the sharing's to pay for.
pub(crate) fn deferred_bind_reads(body: &Expr, var: &str) -> Option<DeferredReads> {
    if walk(body, var, State::START, false).is_some() {
        return Some(DeferredReads::Navigating);
    }
    let sharing = State {
        shares: LAZY_SHARES,
        ..State::START
    };
    (LAZY_SHARES && walk(body, var, sharing, false).is_some()).then_some(DeferredReads::Sharing)
}

/// Whether `expr` reads `$var` anywhere. Over-approximate: a nested binder
/// of the same name still counts, which only keeps a bind eager.
pub(crate) fn mentions_var(expr: &Expr, var: &str) -> bool {
    any_subexpr(expr, &mut |e| matches!(e, Expr::Var(name) if name == var))
}

/// Whether `expr` may yield more than one output (or, for a form this does not
/// know, whether it might). A generator makes whatever is evaluated alongside
/// or after it run once per output.
fn may_yield_many(expr: &Expr) -> bool {
    match expr {
        Expr::Identity
        | Expr::Field(_)
        | Expr::Index { .. }
        | Expr::Literal(_)
        | Expr::Var(_)
        | Expr::Not
        // Collects whatever its body yields into one array.
        | Expr::Array(_) => false,
        Expr::Paren(inner) | Expr::Optional(inner) | Expr::Negate(inner) => may_yield_many(inner),
        Expr::Pipe(stages) => stages.iter().any(may_yield_many),
        Expr::Compare { left, right, .. }
        | Expr::Arithmetic { left, right, .. }
        | Expr::And(left, right)
        | Expr::Or(left, right)
        | Expr::Alternative(left, right) => may_yield_many(left) || may_yield_many(right),
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => may_yield_many(cond) || may_yield_many(then_branch) || may_yield_many(else_branch),
        Expr::Try { expr, catch } => {
            may_yield_many(expr) || catch.as_deref().is_some_and(may_yield_many)
        }
        Expr::Object(entries) => entries.iter().any(|entry| {
            may_yield_many(&entry.value)
                || matches!(&entry.key, ObjectKey::Expr(key) if may_yield_many(key))
        }),
        Expr::As { expr, body, .. } => may_yield_many(expr) || may_yield_many(body),
        _ => true,
    }
}

/// A stage that reads one member of an object: `.k`, `.k?`.
fn is_field_read(stage: &Expr) -> bool {
    match stage {
        Expr::Field(_) => true,
        Expr::Optional(inner) => matches!(inner.as_ref(), Expr::Field(_)),
        _ => false,
    }
}

/// The state after `expr` runs against `st`, or `None` when `expr` reads
/// `$var` somewhere a deferred binding cannot be resolved or is not cheap.
/// `piped` is whether a pipe stage consumes `expr`'s output.
fn walk(expr: &Expr, var: &str, st: State, piped: bool) -> Option<State> {
    // Everything outside the whitelist: harmless without the variable, which
    // then leaves the ambient unknown for any later stage, and (since it may
    // be a generator) repeated.
    let opaque = |e: &Expr| {
        (!mentions_var(e, var)).then_some(State {
            at: Ambient::Opaque,
            repeated: true,
            ..st
        })
    };
    match expr {
        // A repeated bare `$x` materializes the whole node per run, which is
        // free once the first run's copy is shared (`State::shares`); the
        // field-chain read is `walk_pipe`'s.
        Expr::Var(name) if name == var => (st.at == Ambient::Cursor && (st.shares || !st.repeated))
            .then_some(st.with(Ambient::Cursor)),
        Expr::Identity => Some(st),
        // A member that exists is a cursor; one that does not is an owned
        // `null` -- so the result is `Nav` whatever the input was, unless the
        // input is already opaque.
        Expr::Field(_) | Expr::Index { .. } => Some(st.with(match st.at {
            Ambient::Opaque => Ambient::Opaque,
            _ => Ambient::Nav,
        })),
        // Only an actual container has elements to yield, and a container
        // reached from a node is a node: `.[]` always yields cursors. One per
        // element, so everything after it is repeated.
        Expr::Iterate => Some(State {
            at: match st.at {
                Ambient::Opaque => Ambient::Opaque,
                _ => Ambient::Cursor,
            },
            repeated: true,
            ..st
        }),
        Expr::Paren(inner) | Expr::Optional(inner) => walk(inner, var, st, piped),
        Expr::Pipe(stages) => walk_pipe(stages, var, st, piped),
        Expr::Comma(branches) => branches
            .iter()
            .try_fold(None::<State>, |joined, branch| {
                let out = walk(branch, var, st, piped)?;
                Some(Some(joined.map_or(out, |j| j.join(out))))
            })?
            .or(Some(st)),
        Expr::Compare { left, right, .. }
        | Expr::Arithmetic { left, right, .. }
        | Expr::And(left, right)
        | Expr::Or(left, right) => {
            // The right operand is the outer loop (#768), so each operand runs
            // once per output of the other.
            walk(left, var, st.repeating(may_yield_many(right)), false)?;
            walk(right, var, st.repeating(may_yield_many(left)), false)?;
            Some(st.with(Ambient::Opaque).repeating(may_yield_many(expr)))
        }
        Expr::Negate(inner) => {
            walk(inner, var, st, false)?;
            Some(st.with(Ambient::Opaque).repeating(may_yield_many(inner)))
        }
        Expr::Alternative(left, right) => {
            Some(walk(left, var, st, piped)?.join(walk(right, var, st, piped)?))
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            walk(cond, var, st, false)?;
            // A branch runs once per output of the condition.
            let st = st.repeating(may_yield_many(cond));
            Some(walk(then_branch, var, st, piped)?.join(walk(else_branch, var, st, piped)?))
        }
        // The handler runs on the error value, not on the ambient.
        Expr::Try { expr, catch } => {
            let body = walk(expr, var, st, piped)?;
            match catch {
                Some(handler) => walk(handler, var, st.with(Ambient::Opaque), piped)
                    .map(|handled| handled.join(body.with(Ambient::Opaque))),
                None => Some(body),
            }
        }
        Expr::Array(inner) => {
            walk(inner, var, st, false)?;
            Some(st.with(Ambient::Opaque))
        }
        Expr::Object(entries) => {
            // Entries combine as a cartesian product, so one entry's generator
            // repeats the others.
            let st = st.repeating(may_yield_many(expr));
            for entry in entries {
                if let ObjectKey::Expr(key) = &entry.key {
                    walk(key, var, st, false)?;
                }
                walk(&entry.value, var, st, false)?;
            }
            Some(st.with(Ambient::Opaque))
        }
        // `select` hands its input on unchanged, once per truthy output.
        Expr::Builtin(Builtin::Select(cond)) => {
            walk(cond, var, st, false)?;
            Some(st.repeating(may_yield_many(cond)))
        }
        // A nested bind runs its body on the same ambient the bind sees, once
        // per output of its source; the source is read there too. The same
        // name shadows ours.
        Expr::As {
            expr,
            var: inner,
            body,
        } => {
            walk(expr, var, st, false)?;
            if inner == var {
                Some(st.with(Ambient::Opaque))
            } else {
                walk(body, var, st.repeating(may_yield_many(expr)), piped)
            }
        }
        other => opaque(other),
    }
}

/// [`walk`] over the stages of a pipe. A read of `$var` followed only by field
/// reads is one read of that field chain (`$x.a.b`), which is what a repeated
/// ambient admits; a bare `$var`, or any other stage after it, consumes the
/// whole node.
fn walk_pipe(stages: &[Expr], var: &str, st: State, piped: bool) -> Option<State> {
    let mut cur = st;
    let mut i = 0;
    while i < stages.len() {
        if matches!(&stages[i], Expr::Var(name) if name == var) && cur.repeated {
            let end = i
                + 1
                + stages[i + 1..]
                    .iter()
                    .take_while(|stage| is_field_read(stage))
                    .count();
            // A bare `$var` stage reads the whole node, which shares one copy
            // (`State::shares`); only the fields it navigates are refused.
            if cur.shares && end == i + 1 {
                if cur.at != Ambient::Cursor {
                    return None;
                }
                cur = cur.with(Ambient::Cursor);
                i += 1;
                continue;
            }
            if cur.at != Ambient::Cursor || end == i + 1 || end < stages.len() || piped {
                return None;
            }
            // At least one field read follows (a bare `$var` returned above), so
            // the result is a member, which may be absent.
            cur = cur.with(Ambient::Nav);
            i = end;
            continue;
        }
        let consumed = i + 1 < stages.len() || piped;
        cur = walk(&stages[i], var, cur, consumed)?;
        i += 1;
    }
    Some(cur)
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

    fn reads(filter: &str) -> Option<DeferredReads> {
        // `. as $x | body`: split off the body the way the parser does.
        let Expr::As { var, body, .. } = parse(filter).expect("parses") else {
            panic!("not an `as` bind") // patchcov: coverage tolerate-line reason="unreachable in a passing suite: every row is an `as` bind (#3856)"
        };
        deferred_bind_reads(&body, &var)
    }

    fn sound(filter: &str) -> bool {
        // `. as $x | body`: split off the body the way the parser does.
        let Expr::As { var, body, .. } = parse(filter).expect("parses") else {
            panic!("not an `as` bind") // patchcov: coverage tolerate-line reason="unreachable in a passing suite: every row is an `as` bind (#3856)"
        };
        deferred_bind_is_sound(&body, &var)
    }

    /// What a repeated bare read of the node is: shared through the lazy embed
    /// entry where there is one (#4036), refused where there is no table to
    /// hold it.
    fn sharing() -> Option<DeferredReads> {
        LAZY_SHARES.then_some(DeferredReads::Sharing)
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
    fn a_single_read_is_sound_however_it_is_consumed() {
        assert!(sound(". as $x | $x.users | length"));
        assert!(sound(". as $x | $x.users[0]"));
        assert!(sound(". as $x | $x | tojson"));
        assert!(sound(". as $x | ($x.a, $x.b) | length"));
    }

    #[test]
    fn a_repeated_read_is_sound_only_as_a_field_chain() {
        // The head of a field chain nothing consumes as a pipe stage.
        assert!(sound(". as $x | .users[] | $x.meta"));
        assert!(sound(". as $x | .users[] | $x.meta.n"));
        assert!(sound(". as $x | .users[] | select(.id == $x.limit)"));
        assert!(sound(". as $x | .users[] | {a: $x.a, b: [$x.b]}"));
        assert!(sound(
            ". as $x | .users[] | if .id > $x.min then 1 else 2 end"
        ));
        assert!(sound(". as $x | .users[] | $x.a?"));
    }

    #[test]
    fn a_repeated_read_of_the_whole_node_shares_one_copy() {
        // The eager bind decoded the node once and handed every later read the
        // one `Rc`; a deferred read has the lazy entry to do the same (#4036).
        for filter in [
            ". as $x | .users[] | $x",
            ". as $x | .users[] | {r: $x}",
            ". as $x | .users[] | select($x == .)",
            ". as $x | .users[] | [$x]",
            ". as $x | .[] | .[] | $x",
            ". as $x | .users[] | $x | length",
            ". as $x | .users[] | $x | tojson",
            ". as $x | .users[] | ($x, 1)",
            // A generator beside the read repeats it too.
            ". as $x | $x == (.[] | .b)",
            ". as $x | ($x | length) == (.[] | .b)",
            ". as $x | [.[] | $x]",
        ] {
            assert_eq!(reads(filter), sharing(), "{filter}");
        }
        // A comma beside it is a fixed number of branches, not a repetition.
        assert_eq!(
            reads(". as $x | ($x | length), (.[] | .b)"),
            Some(DeferredReads::Navigating)
        );
    }

    #[test]
    fn a_repeated_member_read_that_something_consumes_is_refused() {
        // Navigating the node on a cursor per element is not the whole-node
        // read the shared copy covers: the eager bind walked an owned value
        // here (#4035 is the array-index half of it).
        assert!(!sound(". as $x | .users[] | $x.users | length"));
        assert!(!sound(". as $x | .users[] | $x.users[0]"));
        assert!(!sound(". as $x | .users[] | $x.a | .b"));
        assert!(!sound(". as $x | .users[] | [$x.users[0]]"));
        assert!(!sound(". as $x | .edges[] | $x.nodes[.from]"));
        assert!(!sound(". as $x | .users[] | ($x.users | length)"));
        // And a bare read beside one is refused with it.
        assert!(!sound(". as $x | .users[] | ($x, $x.users[0])"));
    }

    #[test]
    fn operators_negation_and_computed_keys_are_walked() {
        assert!(sound(". as $x | -($x.a)"));
        assert!(sound(". as $x | try $x.a"));
        assert!(sound(". as $x | {($x.k): 1}"));
        assert!(sound(". as $x | {a: 1, ($x.k): $x.v}"));
        // A computed key that yields many repeats the other entries.
        assert_eq!(reads(". as $x | {(.[] | .k): $x}"), sharing());
        assert!(!sound(". as $x | {(.[] | .k): $x.a | length}"));
        // A negated value is computed, so there is no cursor to read from after it.
        assert!(!sound(". as $x | -(.[] | .a) | $x"));
        assert!(!sound(". as $x | try (.[] | .a) catch $x"));
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
