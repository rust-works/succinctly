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
#[cfg(not(test))]
use alloc::vec::Vec;
#[cfg(test)]
use std::rc::Rc;

use super::error::EvalError;
use super::eval::{
    bind_def_call, bind_def_call_body, def_call_depth_refusal, pipe_needs_path_context,
    single_valued_pure, yields_at_most_one_value, Flow, PureWalk, MAX_LOOP_ROUNDS,
};
use super::expr::{BoundBody, Builtin, Expr, FuncDefData, ObjectKey, PipeStages, StringPart};
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
    /// The definition whose body is being walked for [`call_head_settles`] (#7460): a call to
    /// itself with its arity, and a read of one of its bare parameters, are taken to settle.
    own: Option<&'a FuncDefData>,
    /// Whether this is [`call_head_settles`]'s question (#7713), which also reads the shapes that
    /// answer at most once without being a stage a tail loop runs ahead of its call: `try`, `?`, a
    /// `label`, a `reduce`, an object, `first`/`last`, a call to another def with arguments, and
    /// a def declared in the body. The tail loop's own question (`wide: false`) is unchanged.
    wide: bool,
    /// The defs declared in the body being walked, innermost last, with whether each one's body
    /// settles (#7713; #7787: with parameters too, in the wide question): a call to one, by name
    /// and arity, settles when that body does and its arguments do.
    local_defs: &'a [LocalDef<'a>],
    /// The bare parameters in scope of the body being walked (#7787, wide only): each reads an
    /// argument the call that bound it was asked about, so a read of one settles.
    params: &'a [&'a str],
    /// Whether a read of one of `own`'s bare parameters is also taken not to raise after its output
    /// (#7928): the half of [`call_settles`] asked when the plain question says no, which then
    /// checks each argument of the call it answers for, and each argument of the recursive calls
    /// in the body, cannot raise late either.
    quiet_params: bool,
}

