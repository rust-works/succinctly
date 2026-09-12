//! Compile-time resolution of function calls (#1473).
//!
//! Real jq resolves every function call *before* any input is read. A call to
//! an undefined function — or to an undefined arity of an existing one — is a
//! compile error: unconditional, unaffected by whether the branch containing
//! it is ever reached, and uncatchable by `try`/`catch` or `?`, since
//! compilation fails before evaluation (and thus before `try`) ever begins.
//! Exit code 3.
//!
//! succinctly resolves user-defined `def`s by static AST substitution
//! (`expand_func_calls`, `src/jq/eval.rs`), which runs *during* evaluation and
//! leaves an unresolvable call in place to be discovered lazily. Three
//! divergences followed, all confirmed live against jq 1.7.1:
//!
//! - an arity-mismatched call in a never-taken branch never errored at all;
//! - the error was swallowable by `try`/`?`;
//! - it surfaced as a runtime error (exit 5), not a compile error (exit 3).
//!
//! and one silently-wrong result, the severe case: substitution has no notion
//! of *lexical position*, so a body substituted into a later call site becomes
//! indistinguishable from a call genuinely written there, and a later `def`'s
//! own expansion pass resolves it. Both `def f(x): f(x; 99); def f(x; y): x +
//! y; f(1)` and `def f: g; def g: 42; f` are forward references real jq
//! rejects, and both computed a value instead.
//!
//! ## Why a *check* is enough
//!
//! The issue scoped this as needing a real resolution mechanism. It does not.
//! `src/jq/eval.rs` has exactly one evaluation arm for `Expr::FuncCall`, and it
//! routes unconditionally to `eval_func_call`, which always returns an error —
//! so *any* residual `Expr::FuncCall` reaching evaluation is already an error
//! today. This pass therefore introduces no new error class for a call that is
//! actually reached; it only moves the error earlier and extends it to the
//! unreached, caught and forward-referencing cases jq also rejects.
//!
//! `expand_func_calls`'s substitution model is left exactly as it was. The
//! programs it mis-resolves are simply rejected before it ever runs.
//!
//! ## Scope rules
//!
//! Only [`Expr::FuncDef`] changes function scope — `as`/`reduce`/`foreach`
//! patterns bind variables and `label` binds labels, never functions. Each rule
//! below was verified against jq 1.7.1 rather than taken from the manual:
//!
//! | program | jq 1.7.1 |
//! |---|---|
//! | `def f: 1; def g: f; g` | `1` — a def is visible to later siblings |
//! | `def f: g; def g: 42; f` | error — but *not* to earlier ones |
//! | `def f: def g: 1; g; g` | error — a nested def does not leak |
//! | `def f($a): a; f(1)` | `1` — a `$`-param binds the bare name too |
//! | `def f(g): g(1); f(.)` | error `g/1` — a param binds arity 0 only |
//! | `def f: 1; def g: f; def f: 2; g` | `1` — later same-arity def shadows |
//!
//! `Expr::FuncDef::params` distinguishes `def f($a)` from `def f(a)` (`Param`,
//! `src/jq/expr.rs` -- #2283), but this pass only ever needs
//! [`Param::name`](super::Param::name), the bare identifier either spelling
//! binds: jq's `def f($a): …` desugars to `def f(a): a as $a | …`, which
//! leaves `a` callable at arity 0 regardless of which spelling was written.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::walk::{builtin_kids, map_builtin_subexprs, BuiltinKids};
use super::{Expr, ObjectKey, StringPart};

/// A call this pass could not resolve to any in-scope `def`, parameter or
/// builtin — the compile error's payload.
///
/// Carries the name and arity rather than a formatted message: the two runners
/// word it differently (jq's `f/2 is not defined at <top-level>, line N:` with
/// the offending source line echoed, against yq's uniform `Error: …`), and
/// only the runner has the filter source to quote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedCall {
    /// The called name, as written (a module import arrives here already
    /// rewritten to its `namespace::name` form).
    pub name: String,
    /// How many arguments the call site passed.
    pub arity: usize,
}

impl core::fmt::Display for UnresolvedCall {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}/{} is not defined", self.name, self.arity)
    }
}

/// Every builtin the pinned jq (1.7.1) defines, as `(name, arity)`.
///
/// **Not** the builtins succinctly implements. succinctly lowers the ones it
/// does implement to typed `Builtin::*` variants at parse time, so those never
/// reach this pass as an `Expr::FuncCall` at all. This roster matters for the
/// opposite set: the ~45 jq builtins succinctly does *not* implement (the libm
/// family — `cbrt`, `hypot`, `fma`, `ldexp`, `frexp`, `lgamma`, `j0`/`j1`/`jn`,
/// `y0`/`y1`/`yn`, … — plus `JOIN/2..4`, `format/1`, `input_filename/0`,
/// `get_search_list/0`, `get_jq_origin/0`, `get_prog_origin/0`,
/// `strflocaltime/1`), which *do* reach it as bare calls.
///
/// Without this roster, `if false then cbrt else 1 end` would become a compile
/// error here where real jq compiles it happily — a regression this pass would
/// otherwise introduce, and the only false-positive class it has. Accepting the
/// name at compile time leaves a *reached* call to fail at runtime exactly as
/// it does today.
///
/// Generated by `./scripts/sync-jq-builtin-names.sh` and checked entry for
/// entry against `tests/data/jq-builtin-names.txt` by this module's own
/// `jq_builtin_roster_matches_the_pinned_capture`, so the two cannot drift
/// apart silently. That the roster is actually *consulted* is covered from the
/// CLI by `test_unimplemented_jq_builtins_still_compile_when_unreached_1473`
/// (`tests/jq_cli_tests.rs`).
const JQ_BUILTIN_ROSTER: &[(&str, usize)] = &[
    ("IN", 1),
    ("IN", 2),
    ("INDEX", 1),
    ("INDEX", 2),
    ("JOIN", 2),
    ("JOIN", 3),
    ("JOIN", 4),
    ("abs", 0),
    ("acos", 0),
    ("acosh", 0),
    ("add", 0),
    ("all", 0),
    ("all", 1),
    ("all", 2),
    ("any", 0),
    ("any", 1),
    ("any", 2),
    ("arrays", 0),
    ("ascii_downcase", 0),
    ("ascii_upcase", 0),
    ("asin", 0),
    ("asinh", 0),
    ("atan", 0),
    ("atan2", 2),
    ("atanh", 0),
    ("booleans", 0),
    ("bsearch", 1),
    ("builtins", 0),
    ("capture", 1),
    ("capture", 2),
    ("cbrt", 0),
    ("ceil", 0),
    ("combinations", 0),
    ("combinations", 1),
    ("contains", 1),
    ("copysign", 2),
    ("cos", 0),
    ("cosh", 0),
    ("debug", 0),
    ("debug", 1),
    ("del", 1),
    ("delpaths", 1),
    ("drem", 2),
    ("empty", 0),
    ("endswith", 1),
    ("env", 0),
    ("erf", 0),
    ("erfc", 0),
    ("error", 0),
    ("error", 1),
    ("exp", 0),
    ("exp10", 0),
    ("exp2", 0),
    ("explode", 0),
    ("expm1", 0),
    ("fabs", 0),
    ("fdim", 2),
    ("finites", 0),
    ("first", 0),
    ("first", 1),
    ("flatten", 0),
    ("flatten", 1),
    ("floor", 0),
    ("fma", 3),
    ("fmax", 2),
    ("fmin", 2),
    ("fmod", 2),
    ("format", 1),
    ("frexp", 0),
    ("from_entries", 0),
    ("fromdate", 0),
    ("fromdateiso8601", 0),
    ("fromjson", 0),
    ("fromstream", 1),
    ("gamma", 0),
    ("get_jq_origin", 0),
    ("get_prog_origin", 0),
    ("get_search_list", 0),
    ("getpath", 1),
    ("gmtime", 0),
    ("group_by", 1),
    ("gsub", 2),
    ("gsub", 3),
    ("halt", 0),
    ("halt_error", 0),
    ("halt_error", 1),
    ("has", 1),
    ("hypot", 2),
    ("implode", 0),
    ("in", 1),
    ("index", 1),
    ("indices", 1),
    ("infinite", 0),
    ("input", 0),
    ("input_filename", 0),
    ("input_line_number", 0),
    ("inputs", 0),
    ("inside", 1),
    ("isempty", 1),
    ("isfinite", 0),
    ("isinfinite", 0),
    ("isnan", 0),
    ("isnormal", 0),
    ("iterables", 0),
    ("j0", 0),
    ("j1", 0),
    ("jn", 2),
    ("join", 1),
    ("keys", 0),
    ("keys_unsorted", 0),
    ("last", 0),
    ("last", 1),
    ("ldexp", 2),
    ("length", 0),
    ("lgamma", 0),
    ("lgamma_r", 0),
    ("limit", 2),
    ("localtime", 0),
    ("log", 0),
    ("log10", 0),
    ("log1p", 0),
    ("log2", 0),
    ("logb", 0),
    ("ltrimstr", 1),
    ("map", 1),
    ("map_values", 1),
    ("match", 1),
    ("match", 2),
    ("max", 0),
    ("max_by", 1),
    ("min", 0),
    ("min_by", 1),
    ("mktime", 0),
    ("modf", 0),
    ("modulemeta", 0),
    ("nan", 0),
    ("nearbyint", 0),
    ("nextafter", 2),
    ("nexttoward", 2),
    ("normals", 0),
    ("not", 0),
    ("now", 0),
    ("nth", 1),
    ("nth", 2),
    ("nulls", 0),
    ("numbers", 0),
    ("objects", 0),
    ("path", 1),
    ("paths", 0),
    ("paths", 1),
    ("pick", 1),
    ("pow", 2),
    ("pow10", 0),
    ("range", 1),
    ("range", 2),
    ("range", 3),
    ("recurse", 0),
    ("recurse", 1),
    ("recurse", 2),
    ("remainder", 2),
    ("repeat", 1),
    ("reverse", 0),
    ("rindex", 1),
    ("rint", 0),
    ("round", 0),
    ("rtrimstr", 1),
    ("scalars", 0),
    ("scalb", 2),
    ("scalbln", 2),
    ("scan", 1),
    ("scan", 2),
    ("select", 1),
    ("setpath", 2),
    ("significand", 0),
    ("sin", 0),
    ("sinh", 0),
    ("sort", 0),
    ("sort_by", 1),
    ("split", 1),
    ("split", 2),
    ("splits", 1),
    ("splits", 2),
    ("sqrt", 0),
    ("startswith", 1),
    ("stderr", 0),
    ("strflocaltime", 1),
    ("strftime", 1),
    ("strings", 0),
    ("strptime", 1),
    ("sub", 2),
    ("sub", 3),
    ("tan", 0),
    ("tanh", 0),
    ("test", 1),
    ("test", 2),
    ("tgamma", 0),
    ("to_entries", 0),
    ("todate", 0),
    ("todateiso8601", 0),
    ("tojson", 0),
    ("tonumber", 0),
    ("tostream", 0),
    ("tostring", 0),
    ("transpose", 0),
    ("trunc", 0),
    ("truncate_stream", 1),
    ("type", 0),
    ("unique", 0),
    ("unique_by", 1),
    ("until", 2),
    ("utf8bytelength", 0),
    ("values", 0),
    ("walk", 1),
    ("while", 2),
    ("with_entries", 1),
    ("y0", 0),
    ("y1", 0),
    ("yn", 2),
];

