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
//! A call-free subtree is a leaf, run by `Interp::eval` with `Reg::top()`, reading its free
//! variables from the slots through a borrowed view ([`LeafView`]); its outputs past the first
//! become a fork point that yields them one per backtrack. A leaf holds `.[]` and `range(n)` only
//! where it takes all their outputs itself, under a `[...]` or in a `reduce` (#7977 step 3), so
//! `[range(1e6)]` is built in one pass and `first(.[])` still stops after one element.
//!
//! The machine runs only once a caller turns it on ([`super::set_call_machine`]; the CLI does so
//! for `SUCCINCTLY_CALL_MACHINE`, #7961). Off, the default, [`decide`] is never asked and nothing
//! here runs.
//!
//! A program is compiled once, by [`decide`], and the [`Compiled`] code it answers is what [`run`]
//! runs. The code owns what it holds and nothing in it changes as it runs, so a run whose route is
//! pinned keeps it with the pin and runs it on every document (#7977 step 5).

use super::*;

mod compile;

use compile::{ArgSpec, Callee, Code, Insn, Tail, NO_LEVEL};
use core::sync::atomic::{AtomicBool, Ordering};

/// The byte budget of the control state ([`Vm::bytes`]): the capacity of the data, frame, slot
/// and fork stacks, as this run has grown them ([`grow`]; a stack kept from an earlier document
/// is charged as it is grown into, [`Spare`]). Past it a call, a fork point or a growth refuses with the shipped
/// evaluators' recursion wording ([`refusal`]), and so does a failed allocation of any of them.
/// jq itself has no such bound: it runs until memory runs out, an ADR-0018 rule 4(c) divergence
/// recorded in `docs/compliance/jq/limitations.md`. It bounds the machine's own state, not
/// memory: what the values hold is not counted, nor are `[...]` accumulators and buffered leaf
/// outputs, which hold values the program builds (they grow fallibly, so a failed allocation is
/// a refusal too). What a run holds in all is bounded by the memory limit ([`Limits::memory`],
/// #8064) where a caller installed a probe.
const BUDGET: usize = 1 << 30;

/// A frame's `rounds` in a `repeat` loop a consumer can stop (`limit`, `first`), which the
/// shipped evaluators do not bound ([`Vm::repeat_is_eager`]): its rounds are not counted, and
/// `emitted` holds [`QUIET`] and its rounds in a row that output nothing.
const UNCOUNTED: u32 = u32::MAX;

/// The `emitted` of an uncounted `repeat`: above every output bound, so each of its outputs
/// takes [`Vm::charge_output`]'s path, which marks the round [`HEARD`].
const QUIET: u32 = 1 << 31;

/// An uncounted `repeat`'s round that has output something.
const HEARD: u32 = 1 << 30;

/// A [`Counts::owner`] for a loop entered by a tail call: the frame's own, and its outputs
/// counted although it is not yet a round.
const COUNTED: i32 = -2;

// A frame counts its loop's rounds and outputs in `u32`s, below `UNCOUNTED` and `HEARD`.
const _: () = assert!(MAX_LOOP_ROUNDS < u32::MAX as usize - BOUND_ROUND_WEIGHT);
const _: () = assert!(MAX_LOOP_OUTPUTS < HEARD as usize);
const _: () = assert!(MAX_REPEAT_ROUNDS < MAX_LOOP_ROUNDS);
const _: () = assert!(REPEAT_WIDTH_BUDGET < MAX_LOOP_OUTPUTS);

/// The bounds one run keeps: [`SHIPPED`] for every run, smaller ones in the unit tests.
#[derive(Clone, Copy)]
struct Limits {
    /// The byte budget ([`BUDGET`]).
    bytes: usize,
    /// The rounds a tail loop may run: the shipped `MAX_LOOP_ROUNDS` (#5379, #5292).
    rounds: u32,
    /// The outputs a zero-argument `def`'s tail loop may emit once it has run
    /// `OUTPUT_BUDGET_AFTER_ROUNDS` rounds: the shipped `MAX_LOOP_OUTPUTS` (#5379).
    outputs: u32,
    /// The numbers one `range` may produce: the shipped `MAX_RANGE_TOTAL` (#5292).
    range: usize,
    /// The process's memory and its limit, when a caller installed a probe
    /// ([`super::set_call_machine_memory`], #8064). With one, the run is refused once the
    /// process holds more than the limit, and no loop is refused for its outputs; without one,
    /// the output bound above stands, so a run is never unbounded.
    memory: Option<MemoryProbe>,
}

/// The bounds of every run, the shipped evaluators' own.
const SHIPPED: Limits = Limits {
    bytes: BUDGET,
    rounds: MAX_LOOP_ROUNDS as u32,
    outputs: MAX_LOOP_OUTPUTS as u32,
    range: MAX_RANGE_TOTAL,
    memory: None,
};

/// How a run reads the memory the process holds, and the most it may (#8064).
///
/// The library reads no figure of the process itself: the CLI installs one, sampling its resident
/// memory (`/proc/self/statm` on Linux, the physical footprint on macOS) against a limit it
/// computes once per process from the memory available to it.
#[derive(Clone, Copy)]
pub(super) struct MemoryProbe {
    /// The bytes the process holds now, `None` when they cannot be read.
    pub(super) sample: fn() -> Option<usize>,
    /// The most it may hold before a run is refused.
    pub(super) limit: usize,
}

/// The probe installed for the process ([`set_memory_probe`]).
#[cfg(feature = "std")]
static PROBE: std::sync::OnceLock<MemoryProbe> = std::sync::OnceLock::new();

/// Install `probe` for every later run of the process (the first one installed stays).
#[cfg(feature = "std")]
pub(super) fn set_memory_probe(probe: MemoryProbe) {
    let _ = PROBE.set(probe);
}

/// The probe installed for the process, if any.
fn memory_probe() -> Option<MemoryProbe> {
    #[cfg(feature = "std")]
    {
        PROBE.get().copied()
    }
    #[cfg(not(feature = "std"))]
    {
        None
    }
}

/// The fewest and the most check points between two samples of the process's memory.
const MIN_INTERVAL: u32 = 16;
const MAX_INTERVAL: u32 = 1 << 16;

/// When a run next samples the process's memory (#8064): every `interval` check points (a call,
/// an append, a store, a leaf, an operator that builds a container or a string), the
/// interval set at each sample so that, at the growth per check point seen since the last one,
/// the next sample comes when at most half the room left is spent. A fast-growing run is so
/// sampled more often as it nears the limit, and a run far from it rarely. Kept between the
/// documents of a run ([`Spare`]), as the process's memory is one figure across them.
#[derive(Clone, Copy)]
struct Sampler {
    /// Check points left before the next sample.
    left: u32,
    /// Check points between the last sample and the next.
    interval: u32,
    /// The bytes the last sample read, 0 before the first.
    last: usize,
}

impl Default for Sampler {
    fn default() -> Self {
        Self {
            left: MIN_INTERVAL,
            interval: MIN_INTERVAL,
            last: 0,
        }
    }
}

impl Sampler {
    /// The check points until the next sample after reading `now` bytes against `limit`, with
    /// `interval` check points since the reading `last` (0: none).
    fn next_interval(last: usize, now: usize, interval: u32, limit: usize) -> u32 {
        let room = limit.saturating_sub(now);
        if last == 0 {
            return MIN_INTERVAL;
        }
        let per_check = now.saturating_sub(last) / interval.max(1) as usize;
        if per_check == 0 {
            return MAX_INTERVAL;
        }
        let checks = room / 2 / per_check;
        checks.clamp(MIN_INTERVAL as usize, MAX_INTERVAL as usize) as u32
    }
}

/// No return address: the main program's frame, whose `RET` is an output.
const NO_RET: u32 = u32::MAX;

/// One block of a [`Stack`]: a value and the index of the block below it (`-1`: none). An index
/// fits in an `i32`, as the budget bounds the stacks well below that.
struct Block<T> {
    value: T,
    next: i32,
}

// A frame block is as small as step 1's (#7977): 36 bytes of frame and a 4-byte link.
const _: () = assert!(core::mem::size_of::<FrameRec>() == 36);
const _: () = assert!(core::mem::size_of::<Block<FrameRec>>() == 40);

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
    /// The capacity the budget charges ([`Vm::bytes`]): what this run has grown the stack to,
    /// never more than `data`'s own capacity, which may be larger when the vector is kept from
    /// an earlier document ([`Spare`]).
    cap: usize,
}

