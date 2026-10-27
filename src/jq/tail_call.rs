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
//! - [`spine_has_tail_call`] says whether a bound body has a tail call at all.
//!   A body that has none keeps the nested path it always took, so the loop
//!   costs a recursion that is not a tail call one spine walk per call.
//! - [`tail_def`] and [`stage_settles`] decide, for one node of the spine, whether
//!   it is one the loop may pass through without changing what runs after it.
//! - [`TailBindings`] rebinds the callee's body each round, from the call that
//!   started the loop: a def with no parameters binds to a body that depends on
//!   `(def, frames)` alone, so the bodies are shared across rounds.
//! - [`TailRounds`] bounds a loop that never ends.
//!
//! The evaluators own the walk itself (`tail_step`), because evaluating a stage
//! is each evaluator's own business; they differ in what a value is, not in
//! which nodes may be passed through.

#[cfg(not(test))]
use alloc::rc::Rc;
#[cfg(not(test))]
use alloc::vec;
#[cfg(not(test))]
use alloc::vec::Vec;
#[cfg(test)]
use std::rc::Rc;

use super::error::EvalError;
use super::eval::{
    bind_def_call_body, def_call_depth_refusal, pipe_needs_path_context, single_valued_pure,
    yields_at_most_one_value, PureWalk, MAX_LOOP_ROUNDS,
};
use super::expr::{Expr, FuncDefData, PipeStages};

/// The def a node calls in tail position, if it is a call jq would jump to:
/// no arguments, to a def with no parameters.
///
/// jq's `TAIL_CALL_JQ` is emitted for exactly a zero-argument call followed by
/// `RET`; a call with arguments is `CALL_JQ` and nests (`f(n - 1)`).
pub(crate) fn tail_def(expr: &Expr) -> Option<&Rc<FuncDefData>> {
    match expr {
        Expr::DefCall { def, args, .. } if args.is_empty() && def.params.is_empty() => Some(def),
        _ => None,
    }
}

/// Whether the tail spine of a bound `body` ends in a [`tail_def`] anywhere:
/// through parentheses, both branches of an `if` (an `elif` is a nested `if`),
/// the last stage of a pipe and the last branch of a comma.
///
/// Only asks about shape. Whether the loop may pass through the conditions and
/// stages in front of the call is [`stage_settles`] and `tail_step`'s question,
/// answered with the value in hand.
pub(crate) fn spine_has_tail_call(body: &Expr) -> bool {
    let mut expr = body;
    loop {
        match expr {
            Expr::Paren(inner) => expr = inner,
            Expr::DefCall { .. } => return tail_def(expr).is_some(),
            Expr::If {
                then_branch,
                else_branch,
                ..
            } => {
                if spine_has_tail_call(then_branch) {
                    return true;
                }
                expr = else_branch;
            }
            Expr::Pipe(stages) => match stages.last() {
                Some(last) => expr = last,
                None => return false,
            },
            Expr::Comma(items) => match items.last() {
                Some(last) => expr = last,
                None => return false,
            },
            // A def declared inside the body is bound when evaluation reaches it,
            // so its `then` still holds its calls by name: a zero-argument call
            // at the end of it may be a tail call once bound.
            Expr::FuncDef { then, .. } => return declared_tail_call(then),
            _ => return false,
        }
    }
}

/// [`spine_has_tail_call`] for what follows a `def` that is not bound yet: a
/// zero-argument call by name (or one already installed) at the end of the spine.
/// May say yes for a call that turns out to be a builtin, which only sends the
/// body through the loop, whose answer is the nested one's.
fn declared_tail_call(then: &Expr) -> bool {
    let mut expr = then;
    loop {
        match expr {
            Expr::Paren(inner) => expr = inner,
            Expr::FuncCall { args, .. } => return args.is_empty(),
            Expr::DefCall { .. } => return tail_def(expr).is_some(),
            Expr::FuncDef { then, .. } => expr = then,
            Expr::If {
                then_branch,
                else_branch,
                ..
            } => {
                if declared_tail_call(then_branch) {
                    return true;
                }
                expr = else_branch;
            }
            Expr::Pipe(stages) => match stages.last() {
                Some(last) => expr = last,
                None => return false,
            },
            Expr::Comma(items) => match items.last() {
                Some(last) => expr = last,
                None => return false,
            },
            _ => return false,
        }
    }
}