/// A lexical function scope: the `(name, arity)` pairs visible at a point in
/// the tree.
///
/// A `Vec` used as a stack, not a map: scopes are small (a handful of `def`s
/// and parameters), shadowing falls out of searching from the top, and pushing
/// and truncating is cheaper than cloning a map per node.
type Scope = Vec<(String, usize)>;

/// [`Scope`]'s sibling for [`build_call_graph`] (#2740): each entry also
/// carries the identity of the `def` it resolves to -- a parameter has none
/// (`None`), since it has no body of its own to ever mark reachable.
///
/// A `def`'s identity is its `body`'s own heap address (`&**body as *const
/// Expr as usize`), not a sequential counter: computing it needs no walk
/// state threaded through this pass at all, and -- the property that
/// actually matters -- [`check`]'s own later, separate, `&mut`-based walk
/// over the *same* tree can recompute the identical address for the same
/// `Expr::FuncDef` node without this pass having to hand it anything, since
/// this pass never restructures the tree it walks. A sequential counter
/// would need the two passes to agree on visitation order down to the
/// exact node, forever, as a silent invariant nothing enforces; a
/// recomputed address needs the two passes to agree on nothing at all past
/// "this box hasn't moved since the last pass read its address".
type ReachScope = Vec<(String, usize, ScopeHit)>;

/// What a [`ReachScope`] entry resolves to: a real `def` (identified by its
/// body's address, for [`build_call_graph`]'s call graph) or a parameter,
/// which has no body of its own to ever mark reachable.
#[derive(Clone, Copy)]
enum ScopeHit {
    Def(usize),
    Param,
}

/// Whether `(name, arity)` resolves against `scope`, and if so, which
/// `def`'s body it reaches (or that it's a parameter). Innermost first,
/// mirroring [`in_scope`].
fn reach_in_scope(scope: &ReachScope, name: &str, arity: usize) -> Option<ScopeHit> {
    scope
        .iter()
        .rev()
        .find(|(n, a, _)| *a == arity && n == name)
        .map(|(_, _, hit)| *hit)
}

/// Read-only twin of [`builtin_fallback_into_args`]: yields `fallback`'s own
/// sub-expressions by reference rather than consuming it, for
/// [`build_call_graph`] (#2740), which only needs to see into them, not
/// resolve or replace the call itself. Kept in sync with that function by
/// construction, not just convention: both match the exact same shapes
/// `builtin_fallback_arity` names, one returning owned children, this one
/// borrowed ones.
fn builtin_fallback_children(fallback: &Expr) -> Vec<&Expr> {
    match fallback {
        Expr::Not => Vec::new(),
        Expr::Limit { n, expr } => alloc::vec![n.as_ref(), expr.as_ref()],
        Expr::Until { cond, update } | Expr::While { cond, update } => {
            alloc::vec![cond.as_ref(), update.as_ref()]
        }
        Expr::Repeat(inner) | Expr::FirstExpr(inner) | Expr::LastExpr(inner) => {
            alloc::vec![inner.as_ref()]
        }
        Expr::Paren(inner) if matches!(inner.as_ref(), Expr::Range { .. }) => {
            match inner.as_ref() {
                Expr::Range { to: Some(to), .. } => alloc::vec![to.as_ref()],
                Expr::Range { .. } => Vec::new(),
                _ => unreachable!("guarded by the outer match arm's pattern"),
            }
        }
        Expr::Range { from, to, step } => {
            let mut children = alloc::vec![from.as_ref()];
            children.extend(to.as_deref());
            children.extend(step.as_deref());
            children
        }
        Expr::Error(msg) => msg.iter().map(alloc::boxed::Box::as_ref).collect(),
        Expr::Break(_) => Vec::new(),
        Expr::Builtin(builtin) => match builtin_kids(builtin) {
            BuiltinKids::None => Vec::new(),
            BuiltinKids::One(a) => alloc::vec![a],
            BuiltinKids::Two(a, b) => alloc::vec![a, b],
            BuiltinKids::Three(a, b, c) => alloc::vec![a, b, c],
        },
        // Genuinely unreachable -- see `builtin_fallback_arity`'s own
        // identical fallback arm.
        _ => Vec::new(),
    }
}

