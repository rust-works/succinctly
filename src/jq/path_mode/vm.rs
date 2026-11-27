//! The call machine (#7977, ADR-0026's route): jq-mode programs that hold a recursive `def`,
//! compiled to instructions and run on contiguous stacks in the image of jq 1.7.1's
//! `execute.c`.
//!
//! The shipped evaluators run a `def` call by substitution (ADR-0020) on the native stack
//! (ADR-0025), which is exponential or refused on the shapes of #7699, #7787, #7803, #7870,
//! #7893 and #7928. jq instead compiles a program once (`compile.c`) and runs it on explicit
//! stacks, so depth is data and a call costs a frame. This module is that model:
//!
//! - [`compile`] turns the program, and the prelude bodies it reaches, into one flat
//!   instruction vector ([`compile::Insn`]);
//! - a **data stack** holds values;
//! - a **frame stack** holds call frames, each naming its function, the frame it closes over
//!   (its static link, jq's `frame_get_level`) and its return address, with its parameters
//!   and locals in one flat **slot** vector (a closure is code plus a frame index, as jq's
//!   `struct closure`);
//! - a **fork stack** holds choice points (jq's `fork_point`), each saving the heights of the
//!   other three.
//!
//! The data and frame stacks follow gojq's `stack.go` ([`Stack`]): a block records the block
//! below it (`next`), `save` raises the protection watermark (`limit`) to the top, and `push`
//! writes above `max(index, limit)`, so a block a live fork point can still reach is never
//! overwritten. A value popped above the watermark is moved out; one at or below it is copied
//! (jq's `stack_pop`). A frame popped above the watermark frees its slots (`frame_pop`). The
//! same rule guards the slot vector: a slot below the top fork point's saved slot height
//! ([`Vm::watermark`]) belongs to a frame a fork point can resume into, so a last read of a
//! variable ([`compile::Insn::LoadLast`]) moves its value out only above that height.
//!
//! Backtracking pops a fork point, restores the heights it saved and resumes where it says
//! (jq's `stack_restore` and the `ON_BACKTRACK` arms). An error or a `break` is a backtrack
//! carrying the error; `try` is jq's `TRY_BEGIN`/`TRY_END`, and `label` a `try` whose handler
//! stops its own break. The dispatch loop ([`Vm::exec`]) is the only loop, so the native stack
//! stays constant whatever the recursion depth; only a leaf recurses natively, bounded by its
//! own text.
//!
//! A call-free subtree with no `.[]` and no `range` is a leaf, run by `Interp::eval` with
//! `Reg::top()`; its outputs past the first become a fork point that yields them one per
//! backtrack.
//!
//! The machine runs only once a caller turns it on ([`super::set_call_machine`]; the CLI does so
//! for `SUCCINCTLY_CALL_MACHINE`, #7961). Off, the default, [`takes`] is never asked and nothing
//! here runs.

use super::*;

mod compile;

use compile::{ArgSpec, Callee, Code, Insn, NO_LEVEL};
use core::sync::atomic::{AtomicBool, Ordering};

/// The byte budget of the control state: the stacks' blocks, slots and fork points. Past it a
/// call or a fork point refuses with the shipped evaluators' recursion wording ([`refusal`]).
/// jq itself has no such bound: it runs until memory runs out (a divergence #7977 step 2 records).
const BUDGET: usize = 1 << 30;

/// No return address: the main program's frame, whose `RET` is an output.
const NO_RET: usize = usize::MAX;

/// One block of a [`Stack`]: a value and the index of the block below it (`-1`: none).
struct Block<T> {
    value: T,
    next: isize,
}

/// gojq's `stack` (`stack.go`): a forest of blocks in one `Vec`, with a protection watermark.
///
/// `index` is the top block of the live stack and `limit` the highest block a live fork point
/// can still reach. Every block at or below `limit` is protected: it is never overwritten and
/// its value is never moved out, so a fork point that restores `(index, limit)` finds the stack
/// it saved. That one rule is what keeps backtracking sound; everything that moves a value out
/// asks [`Stack::top_free`] first. The one exception is [`Stack::pop_n`] (jq's `DUPN`), whose
/// protecting fork point resumes by popping the block unread.
struct Stack<T> {
    data: Vec<Block<T>>,
    index: isize,
    limit: isize,
}

impl<T> Stack<T> {
    fn new() -> Self {
        Self {
            data: Vec::new(),
            index: -1,
            limit: -1,
        }
    }

