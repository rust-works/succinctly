//! Zero-argument tail calls of a user `def` run in place (#5379).
//!
//! jq compiles a call to a def that takes no arguments, and is followed by
//! nothing, to a jump (`TAIL_CALL_JQ`), so `def f: if . < 300000 then . + 1 | f
//! else . end; 0 | f` runs in constant space. The evaluators here are recursive
//! tree walks: a call nests the native frames of its body, and
//! [`MAX_EVAL_FRAMES`](super::eval::MAX_EVAL_FRAMES) and the native-stack floor
//! (ADR-0025) refuse it long before that.
//!
//! A tail call needs nothing from the call it leaves: no frames to return to,
//! no consumer to resume. This module holds the parts of that which do not
//! depend on which evaluator walks the body:
//!
//! - [`tail_reachable`] says, once per definition, whether a walk of the body's
//!   tail spine could reach a call at all. A body it says no to keeps the nested
//!   path it always took, so the loop costs a recursion that is not a tail call
//!   one read of a cell per call.
//! - [`stage_settles`] says whether a condition, a stage or an `as` source
//!   answers at most once, so the walk may run it to its one value ahead of the
//!   call that follows it.
//! - [`Tail`] is where one round's walk ended, and [`bind_next`] binds the body
//!   of the call it ended in.
//! - [`TailRounds`] bounds a loop that never ends, and one that never stops emitting.
//!
//! The evaluators own the walk itself (`tail_step`), because evaluating a stage
//! is each evaluator's own business; they differ in what a value is, not in
//! which nodes may be passed through.

#[cfg(not(test))]
use alloc::format;
#[cfg(not(test))]
use alloc::rc::Rc;
#[cfg(test)]
use std::rc::Rc;

use super::error::EvalError;
use super::eval::{
    bind_def_call, bind_def_call_body, def_call_depth_refusal, pipe_needs_path_context,
    single_valued_pure, yields_at_most_one_value, Flow, PureWalk, MAX_LOOP_ROUNDS,
};
use super::expr::{BoundBody, Builtin, Expr, FuncDefData, PipeStages};
use super::resolve::ModuleRun;
use super::walk::{search_subexpr, Visit};

/// Where a tail loop's walk of one round's body ended (#5379).
pub(crate) enum Tail<'e> {
    /// A call jq jumps to, over the value the walk left in the loop: run the
    /// callee's body next, with this level gone. `frames` and `bound` are the
    /// call node's own.
    Call {
        def: &'e Rc<FuncDefData>,
        frames: u32,
        bound: &'e BoundBody,
    },
    /// The walk reached something it does not pass through: evaluate it over the
    /// value, as the nested call would have.
    Rest(&'e Expr),
    /// The walk went on in an expression it built (a `def` declared in the body
    /// bound what follows it, an `as` rebuilt its body): carry on from there,
    /// over the same value, in the same round.
    Bound(Rc<Expr>),
    /// The round is over without a call: the walk delivered what the body
    /// yields, or ended in a flow the caller returns.
    Done(Flow),
}

/// The call a node makes in tail position, if it is one jq would jump to: no
/// arguments, to a def with no parameters; with the node's `frames` and cache.
///
/// jq's `TAIL_CALL_JQ` is emitted for a zero-argument call followed by `RET`; a
/// call with arguments is `CALL_JQ` and nests (`f(n - 1)`).
pub(crate) fn tail_def(expr: &Expr) -> Option<(&Rc<FuncDefData>, u32, &BoundBody)> {
    match expr {
        Expr::DefCall {
            def,
            args,
            frames,
            bound,
        } if args.is_empty() && def.params.is_empty() => Some((def, *frames, bound)),
        _ => None,
    }
}