/// Builds the call graph among `def` bodies (#2740): an edge from `def` `A`
/// (identified by its body's address, or the virtual root -- `None` --  for
/// code outside every `def` body, always reachable) to `def` `B` records
/// that `A`'s body (or the root) contains a call resolving to `B`. A call
/// resolving to a parameter contributes no edge -- a parameter is not a
/// `def` with a body to reach, and its actual argument flows from wherever
/// the enclosing function is itself called, a different question this
/// pass does not answer.
///
/// Mirrors [`check`]'s own structure arm for arm (both must stay
/// exhaustive over every [`Expr`] variant, so a missing arm here is a
/// compile error, not a silent gap), but does no error reporting and never
/// mutates: it only needs to know *which* calls exist and where they sit,
/// not to resolve or rewrite them, so `#2036`'s `builtin_fallback` shadowing
/// decision is made read-only here via [`reach_in_scope`]/
/// [`builtin_fallback_children`] rather than the consuming
/// [`builtin_fallback_into_args`].
fn build_call_graph(
    expr: &Expr,
    scope: &mut ReachScope,
    enclosing: Option<usize>,
    graph: &mut BTreeMap<usize, Vec<usize>>,
    roots: &mut Vec<usize>,
) {
    let mut record_edge = |target: usize, graph: &mut BTreeMap<usize, Vec<usize>>| match enclosing {
        Some(e) => graph.entry(e).or_default().push(target),
        None => roots.push(target),
    };

    match expr {
        Expr::Shared(inner) => build_call_graph(inner, scope, enclosing, graph, roots),
        Expr::DefCall { args, .. } => {
            for arg in args {
                build_call_graph(arg, scope, enclosing, graph, roots);
            }
        }
        Expr::Identity
        | Expr::Field(_)
        | Expr::Index { .. }
        | Expr::Slice { .. }
        | Expr::Iterate
        | Expr::Literal(_)
        | Expr::RecursiveDescent
        | Expr::Not
        | Expr::Format(_)
        | Expr::Var(_)
        | Expr::TrackedVar(_)
        | Expr::Loc { .. }
        | Expr::Env
        | Expr::Break(_) => {}

        Expr::Optional(inner)
        | Expr::Array(inner)
        | Expr::Paren(inner)
        | Expr::Negate(inner)
        | Expr::FirstExpr(inner)
        | Expr::LastExpr(inner)
        | Expr::Repeat(inner)
        | Expr::Label { body: inner, .. } => {
            build_call_graph(inner, scope, enclosing, graph, roots);
        }

        Expr::Error(inner) => {
            if let Some(e) = inner.as_deref() {
                build_call_graph(e, scope, enclosing, graph, roots);
            }
        }

        Expr::Arithmetic { left, right, .. }
        | Expr::Compare { left, right, .. }
        | Expr::And(left, right)
        | Expr::Or(left, right)
        | Expr::Alternative(left, right)
        | Expr::IndexExpr {
            target: left,
            key: right,
        }
        | Expr::Limit {
            n: left,
            expr: right,
        }
        | Expr::NthExpr {
            n: left,
            expr: right,
        }
        | Expr::Until {
            cond: left,
            update: right,
        }
        | Expr::While {
            cond: left,
            update: right,
        }
        | Expr::As {
            expr: left,
            body: right,
            ..
        }
        | Expr::AsPattern {
            expr: left,
            body: right,
            ..
        }
        | Expr::Assign {
            path: left,
            value: right,
        }
        | Expr::Update {
            path: left,
            filter: right,
        }
        | Expr::CompoundAssign {
            path: left,
            value: right,
            ..
        }
        | Expr::AlternativeAssign {
            path: left,
            value: right,
        }
        | Expr::MetaAssign {
            target: left,
            value: right,
            ..
        } => {
            build_call_graph(left, scope, enclosing, graph, roots);
            build_call_graph(right, scope, enclosing, graph, roots);
        }

        Expr::FuncDef {
            name,
            params,
            body,
            then,
            ..
        } => {
            let body_addr = body.as_ref() as *const Expr as usize;
            graph.entry(body_addr).or_default();

            let outer = scope.len();
            scope.push((name.clone(), params.len(), ScopeHit::Def(body_addr)));
            let with_self = scope.len();
            for p in params {
                scope.push((p.name().to_string(), 0, ScopeHit::Param));
            }
            build_call_graph(body, scope, Some(body_addr), graph, roots);
            scope.truncate(with_self);

            build_call_graph(then, scope, enclosing, graph, roots);
            scope.truncate(outer);
        }

        Expr::Try { expr, catch } => {
            build_call_graph(expr, scope, enclosing, graph, roots);
            if let Some(c) = catch.as_deref() {
                build_call_graph(c, scope, enclosing, graph, roots);
            }
        }

        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            build_call_graph(cond, scope, enclosing, graph, roots);
            build_call_graph(then_branch, scope, enclosing, graph, roots);
            build_call_graph(else_branch, scope, enclosing, graph, roots);
        }

        Expr::SliceExpr { target, start, end } => {
            build_call_graph(target, scope, enclosing, graph, roots);
            if let Some(s) = start.as_deref() {
                build_call_graph(s, scope, enclosing, graph, roots);
            }
            if let Some(e) = end.as_deref() {
                build_call_graph(e, scope, enclosing, graph, roots);
            }
        }

        Expr::Range { from, to, step } => {
            build_call_graph(from, scope, enclosing, graph, roots);
            if let Some(t) = to.as_deref() {
                build_call_graph(t, scope, enclosing, graph, roots);
            }
            if let Some(s) = step.as_deref() {
                build_call_graph(s, scope, enclosing, graph, roots);
            }
        }

        Expr::Reduce {
            input,
            init,
            update,
            ..
        } => {
            build_call_graph(input, scope, enclosing, graph, roots);
            build_call_graph(init, scope, enclosing, graph, roots);
            build_call_graph(update, scope, enclosing, graph, roots);
        }

        Expr::Foreach {
            input,
            init,
            update,
            extract,
            ..
        } => {
            build_call_graph(input, scope, enclosing, graph, roots);
            build_call_graph(init, scope, enclosing, graph, roots);
            build_call_graph(update, scope, enclosing, graph, roots);
            if let Some(e) = extract.as_deref() {
                build_call_graph(e, scope, enclosing, graph, roots);
            }
        }

        Expr::Pipe(exprs) | Expr::Comma(exprs) => {
            for e in exprs {
                build_call_graph(e, scope, enclosing, graph, roots);
            }
        }

        Expr::FuncCall {
            name,
            args,
            builtin_fallback,
        } => {
            let arity = if args.is_empty() {
                builtin_fallback
                    .as_deref()
                    .map_or(0, builtin_fallback_arity)
            } else {
                args.len()
            };
            match reach_in_scope(scope, name, arity) {
                Some(hit) => {
                    if let ScopeHit::Def(target) = hit {
                        record_edge(target, graph);
                    }
                    if !args.is_empty() {
                        for a in args {
                            build_call_graph(a, scope, enclosing, graph, roots);
                        }
                    } else if let Some(fallback) = builtin_fallback.as_deref() {
                        for child in builtin_fallback_children(fallback) {
                            build_call_graph(child, scope, enclosing, graph, roots);
                        }
                    }
                }
                None => {
                    if let Some(fallback) = builtin_fallback.as_deref() {
                        build_call_graph(fallback, scope, enclosing, graph, roots);
                    } else {
                        for a in args {
                            build_call_graph(a, scope, enclosing, graph, roots);
                        }
                    }
                }
            }
        }

        Expr::NamespacedCall { args, .. } => {
            for a in args {
                build_call_graph(a, scope, enclosing, graph, roots);
            }
        }

        Expr::Object(entries) => {
            for entry in entries {
                if let ObjectKey::Expr(k) = &entry.key {
                    build_call_graph(k, scope, enclosing, graph, roots);
                }
                build_call_graph(&entry.value, scope, enclosing, graph, roots);
            }
        }

        Expr::StringInterpolation(parts) => {
            for part in parts {
                if let StringPart::Expr(e) = part {
                    build_call_graph(e, scope, enclosing, graph, roots);
                }
            }
        }

        Expr::Builtin(builtin) => {
            let kids: Vec<&Expr> = match builtin_kids(builtin) {
                BuiltinKids::None => Vec::new(),
                BuiltinKids::One(a) => alloc::vec![a],
                BuiltinKids::Two(a, b) => alloc::vec![a, b],
                BuiltinKids::Three(a, b, c) => alloc::vec![a, b, c],
            };
            for a in kids {
                build_call_graph(a, scope, enclosing, graph, roots);
            }
        }
    }
}