    /// `stack_push`: write above both the top and the watermark, so no protected block is
    /// overwritten.
    fn push(&mut self, value: T) {
        let at = self.index.max(self.limit) + 1;
        let block = Block {
            value,
            next: self.index,
        };
        let u = at as usize;
        if u < self.data.len() {
            self.data[u] = block;
        } else {
            self.data.push(block);
        }
        self.index = at;
    }

    /// Whether the top block is above the watermark: no fork point reaches it.
    fn top_free(&self) -> bool {
        self.index > self.limit
    }

    /// `stack_save`: protect every block up to the top, and return what [`Stack::restore`]
    /// resets to.
    fn save(&mut self) -> (isize, isize) {
        let saved = (self.index, self.limit);
        if self.index > self.limit {
            self.limit = self.index;
        }
        saved
    }

    /// `stack_restore`: back to a fork point's saved `(index, limit)`.
    fn restore(&mut self, (index, limit): (isize, isize)) {
        self.index = index;
        self.limit = limit;
        // Every block above both is unreachable now.
        self.data.truncate((index.max(limit) + 1) as usize);
    }
}

impl Stack<OwnedValue> {
    /// jq's `stack_pop`: moved out above the watermark, copied at or below it.
    fn pop(&mut self) -> OwnedValue {
        let i = self.index as usize;
        let free = self.top_free();
        self.index = self.data[i].next;
        if free {
            let value = core::mem::replace(&mut self.data[i].value, OwnedValue::Null);
            self.data.truncate(i);
            value
        } else {
            self.data[i].value.clone()
        }
    }

    /// jq's `stack_popn`: moved out either way, `null` left in a protected block.
    fn pop_n(&mut self) -> OwnedValue {
        let i = self.index as usize;
        let free = self.top_free();
        self.index = self.data[i].next;
        let value = core::mem::replace(&mut self.data[i].value, OwnedValue::Null);
        if free {
            self.data.truncate(i);
        }
        value
    }
}

/// A call frame (jq's `struct frame`), its parameters and locals held in [`Vm::slots`].
#[derive(Clone, Copy)]
struct FrameRec {
    /// The function it runs, an index into [`Code::funcs`].
    func: u32,
    /// The frame this one closes over (its static link), or -1.
    env: isize,
    /// Where its `RET` resumes ([`NO_RET`]: the main program's, whose `RET` is an output).
    ret: usize,
    /// Where its slots start.
    base: usize,
}

/// A closure: code over a frame (jq's `struct closure`).
#[derive(Clone, Copy)]
struct Closure {
    func: u32,
    frame: isize,
}

/// A frame's slot: a variable's value, a closure parameter, or a `[...]` accumulator.
enum Slot {
    Val(OwnedValue),
    Clo(Closure),
    Acc(Vec<OwnedValue>),
}

/// What a `try` fork point does with an error that reaches it.
enum Handler {
    /// `try` without `catch`, and `?`: an error ends the body with no output.
    None,
    Code(usize),
    /// A label's: stops this label's break, re-raises anything else.
    Label(u64),
}

/// What a fork point resumes when backtracked into.
enum Kind {
    /// `FORK`: resume at the address, unless an error is raised.
    Branch(usize),
    /// `TRY_BEGIN`, and a label's.
    Try(Handler),
    /// `TRY_END`: an error raised past it is wrapped, so its own `try` does not catch it.
    TryEnd,
    /// `EACH`: the container's next element.
    Each {
        container: OwnedValue,
        next: usize,
        resume: usize,
    },
    /// `RANGE`: the next number.
    Range { cur: f64, max: f64, resume: usize },
    /// A leaf's outputs past its first, then the error it ended with, if any.
    Values {
        vals: alloc::vec::IntoIter<OwnedValue>,
        end: Option<Raise>,
        resume: usize,
    },
}

/// A fork point (jq's `struct forkpoint`): what to resume, and the heights to restore.
struct Fork {
    kind: Kind,
    data: (isize, isize),
    frames: (isize, isize),
    slots: usize,
}

/// Where [`Vm::step`] goes next.
enum Flow {
    /// Run the instruction at this address.
    Next(usize),
    /// Backtrack: resume the top fork point.
    Back,
    /// The consumer wants no more outputs.
    Stop,
}

/// The machine's state for one run of one program.
struct Vm<'c, 'e> {
    it: &'c Interp<'e>,
    code: &'c Code<'e>,
    data: Stack<OwnedValue>,
    frames: Stack<FrameRec>,
    slots: Vec<Slot>,
    forks: Vec<Fork>,
    /// A call's arguments on their way into the callee's slots, kept between calls.
    scratch: Vec<Slot>,
}