/// Bind the body a tail call to `def` runs next (#5379), and the `frames` it was
/// bound with.
///
/// The first hop of a loop binds as the nested call would: through the call
/// node's own cache, with the node's `frames`, so the charge for the call's place
/// in a wide or deep body is paid once, and a call that repeats (`[range(n) | f]`)
/// reuses the body. Every later hop binds a fresh body with the `frames` of the
/// one it leaves: native depth does not grow across rounds, so neither does the
/// charge. Nothing caches it. A body kept by the previous round's call node would
/// chain every round's body to the one before it, and a body reused across rounds
/// would make each of its own calls hot (#3148), keeping their call trees where
/// the nested form freed them as they returned.
pub(crate) fn bind_next(
    def: &Rc<FuncDefData>,
    node_frames: u32,
    bound: &BoundBody,
    first_hop: bool,
    current_frames: u32,
) -> Result<(Rc<Expr>, u32), EvalError> {
    if first_hop {
        Ok((bind_def_call(def, &[], node_frames, bound)?, node_frames))
    } else {
        Ok((
            bind_def_call_body(def, &[], current_frames)?,
            current_frames,
        ))
    }
}

/// The visits one question to [`stage_settles`] or [`tail_reachable`] may make
/// before it answers no (the shipped evaluator's `SETTLE_ANALYSIS_BUDGET`, for the
/// same kind of question).
const SETTLE_BUDGET: u32 = 1024;

/// What a question to [`settles`] knows beyond the expression: the mode, and the names
/// of the defs declared in the body being walked, whose calls are still calls by name
/// in the raw body.
struct Ctx<'a> {
    jq: bool,
    locals: &'a [&'a str],
}

/// Whether `expr` yields at most one output and does nothing observable besides
/// once it has: the recognition [`single_valued_pure`] gives an operand that
/// [`settles_before_consumer`](super::eval::settles_before_consumer) (without its
/// requirement that a `def` be called), widened to the shapes a loop that keeps
/// its state in the value writes ahead of its call.
///
/// A condition or a stage in front of the call that passes this can be
/// evaluated to its one value before the call runs, with nothing left of it to
/// resume once the call has returned: the call is the last thing that runs, so
/// the loop may drop the level.
///
/// One compositional question: an operator, an `if`, a pipe or an assignment
/// settles when its parts do, a call to a def with no parameters when the def's
/// body does (remembered on the definition), and a leaf by
/// [`yields_at_most_one_value`] (the builtins that answer once: `length`,
/// `select`, `tostring`, a slice) or [`single_valued_pure`]. An effect (`debug`,
/// `input`) does not disqualify: it happens while the stage runs, before its one
/// output, which is where jq runs it relative to the call too, and none of those
/// shapes can emit a value and then raise.
///
/// In yq mode a key or a slice bound that is computed does not settle: an index
/// whose key yields nothing answers every child there (`.a[select(false)]`).
pub(crate) fn stage_settles(expr: &Expr, jq: bool) -> bool {
    settles_in(expr, &Ctx { jq, locals: &[] })
}

fn settles_in(expr: &Expr, cx: &Ctx<'_>) -> bool {
    let mut budget = SETTLE_BUDGET;
    settles(expr, &mut budget, cx)
}

fn settles(expr: &Expr, budget: &mut u32, cx: &Ctx<'_>) -> bool {
    let Some(left) = budget.checked_sub(1) else {
        return false;
    };
    *budget = left;
    match expr {
        Expr::Paren(inner) | Expr::Negate(inner) => settles(inner, budget, cx),
        Expr::Pipe(stages) => stages.iter().all(|stage| settles(stage, budget, cx)),
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            settles(cond, budget, cx)
                && settles(then_branch, budget, cx)
                && settles(else_branch, budget, cx)
        }
        Expr::Arithmetic { left, right, .. }
        | Expr::Compare { left, right, .. }
        | Expr::And(left, right)
        | Expr::Or(left, right)
        | Expr::Alternative(left, right) => settles(left, budget, cx) && settles(right, budget, cx),
        // `[f]` collects every output of `f`: one array, or the error `f` raised.
        // An update answers once whatever its filter yields (it takes the first).
        Expr::Array(_) | Expr::Update { .. } => true,
        // An assignment answers once per output of its right side (`rhs as $v | ...`
        // in jq's own definition).
        Expr::Assign { value, .. }
        | Expr::CompoundAssign { value, .. }
        | Expr::AlternativeAssign { value, .. } => settles(value, budget, cx),
        // A call to a def declared in the body being walked: optimistic, the bound
        // call is asked the real question when the walk reaches it.
        Expr::FuncCall { name, .. } if cx.locals.contains(&name.as_str()) => true,
        Expr::DefCall { def, args, .. } if args.is_empty() && def.params.is_empty() => def
            .settles_or_init(cx.jq, |body| {
                let mut fresh = SETTLE_BUDGET;
                settles(
                    body,
                    &mut fresh,
                    &Ctx {
                        jq: cx.jq,
                        locals: &[],
                    },
                )
            }),
        Expr::IndexExpr { .. } | Expr::SliceExpr { .. } if !cx.jq => false,
        Expr::Builtin(builtin) if builtin_settles(builtin) => true,
        _ => {
            yields_at_most_one_value(expr)
                || single_valued_pure(expr, None, &mut PureWalk::new(false)).is_some()
        }
    }
}