/// Every `def` body address reachable from `roots` by following `graph`
/// -- iterative, not recursive, so a long call chain (or the pathological
/// self-recursive/mutually-recursive cases this graph handles trivially,
/// same as any graph reachability computation) never grows this pass's own
/// stack the way naive recursion would.
fn compute_reachable(graph: &BTreeMap<usize, Vec<usize>>, roots: &[usize]) -> BTreeSet<usize> {
    let mut reachable = BTreeSet::new();
    let mut stack: Vec<usize> = roots.to_vec();
    while let Some(id) = stack.pop() {
        if reachable.insert(id) {
            if let Some(callees) = graph.get(&id) {
                stack.extend(callees.iter().copied());
            }
        }
    }
    reachable
}

/// Check every function call in `expr` against the `def`s, parameters and
/// builtins in scope at its position, the way real jq's compiler does.
///
/// Returns the first unresolvable call in traversal order, if any. See
/// [`resolve_func_calls_all`] to collect every one, matching jq's own
/// `jq: N compile errors` behaviour.
///
/// Must run *after* `ModuleLoader::process_program`: that is what inlines
/// `include`/`import`/`~/.jq` definitions as `Expr::FuncDef` wrappers around
/// the program and rewrites `ns::f` into a `FuncCall` named `ns::f`, matching
/// the wrapper it also creates. Running earlier would report every module
/// function as undefined.
pub fn resolve_func_calls(expr: &mut Expr) -> Result<(), UnresolvedCall> {
    match resolve_func_calls_all(expr).into_iter().next() {
        Some(first) => Err(first),
        None => Ok(()),
    }
}

/// Like [`resolve_func_calls`], but keeps traversing past an unresolvable
/// call instead of stopping at the first.
///
/// Returns every unresolvable call it finds, in traversal order (which
/// follows source order for every existing `Expr` variant). Real jq reports
/// every unresolvable call in one compile pass (`jq: N compile errors`);
/// this is what lets the jq runner match that instead of always reporting
/// `jq: 1 compile error` (#2037).
pub fn resolve_func_calls_all(expr: &mut Expr) -> Vec<UnresolvedCall> {
    // #2740: a `def` whose call never appears anywhere reachable is never
    // checked -- jq's own compiler never compiles such a body either, since
    // it only ever compiles a `def` at the call site substituting it in.
    // `build_call_graph` (read-only) computes which bodies are reachable
    // before `check` (below, `&mut`-mutating) walks the tree for real.
    let mut reach_scope = ReachScope::new();
    let mut graph = BTreeMap::new();
    let mut roots = Vec::new();
    build_call_graph(expr, &mut reach_scope, None, &mut graph, &mut roots);
    let reachable = compute_reachable(&graph, &roots);

    let mut scope = Scope::new();
    let mut errors = Vec::new();
    check(expr, &mut scope, &mut errors, &reachable);
    errors
}

/// Whether `(name, arity)` is one of the pinned jq's own builtins.
fn is_jq_builtin(name: &str, arity: usize) -> bool {
    JQ_BUILTIN_ROSTER
        .iter()
        .any(|&(n, a)| a == arity && n == name)
}

/// Whether `(name, arity)` resolves against `scope`, innermost first.
fn in_scope(scope: &Scope, name: &str, arity: usize) -> bool {
    scope.iter().rev().any(|(n, a)| *a == arity && n == name)
}

/// #2036: the arity `fallback` -- a successfully-parsed builtin or
/// fixed-arity special form stashed on `Expr::FuncCall::builtin_fallback` --
/// represents. Read-only: never clones or allocates, so computing it to
/// decide `in_scope` costs nothing beyond the match itself, regardless of
/// how large `fallback`'s own children are.
fn builtin_fallback_arity(fallback: &Expr) -> usize {
    match fallback {
        Expr::Not => 0,
        Expr::Limit { .. } | Expr::Until { .. } | Expr::While { .. } => 2,
        Expr::Repeat(_) | Expr::FirstExpr(_) | Expr::LastExpr(_) => 1,
        // #2036 review round 2: `range(N)` -- the 1-arg sugar form --
        // desugars to the exact same `Range { from: Literal(0), to:
        // Some(_), step: None }` shape a genuine 2-arg `range(0; N)` call
        // produces, so the general `Expr::Range` arm below cannot tell them
        // apart. `parse_range_expr` (`parser.rs`) marks the 1-arg form by
        // wrapping it in `Expr::Paren` (otherwise unused for this purpose,
        // and a no-op everywhere else) precisely so this arm can report the
        // correct arity of 1 here instead of the wrong 2.
        Expr::Paren(inner) if matches!(**inner, Expr::Range { .. }) => 1,
        Expr::Range { to, step, .. } => 1 + usize::from(to.is_some()) + usize::from(step.is_some()),
        Expr::Error(msg) => usize::from(msg.is_some()),
        // #2687: `break $x` desugars to a call to `error/0` -- see
        // `parser.rs`'s `parse_break_expr`. The label name is not an
        // argument (it's not even in scope as a call argument would be),
        // so this is arity 0, same as a bare `error`.
        Expr::Break(_) => 0,
        Expr::Builtin(builtin) => match builtin_kids(builtin) {
            BuiltinKids::None => 0,
            BuiltinKids::One(_) => 1,
            BuiltinKids::Two(_, _) => 2,
            BuiltinKids::Three(_, _, _) => 3,
        },
        // Genuinely unreachable: `wrap_shadowable_call` (`parser.rs`) only
        // ever stores one of the shapes matched above in
        // `builtin_fallback` -- anything else (a plain `Expr::FuncCall`, or
        // any postfix wrapping of one, produced when a dedicated parser's
        // own #2110/#2237 wrong-arity rewind fires) is returned unwrapped,
        // with `builtin_fallback` left `None`, specifically so it can never
        // reach here. Kept as a defensive fallback rather than `unreachable!()`
        // since the two functions have no compiler-enforced link to
        // `wrap_shadowable_call`'s own match -- see that function's doc
        // comment for the bug this arm used to silently paper over.
        _ => 0,
    }
}

