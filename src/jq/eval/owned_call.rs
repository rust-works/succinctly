//! A call to a `def`, a pipe a call leads, an `if` and a `$`-parameter's binding over an owned value,
//! run on the owned tree with the value handed down (#7483).
//!
//! The owned evaluator ([`eval_each_owned`]) answers a stage from the tree when one of its doors
//! can, and otherwise writes the whole value out as JSON, indexes it and runs the stage on that
//! document (the re-index bridge). No door took a call to a `def`, so a recursion that threads a
//! state through its calls bridged the state at every call: `def f($n): if $n == 0 then .a = 1
//! else f($n - 1) | f($n - 1) | .a += 1 end; {x: [range(k)]} | f(12)` wrote `x` out at each of its
//! 8,191 calls, where jq 1.7.1 never copies it. The cost grew with the size of the members the
//! program never touches.
//!
//! The door runs these nodes as the document evaluator runs them, but over the owned value:
//!
//! - **a call** binds its body as [`each_def_call`] does (the same [`bind_def_call`], the same
//!   [`enter_def_call`] guard, a body that ends in a call jq jumps to looping in place through
//!   [`each_def_tail_loop`]) and runs the body over the value;
//! - **a definition** installs itself over what follows it, as `eval_each`'s arm does;
//! - **a pipe led by an assignment** writes in place and runs the rest of the pipe over the answer;
//! - **a pipe led by a call that settles**
//!   ([`call_head_settles`](crate::jq::tail_call::call_head_settles)) runs the call to its outputs
//!   and then the rest of the pipe over each, as [`eval_each_pipe`]'s settled route does, the rest
//!   through the same [`RestPipe`] that route's driver builds;
//! - **an `if`** whose condition the tree answers ([`owned_cond_verdict`]) runs the branch it picks;
//! - **an `as` whose source never reads `.`**, the binding of a `$` parameter, binds its one value
//!   as [`each_as`] binds a value with no node behind it.
//!
//! Each runs its parts through [`eval_moved`], the owned evaluator for a value nothing else holds:
//! the value is handed down rather than borrowed, so the assignment it reaches
//! ([`owned_assign_moved`]) writes into the value's own storage instead of a copy of every
//! container on its path, as jq's `_modify` writes into an unshared input. The tail loop hands the
//! value it built to the stages ahead of its call the same way ([`tail_one_moved`]): the second
//! reference that made every `.p[k] = 1` of `def f: if .i >= n then . else (.p["k\(.i)"] = 1 | .i
//! += 1 | f) end` copy the growing `p` was the loop's own `cur`, which the stage's answer replaces.
//!
//! Everything else is the owned evaluator's own route, bridge included, so a part that bridged
//! before bridges now, over the value it would have been handed. What changes is only that the
//! nodes above no longer bridge the value they share.
//!
//! The door declines, with the value handed back and nothing evaluated, wherever the document route
//! does something these do not model: yq mode; while the #2889 embed table is live (an embedded
//! node's identity is the bridge's to read); in a program with a `?//` bind (its retries cross pipe
//! drivers); for a pipe that needs path context; for a node whose `Snapshot` markers a re-entry
//! would rewrite, or a body that holds such a marker; for a condition the tree cannot answer, or one
//! under `?`; and for an `as` source that reads `.`. The document route then answers, as before.

use super::*;

/// The door for an owned re-entry that holds `input` only by reference: the value is shared with
/// the caller, so the door starts on a handle of its own (a reference count, not a copy) and the
/// first write copies only the containers on its path. `None` when the door declines.
///
/// Out of line and cold: [`eval_each_owned_past_front_doors_in`] calls it on every program that
/// reaches the bridge, and neither its locals nor its code may move that function's hot path (an
/// Ir-flat 2-3.5% on per-record rows of a 7950X was the layout cost of a first draft).
#[cold]
#[inline(never)]
pub(super) fn door_borrowed<S: EvalSemantics>(
    expr: &Expr,
    input: &OwnedValue,
    optional: bool,
    reentry: Reentry,
    sink: &mut dyn FnMut(OwnedValue) -> Demand,
) -> Option<Flow> {
    if !entry_reentry(reentry) || !door_shape(expr) || !door_open::<S>(expr, false) {
        return None;
    }
    // An `if` is decided on the borrowed value: a condition the tree cannot answer declines with
    // nothing taken, not even a handle.
    if let Expr::If {
        cond,
        then_branch,
        else_branch,
    } = unwrap_paren(expr)
    {
        let branch = verdict_branch::<S>(cond, then_branch, else_branch, input, optional)?;
        return Some(branch_flow::<S>(branch, input.clone(), optional, sink));
    }
    door::<S>(expr, input.clone(), optional, sink).ok()
}