/// The refusal past [`BUDGET`]: succinctly's own recursion-limit wording (ADR-0025), naming the
/// definition (`f/1 exceeded maximum recursion depth`).
fn refusal(name: &str, arity: usize) -> Raise {
    Raise::Fatal(EvalError::resource_limit(format!(
        "{}/{} exceeded maximum recursion depth",
        ModuleRun::display_name(name.strip_prefix("_sp_").unwrap_or(name)),
        arity
    )))
}

/// `RANGE`'s first output, as the shipped evaluator's `range` emits it (#3103, #6064): over
/// integer bounds an integer literal's spelling is dropped except `-0`'s, and over a float bound
/// the literals `range_from_literal_override` names keep theirs. So `range(0; 1) | -.` is `-0`, as
/// in jq and on the shipped route.
fn range_start(start: OwnedValue, end: &OwnedValue) -> OwnedValue {
    let is_float = |v: &OwnedValue| {
        matches!(
            v,
            OwnedValue::Float(_) | OwnedValue::NumberLiteral(NumberRepr::Float(_), _)
        )
    };
    let keep = if is_float(&start) || is_float(end) {
        super::super::eval::range_from_literal_override(&start).is_some()
    } else {
        matches!(&start, OwnedValue::NumberLiteral(NumberRepr::Int(0), text) if text.starts_with('-'))
    };
    match start {
        OwnedValue::NumberLiteral(NumberRepr::Int(i), _) if !keep => OwnedValue::Int(i),
        other => other,
    }
}

/// The number of elements `EACH` visits in `container`.
fn length(container: &OwnedValue) -> usize {
    match container {
        OwnedValue::Array(items) => items.len(),
        OwnedValue::Object(map) => map.len(),
        _ => 0,
    }
}

/// The `i`th element `EACH` visits in `container`: an array's, or an object's value in its
/// stored order.
fn element(container: &OwnedValue, i: usize) -> OwnedValue {
    match container {
        OwnedValue::Array(items) => items[i].clone(),
        OwnedValue::Object(map) => map.iter_stored().as_slice()[i].1.clone(),
        _ => OwnedValue::Null,
    }
}

/// Run `program` on `input`, a prepared document, on the machine, handing each output to
/// `on_value` (which answers whether it wants more). Asked only for a program [`takes`]
/// accepted.
///
/// Out of line and cold, so the shipped routes' frames do not grow by the machine's.
#[cold]
#[inline(never)]
pub(super) fn run<'e>(
    it: &Interp<'e>,
    program: &'e Expr,
    input: OwnedValue,
    on_value: &mut dyn FnMut(OwnedValue) -> bool,
) -> R {
    let code = compile::compile(it, program).map_err(unsupported)?;
    run_code(it, &code, input, on_value)
}

/// [`run`] of a compiled program.
fn run_code(
    it: &Interp<'_>,
    code: &Code<'_>,
    input: OwnedValue,
    on_value: &mut dyn FnMut(OwnedValue) -> bool,
) -> R {
    let mut vm = Vm {
        it,
        code,
        data: Stack::new(),
        frames: Stack::new(),
        slots: Vec::new(),
        forks: Vec::new(),
        scratch: Vec::new(),
    };
    vm.exec(input, on_value)
}

impl Vm<'_, '_> {
    /// The bytes the control state holds, for [`BUDGET`].
    fn bytes(&self) -> usize {
        self.data.data.len() * core::mem::size_of::<Block<OwnedValue>>()
            + self.frames.data.len() * core::mem::size_of::<Block<FrameRec>>()
            + self.slots.len() * core::mem::size_of::<Slot>()
            + self.forks.len() * core::mem::size_of::<Fork>()
    }

    /// The innermost definition on the call chain, for a refusal.
    fn current_def(&self) -> Raise {
        let mut f = self.frames.index;
        while f >= 0 {
            let fr = self.frames.data[f as usize].value;
            let func = &self.code.funcs[fr.func as usize];
            if !func.name.is_empty() {
                return refusal(func.name, func.arity);
            }
            f = self.frames.data[f as usize].next;
        }
        refusal("<program>", 0)
    }

    /// A fork point resuming `kind`, protecting what the stacks hold now (`stack_save`).
    fn save(&mut self, kind: Kind) -> Fork {
        Fork {
            kind,
            data: self.data.save(),
            frames: self.frames.save(),
            slots: self.slots.len(),
        }
    }