/// The builtins that collect, or compute, one answer per input, which
/// [`yields_at_most_one_value`] (written for key positions) leaves out.
fn builtin_settles(builtin: &Builtin) -> bool {
    matches!(
        builtin,
        Builtin::Map(_)
            | Builtin::MapValues(_)
            | Builtin::WithEntries(_)
            | Builtin::ToEntries
            | Builtin::FromEntries
            | Builtin::Sort
            | Builtin::SortBy(_)
            | Builtin::GroupBy(_)
            | Builtin::Unique
            | Builtin::UniqueBy(_)
            | Builtin::Reverse
            | Builtin::Flatten
            | Builtin::Transpose
            | Builtin::Del(_)
            | Builtin::MinBy(_)
            | Builtin::MaxBy(_)
            | Builtin::Floor
            | Builtin::Ceil
            | Builtin::Round
            | Builtin::Sqrt
            | Builtin::Fabs
            | Builtin::Abs
            | Builtin::ToJson
            | Builtin::Utf8ByteLength
            | Builtin::Explode
            | Builtin::Implode
            | Builtin::Trim
            | Builtin::Ltrim
            | Builtin::Rtrim
    )
}

/// Whether a pipe's stages in front of its last are all [`stage_settles`], and
/// the evaluator would not take a different route for the pipe than running
/// them in order (`needs_path_context`).
pub(crate) fn pipe_settles_before_last(stages: &PipeStages, jq: bool) -> bool {
    pipe_settles_in(stages, &Ctx { jq, locals: &[] })
}

fn pipe_settles_in(stages: &PipeStages, cx: &Ctx<'_>) -> bool {
    let Some((_, before)) = stages.split_last() else {
        return false;
    };
    !pipe_needs_path_context(stages) && before.iter().all(|stage| settles_in(stage, cx))
}

/// Whether a walk of `def`'s body could reach a call that can bring the loop round
/// again, for jq (`jq`) or yq mode; remembered on the definition (#5379).
///
/// Asked on every call of every `def`, so the whole answer is a read of a cell
/// after the first. It is asked of the raw body, where the definition's own calls
/// (and those of a def declared inside it) are still calls by name, and only such a
/// call can bring a loop round: a call to the def itself, or to one declared in
/// its body, whose own calls may come back. A call to an earlier def (a helper,
/// which is a call already resolved in the raw body) or to a closure parameter
/// cannot, and a helper that loops runs its own loop when it is called, so a body
/// that ends in one keeps the nested path and its pipes the fused evaluation they
/// had. The stages in front of the call are asked with the local defs taken to
/// settle: a yes only sends the call through a loop that finds out for itself.
#[inline]
pub(crate) fn tail_reachable(def: &FuncDefData, jq: bool) -> bool {
    def.tail_reachable_or_init(jq, |body| reach(body, def, jq, &mut Vec::new()))
}