/// Whether `expr`'s node is one [`door_shape`] can take at all, on its discriminant alone: the check
/// bridges (`owned_bridge`, `eval_owned_input_bridge`) and the fold steps make inline before they call the cold door, so the
/// programs that reach the bridge with another node (an empty rest, `Pipe([])`, once per element
/// of `map(length) | .[]`) pay a compare, not a call (64 instructions per element before).
#[inline(always)]
pub(super) fn door_candidate(expr: &Expr) -> bool {
    match unwrap_paren(expr) {
        Expr::DefCall { .. } | Expr::FuncDef { .. } | Expr::If { .. } | Expr::As { .. } => true,
        // A pipe the door can take is led by a call or an assignment, or is one such stage: the
        // first stage's discriminant says so, so a pipe of other stages (the input-free operand of
        // #7451's fold step, `$t | tostring | split("")`, evaluated once per step) pays a compare
        // and no call (#7702: 160 instructions per step through the door's own shape check).
        Expr::Pipe(stages) => match stages.as_slice() {
            [] => false,
            [only] => matches!(
                unwrap_paren(only),
                Expr::DefCall { .. } | Expr::FuncDef { .. } | Expr::If { .. } | Expr::As { .. }
            ),
            [first, ..] => matches!(
                unwrap_paren(first),
                Expr::DefCall { .. }
                    | Expr::Assign { .. }
                    | Expr::Update { .. }
                    | Expr::CompoundAssign { .. }
                    | Expr::AlternativeAssign { .. }
                    | Expr::Alternative(..)
                    | Expr::As { .. }
            ),
        },
        _ => false,
    }
}

/// The door for a fold or loop step whose UPDATE no by-value step answered (#7702): the state is
/// the loop's own, replaced by what the step answers, so it is handed down rather than borrowed and
/// a write in a call's body lands in place. `Err(state)`, untouched, when the door declines: the
/// caller then takes its usual route, the re-index bridge for a call, over the same state.
///
/// `reentry` is the step's: [`Reentry::Proven`] for an UPDATE the fold demoted once for the whole
/// run, [`Reentry::REBUILT`] otherwise. Both are taken, as [`door_borrowed`] takes them: under the
/// gates of [`door_open`] a re-entry's reroot changes nothing in `expr`, so whichever the bridge
/// would have done, its parts see the expression as written.
#[cold]
#[inline(never)]
pub(super) fn door_moved<S: EvalSemantics>(
    expr: &Expr,
    state: OwnedValue,
    optional: bool,
    reentry: Reentry,
    sink: &mut dyn FnMut(OwnedValue) -> Demand,
) -> Result<Flow, OwnedValue> {
    if !entry_reentry(reentry) || !door_shape(expr) || !door_open::<S>(expr, false) {
        return Err(state);
    }
    door::<S>(expr, state, optional, sink)
}

/// The re-entries the door is taken under: [`Reentry::REBUILT`] (no node backs the value, the
/// embed table having proved none) and [`Reentry::Proven`] (the markers were rerooted upstream, a
/// fold's or a loop's UPDATE, #7702). A value an embed-table entry witnesses is the bridge's.
fn entry_reentry(reentry: Reentry) -> bool {
    reentry == Reentry::REBUILT || reentry == Reentry::Proven
}

/// The owned evaluator for a value nothing else holds: [`eval_each_owned`] over `input`, except
/// that an assignment writes into `input` itself and that the nodes [`door_shape`] admits take this
/// module's door with `input` handed down. The front doors come first, as in
/// [`eval_each_owned`]; none of them takes an assignment, a call or an `if` it does not answer
/// whole.
pub(super) fn eval_moved<S: EvalSemantics>(
    expr: &Expr,
    input: OwnedValue,
    optional: bool,
    sink: &mut dyn FnMut(OwnedValue) -> Demand,
) -> Flow {
    eval_moved_in::<S>(expr, input, optional, false, sink)
}

/// [`eval_moved`] for an `expr` whose markers may already be known to be none a re-entry would
/// rewrite (`checked`): a part of a node the door took, which [`door_open`] walked whole. Walking
/// each part again cost a walk of the body per level of a recursion, about a tenth of the time of
/// `def f($n): if $n == 0 then . + 1 else f($n - 1) | f($n - 1) end; 0 | f(16)`.
fn eval_moved_in<S: EvalSemantics>(
    expr: &Expr,
    input: OwnedValue,
    optional: bool,
    checked: bool,
    sink: &mut dyn FnMut(OwnedValue) -> Demand,
) -> Flow {
    if let Some(flow) = eval_each_owned_front_doors::<S>(expr, &input, optional, sink) {
        return flow;
    }
    moved_past_front_doors::<S>(expr, input, optional, checked, sink)
}