    /// Push a fork point, or refuse past [`BUDGET`].
    fn fork(&mut self, kind: Kind) -> Result<(), Raise> {
        if self.bytes() > BUDGET {
            return Err(self.current_def());
        }
        let fork = self.save(kind);
        self.forks.push(fork);
        Ok(())
    }

    /// `frame_get_level`: the frame `level` static links out from the current one.
    fn frame_at(&self, level: u16) -> isize {
        let mut f = self.frames.index;
        for _ in 0..level {
            f = self.frames.data[f as usize].value.env;
        }
        f
    }

    /// The slots watermark: every slot below it belongs to a frame a live fork point can
    /// resume into. Fork points save non-decreasing slot heights, so the top one is the most.
    fn watermark(&self) -> usize {
        self.forks.last().map_or(0, |f| f.slots)
    }

    /// The index in [`Vm::slots`] of slot `idx` of the frame `level` links out.
    fn slot_at(&self, level: u16, idx: u16) -> usize {
        let f = self.frame_at(level);
        self.frames.data[f as usize].value.base + idx as usize
    }

    fn slot(&mut self, level: u16, idx: u16) -> &mut Slot {
        let f = self.frame_at(level);
        let base = self.frames.data[f as usize].value.base;
        &mut self.slots[base + idx as usize]
    }

    /// `LOADV`: a copy of the variable at `(level, idx)`.
    fn value(&mut self, level: u16, idx: u16) -> OwnedValue {
        match self.slot(level, idx) {
            Slot::Val(v) => v.clone(),
            _ => OwnedValue::Null,
        }
    }

    /// `frame_push`, without its slots: the caller pushes those.
    fn push_frame(&mut self, func: u32, env: isize, ret: usize) {
        let base = self.slots.len();
        self.frames.push(FrameRec {
            func,
            env,
            ret,
            base,
        });
    }

    /// `frame_pop`: a frame above the watermark frees its slots.
    fn pop_frame(&mut self) -> FrameRec {
        let i = self.frames.index as usize;
        let free = self.frames.top_free();
        let fr = self.frames.data[i].value;
        self.frames.index = self.frames.data[i].next;
        if free {
            self.slots.truncate(fr.base);
            self.frames.data.truncate(i);
        }
        fr
    }

    /// The dispatch loop (`jq_next`): run from the main function's entry until no fork point is
    /// left, an error reaches the top, or the consumer stops.
    fn exec(&mut self, input: OwnedValue, on_value: &mut dyn FnMut(OwnedValue) -> bool) -> R {
        let code = self.code;
        let main = &code.funcs[code.main as usize];
        self.push_frame(code.main, -1, NO_RET);
        for _ in 0..main.nslots {
            self.slots.push(Slot::Val(OwnedValue::Null));
        }
        self.data.push(input);
        let mut pc = main.entry;
        loop {
            let flow = match self.step(pc, on_value) {
                Ok(flow) => flow,
                Err(raise) => match self.backtrack(Some(raise))? {
                    Some(at) => {
                        pc = at;
                        continue;
                    }
                    None => return Ok(()),
                },
            };
            pc = match flow {
                Flow::Next(at) => at,
                Flow::Back => match self.backtrack(None)? {
                    Some(at) => at,
                    None => return Ok(()),
                },
                Flow::Stop => return Err(Raise::Stop),
            };
        }
    }