fn reach<'a>(body: &'a Expr, def: &FuncDefData, jq: bool, locals: &mut Vec<&'a str>) -> bool {
    let depth = locals.len();
    let mut expr = body;
    let reached = loop {
        match expr {
            Expr::Paren(inner) => expr = inner,
            Expr::FuncCall { name, args, .. } => {
                break args.is_empty()
                    && ((def.params.is_empty() && *name == def.name)
                        || locals.contains(&name.as_str()));
            }
            // The walk binds what follows a `def` declared in the body.
            Expr::FuncDef { name, then, .. } => {
                locals.push(name.as_str());
                expr = then;
            }
            // `src as $x | body` is passed through in jq mode only.
            Expr::As {
                expr: source, body, ..
            } => {
                if !jq || !settles_in(source, &Ctx { jq, locals }) {
                    break false;
                }
                expr = body;
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                if !settles_in(cond, &Ctx { jq, locals }) {
                    break false;
                }
                if reach(then_branch, def, jq, locals) {
                    break true;
                }
                expr = else_branch;
            }
            Expr::Pipe(stages) => match stages.last() {
                Some(last) if pipe_settles_in(stages, &Ctx { jq, locals }) => expr = last,
                _ => break false,
            },
            Expr::Comma(items) => match items.last() {
                Some(last) => expr = last,
                None => break false,
            },
            _ => break false,
        }
    };
    locals.truncate(depth);
    reached
}

/// [`tail_reachable`] for the path-mode interpreter, which walks the program as
/// written (a call is a call by name there, a helper included) and keeps its own
/// notion of "answers once" over its environment: only the call at the end of the
/// spine is asked, and it is `name`'s own or a def declared in the body.
pub(crate) fn declared_tail_call(body: &Expr, name: &str) -> bool {
    fn walk<'a>(body: &'a Expr, name: &str, locals: &mut Vec<&'a str>) -> bool {
        let depth = locals.len();
        let mut expr = body;
        let reached = loop {
            match expr {
                Expr::Paren(inner) => expr = inner,
                Expr::As { body, .. } => expr = body,
                Expr::FuncCall {
                    name: callee, args, ..
                } => {
                    break args.is_empty() && (callee == name || locals.contains(&callee.as_str()));
                }
                Expr::FuncDef { name, then, .. } => {
                    locals.push(name.as_str());
                    expr = then;
                }
                Expr::If {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    if walk(then_branch, name, locals) {
                        break true;
                    }
                    expr = else_branch;
                }
                Expr::Pipe(stages) => match stages.last() {
                    Some(last) => expr = last,
                    None => break false,
                },
                Expr::Comma(items) => match items.last() {
                    Some(last) => expr = last,
                    None => break false,
                },
                _ => break false,
            }
        };
        locals.truncate(depth);
        reached
    }
    walk(body, name, &mut Vec::new())
}

/// How many calls of the tail spine of `expr` jump (#5379): [`tail_def`]s through
/// parentheses, an `as` body, both branches of an `if`, the last stage of a pipe
/// and the last branch of a comma.
fn spine_calls(expr: &Expr) -> usize {
    let mut expr = expr;
    loop {
        match expr {
            Expr::Paren(inner) => expr = inner,
            Expr::As { body, .. } => expr = body,
            Expr::DefCall { .. } => return usize::from(tail_def(expr).is_some()),
            Expr::If {
                then_branch,
                else_branch,
                ..
            } => return spine_calls(then_branch) + spine_calls(else_branch),
            Expr::Pipe(stages) => match stages.last() {
                Some(last) => expr = last,
                None => return 0,
            },
            Expr::Comma(items) => match items.last() {
                Some(last) => expr = last,
                None => return 0,
            },
            _ => return 0,
        }
    }
}

/// Whether a round's bound `body` can be run again for the next round as it is
/// (#5379): its only calls are the ones the loop jumps from, and it declares no
/// `def`. Then no node of it is ever evaluated through its own cache, so none can turn hot
/// (#3148) and keep a call tree, and binding the same body again would only
/// rebuild what it already holds. Asked once for a body, not per round.
pub(crate) fn body_is_reusable(body: &Expr) -> bool {
    let mut calls = 0usize;
    let declares = search_subexpr(body, &mut |e| match e {
        Expr::DefCall { .. } => {
            calls += 1;
            Visit::Skip
        }
        Expr::FuncDef { .. } => Visit::Found,
        _ => Visit::Descend,
    });
    !declares && calls == spine_calls(body)
}

/// The most outputs a tail loop may emit ahead of its calls (the front branches
/// of a comma), over all its rounds (#5379): 2^20.
///
/// A generator that tail-calls itself (`def f: ., (. + 1 | f)`) emits one output
/// per round, and a consumer that collects (`[...]`, an argument of a builtin) holds
/// them all: the round budget alone would let it hold 2^24 of them, gigabytes,
/// before it refused. jq's own loop runs until its heap is gone. Counted in
/// outputs, not bytes.
pub(crate) const MAX_LOOP_OUTPUTS: usize = 1 << 20;