/// #2036: moves `fallback`'s own sub-expressions out as the argument list a
/// generic `NAME(args;args)` call over the same source span would have
/// produced -- called only once [`check`] has determined the call is
/// genuinely shadowed by an in-scope `def`. Takes `fallback` *by value* and
/// moves its children rather than cloning them: `fallback` is discarded
/// immediately after this returns (the caller already took it out of
/// `builtin_fallback` via `Option::take`), so there is nothing left that
/// would need its own independent copy -- cloning here, even though it
/// would only run once per confirmed-shadowed node rather than compounding
/// across every declined one, could still duplicate an arbitrarily large
/// not-yet-resolved subtree sitting in one of `fallback`'s own arguments.
fn builtin_fallback_into_args(fallback: Expr) -> Vec<Expr> {
    match fallback {
        Expr::Not => Vec::new(),
        Expr::Limit { n, expr } => alloc::vec![*n, *expr],
        Expr::Until { cond, update } | Expr::While { cond, update } => {
            alloc::vec![*cond, *update]
        }
        Expr::Repeat(inner) | Expr::FirstExpr(inner) | Expr::LastExpr(inner) => {
            alloc::vec![*inner]
        }
        // #2036 review round 2: the 1-arg `range(N)` marker -- see
        // `builtin_fallback_arity`'s matching arm. The single real argument
        // the user wrote is `to` (the synthesized `from: Literal(0)` was
        // never written and must not be surfaced as a second argument).
        Expr::Paren(inner) if matches!(*inner, Expr::Range { .. }) => match *inner {
            Expr::Range { to: Some(to), .. } => alloc::vec![*to],
            // `parse_range_expr` only ever produces this marker with `to:
            // Some(_)` -- defensive, not reachable.
            Expr::Range { .. } => Vec::new(),
            _ => unreachable!("guarded by the outer match arm's pattern"),
        },
        Expr::Range { from, to, step } => {
            let mut args = alloc::vec![*from];
            args.extend(to.map(|b| *b));
            args.extend(step.map(|b| *b));
            args
        }
        Expr::Error(msg) => msg.into_iter().map(|b| *b).collect(),
        // #2687: `break $x`'s label name is not surfaced as an `error/0`
        // argument -- see `builtin_fallback_arity`'s matching arm. Once this
        // arm is reached, the shadowing `def error:` has already won (`check`
        // only calls this after confirming `in_scope`), so the original
        // `Expr::Break` -- label name included -- is simply discarded; it was
        // never going to reach the label-catching machinery either way.
        Expr::Break(_) => Vec::new(),
        // `builtin_kids` only ever borrows -- there is no by-value
        // counterpart, so this one case clones rather than moves. Bounded
        // even so: it can only fire once per node that check() has just
        // confirmed is genuinely shadowed, immediately followed by
        // recursing into (and thereby resolving/shrinking) each cloned
        // child -- it does not compound across the *declined* case above
        // (`*expr = *fallback`, always a pure move), which is the shape
        // nested-but-not-actually-shadowed candidates take and the one
        // this issue's own review found exponential before this fix.
        Expr::Builtin(builtin) => match builtin_kids(&builtin) {
            BuiltinKids::None => Vec::new(),
            BuiltinKids::One(a) => alloc::vec![a.clone()],
            BuiltinKids::Two(a, b) => alloc::vec![a.clone(), b.clone()],
            BuiltinKids::Three(a, b, c) => alloc::vec![a.clone(), b.clone(), c.clone()],
        },
        // Genuinely unreachable -- see `builtin_fallback_arity`'s own
        // identical fallback arm, which this must stay consistent with.
        _ => Vec::new(),
    }
}

