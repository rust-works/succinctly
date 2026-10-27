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
use super::expr::{Builtin, Expr, FuncDefData, PipeStages};

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
            // `src as $x | body`: the call is in `body`, which keeps what follows
            // `as` last in the body.
            Expr::As { body, .. } => expr = body,
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
pub(crate) fn declared_tail_call(then: &Expr) -> bool {
    let mut expr = then;
    loop {
        match expr {
            Expr::Paren(inner) => expr = inner,
            Expr::As { body, .. } => expr = body,
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
        || settles_once_more(expr)
}

/// What [`stage_settles`] adds to the two recognitions it borrows: the shapes a
/// loop that carries its state in the value writes ahead of its call
/// (`.n += 1 | f`, `.a |= .[1:] | f`, `map(g) | f`) and that neither names.
///
/// An assignment answers once per output of its right side (`rhs as $v | ...`
/// in jq's own definition), so it settles when that side does; `|=` answers
/// once whatever its filter yields (it takes the first). The builtins are the
/// ones that collect, or compute, one answer per input.
fn settles_once_more(expr: &Expr) -> bool {
    match expr {
        Expr::Paren(inner) => stage_settles(inner),
        Expr::Pipe(stages) => stages.iter().all(stage_settles),
        Expr::Assign { value, .. }
        | Expr::CompoundAssign { value, .. }
        | Expr::AlternativeAssign { value, .. } => stage_settles(value),
        // `[f]` collects every output of `f`: one array, or the error `f` raised.
        Expr::Update { .. } | Expr::Array(_) => true,
        Expr::Builtin(builtin) => matches!(
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
                | Builtin::Min
                | Builtin::Max
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
        ),
        _ => false,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jq::eval::install_def_calls;
    use crate::jq::parse;

    /// The bound body of `def f: <body>; f`: the one a call to `f` runs, with `f`'s own calls
    /// installed as `DefCall`s.
    fn bound_body(src: &str) -> (Rc<FuncDefData>, Rc<Expr>) {
        let Expr::FuncDef {
            name, params, body, ..
        } = parse(src).expect("parses")
        else {
            panic!("`{src}` does not start with a def");
        };
        let def = Rc::new(FuncDefData::new(name, params, *body));
        let body = bind_def_call_body(&def, &[], 0).expect("binds");
        (def, body)
    }

    #[test]
    fn the_spine_ends_in_a_zero_argument_call_5379() {
        for (src, want) in [
            ("def f: f; f", true),
            ("def f: if . < 3 then . + 1 | f else . end; f", true),
            ("def f: if . < 3 then . else . + 1 | f end; f", true),
            (
                "def f: if . < 3 then . + 1 | f elif . < 5 then . | f else . end; f",
                true,
            ),
            ("def f: (. + 1 | f); f", true),
            ("def f: ., (. + 1 | f); f", true),
            ("def f: def g: f; . + 1 | g; f", true),
            ("def f: . as $x | f; f", true),
            ("def f: . as $x | if $x then . + 1 | f else . end; f", true),
            ("def f: . as $x | [f]; f", false),
            // A call that is not last, or not a call, or not a jump in jq.
            ("def f: f | . + 1; f", false),
            ("def f: [f]; f", false),
            ("def f: 1 + f; f", false),
            ("def f: f, 1; f", false),
            ("def f: try f catch .; f", false),
            ("def f: f?; f", false),
            ("def f: label $out | f; f", false),
            ("def f: if f then . else . end; f", false),
            ("def f: . + 1; f", false),
            ("def f: if . < 3 then 1 else 2 end; f", false),
        ] {
            let (_, body) = bound_body(src);
            assert_eq!(spine_has_tail_call(&body), want, "`{src}`");
        }
    }

    #[test]
    fn a_call_with_an_argument_is_not_a_jump_5379() {
        let Expr::FuncDef {
            name, params, body, ..
        } = parse("def f(n): if n > 0 then f(n - 1) else 0 end; f(3)").expect("parses")
        else {
            panic!("a def");
        };
        let def = Rc::new(FuncDefData::new(name, params, *body));
        let installed = install_def_calls(&def.body, &def, 0, true);
        assert!(!spine_has_tail_call(&installed));
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
            // Writes ahead of a call, and what collects.
            (".n += 1", true),
            (".a |= . + 1", true),
            (".a = 1", true),
            (".a //= 1", true),
            ("map(. + 1)", true),
            ("sort", true),
            ("del(.a)", true),
            // A generator, however it is spelled.
            (".[]", false),
            ("(., .)", false),
            ("1, 2", false),
            ("range(3)", false),
            ("recurse", false),
            (".n += (1, 2)", false),
            ("first(.[])", false),
            ("limit(3; .[])", false),
            ("empty", true),
        ] {
            let expr = parse(src).expect("parses");
            assert_eq!(stage_settles(&expr), want, "`{src}`");
        }
    }

    #[test]
    fn rebinding_shares_a_body_per_def_5379() {
        let (def, body) = bound_body("def f: if . < 3 then . + 1 | f else . end; f");
        let (other, _) = bound_body("def g: . + 1; g");
        let mut bodies = TailBindings::new(&def, &body, 0);
        // A self call runs the body it is already running.
        assert!(Rc::ptr_eq(&bodies.body_of(&def).expect("self"), &body));
        // Another def binds once and is kept.
        let first = bodies.body_of(&other).expect("binds");
        let again = bodies.body_of(&other).expect("kept");
        assert!(Rc::ptr_eq(&first, &again));
    }

    #[test]
    fn the_round_budget_runs_out_5379() {
        let (def, _) = bound_body("def f: f; f");
        let mut rounds = TailRounds(2);
        assert!(rounds.charge(&def).is_ok());
        assert!(rounds.charge(&def).is_ok());
        let refused = rounds
            .charge(&def)
            .expect_err("the third round is over budget");
        assert_eq!(refused.to_string(), "f/0 exceeded maximum recursion depth");
        assert_eq!(TailRounds::new().0, MAX_LOOP_ROUNDS);
    }
}