/// [`eval_moved_in`] once the front doors have declined `expr`.
#[inline(never)]
fn moved_past_front_doors<S: EvalSemantics>(
    expr: &Expr,
    input: OwnedValue,
    optional: bool,
    checked: bool,
    sink: &mut dyn FnMut(OwnedValue) -> Demand,
) -> Flow {
    // #7260's in-place answer: no door between the front doors and `eval_owned_reindex_free`
    // takes an assignment, so this is the answer `eval_each_owned` gives, written in place.
    let input = match owned_assign_moved::<S>(expr, input) {
        OwnedStep::Handled(Ok(value)) => return emit_one(value, sink),
        OwnedStep::Handled(Err(_)) if optional => return Flow::Exhausted,
        OwnedStep::Handled(Err(error)) => return Flow::Escaped(Control::Error(error)),
        OwnedStep::Declined(input) => input,
    };
    let input = if door_shape(expr) && door_open::<S>(expr, checked) {
        match door::<S>(expr, input, optional, sink) {
            Ok(flow) => return flow,
            Err(input) => input,
        }
    } else {
        input
    };
    eval_each_owned_past_front_doors::<S>(expr, &input, optional, Reentry::REBUILT, sink)
}

fn emit_one(value: OwnedValue, sink: &mut dyn FnMut(OwnedValue) -> Demand) -> Flow {
    match sink(value) {
        Demand::Continue => Flow::Exhausted,
        Demand::Stop => Flow::Stopped { pending: None },
    }
}

/// Whether `expr` is a node the door may take, on its shape alone: a call, a definition, a pipe of two or more
/// stages that a call settling to its value or an assignment leads, an `if`, an `as` whose source never reads `.`
/// (a `$`-parameter's binding, `Shared(ARG) as $n | BODY`), or a pipe of one such stage (the copy a
/// [`RestPipe`] makes of a lone stage that holds evaluation state).
fn door_shape(expr: &Expr) -> bool {
    match unwrap_paren(expr) {
        Expr::DefCall { .. } | Expr::FuncDef { .. } | Expr::If { .. } => true,
        Expr::As { expr: source, .. } => closed_expr_shape(peel_shared(source)),
        Expr::Pipe(stages) => match stages.as_slice() {
            [only] => door_shape(only),
            [first, _, ..] => {
                crate::jq::tail_call::call_head_settles(first, true)
                    || is_owned_assign(unwrap_paren(first))
            }
            [] => false,
        },
        _ => false,
    }
}

/// The gates the door takes `expr` under (see the module's doc comment): jq mode, no embed table,
/// no `?//` bind in the program, and no marker a re-entry would rewrite, which a `checked` part of
/// a node already walked is known not to hold.
fn door_open<S: EvalSemantics>(expr: &Expr, checked: bool) -> bool {
    #[cfg(test)]
    if tests::closed() {
        return false;
    }
    S::TAG == EvalTag::Jq
        && !crate::jq::eval_generic::embed_table_active()
        && !contains_retrying_pattern_bind(expr)
        && (checked || markers_unread() || unrewritten::<S>(expr))
}

/// Whether no resolver of this program can read what a re-entry's marker rewrite changes, so the
/// walk for such markers can be skipped: the program reads no variable, and calls nothing, in path
/// position (`AnchorScope::Off`, decided once per program by the entry points' anchor scope,
/// #3134). A rewritten marker is read only by a resolver certifying it there
/// ([`reentry_can_observe`]), so in such a program the bridge's reroot changes no answer. A scope
/// no entry point decided (a library caller, a unit test) walks. This is what makes the door's
/// per-call check a load rather than a walk of the body (+5% instructions per call on
/// `0 | f(16)` with the walks).
fn markers_unread() -> bool {
    !crate::jq::eval_generic::scalar_identity_readable()
}

/// Whether a re-entry would leave `expr` as it is ([`Reentry::reroot`]): no marker it would rewrite.
fn unrewritten<S: EvalSemantics>(expr: &Expr) -> bool {
    matches!(Reentry::REBUILT.reroot::<S>(expr), Cow::Borrowed(_))
}

/// Run a [`door_shape`] node over `input`, or hand `input` back untouched when a part of it
/// declines before anything was evaluated.
fn door<S: EvalSemantics>(
    expr: &Expr,
    input: OwnedValue,
    optional: bool,
    sink: &mut dyn FnMut(OwnedValue) -> Demand,
) -> Result<Flow, OwnedValue> {
    #[cfg(test)]
    tests::entered();
    match unwrap_paren(expr) {
        Expr::DefCall {
            def,
            args,
            frames,
            bound,
        } => call::<S>(def, args, *frames, bound, input, optional, sink),
        // As `eval_each`'s own arm: the definition installed over what follows it, which runs over
        // the same value.
        Expr::FuncDef {
            name,
            params,
            body,
            then,
            bound,
            // Installing the definition adds calls, not markers, so what follows stays checked.
        } => Ok(eval_moved_in::<S>(
            &bind_def(name, params, body, then, bound),
            input,
            optional,
            true,
            sink,
        )),
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => if_branch::<S>(cond, then_branch, else_branch, input, optional, sink),
        Expr::As {
            expr: source,
            var,
            body,
        } => closed_bind::<S>(source, var, body, input, optional, sink),
        Expr::Pipe(stages) => match stages.as_slice() {
            [only] => door::<S>(only, input, optional, sink),
            [first, _, ..] if !pipe_needs_path_context(stages) => {
                if is_owned_assign(unwrap_paren(first)) {
                    assign_pipe::<S>(RestTail::whole(stages), input, optional, sink)
                } else {
                    call_pipe::<S>(RestTail::whole(stages), input, optional, sink)
                }
            }
            _ => Err(input),
        },
        _ => Err(input),
    }
}