/// Recurse into `expr` under `scope`, restoring `scope` before returning so a
/// sibling never sees a binding introduced by its neighbour. Appends every
/// unresolvable call to `errors` rather than stopping at the first, matching
/// how real jq's own compiler keeps going to report every compile error in
/// one pass (#2037).
fn check(
    expr: &mut Expr,
    scope: &mut Scope,
    errors: &mut Vec<UnresolvedCall>,
    reachable: &BTreeSet<usize>,
) {
    match expr {
        // #1371: neither variant can occur here. This pass runs once, on the
        // freshly parsed program, before evaluation begins; both are built
        // *by* evaluation. They are still given real arms rather than being
        // folded into a leaf group, so that if a future caller ever runs this
        // check over an evaluation-time tree it reports honestly instead of
        // silently treating a whole subtree as having no calls in it.
        //
        // A `DefCall` is by construction already resolved -- it holds the
        // definition it resolved to -- so only its arguments can carry an
        // unresolved call, and they are checked in the scope this node sits
        // in. The definition's own body was checked at its `FuncDef`.
        //
        // `Shared` wraps an `Rc`. `resolve_func_calls`/`Expr::Shared` are
        // both public API, so an external caller can hand this a multi-owner
        // `Rc` even though this crate's own 3 CLI call sites never do (see
        // above) -- `Rc::make_mut` (clone-on-write if not uniquely owned,
        // unlike `Rc::get_mut`'s silent "return None, skip this subtree
        // entirely" on the same case) keeps recursion unconditional exactly
        // like the pre-`&mut Expr` version of this arm did, at the cost of
        // one clone in the (self-inflicted, still never hit by this crate's
        // own callers) multi-owner case.
        Expr::Shared(inner) => {
            check(Rc::make_mut(inner), scope, errors, reachable);
        }
        Expr::DefCall { args, .. } => {
            for arg in args.iter_mut() {
                check(arg, scope, errors, reachable);
            }
        }
        // Leaves: nothing nested to descend into. Mirrors `walk::any_subexpr`'s
        // own grouping so the two stay comparable arm for arm.
        Expr::Identity
        | Expr::Field(_)
        | Expr::Index { .. }
        | Expr::Slice { .. }
        | Expr::Iterate
        | Expr::Literal(_)
        | Expr::RecursiveDescent
        | Expr::Not
        | Expr::Format(_)
        | Expr::Var(_)
        | Expr::TrackedVar(_)
        | Expr::Loc { .. }
        | Expr::Env
        | Expr::Break(_) => {}

        Expr::Optional(inner)
        | Expr::Array(inner)
        | Expr::Paren(inner)
        | Expr::Negate(inner)
        | Expr::FirstExpr(inner)
        | Expr::LastExpr(inner)
        | Expr::Repeat(inner)
        | Expr::Label { body: inner, .. } => check(inner, scope, errors, reachable),

        Expr::Error(inner) => {
            if let Some(e) = inner.as_deref_mut() {
                check(e, scope, errors, reachable);
            }
        }

        Expr::Arithmetic { left, right, .. }
        | Expr::Compare { left, right, .. }
        | Expr::And(left, right)
        | Expr::Or(left, right)
        | Expr::Alternative(left, right)
        | Expr::IndexExpr {
            target: left,
            key: right,
        }
        | Expr::Limit {
            n: left,
            expr: right,
        }
        // `Expr::NthExpr` has no parser construction site at all -- `nth(n; f)`
        // parses to `Builtin::NthStream` (see `eval_generic.rs`'s own
        // `Builtin::NthStream` arm, which records the same finding). It is
        // named here because this match is exhaustive with no wildcard, not
        // because a parsed program can reach it, so it shows as uncovered and
        // no test can change that.
        | Expr::NthExpr {
            n: left,
            expr: right,
        }
        | Expr::Until {
            cond: left,
            update: right,
        }
        | Expr::While {
            cond: left,
            update: right,
        }
        | Expr::As {
            expr: left,
            body: right,
            ..
        }
        | Expr::AsPattern {
            expr: left,
            body: right,
            ..
        }
        | Expr::Assign {
            path: left,
            value: right,
        }
        | Expr::Update {
            path: left,
            filter: right,
        }
        | Expr::CompoundAssign {
            path: left,
            value: right,
            ..
        }
        | Expr::AlternativeAssign {
            path: left,
            value: right,
        }
        | Expr::MetaAssign {
            target: left,
            value: right,
            ..
        } => {
            check(left, scope, errors, reachable);
            check(right, scope, errors, reachable);
        }

        // The one arm that changes scope. `body` sees the function itself
        // (self-recursion is legal) plus its parameters as arity-0 functions;
        // `then` sees the function but *not* its parameters. Neither sees a
        // `def` that comes later — which is exactly the forward reference
        // `expand_func_calls` could not detect, and the reason
        // `def f: g; def g: 42; f` computed `42` instead of failing.
        Expr::FuncDef {
            name,
            params,
            body,
            then,
            ..
        } => {
            let outer = scope.len();
            scope.push((name.clone(), params.len()));
            let with_self = scope.len();

            // A parameter binds its bare name at arity 0 only: `def f(g):
            // g(1)` is `g/1 is not defined` in jq, not a call to the outer
            // `g`. Pushed after the function's own name so a parameter
            // shadowing it wins.
            for p in params.iter() {
                scope.push((p.name().to_string(), 0));
            }
            // #2740: a `def` whose call never appears anywhere reachable is
            // never compiled by real jq either -- its own substitution model
            // only ever compiles a body at the call site that references it.
            // `build_call_graph` (run once, up front, in
            // `resolve_func_calls_all`) already answered this for every
            // `def` in the program by the time this walk reaches it, keyed
            // by the same body-address identity recomputed here -- so
            // skipping the check costs nothing (this is not a second
            // graph walk, just one `BTreeSet` lookup) and never revisits
            // scope/`then`, which still need the same treatment either way.
            let body_addr = body.as_ref() as *const Expr as usize;
            if reachable.contains(&body_addr) {
                check(body, scope, errors, reachable);
            }
            scope.truncate(with_self);

            check(then, scope, errors, reachable);
            scope.truncate(outer);
        }

        Expr::Try { expr, catch } => {
            check(expr, scope, errors, reachable);
            if let Some(c) = catch.as_deref_mut() {
                check(c, scope, errors, reachable);
            }
        }

        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            check(cond, scope, errors, reachable);
            check(then_branch, scope, errors, reachable);
            check(else_branch, scope, errors, reachable);
        }

        Expr::SliceExpr { target, start, end } => {
            check(target, scope, errors, reachable);
            check_opt(start.as_deref_mut(), scope, errors, reachable);
            check_opt(end.as_deref_mut(), scope, errors, reachable);
        }

        Expr::Range { from, to, step } => {
            check(from, scope, errors, reachable);
            check_opt(to.as_deref_mut(), scope, errors, reachable);
            check_opt(step.as_deref_mut(), scope, errors, reachable);
        }

        Expr::Reduce {
            input,
            init,
            update,
            ..
        } => {
            check(input, scope, errors, reachable);
            check(init, scope, errors, reachable);
            check(update, scope, errors, reachable);
        }

        Expr::Foreach {
            input,
            init,
            update,
            extract,
            ..
        } => {
            check(input, scope, errors, reachable);
            check(init, scope, errors, reachable);
            check(update, scope, errors, reachable);
            check_opt(extract.as_deref_mut(), scope, errors, reachable);
        }

        Expr::Pipe(exprs) | Expr::Comma(exprs) => {
            for e in exprs.iter_mut() {
                check(e, scope, errors, reachable);
            }
        }

        // The check itself. Arguments are checked in the *caller's* scope, not
        // the callee's — an argument is an expression written at the call
        // site — but only when the callee itself resolves. Real jq's compiler
        // binds a call's argument closures to the callee's parameter slots,
        // which requires already having found the callee; an unresolved
        // callee means there is no such binding to attempt, so jq never
        // compiles the arguments at all and reports only the callee
        // (verified live: `nosucha(nosuchb; nosuchc)` is `nosucha/2 is not
        // defined`, one error, not three). Checking the arguments anyway —
        // the pre-#2037 code did, via `?`-propagation that just happened to
        // discard the extra errors by stopping at the first one — would
        // report errors real jq's compiler never reaches once every call is
        // collected instead of just the first (#2037).
        //
        // #2036: `builtin_fallback` is this node's alternate parse -- the
        // builtin or special form the parser would have produced had the
        // name never been shadowable -- attached only when the lexical
        // prescan flagged `name` as possibly `def`'d somewhere in the
        // program (see `Expr::FuncCall`'s own doc comment). `args` starts
        // out *empty* whenever `builtin_fallback` is `Some` (the parser
        // deliberately does not clone the fallback's own children into it
        // -- see `wrap_shadowable_call`'s own doc comment for why that
        // duplication made nested shadow candidates an `O(2^depth)`
        // parser-level denial-of-service), so the arity to check scope
        // against has to come from the fallback's own shape
        // (`builtin_fallback_arity`) whenever `args` is still empty, not
        // from `args.len()` directly.
        //
        // Real shadowing (`in_scope`) is checked *first* and wins outright:
        // a `def` at this exact `(name, arity)` always shadows the
        // builtin, matching real jq. Only then is `args` actually
        // populated -- moved out of `fallback` (`builtin_fallback_into_args`),
        // not cloned, since `fallback` is discarded immediately after and
        // there is nothing left needing its own copy. If shadowing is
        // declined instead, the *whole* fallback is moved back into `expr`
        // in one piece -- again no cloning -- and re-checked so its own
        // nested children get the identical treatment. Whichever way it
        // resolves, `builtin_fallback` is always `None` and `args` always
        // populated by the time this function returns for this node -- so
        // no other pass over the tree, before or after this one runs to
        // completion, ever needs to know `builtin_fallback` exists, or
        // sees a node whose `args` is emptily lying about its real arity.
        Expr::FuncCall {
            name,
            args,
            builtin_fallback,
        } => {
            let arity = if args.is_empty() {
                builtin_fallback
                    .as_deref()
                    .map_or(0, builtin_fallback_arity)
            } else {
                args.len()
            };
            if in_scope(scope, name, arity) {
                if let Some(fallback) = builtin_fallback.take() {
                    *args = builtin_fallback_into_args(*fallback);
                }
                for a in args.iter_mut() {
                    check(a, scope, errors, reachable);
                }
            } else if let Some(fallback) = builtin_fallback.take() {
                // Not shadowed after all -- restore the original parse in
                // one move, no cloning.
                *expr = *fallback;
                check(expr, scope, errors, reachable);
            } else if is_jq_builtin(name, arity) {
                for a in args.iter_mut() {
                    check(a, scope, errors, reachable);
                }
            } else {
                errors.push(UnresolvedCall {
                    name: name.clone(),
                    arity,
                });
            }
        }

        // `NamespacedCall` survives only when this pass runs without the jq
        // runner's `rewrite_namespaced_calls` (the yq runner has no module
        // system). Its arguments still need checking; the call itself is left
        // to `eval`'s own "module not loaded" reporting rather than claimed
        // undefined here.
        Expr::NamespacedCall { args, .. } => {
            for a in args.iter_mut() {
                check(a, scope, errors, reachable);
            }
        }

        Expr::Object(entries) => {
            for entry in entries.iter_mut() {
                if let ObjectKey::Expr(k) = &mut entry.key {
                    check(k, scope, errors, reachable);
                }
                check(&mut entry.value, scope, errors, reachable);
            }
        }

        Expr::StringInterpolation(parts) => {
            for part in parts.iter_mut() {
                if let StringPart::Expr(e) = part {
                    check(e, scope, errors, reachable);
                }
            }
        }

        // No builtin introduces a function binding, so its sub-expressions
        // inherit the current scope unchanged. Rebuilt via
        // `map_builtin_subexprs` rather than an in-place mutable walker: a
        // builtin's own operand can itself be a `FuncCall` this same #2036
        // rewrite needs to reach (`map(length)` where `length` is
        // shadowed, say), so each sub-expression is cloned, checked (which
        // may substitute it), and used to rebuild the builtin -- the same
        // clone-and-rebuild shape `jq_runner.rs`'s `rewrite_namespaced_calls`
        // already uses for its own builtin arm, reusing this function's
        // existing, tested infrastructure rather than hand-writing a
        // second, mutable 207-arm match beside `builtin_kids`.
        Expr::Builtin(builtin) => {
            *builtin = map_builtin_subexprs(builtin, &mut |sub| {
                let mut sub = sub.clone();
                check(&mut sub, scope, errors, reachable);
                sub
            });
        }
    }
}