/// Whether `expr` yields at most one output and does nothing observable besides:
/// the same recognition [`single_valued_pure`] gives an operand that
/// [`settles_before_consumer`](super::eval::settles_before_consumer), without its
/// requirement that a `def` be called.
///
/// A condition or a stage in front of the call that passes this can be
/// evaluated to its one value before the call runs, with nothing left of it to
/// resume once the call has returned: the call is the last thing that runs, so
/// the loop may drop the level.
///
/// Either recognition is enough. [`single_valued_pure`] knows a call to a `def`;
/// [`yields_at_most_one_value`] knows the builtins that answer once (`length`,
/// `select`, `tostring`, a slice, `[f]`). An effect (`debug`, `input`) does not
/// disqualify the second: it happens while the stage runs, before its one output,
/// which is where jq runs it relative to the call too, and that list admits no
/// shape that can emit a value and then raise.
pub(crate) fn stage_settles(expr: &Expr) -> bool {
    single_valued_pure(expr, None, &mut PureWalk::new(false)).is_some()
        || yields_at_most_one_value(expr)
}

/// Whether a pipe's stages in front of its last are all [`stage_settles`], and
/// the evaluator would not take a different route for the pipe than running
/// them in order (`needs_path_context`).
pub(crate) fn pipe_settles_before_last(stages: &PipeStages) -> bool {
    let Some((_, before)) = stages.split_last() else {
        return false;
    };
    !pipe_needs_path_context(stages) && before.iter().all(stage_settles)
}

/// The bodies a tail loop runs, bound once each.
///
/// Every round of one loop binds with the `frames` of the call that started it
/// (`entry_frames`): native depth does not grow across rounds, so the charge
/// [`MAX_EVAL_FRAMES`](super::eval::MAX_EVAL_FRAMES) keeps is the loop's own,
/// and a def with no parameters binds to a body that depends on `(def, frames)`
/// alone. A self call reuses the body it is already running; a call to another
/// def (a wrapper, or the other half of a mutual recursion) binds it on first
/// reach and keeps it, keyed by the `Rc` it holds so the key outlives the entry.
pub(crate) struct TailBindings {
    entry_frames: u32,
    bodies: Vec<(Rc<FuncDefData>, Rc<Expr>)>,
}

impl TailBindings {
    /// The loop of a call to `def` whose bound body is `body`.
    pub(crate) fn new(def: &Rc<FuncDefData>, body: &Rc<Expr>, entry_frames: u32) -> Self {
        Self {
            entry_frames,
            bodies: vec![(Rc::clone(def), Rc::clone(body))],
        }
    }

    /// The body of a tail call to `next`.
    pub(crate) fn body_of(&mut self, next: &Rc<FuncDefData>) -> Result<Rc<Expr>, EvalError> {
        if let Some((_, body)) = self.bodies.iter().find(|(def, _)| Rc::ptr_eq(def, next)) {
            return Ok(Rc::clone(body));
        }
        let body = bind_def_call_body(next, &[], self.entry_frames)?;
        self.bodies.push((Rc::clone(next), Rc::clone(&body)));
        Ok(body)
    }
}

/// The rounds one tail loop may run.
///
/// jq loops forever on `def f: f; f`, in constant memory. This stops it, with the
/// refusal the nested form reached, after [`MAX_LOOP_ROUNDS`] rounds: the same
/// bound `until` and `while` keep for a loop that steps in place (#5292), counted
/// from the start of each loop.
pub(crate) struct TailRounds(usize);

impl TailRounds {
    pub(crate) fn new() -> Self {
        Self(MAX_LOOP_ROUNDS)
    }

    /// Charge one more round of a loop that is calling `def`.
    pub(crate) fn charge(&mut self, def: &FuncDefData) -> Result<(), EvalError> {
        match self.0.checked_sub(1) {
            Some(left) => {
                self.0 = left;
                Ok(())
            }
            None => Err(def_call_depth_refusal(def)),
        }
    }
}
