//! The call machine's compiler (#7977): the taken subset of [`Expr`], and the prelude bodies
//! the machine runs itself, compile once per program into one flat instruction vector, in the
//! shape of jq 1.7.1's `compile.c` (gojq's `compiler.go` is the same model):
//!
//! - every expression takes its input from the top of the data stack and leaves one output
//!   there, or backtracks; a generator leaves a fork point that yields the next output when
//!   resumed;
//! - a function (the main program, a `def` body, a closure argument) is a [`Func`] with slots
//!   for its parameters and locals; a variable, closure parameter or label is addressed by
//!   `(level, index)`, the level counting lexical frames out along static links
//!   (`frame_get_level`), resolved once here through a scope stack ([`Entry`]);
//! - a call names its callee as a closure (code, frame), and passes each closure parameter as a
//!   new closure over the current frame (a lambda) or a copy of one the caller holds (jq's
//!   `CLOSURE_REF`), so `def f(g): ... f(g)` stays a tail call;
//! - a `$` parameter whose bare name the body never calls is evaluated by the caller and
//!   passed as a value (it is `a as $a | body`, run at the call site), so such a call holds no
//!   closure over the caller's frame and can be a tail call;
//! - a call followed by `RET` that closes over nothing of the current frame is a tail call
//!   (`tail_call_analyze`);
//! - a read of a variable that nothing later in its scope reads is a [`Insn::LoadLast`], and a
//!   variable whose last reads sit where the order is not known is dropped after them
//!   ([`Insn::Kill`]): [`LastUse`]. jq moves only fold accumulators (`LOADVN`); moving every
//!   last read is what keeps a container uniquely held when `setpath` writes it (#7803).
//!
//! The compiler is also the router's decliner: a program it returns `Err` for runs on the
//! shipped routes, so what the machine runs has one definition.

use super::super::*;
use alloc::collections::BTreeSet;

/// The level of a prelude definition: it closes over no frame.
pub(super) const NO_LEVEL: u16 = u16::MAX;

/// The function a [`Insn::Call`] calls.
#[derive(Clone, Copy, Debug)]
pub(super) enum Callee {
    /// A `def`, closing over the frame `level` lexical frames out ([`NO_LEVEL`]: a prelude one).
    Def { func: u32, level: u16 },
    /// A closure parameter held in a slot.
    Param { level: u16, idx: u16 },
}

/// How a [`Insn::Call`] passes one argument.
#[derive(Clone, Copy, Debug)]
pub(super) enum ArgSpec {
    /// A value the caller pushed (a `$` parameter in value mode).
    Value,
    /// `range/3`'s `$from`, a value the caller pushed, passed as the shipped route's `range`
    /// emits its first output ([`super::range_by_start`]).
    RangeStart,
    /// A new closure: this code over the current frame.
    Lambda(u32),
    /// A copy of a closure the caller holds in a slot (jq's `CLOSURE_REF`).
    Ref { level: u16, idx: u16 },
}

/// Whether a [`Insn::Call`] is followed by `RET`, and what becomes of the caller's frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tail {
    /// An ordinary call: its `RET` returns to the instruction after it.
    No,
    /// `TAIL_CALL_JQ`: nothing the callee holds refers to the caller's frame, which goes first.
    Pop,
    /// A call followed by `RET` whose callee closes over the caller's frame (a `def` declared in
    /// it, or a closure argument made here): the frame stays while the callee runs, and the
    /// callee's `RET` returns where the caller's would have.
    Keep,
}

/// One instruction. Stack effects are written `[in] -> [out]`, top last.
#[derive(Debug)]
pub(super) enum Insn {
    /// `[v] -> [v, v]`.
    Dup,
    /// `DUPN`: as `Dup`, but a protected copy below is left `null` (jq's `stack_popn`).
    DupN,
    /// `[v] -> []`.
    Pop,
    /// `[a, b] -> [b, a]`.
    Swap,
    /// `LOADK`: `[v] -> [k]`.
    Const(u32),
    /// A call-free node run by `Interp::eval`: `[v] -> [out]`, more outputs as a fork.
    Leaf(u32),
    /// `EACH` / `EACH_OPT`: `[c] -> [elem]`, one per resumption.
    Each(bool),
    /// `RANGE`: `[start, end] -> [n]`, one per resumption.
    Range,
    /// `FORK`: resume at the target when backtracked into without an error.
    Fork(usize),
    Jump(usize),
    /// `[v] -> []`, jumping when `v` is false or null.
    JumpF(usize),
    /// Jump when the local slot holds a truthy value (`//`'s `found`).
    JumpIfSet(u16, usize),
    /// Set a local slot to a boolean.
    SetSlot(u16, bool),
    Backtrack,
    /// `TRY_BEGIN`: the handler's address (`None`: `try` without `catch`, or `?`).
    TryBegin(Option<usize>),
    /// `TRY_END`.
    TryEnd,
    /// `GENLABEL` into the slot, then a label's `TRY_BEGIN`; `true` for a label of the prelude's
    /// (`first`, `limit`, `nth`), a consumer that can stop a `repeat`.
    Label(u16, bool),
    /// `break $label`: raise the label held at `(level, index)`.
    Break(u16, u16),
    /// `STOREV` into a local slot: `[v] -> []`.
    Store(u16),
    /// `LOADV`: `[x] -> [$v]`.
    Load(u16, u16),
    /// `LOADVN`: `[x] -> [$v]`, leaving `null` in the slot.
    LoadMove(u16, u16),
    /// The last read of a variable: `[x] -> [$v]`, moved out of a slot no live fork point
    /// reaches (above the slots watermark), copied otherwise.
    LoadLast(u16, u16),
    /// A variable no later code reads (its last reads were inside a call's arguments or a
    /// leaf): drop its value if no live fork point reaches the slot.
    Kill(u16),
    /// A fresh accumulator for `[...]` in a local slot.
    NewAcc(u16),
    /// `APPEND`: `[v] -> []`.
    Append(u16),
    /// `[x] -> [array]`, moving the accumulator out.
    LoadAcc(u16),
    /// `[r, l] -> [l op r]`.
    Bin(ArithOp),
    /// `[r, l] -> [l op r]`.
    Cmp(CompareOp),
    /// `[v] -> [truthy(v)]`.
    ToBool,
    /// `[v] -> [not truthy(v)]`.
    Not,
    /// `[v] -> [-v]`.
    Negate,
    /// `[c, k] -> [c[k]]`.
    Index,
    /// `[c] -> [c[k]]`.
    IndexK(u32),
    /// `[v] -> raise error(v)`.
    Raise,
    /// `CALL_BUILTIN` of `setpath`: `[value, path, in] -> [in']`, the input moved in, so a
    /// uniquely held container is written in place.
    SetPath,
    /// `CALL_JQ` / `TAIL_CALL_JQ`: `[v1 .. vn, in] -> [out]`.
    Call {
        callee: Callee,
        args: Box<[ArgSpec]>,
        tail: Tail,
    },
    /// `RET`.
    Ret,
}