    /// Run the instruction at `pc`. An `Err` is a raise, which [`Vm::backtrack`] unwinds.
    fn step(
        &mut self,
        pc: usize,
        on_value: &mut dyn FnMut(OwnedValue) -> bool,
    ) -> Result<Flow, Raise> {
        let code = self.code;
        let next = Flow::Next(pc + 1);
        #[cfg(test)]
        EXECUTED.with(|seen| {
            seen.borrow_mut()
                .insert(compile::insn_name(&code.insns[pc]))
        });
        match &code.insns[pc] {
            Insn::Dup => {
                let v = self.data.pop();
                self.data.push(v.clone());
                self.data.push(v);
            }
            Insn::DupN => {
                let v = self.data.pop_n();
                self.data.push(v.clone());
                self.data.push(v);
            }
            Insn::Pop => {
                self.data.pop();
            }
            Insn::Swap => {
                let a = self.data.pop();
                let b = self.data.pop();
                self.data.push(a);
                self.data.push(b);
            }
            Insn::Const(k) => {
                self.data.pop();
                self.data.push(code.consts[*k as usize].clone());
            }
            Insn::Leaf(k) => return self.leaf(*k, pc),
            Insn::Each(opt) => {
                let c = self.data.pop();
                if !matches!(c, OwnedValue::Array(_) | OwnedValue::Object(_)) {
                    if *opt {
                        return Ok(Flow::Back);
                    }
                    return Err(
                        match self.it.each(c, Reg::top(), false, &mut |_, _| Ok(())) {
                            Err(raise) => raise,
                            Ok(()) => unsupported("iterate answered"),
                        },
                    );
                }
                let n = length(&c);
                if n == 0 {
                    return Ok(Flow::Back);
                }
                let first = element(&c, 0);
                if n > 1 {
                    self.fork(Kind::Each {
                        container: c,
                        next: 1,
                        resume: pc + 1,
                    })?;
                }
                self.data.push(first);
            }
            Insn::Range => {
                let max = self.data.pop();
                let start = self.data.pop();
                let (Some(cur), Some(hi)) = (number_f64(&start), number_f64(&max)) else {
                    return Err(Raise::Error(EvalError::new("Range bounds must be numeric")));
                };
                if cur >= hi {
                    return Ok(Flow::Back);
                }
                if cur + 1.0 < hi || (cur + 1.0).is_nan() || hi.is_nan() {
                    self.fork(Kind::Range {
                        cur: cur + 1.0,
                        max: hi,
                        resume: pc + 1,
                    })?;
                }
                self.data.push(range_start(start, &max));
            }
            Insn::Fork(t) => self.fork(Kind::Branch(*t))?,
            Insn::Jump(t) => return Ok(Flow::Next(*t)),
            Insn::JumpF(t) => {
                let v = self.data.pop();
                if !truthy(&v) {
                    return Ok(Flow::Next(*t));
                }
            }
            Insn::JumpIfSet(s, t) => {
                if matches!(self.slot(0, *s), Slot::Val(v) if truthy(v)) {
                    return Ok(Flow::Next(*t));
                }
            }
            Insn::SetSlot(s, b) => *self.slot(0, *s) = Slot::Val(OwnedValue::Bool(*b)),
            Insn::Backtrack => return Ok(Flow::Back),
            Insn::TryBegin(h) => {
                let handler = h.map_or(Handler::None, Handler::Code);
                self.fork(Kind::Try(handler))?;
            }
            Insn::TryEnd => self.fork(Kind::TryEnd)?,
            Insn::Label(s) => {
                let id = self.it.next_label.get();
                self.it.next_label.set(id + 1);
                *self.slot(0, *s) = Slot::Val(OwnedValue::Int(id as i64));
                self.fork(Kind::Try(Handler::Label(id)))?;
            }
            Insn::Break(level, idx) => {
                let id = match self.value(*level, *idx) {
                    OwnedValue::Int(id) => id as u64,
                    _ => return Err(unsupported("a label slot without a label")),
                };
                return Err(Raise::Break(id));
            }
            Insn::Store(s) => {
                let v = self.data.pop();
                *self.slot(0, *s) = Slot::Val(v);
            }
            Insn::Load(level, idx) => {
                self.data.pop();
                let v = self.value(*level, *idx);
                self.data.push(v);
            }
            Insn::LoadMove(level, idx) => {
                self.data.pop();
                let v = match self.slot(*level, *idx) {
                    Slot::Val(v) => core::mem::replace(v, OwnedValue::Null),
                    _ => OwnedValue::Null,
                };
                self.data.push(v);
            }
            Insn::LoadLast(level, idx) => {
                self.data.pop();
                let at = self.slot_at(*level, *idx);
                let free = at >= self.watermark();
                let v = match &mut self.slots[at] {
                    Slot::Val(v) if free => core::mem::replace(v, OwnedValue::Null),
                    Slot::Val(v) => v.clone(),
                    _ => OwnedValue::Null,
                };
                self.data.push(v);
            }
            Insn::Kill(s) => {
                let at = self.slot_at(0, *s);
                if at >= self.watermark() {
                    self.slots[at] = Slot::Val(OwnedValue::Null);
                }
            }
            Insn::NewAcc(s) => *self.slot(0, *s) = Slot::Acc(Vec::new()),
            Insn::Append(s) => {
                let v = self.data.pop();
                if let Slot::Acc(items) = self.slot(0, *s) {
                    items.push(v);
                }
            }
            Insn::LoadAcc(s) => {
                self.data.pop();
                let items = match self.slot(0, *s) {
                    Slot::Acc(items) => core::mem::take(items),
                    _ => Vec::new(),
                };
                self.data.push(OwnedValue::Array(items.into()));
            }
            Insn::Bin(op) => {
                let l = self.data.pop();
                let r = self.data.pop();
                self.data.push(combine(*op, &l, &r).map_err(raise_eval)?);
            }
            Insn::Cmp(op) => {
                let l = self.data.pop();
                let r = self.data.pop();
                self.data
                    .push(OwnedValue::Bool(apply_compare_op::<JqSemantics>(
                        *op, &l, &r,
                    )));
            }
            Insn::ToBool => {
                let v = self.data.pop();
                self.data.push(OwnedValue::Bool(truthy(&v)));
            }
            Insn::Not => {
                let v = self.data.pop();
                self.data.push(OwnedValue::Bool(!truthy(&v)));
            }
            Insn::Negate => {
                let v = self.data.pop();
                self.data
                    .push(arith_negate::<JqSemantics>(v).map_err(raise_eval)?);
            }
            Insn::Index => {
                let k = self.data.pop();
                let c = self.data.pop();
                let v = self.it.jv_get(&c, &k)?;
                self.data.push(v);
            }
            Insn::IndexK(k) => {
                let c = self.data.pop();
                let v = self.it.jv_get(&c, &code.consts[*k as usize])?;
                self.data.push(v);
            }
            Insn::Raise => {
                let v = self.data.pop();
                return Err(Raise::Error(EvalError::from_value(v)));
            }
            Insn::SetPath => {
                let input = self.data.pop();
                let path = self.data.pop();
                let value = self.data.pop();
                let written = match &path {
                    OwnedValue::Array(keys) => {
                        super::super::eval::set_value_at_path::<JqSemantics>(input, keys, value)
                            .map_err(raise_eval)?
                    }
                    _ => {
                        // Not an array: the leaf evaluator's own answer (its error).
                        let call = Builtin::SetPath(
                            Box::new(Expr::tracked_value(path.clone())),
                            Box::new(Expr::tracked_value(value.clone())),
                        );
                        let mut out = None;
                        emit_c_builtin(
                            &Expr::Builtin(call.clone()),
                            &call,
                            &input,
                            &[path, value],
                            &Reg::top(),
                            true,
                            &mut |v, _| {
                                out.get_or_insert(v);
                                Ok(())
                            },
                        )?;
                        out.ok_or_else(|| unsupported("setpath answered nothing"))?
                    }
                };
                self.data.push(written);
            }
            Insn::Call { callee, args, tail } => return self.call(pc, *callee, args, *tail),
            Insn::Ret => {
                let fr = self.pop_frame();
                if fr.ret == NO_RET {
                    let v = self.data.pop();
                    return Ok(if on_value(v) { Flow::Back } else { Flow::Stop });
                }
                return Ok(Flow::Next(fr.ret));
            }
        }
        Ok(next)
    }