/// Rounds a loop runs before it is charged for the outputs it emitted, so a def
/// that is not recursive at all, whose one round emits a long range, is not.
pub(crate) const OUTPUT_BUDGET_AFTER_ROUNDS: usize = 64;

/// What a round that binds a body of its own is charged against [`MAX_LOOP_ROUNDS`], in
/// rounds that bind none (#5379).
const BOUND_ROUND_WEIGHT: usize = 16;

/// What a tail loop may still spend (#5379).
///
/// jq loops forever on `def f: f; f`, in constant memory. This stops it, with the
/// refusal the nested form reached, after [`MAX_LOOP_ROUNDS`] rounds: the same
/// bound `until` and `while` keep for a loop that steps in place (#5292), counted
/// from the start of each loop, and it cannot tell a loop that ends from one that
/// does not. The outputs it emits ahead of its calls are bounded separately.
pub(crate) struct TailRounds {
    rounds: usize,
    taken: usize,
    emitted: usize,
    def: Rc<FuncDefData>,
}

impl TailRounds {
    /// The budget of a loop that started at a call to `def`, which it is named for.
    pub(crate) fn new(def: &Rc<FuncDefData>) -> Self {
        Self {
            rounds: MAX_LOOP_ROUNDS,
            taken: 0,
            emitted: 0,
            def: Rc::clone(def),
        }
    }

    /// Charge one more round, which `bound` a body of its own: such a round costs the
    /// bind (a rebuilt copy of the body) as well as the walk, and is charged
    /// [`BOUND_ROUND_WEIGHT`] times as much, so the time to refuse a loop that never ends
    /// does not grow with the size of its body as fast as it would.
    pub(crate) fn charge_round(&mut self, bound: bool) -> Result<(), EvalError> {
        self.taken += 1;
        let weight = if bound { BOUND_ROUND_WEIGHT } else { 1 };
        match self.rounds.checked_sub(weight) {
            Some(left) => {
                self.rounds = left;
                Ok(())
            }
            None => Err(def_call_depth_refusal(&self.def)),
        }
    }