/// Every instruction's name, by an exhaustive match: a new variant does not compile until it is
/// named here, and the instruction test then fails until a row runs it.
#[cfg(test)]
pub(super) fn insn_name(insn: &Insn) -> &'static str {
    match insn {
        Insn::Dup => "Dup",
        Insn::DupN => "DupN",
        Insn::Pop => "Pop",
        Insn::Swap => "Swap",
        Insn::Const(_) => "Const",
        Insn::Leaf(_) => "Leaf",
        Insn::Each(_) => "Each",
        Insn::Range => "Range",
        Insn::Fork(_) => "Fork",
        Insn::Jump(_) => "Jump",
        Insn::JumpF(_) => "JumpF",
        Insn::JumpIfSet(..) => "JumpIfSet",
        Insn::SetSlot(..) => "SetSlot",
        Insn::Backtrack => "Backtrack",
        Insn::TryBegin(_) => "TryBegin",
        Insn::TryEnd => "TryEnd",
        Insn::Label(..) => "Label",
        Insn::Break(..) => "Break",
        Insn::Store(_) => "Store",
        Insn::Load(..) => "Load",
        Insn::LoadMove(..) => "LoadMove",
        Insn::LoadLast(..) => "LoadLast",
        Insn::Kill(_) => "Kill",
        Insn::NewAcc(_) => "NewAcc",
        Insn::Append(_) => "Append",
        Insn::LoadAcc(_) => "LoadAcc",
        Insn::Bin(_) => "Bin",
        Insn::Cmp(_) => "Cmp",
        Insn::ToBool => "ToBool",
        Insn::Not => "Not",
        Insn::Negate => "Negate",
        Insn::Index => "Index",
        Insn::IndexK(_) => "IndexK",
        Insn::Raise => "Raise",
        Insn::SetPath => "SetPath",
        Insn::Call { .. } => "Call",
        Insn::Ret => "Ret",
    }
}

/// A compiled function: the main program, a `def` body, a closure argument or a prelude body.
pub(super) struct Func<'e> {
    /// The definition's name (empty for the main program and for closure arguments).
    pub name: &'e str,
    pub arity: usize,
    /// The address of its first instruction.
    pub entry: usize,
    /// Its slots: parameters first, then locals.
    pub nslots: u16,
    /// Its lexical depth, from which a reference's level is the difference.
    depth: u16,
    /// Per parameter: whether the caller passes it as a value.
    value_params: Vec<bool>,
    /// Whether it is a prelude definition, or defined or passed inside one.
    pub prelude: bool,
    /// The loop the prelude's `until`, `while`, `repeat` or `range/3` runs as this function
    /// (`_until`, `_while`, `_repeat`): what a refusal past the loop cap names
    /// (`until: maximum iterations exceeded`, as the shipped loops say).
    pub loop_name: Option<&'static str>,
    /// For a prelude body: the loop name its nested definitions take ([`prelude_loop`]).
    nested_loop: Option<&'static str>,
    /// Whether its body declares definitions of its own: a loop round into it is charged
    /// `BOUND_ROUND_WEIGHT`, as the shipped tail loop charges a round that binds a body.
    pub binds: bool,
}

impl Func<'_> {
    /// Whether its tail loop's outputs are bounded (`MAX_LOOP_OUTPUTS`): a user's `def` with
    /// no parameters, the loops the shipped tail loop bounds (#5379).
    pub fn bounds_outputs(&self) -> bool {
        self.arity == 0 && !self.prelude && !self.name.is_empty()
    }
}

/// The loop a definition nested in the prelude's `name` runs: the shipped loops' own names. The
/// `while` that `range/3` runs is compiled apart and named `range` (#7977).
fn prelude_loop(name: &str) -> Option<&'static str> {
    match name {
        "_sp_until" => Some("until"),
        "_sp_while" => Some("while"),
        "_sp_repeat" => Some("repeat"),
        _ => None,
    }
}

/// A leaf: a call-free subtree `Interp::eval` runs, and where its free variables live.
pub(super) struct LeafCode<'e> {
    pub expr: &'e Expr,
    /// The free variables the leaf reads: name and `(level, index)`.
    pub vars: Vec<(&'e str, u16, u16)>,
}

/// A compiled program.
pub(super) struct Code<'e> {
    pub insns: Vec<Insn>,
    pub funcs: Vec<Func<'e>>,
    pub consts: Vec<OwnedValue>,
    pub leaves: Vec<LeafCode<'e>>,
    /// The main program's function.
    pub main: u32,
    /// The functions compiled from prelude bodies, for tests that look at the program's own code.
    #[cfg(test)]
    pub prelude_funcs: Vec<u32>,
    /// The resume addresses of the fork points of `[...]` and `reduce`, in ascending order: the
    /// consumers that take every output of what they run, as the shipped evaluators' eager
    /// `repeat` does (`Vm::repeat_is_eager`).
    pub collects: Vec<usize>,
}