    /// Run leaf `k` on `Interp::eval` over its free variables: the first output continues at
    /// once, the rest (and an error after them) become a [`Kind::Values`] fork point.
    fn leaf(&mut self, k: u32, pc: usize) -> Result<Flow, Raise> {
        let leaf = &self.code.leaves[k as usize];
        let input = self.data.pop();
        let mut env = Env::default();
        for &(name, level, idx) in &leaf.vars {
            let v = self.value(level, idx);
            env = env.var_ref(name, v);
        }
        let mut first = None;
        let mut rest = Vec::new();
        let end = self
            .it
            .eval(leaf.expr, &env, input, Reg::top(), &mut |v, _| {
                if first.is_none() {
                    first = Some(v);
                } else {
                    rest.push(v);
                }
                Ok(())
            });
        let Some(v) = first else {
            return match end {
                Ok(()) => Ok(Flow::Back),
                Err(raise) => Err(raise),
            };
        };
        if !rest.is_empty() || end.is_err() {
            self.fork(Kind::Values {
                vals: rest.into_iter(),
                end: end.err(),
                resume: pc + 1,
            })?;
        }
        self.data.push(v);
        Ok(Flow::Next(pc + 1))
    }

    /// `CALL_JQ` / `TAIL_CALL_JQ`: bind the arguments into a new frame and jump to the
    /// callee's entry. A tail call pops the caller's frame first (it holds nothing the callee
    /// refers to, [`compile`]'s `mark_tails`), so a tail loop runs in constant frames.
    fn call(
        &mut self,
        pc: usize,
        callee: Callee,
        args: &[ArgSpec],
        tail: bool,
    ) -> Result<Flow, Raise> {
        let code = self.code;
        let input = self.data.pop();
        let cur = self.frames.index;
        let (func, env) = match callee {
            Callee::Def { func, level } => (
                func,
                if level == NO_LEVEL {
                    -1
                } else {
                    self.frame_at(level)
                },
            ),
            Callee::Param { level, idx } => match self.slot(level, idx) {
                Slot::Clo(c) => (c.func, c.frame),
                _ => return Err(unsupported("a parameter slot without a closure")),
            },
        };
        let f = &code.funcs[func as usize];
        if self.bytes() > BUDGET {
            return Err(refusal(f.name, f.arity));
        }
        // The arguments, last first: values come off the data stack.
        let mut scratch = core::mem::take(&mut self.scratch);
        scratch.clear();
        for spec in args.iter().rev() {
            scratch.push(match *spec {
                ArgSpec::Value => Slot::Val(self.data.pop()),
                ArgSpec::Lambda(l) => Slot::Clo(Closure {
                    func: l,
                    frame: cur,
                }),
                ArgSpec::Ref { level, idx } => match self.slot(level, idx) {
                    Slot::Clo(c) => Slot::Clo(*c),
                    _ => return Err(unsupported("a parameter slot without a closure")),
                },
            });
        }
        let ret = if tail {
            // `TAIL_CALL_JQ`: the frame goes before its successor comes; nothing the new
            // frame holds refers to it.
            self.pop_frame().ret
        } else {
            pc + 1
        };
        self.push_frame(func, env, ret);
        while let Some(slot) = scratch.pop() {
            self.slots.push(slot);
        }
        for _ in args.len()..f.nslots as usize {
            self.slots.push(Slot::Val(OwnedValue::Null));
        }
        self.scratch = scratch;
        self.data.push(input);
        Ok(Flow::Next(f.entry))
    }