/// [`each_def_call`] over an owned `input` (#7483). The body is probed before it is bound, so a
/// body this route declines (a marker a re-entry would rewrite) leaves the node's cache as a
/// static look leaves it (`BoundBody::probe`), and the document route's bind that follows is the
/// node's first reach, as it would have been.
#[inline(never)]
fn call<S: EvalSemantics>(
    def: &Rc<FuncDefData>,
    args: &[Expr],
    frames: u32,
    bound: &BoundBody,
    input: OwnedValue,
    optional: bool,
    sink: &mut dyn FnMut(OwnedValue) -> Demand,
) -> Result<Flow, OwnedValue> {
    // The marker check needs the body; a program whose markers no resolver reads skips both.
    if !markers_unread() {
        match probe_def_call(def, args, frames, bound) {
            Ok(body) if unrewritten::<S>(&body) => {}
            Ok(_) => return Err(input),
            // A bind that fails is the call's error, which the document route raises the same way.
            Err(error) => return Ok(Flow::Escaped(Control::Error(error))),
        }
    }
    let body = match bind_def_call(def, args, frames, bound) {
        Ok(body) => body,
        Err(error) => return Ok(Flow::Escaped(Control::Error(error))), // patchcov: coverage tolerate-line reason="unreachable while the probe above succeeded: binding is a pure function of the node, and the probe left the body in its cache (#7483)"
    };
    if tail_reachable(def, true) {
        return Ok(each_def_tail_loop::<Vec<u64>, S>(
            def,
            frames,
            bound,
            body,
            TailValue::Owned(input),
            optional,
            &mut |item| sink(owned_item::<S>(item)),
        ));
    }
    Ok(match enter_def_call(def, frames, bound) {
        Ok(_guard) => eval_moved_in::<S>(&body, input, optional, true, sink),
        Err(error) => Flow::Escaped(Control::Error(error)),
    })
}

/// An output of a tail loop that started on an owned value. Every value such a loop hands on is
/// one it built (a stage over a built value builds), so the borrowed arm is never taken; it decodes
/// as the bridge decodes its own document's values.
fn owned_item<S: EvalSemantics>(item: Item<'_, Vec<u64>>) -> OwnedValue {
    match item {
        Item::Owned(value) => value,
        Item::Borrowed(value) => to_owned_lossy_shared::<S, _>(&value), // patchcov: coverage tolerate-line reason="unreachable: a tail loop over an owned value hands on only values it built (#7483)"
    }
}

/// [`each_if`] over an owned `input`, for a condition [`owned_cond_verdict`] answers: one verdict,
/// so the branch it picks runs once, over the value handed down. A condition it cannot answer, or
/// one that raises, declines, so the error and its text stay the document route's.
fn if_branch<S: EvalSemantics>(
    cond: &Expr,
    then_branch: &Expr,
    else_branch: &Expr,
    input: OwnedValue,
    optional: bool,
    sink: &mut dyn FnMut(OwnedValue) -> Demand,
) -> Result<Flow, OwnedValue> {
    match verdict_branch::<S>(cond, then_branch, else_branch, &input, optional) {
        Some(branch) => Ok(branch_flow::<S>(branch, input, optional, sink)),
        None => Err(input),
    }
}

/// The branch [`owned_cond_verdict`] picks, or `None` when it cannot answer or the condition raises.
fn verdict_branch<'e, S: EvalSemantics>(
    cond: &Expr,
    then_branch: &'e Expr,
    else_branch: &'e Expr,
    input: &OwnedValue,
    optional: bool,
) -> Option<&'e Expr> {
    match owned_cond_verdict::<S>(cond, input, optional) {
        Some(Ok(true)) => Some(then_branch),
        Some(Ok(false)) => Some(else_branch),
        Some(Err(_)) | None => None,
    }
}

/// The picked branch over `input`, handed down. As `each_if`: a stop in the branch stops the
/// condition's drive, which ends stopped with nothing pending.
fn branch_flow<S: EvalSemantics>(
    branch: &Expr,
    input: OwnedValue,
    optional: bool,
    sink: &mut dyn FnMut(OwnedValue) -> Demand,
) -> Flow {
    match eval_moved_in::<S>(branch, input, optional, true, sink) {
        Flow::Stopped { .. } => Flow::Stopped { pending: None },
        other => other,
    }
}