    /// Charge `emitted` outputs a round sent to the consumer, once the loop has
    /// run [`OUTPUT_BUDGET_AFTER_ROUNDS`] rounds.
    pub(crate) fn charge_outputs(&mut self, emitted: usize) -> Result<(), EvalError> {
        self.emitted = self.emitted.saturating_add(emitted);
        if self.emitted > MAX_LOOP_OUTPUTS && self.taken >= OUTPUT_BUDGET_AFTER_ROUNDS {
            return Err(EvalError::resource_limit(format!(
                "{}/{}: maximum outputs exceeded",
                ModuleRun::display_name(&self.def.name),
                self.def.params.len()
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jq::eval::{bind_def_call_body, install_def_calls};
    use crate::jq::parse;

    /// The definition of `def f: <body>; f`, as the evaluators hold it.
    fn definition(src: &str) -> Rc<FuncDefData> {
        let Expr::FuncDef {
            name, params, body, ..
        } = parse(src).expect("parses")
        else {
            panic!("`{src}` does not start with a def");
        };
        Rc::new(FuncDefData::new(name, params, *body))
    }

    /// The last definition of a program of defs, as the evaluators hold it: the calls in its
    /// body to the defs before it are installed, and its own are still calls by name.
    fn last_definition(src: &str) -> Rc<FuncDefData> {
        let mut earlier: Vec<Rc<FuncDefData>> = Vec::new();
        let mut rest = parse(src).expect("parses");
        while let Expr::FuncDef {
            name,
            params,
            body,
            then,
            ..
        } = rest
        {
            let body = earlier
                .iter()
                .fold(*body, |body, def| install_def_calls(&body, def, 0, false));
            earlier.push(Rc::new(FuncDefData::new(name, params, body)));
            rest = *then;
        }
        earlier.pop().expect("a def")
    }

    #[test]
    fn the_spine_ends_in_a_call_that_can_bring_the_loop_round_5379() {
        // (program, reachable in jq mode, reachable in yq mode)
        for (src, jq, yq) in [
            ("def f: f; f", true, true),
            ("def f: if . < 3 then . + 1 | f else . end; f", true, true),
            ("def f: if . < 3 then . else . + 1 | f end; f", true, true),
            (
                "def f: if . < 3 then . + 1 | f elif . < 5 then . | f else . end; f",
                true,
                true,
            ),
            ("def f: (. + 1 | f); f", true, true),
            ("def f: ., (. + 1 | f); f", true, true),
            // A def declared in the body: the call to it, or to the def from inside it.
            ("def f: def g: f; . + 1 | g; f", true, true),
            // A local helper used as the condition or a stage settles for the walk's purposes.
            (
                "def f: def step: . + 1; if . < 3 then step | f else . end; f",
                true,
                true,
            ),
            // Passed through in jq mode only.
            ("def f: . as $x | f; f", true, false),
            (
                "def f: . as $x | if $x then . + 1 | f else . end; f",
                true,
                false,
            ),
            // The call is in the body of an `as` whose source is a generator.
            ("def f: .[] as $x | f; f", false, false),
            // A call that is not last, or not a call, or not a jump in jq.
            ("def f: f | . + 1; f", false, false),
            ("def f: [f]; f", false, false),
            ("def f: 1 + f; f", false, false),
            ("def f: f, 1; f", false, false),
            ("def f: try f catch .; f", false, false),
            ("def f: f?; f", false, false),
            ("def f: label $out | f; f", false, false),
            ("def f: if f then . else . end; f", false, false),
            ("def f: . + 1; f", false, false),
            ("def f: if . < 3 then 1 else 2 end; f", false, false),
            // The walk cannot pass a stage that may yield more than once.
            ("def f: .[]? | f; f", false, false),
            ("def f: ., (.[]? | f); f", false, false),
            (
                "def f: if (.[]? | . > 1) then f else . end; f",
                false,
                false,
            ),
        ] {
            let def = definition(src);
            assert_eq!(tail_reachable(&def, true), jq, "jq mode: `{src}`");
            assert_eq!(tail_reachable(&def, false), yq, "yq mode: `{src}`");
        }
    }

    /// A call to an earlier def is a helper, already resolved in the raw body: it runs its
    /// own loop if it has one, and nothing it calls can come back, so a body that ends in
    /// one is not the loop's to walk. Nor is a closure parameter, or a call to a def with a
    /// parameter, or to another def's name.
    #[test]
    fn a_trailing_call_that_cannot_come_back_is_not_a_loop_5379() {
        for (src, why) in [
            ("def render: . + 1; def top: sort | render; top", "a helper"),
            ("def ap(g): g; ap(1)", "a closure parameter"),
            ("def f: other; f", "another def's name"),
            (
                "def f($n): if $n > 0 then f else . end; f(3)",
                "a zero-argument call is not a call to a def that takes a parameter",
            ),
        ] {
            assert!(
                !tail_reachable(&last_definition(src), true),
                "{why}: `{src}`"
            );
        }
    }

    #[test]
    fn a_stage_settles_when_it_answers_at_most_once_5379() {
        for (src, want) in [
            // The recognitions `single_valued_pure` and `yields_at_most_one_value` give.
            (". + 1", true),
            (". < 3", true),
            (".a.b", true),
            (".[1:]", true),
            ("length", true),
            ("select(. > 1)", true),
            ("tostring | length", true),
            ("[.[] | . + 1]", true),
            // Composed of parts that settle, across the two recognitions.
            ("(length * 0) + 1", true),
            ("if length > 1 then . + 1 else sort end", true),
            ("(.a // 1) + (. | length)", true),
            // Writes ahead of a call, and what collects.
            (".n += 1", true),
            (".a |= . + 1", true),
            (".a = 1", true),
            (".a //= 1", true),
            ("map(. + 1)", true),
            ("sort", true),
            ("del(.a)", true),
            ("unique | .[0] += 1", true),
            // A generator, however it is spelled.
            (".[]", false),
            ("(., .)", false),
            ("1, 2", false),
            ("range(3)", false),
            ("recurse", false),
            (".n += (1, 2)", false),
            ("first(.[])", false),
            ("limit(3; .[])", false),
            ("if (1, 2) then 1 else 2 end", false),
            ("empty", true),
        ] {
            let expr = parse(src).expect("parses");
            assert_eq!(stage_settles(&expr, true), want, "`{src}`");
        }
    }

    /// An index whose key yields nothing answers every child in yq, and nothing in jq, so a
    /// computed key or bound does not settle in yq mode.
    #[test]
    fn a_computed_index_settles_in_jq_mode_only_5379() {
        for src in [".a[select(false)]", ".[.n]", ".a[1:.n]", "(.a)[.n] + 1"] {
            let expr = parse(src).expect("parses");
            assert!(stage_settles(&expr, true), "jq: `{src}`");
            assert!(!stage_settles(&expr, false), "yq: `{src}`");
        }
        // A literal key reads one child in both.
        let expr = parse(".a[1] + .a[0:1][0]").expect("parses");
        assert!(stage_settles(&expr, false));
    }

    /// A call to a def with no parameters settles when the def's body does, and the answer
    /// is the definition's own: asked again, it is read back.
    #[test]
    fn a_call_to_a_settling_def_settles_5379() {
        for (def_src, want) in [
            ("def g: . + 1; g", true),
            ("def g: .n += 1; g", true),
            ("def g: .[]; g", false),
            ("def g: g | . + 1; g", false),
        ] {
            let Expr::FuncDef {
                name,
                params,
                body,
                then,
                ..
            } = parse(def_src).expect("parses")
            else {
                panic!("a def");
            };
            let def = Rc::new(FuncDefData::new(name, params, *body));
            let call = install_def_calls(&then, &def, 0, false);
            assert_eq!(stage_settles(&call, true), want, "`{def_src}`");
            assert_eq!(stage_settles(&call, true), want, "`{def_src}`, remembered");
        }
    }

    /// A body is run again as it is only when its calls are all jumps and it declares nothing
    /// that a round could reach twice.
    #[test]
    fn a_body_with_only_jumps_is_reused_5379() {
        for (src, want) in [
            ("def f: if . < 3 then . + 1 | f else . end; f", true),
            ("def f: ., (. + 1 | f); f", true),
            ("def f: . as $x | if $x < 3 then f else . end; f", true),
            // Another call, in a stage or the condition: its node would turn hot.
            (
                "def g: . + 1; def f: if . < 3 then g | f else . end; f",
                false,
            ),
            (
                "def g: . < 3; def f: if g then . + 1 | f else . end; f",
                false,
            ),
            // A def declared in the body is bound once and kept.
            (
                "def f: def g: f; if . < 3 then . + 1 | g else . end; f",
                false,
            ),
        ] {
            let def = last_definition(src);
            let body = bind_def_call_body(&def, &[], 0).expect("binds");
            assert_eq!(body_is_reusable(&body), want, "`{src}`");
        }
    }

    #[test]
    fn the_budgets_run_out_5379() {
        let def = definition("def f: f; f");
        let mut rounds = TailRounds::new(&def);
        rounds.rounds = 2;
        assert!(rounds.charge_round(false).is_ok());
        assert!(rounds.charge_round(false).is_ok());
        let refused = rounds
            .charge_round(false)
            .expect_err("the third round is over budget");
        assert_eq!(refused.to_string(), "f/0 exceeded maximum recursion depth");

        // Outputs are charged once the loop has run a few rounds: a def that is not
        // recursive, whose one round emits a long range, is not.
        let mut rounds = TailRounds::new(&def);
        assert_eq!(rounds.rounds, MAX_LOOP_ROUNDS);
        assert!(rounds.charge_outputs(MAX_LOOP_OUTPUTS + 1).is_ok());
        for _ in 0..OUTPUT_BUDGET_AFTER_ROUNDS {
            rounds.charge_round(false).expect("within the round budget");
        }
        let refused = rounds
            .charge_outputs(1)
            .expect_err("one output past the budget");
        assert_eq!(refused.to_string(), "f/0: maximum outputs exceeded");

        // A round that bound a body of its own is charged more.
        let mut rounds = TailRounds::new(&def);
        rounds.rounds = BOUND_ROUND_WEIGHT;
        assert!(rounds.charge_round(true).is_ok());
        assert!(rounds.charge_round(false).is_err());
    }
}