    /// `stack_restore` and the `ON_BACKTRACK` arms: the address to resume at, `None` when no
    /// fork point is left and nothing is raised.
    fn backtrack(&mut self, mut err: Option<Raise>) -> Result<Option<usize>, Raise> {
        loop {
            let Some(fork) = self.forks.pop() else {
                return match err {
                    Some(raise) => Err(raise),
                    None => Ok(None),
                };
            };
            // Nothing past an uncatchable raise runs: drop the rest at once.
            if matches!(
                err,
                Some(Raise::Fatal(_) | Raise::Halt(_) | Raise::Stop | Raise::Unsupported(_))
            ) {
                self.forks.clear();
                return Err(err.take().unwrap_or(Raise::Stop));
            }
            self.data.restore(fork.data);
            self.frames.restore(fork.frames);
            self.slots.truncate(fork.slots);
            match fork.kind {
                Kind::Branch(at) => {
                    if err.is_none() {
                        return Ok(Some(at));
                    }
                }
                Kind::TryEnd => {
                    // Wrap the error, so the matching `try` does not catch it.
                    if let Some(raise) = err.take() {
                        err = Some(Raise::Wrapped(Box::new(raise)));
                    }
                }
                Kind::Try(handler) => match err.take() {
                    None => {}
                    Some(Raise::Wrapped(inner)) => err = Some(*inner),
                    Some(raise) if raise.is_error() => match handler {
                        Handler::None => {}
                        Handler::Code(at) => {
                            self.data.pop();
                            self.data.push(caught(raise));
                            return Ok(Some(at));
                        }
                        Handler::Label(id) => {
                            let mine = match &raise {
                                Raise::Break(n) => *n == id,
                                Raise::Error(e) => is_label_payload(e, id),
                                _ => false,
                            };
                            if !mine {
                                err = Some(raise);
                            }
                        }
                    },
                    Some(other) => err = Some(other),
                },
                Kind::Each {
                    container,
                    next,
                    resume,
                } => {
                    if err.is_none() {
                        let v = element(&container, next);
                        if next + 1 < length(&container) {
                            let fork = self.save(Kind::Each {
                                container,
                                next: next + 1,
                                resume,
                            });
                            self.forks.push(fork);
                        }
                        self.data.push(v);
                        return Ok(Some(resume));
                    }
                }
                Kind::Range { cur, max, resume } => {
                    if err.is_none() {
                        if cur + 1.0 < max || cur.is_nan() || max.is_nan() {
                            let fork = self.save(Kind::Range {
                                cur: cur + 1.0,
                                max,
                                resume,
                            });
                            self.forks.push(fork);
                        }
                        self.data.push(number_owned(cur));
                        return Ok(Some(resume));
                    }
                }
                Kind::Values {
                    mut vals,
                    end,
                    resume,
                } => {
                    if err.is_none() {
                        match vals.next() {
                            Some(v) => {
                                if vals.len() > 0 || end.is_some() {
                                    let fork = self.save(Kind::Values { vals, end, resume });
                                    self.forks.push(fork);
                                }
                                self.data.push(v);
                                return Ok(Some(resume));
                            }
                            None => err = end,
                        }
                    }
                }
            }
        }
    }
}