impl<T> Stack<T> {
    #[cfg(test)]
    fn new() -> Self {
        Self::over(Vec::new())
    }

    /// An empty stack over `data`, an empty vector kept from an earlier run, none of whose
    /// capacity is charged yet.
    fn over(data: Vec<Block<T>>) -> Self {
        debug_assert!(data.is_empty());
        Self {
            data,
            index: -1,
            limit: -1,
            cap: 0,
        }
    }

    /// `stack_push`: write above both the top and the watermark, so no protected block is
    /// overwritten.
    fn push(&mut self, value: T) {
        let at = self.index.max(self.limit) + 1;
        let block = Block {
            value,
            next: self.index as i32,
        };
        let u = at as usize;
        if u < self.data.len() {
            self.data[u] = block;
        } else {
            self.data.push(block);
        }
        self.index = at;
    }

    /// Whether [`Stack::push`] has room within the charged capacity, so without growing.
    fn has_room(&self) -> bool {
        ((self.index.max(self.limit) + 1) as usize) < self.cap
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
        self.index = self.data[i].next as isize;
        if free {
            // The top block is most often the last: taken off whole, with nothing left to drop.
            if i + 1 == self.data.len() {
                if let Some(block) = self.data.pop() {
                    return block.value;
                }
            }
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
        self.index = self.data[i].next as isize;
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
    /// The counts of the loop it is in.
    counts: Counts,
    /// Where its `RET` resumes ([`NO_RET`]: the main program's, whose `RET` is an output).
    ret: u32,
    /// The frame its `RET` returns into, when it is not the one below it (`-1`): a callee that
    /// closes over its caller's frame, called in tail position (`Tail::Keep`), returns past
    /// the caller, whose frame it needed only while it ran.
    exit: i32,
    /// The frame this one closes over (its static link), or -1. A frame index fits in an `i32`
    /// and a slot index in a `u32`, as the budget bounds both stacks well below that.
    env: i32,
    /// Where its slots start.
    base: u32,
}

/// A frame's loop counts ([`Vm::loop_counts`]). A tail call hands them on; a protected frame's
/// counts are written in place, as they are the loop's, whichever path through it runs.
#[derive(Clone, Copy)]
struct Counts {
    /// The rounds of the loop: the tail calls to its definitions since the ordinary call that
    /// started it, charged against [`Limits::rounds`] ([`UNCOUNTED`] in a `repeat` a consumer
    /// can stop).
    rounds: u32,
    /// The outputs (`RET`s) the loop has made so far, charged against [`Limits::outputs`]
    /// ([`QUIET`] in an uncounted `repeat`).
    emitted: u32,
    /// The frame whose loop this frame's outputs are charged to, when it is not this one's own:
    /// a helper or a closure the loop's round tail-called, while a fork point keeps the loop's
    /// frame (`-1`: its own, counted once it is a round; [`COUNTED`]: its own, counted from the
    /// call that entered the loop).
    owner: i32,
    /// The definition the loop started at, which a refusal is named for, as the shipped tail
    /// loop names it (the low [`ENTRY_BITS`]), and the loop's rounds unweighted, saturating at
    /// `OUTPUT_BUDGET_AFTER_ROUNDS` (the bits above): the shipped output bound starts after that
    /// many tail rounds, however much each one costs.
    entry: u32,
}

/// The bits of [`Counts::entry`] that name the definition; the compiler declines a program of
/// more functions.
pub(super) const ENTRY_BITS: u32 = 25;

const ENTRY_MASK: u32 = (1 << ENTRY_BITS) - 1;

// The unweighted round count fits above the function index.
const _: () = assert!(OUTPUT_BUDGET_AFTER_ROUNDS < (1 << (32 - ENTRY_BITS)));

impl Counts {
    /// The counts of a loop that starts at `func`.
    const fn fresh(func: u32) -> Self {
        Self {
            rounds: 0,
            emitted: 0,
            owner: -1,
            entry: func,
        }
    }

    /// The definition the loop started at.
    fn entry(self) -> u32 {
        self.entry & ENTRY_MASK
    }

    /// The loop's tail rounds, unweighted, up to `OUTPUT_BUDGET_AFTER_ROUNDS`.
    fn taken(self) -> u32 {
        self.entry >> ENTRY_BITS
    }

    /// The counts of the loop's next round, its weighted rounds now `rounds`.
    #[inline(always)]
    fn next_round(self, rounds: u32) -> Self {
        let taken = self.taken();
        let taken = if taken < OUTPUT_BUDGET_AFTER_ROUNDS as u32 {
            taken + 1
        } else {
            taken
        };
        Self {
            rounds,
            entry: self.entry() | (taken << ENTRY_BITS),
            ..self
        }
    }
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
    /// A label's: stops this label's break, re-raises anything else. `stops` marks a label of
    /// the prelude's (`first`, `limit`, `nth`), the consumers that can stop a `repeat`.
    Label {
        id: u64,
        stops: bool,
    },
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
    /// `RANGE`: the next number, and how many it has produced.
    Range {
        cur: f64,
        max: f64,
        made: usize,
        resume: usize,
    },
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

/// The machine's state for one run of one program on one document.
struct Vm<'c, 'e> {
    it: &'c Interp<'e>,
    code: &'e Code,
    data: Stack<OwnedValue>,
    frames: Stack<FrameRec>,
    slots: Vec<Slot>,
    forks: Vec<Fork>,
    /// The charged capacities of `slots` and `forks` (as [`Stack::cap`]).
    slots_cap: usize,
    forks_cap: usize,
    /// A call's arguments on their way into the callee's slots, kept between calls.
    scratch: Vec<Slot>,
    limits: Limits,
    /// When the run next samples the process's memory ([`Limits::memory`]).
    sampler: Sampler,
    /// Whether a growth was refused for want of memory ([`Vm::growth_fits`]): the refusal is then
    /// the memory bound's, not the budget's.
    memory_over: bool,
    /// The instructions run, for the tests that pin how work scales.
    #[cfg(test)]
    steps: u64,
    /// Whether dead fork points are dropped ([`Vm::drop_dead_forks`]); off only for the
    /// differential test that dropping them changes no answer.
    #[cfg(test)]
    drops: bool,
}

/// Make room for one more element in `v` by doubling its charged capacity `cap`, or say it
/// cannot: when the bytes the control state holds while it grows (`used` now, plus the new
/// block, the old one still held until it is copied) pass `budget`, or the allocation fails.
/// Answers the bytes the charge grew by.
///
/// The charge is what a fresh vector would have grown to, so a run's bounds do not depend on
/// what an earlier document left in a kept vector ([`Spare`]): one kept with room allocates
/// nothing until the charge passes its capacity.
#[cold]
#[inline(never)]
fn grow<T>(v: &mut Vec<T>, cap: &mut usize, used: usize, budget: usize) -> Result<usize, ()> {
    let want = cap.saturating_mul(2).max(4);
    let size = core::mem::size_of::<T>();
    if used.saturating_add(want.saturating_mul(size)) > budget {
        return Err(());
    }
    if v.capacity() < want {
        move_to_capacity(v, want)?;
    }
    let grew = (want - *cap) * size;
    *cap = want;
    Ok(grew)
}

/// The bytes `v`'s allocation holds.
fn bytes_of<T>(v: &Vec<T>) -> usize {
    v.capacity().saturating_mul(core::mem::size_of::<T>())
}

/// Make room for one more value in an accumulator, fallibly: values the program builds are not
/// the budget's, but a failed allocation is still a refusal, not an abort.
#[cold]
#[inline(never)]
fn reserve<T>(items: &mut Vec<T>) -> Result<(), ()> {
    let want = items.capacity().saturating_mul(2).max(4);
    move_to_capacity(items, want)
}

/// Give `v` room for `want` elements, fallibly.
///
/// On macOS not by `realloc`: a large `realloc` there maps the old pages into the new block
/// lazily, and the process's footprint does not count them until they are next read, so the
/// memory bound read too little while the machine's own stacks and accumulators grew, and the run
/// passed its limit by up to a fifth when the refusal's unwinding read them (#8064). Moving the
/// elements into a new block writes them, so the footprint counts what the vector holds as soon as
/// it holds it; the copy costs a page fault per page the old block held, once per doubling. On
/// Linux a large `realloc` moves the pages (`mremap`), and the resident set counts them as it
/// moves them, so the vector is grown in place.
fn move_to_capacity<T>(v: &mut Vec<T>, want: usize) -> Result<(), ()> {
    if cfg!(target_vendor = "apple") {
        let mut grown = Vec::new();
        grown.try_reserve_exact(want).map_err(|_| ())?;
        grown.append(v);
        *v = grown;
        Ok(())
    } else {
        v.try_reserve_exact(want.saturating_sub(v.len()))
            .map_err(|_| ())
    }
}

/// The refusal past [`BUDGET`] or [`Limits::rounds`]: succinctly's own recursion-limit wording
/// (ADR-0025), naming the definition (`f/1 exceeded maximum recursion depth`).
fn refusal(name: &str, arity: usize) -> Raise {
    Raise::Fatal(named_depth_refusal(
        name.strip_prefix("_sp_").unwrap_or(name),
        arity,
    ))
}

/// The refusal of a run whose process holds more than its memory limit (#8064): its own wording,
/// naming the definition (`f/0: memory limit exceeded`), as nothing need recurse to reach it.
fn memory_refusal(name: &str, arity: usize) -> Raise {
    let name = name.strip_prefix("_sp_").unwrap_or(name);
    Raise::Fatal(EvalError::resource_limit(format!(
        "{}/{arity}: memory limit exceeded",
        super::super::resolve::ModuleRun::display_name(name),
    )))
}

/// The refusal of a zero-argument `def`'s tail loop past [`Limits::outputs`]: the shipped tail
/// loop's (`f/0: maximum outputs exceeded`, #5379).
fn loop_outputs_refusal(name: &str) -> Raise {
    Raise::Fatal(outputs_refusal(
        name.strip_prefix("_sp_").unwrap_or(name),
        0,
    ))
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

/// `range/3`'s first output, as the shipped route's `range` emits it: an integer literal's
/// spelling is dropped, unless it is `-0` or an integer a double cannot hold, which jq prints as
/// written. So `0 as $x | range($x; 3; 1) | -.` starts at `-0`, as in jq.
fn range_by_start(value: OwnedValue) -> OwnedValue {
    match value {
        OwnedValue::NumberLiteral(NumberRepr::Int(i), text)
            if !(i == 0 && text.starts_with('-'))
                && super::super::eval::jq_int_within_exact_f64_range(i) =>
        {
            OwnedValue::Int(i)
        }
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

/// The slots a leaf reads its free variables from, borrowed for the leaf's run (#7977 step 3):
/// the leaf's variables, by name and `(level, index)` from the frame it runs in.
pub(super) struct LeafView<'v> {
    vars: &'v [(Box<str>, u16, u16)],
    frames: &'v Stack<FrameRec>,
    slots: &'v [Slot],
}

impl LeafView<'_> {
    /// `LOADV` of the leaf's free variable `name`, or `None` when the leaf has none of that name
    /// (a name the leaf binds itself is found before the view is asked).
    pub(super) fn var(&self, name: &str) -> Option<OwnedValue> {
        let (_, level, idx) = self.vars.iter().find(|(n, ..)| **n == *name)?;
        let mut f = self.frames.index;
        for _ in 0..*level {
            f = self.frames.data[f as usize].value.env as isize;
        }
        let base = self.frames.data[f as usize].value.base as usize;
        Some(match &self.slots[base + *idx as usize] {
            Slot::Val(v) => v.clone(),
            _ => OwnedValue::Null,
        })
    }
}

/// A program the machine takes, compiled ([`decide`]): what [`run`] runs, on as many documents as
/// the caller keeps it for.
pub(super) struct Compiled {
    code: Code,
    /// [`hands_out_keys`] of the program, which [`Interp::prepare`] asks for each document.
    keys: bool,
    /// The stacks the last document's run left ([`Spare`]): nothing of a run's state, only
    /// allocations, which the next document's run reuses.
    spare: core::cell::Cell<Spare>,
}

impl Compiled {
    /// Whether the program can hand a key out ([`hands_out_keys`]), decided with the code.
    pub(super) fn hands_out_keys(&self) -> bool {
        self.keys
    }
}

/// Run `program`, compiled ([`decide`]), on `input`, a prepared document, on the machine,
/// handing each output to `on_value` (which answers whether it wants more).
///
/// Out of line and cold, so the shipped routes' frames do not grow by the machine's.
#[cold]
#[inline(never)]
pub(super) fn run<'e>(
    it: &Interp<'e>,
    program: &'e Compiled,
    input: OwnedValue,
    on_value: &mut dyn FnMut(OwnedValue) -> bool,
) -> R {
    let limits = Limits {
        memory: memory_probe(),
        ..SHIPPED
    };
    run_within(limits, it, program, input, on_value)
}

/// [`run`] within `limits`.
fn run_within<'e>(
    limits: Limits,
    it: &Interp<'e>,
    program: &'e Compiled,
    input: OwnedValue,
    on_value: &mut dyn FnMut(OwnedValue) -> bool,
) -> R {
    // The vectors an earlier document of the run left, if no other run of this code holds them
    // (a nested one starts empty).
    let spare = program.spare.take();
    let mut vm = Vm::with_spare(it, &program.code, limits, spare);
    let result = vm.exec(input, on_value);
    program.spare.set(vm.into_spare());
    result
}

/// The vectors a run leaves for the next document of the same code (#7977 step 5): empty, each
/// with the capacity it grew to, cut back to [`KEEP_BYTES`]. A per-record run then allocates its
/// stacks once. Nothing else carries over: a run's bounds, counts and state are its own
/// [`Vm`]'s, and the budget charges a kept vector's capacity only as the run grows into it
/// ([`grow`]).
#[derive(Default)]
pub(super) struct Spare {
    data: Vec<Block<OwnedValue>>,
    frames: Vec<Block<FrameRec>>,
    slots: Vec<Slot>,
    forks: Vec<Fork>,
    scratch: Vec<Slot>,
    /// When the next document's run samples the process's memory.
    sampler: Sampler,
}

/// The most a kept vector holds on to between documents, so that a document that grew its
/// stacks near the budget does not leave that memory held, uncharged, for the rest of the run.
const KEEP_BYTES: usize = 64 << 10;

/// Empty `v` for the next document, keeping at most [`KEEP_BYTES`] of its capacity.
fn keep<T>(mut v: Vec<T>) -> Vec<T> {
    v.clear();
    let most = KEEP_BYTES / core::mem::size_of::<T>().max(1);
    if v.capacity() > most {
        v.shrink_to(most);
    }
    v
}

impl<'c, 'e> Vm<'c, 'e> {
    #[cfg(test)]
    fn new(it: &'c Interp<'e>, code: &'e Code, limits: Limits) -> Self {
        Self::with_spare(it, code, limits, Spare::default())
    }

    /// A machine for one document, over the empty vectors `spare` (kept from an earlier
    /// document, or new).
    fn with_spare(it: &'c Interp<'e>, code: &'e Code, limits: Limits, spare: Spare) -> Self {
        Self {
            it,
            code,
            data: Stack::over(spare.data),
            frames: Stack::over(spare.frames),
            slots: spare.slots,
            forks: spare.forks,
            slots_cap: 0,
            forks_cap: 0,
            scratch: spare.scratch,
            limits,
            sampler: spare.sampler,
            memory_over: false,
            #[cfg(test)]
            steps: 0,
            #[cfg(test)]
            drops: true,
        }
    }

    /// The vectors for the next document: emptied, which drops every value, frame, slot and fork
    /// point this run left, whether it ended, stopped, refused or failed.
    fn into_spare(self) -> Spare {
        Spare {
            data: keep(self.data.data),
            frames: keep(self.frames.data),
            slots: keep(self.slots),
            forks: keep(self.forks),
            scratch: keep(self.scratch),
            sampler: self.sampler,
        }
    }

    /// A check point of the memory bound (#8064): one in every [`Sampler::interval`] samples the
    /// process's memory against [`Limits::memory`]'s limit. Asked at every call (a tail jump
    /// included), append and store, after every leaf, and after an operator or `setpath` that
    /// builds a container or a string: wherever the run can come to hold more.
    #[inline(always)]
    fn check_memory(&mut self) -> Result<(), Raise> {
        self.sampler.left -= 1;
        if self.sampler.left == 0 {
            return self.sample_memory();
        }
        Ok(())
    }

    /// [`Vm::check_memory`] for `v`, a value just built, when it can hold heap of its own.
    #[inline(always)]
    fn check_built(&mut self, v: &OwnedValue) -> Result<(), Raise> {
        match v {
            OwnedValue::Array(_) | OwnedValue::Object(_) | OwnedValue::String(_) => {
                self.check_memory()
            }
            _ => Ok(()),
        }
    }

    /// Read the process's memory, refuse past the limit, and set when to read it next. Without
    /// a probe no check point samples at all: the count is set as far off as it goes.
    #[cold]
    #[inline(never)]
    fn sample_memory(&mut self) -> Result<(), Raise> {
        let Some(probe) = self.limits.memory else {
            self.sampler.left = u32::MAX;
            return Ok(());
        };
        let Some(now) = (probe.sample)() else {
            self.sampler.left = MAX_INTERVAL;
            return Ok(());
        };
        if now > probe.limit {
            return Err(self.memory_refusal());
        }
        let s = &mut self.sampler;
        let next = Sampler::next_interval(s.last, now, s.interval, probe.limit);
        *s = Sampler {
            left: next,
            interval: next,
            last: now,
        };
        Ok(())
    }

    /// [`memory_refusal`] naming the innermost user definition on the call chain, or none.
    #[cold]
    #[inline(never)]
    fn memory_refusal(&self) -> Raise {
        let mut f = self.frames.index;
        while f >= 0 {
            let fr = self.frames.data[f as usize].value;
            let func = &self.code.funcs[fr.func as usize];
            if !func.name.is_empty() && !func.prelude {
                return memory_refusal(&func.name, func.arity);
            }
            f = self.frames.data[f as usize].next as isize;
        }
        Raise::Fatal(EvalError::resource_limit(String::from(
            "memory limit exceeded",
        )))
    }

    /// The bytes the control state holds, for [`BUDGET`]: the stacks, slots and fork points by
    /// the capacity this run has grown them to ([`grow`]).
    fn bytes(&self) -> usize {
        self.data.cap * core::mem::size_of::<Block<OwnedValue>>()
            + self.frames.cap * core::mem::size_of::<Block<FrameRec>>()
            + self.slots_cap * core::mem::size_of::<Slot>()
            + self.forks_cap * core::mem::size_of::<Fork>()
    }

    /// Push onto the data stack, growing it within the budget.
    #[inline(always)]
    fn push(&mut self, value: OwnedValue) -> Result<(), Raise> {
        if !self.data.has_room() {
            self.grow_data()?;
        }
        self.data.push(value);
        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn grow_data(&mut self) -> Result<(), Raise> {
        if !self.growth_fits(bytes_of(&self.data.data), false) {
            return Err(self.current_def());
        }
        let used = self.bytes();
        grow(
            &mut self.data.data,
            &mut self.data.cap,
            used,
            self.limits.bytes,
        )
        .map_err(|()| self.current_def())?;
        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn grow_frames(&mut self) -> Result<(), ()> {
        if !self.growth_fits(bytes_of(&self.frames.data), false) {
            return Err(());
        }
        let used = self.bytes();
        grow(
            &mut self.frames.data,
            &mut self.frames.cap,
            used,
            self.limits.bytes,
        )
        .map(|_| ())
    }

    #[cold]
    #[inline(never)]
    fn grow_slots(&mut self, room: usize) -> Result<(), ()> {
        while self.slots_cap - self.slots.len() < room {
            if !self.growth_fits(bytes_of(&self.slots), false) {
                return Err(());
            }
            let used = self.bytes();
            grow(
                &mut self.slots,
                &mut self.slots_cap,
                used,
                self.limits.bytes,
            )?;
        }
        Ok(())
    }

    /// Grow the frame stack and the slots, within the budget, so that a call's frame and its
    /// `nslots` slots fit without growing either: the budget check of a call. Whether they fit.
    #[inline(always)]
    fn room_for_call(&mut self, nslots: usize) -> bool {
        (self.frames.has_room() || self.grow_frames().is_ok())
            && (self.slots_cap - self.slots.len() >= nslots || self.grow_slots(nslots).is_ok())
    }

    /// Push a slot, growing the slots within the budget.
    #[inline(always)]
    fn push_slot(&mut self, slot: Slot) -> Result<(), Raise> {
        if self.slots.len() == self.slots_cap && self.grow_slots(1).is_err() {
            return Err(self.current_def());
        }
        self.slots.push(slot);
        Ok(())
    }

    /// `APPEND` into the accumulator in slot `at`, growing it fallibly.
    #[inline(always)]
    fn append(&mut self, at: usize, value: OwnedValue) -> Result<(), Raise> {
        self.check_memory()?;
        let Slot::Acc(items) = &mut self.slots[at] else {
            return Ok(());
        };
        if items.len() < items.capacity() {
            items.push(value);
            return Ok(());
        }
        let old = bytes_of(items);
        if !self.growth_fits(old, true) {
            return Err(self.current_def());
        }
        let Slot::Acc(items) = &mut self.slots[at] else {
            return Ok(());
        };
        if reserve(items).is_err() {
            return Err(self.current_def());
        }
        items.push(value);
        Ok(())
    }

    /// Whether the process has room, under [`Limits::memory`]'s limit, for a vector of `old`
    /// bytes to grow: its elements are moved into the new block while the old one is still held,
    /// so for that moment the process holds `old` bytes more (#8064). Asked of every growth of a
    /// stack or an accumulator, which is rare and moves the process's memory by more than any one
    /// check point does. An accumulator's refusal is the memory bound's (`memory`), with its
    /// wording; a control stack's is a deep recursion's, with the recursion wording, as past the
    /// control-state budget, unless the process was past the limit before the stack grew.
    #[cold]
    #[inline(never)]
    fn growth_fits(&mut self, old: usize, memory: bool) -> bool {
        let Some(probe) = self.limits.memory else {
            return true;
        };
        let Some(now) = (probe.sample)() else {
            return true;
        };
        if now.saturating_add(old) <= probe.limit {
            return true;
        }
        // A stack's growth is a deep recursion's only when its own old block is what does not
        // fit; a process already past the limit (a large input, say) is the memory bound's.
        self.memory_over = memory || now > probe.limit;
        false
    }

    #[cold]
    #[inline(never)]
    fn grow_forks(&mut self) -> Result<(), Raise> {
        if !self.growth_fits(bytes_of(&self.forks), false) {
            return Err(self.current_def());
        }
        let used = self.bytes();
        grow(
            &mut self.forks,
            &mut self.forks_cap,
            used,
            self.limits.bytes,
        )
        .map_err(|()| self.current_def())?;
        Ok(())
    }

    /// The refusal of a loop that started at `entry` past [`Limits::rounds`]: a prelude loop's
    /// own wording (`until: maximum iterations exceeded`), or the recursion wording naming the
    /// definition (the innermost user definition when the loop is another prelude one's).
    #[cold]
    #[inline(never)]
    fn loop_refusal(&self, entry: u32) -> Raise {
        let func = &self.code.funcs[entry as usize];
        match func.loop_name {
            Some(name) => Raise::Fatal(iterations_refusal(name)),
            None if func.prelude || func.name.is_empty() => self.current_def(),
            None => refusal(&func.name, func.arity),
        }
    }

    /// Whether the `repeat` starting now is eager, as the shipped evaluators tell it: the nearest
    /// consumer that can bound it is a `[...]` or a `reduce`, which take every output, rather
    /// than one of the prelude's labels (`first`, `limit`, `nth`), which can stop it, or the
    /// run's own output. The shipped evaluators stop an eager `repeat` after
    /// `MAX_REPEAT_ROUNDS` rounds and `REPEAT_WIDTH_BUDGET` outputs, and run any other
    /// unbounded (#2014).
    ///
    /// A label consumes the `repeat` only while the `repeat` runs inside its body. One whose
    /// body has already output (`first(1) | repeat(1)`) left a `TRY_END` fork point above its
    /// own, so the walk pairs each `TRY_END` with the `try` or label below it and passes both.
    fn repeat_is_eager(&self) -> bool {
        let mut ended = 0usize;
        for fork in self.forks.iter().rev() {
            match fork.kind {
                Kind::TryEnd => ended += 1,
                Kind::Try(_) if ended > 0 => ended -= 1,
                Kind::Try(Handler::Label { stops: true, .. }) => return false,
                Kind::Branch(at) if self.code.collects.binary_search(&at).is_ok() => return true,
                _ => {}
            }
        }
        false
    }

    /// The refusal naming the innermost user definition on the call chain (the prelude's are
    /// passed, as the shipped evaluators name the user's `f` in `def f: first(f); f`), or the
    /// shipped unnamed one when none is on it.
    #[cold]
    #[inline(never)]
    fn current_def(&self) -> Raise {
        if self.memory_over {
            return self.memory_refusal();
        }
        let mut f = self.frames.index;
        while f >= 0 {
            let fr = self.frames.data[f as usize].value;
            let func = &self.code.funcs[fr.func as usize];
            if !func.name.is_empty() && !func.prelude {
                return refusal(&func.name, func.arity);
            }
            f = self.frames.data[f as usize].next as isize;
        }
        Raise::Fatal(shared_arg_depth_refusal())
    }

    /// A fork point resuming `kind`, protecting what the stacks hold now (`stack_save`).
    #[inline(always)]
    fn save(&mut self, kind: Kind) -> Fork {
        Fork {
            kind,
            data: self.data.save(),
            frames: self.frames.save(),
            slots: self.slots.len(),
        }
    }

    /// Push a fork point, or refuse past [`BUDGET`].
    #[inline(always)]
    fn fork(&mut self, kind: Kind) -> Result<(), Raise> {
        if self.forks.len() == self.forks_cap {
            self.grow_forks()?;
        }
        // Built in place: a fork point is large, and is not passed through a call.
        let fork = self.save(kind);
        self.forks.push(fork);
        Ok(())
    }

    /// `frame_get_level`: the frame `level` static links out from the current one.
    fn frame_at(&self, level: u16) -> isize {
        let mut f = self.frames.index;
        for _ in 0..level {
            f = self.frames.data[f as usize].value.env as isize;
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
        self.frames.data[f as usize].value.base as usize + idx as usize
    }

    fn slot(&mut self, level: u16, idx: u16) -> &mut Slot {
        let f = self.frame_at(level);
        let base = self.frames.data[f as usize].value.base as usize;
        &mut self.slots[base + idx as usize]
    }

    /// `LOADV`: a copy of the variable at `(level, idx)`.
    fn value(&mut self, level: u16, idx: u16) -> OwnedValue {
        match self.slot(level, idx) {
            Slot::Val(v) => v.clone(),
            _ => OwnedValue::Null,
        }
    }

    /// `frame_push`, without its slots: the caller pushes those. `counts` are the loop's so far,
    /// and `exit` the frame its `RET` returns into when it is not the one below.
    #[inline(always)]
    fn push_frame(
        &mut self,
        func: u32,
        env: isize,
        (ret, exit): (u32, i32),
        counts: Counts,
    ) -> Result<(), Raise> {
        if !self.frames.has_room() && self.grow_frames().is_err() {
            return Err(self.current_def());
        }
        let base = self.slots.len() as u32;
        self.frames.push(FrameRec {
            func,
            counts,
            ret,
            exit,
            env: env as i32,
            base,
        });
        Ok(())
    }

    /// `frame_pop`: back to the frame the top one returns into, freeing what no fork point
    /// protects. A frame with an `exit` returns past the frames between, which are freed too.
    #[inline(always)]
    fn pop_frame(&mut self) -> FrameRec {
        let i = self.frames.index as usize;
        let free = self.frames.top_free();
        let fr = self.frames.data[i].value;
        if fr.exit < 0 {
            self.frames.index = self.frames.data[i].next as isize;
            if free {
                self.slots.truncate(fr.base as usize);
                self.frames.data.truncate(i);
            }
        } else {
            self.frames.index = fr.exit as isize;
            if free {
                // Every block above the target and the watermark is unreachable, and the
                // first of them holds the lowest slots of the frames being dropped.
                let keep = (self.frames.index.max(self.frames.limit) + 1) as usize;
                if keep < self.frames.data.len() {
                    self.slots
                        .truncate(self.frames.data[keep].value.base as usize);
                    self.frames.data.truncate(keep);
                }
            }
        }
        fr
    }

    /// Where a frame that takes over from the top one (a tail call's callee) returns into: the
    /// top one's own `exit`, or (`-1`) the frame below it, which is below the callee too.
    #[inline(always)]
    fn exit_of_top(&self) -> i32 {
        self.frames.data[self.frames.index as usize].value.exit
    }

    /// The dispatch loop (`jq_next`): run from the main function's entry until no fork point is
    /// left, an error reaches the top, or the consumer stops.
    fn exec(&mut self, input: OwnedValue, on_value: &mut dyn FnMut(OwnedValue) -> bool) -> R {
        let code = self.code;
        let main = &code.funcs[code.main as usize];
        self.push_frame(code.main, -1, (NO_RET, -1), Counts::fresh(code.main))?;
        for _ in 0..main.nslots {
            self.push_slot(Slot::Val(OwnedValue::Null))?;
        }
        self.push(input)?;
        let mut pc = main.entry;
        loop {
            #[cfg(test)]
            {
                self.steps += 1;
            }
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
                self.push(v.clone())?;
                self.push(v)?;
            }
            Insn::DupN => {
                let v = self.data.pop_n();
                self.push(v.clone())?;
                self.push(v)?;
            }
            Insn::Pop => {
                self.data.pop();
            }
            Insn::Swap => {
                let a = self.data.pop();
                let b = self.data.pop();
                self.push(a)?;
                self.push(b)?;
            }
            Insn::Const(k) => {
                self.data.pop();
                self.push(code.consts[*k as usize].clone())?;
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
                self.push(first)?;
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
                        made: 1,
                        resume: pc + 1,
                    })?;
                }
                self.push(range_start(start, &max))?;
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
            Insn::Label(s, stops) => {
                let id = self.it.next_label.get();
                self.it.next_label.set(id + 1);
                *self.slot(0, *s) = Slot::Val(OwnedValue::Int(id as i64));
                self.fork(Kind::Try(Handler::Label { id, stops: *stops }))?;
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
                // A binding, or a `reduce`'s or `foreach`'s state, is held from here on.
                self.check_memory()?;
            }
            Insn::Load(level, idx) => {
                self.data.pop();
                let v = self.value(*level, *idx);
                self.push(v)?;
            }
            Insn::LoadMove(level, idx) => {
                self.data.pop();
                let v = match self.slot(*level, *idx) {
                    Slot::Val(v) => core::mem::replace(v, OwnedValue::Null),
                    _ => OwnedValue::Null,
                };
                self.push(v)?;
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
                self.push(v)?;
            }
            Insn::Kill(s) => {
                let at = self.slot_at(0, *s);
                if at >= self.watermark() {
                    self.slots[at] = Slot::Val(OwnedValue::Null);
                }
            }
            Insn::NewAcc(s) => {
                let at = self.slot_at(0, *s);
                self.slots[at] = Slot::Acc(Vec::new());
            }
            Insn::Append(s) => {
                let v = self.data.pop();
                let at = self.slot_at(0, *s);
                self.append(at, v)?;
            }
            Insn::LoadAcc(s) => {
                self.data.pop();
                let at = self.slot_at(0, *s);
                let items = match &mut self.slots[at] {
                    Slot::Acc(items) => core::mem::take(items),
                    _ => Vec::new(),
                };
                self.push(OwnedValue::Array(items.into()))?;
            }
            Insn::Bin(op) => {
                let l = self.data.pop();
                let r = self.data.pop();
                let v = combine(*op, &l, &r).map_err(raise_eval)?;
                self.check_built(&v)?;
                self.push(v)?;
            }
            Insn::Cmp(op) => {
                let l = self.data.pop();
                let r = self.data.pop();
                self.push(OwnedValue::Bool(apply_compare_op::<JqSemantics>(
                    *op, &l, &r,
                )))?;
            }
            Insn::BinK(op, k) => {
                let l = self.data.pop();
                let r = &code.consts[*k as usize];
                let v = combine(*op, &l, r).map_err(raise_eval)?;
                self.check_built(&v)?;
                self.push(v)?;
            }
            Insn::CmpK(op, k) => {
                let l = self.data.pop();
                let r = &code.consts[*k as usize];
                self.push(OwnedValue::Bool(apply_compare_op::<JqSemantics>(
                    *op, &l, r,
                )))?;
            }
            Insn::ToBool => {
                let v = self.data.pop();
                self.push(OwnedValue::Bool(truthy(&v)))?;
            }
            Insn::Not => {
                let v = self.data.pop();
                self.push(OwnedValue::Bool(!truthy(&v)))?;
            }
            Insn::Negate => {
                let v = self.data.pop();
                self.push(arith_negate::<JqSemantics>(v).map_err(raise_eval)?)?;
            }
            Insn::Index => {
                let k = self.data.pop();
                let c = self.data.pop();
                let v = self.it.jv_get(&c, &k)?;
                self.push(v)?;
            }
            Insn::IndexK(k) => {
                let c = self.data.pop();
                let v = self.it.jv_get(&c, &code.consts[*k as usize])?;
                self.push(v)?;
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
                self.check_built(&written)?;
                self.push(written)?;
            }
            Insn::Call { callee, args, tail } => return self.call(pc, *callee, args, *tail),
            Insn::TailJump { func, level } => return self.tail_jump(pc, *func, *level),
            Insn::Ret => {
                // Outputs are counted in a loop's frames (and charged to an owner's), where a
                // bound can apply; a frame of an ordinary call that is not yet a loop counts none.
                let top = self.frames.index as usize;
                let counts = self.frames.data[top].value.counts;
                if counts.rounds != 0 || counts.owner != -1 {
                    self.count_output(top, counts.owner)?;
                }
                let fr = self.pop_frame();
                if fr.ret == NO_RET {
                    let v = self.data.pop();
                    return Ok(if on_value(v) { Flow::Back } else { Flow::Stop });
                }
                return Ok(Flow::Next(fr.ret as usize));
            }
        }
        Ok(next)
    }

    /// An output of the frame at `top`, charged to its loop, or to its owner's.
    #[inline(always)]
    fn count_output(&mut self, top: usize, owner: i32) -> Result<(), Raise> {
        let owner = if owner >= 0 { owner as usize } else { top };
        let counts = &mut self.frames.data[owner].value.counts;
        if counts.emitted < self.limits.outputs {
            counts.emitted += 1;
            Ok(())
        } else {
            self.charge_output(owner)
        }
    }

    /// An output charged to the loop of the frame at `owner`, once its count has reached
    /// [`Limits::outputs`] (or the loop is an uncounted `repeat`, whose `emitted` is above it):
    /// the shipped tail loop's output bound (#5379), checked before this output as the shipped
    /// loop checks after the one before; an eager `repeat`'s width (`REPEAT_WIDTH_BUDGET`, its
    /// count started that far below the bound); or an uncounted `repeat`'s round marked
    /// [`HEARD`].
    #[cold]
    #[inline(never)]
    fn charge_output(&mut self, owner: usize) -> Result<(), Raise> {
        let limit = self.limits.outputs;
        let counts = &mut self.frames.data[owner].value.counts;
        if counts.rounds == UNCOUNTED {
            counts.emitted |= HEARD;
            return Ok(());
        }
        if counts.emitted > limit {
            let entry = &self.code.funcs[counts.entry() as usize];
            if entry.loop_name == Some("range") {
                // `range/3`'s numbers: the round that would output the one past
                // `MAX_RANGE_TOTAL` is refused, as the shipped `range` counts what it emits.
                if counts.rounds >= self.limits.rounds {
                    return Err(Raise::Fatal(range_max_exceeded_error()));
                }
                return Ok(());
            }
            if entry.loop_name == Some("repeat") {
                return Err(Raise::Fatal(iterations_refusal("repeat")));
            }
            if self.limits.memory.is_some() {
                // With a memory probe no loop is refused for its outputs (#8064): what a
                // collector holds is the memory bound's. The count starts again, so this path is
                // taken once per `Limits::outputs` outputs.
                counts.emitted = 0;
                return Ok(());
            }
            if counts.taken() >= OUTPUT_BUDGET_AFTER_ROUNDS as u32 && entry.bounds_outputs() {
                return Err(loop_outputs_refusal(&entry.name));
            }
        }
        counts.emitted = counts.emitted.saturating_add(1);
        Ok(())
    }

    /// Run leaf `k` on `Interp::eval` over its free variables: the first output continues at
    /// once, the rest (and an error after them) become a [`Kind::Values`] fork point.
    #[inline(never)]
    fn leaf(&mut self, k: u32, pc: usize) -> Result<Flow, Raise> {
        let code = self.code;
        let leaf = &code.leaves[k as usize];
        let input = self.data.pop();
        // The free variables are read where they are, as the leaf reads them: no frame is
        // built for one, and none is copied unless the leaf reads it.
        let view = LeafView {
            vars: &leaf.vars,
            frames: &self.frames,
            slots: &self.slots,
        };
        let env = Env::over_slots(&view);
        let mut first = None;
        let mut rest = Vec::new();
        let mut full = false;
        let end = self
            .it
            .eval(&leaf.expr, &env, input, Reg::top(), &mut |v, _| {
                if first.is_none() {
                    first = Some(v);
                } else {
                    // Uncounted, as an accumulator is, but grown fallibly.
                    if rest.len() == rest.capacity() && reserve(&mut rest).is_err() {
                        full = true;
                        return Err(Raise::Stop);
                    }
                    rest.push(v);
                }
                Ok(())
            });
        if full {
            return Err(self.current_def());
        }
        let Some(v) = first else {
            return match end {
                Ok(()) => Ok(Flow::Back),
                Err(raise) => Err(raise),
            };
        };
        // What the leaf built is held from here on.
        self.check_memory()?;
        if !rest.is_empty() || end.is_err() {
            self.fork(Kind::Values {
                vals: rest.into_iter(),
                end: end.err(),
                resume: pc + 1,
            })?;
        }
        self.push(v)?;
        Ok(Flow::Next(pc + 1))
    }

    /// `CALL_JQ` / `TAIL_CALL_JQ`: bind the arguments into a new frame and jump to the
    /// callee's entry. A tail call pops the caller's frame first (it holds nothing the callee
    /// refers to, [`compile`]'s `mark_tails`), so a tail loop runs in constant frames. A tail call
    /// to a callee that closes over the caller's frame (`Tail::Keep`) keeps that frame, and the
    /// callee returns past it, so an output of a loop through a nested `def` returns at once
    /// rather than through every frame the loop has kept.
    #[inline(never)]
    fn call(
        &mut self,
        pc: usize,
        callee: Callee,
        args: &[ArgSpec],
        tail: Tail,
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
        if !self.room_for_call(f.nslots as usize) {
            return Err(self.call_refusal(func));
        }
        self.check_memory()?;
        let caller = self.frames.data[cur as usize].value;
        let counts = if tail == Tail::No && f.loop_name.is_none() {
            Counts::fresh(func)
        } else if tail == Tail::Pop
            && caller.func == func
            && caller.counts.rounds != UNCOUNTED
            && !f.binds
            && caller.counts.rounds < self.limits.rounds
        {
            // A self tail call: the next round of the loop, charged one.
            caller.counts.next_round(caller.counts.rounds + 1)
        } else {
            let declared = matches!(callee, Callee::Def { level: 0, .. });
            match self.loop_counts(func, tail, declared)? {
                Some(counts) => counts,
                // An uncounted `repeat` past its empty rounds: its stream ends, as on the
                // shipped routes.
                None => return Ok(Flow::Back),
            }
        };
        // The arguments, last first: values come off the data stack.
        let mut scratch = core::mem::take(&mut self.scratch);
        scratch.clear();
        for spec in args.iter().rev() {
            scratch.push(match *spec {
                ArgSpec::Value => Slot::Val(self.data.pop()),
                ArgSpec::RangeStart => Slot::Val(range_by_start(self.data.pop())),
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
        let link = match tail {
            Tail::No => {
                // The compiler declines a program of `u32::MAX` instructions or more.
                ((pc + 1) as u32, -1)
            }
            Tail::Pop => {
                // `TAIL_CALL_JQ`: the frame goes before its successor comes; nothing the new
                // frame holds refers to it. Only that frame goes: the callee may close over the
                // one below it, which an `exit` would return past.
                if !self.frames.top_free() && !self.forks.is_empty() {
                    self.drop_dead_forks();
                }
                let exit = self.exit_of_top();
                (self.pop_top_frame().ret, exit)
            }
            Tail::Keep => {
                let exit = if caller.exit >= 0 {
                    caller.exit
                } else {
                    self.frames.data[cur as usize].next
                };
                (caller.ret, exit)
            }
        };
        // `room_for_call` made room for the frame and its slots, so these pushes do not grow.
        let base = self.slots.len() as u32;
        self.frames.push(FrameRec {
            func,
            counts,
            ret: link.0,
            exit: link.1,
            env: env as i32,
            base,
        });
        while let Some(slot) = scratch.pop() {
            self.slots.push(slot);
        }
        for _ in args.len()..f.nslots as usize {
            self.slots.push(Slot::Val(OwnedValue::Null));
        }
        self.scratch = scratch;
        // The input may land above a protected block, so this one is checked.
        self.push(input)?;
        Ok(Flow::Next(f.entry))
    }

    /// [`Insn::TailJump`]: the self tail call of a zero-argument definition that makes no closure
    /// over its frame (#7977 step 4). When the call is the next round of the loop (as `call`'s
    /// self tail call reads it: counted, below the cap) and, once the dead fork points are
    /// dropped as `call` drops them, no fork point protects the frame, the round runs in the
    /// frame it is in: the state is the one `call` would reach by popping the frame and pushing
    /// its successor in the same place (the same function, static link, return address, `exit`
    /// and slot base, the loop's counts a round on, the slots fresh, the input on top), without
    /// building it. Anything else is `call`'s `Tail::Pop` call, which keeps the frame a fork
    /// point protects, and counts or refuses a round past the cap.
    #[inline(always)]
    fn tail_jump(&mut self, pc: usize, func: u32, level: u16) -> Result<Flow, Raise> {
        let call = |vm: &mut Self| vm.call(pc, Callee::Def { func, level }, &[], Tail::Pop);
        let code = self.code;
        let f = &code.funcs[func as usize];
        let nslots = f.nslots as usize;
        // `call` checks the room first, so a refusal past the budget is the same.
        if !self.room_for_call(nslots) {
            return Err(self.call_refusal(func));
        }
        self.check_memory()?;
        let top = self.frames.index as usize;
        let fr = self.frames.data[top].value;
        debug_assert_eq!(
            fr.func, func,
            "a tail jump runs in its own function's frame"
        );
        let rounds = fr.counts.rounds;
        if rounds == UNCOUNTED || rounds >= self.limits.rounds {
            return call(self);
        }
        if !self.frames.top_free() {
            if !self.forks.is_empty() {
                self.drop_dead_forks();
            }
            if !self.frames.top_free() {
                return call(self);
            }
        }
        // `call` would push the successor at `max(index, limit) + 1` once the frame is popped:
        // here, unless the frame sits above a gap the stack left.
        let below = self.frames.data[top].next as isize;
        if (below.max(self.frames.limit) + 1) as usize != top {
            return call(self);
        }
        self.frames.data.truncate(top + 1);
        self.frames.data[top].value.counts = fr.counts.next_round(rounds + 1);
        self.slots.truncate(fr.base as usize);
        for _ in 0..nslots {
            self.slots.push(Slot::Val(OwnedValue::Null));
        }
        // The input, which `call` pops and pushes again: where it is already when nothing lies
        // above it and it lies where the push would put it.
        let i = self.data.index;
        let next = self.data.data[i as usize].next as isize;
        if self.data.data.len() != i as usize + 1 || next.max(self.data.limit) + 1 != i {
            let input = self.data.pop();
            self.push(input)?;
        }
        Ok(Flow::Next(f.entry))
    }

    /// Drop the fork points at the top that are dead: ones a backtrack would pass without
    /// resuming any code and without changing what it carries. So a loop whose round left them
    /// does not keep a frame per round: an update `. + 1, empty`, `x // d` once `x` gave a truthy
    /// output, a finished `try` or label (`try (. + 1) catch 0`, `label $x | . + 1`), or
    /// `first(. + 1, 0)`, whose `break` ends its label. The shipped evaluators run these loops in
    /// constant memory too, stepping in place a round whose update answers once.
    ///
    /// A backtrack reaches the fork points at the top either with no error, when a later output
    /// is asked for, or with an error the code above them raises later. Read by
    /// [`Vm::backtrack`]'s arms, a run of fork points is dead when both come out of it as they
    /// went in, with no code resumed:
    /// - With no error, a `FORK` resuming at a `BACKTRACK`, at `//`'s check of a `found` slot
    ///   that is set, or at a `break` passes, the last raising the break through the fork points
    ///   below; `TRY_END` and a `try` pass.
    /// - An error, the break included, passes a `FORK`, `EACH`, `RANGE` or a leaf's outputs;
    ///   `TRY_END` wraps it and a `try` unwraps it again. A `try` it reaches unwrapped catches
    ///   it, or swallows it (no `catch`), or stops it (a label's own break).
    ///
    /// For an error raised later that last is a change, so a `try` is passed only with its
    /// `TRY_END` above it in the run: a `try` or label whose body has finished. For the break,
    /// its own label's `try` ending it, or a `try` without `catch` swallowing it, is the end the
    /// backtrack would reach. The longest run after which both readings are clean (no break in
    /// flight, every `TRY_END` unwrapped) is dropped, and the watermarks are lowered to what its
    /// lowest fork point saved: the state a backtrack through it would reach.
    #[cold]
    #[inline(never)]
    fn drop_dead_forks(&mut self) {
        #[cfg(test)]
        if !self.drops {
            return;
        }
        // The `TRY_END`s an error raised later has passed and no `try` has unwrapped.
        let mut wraps = 0u32;
        // The break a backtrack with no error raises on its way down: its label's id, and the
        // `TRY_END`s it has passed and no `try` has unwrapped.
        let mut raised: Option<(u64, u32)> = None;
        let mut cut = self.forks.len();
        for i in (0..self.forks.len()).rev() {
            let fork = &self.forks[i];
            match fork.kind {
                Kind::TryEnd => wraps += 1,
                Kind::Try(_) if wraps > 0 => wraps -= 1,
                Kind::Try(_) => break,
                _ => {}
            }
            match (&fork.kind, raised) {
                (Kind::Branch(at), None) => match self.code.insns[*at] {
                    Insn::Backtrack => {}
                    Insn::JumpIfSet(s, t)
                        if matches!(self.code.insns[t], Insn::Backtrack)
                            && matches!(self.slot_of(fork, 0, s), Some(Slot::Val(v)) if truthy(v)) =>
                        {}
                    Insn::Break(level, idx) => match self.slot_of(fork, level, idx) {
                        Some(Slot::Val(OwnedValue::Int(id))) => raised = Some((*id as u64, 0)),
                        _ => break,
                    },
                    _ => break,
                },
                (Kind::Each { .. } | Kind::Range { .. } | Kind::Values { .. }, None) => break,
                (_, None) => {}
                (Kind::TryEnd, Some((id, n))) => raised = Some((id, n + 1)),
                (Kind::Try(_), Some((id, n))) if n > 0 => raised = Some((id, n - 1)),
                (Kind::Try(Handler::None), Some(_)) => raised = None,
                (Kind::Try(Handler::Label { id: own, .. }), Some((id, _))) => {
                    if *own == id {
                        raised = None;
                    }
                }
                (Kind::Try(Handler::Code(_)), Some(_)) => break,
                (_, Some(_)) => {}
            }
            if wraps == 0 && raised.is_none() {
                cut = i;
            }
        }
        if let Some(fork) = self.forks.get(cut) {
            let (data, frames) = (fork.data.1, fork.frames.1);
            self.forks.truncate(cut);
            self.data.limit = data;
            self.frames.limit = frames;
        }
    }

    /// The slot `idx` of the frame `level` static links out from the one `fork` resumes in, as
    /// the instruction it resumes at would read it ([`Vm::slot`]), if there is one.
    fn slot_of(&self, fork: &Fork, level: u16, idx: u16) -> Option<&Slot> {
        let mut f = fork.frames.0;
        for _ in 0..level {
            f = self.frames.data.get(usize::try_from(f).ok()?)?.value.env as isize;
        }
        let base = self.frames.data.get(usize::try_from(f).ok()?)?.value.base as usize;
        self.slots.get(base + idx as usize)
    }

    /// The refusal of a call to `func` whose frame does not fit: named for it, or for the
    /// innermost user definition when it is a closure or a prelude definition.
    #[cold]
    #[inline(never)]
    fn call_refusal(&self, func: u32) -> Raise {
        if self.memory_over {
            return self.memory_refusal();
        }
        let f = &self.code.funcs[func as usize];
        if f.name.is_empty() || f.prelude {
            self.current_def()
        } else {
            refusal(&f.name, f.arity)
        }
    }

    /// `frame_pop` of the top frame alone, for a tail call: its `exit` passes to the callee.
    #[inline(always)]
    fn pop_top_frame(&mut self) -> FrameRec {
        let i = self.frames.index as usize;
        let free = self.frames.top_free();
        let fr = self.frames.data[i].value;
        self.frames.index = self.frames.data[i].next as isize;
        if free {
            self.slots.truncate(fr.base as usize);
            self.frames.data.truncate(i);
        }
        fr
    }

    /// The loop counts of the frame a call to `func` pushes ([`Counts`]), or a refusal past
    /// them; `None` ends an uncounted `repeat` past `MAX_EMPTY_REPEAT_ROUNDS` empty rounds.
    ///
    /// A tail call to the caller's own definition, to one it is nested in (a wrapper, the other
    /// half of a mutual recursion) or to one declared in it is the next round of the caller's
    /// loop: it inherits the loop's counts and is charged a round against `MAX_LOOP_ROUNDS`, as
    /// the shipped tail loop and `until`/`while` charge it, `BOUND_ROUND_WEIGHT` rounds for a
    /// definition that declares definitions of its own, as the shipped loop charges a round
    /// that binds a body. A tail call to a closure is part of the round it is in, and one to any
    /// other definition (a helper) starts a loop of its own, whose outputs are still charged to
    /// the caller's loop, as the shipped loop counts a helper's outputs in a comma in front of
    /// its call. A loop never runs through a closure (a closure argument makes its call an
    /// ordinary one). An ordinary call starts a loop of its own ([`Vm::loop_start`]).
    fn loop_counts(&self, func: u32, tail: Tail, declared: bool) -> Result<Option<Counts>, Raise> {
        let funcs = &self.code.funcs;
        let f = &funcs[func as usize];
        if tail == Tail::No || (tail == Tail::Keep && f.loop_name.is_some()) {
            return Ok(Some(self.loop_start(func)));
        }
        let top = self.frames.index;
        let caller = self.frames.data[top as usize].value;
        let c = caller.counts;
        let caller_named = !funcs[caller.func as usize].name.is_empty();
        // Where the outputs of a frame that is not one of the loop's rounds go.
        let owner = if c.owner >= 0 || !caller_named {
            c.owner
        } else if self.frames.top_free() {
            -1
        } else {
            top as i32
        };
        if f.name.is_empty() {
            return Ok(Some(Counts { owner, ..c }));
        }
        if c.rounds == UNCOUNTED {
            if f.loop_name == Some("repeat") {
                let empty = if c.emitted & HEARD == 0 {
                    (c.emitted & !QUIET) + 1
                } else {
                    0
                };
                if empty as usize >= MAX_EMPTY_REPEAT_ROUNDS {
                    return Ok(None);
                }
                return Ok(Some(Counts {
                    emitted: QUIET | empty,
                    owner: -1,
                    ..c
                }));
            }
        } else if caller_named
            && ((tail == Tail::Keep && declared)
                || caller.func == func
                || self.in_static_chain(func))
        {
            let weight = if f.binds {
                BOUND_ROUND_WEIGHT as u32
            } else {
                1
            };
            let rounds = c.rounds + weight;
            if rounds > self.limits.rounds {
                return Err(self.loop_refusal(c.entry()));
            }
            return Ok(Some(c.next_round(rounds)));
        }
        // The call that enters a loop is not one of its rounds, as the shipped tail loop
        // counts: `def f: if . >= 16777216 then . else . + 1 | f end; 0 | f` answers. Its
        // outputs are counted from the start all the same ([`COUNTED`]).
        Ok(Some(Counts {
            owner: if owner < 0 { COUNTED } else { owner },
            ..Counts::fresh(func)
        }))
    }

    /// The counts of a loop that starts at `func`. Four prelude loops start with the shipped
    /// evaluators' own caps, counting the round that starts here, as they count it: `until` and
    /// `while` with `MAX_LOOP_ROUNDS` rounds, `range/3`'s `while` with `MAX_RANGE_TOTAL`, an
    /// eager `repeat` with `MAX_REPEAT_ROUNDS` rounds and `REPEAT_WIDTH_BUDGET` outputs, and one a
    /// consumer can stop with neither ([`UNCOUNTED`]).
    fn loop_start(&self, func: u32) -> Counts {
        let f = &self.code.funcs[func as usize];
        let limits = self.limits;
        let start = |cap: usize| limits.rounds.saturating_sub(cap as u32).saturating_add(1);
        match f.loop_name {
            None => Counts::fresh(func),
            Some("repeat") if self.repeat_is_eager() => Counts {
                rounds: start(MAX_REPEAT_ROUNDS),
                emitted: limits
                    .outputs
                    .saturating_sub(REPEAT_WIDTH_BUDGET as u32 - 1),
                ..Counts::fresh(func)
            },
            Some("repeat") => Counts {
                rounds: UNCOUNTED,
                emitted: QUIET,
                ..Counts::fresh(func)
            },
            // Its rounds reach the cap at the round that would output the number past
            // `MAX_RANGE_TOTAL`, which `charge_output` refuses; a round whose condition fails
            // (the one after the last number) is not refused, as the shipped `range` does not
            // count it. Every output takes that path ([`QUIET`]).
            Some("range") => Counts {
                rounds: limits.rounds.saturating_sub(limits.range as u32),
                emitted: QUIET,
                ..Counts::fresh(func)
            },
            Some(_) => Counts {
                rounds: 1,
                ..Counts::fresh(func)
            },
        }
    }

    /// Whether the current frame, or one it is nested in, runs `func`.
    fn in_static_chain(&self, func: u32) -> bool {
        let mut f = self.frames.index;
        while f >= 0 {
            let frame = self.frames.data[f as usize].value;
            if frame.func == func {
                return true;
            }
            f = frame.env as isize;
        }
        false
    }

    /// `stack_restore` and the `ON_BACKTRACK` arms: the address to resume at, `None` when no
    /// fork point is left and nothing is raised.
    #[inline(never)]
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
                            self.push(caught(raise))?;
                            return Ok(Some(at));
                        }
                        Handler::Label { id, .. } => {
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
                            self.fork(Kind::Each {
                                container,
                                next: next + 1,
                                resume,
                            })?;
                        }
                        self.push(v)?;
                        return Ok(Some(resume));
                    }
                }
                Kind::Range {
                    cur,
                    max,
                    made,
                    resume,
                } => {
                    if err.is_none() {
                        // The shipped `range`'s cap (#5292): past `MAX_RANGE_TOTAL` numbers.
                        if made >= self.limits.range {
                            return Err(Raise::Fatal(range_max_exceeded_error()));
                        }
                        if cur + 1.0 < max || cur.is_nan() || max.is_nan() {
                            self.fork(Kind::Range {
                                cur: cur + 1.0,
                                max,
                                made: made + 1,
                                resume,
                            })?;
                        }
                        self.push(number_owned(cur))?;
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
                                    self.fork(Kind::Values { vals, end, resume })?;
                                }
                                self.push(v)?;
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

/// The function told of each routing decision and each compilation while the machine is on
/// ([`super::observe_call_machine`]).
#[cfg(feature = "std")]
static OBSERVER: std::sync::Mutex<Option<fn(CallMachineEvent)>> = std::sync::Mutex::new(None);

/// Install `observer` ([`super::observe_call_machine`]).
#[cfg(feature = "std")]
pub(super) fn set_observer(observer: Option<fn(CallMachineEvent)>) {
    if let Ok(mut slot) = OBSERVER.lock() {
        *slot = observer;
    }
}

/// The routing rule: a program with a self-recursive `def` whose every node [`compile`] accepts
/// (it declines the input builtins among the rest), answered with its code. The compiler is the
/// decliner, so there is one definition of what the machine runs.
///
/// A run whose route is pinned (`pin_route`) decides once, as its route is pinned, and keeps the
/// code with the pin for its documents; a program that is not pinned is decided per call.
///
/// Callers ask [`switched_on`] first: this compiles the program.
#[cold]
#[inline(never)]
pub(super) fn decide(program: &Expr) -> Option<Compiled> {
    let code = if self_recursive(program) {
        with_prelude(|prelude| compile::compile(&Interp::new(prelude), program).ok())
    } else {
        None
    };
    note(if code.is_some() {
        CallMachineEvent::Taken
    } else {
        CallMachineEvent::Declined
    });
    code.map(|code| Compiled {
        code,
        keys: hands_out_keys(program),
        spare: core::cell::Cell::default(),
    })
}

/// Tell the observer, if one is installed, of a decision or a compilation.
fn note(event: CallMachineEvent) {
    #[cfg(feature = "std")]
    if let Some(observer) = OBSERVER.lock().ok().and_then(|slot| *slot) {
        observer(event);
    }
    #[cfg(not(feature = "std"))]
    let _ = event;
}

#[cfg(test)]
std::thread_local! {
    /// The instructions [`Vm::step`] ran on this thread, by name, for the instruction test.
    static EXECUTED: core::cell::RefCell<alloc::collections::BTreeSet<&'static str>> =
        const { core::cell::RefCell::new(alloc::collections::BTreeSet::new()) };
}

#[cfg(test)]
mod tests;
