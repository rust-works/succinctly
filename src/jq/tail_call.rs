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
pub(crate) fn stage_settles(expr: &Expr) -> bool {
    let mut budget = SETTLE_BUDGET;
    settles(expr, &mut budget)
}

fn settles(expr: &Expr, budget: &mut u32) -> bool {
    let Some(left) = budget.checked_sub(1) else {
        return false;
    };
    *budget = left;
    match expr {
        Expr::Paren(inner) | Expr::Negate(inner) => settles(inner, budget),
        Expr::Pipe(stages) => stages.iter().all(|stage| settles(stage, budget)),
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => settles(cond, budget) && settles(then_branch, budget) && settles(else_branch, budget),
        Expr::Arithmetic { left, right, .. }
        | Expr::Compare { left, right, .. }
        | Expr::And(left, right)
        | Expr::Or(left, right)
        | Expr::Alternative(left, right) => settles(left, budget) && settles(right, budget),
        // `[f]` collects every output of `f`: one array, or the error `f` raised.
        // An update answers once whatever its filter yields (it takes the first).
        Expr::Array(_) | Expr::Update { .. } => true,
        // An assignment answers once per output of its right side (`rhs as $v | ...`
        // in jq's own definition).
        Expr::Assign { value, .. }
        | Expr::CompoundAssign { value, .. }
        | Expr::AlternativeAssign { value, .. } => settles(value, budget),
        Expr::DefCall { def, args, .. } if args.is_empty() && def.params.is_empty() => def
            .settles_or_init(|body| {
                let mut fresh = SETTLE_BUDGET;
                settles(body, &mut fresh)
            }),
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
pub(crate) fn pipe_settles_before_last(stages: &PipeStages) -> bool {
    let Some((_, before)) = stages.split_last() else {
        return false;
    };
    !pipe_needs_path_context(stages) && before.iter().all(stage_settles)
}

/// Whether a walk of `def`'s body could reach a call at the end of it, for jq
/// (`jq`) or yq mode; remembered on the definition (#5379).
///
/// Asked on every call of every `def`, so the whole answer is a read of a cell
/// after the first. Of the raw body, where the definition's own calls are still
/// calls by name, so it is optimistic about a zero-argument call (it may be a
/// builtin, or a self call a stage ahead of it would not let the walk past): a
/// yes only sends the call through the loop, whose answer is the nested one's.
pub(crate) fn tail_reachable(def: &FuncDefData, jq: bool) -> bool {
    def.tail_reachable_or_init(jq, |body| reach(body, jq))
}

fn reach(body: &Expr, jq: bool) -> bool {
    let mut expr = body;
    loop {
        match expr {
            Expr::Paren(inner) => expr = inner,
            Expr::FuncCall { args, .. } => return args.is_empty(),
            Expr::DefCall { .. } => return tail_def(expr).is_some(),
            // The walk binds what follows a `def` declared in the body.
            Expr::FuncDef { then, .. } => expr = then,
            // `src as $x | body` is passed through in jq mode only.
            Expr::As {
                expr: source, body, ..
            } => {
                if !jq || !stage_settles(source) {
                    return false;
                }
                expr = body;
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                if !stage_settles(cond) {
                    return false;
                }
                if reach(then_branch, jq) {
                    return true;
                }
                expr = else_branch;
            }
            Expr::Pipe(stages) => match stages.last() {
                Some(last) if pipe_settles_before_last(stages) => expr = last,
                _ => return false,
            },
            Expr::Comma(items) => match items.last() {
                Some(last) => expr = last,
                None => return false,
            },
            _ => return false,
        }
    }
}

/// [`reach`] for what a `def` the interpreter has not bound yet declares: a
/// zero-argument call by name at the end of the spine, with the stages in front
/// of it left to the interpreter's own question (it keeps its own notion of
/// "answers once", over its environment).
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

/// The most outputs a tail loop may emit ahead of its calls (the front branches
/// of a comma), over all its rounds (#5379): 2^22.
///
/// A generator that tail-calls itself (`def f: ., (. + 1 | f)`) emits one output
/// per round, and a consumer that collects (`[...]`, an argument of a builtin) holds
/// them all: the round budget alone would let it hold 2^24 of them, gigabytes,
/// before it refused. jq's own loop runs until its heap is gone.
const MAX_LOOP_OUTPUTS: usize = 1 << 22;

/// What a tail loop may still spend (#5379).
///
/// jq loops forever on `def f: f; f`, in constant memory. This stops it, with the
/// refusal the nested form reached, after [`MAX_LOOP_ROUNDS`] rounds: the same
/// bound `until` and `while` keep for a loop that steps in place (#5292), counted
/// from the start of each loop, and it cannot tell a loop that ends from one that
/// does not. The outputs it emits ahead of its calls are bounded separately.
pub(crate) struct TailRounds {
    rounds: usize,
    outputs: usize,
    def: Rc<FuncDefData>,
}

impl TailRounds {
    /// The budget of a loop that started at a call to `def`, which it is named for.
    pub(crate) fn new(def: &Rc<FuncDefData>) -> Self {
        Self {
            rounds: MAX_LOOP_ROUNDS,
            outputs: MAX_LOOP_OUTPUTS,
            def: Rc::clone(def),
        }
    }

    /// Charge one more round.
    pub(crate) fn charge_round(&mut self) -> Result<(), EvalError> {
        match self.rounds.checked_sub(1) {
            Some(left) => {
                self.rounds = left;
                Ok(())
            }
            None => Err(def_call_depth_refusal(&self.def)),
        }
    }

    /// Charge `emitted` outputs a round sent to the consumer.
    pub(crate) fn charge_outputs(&mut self, emitted: usize) -> Result<(), EvalError> {
        match self.outputs.checked_sub(emitted) {
            Some(left) => {
                self.outputs = left;
                Ok(())
            }
            None => Err(EvalError::resource_limit(format!(
                "{}/{}: maximum outputs exceeded",
                ModuleRun::display_name(&self.def.name),
                self.def.params.len()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn the_spine_ends_in_a_zero_argument_call_5379() {
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
            ("def f: def g: f; . + 1 | g; f", true, true),
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

    #[test]
    fn a_call_with_an_argument_is_not_a_jump_5379() {
        let def = definition("def f(n): if n > 0 then f(n - 1) else 0 end; f(3)");
        // `f(n - 1)` is a call by name with an argument: not a zero-argument call.
        assert!(!tail_reachable(&def, true));
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
            assert_eq!(stage_settles(&expr), want, "`{src}`");
        }
    }

    /// A call to a def with no parameters settles when the def's body does, and the answer
    /// is the definition's own: asked again, it is read back.
    #[test]
    fn a_call_to_a_settling_def_settles_5379() {
        use crate::jq::eval::install_def_calls;
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
            assert_eq!(stage_settles(&call), want, "`{def_src}`");
            assert_eq!(stage_settles(&call), want, "`{def_src}`, remembered");
        }
    }

    #[test]
    fn the_budgets_run_out_5379() {
        let def = definition("def f: f; f");
        let mut rounds = TailRounds::new(&def);
        rounds.rounds = 2;
        assert!(rounds.charge_round().is_ok());
        assert!(rounds.charge_round().is_ok());
        let refused = rounds
            .charge_round()
            .expect_err("the third round is over budget");
        assert_eq!(refused.to_string(), "f/0 exceeded maximum recursion depth");

        let mut rounds = TailRounds::new(&def);
        assert_eq!(rounds.rounds, MAX_LOOP_ROUNDS);
        assert!(rounds.charge_outputs(MAX_LOOP_OUTPUTS).is_ok());
        let refused = rounds
            .charge_outputs(1)
            .expect_err("one output past the budget");
        assert_eq!(refused.to_string(), "f/0: maximum outputs exceeded");
    }
}