/// Whether `program` holds a `def` whose body calls it by name and arity: the programs the
/// machine is for. A program without one keeps the shipped route, so the machine changes
/// nothing for it.
fn self_recursive(program: &Expr) -> bool {
    any_subexpr(program, &mut |e| match e {
        Expr::FuncDef {
            name, params, body, ..
        } => any_subexpr(body, &mut |m| {
            matches!(m, Expr::FuncCall { name: n, args, .. }
                if n == name && args.len() == params.len())
        }),
        _ => false,
    })
}

/// Whether the machine is on for this process ([`set_switched_on`]); off until a caller turns it
/// on. The library reads no environment: the CLI turns it on from `SUCCINCTLY_CALL_MACHINE`
/// (`jq_runner/call_machine.rs`, #7961).
static ON: AtomicBool = AtomicBool::new(false);

/// Whether the machine is on. Everything that could hand a program to the machine asks this
/// first, so with it off the machine costs one load per question and nothing else.
#[inline]
pub(super) fn switched_on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Turn the machine on or off ([`super::set_call_machine`]).
pub(super) fn set_switched_on(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

/// The function told of each routing decision while the machine is on
/// ([`super::observe_call_machine`]).
#[cfg(feature = "std")]
static OBSERVER: std::sync::Mutex<Option<fn(bool)>> = std::sync::Mutex::new(None);

/// Install `observer` ([`super::observe_call_machine`]).
#[cfg(feature = "std")]
pub(super) fn set_observer(observer: Option<fn(bool)>) {
    if let Ok(mut slot) = OBSERVER.lock() {
        *slot = observer;
    }
}

#[cfg(feature = "std")]
std::thread_local! {
    /// [`takes`]'s answer for the pinned program, by address: `(0, _)` when none is kept.
    static TAKES: Cell<(usize, bool)> = const { Cell::new((0, false)) };
}

/// Whether the machine runs `program` ([`decide`]). The answer is remembered while the
/// program's route is pinned (`PINNED_ROUTE`, #7719), so a CLI run decides twice whatever its
/// number of documents: once as its route is pinned, once more on its first document. A program
/// that is not pinned is decided afresh.
///
/// Callers ask [`switched_on`] first: this compiles the program.
#[inline(never)]
pub(super) fn takes(program: &Expr) -> bool {
    #[cfg(feature = "std")]
    {
        let key = program as *const Expr as usize;
        let pinned = PINNED_ROUTE.with(Cell::get).0 == key;
        let (seen, answer) = TAKES.with(Cell::get);
        if pinned && seen == key {
            return answer;
        }
        let answer = decide(program);
        TAKES.with(|t| t.set(if pinned { (key, answer) } else { (0, false) }));
        answer
    }
    #[cfg(not(feature = "std"))]
    {
        decide(program)
    }
}

/// The routing rule: a program with a self-recursive `def` whose every node [`compile`] accepts
/// (it declines the input builtins among the rest). The compiler is the decliner, so there is one
/// definition of what the machine runs.
#[cold]
#[inline(never)]
fn decide(program: &Expr) -> bool {
    let answer = self_recursive(program)
        && with_prelude(|prelude| {
            let it = Interp::new(prelude);
            compile::compile(&it, program).is_ok()
        });
    note_decision(answer);
    answer
}

/// Tell the observer, if one is installed, of a decision.
fn note_decision(taken: bool) {
    #[cfg(feature = "std")]
    if let Some(observer) = OBSERVER.lock().ok().and_then(|slot| *slot) {
        observer(taken);
    }
    #[cfg(not(feature = "std"))]
    let _ = taken;
}

#[cfg(test)]
std::thread_local! {
    /// The instructions [`Vm::step`] ran on this thread, by name, for the instruction test.
    static EXECUTED: core::cell::RefCell<alloc::collections::BTreeSet<&'static str>> =
        const { core::cell::RefCell::new(alloc::collections::BTreeSet::new()) };
}

#[cfg(test)]
mod tests;