/// A name in scope at compile time, with the function that owns its slot.
#[derive(Clone)]
enum Entry<'e> {
    /// `$name`: a value in slot `.2` of function `.1`.
    Var(&'e str, u32, u16),
    /// A closure parameter: a closure in slot `.2` of function `.1`.
    Param(&'e str, u32, u16),
    /// `def name/arity`, compiled as function `.2`, closing over function `.3` (`None`: a
    /// prelude definition, which closes over nothing).
    Def(&'e str, usize, u32, Option<u32>),
    /// `label $name`: the label's id in slot `.2` of function `.1`.
    Label(&'e str, u32, u16),
}

/// What a pending function compiles.
enum Body<'e> {
    Main(&'e Expr),
    Def(&'e [Param], &'e Expr),
    Lambda(&'e Expr),
}

/// A compile step: `Err` says why the machine declines the program.
type C = Result<(), String>;

struct Compiler<'c, 'e> {
    it: &'c Interp<'e>,
    code: Code<'e>,
    /// Functions still to compile, with the scope each sees.
    pending: Vec<(u32, Body<'e>, Vec<Entry<'e>>)>,
    /// The prelude definitions compiled so far, by name, arity and the loop name their nested
    /// definitions take when it is not their own (`range/3`'s `while`).
    prelude: BTreeMap<(&'e str, usize, Option<&'static str>), u32>,
    /// The function being compiled.
    cur: u32,
    scope: Vec<Entry<'e>>,
    /// The slots the current function has used so far.
    nslots: u16,
    /// `Var` nodes, by address, that are their binding's last read.
    last: BTreeSet<usize>,
    /// Nodes, by address, after which these local slots are dead.
    kills: BTreeMap<usize, Vec<u16>>,
    /// Whether last reads move ([`LastUse`]); off only in a test.
    moves: bool,
}

/// Compile `program`, or say why the machine declines it.
pub(super) fn compile<'e>(it: &Interp<'e>, program: &'e Expr) -> Result<Code<'e>, String> {
    compile_moving(it, program, true)
}

/// [`compile`] with no last-use moves (`moves: false`): every read a `Load`, no `Kill`, for the
/// differential test that the moves change no answer.
#[cfg(test)]
pub(super) fn compile_without_moves<'e>(
    it: &Interp<'e>,
    program: &'e Expr,
) -> Result<Code<'e>, String> {
    compile_moving(it, program, false)
}

fn compile_moving<'e>(it: &Interp<'e>, program: &'e Expr, moves: bool) -> Result<Code<'e>, String> {
    if any_subexpr(program, &mut declined_anywhere) {
        return Err(String::from(
            "a path form, `last`, `?//` or an input builtin",
        ));
    }
    let mut c = Compiler {
        it,
        code: Code {
            insns: Vec::new(),
            funcs: Vec::new(),
            consts: Vec::new(),
            leaves: Vec::new(),
            main: 0,
            #[cfg(test)]
            prelude_funcs: Vec::new(),
            collects: Vec::new(),
        },
        pending: Vec::new(),
        prelude: BTreeMap::new(),
        cur: 0,
        scope: Vec::new(),
        nslots: 0,
        last: BTreeSet::new(),
        kills: BTreeMap::new(),
        moves,
    };
    let main = c.new_func("", 0, 0, Vec::new());
    c.pending.push((main, Body::Main(program), Vec::new()));
    while let Some((f, body, scope)) = c.pending.pop() {
        c.func(f, body, scope)?;
    }
    if c.code.funcs.len() >= 1 << super::ENTRY_BITS {
        // A frame names its loop's entry in that many bits.
        return Err(String::from("too many functions"));
    }
    if c.code.insns.len() >= u32::MAX as usize {
        // A frame holds its return address in a `u32`.
        return Err(String::from("too many instructions"));
    }
    c.mark_tails();
    c.code.main = main;
    Ok(c.code)
}

/// What the machine declines anywhere in a program, prelude bodies included: everything the
/// path-mode interpreter declines (`declined_construct`: the shipped evaluator's own rewrites,
/// the extensions it does not run), and on top of that the forms the machine does not run yet:
/// the path forms (they need the path register in every fork point, #7977 step 8), and `last`,
/// `?//` and the input builtins (step 7).
fn declined_anywhere(e: &Expr) -> bool {
    declined_construct(e).is_some() || not_yet_the_machines(e)
}

/// The forms the path-mode interpreter runs and the machine does not, yet.
fn not_yet_the_machines(e: &Expr) -> bool {
    use Builtin as B;
    match e {
        Expr::Assign { .. }
        | Expr::Update { .. }
        | Expr::CompoundAssign { .. }
        | Expr::AlternativeAssign { .. }
        | Expr::LastExpr(_) => true,
        Expr::AsPattern { patterns, .. }
        | Expr::Reduce { patterns, .. }
        | Expr::Foreach { patterns, .. } => patterns.len() != 1,
        Expr::Builtin(b) => matches!(
            b,
            B::Path(_)
                | B::Paths
                | B::PathsFilter(_)
                | B::LeafPaths
                | B::Del(_)
                | B::Pick(_)
                | B::ToStream
                | B::LastStream(_)
                | B::GetPath(_)
                | B::Input
                | B::Inputs
                | B::InputLineNumber
        ),
        _ => false,
    }
}

/// What a leaf may not hold: a call (jq-defined builtins included), `.[]`, `range`, or the
/// control the machine runs itself.
fn blocks_leaf(e: &Expr) -> bool {
    match e {
        Expr::FuncCall { .. }
        | Expr::FuncDef { .. }
        | Expr::Iterate
        | Expr::Range { .. }
        | Expr::Label { .. }
        | Expr::Break(_)
        | Expr::Limit { .. }
        | Expr::FirstExpr(_)
        | Expr::LastExpr(_)
        | Expr::NthExpr { .. }
        | Expr::Until { .. }
        | Expr::While { .. }
        | Expr::Repeat(_)
        | Expr::RecursiveDescent => true,
        Expr::Var(name) => name.starts_with(LOADVN),
        Expr::Builtin(b) => prelude_call(b).is_some(),
        other => declined_anywhere(other),
    }
}

/// Whether `e` runs as one [`Insn::Leaf`]. A leaf runs to its end before its first output is
/// used, so it may not hold a side effect ([`is_effect`]) unless it is that builtin alone, with
/// none in its arguments: in a leaf that can answer more than once, the effect of a later output
/// would fire before the consumer had seen the earlier one, and past a consumer that stops.
fn is_leaf(e: &Expr) -> bool {
    if any_subexpr(e, &mut blocks_leaf) {
        return false;
    }
    let mut effects = 0;
    any_subexpr(e, &mut |n| {
        effects += usize::from(is_effect(n));
        false
    });
    effects == 0 || (effects == 1 && is_effect(e))
}

/// A C builtin that touches the outside world at the moment it runs: `debug` and `stderr`
/// write, `halt_error/1` writes its message before it raises, and `modulemeta` reports a module
/// it cannot load; yq's `load`, `system` and `eval` are here should they ever reach a jq-mode
/// program. `halt` only raises, and a leaf's raise waits for its earlier outputs; `halt_error/0`
/// is a prelude call, which no leaf holds; `input` and its kin are declined. The rest read only
/// the process's own state (`env`, `$ENV`, `now`, `input_filename`, `get_search_list`), so it
/// does not matter when in a leaf they run.
fn is_effect(e: &Expr) -> bool {
    use Builtin as B;
    matches!(
        e,
        Expr::Builtin(
            B::Debug
                | B::Stderr
                | B::HaltErrorCode(_)
                | B::ModuleMeta
                | B::Load(..)
                | B::System(..)
                | B::EvalExpr(..)
        )
    )
}

/// Names a leaf binds inside itself, and the names it reads.
fn leaf_names(e: &Expr) -> (Vec<String>, Vec<String>) {
    let mut inside: Vec<String> = Vec::new();
    let mut read: Vec<String> = Vec::new();
    any_subexpr(e, &mut |n| {
        let mut bound = Vec::new();
        match n {
            Expr::As { var, .. } => bound.push(var.as_str()),
            Expr::AsPattern { patterns, .. }
            | Expr::Reduce { patterns, .. }
            | Expr::Foreach { patterns, .. } => {
                for p in patterns {
                    pattern_vars(p, &mut bound);
                }
            }
            Expr::Var(name) if !read.iter().any(|r| r == name) => read.push(name.clone()),
            _ => {}
        }
        inside.extend(bound.into_iter().map(String::from));
        false
    });
    (inside, read)
}

impl<'e> Compiler<'_, 'e> {
    /// A new function, to be given its entry and slots when it is compiled.
    fn new_func(
        &mut self,
        name: &'e str,
        arity: usize,
        depth: u16,
        value_params: Vec<bool>,
    ) -> u32 {
        // A function made while a prelude body compiles belongs to the prelude.
        let outer = self.code.funcs.get(self.cur as usize);
        let prelude = outer.is_some_and(|f| f.prelude);
        let loop_name = outer.filter(|f| f.prelude).and_then(|f| f.nested_loop);
        self.code.funcs.push(Func {
            name,
            arity,
            entry: 0,
            nslots: 0,
            depth,
            value_params,
            prelude,
            loop_name: loop_name.filter(|_| arity == 0 && !name.is_empty()),
            nested_loop: None,
            binds: false,
        });
        (self.code.funcs.len() - 1) as u32
    }

    fn emit(&mut self, insn: Insn) -> usize {
        self.code.insns.push(insn);
        self.code.insns.len() - 1
    }

    fn here(&self) -> usize {
        self.code.insns.len()
    }

    /// Point the jump at `at` here.
    fn patch(&mut self, at: usize) {
        let here = self.here();
        match &mut self.code.insns[at] {
            Insn::Fork(t)
            | Insn::Jump(t)
            | Insn::JumpF(t)
            | Insn::JumpIfSet(_, t)
            | Insn::TryBegin(Some(t)) => *t = here,
            other => unreachable!("patching {other:?}"),
        }
    }

    /// A new local slot of the current function.
    fn slot(&mut self) -> Result<u16, String> {
        let s = self.nslots;
        self.nslots = self
            .nslots
            .checked_add(1)
            .ok_or_else(|| String::from("too many locals"))?;
        Ok(s)
    }

    fn konst(&mut self, value: OwnedValue) -> u32 {
        self.code.consts.push(value);
        (self.code.consts.len() - 1) as u32
    }

    fn depth(&self, f: u32) -> u16 {
        self.code.funcs[f as usize].depth
    }

    /// How many static links out from the current function `owner`'s frame is.
    fn level(&self, owner: u32) -> u16 {
        self.depth(self.cur) - self.depth(owner)
    }

    /// Compile function `f`'s body, ending in `RET`.
    fn func(&mut self, f: u32, body: Body<'e>, scope: Vec<Entry<'e>>) -> C {
        self.cur = f;
        self.scope = scope;
        self.nslots = 0;
        self.code.funcs[f as usize].entry = self.here();
        match body {
            Body::Main(e) | Body::Lambda(e) => self.expr(e)?,
            Body::Def(params, body) => {
                if any_subexpr(body, &mut declined_anywhere) {
                    return Err(String::from("a prelude body holding a declined form"));
                }
                self.code.funcs[f as usize].binds =
                    any_subexpr(body, &mut |e| matches!(e, Expr::FuncDef { .. }));
                self.nslots = u16::try_from(params.len()).map_err(|_| "too many parameters")?;
                let value = self.code.funcs[f as usize].value_params.clone();
                let mut binds = Vec::new();
                for (i, p) in params.iter().enumerate() {
                    let i = i as u16;
                    match p {
                        Param::Bare(n) => self.scope.push(Entry::Param(n, f, i)),
                        Param::Dollar(n) if value[i as usize] => {
                            self.scope.push(Entry::Var(n, f, i));
                        }
                        Param::Dollar(n) => {
                            self.scope.push(Entry::Param(n, f, i));
                            binds.push((n.as_str(), i));
                        }
                    }
                }
                // `def f($a): body` is `def f(a): a as $a | body`.
                for (i, p) in params.iter().enumerate() {
                    if let Param::Dollar(n) = p {
                        if value[i] {
                            self.last_uses(n, i as u16, body);
                        }
                    }
                }
                for (n, i) in binds {
                    let s = self.slot()?;
                    self.last_uses(n, s, body);
                    self.emit(Insn::Dup);
                    self.emit(Insn::Call {
                        callee: Callee::Param { level: 0, idx: i },
                        args: Box::new([]),
                        tail: Tail::No,
                    });
                    self.emit(Insn::Store(s));
                    self.scope.push(Entry::Var(n, f, s));
                }
                self.expr(body)?;
            }
        }
        self.emit(Insn::Ret);
        self.code.funcs[f as usize].nslots = self.nslots;
        Ok(())
    }

    fn find_var(&self, name: &str) -> Option<(u16, u16)> {
        self.scope.iter().rev().find_map(|e| match e {
            Entry::Var(n, f, s) if *n == name => Some((self.level(*f), *s)),
            _ => None,
        })
    }

    fn find_var_named(&self, name: &str) -> Option<(&'e str, u16, u16)> {
        self.scope.iter().rev().find_map(|e| match e {
            Entry::Var(n, f, s) if *n == name => Some((*n, self.level(*f), *s)),
            _ => None,
        })
    }

    fn find_label(&self, name: &str) -> Option<(u16, u16)> {
        self.scope.iter().rev().find_map(|e| match e {
            Entry::Label(n, f, s) if *n == name => Some((self.level(*f), *s)),
            _ => None,
        })
    }

    fn find_func(&self, name: &str, arity: usize) -> Option<Entry<'e>> {
        self.scope
            .iter()
            .rev()
            .find(|e| match e {
                Entry::Def(n, a, ..) => *n == name && *a == arity,
                Entry::Param(n, ..) => *n == name && arity == 0,
                _ => false,
            })
            .cloned()
    }

    /// A closure parameter passed on as it is.
    fn param_ref(&self, arg: &Expr) -> Option<(u16, u16)> {
        match arg {
            Expr::Paren(inner) => self.param_ref(inner),
            Expr::FuncCall { name, args, .. } if args.is_empty() => {
                match self.find_func(name, 0)? {
                    Entry::Param(_, f, s) => Some((self.level(f), s)),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// A closure argument: a function over the current frame, compiled later.
    fn lambda(&mut self, arg: &'e Expr) -> Result<u32, String> {
        let depth = self.inner_depth()?;
        let f = self.new_func("", 0, depth, Vec::new());
        self.pending
            .push((f, Body::Lambda(arg), self.scope.clone()));
        Ok(f)
    }

    /// The lexical depth of a function nested in the current one.
    fn inner_depth(&self) -> Result<u16, String> {
        self.depth(self.cur)
            .checked_add(1)
            .ok_or_else(|| String::from("definitions nested too deep"))
    }

    /// A call: value arguments evaluated here, the first outermost (`def f($a; $b)` binds as
    /// `a as $a | b as $b | ...`), closure arguments passed as lambdas or references.
    fn call_func(&mut self, callee: Callee, func: u32, args: &[&'e Expr]) -> C {
        let modes = self.code.funcs[func as usize].value_params.clone();
        if modes.len() != args.len() {
            return Err(String::from("arity mismatch"));
        }
        let mut specs = Vec::with_capacity(args.len());
        for (&arg, value) in args.iter().zip(modes) {
            if value {
                self.emit(Insn::Dup);
                self.expr(arg)?;
                self.emit(Insn::Swap);
                specs.push(ArgSpec::Value);
            } else if let Some((level, idx)) = self.param_ref(arg) {
                specs.push(ArgSpec::Ref { level, idx });
            } else {
                let l = self.lambda(arg)?;
                specs.push(ArgSpec::Lambda(l));
            }
        }
        self.emit(Insn::Call {
            callee,
            args: specs.into_boxed_slice(),
            tail: Tail::No,
        });
        Ok(())
    }

    /// Per parameter, whether it is passed by value: a `$` parameter, unless the body calls
    /// one of the `$` parameters by its bare name (then all keep closures, as jq binds them).
    fn value_modes(params: &[Param], body: &Expr) -> Vec<bool> {
        let dollars: Vec<&str> = params
            .iter()
            .filter_map(|p| match p {
                Param::Dollar(n) => Some(n.as_str()),
                Param::Bare(_) => None,
            })
            .collect();
        let bare_use = !dollars.is_empty()
            && any_subexpr(body, &mut |e| {
                matches!(e, Expr::FuncCall { name, args, .. }
                    if args.is_empty() && dollars.contains(&name.as_str()))
            });
        params
            .iter()
            .map(|p| matches!(p, Param::Dollar(_)) && !bare_use)
            .collect()
    }

    /// The function of the prelude definition `name/arity`, queued for compiling on first use.
    /// `tag` names the loop its nested definitions run, when it is not the definition's own.
    fn prelude_func(
        &mut self,
        name: &str,
        arity: usize,
        tag: Option<&'static str>,
    ) -> Result<u32, String> {
        let prelude: &'e Prelude = self.it.prelude;
        let def = prelude
            .find(name, arity)
            .ok_or_else(|| format!("no definition of {name}/{arity}"))?;
        if let Some(&f) = self.prelude.get(&(def.name, arity, tag)) {
            return Ok(f);
        }
        let Expr::FuncDef {
            name, params, body, ..
        } = def.expr()
        else {
            return Err(String::from("a prelude definition that is not a def"));
        };
        let f = self.new_func(name, arity, 0, Self::value_modes(params, body));
        let func = &mut self.code.funcs[f as usize];
        func.prelude = true;
        func.loop_name = None;
        func.nested_loop = tag.or_else(|| prelude_loop(def.name));
        self.prelude.insert((def.name, arity, tag), f);
        #[cfg(test)]
        self.code.prelude_funcs.push(f);
        self.pending.push((f, Body::Def(params, body), Vec::new()));
        Ok(f)
    }

    fn prelude_call(&mut self, name: &str, args: &[&'e Expr]) -> C {
        self.prelude_call_tagged(name, args, None)
    }

    /// [`Compiler::prelude_call`] of an instance whose nested loop is named `tag`.
    fn prelude_call_tagged(
        &mut self,
        name: &str,
        args: &[&'e Expr],
        tag: Option<&'static str>,
    ) -> C {
        let f = self.prelude_func(name, args.len(), tag)?;
        self.call_func(
            Callee::Def {
                func: f,
                level: NO_LEVEL,
            },
            f,
            args,
        )
    }

    /// `e` as a [`Insn::Leaf`], its free variables resolved to slots.
    fn leaf(&mut self, e: &'e Expr) -> C {
        if !is_leaf(e) {
            return Err(format!(
                "a call, a generator the machine runs, or a side effect inside {}",
                variant_name(e)
            ));
        }
        let (inside, read) = leaf_names(e);
        let mut vars = Vec::new();
        for name in &read {
            match self.find_var_named(name) {
                Some(found) => vars.push(found),
                None if inside.contains(name) => {}
                None => return Err(format!("${name} is not bound")),
            }
        }
        self.code.leaves.push(LeafCode { expr: e, vars });
        let k = (self.code.leaves.len() - 1) as u32;
        self.emit(Insn::Leaf(k));
        Ok(())
    }

    /// A destructuring pattern (`gen_array_matcher`, `gen_object_matcher`): `[v] -> []`,
    /// storing its variables in new slots. They come into scope only once the whole pattern is
    /// compiled ([`Self::bind_pattern`]): a key expression (`{a: $x, ($x): $y}`) sees the scope
    /// outside the pattern, as in jq, where `$x` there is not the pattern's own.
    fn pattern(&mut self, p: &'e Pattern, pending: &mut Vec<(&'e str, u16)>) -> C {
        match p {
            Pattern::Var(name) => {
                let s = self.slot()?;
                self.emit(Insn::Store(s));
                Self::pend_pattern_var(pending, name, s);
            }
            Pattern::Array(elems) => {
                // `gen_array_matcher`: the last element first.
                for (i, sub) in elems.iter().enumerate().rev() {
                    self.emit(Insn::Dup);
                    let k = self.konst(OwnedValue::Int(i as i64));
                    self.emit(Insn::IndexK(k));
                    self.pattern(sub, pending)?;
                }
                self.emit(Insn::Pop);
            }
            Pattern::Object(entries) => {
                for entry in entries {
                    self.emit(Insn::Dup);
                    match &entry.key {
                        ObjectKey::Literal(s) => {
                            let k = self.konst(string(s));
                            self.emit(Insn::IndexK(k));
                        }
                        ObjectKey::Expr(key) => {
                            self.emit(Insn::Dup);
                            self.expr(key)?;
                            self.emit(Insn::Index);
                        }
                        ObjectKey::Bare => return Err(String::from("a bare pattern key")),
                    }
                    if let Some(name) = &entry.bind {
                        let s = self.slot()?;
                        self.emit(Insn::Dup);
                        self.emit(Insn::Store(s));
                        Self::pend_pattern_var(pending, name, s);
                    }
                    self.pattern(&entry.pattern, pending)?;
                }
                self.emit(Insn::Pop);
            }
        }
        Ok(())
    }

    /// Note `$name`, stored in slot `s`, for [`Self::bind_pattern`], unless the pattern has
    /// stored it already. A name bound twice in one pattern is the first store that runs, as in
    /// jq: the last element of an array pattern runs first, an object pattern's entries in order
    /// (`[1,2] as [$a, $a] | $a` is `2`, `{"a":1,"b":2} as {a: $x, b: $x} | $x` is `1`). The
    /// later stores still run, for their errors.
    fn pend_pattern_var(pending: &mut Vec<(&'e str, u16)>, name: &'e str, s: u16) {
        if !pending.iter().any(|(n, _)| *n == name) {
            pending.push((name, s));
        }
    }

    /// Compile pattern `p` and bring its variables into scope.
    fn bind_pattern(&mut self, p: &'e Pattern) -> C {
        let mut pending = Vec::new();
        self.pattern(p, &mut pending)?;
        for (name, s) in pending {
            self.scope.push(Entry::Var(name, self.cur, s));
        }
        Ok(())
    }

    /// A binary operator: the right operand runs first, as in jq (`gen_binop`).
    fn binop(&mut self, left: &'e Expr, right: &'e Expr, op: Insn) -> C {
        self.emit(Insn::Dup);
        self.expr(right)?;
        self.emit(Insn::Swap);
        self.expr(left)?;
        self.emit(op);
        Ok(())
    }

    /// Compile `e`: `[in] -> [out]`, or a backtrack.
    fn expr(&mut self, e: &'e Expr) -> C {
        if let Some(value) = self.it.constant(e) {
            let k = self.konst(value);
            self.emit(Insn::Const(k));
            return Ok(());
        }
        match e {
            Expr::Identity => {}
            Expr::Paren(inner) => self.expr(inner)?,
            Expr::Pipe(stages) => {
                for s in stages {
                    self.expr(s)?;
                }
            }
            Expr::Comma(items) => match items.split_last() {
                None => {
                    self.emit(Insn::Backtrack);
                }
                Some((last, init)) => {
                    let mut ends = Vec::new();
                    for item in init {
                        let f = self.emit(Insn::Fork(0));
                        self.expr(item)?;
                        ends.push(self.emit(Insn::Jump(0)));
                        self.patch(f);
                    }
                    self.expr(last)?;
                    for j in ends {
                        self.patch(j);
                    }
                }
            },
            Expr::Arithmetic {
                op, left, right, ..
            } => self.binop(left, right, Insn::Bin(*op))?,
            Expr::Compare { op, left, right } => self.binop(left, right, Insn::Cmp(*op))?,
            Expr::And(lhs, rhs) => {
                self.emit(Insn::Dup);
                self.expr(lhs)?;
                let f = self.emit(Insn::JumpF(0));
                self.expr(rhs)?;
                self.emit(Insn::ToBool);
                let j = self.emit(Insn::Jump(0));
                self.patch(f);
                let k = self.konst(OwnedValue::Bool(false));
                self.emit(Insn::Const(k));
                self.patch(j);
            }
            Expr::Or(lhs, rhs) => {
                self.emit(Insn::Dup);
                self.expr(lhs)?;
                let f = self.emit(Insn::JumpF(0));
                let k = self.konst(OwnedValue::Bool(true));
                self.emit(Insn::Const(k));
                let j = self.emit(Insn::Jump(0));
                self.patch(f);
                self.expr(rhs)?;
                self.emit(Insn::ToBool);
                self.patch(j);
            }
            Expr::Not => {
                self.emit(Insn::Not);
            }
            Expr::Negate(inner) if !is_leaf(e) => {
                self.expr(inner)?;
                self.emit(Insn::Negate);
            }
            // `gen_definedor`.
            Expr::Alternative(a, b) => {
                let found = self.slot()?;
                self.emit(Insn::SetSlot(found, false));
                let f = self.emit(Insn::Fork(0));
                self.expr(a)?;
                self.emit(Insn::Dup);
                let back = self.emit(Insn::JumpF(0));
                self.emit(Insn::SetSlot(found, true));
                let out = self.emit(Insn::Jump(0));
                self.patch(back);
                let back_at = self.emit(Insn::Backtrack);
                self.patch(f);
                self.emit(Insn::JumpIfSet(found, back_at));
                self.expr(b)?;
                self.patch(out);
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.emit(Insn::Dup);
                self.expr(cond)?;
                let f = self.emit(Insn::JumpF(0));
                self.expr(then_branch)?;
                let j = self.emit(Insn::Jump(0));
                self.patch(f);
                self.expr(else_branch)?;
                self.patch(j);
            }
            Expr::Try { expr: body, catch } => self.try_(body, catch.as_deref())?,
            Expr::Optional(inner) => match &**inner {
                Expr::Iterate => {
                    self.emit(Insn::Each(true));
                }
                _ if is_leaf(e) => self.leaf(e)?,
                _ => self.try_(inner, None)?,
            },
            Expr::Error(None) => {
                self.emit(Insn::Raise);
            }
            Expr::Error(Some(msg)) => {
                self.expr(msg)?;
                self.emit(Insn::Raise);
            }
            Expr::Iterate => {
                self.emit(Insn::Each(false));
            }
            // `gen_reduce`.
            Expr::Reduce {
                input: source,
                patterns,
                init,
                update,
            } => {
                let mark = self.scope.len();
                self.emit(Insn::Dup);
                self.expr(init)?;
                let acc = self.slot()?;
                self.emit(Insn::Store(acc));
                let f = self.emit(Insn::Fork(0));
                self.emit(Insn::DupN);
                self.expr(source)?;
                self.bind_pattern(&patterns[0])?;
                self.emit(Insn::LoadMove(0, acc));
                self.expr(update)?;
                self.emit(Insn::Store(acc));
                self.emit(Insn::Backtrack);
                self.code.collects.push(self.here());
                self.patch(f);
                self.emit(Insn::LoadMove(0, acc));
                self.scope.truncate(mark);
            }
            // `gen_foreach`.
            Expr::Foreach {
                input: source,
                patterns,
                init,
                update,
                extract,
            } => {
                let mark = self.scope.len();
                self.emit(Insn::Dup);
                self.expr(init)?;
                let state = self.slot()?;
                self.emit(Insn::Store(state));
                let f = self.emit(Insn::Fork(0));
                self.emit(Insn::DupN);
                self.expr(source)?;
                self.bind_pattern(&patterns[0])?;
                self.emit(Insn::LoadMove(0, state));
                self.expr(update)?;
                self.emit(Insn::Dup);
                self.emit(Insn::Store(state));
                if let Some(x) = extract {
                    self.expr(x)?;
                }
                let out = self.emit(Insn::Jump(0));
                self.patch(f);
                self.emit(Insn::Backtrack);
                self.patch(out);
                self.scope.truncate(mark);
            }
            // `gen_collect`.
            Expr::Array(inner) => {
                let acc = self.slot()?;
                self.emit(Insn::NewAcc(acc));
                let f = self.emit(Insn::Fork(0));
                self.expr(inner)?;
                self.emit(Insn::Append(acc));
                self.emit(Insn::Backtrack);
                self.code.collects.push(self.here());
                self.patch(f);
                self.emit(Insn::LoadAcc(acc));
            }
            // `gen_label`: a `try` whose handler stops this label's break.
            Expr::Label { name, body } => {
                let s = self.slot()?;
                let stops = self.code.funcs[self.cur as usize].prelude;
                self.emit(Insn::Label(s, stops));
                self.scope.push(Entry::Label(name, self.cur, s));
                self.expr(body)?;
                self.scope.pop();
                self.emit(Insn::TryEnd);
            }
            Expr::Break(name) => {
                let (level, idx) = self
                    .find_label(name)
                    .ok_or_else(|| format!("$*label-{name} is not defined"))?;
                self.emit(Insn::Break(level, idx));
            }
            Expr::FuncDef {
                name,
                params,
                body,
                then,
                ..
            } => {
                let depth = self.inner_depth()?;
                let f = self.new_func(name, params.len(), depth, Self::value_modes(params, body));
                self.scope
                    .push(Entry::Def(name, params.len(), f, Some(self.cur)));
                self.pending
                    .push((f, Body::Def(params, body), self.scope.clone()));
                self.expr(then)?;
                self.scope.pop();
            }
            Expr::FuncCall {
                name,
                args,
                builtin_fallback,
            } => match self.find_func(name, args.len()) {
                Some(Entry::Def(_, _, func, owner)) => {
                    let level = owner.map_or(NO_LEVEL, |o| self.level(o));
                    let args: Vec<&'e Expr> = args.iter().collect();
                    self.call_func(Callee::Def { func, level }, func, &args)?;
                }
                Some(Entry::Param(_, f, idx)) => {
                    let level = self.level(f);
                    self.emit(Insn::Call {
                        callee: Callee::Param { level, idx },
                        args: Box::new([]),
                        tail: Tail::No,
                    });
                }
                _ => match builtin_fallback {
                    Some(fallback) => self.expr(fallback)?,
                    None => {
                        let args: Vec<&'e Expr> = args.iter().collect();
                        self.prelude_call(name, &args)?;
                    }
                },
            },
            Expr::Var(name) if !name.starts_with(LOADVN) => {
                let (level, idx) = self
                    .find_var(name)
                    .ok_or_else(|| format!("${name} is not bound"))?;
                if level == 0 && self.last.contains(&(e as *const Expr as usize)) {
                    self.emit(Insn::LoadLast(level, idx));
                } else {
                    self.emit(Insn::Load(level, idx));
                }
            }
            Expr::As {
                expr: source,
                var,
                body,
            } if !is_bound_inside(var, body) => {
                self.emit(Insn::Dup);
                self.expr(source)?;
                let s = self.slot()?;
                self.emit(Insn::Store(s));
                self.scope.push(Entry::Var(var, self.cur, s));
                self.last_uses(var, s, body);
                self.expr(body)?;
                self.scope.pop();
            }
            Expr::AsPattern {
                expr: source,
                patterns,
                body,
            } if patterns.len() == 1 => {
                let mark = self.scope.len();
                self.emit(Insn::Dup);
                self.expr(source)?;
                self.bind_pattern(&patterns[0])?;
                let bound: Vec<(&str, u16)> = self.scope[mark..]
                    .iter()
                    .filter_map(|entry| match entry {
                        Entry::Var(n, _, s) => Some((*n, *s)),
                        _ => None,
                    })
                    .collect();
                for (n, s) in bound {
                    self.last_uses(n, s, body);
                }
                self.expr(body)?;
                self.scope.truncate(mark);
            }
            Expr::Limit { n, expr: g } => self.prelude_call("_sp_limit", &[n, g])?,
            Expr::FirstExpr(g) => self.prelude_call("_sp_first", &[g])?,
            Expr::NthExpr { n, expr: g } => self.prelude_call("_sp_nth", &[n, g])?,
            Expr::Until { cond, update } => self.prelude_call("_sp_until", &[cond, update])?,
            Expr::While { cond, update } => {
                // `range/3` runs a `while` of its own, whose cap is `range`'s (#7977).
                let cur = &self.code.funcs[self.cur as usize];
                let tag = (cur.prelude && cur.name == "_sp_range").then_some("range");
                self.prelude_call_tagged("_sp_while", &[cond, update], tag)?;
            }
            Expr::Repeat(g) => self.prelude_call("_sp_repeat", &[g])?,
            Expr::RecursiveDescent => self.prelude_call("_sp_recurse", &[])?,
            // `def range($x): range(0; $x)` and the bytecoded `range/2`.
            Expr::Range {
                from,
                to: Some(to),
                step: None,
            } => {
                self.emit(Insn::Dup);
                self.expr(from)?;
                self.emit(Insn::Swap);
                self.expr(to)?;
                self.emit(Insn::Range);
            }
            Expr::Range {
                from,
                to: Some(to),
                step: Some(step),
            } => {
                self.prelude_call("_sp_range", &[from, to, step])?;
                // `$from` is `range`'s first output: passed as the shipped route emits it.
                if let Some(Insn::Call { args, .. }) = self.code.insns.last_mut() {
                    if let Some(first @ ArgSpec::Value) = args.first_mut() {
                        *first = ArgSpec::RangeStart;
                    }
                }
            }
            // `CALL_BUILTIN`: the last argument is the outermost loop.
            Expr::Builtin(Builtin::SetPath(path, value)) => {
                self.emit(Insn::Dup);
                self.expr(value)?;
                self.emit(Insn::Swap);
                self.emit(Insn::Dup);
                self.expr(path)?;
                self.emit(Insn::Swap);
                self.emit(Insn::SetPath);
            }
            Expr::Builtin(Builtin::Empty) => {
                self.emit(Insn::Backtrack);
            }
            Expr::Builtin(b) => match prelude_call(b) {
                Some((name, args)) => self.prelude_call(name, &args)?,
                None => self.leaf(e)?,
            },
            _ => self.leaf(e)?,
        }
        if let Some(slots) = self.kills.get(&(e as *const Expr as usize)) {
            for s in slots.clone() {
                self.emit(Insn::Kill(s));
            }
        }
        Ok(())
    }

    /// Mark the last reads of `$name` (bound to local slot `slot`) in `body`, its scope.
    fn last_uses(&mut self, name: &str, slot: u16, body: &Expr) {
        if !self.moves {
            return;
        }
        let mut live = LastUse {
            name,
            last: Vec::new(),
            kills: Vec::new(),
            excluded: false,
        };
        live.walk(body, false);
        if live.excluded {
            return;
        }
        self.last.extend(live.last);
        for at in live.kills {
            self.kills.entry(at).or_default().push(slot);
        }
    }

    /// `gen_try`.
    fn try_(&mut self, body: &'e Expr, catch: Option<&'e Expr>) -> C {
        match catch {
            None => {
                self.emit(Insn::TryBegin(None));
                self.expr(body)?;
                self.emit(Insn::TryEnd);
            }
            Some(handler) => {
                let t = self.emit(Insn::TryBegin(Some(0)));
                self.expr(body)?;
                self.emit(Insn::TryEnd);
                let j = self.emit(Insn::Jump(0));
                self.patch(t);
                self.expr(handler)?;
                self.patch(j);
            }
        }
        Ok(())
    }

    /// `tail_call_analyze`: a call followed by `RET` (through jumps) that holds no closure over
    /// the current frame, callee included, becomes a tail call.
    fn mark_tails(&mut self) {
        for pc in 0..self.code.insns.len() {
            let Insn::Call { callee, args, .. } = &self.code.insns[pc] else {
                continue;
            };
            let closes_here = matches!(callee, Callee::Def { level: 0, .. })
                || args.iter().any(|a| matches!(a, ArgSpec::Lambda(_)));
            let mut next = pc + 1;
            // Jumps nest one per enclosing `if` or `,`; the bound only keeps a jump chain from
            // being followed forever, and a call past it stays an ordinary call.
            let mut hops = 0;
            let ret = loop {
                match self.code.insns.get(next) {
                    Some(Insn::Jump(t)) if hops < 64 => {
                        next = *t;
                        hops += 1;
                    }
                    // A tail call drops the whole frame anyway.
                    Some(Insn::Kill(_)) => next += 1,
                    Some(Insn::Ret) => break true,
                    _ => break false,
                }
            };
            if ret {
                if let Insn::Call { tail, .. } = &mut self.code.insns[pc] {
                    *tail = if closes_here { Tail::Keep } else { Tail::Pop };
                }
            }
        }
    }
}

/// The last-use analysis of one variable over its scope, in the order the compiled code runs
/// forward. Code reached again only by backtracking is not "later": a fork point that can
/// re-run a read is live while the read runs, so the slot is below the watermark and the read
/// copies at run time. Everything this cannot order (call arguments, leaves, folds) is opaque:
/// no read inside is a last one, and the slot dies after the node instead.
struct LastUse<'n> {
    name: &'n str,
    last: Vec<usize>,
    kills: Vec<usize>,
    /// A nested `def` reads the variable: whoever calls it later reads it too.
    excluded: bool,
}

impl LastUse<'_> {
    fn reads(&self, e: &Expr) -> bool {
        any_subexpr(e, &mut |n| matches!(n, Expr::Var(v) if v == self.name))
    }

    /// `items` run one after another: each is followed by the ones after it.
    fn seq(&mut self, items: &[&Expr], live: bool) -> bool {
        let mut live = live;
        let mut any = false;
        for item in items.iter().rev() {
            let r = self.walk(item, live);
            live |= r;
            any |= r;
        }
        any
    }

    /// Whether `e` reads the variable; marks its last reads given whether later code does.
    fn walk(&mut self, e: &Expr, live: bool) -> bool {
        match e {
            Expr::Var(v) if v == self.name => {
                if !live {
                    self.last.push(e as *const Expr as usize);
                }
                true
            }
            Expr::Paren(inner) | Expr::Optional(inner) => self.walk(inner, live),
            Expr::Label { body, .. } => self.walk(body, live),
            Expr::Pipe(stages) => {
                let stages: Vec<&Expr> = stages.iter().collect();
                self.seq(&stages, live)
            }
            // The right operand runs first.
            Expr::Arithmetic { left, right, .. } | Expr::Compare { left, right, .. } => {
                self.seq(&[right, left], live)
            }
            Expr::And(a, b) | Expr::Or(a, b) => self.seq(&[a, b], live),
            // Alternatives: each is reached from the one before only by backtracking.
            Expr::Comma(items) => {
                let mut any = false;
                for item in items {
                    any |= self.walk(item, live);
                }
                any
            }
            Expr::Alternative(a, b) => {
                let ra = self.walk(a, live);
                self.walk(b, live) | ra
            }
            Expr::Try { expr: body, catch } => {
                let rb = self.walk(body, live);
                let rc = catch.as_deref().is_some_and(|c| self.walk(c, live));
                rb | rc
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let rt = self.walk(then_branch, live);
                let re = self.walk(else_branch, live);
                self.walk(cond, live || rt || re) | rt | re
            }
            Expr::As {
                expr: source,
                var,
                body,
            } => {
                let rb = if var == self.name {
                    false
                } else {
                    self.walk(body, live)
                };
                self.walk(source, live || rb) | rb
            }
            Expr::AsPattern {
                expr: source,
                patterns,
                body,
            } => {
                let mut bound = Vec::new();
                for p in patterns {
                    pattern_vars(p, &mut bound);
                }
                let keys = patterns
                    .iter()
                    .any(|p| super::super::super::walk::any_pattern_key(p, &mut |k| self.reads(k)));
                let rb = if bound.contains(&self.name) {
                    false
                } else {
                    self.walk(body, live)
                };
                self.walk(source, live || rb || keys) | rb | keys
            }
            Expr::FuncDef { body, then, .. } => {
                if self.reads(body) {
                    self.excluded = true;
                }
                self.walk(then, live)
            }
            // Opaque: call arguments (closures, which may run any number of times), leaves,
            // folds, generators and everything else.
            other => {
                let r = self.reads(other);
                if r && !live {
                    self.kills.push(other as *const Expr as usize);
                }
                r
            }
        }
    }
}