/// [`each_as`] over an owned `input`, for a source that never reads `.`: its one value, which
/// [`eval_owned_closed`] answers with nothing evaluated over `input`, bound as `each_as` binds a
/// value with no node behind it (no embed-table entry, no origin), then the body over the value
/// handed down. This is a `$`-parameter's binding (`Shared(ARG) as $n | BODY`, jq's own `ARG as
/// $n`), so a recursion that counts down `$n` runs each level's body on the tree. A source the
/// closed grammar declines (it errors, fans out or reads `.`) hands `input` back.
fn closed_bind<S: EvalSemantics>(
    source: &Expr,
    var: &str,
    body: &Expr,
    input: OwnedValue,
    optional: bool,
    sink: &mut dyn FnMut(OwnedValue) -> Demand,
) -> Result<Flow, OwnedValue> {
    let Some(bound) = eval_owned_closed::<S>(peel_shared(source)) else {
        return Err(input);
    };
    // #7471, as `each_as`: a body that does not read `$var` is evaluated as it is.
    if !as_body_reads_var(body, var).reads {
        return Ok(eval_moved_in::<S>(body, input, optional, true, sink));
    }
    // A scalar is spliced in as a literal; a container rides a marker of its own, so the body is
    // walked again.
    let scalar = !matches!(bound, OwnedValue::Array(_) | OwnedValue::Object(_));
    let body = substitute_bound_var_from::<S>(source, body, var, &bound, None);
    Ok(eval_moved_in::<S>(&body, input, optional, scalar, sink))
}

/// `expr` with the `Expr::Shared` wrappers of a captured argument taken off: evaluation is
/// transparent through one (ADR-0020), so its value is the value of what it wraps.
fn peel_shared(mut expr: &Expr) -> &Expr {
    while let Expr::Shared(arg) = expr {
        expr = arg.expr();
    }
    expr
}

/// [`eval_each_pipe`]'s settled route over an owned `input`, for a pipe of two or more stages a
/// call leads: the call runs to its outputs before the rest of the pipe runs on any (#7460), and
/// the rest runs over each, handed down, through the [`RestPipe`] the document route's driver
/// builds. A call that declines hands `input` back, nothing evaluated.
#[inline(never)]
fn call_pipe<S: EvalSemantics>(
    tail: RestTail<'_>,
    input: OwnedValue,
    optional: bool,
    sink: &mut dyn FnMut(OwnedValue) -> Demand,
) -> Result<Flow, OwnedValue> {
    let first = &tail.stages()[0];
    let Expr::DefCall {
        def,
        args,
        frames,
        bound,
    } = unwrap_paren(first)
    else {
        return Err(input); // patchcov: coverage tolerate-line reason="unreachable: door_shape admits a pipe only when call_head_settles, which takes only a call (#7483)"
    };
    let mut outputs: Vec<OwnedValue> = Vec::new();
    let upstream = call::<S>(def, args, *frames, bound, input, optional, &mut |value| {
        outputs.push(value);
        Demand::Continue
    })?;
    let mut downstream: Option<Flow> = None;
    let mut stopped_at = pipe_retry_generation();
    let mut rest = RestPipe::of_tail(tail.rest());
    for value in outputs {
        match rest.run_moved::<S>(value, optional, true, sink) {
            Flow::Exhausted => {}
            other => {
                stop_with_downstream(&mut downstream, other);
                stopped_at = pipe_retry_generation();
                break;
            }
        }
    }
    // `settle_then_replay`: a replay the rest stopped ends the drive stopped, nothing pending.
    let upstream = if downstream.is_some() {
        Flow::Stopped { pending: None }
    } else {
        upstream
    };
    Ok(pipe_terminal_after_retry(
        upstream,
        downstream,
        stopped_at,
        direct_pattern_retry(first),
    ))
}

/// [`eval_each_pipe`] over an owned `input`, for a pipe of two or more stages an assignment leads:
/// the assignment writes into `input` ([`owned_assign_moved`]) and the rest of the pipe runs over
/// its one answer, handed down, through the [`RestPipe`] that route's driver builds. Before, the
/// rest re-entered the owned evaluator borrowed, which bridged the whole value again at each stage
/// of `.a = 1 | .b = 2 | ...`. An assignment the in-place step does not take hands `input` back.
fn assign_pipe<S: EvalSemantics>(
    tail: RestTail<'_>,
    input: OwnedValue,
    optional: bool,
    sink: &mut dyn FnMut(OwnedValue) -> Demand,
) -> Result<Flow, OwnedValue> {
    let first = unwrap_paren(&tail.stages()[0]);
    let value = match owned_assign_moved::<S>(first, input) {
        OwnedStep::Handled(Ok(value)) => value,
        OwnedStep::Handled(Err(_)) if optional => return Ok(Flow::Exhausted),
        OwnedStep::Handled(Err(error)) => return Ok(Flow::Escaped(Control::Error(error))),
        OwnedStep::Declined(input) => return Err(input),
    };
    // The driver's: the rest's verdict is the pipe's, an escape marked as it stops the drive.
    let flow = RestPipe::of_tail(tail.rest()).run_moved::<S>(value, optional, true, sink);
    if let Flow::Escaped(control) = &flow {
        mark_nonretryable_escape(control);
    }
    Ok(flow)
}