/// [`check`] over an optional sub-expression.
fn check_opt(
    expr: Option<&mut Expr>,
    scope: &mut Scope,
    errors: &mut Vec<UnresolvedCall>,
    reachable: &BTreeSet<usize>,
) {
    if let Some(e) = expr {
        check(e, scope, errors, reachable);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jq::parse;
    use alloc::format;

    /// Resolve a filter, returning the failing `name/arity` if any. Every
    /// expectation in this module was captured from the pinned oracle
    /// (`/usr/bin/jq`, jq-1.7.1), never from succinctly's own output.
    fn resolve(filter: &str) -> Result<(), String> {
        let mut expr = parse(filter).expect("filter must parse");
        resolve_func_calls(&mut expr).map_err(|e| format!("{e}"))
    }

    /// The compiled-in roster must match what `./scripts/sync-jq-builtin-names.sh`
    /// captured from the pinned jq (1.7.1), entry for entry and in order.
    ///
    /// A stale table silently reintroduces the one false-positive class this
    /// pass has: a jq builtin succinctly does not implement, mentioned
    /// somewhere evaluation never reaches, would be rejected at compile time
    /// where real jq compiles it. Compared against the checked-in capture
    /// rather than a live `jq` -- CI runners have none, and the file *is* the
    /// pin's output.
    #[test]
    fn jq_builtin_roster_matches_the_pinned_capture() {
        let captured: Vec<(&str, usize)> = include_str!("../../tests/data/jq-builtin-names.txt")
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .map(|l| {
                let (name, arity) = l.rsplit_once('/').expect("malformed roster entry");
                (name, arity.parse().expect("malformed roster arity"))
            })
            .collect();

        assert_eq!(
            JQ_BUILTIN_ROSTER.len(),
            captured.len(),
            "roster size drifted from the capture -- rerun \
             ./scripts/sync-jq-builtin-names.sh and update JQ_BUILTIN_ROSTER"
        );
        for (compiled, captured) in JQ_BUILTIN_ROSTER.iter().zip(captured.iter()) {
            assert_eq!(compiled, captured, "roster entry drifted from the capture");
        }
    }

    #[test]
    fn accepts_a_call_to_a_preceding_def() {
        assert_eq!(resolve("def f: 1; def g: f; g"), Ok(()));
    }

    #[test]
    fn rejects_a_forward_reference_across_names() {
        // jq: `g/0 is not defined`. succinctly computed `42` before #1473.
        assert_eq!(
            resolve("def f: g; def g: 42; f"),
            Err("g/0 is not defined".into())
        );
    }

    #[test]
    fn rejects_a_forward_reference_to_a_later_arity_of_itself() {
        // jq: `f/2 is not defined`. succinctly computed `100` before #1473 --
        // the silently-wrong case #1376's review found.
        assert_eq!(
            resolve("def f(x): f(x; 99); def f(x; y): x + y; f(1)"),
            Err("f/2 is not defined".into())
        );
    }

    #[test]
    fn accepts_self_recursion() {
        assert_eq!(
            resolve("def f(n): if n == 0 then 0 else f(n-1) end; f(3)"),
            Ok(())
        );
    }

    #[test]
    fn accepts_arity_overloading_in_both_directions() {
        // #1376's own repro, plus the earlier arity called from inside the
        // later one's body -- neither is a forward reference.
        assert_eq!(
            resolve("def f(x): x + 1; def f(x; y): x + y; [f(1), f(2;3)]"),
            Ok(())
        );
        assert_eq!(
            resolve("def f(x): x+1; def f(x;y): (f(x)) + y; f(2;3)"),
            Ok(())
        );
    }

    #[test]
    fn rejects_an_arity_no_def_provides() {
        assert_eq!(
            resolve("def f(x): x+1; def f(x;y): x+y; f(1;2;3)"),
            Err("f/3 is not defined".into())
        );
    }

    #[test]
    fn rejects_an_unreached_branch_and_a_caught_call() {
        // The two laziness symptoms: jq rejects both unconditionally.
        assert_eq!(
            resolve("def f(x): x; if false then f(1;2;3) else 1 end"),
            Err("f/3 is not defined".into())
        );
        assert_eq!(
            resolve("def f(x): x; try f(1;2) catch \"caught\""),
            Err("f/2 is not defined".into())
        );
    }

    /// #2740: a `def` whose call never appears anywhere in the program is
    /// never checked -- jq's own substitution-based compiler never compiles
    /// such a body either, since it only ever compiles a `def` at the call
    /// site referencing it. Distinct from the *unreached-branch* case above:
    /// `if false then f(1;2;3) ...` still contains a real, textual call to
    /// `f` (jq rejects it regardless of which branch runs at evaluation
    /// time) -- this test's `h` has no call to it anywhere at all.
    #[test]
    fn accepts_a_def_whose_body_is_never_referenced_anywhere() {
        assert_eq!(resolve("def h: nosuchfn; 1"), Ok(()));
        // A nested, locally-scoped def that is likewise never called.
        assert_eq!(resolve("def h: def g: nosuchfn; 1; 1"), Ok(()));
        // Two mutually unreferenced defs.
        assert_eq!(resolve("def a: nosuchfn; def b: nosuchfn2; 1"), Ok(()));
    }

    /// #2740: a call reached only through a genuine (non-shadowed)
    /// `Expr::Range`'s own `to` bound still propagates reachability --
    /// `range(1; b)` parses straight to `Expr::Range`, not a `FuncCall`,
    /// since no `def range` exists anywhere in this program to trigger
    /// #2036's shadowable-call wrapping.
    #[test]
    fn a_call_reached_through_a_range_bound_still_raises() {
        assert_eq!(
            resolve("def b: nosuchfn; def a: range(1; b); a"),
            Err("nosuchfn/0 is not defined".into())
        );
    }

    /// #2740: a def that IS referenced still gets its body checked, even
    /// when the only reference is as a bare *argument* to another call
    /// that never actually invokes its parameter -- confirmed live against
    /// jq 1.7.1: the argument expression itself must still resolve/compile
    /// at the call site, independent of whether the callee's body ever
    /// uses the parameter it's bound to.
    #[test]
    fn rejects_a_def_referenced_only_as_an_unused_argument() {
        assert_eq!(
            resolve("def h: nosuchfn; def use(f): 1; use(h)"),
            Err("nosuchfn/0 is not defined".into())
        );
    }

    /// #2740: mutual recursion between two otherwise-unreferenced defs stays
    /// unreachable as a pair -- reachability is computed over the whole
    /// call graph, not a single def in isolation, so a cycle with no
    /// incoming edge from outside it is still dead.
    #[test]
    fn a_mutually_recursive_pair_stays_unreachable_together() {
        assert_eq!(resolve("def a: b; def b: nosuchfn; 1"), Ok(()));
    }

    /// #2740: reachability must not weaken the *existing* checks -- a call
    /// that genuinely reaches an unresolvable name still raises, even when
    /// discovered only via a chain of several defs.
    #[test]
    fn a_call_reached_through_several_defs_still_raises() {
        // Dependency order matters here for a reason unrelated to
        // reachability: `def a: b; ...` would be a forward reference (#1473
        // rejects it before this pass's own reachability question is even
        // reached), so each def below only calls one already defined.
        assert_eq!(
            resolve("def c: nosuchfn; def b: c; def a: b; a"),
            Err("nosuchfn/0 is not defined".into())
        );
    }

    #[test]
    fn a_nested_def_does_not_leak_into_the_outer_scope() {
        assert_eq!(
            resolve("def f: def g: 1; g; g"),
            Err("g/0 is not defined".into())
        );
    }

    #[test]
    fn a_parameter_binds_its_bare_name_at_arity_zero_only() {
        assert_eq!(resolve("def f(g): g; f(1)"), Ok(()));
        // jq: `def f($a): a; f(1)` is `1` -- the `$` form binds both.
        assert_eq!(resolve("def f($a): a; f(1)"), Ok(()));
        // ...but only at arity 0.
        assert_eq!(
            resolve("def f(g): g(1); f(.)"),
            Err("g/1 is not defined".into())
        );
    }

    #[test]
    fn a_parameter_is_out_of_scope_outside_its_own_body() {
        assert_eq!(resolve("def f(g): g; g"), Err("g/0 is not defined".into()));
    }

    #[test]
    fn accepts_a_jq_builtin_succinctly_does_not_implement() {
        // `cbrt` is a real jq 1.7.1 builtin succinctly lowers to nothing, so it
        // arrives here as a bare call. jq compiles it; rejecting it would be a
        // regression this pass introduced. A *reached* call still fails at
        // runtime, exactly as before.
        assert_eq!(resolve("if false then cbrt else 1 end"), Ok(()));
        assert_eq!(resolve("JOIN(.; .; .; .)"), Ok(()));
    }

    #[test]
    fn rejects_a_name_neither_defined_nor_a_builtin() {
        assert_eq!(resolve("nosuchfn"), Err("nosuchfn/0 is not defined".into()));
    }

    #[test]
    fn a_call_argument_is_checked_in_the_callers_scope() {
        // The argument `h` is written at the call site, where only `f/1` and
        // nothing else is in scope -- not inside `f`'s body, where `x` is.
        assert_eq!(
            resolve("def f(x): x; f(h)"),
            Err("h/0 is not defined".into())
        );
    }

    #[test]
    fn descends_into_builtin_sub_expressions() {
        // `map(f)` carries its argument inside a `Builtin`, not an
        // `Expr::FuncCall` -- the `builtin_kids` arm is what reaches it.
        assert_eq!(
            resolve("map(nosuchfn)"),
            Err("nosuchfn/0 is not defined".into())
        );
    }

    #[test]
    fn a_later_same_arity_def_shadows_but_the_earlier_one_stays_resolvable() {
        assert_eq!(resolve("def f: 1; def g: f; def f: 2; g"), Ok(()));
    }

    /// #1371: `Shared`/`DefCall` can never actually reach `check` -- this pass
    /// runs once, on the freshly parsed program, strictly before evaluation
    /// ever builds either variant. Named rather than folded into the leaf
    /// group regardless (see the arm's own comment), so exercised directly
    /// here via `check` itself rather than through `resolve_func_calls`'s
    /// parser-only entry point: each arm must actually recurse into its
    /// payload, not just be present and silently report "no unresolved
    /// calls" the way the leaf arms correctly do for real leaves.
    #[test]
    fn check_recurses_through_shared_and_defcall() {
        use alloc::rc::Rc;

        let unresolved = || Expr::FuncCall {
            name: "nosuchfn".into(),
            args: Vec::new(),
            builtin_fallback: None,
        };

        let mut scope = Scope::new();
        let mut errors = Vec::new();
        check(
            &mut Expr::Shared(Rc::new(unresolved())),
            &mut scope,
            &mut errors,
            &BTreeSet::new(),
        );
        assert_eq!(
            errors,
            [UnresolvedCall {
                name: "nosuchfn".into(),
                arity: 0,
            }]
        );

        let mut scope = Scope::new();
        let mut errors = Vec::new();
        check(
            &mut Expr::DefCall {
                def: Rc::new(crate::jq::FuncDefData {
                    name: "f".into(),
                    params: Vec::new(),
                    body: Expr::Identity,
                }),
                args: alloc::vec![unresolved()],
                frames: 0,
                bound: crate::jq::BoundBody::default(),
            },
            &mut scope,
            &mut errors,
            &BTreeSet::new(),
        );
        assert_eq!(
            errors,
            [UnresolvedCall {
                name: "nosuchfn".into(),
                arity: 0,
            }]
        );
    }

    /// #1371 (mirroring `check_recurses_through_shared_and_defcall` just
    /// above, same reasoning): `build_call_graph` must also recurse into
    /// `Shared`/`DefCall` payloads, even though neither variant survives to
    /// reach it in practice -- this pass runs once, on the freshly parsed
    /// program, strictly before evaluation ever builds either.
    #[test]
    fn build_call_graph_recurses_through_shared_and_defcall() {
        use alloc::rc::Rc;

        let call_to_f = || Expr::FuncCall {
            name: "f".into(),
            args: Vec::new(),
            builtin_fallback: None,
        };

        let mut scope: ReachScope = alloc::vec![("f".to_string(), 0, ScopeHit::Def(42))];
        let mut graph = BTreeMap::new();
        let mut roots = Vec::new();
        build_call_graph(
            &Expr::Shared(Rc::new(call_to_f())),
            &mut scope,
            None,
            &mut graph,
            &mut roots,
        );
        assert_eq!(roots, alloc::vec![42]);

        let mut scope: ReachScope = alloc::vec![("f".to_string(), 0, ScopeHit::Def(42))];
        let mut graph = BTreeMap::new();
        let mut roots = Vec::new();
        build_call_graph(
            &Expr::DefCall {
                def: Rc::new(crate::jq::FuncDefData {
                    name: "f".into(),
                    params: Vec::new(),
                    body: Expr::Identity,
                }),
                args: alloc::vec![call_to_f()],
                frames: 0,
                bound: crate::jq::BoundBody::default(),
            },
            &mut scope,
            None,
            &mut graph,
            &mut roots,
        );
        assert_eq!(roots, alloc::vec![42]);
    }

    /// #2687: `break $x` desugars to a resolvable `error/0` call -- a
    /// same-name, same-arity `def error:` in scope at the break's own
    /// position shadows it, same as any other call. Oracle: `jq-1.7.1`
    /// answers `1`, `"S"`, `3` for
    /// `def error: "S"; label $out | (1, break $out, 3)`.
    #[test]
    fn break_with_a_shadowing_arity_0_def_error_resolves_clean() {
        assert_eq!(
            resolve("def error: \"S\"; label $out | (1, break $out, 3)"),
            Ok(())
        );
    }

    /// #2687: an arity-1 `def error(m):` does not shadow a bare `break $x`
    /// (arity 0) -- real jq's own `is_jq_builtin`-style arity distinction
    /// applies here exactly as it does to a plain `error` call. Confirmed
    /// live: `def error(m): "S1"; label $out | 1, break $out` still answers
    /// `1` in jq 1.7.1.
    #[test]
    fn break_is_not_shadowed_by_a_different_arity_def_error() {
        assert_eq!(
            resolve("def error(m): \"S1\"; label $out | 1, break $out"),
            Ok(())
        );
    }

    /// #2687: with no `def error` anywhere in the program, `break $x` must
    /// parse to the exact same `Expr::Break` node it always has -- not a
    /// `FuncCall` that then falls back at resolve time. This is what keeps
    /// every existing break/label test byte-for-byte unaffected by this
    /// fix's parser change (`Parser::shadowable_defs`'s fast-reject, the
    /// same one every other shadowable special form already relies on).
    #[test]
    fn break_with_no_def_error_anywhere_stays_a_bare_break_node() {
        let mut expr = parse("label $out | break $out").expect("filter must parse");
        assert!(
            resolve_func_calls(&mut expr).is_ok(),
            "must resolve with no unresolved calls"
        );
        assert!(
            crate::jq::walk::any_subexpr(
                &expr,
                &mut |e| matches!(e, Expr::Break(name) if name == "out")
            ),
            "expected a bare Expr::Break(\"out\") node, found: {expr:?}"
        );
        assert!(
            !crate::jq::walk::any_subexpr(&expr, &mut |e| matches!(
                e,
                Expr::FuncCall { name, .. } if name == "error"
            )),
            "must not have been wrapped as a FuncCall when nothing shadows it: {expr:?}"
        );
    }

    /// #2687: the flip side of the above -- once a shadowing `def error:` is
    /// genuinely in scope, the break site must resolve to a real call
    /// (`DefCall`, after `eval::install_def_calls` runs) rather than staying
    /// an `Expr::Break`. `resolve_func_calls` alone leaves a shadowed site as
    /// an ordinary zero-arg `Expr::FuncCall { name: "error", .. }` -- the
    /// `DefCall` rewrite is a separate, later eval-time pass -- so this
    /// checks that intermediate shape directly.
    #[test]
    fn break_with_a_shadowing_def_error_becomes_an_error_call_node() {
        let mut expr =
            parse("def error: \"S\"; label $out | break $out").expect("filter must parse");
        assert!(resolve_func_calls(&mut expr).is_ok());
        assert!(
            crate::jq::walk::any_subexpr(&expr, &mut |e| matches!(
                e,
                Expr::FuncCall { name, args, .. } if name == "error" && args.is_empty()
            )),
            "expected a resolved zero-arg `error` FuncCall node, found: {expr:?}"
        );
        assert!(
            !crate::jq::walk::any_subexpr(&expr, &mut |e| matches!(e, Expr::Break(_))),
            "the original Expr::Break must not survive once shadowed: {expr:?}"
        );
    }
}