/// A def declared in a body [`settles`] walks: what a call to it by name settles on.
#[derive(Clone, Copy)]
struct LocalDef<'a> {
    name: &'a str,
    arity: usize,
    settles: bool,
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
    settles_in(
        expr,
        &Ctx {
            jq,
            locals: &[],
            own: None,
            wide: false,
            local_defs: &[],
            params: &[],
            quiet_params: false,
        },
    )
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
        // #7460: within a definition's body, a call to the definition itself settles by induction
        // on the recursion (every level's body settles given that its calls do), once its arguments
        // do, and a bare parameter's read reads an argument its call already checked.
        Expr::FuncCall { name, args, .. }
            if cx.own.is_some_and(|def| {
                (*name == def.name && args.len() == def.params.len())
                    || (args.is_empty() && def.params.iter().any(|p| p.name() == name))
            }) && !shadowed_by_local(cx, name, args.len()) =>
        {
            // #7928: under `quiet_params` the induction also needs the recursive call's arguments
            // not to raise late (a read of a bare parameter is taken not to).
            args.iter().all(|arg| {
                settles(arg, budget, cx) && !(cx.quiet_params && may_raise_after_output(arg, cx))
            })
        }
        // #7460: one value bound, then one body run for it, as `single_valued_pure` reads `as`;
        // asked only for a call at a pipe's head, so the tail loop's own question is unchanged.
        Expr::As {
            expr: source, body, ..
        } if cx.own.is_some() => settles(source, budget, cx) && settles(body, budget, cx),
        // One write, or read, per path (and value) its arguments yield: a memoised recursion
        // stores its answer with `setpath` (Rosetta Code's Hofstadter Q sequence).
        Expr::Builtin(Builtin::SetPath(path, value)) if cx.own.is_some() => {
            settles(path, budget, cx) && settles(value, budget, cx)
        }
        Expr::Builtin(Builtin::GetPath(path) | Builtin::DelPaths(path)) if cx.own.is_some() => {
            settles(path, budget, cx)
        }
        // #7713, #7787: the call-head question (`wide`) reads every form of the language, each by
        // what it yields: at most one output, and nothing left to run once it has given it but an
        // `empty`, a `break` or an error (see `comma_settles`). A form that can yield more, or
        // whose count depends on the values it reads (`.[]`, `..`, `range`, `limit(2; ...)`, the
        // regex generators), does not settle; nor does a form this walk cannot see into
        // (a call by namespace). The tail loop's question (`wide: false`) is unchanged.
        Expr::Identity
        | Expr::Field(_)
        | Expr::Index { .. }
        | Expr::Slice { .. }
        | Expr::ArrayKey(_)
        | Expr::Literal(_)
        | Expr::Var(_)
        | Expr::TrackedVar(_)
        | Expr::DeferredVar(_)
        | Expr::Loc { .. }
        | Expr::Env
        | Expr::Not
        | Expr::Format(_)
        | Expr::Break(_)
            if cx.wide =>
        {
            true
        }
        Expr::Iterate
        | Expr::RecursiveDescent
        | Expr::RecursiveDescentWithKeys
        | Expr::Range { .. }
        | Expr::Repeat(_)
        | Expr::While { .. }
        | Expr::NamespacedCall { .. }
        | Expr::MetaAssign { .. }
            if cx.wide =>
        {
            false
        }
        Expr::Comma(items) if cx.wide => comma_settles(items, budget, cx),
        // A `try` or `?`: its body's one output, or else its handler's. #7928: a body that settles
        // may still raise after its output (`1, error("x")`, `comma_settles`), and jq's `try`
        // catches that error, or a `break`, and runs the handler too: `try (1, error("x")) catch 0`
        // is `1, 0`. A handler that yields is therefore taken only over a body that cannot.
        Expr::Try { expr, catch } if cx.wide => {
            settles(expr, budget, cx)
                && catch.as_deref().map_or(true, |c| {
                    settles(c, budget, cx)
                        && (yields_nothing(c) || !may_raise_after_output(expr, cx))
                })
        }
        // A `label` only stops what it holds (a `break` settles, see `comma_settles`).
        Expr::Optional(inner) | Expr::Label { body: inner, .. } if cx.wide => {
            settles(inner, budget, cx)
        }
        // One output per output of the initial value; the source and the update run inside it.
        Expr::Reduce { init, .. } if cx.wide => settles(init, budget, cx),
        // One round, for a source and an update that answer once.
        Expr::Foreach {
            input,
            patterns,
            init,
            update,
            extract,
        } if cx.wide => {
            patterns.len() == 1
                && settles(input, budget, cx)
                && settles(init, budget, cx)
                && settles(update, budget, cx)
                && extract.as_deref().map_or(true, |e| settles(e, budget, cx))
        }
        // `first`, `last` and `nth(n; f)` answer at most once whatever they read; `limit(n; f)`
        // does for a literal count of 0 or 1, or for a source that does. (A `?//` that a later
        // failure retries through them is refused at the question's entry, `call_head_settles`.)
        Expr::NthExpr { n, .. } if cx.wide => settles(n, budget, cx),
        Expr::Limit { n, expr } if cx.wide => {
            limit_settles(n) || (settles(n, budget, cx) && settles(expr, budget, cx))
        }
        Expr::Until { cond, update } if cx.wide => {
            settles(cond, budget, cx) && settles(update, budget, cx)
        }
        Expr::As {
            expr: source, body, ..
        } if cx.wide => settles(source, budget, cx) && settles(body, budget, cx),
        // A destructuring bind with one pattern binds once per output of its source; one with
        // alternatives (`?//`) is retried by a failure raised after it, so it streams.
        Expr::AsPattern {
            expr: source,
            patterns,
            body,
        } if cx.wide => {
            patterns.len() == 1 && settles(source, budget, cx) && settles(body, budget, cx)
        }
        Expr::Object(entries) if cx.wide => entries.iter().all(|entry| {
            (match &entry.key {
                ObjectKey::Expr(key) => settles(key, budget, cx),
                ObjectKey::Literal(_) | ObjectKey::Bare => true,
            }) && settles(&entry.value, budget, cx)
        }),
        // #7795: `first(f)` stops `f` at its first output, but a `?//` in `f` is still retried by a
        // failure raised after the call has returned (jq retries it through `first`), so it keeps
        // the streaming route. `last(f)` runs `f` to the end, and jq does not retry through it.
        Expr::FirstExpr(inner) if cx.wide => !super::eval::contains_retrying_pattern_bind(inner),
        Expr::LastExpr(_) if cx.wide => true,
        Expr::StringInterpolation(parts) if cx.wide => parts.iter().all(|part| match part {
            StringPart::Literal(_) => true,
            StringPart::Expr(inner) => settles(inner, budget, cx),
        }),
        Expr::IndexExpr { target, key } if cx.wide && cx.jq => {
            settles(target, budget, cx) && settles(key, budget, cx)
        }
        Expr::SliceExpr { target, start, end } if cx.wide && cx.jq => {
            settles(target, budget, cx)
                && start.as_deref().map_or(true, |e| settles(e, budget, cx))
                && end.as_deref().map_or(true, |e| settles(e, budget, cx))
        }
        Expr::Error(msg) if cx.wide => msg.as_deref().map_or(true, |m| settles(m, budget, cx)),
        Expr::Builtin(builtin) if cx.wide => builtin_settles_wide(builtin, budget, cx),
        // An argument a call captured: caller-scope code, asked the question once (its answer is
        // remembered on it), so a closure passed down a recursion is not re-walked at each level.
        Expr::Shared(arg) if cx.wide => arg.settles_or_init(|inner| {
            let mut fresh = SETTLE_BUDGET;
            settles(inner, &mut fresh, &Ctx::wide(cx.jq))
        }),
        Expr::DefCall { .. } if cx.wide => call_head_settles_in(expr, cx.jq, budget),
        // A def declared in the body: its calls settle when its body does, which is walked with
        // its own calls taken to settle (induction, as for the definition being asked about) and,
        // in the wide question, its bare parameters too. In both questions for a def with no
        // parameters: a module's forwarding stub is one (`def g: <link name>;` wrapped into the
        // body of a def that calls `g`, `jq::ModuleRun`), #7713.
        Expr::FuncDef {
            name,
            params,
            body,
            then,
            ..
        } => {
            let mut inner_defs: Vec<LocalDef<'_>> = cx.local_defs.to_vec();
            inner_defs.push(LocalDef {
                name,
                arity: params.len(),
                settles: true,
            });
            let mut inner_params: Vec<&str> = cx.params.to_vec();
            if cx.wide {
                inner_params.extend(params.iter().map(super::expr::Param::name));
            } else {
                // The tail loop's question takes no induction step: as before #7787.
                inner_defs.pop();
            }
            let body_settles = (cx.wide || params.is_empty())
                && settles(
                    body,
                    budget,
                    &Ctx {
                        // A call from in here to the definition being asked about is still one
                        // of its recursive calls (wide); the tail loop's question never had one.
                        own: if cx.wide { cx.own } else { None },
                        local_defs: &inner_defs,
                        params: &inner_params,
                        ..*cx
                    },
                );
            let mut known: Vec<LocalDef<'_>> = cx.local_defs.to_vec();
            known.push(LocalDef {
                name,
                arity: params.len(),
                settles: body_settles,
            });
            settles(
                then,
                budget,
                &Ctx {
                    local_defs: &known,
                    ..*cx
                },
            )
        }
        // A call by name to a def declared in the body (the innermost of that name and arity),
        // then a read of a bare parameter in scope.
        Expr::FuncCall { name, args, .. }
            if cx
                .local_defs
                .iter()
                .any(|local| local.name == name && local.arity == args.len()) =>
        {
            let local = cx
                .local_defs
                .iter()
                .rev()
                .find(|local| local.name == name && local.arity == args.len())
                .copied();
            local.is_some_and(|local| local.settles)
                && (cx.wide || args.is_empty())
                && args.iter().all(|arg| settles(arg, budget, cx))
        }
        Expr::FuncCall { name, args, .. }
            if cx.wide && args.is_empty() && cx.params.contains(&name.as_str()) =>
        {
            true
        }
        Expr::DefCall { def, args, .. } if args.is_empty() && def.params.is_empty() => def
            .settles_or_init(cx.jq, |body| {
                let mut fresh = SETTLE_BUDGET;
                settles(
                    body,
                    &mut fresh,
                    &Ctx {
                        jq: cx.jq,
                        locals: &[],
                        own: None,
                        wide: false,
                        local_defs: &[],
                        params: &[],
                        quiet_params: false,
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

/// [`settles`] for a builtin, in the wide question (#7787): every builtin of the language, by
/// what it yields for one input. One value (or an error, or nothing) per input when its arguments
/// answer once: the scalar and container functions, the type filters, `select`, the path readers
/// and writers, `test` and `sub`; one value whatever its closure yields: the collectors (`map`,
/// `sort_by`, `with_entries`, `del`, `pick`, `INDEX`) and the ones that stop at a first output
/// (`isempty`, `any`, `all`, `IN`, `first(f)`); and an effect before the one output is no
/// obstacle (`debug`, `stderr`, `input`, `input_line_number`, `halt`), since it happens there on
/// the streaming route too. The generators (`.[]`-like, `recurse`, `paths`, `tostream`, `inputs`,
/// `splits`, `scan`, `match`, `capture`, `combinations`) do not, nor do yq's node and document
/// builtins, which jq mode never reaches. Listed without a wildcard, so a new builtin is a
/// compile error here rather than an unasked answer.
fn builtin_settles_wide(builtin: &Builtin, budget: &mut u32, cx: &Ctx<'_>) -> bool {
    let mut each = |args: &[&Expr]| args.iter().all(|arg| settles(arg, budget, cx));
    match builtin {
        Builtin::Type
        | Builtin::IsNull
        | Builtin::IsBoolean
        | Builtin::IsNumber
        | Builtin::IsString
        | Builtin::IsArray
        | Builtin::IsObject
        | Builtin::Values
        | Builtin::Nulls
        | Builtin::Booleans
        | Builtin::Numbers
        | Builtin::Strings
        | Builtin::Arrays
        | Builtin::Objects
        | Builtin::Iterables
        | Builtin::Scalars
        | Builtin::Normals
        | Builtin::Finites
        | Builtin::Length
        | Builtin::Utf8ByteLength
        | Builtin::Keys
        | Builtin::KeysUnsorted
        | Builtin::Empty
        | Builtin::Add
        | Builtin::Any
        | Builtin::All
        | Builtin::Min
        | Builtin::Max
        | Builtin::AsciiDowncase
        | Builtin::AsciiUpcase
        | Builtin::Downcase
        | Builtin::Upcase
        | Builtin::First
        | Builtin::Last
        | Builtin::Reverse
        | Builtin::Flatten
        | Builtin::Unique
        | Builtin::Sort
        | Builtin::ToEntries
        | Builtin::FromEntries
        | Builtin::ToString
        | Builtin::ToNumber
        | Builtin::ToJson
        | Builtin::FromJson
        | Builtin::Explode
        | Builtin::Implode
        | Builtin::Floor
        | Builtin::Ceil
        | Builtin::Round
        | Builtin::Sqrt
        | Builtin::Fabs
        | Builtin::Log
        | Builtin::Log10
        | Builtin::Log2
        | Builtin::Exp
        | Builtin::Exp10
        | Builtin::Exp2
        | Builtin::Sin
        | Builtin::Cos
        | Builtin::Tan
        | Builtin::Asin
        | Builtin::Acos
        | Builtin::Atan
        | Builtin::Sinh
        | Builtin::Cosh
        | Builtin::Tanh
        | Builtin::Asinh
        | Builtin::Acosh
        | Builtin::Atanh
        | Builtin::Libm1(_)
        | Builtin::Infinite
        | Builtin::Nan
        | Builtin::IsInfinite
        | Builtin::IsNan
        | Builtin::IsNormal
        | Builtin::IsFinite
        | Builtin::Debug
        | Builtin::Halt
        | Builtin::Stderr
        | Builtin::HaltError
        | Builtin::Env
        | Builtin::EnvObject(_)
        | Builtin::StrEnv(_)
        | Builtin::NullLit
        | Builtin::Trim
        | Builtin::Ltrim
        | Builtin::Rtrim
        | Builtin::Transpose
        | Builtin::ModuleMeta
        | Builtin::Now
        | Builtin::Input
        | Builtin::InputLineNumber
        | Builtin::InputFilename
        | Builtin::GetSearchList
        | Builtin::GetJqOrigin
        | Builtin::GetProgOrigin
        | Builtin::Abs
        | Builtin::Builtins
        | Builtin::Gmtime
        | Builtin::Localtime
        | Builtin::Mktime
        | Builtin::Todate
        | Builtin::Fromdate
        | Builtin::Todateiso8601
        | Builtin::Fromdateiso8601
        | Builtin::Trunc
        | Builtin::ToBoolean => true,
        // Collect every output of their closure, or stop at its first: one value either way.
        Builtin::Map(_)
        | Builtin::MapValues(_)
        | Builtin::WithEntries(_)
        | Builtin::MinBy(_)
        | Builtin::MaxBy(_)
        | Builtin::GroupBy(_)
        | Builtin::UniqueBy(_)
        | Builtin::SortBy(_)
        | Builtin::Del(_)
        | Builtin::Pick(_)
        | Builtin::UpperIndex(_)
        | Builtin::UpperIndexStream(_, _)
        | Builtin::AnyF(_)
        | Builtin::AllF(_)
        | Builtin::AnyCond(_, _)
        | Builtin::AllCond(_, _)
        | Builtin::UpperIn(_)
        | Builtin::UpperInSrc(_, _)
        | Builtin::IsEmpty(_)
        | Builtin::FirstStream(_)
        | Builtin::LastStream(_) => true,
        Builtin::NthStream(n, _) => each(&[n]),
        Builtin::Limit(n, f) => limit_settles(n) || each(&[n, f]),
        // One value per value of their arguments.
        Builtin::Has(a)
        | Builtin::In(a)
        | Builtin::Select(a)
        | Builtin::Ltrimstr(a)
        | Builtin::Rtrimstr(a)
        | Builtin::Startswith(a)
        | Builtin::Endswith(a)
        | Builtin::Split(a)
        | Builtin::Join(a)
        | Builtin::Contains(a)
        | Builtin::Inside(a)
        | Builtin::Nth(a)
        | Builtin::FlattenDepth(a)
        | Builtin::Test(a)
        | Builtin::Indices(a)
        | Builtin::Index(a)
        | Builtin::Rindex(a)
        | Builtin::GetPath(a)
        | Builtin::Walk(a)
        | Builtin::IsValid(a)
        | Builtin::Path(a)
        | Builtin::DelPaths(a)
        | Builtin::DebugMsg(a)
        | Builtin::HaltErrorCode(a)
        | Builtin::EnvVar(a)
        | Builtin::BSearch(a)
        | Builtin::Strftime(a)
        | Builtin::FormatNamed(a)
        | Builtin::Strflocaltime(a)
        | Builtin::Strptime(a)
        | Builtin::AtOffset(a) => each(&[a]),
        Builtin::SetPath(a, b)
        | Builtin::Pow(a, b)
        | Builtin::Atan2(a, b)
        | Builtin::Libm2(_, a, b)
        | Builtin::TestFlags(a, b)
        | Builtin::Sub(a, b)
        | Builtin::Gsub(a, b)
        | Builtin::SplitRegex(a, b)
        | Builtin::AtPosition(a, b) => each(&[a, b]),
        Builtin::Libm3(_, a, b, c) | Builtin::SubFlags(a, b, c) | Builtin::GsubFlags(a, b, c) => {
            each(&[a, b, c])
        }
        // Generators, and what jq mode never reaches (yq's node and document builtins).
        Builtin::ToJsonStream
        | Builtin::FromJsonStream
        | Builtin::ToStream
        | Builtin::FromStream(_)
        | Builtin::TruncateStream(_)
        | Builtin::Recurse
        | Builtin::RecurseF(_)
        | Builtin::RecurseCond(_, _)
        | Builtin::RecurseDown
        | Builtin::PathNoArg
        | Builtin::Parent
        | Builtin::ParentN(_)
        | Builtin::Paths
        | Builtin::PathsFilter(_)
        | Builtin::LeafPaths
        | Builtin::Inputs
        | Builtin::Match(_)
        | Builtin::MatchFlags(_, _)
        | Builtin::Capture(_)
        | Builtin::CaptureFlags(_, _)
        | Builtin::Scan(_)
        | Builtin::ScanFlags(_, _)
        | Builtin::Splits(_)
        | Builtin::SplitsFlags(_, _)
        | Builtin::Combinations
        | Builtin::CombinationsN(_)
        | Builtin::Skip(_, _)
        | Builtin::FromYaml
        | Builtin::TextCodec(_)
        | Builtin::Envsubst(_)
        | Builtin::Filter(_)
        | Builtin::AnyC(..)
        | Builtin::AllC(..)
        | Builtin::Omit(_)
        | Builtin::Tag
        | Builtin::Anchor
        | Builtin::Alias
        | Builtin::Style
        | Builtin::Kind
        | Builtin::Key
        | Builtin::Line
        | Builtin::Column
        | Builtin::DocumentIndex
        | Builtin::LineComment
        | Builtin::HeadComment
        | Builtin::FootComment
        | Builtin::FileIndex
        | Builtin::Filename
        | Builtin::Shuffle
        | Builtin::Pivot
        | Builtin::SplitDoc
        | Builtin::SortKeys(_)
        | Builtin::SortKeysOneLevel
        | Builtin::FromUnix
        | Builtin::ToUnix
        | Builtin::Tz(_)
        | Builtin::FormatDatetime(_)
        | Builtin::WithDtf(_, _)
        | Builtin::EvalExpr(_, _)
        | Builtin::Load(_, _)
        | Builtin::System(_, _) => false,
    }
}

/// How many nodes [`call_head_settles`] walks of a call's arguments. Small: they are walked on
/// every evaluation of the pipe the call leads, and a long one (a lazy argument chain) only keeps the
/// route the pipe took before.
const CALL_ARGS_BUDGET: u32 = 128;

/// Whether `expr` may be run to its one value before its consumer runs (#7460, #7713, #7787): a
/// call to a def that may recurse sits on its spine ([`may_hold_call`], a shape test), and it
/// answers at most once ([`answers_once`]: every form of the language read by what it yields; a
/// definition's body is asked once, with its own recursive calls and bare parameters taken to
/// settle, and remembered on the definition). Asked at a pipe's head, of an `as` source or
/// argument, of an object's key or value, of a string interpolation's part, and of an operator's
/// operands.
///
/// A tree recursion that pipes one call into the next, `def f($n): if $n == 0 then . + 1 else
/// f($n - 1) | f($n - 1) end`, ran the second call inside the first call's output sink, so every
/// call ever made stayed on the native stack until the outermost one returned: 8,192 calls at
/// depth 13 spent it, where jq's stack holds only the live calls. Settled, the first call returns
/// before the second runs, and the stack holds the recursion's depth.
#[inline(always)]
pub(crate) fn call_head_settles(expr: &Expr, jq: bool) -> bool {
    may_hold_call(expr, SPINE_DEPTH) && answers_once(expr, jq)
}

/// Whether `expr` is a call to a def (inside parentheses or not) that answers at most once, whether
/// or not the def may recurse: the owned-call door's question for a pipe a call leads (#7483,
/// #7702), which runs the call to its outputs and the rest of the pipe over each with the value
/// handed down. [`call_head_settles`] also asks that the call may recurse (#7787), which is a
/// question of native stack depth: a door that settles a helper's call writes in place where the
/// re-index bridge would copy, whatever the helper's depth. Asked only by the door
/// (`eval::owned_call`), which asked `call_head_settles` the same question before #7787.
pub(crate) fn call_answers_once(expr: &Expr, jq: bool) -> bool {
    matches!(super::eval::unwrap_paren(expr), Expr::DefCall { .. }) && answers_once(expr, jq)
}

/// Whether `expr` answers at most once and leaves nothing observable to run after its output
/// (#7787): the call-head question without its requirement that a def be called, for the other
/// operand of an operator whose one operand is such a call (`eval::operand_pair_settles`).
pub(crate) fn answers_once(expr: &Expr, jq: bool) -> bool {
    let mut budget = CALL_ARGS_BUDGET;
    !super::eval::contains_retrying_pattern_bind(expr) && settles(expr, &mut budget, &Ctx::wide(jq))
}

/// How deep [`may_hold_call`] looks along an expression's spine.
pub(crate) const SPINE_DEPTH: u32 = 4;

/// Whether a call to a `def` sits on `expr`'s spine (#7787): the expression itself, or what a
/// parenthesis, a pipe, `//`, `first`/`last`/`limit`/`nth`, `try`/`?`, a `label`, an `if`, an
/// `as`, a `reduce`/`foreach`/`until`, a `,`, an object or a string interpolation holds, to
/// `depth` levels. The only shapes worth running to their value first: a call there is what
/// would still hold deep native frames when the expression's output reaches its consumer.
///
/// A shape test, not a walk: a leaf, an operator, a builtin or anything else answers no at once,
/// so the operands of `.age > 30` or `.score + .id` cost one match each, on every evaluation. A
/// call further down (inside an operator or a builtin's closure) keeps the route it took before;
/// the operator around it is its own site and settles its operands itself.
#[inline(always)]
pub(crate) fn may_hold_call(expr: &Expr, depth: u32) -> bool {
    // The answer for every leaf, operator and builtin, inline: one discriminant test, so a site
    // whose operands are `.age` and `30` pays nothing more than that (#7787).
    could_hold_call(expr) && may_hold_call_on_spine(expr, depth)
}

/// Whether `expr` is a call or one of the forms [`may_hold_call`] looks through.
#[inline(always)]
fn could_hold_call(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::DefCall { .. }
            | Expr::FuncCall { .. }
            | Expr::NamespacedCall { .. }
            | Expr::Paren(_)
            | Expr::Optional(_)
            | Expr::FirstExpr(_)
            | Expr::LastExpr(_)
            | Expr::Label { .. }
            | Expr::Try { .. }
            | Expr::Limit { .. }
            | Expr::NthExpr { .. }
            | Expr::Pipe(_)
            | Expr::Comma(_)
            | Expr::Alternative(..)
            | Expr::If { .. }
            | Expr::As { .. }
            | Expr::AsPattern { .. }
            | Expr::Reduce { .. }
            | Expr::Foreach { .. }
            | Expr::Until { .. }
            | Expr::Object(_)
            | Expr::StringInterpolation(_)
    )
}

/// Whether any of `items` holds a call on its spine: a plain loop, so a pipe's stages cost one
/// inline test each (`(.tags | length)`), with no iterator adapter between them.
fn any_on(items: &[Expr], depth: u32) -> bool {
    for item in items {
        if may_hold_call(item, depth) {
            return true;
        }
    }
    false
}

/// Whether a call's arguments hold a call that may recurse (a closure passed down to it).
#[inline(never)]
fn args_may_recurse(args: &[Expr]) -> bool {
    let mut budget = CALL_ARGS_BUDGET;
    for arg in args {
        if holds_recursion(arg, &mut budget) {
            return true;
        }
    }
    false
}

/// [`may_hold_call`] past its inline test. A call is answered here, in a small frame; the wrappers
/// out of line.
fn may_hold_call_on_spine(mut expr: &Expr, mut depth: u32) -> bool {
    loop {
        match expr {
            // A call to a definition that cannot recurse returns before its output reaches the
            // consumer with nothing deep left on the stack: it is not worth running first.
            Expr::DefCall { def, args, .. } => {
                return def_may_recurse(def) || (!args.is_empty() && args_may_recurse(args));
            }
            Expr::FuncCall { .. } | Expr::NamespacedCall { .. } => return true,
            // The two wrappers an operand is most often written with, stepped through here.
            Expr::Paren(inner) => {
                let Some(next) = depth.checked_sub(1) else {
                    return false;
                };
                depth = next;
                expr = inner;
            }
            Expr::Pipe(stages) => {
                let Some(next) = depth.checked_sub(1) else {
                    return false;
                };
                return any_on(stages, next);
            }
            _ => return may_hold_call_in_wrapper(expr, depth),
        }
    }
}

/// [`may_hold_call_on_spine`] for a form that holds other expressions.
#[inline(never)]
fn may_hold_call_in_wrapper(expr: &Expr, depth: u32) -> bool {
    let Some(depth) = depth.checked_sub(1) else {
        return false;
    };
    let on = |e: &Expr| may_hold_call(e, depth);
    match expr {
        Expr::Paren(inner)
        | Expr::Optional(inner)
        | Expr::FirstExpr(inner)
        | Expr::LastExpr(inner)
        | Expr::Label { body: inner, .. }
        | Expr::Try { expr: inner, .. }
        | Expr::Limit { expr: inner, .. }
        | Expr::NthExpr { expr: inner, .. } => on(inner),
        Expr::Pipe(stages) => any_on(stages, depth),
        Expr::Comma(items) => any_on(items, depth),
        Expr::Alternative(left, right) => on(left) || on(right),
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => on(cond) || on(then_branch) || on(else_branch),
        Expr::As { expr, body, .. } | Expr::AsPattern { expr, body, .. } => on(expr) || on(body),
        Expr::Reduce { input, init, .. } => on(input) || on(init),
        Expr::Foreach { input, update, .. } => on(input) || on(update),
        Expr::Until { cond, update } => on(cond) || on(update),
        Expr::Object(entries) => entries
            .iter()
            .any(|entry| on(&entry.value) || matches!(&entry.key, ObjectKey::Expr(key) if on(key))),
        Expr::StringInterpolation(parts) => parts
            .iter()
            .any(|part| matches!(part, StringPart::Expr(inner) if on(inner))),
        _ => false,
    }
}

/// Whether a call to `def` may recurse (#7787), remembered on the definition: its body calls a def
/// by name (itself, a def it declares, or a bare parameter, which may be any closure), a def bound
/// into it that may recurse, or a module's def; or the walk runs out of budget.
#[inline]
pub(crate) fn def_may_recurse(def: &FuncDefData) -> bool {
    def.may_recurse_or_init(|body| {
        let mut budget = SETTLE_BUDGET;
        holds_recursion(body, &mut budget)
    })
}

/// [`def_may_recurse`]'s walk: whether `expr` holds a call that may recurse, out of `budget`
/// nodes, answering yes when they run out. Out of line: `def_may_recurse` is asked by every
/// evaluator's `DefCall` arm (#7856), whose frame each level of a recursion pays for.
#[inline(never)]
fn holds_recursion(expr: &Expr, budget: &mut u32) -> bool {
    let mut out_of_budget = false;
    let found = search_subexpr(expr, &mut |e: &Expr| {
        let Some(left) = budget.checked_sub(1) else {
            out_of_budget = true;
            return Visit::Found;
        };
        *budget = left;
        match e {
            Expr::FuncCall { .. } | Expr::NamespacedCall { .. } => Visit::Found,
            Expr::DefCall { def, args, .. } => {
                if def_may_recurse(def) || args.iter().any(|arg| holds_recursion(arg, budget)) {
                    Visit::Found
                } else {
                    Visit::Skip
                }
            }
            _ => Visit::Descend,
        }
    });
    found || out_of_budget
}

/// [`settles`] for a call to `def` with `args` (#7460): the definition's body, taking its own
/// recursive calls and its bare parameters to settle (remembered on the definition), then the
/// arguments, out of `budget`.
///
/// #7928: a body that settles only when a read of a bare parameter cannot raise after its output
/// (`try g catch 0`: jq's `try` runs the handler after an output its body raised behind) is asked
/// that question too, remembered apart, and then settles a call whose arguments cannot raise late.
fn call_settles(def: &FuncDefData, args: &[Expr], jq: bool, budget: &mut u32) -> bool {
    let body_settles = |quiet_params: bool| {
        def.call_settles_or_init(jq, quiet_params, |body| {
            let mut fresh = SETTLE_BUDGET;
            settles(
                body,
                &mut fresh,
                &Ctx {
                    own: Some(def),
                    quiet_params,
                    ..Ctx::wide(jq)
                },
            )
        })
    };
    if body_settles(false) {
        return args.iter().all(|arg| settles(arg, budget, &Ctx::wide(jq)));
    }
    body_settles(true)
        && args.iter().all(|arg| {
            settles(arg, budget, &Ctx::wide(jq)) && !may_raise_after_output(arg, &Ctx::wide(jq))
        })
}

/// The arm of [`settles`] for a bound call (wide question).
fn call_head_settles_in(expr: &Expr, jq: bool, budget: &mut u32) -> bool {
    match expr {
        Expr::DefCall { def, args, .. } => call_settles(def, args, jq, budget),
        _ => false,
    }
}

impl Ctx<'static> {
    /// The call-head question's context with nothing in scope.
    const fn wide(jq: bool) -> Self {
        Self {
            jq,
            locals: &[],
            own: None,
            wide: true,
            local_defs: &[],
            params: &[],
            quiet_params: false,
        }
    }
}

/// [`settles`] for `items` joined by `,` (#7787): the items in front of the one that yields
/// yield nothing (`empty`, an error, a `break`, or a pipe through one of those: they run before
/// any output, as they would on the streaming route), the one that yields settles, and the items
/// after it can only end the evaluation (`empty`, a `break`, an error with a constant message):
/// what runs after the output is nothing that can be observed in a different order, so
/// `label $f | 1, break $f` and `1, empty` settle and `1, 2` and `1, debug` do not.
fn comma_settles(items: &[Expr], budget: &mut u32, cx: &Ctx<'_>) -> bool {
    let mut rest = items.iter();
    for item in rest.by_ref() {
        if yields_nothing(item) {
            continue;
        }
        return settles(item, budget, cx) && rest.all(only_ends);
    }
    true
}

/// Whether `expr` never yields an output (#7787): an error, `empty`, a `break`, or a sequence or
/// pipe made of one.
fn yields_nothing(expr: &Expr) -> bool {
    match expr {
        Expr::Paren(inner) => yields_nothing(inner),
        Expr::Error(_) | Expr::Break(_) | Expr::Builtin(Builtin::Empty) => true,
        Expr::Comma(items) => items.iter().all(yields_nothing),
        Expr::Pipe(stages) => stages.iter().any(yields_nothing),
        _ => false,
    }
}

/// Whether `expr`, run after an output, only ends the evaluation and does nothing that can be
/// seen (#7787): `empty`, a `break`, or an error whose message is a constant or `.`.
fn only_ends(expr: &Expr) -> bool {
    match expr {
        Expr::Paren(inner) => only_ends(inner),
        Expr::Break(_) | Expr::Builtin(Builtin::Empty) => true,
        Expr::Error(msg) => msg.as_deref().map_or(true, |m| {
            matches!(
                m,
                Expr::Literal(_) | Expr::Identity | Expr::Var(_) | Expr::Field(_)
            )
        }),
        Expr::Comma(items) => items.iter().all(only_ends),
        _ => false,
    }
}

/// Whether `expr`, which [`settles`], may raise an error or a `break` after its one output (#7928):
/// a `,` with one after the item that yields (which `comma_settles` admits), anywhere in it, through
/// a def it calls (whose own recursive calls are taken not to, by induction) and the arguments it
/// passes. A read of a bare parameter, a call by name to anything but the definition being walked,
/// a call by namespace and a def declared inside are a yes, since the walk cannot see what they
/// run; so is running out of budget. Asked only of a `try` body whose handler yields, on each
/// evaluation of the `try`'s settle question: the walk of a def's body is remembered on the
/// definition ([`def_raises_late`]), so a `try` around a call costs the call and its arguments.
fn may_raise_after_output(expr: &Expr, cx: &Ctx<'_>) -> bool {
    let mut budget = CALL_ARGS_BUDGET;
    let own = cx
        .own
        .filter(|def| !shadowed_by_local(cx, &def.name, def.params.len()));
    // Under `quiet_params`, the bare parameters of `own` this position still reads: not one a def
    // declared around it binds or hides.
    let quiet: Vec<&str> = match own {
        Some(def) if cx.quiet_params => def
            .params
            .iter()
            .map(super::expr::Param::name)
            .filter(|name| !cx.params.contains(name) && !shadowed_by_local(cx, name, 0))
            .collect(),
        _ => Vec::new(),
    };
    raises_late(expr, own, &quiet, false, true, &mut Vec::new(), &mut budget)
}

/// Whether the body of `def` may raise an error or a `break` after an output (#7928), its own
/// recursive calls taken not to: [`raises_late`] over the body with `def` in scope, out of a budget
/// of its own, remembered on the definition. The answer is a property of the definition alone: once
/// `scope` holds a def, a call by name is read against it, never against the `own` definition.
fn def_raises_late(def: &Rc<FuncDefData>) -> bool {
    def.raises_late_or_init(|body| {
        let mut fresh = CALL_ARGS_BUDGET;
        raises_late(
            body,
            None,
            &[],
            false,
            false,
            &mut Vec::from([Rc::clone(def)]),
            &mut fresh,
        )
    })
}

/// [`raises_late`] over a closure argument (`remember`), remembered on the argument with the nodes
/// its walk visited (#7928): a recursion that passes `try g catch 0` down nests the argument it was
/// given one level deeper at every call, and each call's settle question asked it again, a walk of
/// the whole chain. A read charges `budget` what the walk cost, so every answer is the one the walk
/// would give; a walk that ran out of budget is not remembered.
fn shared_raises_late(arg: &super::expr::SharedArg, budget: &mut u32) -> bool {
    if let Some((late, cost)) = arg.raises_late_known() {
        return match budget.checked_sub(u32::from(cost)) {
            Some(left) => {
                *budget = left;
                late
            }
            None => {
                *budget = 0;
                true
            }
        };
    }
    let before = *budget;
    let late = raises_late(arg.expr(), None, &[], false, true, &mut Vec::new(), budget);
    if *budget > 0 {
        if let Ok(cost) = u8::try_from(before - *budget) {
            arg.remember_raises_late(late, cost);
        }
    }
    late
}

/// [`may_raise_after_output`]'s walk. `scope` holds the defs whose bodies are being walked, the
/// innermost last (then `own`, the definition being asked about, whose body is walked once, with
/// `inducting`): a call from a body to its own def is taken not to raise late, and a read of a bare
/// parameter of a def in `scope` reads an argument already walked. With `remember` (a walk not
/// itself filling a definition's answer, so `scope` is empty), a bound call reads its def's body
/// through [`def_raises_late`]; without it, the walk descends into the body, so filling one answer
/// never asks for another.
fn raises_late(
    expr: &Expr,
    own: Option<&FuncDefData>,
    quiet: &[&str],
    inducting: bool,
    remember: bool,
    scope: &mut Vec<Rc<FuncDefData>>,
    budget: &mut u32,
) -> bool {
    search_subexpr(expr, &mut |e: &Expr| {
        let Some(left) = budget.checked_sub(1) else {
            return Visit::Found;
        };
        *budget = left;
        match e {
            Expr::Comma(items) => {
                let mut after = items.iter().skip_while(|item| yields_nothing(item)).skip(1);
                if after.any(|item| !ends_quietly(item)) {
                    Visit::Found
                } else {
                    Visit::Descend
                }
            }
            // What stops at, or collects, everything before its one output; and a `try` with no
            // handler or a `?`, which stops at an error or a `break` its body raises (jq 1.7.1:
            // `[label $l | try (try (1, break $l)) catch 0]` is `[1]`).
            Expr::Array(_)
            | Expr::FirstExpr(_)
            | Expr::LastExpr(_)
            | Expr::Optional(_)
            | Expr::Try { catch: None, .. } => Visit::Skip,
            // `limit(1; f)` stops at `f`'s first output; a `reduce`'s source and update run before
            // its output, so only its initial value (once per value it yields) can raise after one.
            Expr::Limit { n, .. } if limit_settles(n) => Visit::Skip,
            Expr::Reduce { init, .. } => {
                if raises_late(init, own, quiet, inducting, remember, scope, budget) {
                    Visit::Found
                } else {
                    Visit::Skip
                }
            }
            Expr::NamespacedCall { .. } | Expr::FuncDef { .. } => Visit::Found,
            // An argument is caller-scope code: its names are not the ones in `scope`.
            Expr::Shared(arg) => {
                let late = if remember {
                    shared_raises_late(arg, budget)
                } else {
                    raises_late(arg.expr(), None, &[], false, false, &mut Vec::new(), budget)
                };
                if late {
                    Visit::Found
                } else {
                    Visit::Skip
                }
            }
            Expr::FuncCall { name, args, .. } => {
                let innermost = scope.last().cloned();
                let (def, walked_args) = match &innermost {
                    Some(def) => (Some(&**def), true),
                    None => (own, false),
                };
                let Some(def) = def else {
                    return Visit::Found;
                };
                if *name == def.name && args.len() == def.params.len() {
                    // A recursive call: by induction when its body is the one being walked; the
                    // definition being asked about has its body walked first.
                    if walked_args || inducting {
                        return Visit::Descend;
                    }
                    return if raises_late(&def.body, own, quiet, true, remember, scope, budget) {
                        Visit::Found
                    } else {
                        Visit::Descend
                    };
                }
                if walked_args
                    && args.is_empty()
                    && def.params.iter().any(|p| p.name() == name.as_str())
                {
                    return Visit::Descend;
                }
                // A bare parameter of the definition asked about, under `quiet_params`: every call
                // that reaches it has its arguments checked not to raise late (`call_settles`).
                if !walked_args && args.is_empty() && quiet.contains(&name.as_str()) {
                    return Visit::Descend;
                }
                Visit::Found
            }
            Expr::DefCall { def, args, .. } => {
                if args
                    .iter()
                    .any(|arg| raises_late(arg, own, quiet, inducting, remember, scope, budget))
                {
                    return Visit::Found;
                }
                if scope.iter().any(|walked| Rc::ptr_eq(walked, def)) {
                    return Visit::Skip;
                }
                let body_late = if remember && scope.is_empty() {
                    def_raises_late(def)
                } else {
                    scope.push(Rc::clone(def));
                    let late = raises_late(&def.body, own, &[], inducting, false, scope, budget);
                    scope.pop();
                    late
                };
                if body_late {
                    Visit::Found
                } else {
                    Visit::Skip
                }
            }
            _ => Visit::Descend,
        }
    })
}

/// Whether `expr` ends the evaluation without a value or a jump a `try` catches (#7928): `empty`.
fn ends_quietly(expr: &Expr) -> bool {
    match expr {
        Expr::Paren(inner) => ends_quietly(inner),
        Expr::Builtin(Builtin::Empty) => true,
        Expr::Comma(items) => items.iter().all(ends_quietly),
        _ => false,
    }
}

/// Whether a def declared in the body being walked hides the call `name/arity` (#7787): the
/// definition being asked about, or its bare parameter, is then not what the call reaches.
fn shadowed_by_local(cx: &Ctx<'_>, name: &str, arity: usize) -> bool {
    cx.local_defs
        .iter()
        .any(|local| local.name == name && local.arity == arity)
}

/// Whether `limit(n; ...)` answers at most once for a constant `n` of 0 or 1 (#7787), whatever
/// its source yields: it stops the source at its first output.
fn limit_settles(n: &Expr) -> bool {
    matches!(super::eval::constant_number(n), Some(c) if c == 0.0 || c == 1.0)
}

/// Whether a pipe's stages in front of its last are all [`stage_settles`], and
/// the evaluator would not take a different route for the pipe than running
/// them in order (`needs_path_context`).
pub(crate) fn pipe_settles_before_last(stages: &PipeStages, jq: bool) -> bool {
    pipe_settles_in(
        stages,
        &Ctx {
            jq,
            locals: &[],
            own: None,
            wide: false,
            local_defs: &[],
            params: &[],
            quiet_params: false,
        },
    )
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
                if !jq
                    || !settles_in(
                        source,
                        &Ctx {
                            jq,
                            locals,
                            own: None,
                            wide: false,
                            local_defs: &[],
                            params: &[],
                            quiet_params: false,
                        },
                    )
                {
                    break false;
                }
                expr = body;
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                if !settles_in(
                    cond,
                    &Ctx {
                        jq,
                        locals,
                        own: None,
                        wide: false,
                        local_defs: &[],
                        params: &[],
                        quiet_params: false,
                    },
                ) {
                    break false;
                }
                if reach(then_branch, def, jq, locals) {
                    break true;
                }
                expr = else_branch;
            }
            Expr::Pipe(stages) => match stages.last() {
                Some(last)
                    if pipe_settles_in(
                        stages,
                        &Ctx {
                            jq,
                            locals,
                            own: None,
                            wide: false,
                            local_defs: &[],
                            params: &[],
                            quiet_params: false,
                        },
                    ) =>
                {
                    expr = last;
                }
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

    /// #7460: a call at a pipe's head settles when its arguments do and its definition's body
    /// does, the definition's own calls and bare parameters taken to settle; asked again, the
    /// body's answer is read back.
    #[test]
    fn a_call_at_a_pipes_head_settles_when_its_body_does_7460() {
        for (def_src, want) in [
            (
                "def f($n): if $n == 0 then . + 1 else f($n - 1) | f($n - 1) end; f(3)",
                true,
            ),
            (
                "def f(n): if n == 0 then . + 1 else f(n - 1) | f(n - 1) end; f(3)",
                true,
            ),
            (
                "def f($m; $i): (if $i <= $m then .c += 1 else . end) \
                 | if $i > $m then . else f($m; $i * 2) | f($m; $i * 2 + 1) end; f(9; 1)",
                true,
            ),
            // What a level computes, bound with `as`.
            (
                "def f($n): if $n == 0 then . else ($n - 1) as $m | f($m) | f($m) end; f(3)",
                true,
            ),
            // A memoised answer stored with `setpath`.
            (
                "def f($n): if $n == 0 then . else f($n - 1) as $q | $q | setpath([$n]; 1) end; f(3)",
                true,
            ),
            ("def f($n): setpath([$n]; (1, 2)) | f($n - 1); f(3)", false),
            // A generator in the body, or in an argument, or a call with another arity.
            ("def f($n): .[] | f($n - 1); f(3)", false),
            (
                "def f($n): if $n == 0 then . else f($n - 1) end; f(1, 2)",
                false,
            ),
            ("def f($n): f($n; 1); f(3)", false),
            ("def f($n): ($n, 1) | f(0); f(3)", false),
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
            assert_eq!(call_head_settles(&call, true), want, "`{def_src}`");
            assert_eq!(
                call_head_settles(&call, true),
                want,
                "`{def_src}`, remembered"
            );
        }
        // Only a call: anything else at a pipe's head keeps its route.
        assert!(!call_head_settles(&parse(". + 1").expect("parses"), true));
    }

    /// #7795: `first(f)` settles unless `f` holds a `?//`, which a failure after the call retries.
    #[test]
    fn a_first_around_a_retrying_bind_does_not_settle_7795() {
        for (def_src, want) in [
            ("def f: first([1] as [$a] ?// $a | $a); f", false),
            ("def f: first([1] as [$a] | $a); f", true),
            // #7787: a `?//` anywhere keeps the expression streaming (the question's entry),
            // conservatively, though jq does not retry through `last`.
            ("def f: last([1] as [$a] ?// $a | $a); f", false),
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
            assert_eq!(answers_once(&call, true), want, "`{def_src}`");
        }
    }

    /// #7787: the shape test a site asks first: a call to a definition that may recurse on the
    /// spine; a call to one that cannot, or anything off the spine, answers no without a walk.
    #[test]
    fn only_a_call_that_may_recurse_on_the_spine_is_worth_settling_7787() {
        for (src, want) in [
            (
                "def t($n): if $n == 0 then 1 else t($n - 1) + t($n - 1) end; t(3)",
                true,
            ),
            (
                "def t($n): if $n == 0 then 1 else t($n - 1) end; (t(3) // 0)",
                true,
            ),
            (
                "def t($n): if $n == 0 then 1 else t($n - 1) end; first(t(3))",
                true,
            ),
            (
                "def t($n): if $n == 0 then 1 else t($n - 1) end; {a: t(3)}",
                true,
            ),
            (
                "def t($n): if $n == 0 then 1 else t($n - 1) end; t(3) + 1",
                false,
            ),
            ("def nm: .name; nm", false),
            ("def old: .age > 30; old", false),
            ("def k($x): $x * 2; k(.id)", false),
            ("def k(f): f; k(.id)", true),
            ("def g: 1; def h: g; h", false),
            ("def f: def g: f; g; f", true),
        ] {
            let mut program = parse(src).expect("parses");
            // Bind each def over what follows it, as the evaluator does, down to the last one.
            let mut defs = Vec::new();
            while let Expr::FuncDef {
                name,
                params,
                body,
                then,
                ..
            } = program
            {
                let body = defs
                    .iter()
                    .fold(*body, |e, d| install_def_calls(&e, d, 0, false));
                defs.push(Rc::new(FuncDefData::new(name, params, body)));
                program = *then;
            }
            let call = defs
                .iter()
                .fold(program, |e, d| install_def_calls(&e, d, 0, false));
            assert_eq!(may_hold_call(&call, SPINE_DEPTH), want, "`{src}`");
        }
        // Leaves and operators off the spine answer at once.
        for src in [".age > 30", ".score + .id", "$n - 1", ".a", "length"] {
            assert!(
                !may_hold_call(&parse(src).expect("parses"), SPINE_DEPTH),
                "`{src}`"
            );
        }
    }

    /// #7787: the wide question over every form, each by what it yields: answers at most once
    /// and leaves nothing observable to run after the output; a generator, an effect or a second
    /// output after it, a value-dependent count and a `?//` do not.
    #[test]
    fn the_wide_question_reads_every_form_by_what_it_yields_7787() {
        for (def_src, want) in [
            // Leaves the hunt found refused.
            ("def f: [1] | first; f", true),
            ("def f: [1] | last; f", true),
            ("def f: limit(1; 1, 2); f", true),
            ("def f: limit(0; 1, 2); f", true),
            ("def f: limit(2; 1, 2); f", false),
            ("def f: limit(2; 1); f", true),
            ("def f: foreach (1) as $x (0; $x); f", true),
            ("def f: foreach (1, 2) as $x (0; $x); f", false),
            ("def f: 0 | until(. >= 1; . + 1); f", true),
            ("def f: until(true; 1, 2); f", false),
            ("def f: (label $f | 1, break $f); f", true),
            ("def f: def k($a): $a; k(1); f", true),
            ("def f: def k(f): f; k(1); f", true),
            ("def f: def k(f): f; k(1, 2); f", false),
            ("def f: env | 1; f", true),
            ("def f: input_line_number + 1; f", true),
            ("def f: tojson | fromjson | 1; f", true),
            ("def f: [1] | IN(.[]; 1); f", true),
            ("def f: nth(0; 1, 2); f", true),
            ("def f: isempty(1, 2); f", true),
            ("def f: {a: 1, (\"b\"): 2} | .a; f", true),
            ("def f: \"\\(1)-\\(2)\"; f", true),
            ("def f: \"\\(1, 2)\"; f", false),
            ("def f: .[1:2] | .[0]; f", true),
            // A sequence: nothing before the output but what yields nothing, nothing after it but
            // an end.
            ("def f: 1, empty; f", true),
            ("def f: empty, 1; f", true),
            ("def f: error(\"x\"), 1; f", true),
            ("def f: 1, error(\"x\"); f", true),
            ("def f: 1, error(debug); f", false),
            ("def f: 1, 2; f", false),
            ("def f: 1, (2 | debug | empty); f", false),
            ("def f: 1, halt; f", false),
            // Effects before the output are where they would be anyway.
            ("def f: debug | input | . + 1; f", true),
            // Generators, and counts that depend on the value read.
            ("def f: .[]; f", false),
            ("def f: .. | 1; f", false),
            ("def f: range(1); f", false),
            ("def f: (1, 2) | select(. == 1); f", false),
            ("def f: [1] | .[] as $v | $v; f", false),
            ("def f: tostream; f", false),
            ("def f: splits(\"a\"); f", false),
            ("def f: repeat(1); f", false),
            // A destructuring bind: one pattern binds once; alternatives are retried by a later
            // failure, so they stream.
            ("def f: [1] as [$x] | $x; f", true),
            ("def f: [1] as [$x] ?// $x | $x; f", false),
            // A def declared in the body that recurses into the one asked about, and one that
            // hides it.
            (
                "def f($n): def g: f($n - 1); if $n == 0 then 1 else g + g end; f(3)",
                true,
            ),
            (
                "def f($n): def f($m): (1, 2); if $n == 0 then 1 else f($n - 1) end; f(3)",
                false,
            ),
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
            assert_eq!(answers_once(&call, true), want, "`{def_src}`");
        }
        // `first` around a `?//`: a failure after the call retries it, so it is not run first.
        let Expr::FuncDef {
            name,
            params,
            body,
            then,
            ..
        } = parse("def f: first([1] as [$a] ?// $a | $a); f").expect("parses")
        else {
            panic!("a def");
        };
        let def = Rc::new(FuncDefData::new(name, params, *body));
        crate::jq::eval::note_alternative_bind_parsed();
        assert!(!call_head_settles(
            &install_def_calls(&then, &def, 0, false),
            true
        ));
        // Only an expression that calls a def is run first; one that calls nothing answers once
        // all the same.
        assert!(!call_head_settles(&parse("1 + 2").expect("parses"), true));
        assert!(answers_once(&parse("1 + 2").expect("parses"), true));
        assert!(!answers_once(&parse(".[] + 2").expect("parses"), true));
    }

    /// #7713: the shapes a body answers at most once with beyond the tail loop's -- `try`, `?`,
    /// `label`, `reduce`, an object, `first`/`last`, a def declared in the body, a call with
    /// arguments -- and a pipe that ends in such a call; each with a generator in it does not.
    #[test]
    fn a_call_whose_body_uses_try_label_reduce_or_a_local_def_settles_7713() {
        for (def_src, want) in [
            ("def f: try 1; f", true),
            ("def f: try 1 catch 2; f", true),
            ("def f: try (1, 2); f", false),
            ("def f: try 1 catch (2, 3); f", false),
            // #7928: a handler that yields runs after an output its body raises behind (an error
            // or a `break`), so such a body does not settle under it; one that cannot raise late,
            // or a handler that yields nothing, still does.
            ("def f: try (1, error(\"x\")) catch 0; f", false),
            ("def f: label $l | try (1, break $l) catch 0; f", false),
            ("def f: try (1, error(\"x\")) catch empty; f", true),
            ("def f: try (1, error(\"x\")); f", true),
            ("def f: try first(1, error(\"x\")) catch 0; f", true),
            ("def f: try (error(\"x\"), 1) catch 0; f", true),
            ("def f: try ([1] | first) catch 0; f", true),
            // What stops at its first output or collects before it, and a `try` with no handler or
            // a `?` inside, cannot raise late; a `reduce`'s initial value can.
            ("def f: try reduce (1, error(\"x\")) as $x (0; $x) catch 0; f", true),
            ("def f: try reduce empty as $x ((1, error(\"x\")); .) catch 0; f", false),
            ("def f: try limit(1; 1, error(\"x\")) catch 0; f", true),
            ("def f: try (try (1, error(\"x\"))) catch 0; f", true),
            ("def f: try ((1, error(\"x\"))?) catch 0; f", true),
            ("def f: def g: 1, error(\"x\"); try g catch 0; f", false),
            (
                "def f($n): if $n == 0 then (1, error(\"x\")) else try f($n - 1) catch 0 end; f(3)",
                false,
            ),
            (
                "def f($n): if $n == 0 then 1 else try f($n - 1) catch 0 end; f(3)",
                true,
            ),
            (
                "def f(g; $n): if $n > 0 then f(try (g + (1, error(\"x\"))) catch 0; $n - 1) + f(g; $n - 1) else g end; f(null; 4)",
                false,
            ),
            (
                "def f(g; $n): if $n > 0 then f(try g catch 0; $n - 1) else g end; f((1, error(\"x\")); 3)",
                false,
            ),
            // A read of a bare parameter under such a `try` settles once the call's arguments, and
            // those of the recursive calls, cannot raise late either.
            (
                "def f(g; $n): if $n > 0 then f(try g catch 0; $n - 1) + f(g; $n - 1) else g end; f(1; 4)",
                true,
            ),
            (
                "def f(g; $n): if $n > 0 then f(g; $n - 1) + f(try g catch 0; $n - 1) else g end; f(1; 4)",
                true,
            ),
            (
                "def f(g; $n): if $n > 0 then f(try g catch 0; $n - 1) else g end; f(1; 3)",
                true,
            ),
            (
                "def f(g; $n): if $n > 0 then f(try g catch 0; $n - 1) + f(g; $n - 1) else g end; f((1, error(\"x\")); 4)",
                false,
            ),
            // A recursive call passing an argument that raises late, and a local def's parameter
            // of the same name, keep it from settling.
            (
                "def f(g; $n): if $n > 0 then f(try g catch 0; $n - 1) + f((g, error(\"x\")); $n - 1) else g end; f(1; 4)",
                false,
            ),
            (
                "def f(g; $n): def h(g): try g catch 0; if $n > 0 then h(1, error(\"x\")) + f(g; $n - 1) else g end; f(1; 3)",
                false,
            ),
            (
                "def f(g; $n): def g: 1, error(\"x\"); if $n > 0 then (try g catch 0) + f(g; $n - 1) else 1 end; f(1; 3)",
                false,
            ),
            ("def f: 1?; f", true),
            ("def f: label $l | 1; f", true),
            ("def f: label $l | (1, 2); f", false),
            ("def f: reduce .[] as $x (0; . + $x); f", true),
            ("def f: reduce .[] as $x (0, 1; . + $x); f", false),
            ("def f: {a: 1, (\"b\"): .c}; f", true),
            ("def f: {a: (1, 2)}; f", false),
            ("def f: first(.[]); f", true),
            ("def f: last(.[]); f", true),
            ("def f: def h: 1; h + h; f", true),
            ("def f: def h: .[]; h; f", false),
            ("def f: def h: 1; def h: .[]; h; f", false),
            // #7787: a local def with parameters settles when its body does.
            ("def f: def h($x): $x; h(1); f", true),
            ("def f: .[0] | f; .[1] | f", true),
            ("def f: .[0] | f; .[] | f", false),
            // #7787: any expression that calls a def and answers once, not only a pipe ending in
            // the call.
            ("def f: .[0] | f; f | .a", true),
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
            assert_eq!(answers_once(&call, true), want, "`{def_src}`");
        }
        // A call with arguments to another def is asked the same question.
        let Expr::FuncDef {
            name,
            params,
            body,
            then,
            ..
        } = parse("def g($x): try $x; def f: g(1) + g(2); f").expect("parses")
        else {
            panic!("a def");
        };
        let g = Rc::new(FuncDefData::new(name, params, *body));
        let Expr::FuncDef {
            name,
            params,
            body,
            then,
            ..
        } = install_def_calls(&then, &g, 0, false)
        else {
            panic!("an inner def");
        };
        let f = Rc::new(FuncDefData::new(name, params, *body));
        assert!(answers_once(&install_def_calls(&then, &f, 0, false), true));
        // The tail loop's own question reads a def declared in the body too (a module's
        // forwarding stub), and none of the rest.
        assert!(stage_settles(
            &parse("def h: . + 1; h").expect("parses"),
            true
        ));
        assert!(!stage_settles(&parse("try 1").expect("parses"), true));
    }

    /// #7928: a `try` with a handler that yields, around a bound call, reads whether the call's body
    /// may raise after its output from the definition (`def_raises_late`), which asks once; the
    /// second ask reads the remembered answer and agrees with the first.
    #[test]
    fn a_try_around_a_bound_call_reads_the_remembered_late_raise_7928() {
        for (src, want) in [
            ("def g: 1, error(\"x\"); try (g + g) catch 0", false),
            ("def g: 1; try (g + g) catch 0", true),
            ("def g: 1, error(\"x\"); try (g + g) catch empty", true),
            (
                "def g($n): if $n == 0 then (1, error(\"x\")) else g($n - 1) end; try g(3) catch 0",
                false,
            ),
            (
                "def g($n): if $n == 0 then 1 else g($n - 1) end; try g(3) catch 0",
                true,
            ),
            // The argument is walked at the call, outside the remembered answer.
            ("def g(h): h; try g(1, error(\"x\")) catch 0", false),
            ("def g(h): h; try g(1) catch 0", true),
        ] {
            let Expr::FuncDef {
                name,
                params,
                body,
                then,
                ..
            } = parse(src).expect("parses")
            else {
                panic!("a def");
            };
            let def = Rc::new(FuncDefData::new(name, params, *body));
            let expr = install_def_calls(&then, &def, 0, false);
            assert_eq!(answers_once(&expr, true), want, "`{src}`");
            assert_eq!(answers_once(&expr, true), want, "`{src}`, asked again");
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