impl RestPipe<'_> {
    /// [`Self::run_owned`] for a value nothing else holds: the same doors and the same copy of the
    /// rest, then [`eval_moved`]'s in-place assignment and door with the value handed down.
    /// `checked` is [`eval_moved_in`]'s: these stages are a part of a pipe the door walked (a copy
    /// of them adds no marker).
    pub(super) fn run_moved<S: EvalSemantics>(
        &mut self,
        input: OwnedValue,
        optional: bool,
        checked: bool,
        sink: &mut dyn FnMut(OwnedValue) -> Demand,
    ) -> Flow {
        let lone = lone_plain_stage(self.stages);
        if let Some(only) = lone {
            if let Some(flow) = eval_each_owned_front_doors::<S>(only, &input, optional, sink) {
                return flow;
            }
        }
        // The nodes `run_owned_past_front_doors` evaluates, resolved as it resolves them: which
        // nodes a rest evaluates decides what its `def` calls retain (#3148, #3692). Spelled out
        // again rather than shared, so that function's code, on the hot path of every owned rest,
        // stays as it was.
        if let Some(only) = lone {
            if matches!(self.copy, RestCopy::Unasked) && lone_stage_can_go_bare(only) {
                self.copy = RestCopy::Bare;
            }
            if matches!(self.copy, RestCopy::Bare) {
                return moved_past_front_doors::<S>(only, input, optional, checked, sink);
            }
        }
        let kept;
        let whole = match lone.is_none().then(|| self.tail().plan::<S>()) {
            Some(TailPlan::Whole(copy)) => {
                kept = copy;
                kept.expr()
            }
            Some(TailPlan::Split) => {
                let tail = self.tail();
                self.copy.split_pipe(|| tail.split())
            }
            Some(TailPlan::Copy) | None => self.copy.owned_pipe::<S>(self.stages, self.owner),
        };
        if lone.is_some() {
            moved_past_front_doors::<S>(whole, input, optional, checked, sink)
        } else {
            eval_moved_in::<S>(whole, input, optional, checked, sink)
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::jq::share_stats;
    use core::cell::Cell;

    thread_local! {
        static CLOSED: Cell<bool> = const { Cell::new(false) };
        static ENTERED: Cell<usize> = const { Cell::new(0) };
    }

    /// Whether a test closed the door, to compare its answers with the route it replaces.
    pub(super) fn closed() -> bool {
        CLOSED.with(Cell::get)
    }

    /// Count an entry of the door.
    pub(super) fn entered() {
        ENTERED.with(|n| n.set(n.get() + 1));
    }

    fn entries() -> usize {
        ENTERED.with(Cell::get)
    }

    /// Every output of `program` over `input` through the owned evaluator, then how it ended.
    fn run(program: &str, input: &OwnedValue, door: bool) -> (Vec<String>, String) {
        let expr = crate::jq::parse(program).unwrap();
        CLOSED.with(|c| c.set(!door));
        let mut out = Vec::new();
        let flow =
            eval_each_owned::<JqSemantics>(&expr, input, false, Reentry::REBUILT, &mut |v| {
                out.push(v.to_json());
                Demand::Continue
            });
        CLOSED.with(|c| c.set(false));
        let end = match flow {
            Flow::Exhausted => "exhausted".to_string(),
            Flow::Stopped { pending } => format!("stopped, pending {pending:?}"),
            Flow::Escaped(Control::Error(error)) => format!("error: {error}"),
            Flow::Escaped(control) => format!("escaped: {control:?}"),
        };
        (out, end)
    }

    /// #7483: a recursion that threads a state through its calls writes into the state where it
    /// lies. The state carries a 100 kB string the program never touches: its allocation is the one
    /// the answer holds, no copy-on-write copied anything, and nothing was written out and indexed
    /// again. Before the door every call bridged the state (the first row bridged it at each of its
    /// 127 calls), and the tail loop's `cur` held a second reference to the state each stage
    /// wrote (the second row copied the growing `p` at every round).
    #[test]
    #[allow(clippy::literal_string_with_formatting_args)] // STYLE-0004: jq object literals, not format strings
    fn def_call_recursion_writes_the_state_in_place_7483() {
        let big = "x".repeat(100_000);
        let state = || {
            OwnedValue::object_from([
                ("x".to_string(), OwnedValue::string(big.as_str())),
                ("a".to_string(), OwnedValue::Int(0)),
                ("i".to_string(), OwnedValue::Int(0)),
                ("p".to_string(), OwnedValue::object_from([])),
                ("v".to_string(), OwnedValue::array_from(Vec::new())),
            ])
        };
        let text = |value: &OwnedValue| match value {
            OwnedValue::Object(map) => match map.get("x") {
                Some(OwnedValue::String(text)) => text.as_ptr(),
                other => panic!("missing sibling: {other:?}"), // patchcov: coverage tolerate-line reason="unreachable in a passing suite by design -- no row writes `x` (#7483)"
            },
            other => panic!("expected an object: {other:?}"), // patchcov: coverage tolerate-line reason="unreachable in a passing suite by design -- every row answers the state (#7483)"
        };
        for (program, read, want) in [
            (
                "def f($n): if $n == 0 then .a = 1 else f($n - 1) | f($n - 1) | .a += 1 end; f(6)",
                "a",
                "7",
            ),
            (
                r#"def f: if .i >= 50 then . else (.p["k\(.i)"] = 1 | .i += 1 | f) end; f"#,
                "p",
                &format!(
                    "{{{}}}",
                    (0..50)
                        .map(|i| format!("\"k{i}\":1"))
                        .collect::<Vec<_>>()
                        .join(",")
                ),
            ),
            ("def f: .a += 1; f | f | f", "a", "3"),
            (
                "def f($n): if $n == 0 then . else f($n - 1) | .v += [$n] end; f(5)",
                "v",
                "[1,2,3,4,5]",
            ),
            (
                "def f($n): if $n == 0 then . else (.a += $n | f($n - 1)) end; f(4)",
                "a",
                "10",
            ),
        ] {
            let input = state();
            let before = text(&input);
            let expr = crate::jq::parse(program).unwrap();
            let reindexed = crate::jq::value::reindex_count::get();
            let mut out = Vec::new();
            let (flow, copies) = share_stats::measure(|| {
                eval_moved::<JqSemantics>(&expr, input, false, &mut |v| {
                    out.push(v);
                    Demand::Continue
                })
            });
            assert!(
                matches!(flow, Flow::Exhausted),
                "{program}: did not run to its end"
            );
            assert_eq!(out.len(), 1, "{program}");
            let OwnedValue::Object(map) = &out[0] else {
                panic!("{program}: {:?}", out[0]); // patchcov: coverage tolerate-line reason="unreachable in a passing suite by design -- every row answers the state (#7483)"
            };
            assert_eq!(
                map.get(read).map(OwnedValue::to_json).as_deref(),
                Some(want),
                "{program}"
            );
            assert_eq!(
                text(&out[0]),
                before,
                "{program}: the untouched sibling was copied"
            );
            assert!(copies.is_empty(), "{program}: forced copies {copies:?}");
            assert_eq!(
                crate::jq::value::reindex_count::get(),
                reindexed,
                "{program}: a call re-indexed the state"
            );
            // The discriminator: with the door closed the call bridges the state, as before.
            CLOSED.with(|c| c.set(true));
            let reindexed = crate::jq::value::reindex_count::get();
            let _ = eval_moved::<JqSemantics>(&expr, state(), false, &mut |_| Demand::Continue);
            CLOSED.with(|c| c.set(false));
            assert!(
                crate::jq::value::reindex_count::get() > reindexed,
                "{program}: the closed door did not bridge"
            );
        }
    }

    /// #7483: the door answers what the route it replaces answers -- outputs, their order and how
    /// the run ends -- over programs that backtrack into a value a call, a pipe or an `if` was
    /// handed, which is where a value consumed too early would show: several outputs of a call, a
    /// branch or a closure argument, a state bound to a variable and read after a write, `label`,
    /// `limit`, `first`, `try`, `reduce`/`foreach` and `path` inside a definition. Every row must
    /// enter the door at least once; the CLI rows in `tests/jq_cli/i7483.rs` pin jq 1.7.1's
    /// answers.
    #[test]
    #[allow(clippy::literal_string_with_formatting_args)] // STYLE-0004: jq object literals, not format strings
    fn door_answers_as_the_route_it_replaces_7483() {
        let input = OwnedValue::object_from([
            ("a".to_string(), OwnedValue::Int(0)),
            (
                "x".to_string(),
                OwnedValue::array_from(vec![OwnedValue::Int(1), OwnedValue::Int(2)]),
            ),
        ]);
        for program in [
            "def f($n): if $n == 0 then .a = 1 else f($n - 1) | f($n - 1) | .a += 1 end; f(3)",
            "def f($n): if $n == 0 then (.a = 1), (.a = 2) else f($n - 1) | f($n - 1) | .c += 1 end; [f(2) | [.a, .c]]",
            "def f($n): if $n == 0 then . else (f($n - 1) | .c += 1), (f($n - 1) | .d += 1) end; [f(2)]",
            "def f: .a = 1, .a = 2; [f]",
            "def f: (.a = 1), .; [f, .]",
            "def f: if .a then .b = 1 else .c = 2 end; f, (.a = 1 | f)",
            "def f(g): g | .z = 1; [f(.a = 2, .a = 3)]",
            "def f: . as $s | .a = 1 | [$s, .]; f",
            "def f($v): .a = $v; . as $o | f(1) | [$o, .]",
            "def f: .x[0] = 9; . as $o | f | [., $o]",
            "def f: if .a < 3 then .a += 1 | f else . end; f, .",
            "def f: if .a < 3 then .a += 1 | f else . end; . as $o | f | [., $o]",
            "def f: if .a < 3 then (.a += 1 | f), . else . end; [f | .a]",
            "def f: label $out | (.a = 1 | ., break $out), .a = 2; [f]",
            "def f: first((.a = 1), (.a = 2)); f",
            "def f: limit(2; .a = (1, 2, 3)); [f]",
            "def f: try ((.a = 1), error(\"e\")) catch \"c\"; [f]",
            "def f: (.a = 1), error(\"e\"); [try f catch .]",
            "def f: .a.b = 1; try f catch .",
            "def f: reduce range(3) as $i (.; .a += $i); f, f",
            "def f: [foreach range(3) as $i (.; .a += $i)]; f",
            "def f: path(.a, .x[0]); [f]",
            "def f: getpath([\"a\"]), setpath([\"a\"]; 9), del(.a); [f]",
            "def f($n): if $n == 0 then . else f($n - 1) as $r | $r | .x += [$n] end; f(3)",
            "def f($n): if $n == 0 then . else f($n - 1) | .x += [$n] end; f(3), .",
            "def f(n): if n == 0 then . else f(n - 1) | .a += 1 end; f(4)",
            "def f: if .a > 0 then .a -= 1 | f | .r += 1 else .r = 0 end; .a = 3 | [f, .]",
            "def f: if . then .a = 1 elif .b then .c = 1 else empty end; f",
            "def f: if .a then error(\"x\") else .b = 1 end; .a = 1 | try f catch \"caught\"",
            "def f: if (.a, .b) then .c = 1 else .d = 1 end; [f]",
            "def f: def g: .a += 1; g | g; f",
            "def f($x; $y): .a = $x + $y; [f(1, 2; 3, 4)]",
            "def f($x): .b = $x; f(.x)",
            "def f: walk(if type == \"number\" then . + 1 else . end); f",
            "def f: recurse(if .a < 3 then .a += 1 else empty end); [f | .a]",
        ] {
            let before = entries();
            let (bridged, bridged_end) = run(program, &input, false);
            assert_eq!(entries(), before, "{program}: the closed door was entered");
            let (answered, answered_end) = run(program, &input, true);
            assert!(entries() > before, "{program}: the door was not entered");
            assert_eq!(
                (answered, answered_end),
                (bridged, bridged_end),
                "{program}"
            );
        }
        // yq mode keeps its route.
        let expr = crate::jq::parse("def f: .a = 1; f | f").unwrap();
        let before = entries();
        let _ = eval_each_owned::<YqSemantics>(&expr, &input, false, Reentry::REBUILT, &mut |_| {
            Demand::Continue
        });
        assert_eq!(entries(), before, "yq mode entered the door");
    }
    /// #7702: a fold or loop step whose UPDATE is a call to a `def` runs through the door with the
    /// state handed down, so the run re-indexes the state a fixed number of times however many steps
    /// it takes; with the door closed it re-indexes the state at every step, as before. Every row
    /// answers as the route it replaces: `reduce`, `foreach` with and without an EXTRACT (a call
    /// too), `until`, `while`, `recurse` and `limit`, and a pipe of a call and a write.
    #[test]
    #[allow(clippy::literal_string_with_formatting_args)] // STYLE-0004: jq object literals, not format strings
    fn fold_and_loop_steps_that_call_a_def_take_the_door_7702() {
        let defs = "def f($v): .a = $v; def g: .a; ";
        for (program, steps) in [
            ("{x:[range(200)]} | reduce range(60) as $i (.; f($i)) | .a", 60),
            ("{x:[range(200)]} | [foreach range(60) as $i (.; f($i); .a)] | length", 60),
            ("{x:[range(200)]} | [foreach range(60) as $i (.; f($i); g)] | add", 60),
            ("{x:[range(200)], a:0} | until(.a >= 60; f(.a + 1)) | .a", 60),
            ("{x:[range(200)], a:0} | [while(.a < 60; f(.a + 1)) | .a] | length", 60),
            ("{x:[range(200)], a:0} | [recurse(if .a < 60 then f(.a + 1) else empty end) | .a] | length", 60),
            ("{x:[range(200)], a:0} | last(limit(60; recurse(f(.a + 1)))) | .a", 60),
            ("{x:[range(200)], n:0} | reduce range(60) as $i (.; f($i) | .n += 1) | .n", 60),
        ] {
            let program = format!("{defs}{program}");
            let count = |door: bool| {
                let before = crate::jq::value::reindex_count::get();
                let answer = run(&program, &OwnedValue::Null, door);
                (answer, crate::jq::value::reindex_count::get() - before)
            };
            let (bridged, bridged_reindexes) = count(false);
            let (answered, reindexes) = count(true);
            assert_eq!(answered, bridged, "{program}");
            assert!(
                bridged_reindexes >= steps,
                "{program}: the closed door re-indexed {bridged_reindexes} times"
            );
            assert!(reindexes <= 4, "{program}: {reindexes} re-indexes over {steps} steps");
        }
    }
}
