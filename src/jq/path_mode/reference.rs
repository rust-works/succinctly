//! The Phase 0 interpreter of #5223, verbatim but for its paths: values are
//! the spike-local `SVal`, whose `jv` identity is exact by construction. It is
//! kept, test-only, as the reference the `OwnedValue` interpreter in the
//! parent module is differentially tested against
//! (`owned_interpreter_matches_the_sval_reference_5223`), and is deleted with
//! the resolver in Phase 3.

#![allow(dead_code)]

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

use crate::jq::error::{dump_truncated_with, Control, EvalError};
use crate::jq::eval::{
    apply_compare_op, arith_combine, arith_negate, compare_values, eval_owned_unrouted,
    substitute_vars, EvalTag, JqSemantics, QueryResult,
};
use crate::jq::expr::{
    Builtin, CompareOp, Expr, FormatType, Literal, NumberKey, ObjectKey, Param, Pattern,
    SliceBoundKey,
};
use crate::jq::parser::parse;
use crate::jq::value::{NumberRepr, OwnedValue};
use crate::jq::walk::any_subexpr;

/// A value with `jv` identity (see the module docs and [`identical`]).
#[derive(Clone, Debug)]
pub enum SVal {
    /// `null`.
    Null,
    /// `true`/`false`.
    Bool(bool),
    /// A number: its value (an `Int`, `Float` or `NumberLiteral`, for output),
    /// and the allocation that makes it a *literal* (`Some`) or `None` for a
    /// computed double.
    Num(OwnedValue, Option<Rc<()>>),
    /// A string.
    Str(Rc<str>),
    /// An array.
    Arr(Rc<Vec<Self>>),
    /// An object, in insertion order.
    Obj(Rc<Vec<(Rc<str>, Self)>>),
}

/// Where a converted [`OwnedValue`]'s numbers come from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NumOrigin {
    /// Parsed from the input document: every number is a literal, as
    /// `jv_parse` makes it.
    Document,
    /// A C builtin's result: a spelled number is a fresh literal, any other a
    /// computed double.
    Fresh,
}

impl SVal {
    pub(super) fn computed(value: OwnedValue) -> Self {
        Self::Num(value, None)
    }

    fn literal(value: OwnedValue) -> Self {
        Self::Num(value, Some(Rc::new(())))
    }

    fn string(s: &str) -> Self {
        Self::Str(Rc::from(s))
    }

    fn truthy(&self) -> bool {
        !matches!(self, Self::Null | Self::Bool(false))
    }

    fn f64(&self) -> Option<f64> {
        match self {
            Self::Num(n, _) => n.as_f64(),
            _ => None,
        }
    }

    fn from_owned(value: &OwnedValue, origin: NumOrigin) -> Self {
        match value {
            OwnedValue::Null => Self::Null,
            OwnedValue::Bool(b) => Self::Bool(*b),
            OwnedValue::NumberLiteral(..) => Self::literal(value.clone()),
            OwnedValue::Int(_) | OwnedValue::Float(_) => match origin {
                NumOrigin::Document => Self::literal(value.clone()),
                NumOrigin::Fresh => Self::computed(value.clone()),
            },
            OwnedValue::String(s) => Self::string(s.as_str()),
            OwnedValue::Array(items) => Self::Arr(Rc::new(
                items.iter().map(|v| Self::from_owned(v, origin)).collect(),
            )),
            OwnedValue::Object(map) => Self::Obj(Rc::new(
                map.iter()
                    .map(|(k, v)| (Rc::from(k.as_str()), Self::from_owned(v, origin)))
                    .collect(),
            )),
        }
    }

    /// This value as an [`OwnedValue`], for output and for the delegated
    /// evaluator.
    pub fn to_owned_value(&self) -> OwnedValue {
        match self {
            Self::Null => OwnedValue::Null,
            Self::Bool(b) => OwnedValue::Bool(*b),
            Self::Num(n, _) => n.clone(),
            Self::Str(s) => OwnedValue::string(&**s),
            Self::Arr(items) => OwnedValue::Array(items.iter().map(Self::to_owned_value).collect()),
            Self::Obj(map) => OwnedValue::Object(
                map.iter()
                    .map(|(k, v)| (k.to_string(), v.to_owned_value()))
                    .collect(),
            ),
        }
    }
}

/// jq 1.7.1's `jv_identical`.
///
/// The same `kind_flags`, then pointer identity for an allocated value
/// (array, object, string, literal number) and payload bits for anything
/// else. A literal and a computed number differ in `kind_flags`, so they are
/// never identical.
pub fn identical(a: &SVal, b: &SVal) -> bool {
    match (a, b) {
        (SVal::Null, SVal::Null) => true,
        (SVal::Bool(x), SVal::Bool(y)) => x == y,
        (SVal::Num(_, Some(x)), SVal::Num(_, Some(y))) => Rc::ptr_eq(x, y),
        (SVal::Num(x, None), SVal::Num(y, None)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => x.to_bits() == y.to_bits(),
            _ => false,
        },
        (SVal::Str(x), SVal::Str(y)) => Rc::ptr_eq(x, y),
        (SVal::Arr(x), SVal::Arr(y)) => Rc::ptr_eq(x, y),
        (SVal::Obj(x), SVal::Obj(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

fn value_eq(a: &SVal, b: &SVal) -> bool {
    apply_compare_op::<JqSemantics>(CompareOp::Eq, &a.to_owned_value(), &b.to_owned_value())
}

/// A persistent path prefix: a fork shares its parent for free.
#[derive(Clone, Default)]
struct PathList(Option<Rc<(Self, SVal)>>);

impl PathList {
    fn push(&self, key: SVal) -> Self {
        Self(Some(Rc::new((self.clone(), key))))
    }

    fn to_array(&self) -> SVal {
        let mut keys = Vec::new();
        let mut at = self;
        while let Some(node) = &at.0 {
            keys.push(node.1.clone());
            at = &node.0;
        }
        keys.reverse();
        SVal::Arr(Rc::new(keys))
    }
}

/// jq 1.7.1's register: `path` (`None` is the `jv_null` it holds outside any
/// `path()`, where nothing is tracked), `value_at_path` and `subexp_nest`.
#[derive(Clone)]
struct Reg {
    path: Option<PathList>,
    node: SVal,
    nest: u32,
}

impl Reg {
    fn top() -> Self {
        Self {
            path: None,
            node: SVal::Null,
            nest: 0,
        }
    }

    fn tracking(&self) -> bool {
        self.nest == 0 && self.path.is_some()
    }

    /// `path_intact`: true when not tracking, else `jv_identical`.
    fn intact(&self, value: &SVal) -> bool {
        !self.tracking() || identical(value, &self.node)
    }

    /// `path_append`: a no-op when not tracking.
    fn append(mut self, key: SVal, value: SVal) -> Self {
        if self.tracking() {
            self.path = Some(self.path.as_ref().map(|p| p.push(key)).unwrap_or_default());
            self.node = value;
        }
        self
    }

    /// `SUBEXP_BEGIN`: tracking suspended for the sub-expression.
    fn sub(&self) -> Self {
        let mut reg = self.clone();
        reg.nest += 1;
        reg
    }
}

/// How an evaluation stopped.
#[derive(Debug)]
pub enum Raise {
    /// A jq error carrying its payload (`error(v)`'s `v`, or a message).
    Error(SVal),
    /// An error raised downstream of a `try` body, wrapped by `TRY_END` so the
    /// `try` re-raises it instead of handling it.
    Wrapped(Box<Self>),
    /// `halt`/`halt_error`: nothing catches it.
    Halt(i32),
    /// A construct the spike does not implement: nothing catches it either.
    Unsupported(String),
}

type R = Result<(), Raise>;
type Sink<'s> = dyn FnMut(SVal, Reg) -> R + 's;

fn raise_eval(error: EvalError) -> Raise {
    if !error.is_uncatchable() {
        return Raise::Error(SVal::from_owned(&error.payload(), NumOrigin::Fresh));
    }
    unsupported(format!("uncatchable delegated error: {}", error.message)) // patchcov: coverage tolerate-line reason="defensive: the delegated builtins the spike reaches raise only catchable errors; a resource limit would land here"
}

/// An error the spike raises itself (jq's three path messages, an object
/// key's type): a plain string error, catchable like any jq error. Only
/// [`raise_eval`] reads `EvalError`'s own catchability, which describes the
/// shipped resolver's refusals, not jq.
fn raise_msg(error: EvalError) -> Raise {
    Raise::Error(SVal::string(&error.message))
}

fn unsupported(what: impl Into<String>) -> Raise {
    Raise::Unsupported(what.into())
}

/// The lexical environment: variables, function definitions, closure
/// parameters and labels.
#[derive(Clone, Default)]
struct Env<'e>(Option<Rc<Frame<'e>>>);

enum Frame<'e> {
    Var {
        name: &'e str,
        value: SVal,
        parent: Env<'e>,
    },
    /// A `def`: its body runs in the environment *this frame* closes, so it
    /// sees itself and every definition before it.
    Def {
        name: &'e str,
        params: &'e [Param],
        body: &'e Expr,
        parent: Env<'e>,
    },
    /// A closure parameter: an argument expression and its caller's
    /// environment.
    Arg {
        name: &'e str,
        body: &'e Expr,
        env: Env<'e>,
        parent: Env<'e>,
    },
    Label {
        name: &'e str,
        value: SVal,
        parent: Env<'e>,
    },
}

/// A resolved function.
enum Callee<'e> {
    Def {
        params: &'e [Param],
        body: &'e Expr,
        env: Env<'e>,
    },
    Arg {
        body: &'e Expr,
        env: Env<'e>,
    },
}

impl<'e> Env<'e> {
    fn push(frame: Frame<'e>) -> Self {
        Self(Some(Rc::new(frame)))
    }

    fn parent<'a>(frame: &'a Frame<'e>) -> &'a Self {
        match frame {
            Frame::Var { parent, .. }
            | Frame::Def { parent, .. }
            | Frame::Arg { parent, .. }
            | Frame::Label { parent, .. } => parent,
        }
    }

    fn lookup_var(&self, name: &str) -> Option<SVal> {
        let mut at = self;
        while let Some(frame) = &at.0 {
            if let Frame::Var { name: n, value, .. } = &**frame {
                if *n == name {
                    return Some(value.clone());
                }
            }
            at = Self::parent(frame);
        }
        None
    }

    fn lookup_label(&self, name: &str) -> Option<SVal> {
        let mut at = self;
        while let Some(frame) = &at.0 {
            if let Frame::Label { name: n, value, .. } = &**frame {
                if *n == name {
                    return Some(value.clone());
                }
            }
            at = Self::parent(frame);
        }
        None
    }

    fn lookup_func(&self, name: &str, arity: usize) -> Option<Callee<'e>> {
        let mut at = self;
        while let Some(frame) = &at.0 {
            match &**frame {
                Frame::Def {
                    name: n,
                    params,
                    body,
                    ..
                } if *n == name && params.len() == arity => {
                    return Some(Callee::Def {
                        params,
                        body,
                        env: at.clone(),
                    });
                }
                Frame::Arg {
                    name: n, body, env, ..
                } if *n == name && arity == 0 => {
                    return Some(Callee::Arg {
                        body,
                        env: env.clone(),
                    });
                }
                _ => {}
            }
            at = Self::parent(frame);
        }
        None
    }
}

const T_VAR: &str = "__sp_t";
const K_VAR: &str = "__sp_k";
const A_VAR: &str = "__sp_a";
const B_VAR: &str = "__sp_b";

impl<'e> Env<'e> {
    /// Bind a variable whose name the program (or prelude) owns.
    fn var_ref(&self, name: &'e str, value: SVal) -> Self {
        Env::push(Frame::Var {
            name,
            value,
            parent: self.clone(),
        })
    }
}

/// The jq-defined builtins the spike runs from source.
///
/// This is jq 1.7.1's `src/builtin.jq`, verbatim but for three mechanical
/// changes. Every name
/// carries an `_sp_` prefix, so the parser cannot fold a definition back into
/// the `Builtin` it replaces. Closure parameters are renamed where
/// succinctly's parser would read the jq name as a builtin (`exp`, `paths`).
/// And `_modify`'s private `$$$$v` (`LOADVN`, which only avoids a refcount)
/// is spelled `$v`.
///
/// Bodies call one another through the `Builtin` variants they parse to,
/// which `prelude_call` maps back here, plus a few direct `_sp_` calls. The
/// C functions those bodies call that succinctly has no builtin for
/// (`_sort_by_impl`, `_strindices`, ...) are `_sp_hook_*` calls, answered by
/// `Interp::hook`.
pub const PRELUDE: &str = r#"
def _sp_select(f): if f then . else empty end;
def _sp_map(f): [.[] | f];
def _sp_recurse(f): def r: ., (f | r); r;
def _sp_recurse(f; cond): def r: ., (f | select(cond) | r); r;
def _sp_recurse: recurse(.[]?);
def _sp_add: reduce .[] as $x (null; . + $x);
def _sp_del(f): delpaths([path(f)]);
def _sp_assign(sp_paths; $value): reduce path(sp_paths) as $p (.; setpath($p; $value));
def _sp_modify(sp_paths; sp_update):
    reduce path(sp_paths) as $p ([., []];
        . as $dot
      | null
      | label $out
      | ($dot[0] | getpath($p)) as $v
      | (
          (   $v
            | sp_update
            | (., break $out) as $v
            | $dot
            | setpath([0] + $p; $v)
          ),
          (
              $dot
            | setpath([1, (.[1] | length)]; $p)
          )
        )
    ) | . as $dot | $dot[0] | delpaths($dot[1]);
def _sp_upd_add(sp_paths; $sp_t): _sp_modify(sp_paths; . + $sp_t);
def _sp_upd_sub(sp_paths; $sp_t): _sp_modify(sp_paths; . - $sp_t);
def _sp_upd_mul(sp_paths; $sp_t): _sp_modify(sp_paths; . * $sp_t);
def _sp_upd_div(sp_paths; $sp_t): _sp_modify(sp_paths; . / $sp_t);
def _sp_upd_mod(sp_paths; $sp_t): _sp_modify(sp_paths; . % $sp_t);
def _sp_upd_alt(sp_paths; $sp_t): _sp_modify(sp_paths; . // $sp_t);
def _sp_map_values(f): .[] |= f;
def _sp_to_entries: [keys_unsorted[] as $k | {key: $k, value: .[$k]}];
def _sp_from_entries: map({(.key // .Key // .name // .Name): (if has("value") then .value else .Value end)}) | add | .//={};
def _sp_with_entries(f): to_entries | map(f) | from_entries;
def _sp_reverse: [.[length - 1 - range(0;length)]];
def _sp_paths: path(..)|select(length > 0);
def _sp_paths(node_filter): path(..|select(node_filter))|select(length > 0);
def _sp_arrays: select(type == "array");
def _sp_objects: select(type == "object");
def _sp_iterables: select(type|. == "array" or . == "object");
def _sp_booleans: select(type == "boolean");
def _sp_numbers: select(type == "number");
def _sp_normals: select(isnormal);
def _sp_finites: select(isfinite);
def _sp_strings: select(type == "string");
def _sp_nulls: select(. == null);
def _sp_values: select(. != null);
def _sp_scalars: select(type|. != "array" and . != "object");
def _sp_join($x): reduce .[] as $i (null;
            (if .==null then "" else .+$x end) +
            ($i | if type=="boolean" or type=="number" then tostring else .//"" end)
        ) // "";
def _sp__flatten($x): reduce .[] as $i ([]; if $i | type == "array" and $x != 0 then . + ($i | _sp__flatten($x-1)) else . + [$i] end);
def _sp_flatten($x): if $x < 0 then error("flatten depth must not be negative") else _sp__flatten($x) end;
def _sp_flatten: _sp__flatten(-1);
def _sp_while(cond; sp_update):
     def _while:
         if cond then ., (sp_update | _while) else empty end;
     _while;
def _sp_until(cond; sp_next):
     def _until:
         if cond then . else (sp_next|_until) end;
     _until;
def _sp_limit($n; sp_exp):
    if $n > 0 then label $out | foreach sp_exp as $item ($n; .-1; $item, if . <= 0 then break $out else empty end)
    elif $n == 0 then empty
    else sp_exp end;
def _sp_range($init; $upto; $by):
    if $by > 0 then $init|while(. < $upto; . + $by)
  elif $by < 0 then $init|while(. > $upto; . + $by)
  else empty end;
def _sp_first(g): label $out | g | ., break $out;
def _sp_isempty(g): first((g|false), true);
def _sp_all(generator; condition): isempty(generator|condition and empty);
def _sp_any(generator; condition): isempty(generator|condition or empty)|not;
def _sp_all(condition): all(.[]; condition);
def _sp_any(condition): any(.[]; condition);
def _sp_all: all(.[]; .);
def _sp_any: any(.[]; .);
def _sp_last(g): reduce g as $item (null; $item);
def _sp_nth($n; g):
  if $n < 0 then error("nth doesn't support negative indices")
  else label $out | foreach g as $item ($n + 1; . - 1; if . <= 0 then $item, break $out else empty end) end;
def _sp_first: .[0];
def _sp_last: .[-1];
def _sp_nth($n): .[$n];
def _sp_in(xs): . as $x | xs | has($x);
def _sp_inside(xs): . as $x | xs | contains($x);
def _sp_repeat(sp_exp):
     def _repeat:
         sp_exp, _repeat;
     _repeat;
def _sp_ascii_downcase:
  explode | map( if 65 <= . and . <= 90 then . + 32  else . end) | implode;
def _sp_ascii_upcase:
  explode | map( if 97 <= . and . <= 122 then . - 32  else . end) | implode;
def _sp_walk(f):
  def w:
    if type == "object"
    then map_values(w)
    elif type == "array" then map(w)
    else .
    end
    | f;
  w;
def _sp_pick(pathexps):
  . as $in
  | reduce path(pathexps) as $a (null;
      setpath($a; $in|getpath($a)) );
def _sp_IN(s): any(s == .; .);
def _sp_IN(src; s): any(src == s; .);
def _sp_abs: if . < 0 then - . else . end;
def _sp_sort_by(f): _sp_hook_sort_by(map([f]));
def _sp_group_by(f): _sp_hook_group_by(map([f]));
def _sp_unique: group_by(.) | map(.[0]);
def _sp_unique_by(f): group_by(f) | map(.[0]);
def _sp_max_by(f): _sp_hook_max_by(map([f]));
def _sp_min_by(f): _sp_hook_min_by(map([f]));
def _sp_indices($i): if type == "array" and ($i|type) == "array" then .[$i]
  elif type == "array" then .[[$i]]
  elif type == "string" and ($i|type) == "string" then _sp_hook_strindices($i)
  else .[$i] end;
def _sp_index($i):   indices($i) | .[0];
def _sp_rindex($i):  indices($i) | .[-1:][0];
def _sp_debug(msg): (msg | debug | empty), .;
.
"#;

/// The parsed [`PRELUDE`], plus the few one-off expressions the spike needs a
/// stable `&Expr` for.
pub struct Prelude {
    defs: Expr,
    /// `$__sp_t`: a compound assignment's right-hand value, re-read as the
    /// `$sp_t` argument of `_sp_upd_*`.
    t_var: Expr,
    /// `.[$__sp_k]`: `jv_get` for the key/container pairs the spike delegates.
    get_k: Expr,
    /// `.[]`: the delegated `Cannot iterate over ...` error.
    iterate: Expr,
}

impl Prelude {
    /// Parse the prelude. Panics only if [`PRELUDE`] itself fails to parse,
    /// which a unit test pins.
    pub fn new() -> Self {
        let parse_ok = |src: &str| parse(src).unwrap_or_else(|e| panic!("prelude: {e:?}"));
        Self {
            defs: parse_ok(PRELUDE),
            t_var: parse_ok("$__sp_t"),
            get_k: parse_ok(".[$__sp_k]"),
            iterate: parse_ok(".[]"),
        }
    }
}

impl Default for Prelude {
    fn default() -> Self {
        Self::new()
    }
}

/// An array's elements, shared.
type Items = Rc<Vec<SVal>>;

/// The interpreter: one per program run.
pub struct Interp<'e> {
    prelude: &'e Prelude,
    prelude_env: Env<'e>,
    /// One `jv` per constant site, as `LOADK` hands out (keyed by the `Expr`'s
    /// address, which is stable: the spike never rewrites the program).
    consts: RefCell<BTreeMap<usize, SVal>>,
    next_label: Cell<u64>,
    /// Every builtin the run answered through the fresh stub although jq's
    /// own version can hand back its input or navigates it ([`stub_risk`]).
    stubbed: RefCell<Vec<String>>,
    /// Array slices as `jvp_array_slice` makes them: a view (`offset`, `size`)
    /// into the root allocation. Keyed by `(root, offset, size)`, so equal
    /// slices of one array are one value; each view maps back to its root
    /// and offset, so a slice of a slice lands on the same key. Both hold
    /// the root, so a compared address is never reused.
    slices: RefCell<BTreeMap<(usize, usize, usize), Items>>,
    slice_roots: RefCell<BTreeMap<usize, (Items, usize)>>,
}

/// How a run ended.
#[derive(Debug)]
pub enum Outcome {
    /// Every output was produced.
    Done,
    /// An uncaught error, with its payload.
    Error(OwnedValue),
    /// `halt`/`halt_error` with this exit code.
    Halt(i32),
    /// A construct the spike does not implement.
    Unsupported(String),
}

/// A finished run: the outputs produced before it ended, and how it ended.
#[derive(Debug)]
pub struct Run {
    /// Outputs, in order.
    pub outputs: Vec<OwnedValue>,
    /// How the run ended.
    pub outcome: Outcome,
    /// The builtins the run answered through the fresh stub although jq's
    /// own version can hand back its input or navigates it: a divergence on
    /// such a run may be the stub's, not a construct rule's.
    pub stubbed: Vec<String>,
}

/// Parse `filter` and run it on `input` (a parsed input document). `Err` is
/// a compile error: the filter does not parse, or jq would reject it.
pub fn run(filter: &str, input: &OwnedValue) -> Result<Run, String> {
    let program = parse(filter).map_err(|e| format!("{e:?}"))?;
    // jq's compile-time checks (an unbound `$x`, an undefined function), which
    // the interpreter alone would only meet if evaluation reached them. The
    // resolver rewrites what it checks, so it runs on a copy.
    let diagnostics = crate::jq::resolve::resolve_all_jq(&mut program.clone());
    if !diagnostics.is_empty() {
        return Err(format!("{diagnostics:?}"));
    }
    let prelude = Prelude::new();
    let interp = Interp::new(&prelude);
    Ok(interp.run(&program, input))
}

impl<'e> Interp<'e> {
    /// An interpreter over `prelude`'s definitions.
    pub fn new(prelude: &'e Prelude) -> Self {
        let mut env = Env::default();
        let mut at = &prelude.defs;
        while let Expr::FuncDef {
            name,
            params,
            body,
            then,
            ..
        } = at
        {
            env = Env::push(Frame::Def {
                name,
                params,
                body,
                parent: env.clone(),
            });
            at = then;
        }
        Self {
            prelude,
            prelude_env: env,
            consts: RefCell::new(BTreeMap::new()),
            next_label: Cell::new(0),
            stubbed: RefCell::new(Vec::new()),
            slices: RefCell::new(BTreeMap::new()),
            slice_roots: RefCell::new(BTreeMap::new()),
        }
    }

    /// Run `program` on `input`.
    pub fn run(&self, program: &'e Expr, input: &OwnedValue) -> Run {
        let mut outputs = Vec::new();
        let input = SVal::from_owned(input, NumOrigin::Document);
        let result = self.eval(program, &Env::default(), input, Reg::top(), &mut |v, _| {
            outputs.push(v.to_owned_value());
            Ok(())
        });
        let outcome = match result {
            Ok(()) => Outcome::Done,
            Err(raise) => Self::outcome(raise),
        };
        Run {
            outputs,
            outcome,
            stubbed: self.stubbed.take(),
        }
    }

    fn outcome(raise: Raise) -> Outcome {
        match raise {
            Raise::Error(payload) => Outcome::Error(payload.to_owned_value()),
            Raise::Wrapped(inner) => Self::outcome(*inner), // patchcov: coverage tolerate-line reason="unreachable: a wrapped error is unwrapped by the try or label that wrapped it before it can reach the top"
            Raise::Halt(code) => Outcome::Halt(code),
            Raise::Unsupported(what) => Outcome::Unsupported(what),
        }
    }

    fn eval(&self, expr: &'e Expr, env: &Env<'e>, input: SVal, reg: Reg, sink: &mut Sink<'_>) -> R {
        match expr {
            Expr::Identity => sink(input, reg),
            Expr::Paren(inner) => self.eval(inner, env, input, reg, sink),
            Expr::Pipe(stages) => self.eval_pipe(stages, env, input, reg, sink),
            // `FORK`: each branch starts from the register at entry.
            Expr::Comma(items) => {
                for item in items {
                    self.eval(item, env, input.clone(), reg.clone(), sink)?;
                }
                Ok(())
            }
            Expr::Literal(lit) => match self.constant(expr) {
                Some(value) => sink(value, reg),
                // A negative spelling is `_negate` of a literal: computed.
                None => match lit {
                    Literal::NumberLiteral(repr, _) => {
                        let value = match repr {
                            NumberRepr::Int(i) => OwnedValue::Int(*i),
                            NumberRepr::Float(f) => OwnedValue::Float(*f),
                        };
                        sink(SVal::computed(value), reg)
                    }
                    _ => Err(unsupported("a literal that is not a constant")), // patchcov: coverage tolerate-line reason="unreachable: constant_value answers every literal but a negative number, which the arm above computes"
                },
            },
            Expr::Loc { .. } => {
                let value = self
                    .constant(expr)
                    .ok_or_else(|| unsupported("a location that is not a constant"))?;
                sink(value, reg)
            }
            Expr::Field(name) => self.index(input, reg, SVal::string(name), false, sink),
            Expr::Index { idx, key } => {
                let key = index_key(*idx, key.as_ref());
                self.index(input, reg, key, false, sink)
            }
            Expr::Slice {
                start,
                end,
                start_key,
                end_key,
            } => {
                let key = static_slice_key(*start, *end, start_key.as_ref(), end_key.as_ref());
                self.index(input, reg, key, false, sink)
            }
            Expr::Iterate => self.each(input, reg, false, sink),
            Expr::IndexExpr { target, key } => {
                self.index_expr(target, key, env, input, reg, false, sink)
            }
            Expr::SliceExpr { target, start, end } => {
                self.slice_expr(target, start, end, env, input, reg, false, sink)
            }
            Expr::Optional(inner) => self.optional(inner, env, input, reg, sink),
            Expr::Array(inner) => self.collect(expr, inner, env, input, reg, sink),
            Expr::Object(entries) => {
                if let Some(value) = self.constant(expr) {
                    return sink(value, reg);
                }
                let entries: Vec<(&'e ObjectKey, &'e Expr)> =
                    entries.iter().map(|e| (&e.key, &e.value)).collect();
                self.object(&entries, Vec::new(), env, &input, &reg, sink)
            }
            Expr::RecursiveDescent => self.call_prelude("_sp_recurse", &[], env, input, reg, sink),
            Expr::Arithmetic {
                op, left, right, ..
            } => {
                if let Some(value) = self.constant(expr) {
                    return sink(value, reg);
                }
                let op = *op;
                self.binop(left, right, env, &input, &reg, sink, &mut |l, r| {
                    arith_combine::<JqSemantics>(op, l.to_owned_value(), r.to_owned_value())
                        .map(|v| SVal::from_owned(&v, NumOrigin::Fresh))
                })
            }
            Expr::Compare { op, left, right } => {
                if let Some(value) = self.constant(expr) {
                    return sink(value, reg);
                }
                let op = *op;
                self.binop(left, right, env, &input, &reg, sink, &mut |l, r| {
                    Ok(SVal::Bool(apply_compare_op::<JqSemantics>(
                        op,
                        &l.to_owned_value(),
                        &r.to_owned_value(),
                    )))
                })
            }
            // `'-' Exp` is `Exp | _negate`: the operand is tracked, not a
            // subexp, and the result is computed.
            Expr::Negate(inner) => self.eval(inner, env, input, reg, &mut |v, r| {
                let negated =
                    arith_negate::<JqSemantics>(v.to_owned_value()).map_err(raise_eval)?;
                sink(SVal::from_owned(&negated, NumOrigin::Fresh), r)
            }),
            Expr::And(a, b) => self.and_or(a, b, true, env, input, reg, sink),
            Expr::Or(a, b) => self.and_or(a, b, false, env, input, reg, sink),
            // Bytecoded `not`: a branch on the input, no call.
            Expr::Not => sink(SVal::Bool(!input.truthy()), reg),
            Expr::Alternative(a, b) => self.defined_or(a, b, env, input, reg, sink),
            // `gen_cond`: the condition is a subexp; each branch starts from
            // the register at entry.
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => self.eval(cond, env, input.clone(), reg.sub(), &mut |c, _| {
                let branch = if c.truthy() { then_branch } else { else_branch };
                self.eval(branch, env, input.clone(), reg.clone(), sink)
            }),
            Expr::Try { expr: body, catch } => {
                self.try_catch(body, catch.as_deref(), env, input, reg, sink)
            }
            // C `error/1` on the input.
            Expr::Error(None) => Err(Raise::Error(input)),
            // `def error(msg): msg|error;`: `msg` is tracked.
            Expr::Error(Some(msg)) => {
                self.eval(msg, env, input, reg, &mut |v, _| Err(Raise::Error(v)))
            }
            Expr::Builtin(builtin) => self.builtin(expr, builtin, env, input, reg, sink),
            Expr::StringInterpolation(_) => delegate(expr, env, input, reg, sink),
            Expr::Format(format) => {
                // `@text` is `f_tostring`, which returns a string input itself.
                if matches!(format, FormatType::Text) {
                    self.note_stub("Format(Text)");
                }
                delegate(expr, env, input, reg, sink)
            }
            Expr::Env => delegate(expr, env, input, reg, sink),
            // `gen_destructure` with a plain `$var`: the source is a subexp,
            // and the body starts from the register at entry.
            Expr::As {
                expr: source,
                var,
                body,
            } => self.eval(source, env, input.clone(), reg.sub(), &mut |v, _| {
                let env = env.var_ref(var, v);
                self.eval(body, &env, input.clone(), reg.clone(), sink)
            }),
            Expr::AsPattern {
                expr: source,
                patterns,
                body,
            } => self.eval(source, env, input.clone(), reg.sub(), &mut |v, _| {
                self.destructure(patterns, v, env, reg.clone(), &mut |env, reg| {
                    self.eval(body, &env, input.clone(), reg, sink)
                })
            }),
            Expr::Var(name) => match env.lookup_var(name) {
                Some(value) => sink(value, reg),
                None => Err(unsupported(format!("${name} is not bound"))), // patchcov: coverage tolerate-line reason="unreachable: run() rejects an unbound variable with resolve_all_jq before evaluating"
            },
            Expr::Reduce {
                input: source,
                patterns,
                init,
                update,
            } => self.reduce(source, patterns, init, update, env, input, reg, sink),
            Expr::Foreach {
                input: source,
                patterns,
                init,
                update,
                extract,
            } => self.foreach(
                source,
                patterns,
                init,
                update,
                extract.as_deref(),
                env,
                input,
                reg,
                sink,
            ),
            Expr::Limit { n, expr: g } => {
                self.call_prelude("_sp_limit", &[n, g], env, input, reg, sink)
            }
            Expr::FirstExpr(g) => self.call_prelude("_sp_first", &[g], env, input, reg, sink),
            Expr::LastExpr(g) => self.call_prelude("_sp_last", &[g], env, input, reg, sink),
            Expr::NthExpr { n, expr: g } => {
                self.call_prelude("_sp_nth", &[n, g], env, input, reg, sink)
            }
            Expr::Until { cond, update } => {
                self.call_prelude("_sp_until", &[cond, update], env, input, reg, sink)
            }
            Expr::While { cond, update } => {
                self.call_prelude("_sp_while", &[cond, update], env, input, reg, sink)
            }
            Expr::Repeat(g) => self.call_prelude("_sp_repeat", &[g], env, input, reg, sink),
            Expr::Range { from, to, step } => match (to, step) {
                (Some(to), None) => self.range(from, to, env, input, reg, sink),
                (Some(to), Some(step)) => {
                    self.call_prelude("_sp_range", &[from, to, step], env, input, reg, sink)
                }
                (None, _) => Err(unsupported("range without an upper bound")),
            },
            Expr::Label { name, body } => self.label(name, body, env, input, reg, sink),
            Expr::Break(name) => match env.lookup_label(name) {
                Some(label) => Err(Raise::Error(label)),
                None => Err(unsupported(format!("$*label-{name} is not defined"))), // patchcov: coverage tolerate-line reason="unreachable: run() rejects a break with no enclosing label with resolve_all_jq before evaluating"
            },
            Expr::FuncDef {
                name,
                params,
                body,
                then,
                ..
            } => {
                let env = Env::push(Frame::Def {
                    name,
                    params,
                    body,
                    parent: env.clone(),
                });
                self.eval(then, &env, input, reg, sink)
            }
            Expr::FuncCall {
                name,
                args,
                builtin_fallback,
            } => match env.lookup_func(name, args.len()) {
                Some(callee) => {
                    let args: Vec<&'e Expr> = args.iter().collect();
                    self.call(callee, &args, env, input, reg, sink)
                }
                None => match builtin_fallback {
                    Some(fallback) => self.eval(fallback, env, input, reg, sink),
                    None => self.hook(name, args, env, input, reg, sink),
                },
            },
            Expr::Assign { path, value } => {
                self.call_prelude("_sp_assign", &[path, value], env, input, reg, sink)
            }
            Expr::Update { path, filter } => {
                self.call_prelude("_sp_modify", &[path, filter], env, input, reg, sink)
            }
            Expr::CompoundAssign { op, path, value } => {
                let name = match op {
                    crate::jq::expr::AssignOp::Add => "_sp_upd_add",
                    crate::jq::expr::AssignOp::Sub => "_sp_upd_sub",
                    crate::jq::expr::AssignOp::Mul(_) => "_sp_upd_mul",
                    crate::jq::expr::AssignOp::Div => "_sp_upd_div",
                    crate::jq::expr::AssignOp::Mod => "_sp_upd_mod",
                };
                self.update_with(name, path, value, env, input, reg, sink)
            }
            Expr::AlternativeAssign { path, value } => {
                self.update_with("_sp_upd_alt", path, value, env, input, reg, sink)
            }
            other => Err(unsupported(format!("expression {}", variant_name(other)))), // patchcov: coverage tolerate-line reason="defensive: the remaining Expr variants are yq-only or resolver products the jq parser never emits"
        }
    }

    fn eval_pipe(
        &self,
        stages: &'e [Expr],
        env: &Env<'e>,
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        match stages {
            [] => sink(input, reg),
            [only] => self.eval(only, env, input, reg, sink),
            [first, rest @ ..] => self.eval(first, env, input, reg, &mut |v, r| {
                self.eval_pipe(rest, env, v, r, sink)
            }),
        }
    }

    /// `LOADK`: one value per constant site. `None` when `expr` is not
    /// something jq compiles to a single constant.
    fn constant(&self, expr: &'e Expr) -> Option<SVal> {
        let key = expr as *const Expr as usize;
        if let Some(value) = self.consts.borrow().get(&key) {
            return Some(value.clone());
        }
        let value = constant_value(expr)?;
        self.consts.borrow_mut().insert(key, value.clone());
        Some(value)
    }

    /// `INDEX`/`INDEX_OPT` on `container` with `key`: `path_intact` first
    /// (a failure raises even under `INDEX_OPT`), then `jv_get`, then
    /// `path_append` with the key as given. 1.7.1 does not renormalize a
    /// negative index (`[1,2,3] | path(.[-1])` is `[-1]`; 1.8 changed that).
    fn index(&self, container: SVal, reg: Reg, key: SVal, opt: bool, sink: &mut Sink<'_>) -> R {
        if !reg.intact(&container) {
            return Err(raise_msg(EvalError::invalid_path_expression_near_access(
                &key.to_owned_value(),
                &container.to_owned_value(),
            )));
        }
        match self.jv_get(&container, &key) {
            Ok(value) => {
                let reg = reg.append(key, value.clone());
                sink(value, reg)
            }
            // `INDEX_OPT` drops the get's own error, never an escape the spike
            // raises past every handler.
            Err(Raise::Error(_)) if opt => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// `jv_get`, natively for the shapes whose result keeps its identity, and
    /// through the existing evaluator for the rest (slices, array-of-indices
    /// keys, and every error), whose results are fresh.
    fn jv_get(&self, container: &SVal, key: &SVal) -> Result<SVal, Raise> {
        match (container, key) {
            (SVal::Obj(map), SVal::Str(k)) => Ok(map
                .iter()
                .find(|(name, _)| **name == **k)
                .map_or(SVal::Null, |(_, v)| v.clone())),
            (SVal::Arr(items), SVal::Num(..)) => {
                let d = key.f64().unwrap_or(f64::NAN);
                if d.is_nan() {
                    return Ok(SVal::Null);
                }
                let mut idx = i64::from(d.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32);
                if idx < 0 {
                    idx += items.len() as i64;
                }
                Ok(usize::try_from(idx)
                    .ok()
                    .and_then(|i| items.get(i))
                    .cloned()
                    .unwrap_or(SVal::Null))
            }
            (SVal::Null, SVal::Str(_) | SVal::Num(..) | SVal::Obj(_)) => Ok(SVal::Null),
            // `jv_string_slice`: by codepoint, and always a fresh string.
            (SVal::Str(text), SVal::Obj(bounds))
                if slice_bounds(text.chars().count(), bounds).is_some() =>
            {
                let (start, end) = slice_bounds(text.chars().count(), bounds).unwrap_or_default();
                let slice: String = text.chars().skip(start).take(end - start).collect();
                Ok(SVal::string(&slice))
            }
            (SVal::Arr(items), SVal::Obj(bounds))
                if slice_bounds(items.len(), bounds).is_some() =>
            {
                let (start, end) = slice_bounds(items.len(), bounds).unwrap_or_default();
                Ok(self.array_slice(items, start, end))
            }
            _ => {
                let expr = substitute_vars(&self.prelude.get_k, [(K_VAR, &key.to_owned_value())]);
                let (outputs, control) = delegate_eval(&expr, &container.to_owned_value());
                if let Some(control) = control {
                    return Err(control_raise(control));
                }
                outputs
                    .first()
                    .map(|v| SVal::from_owned(v, NumOrigin::Fresh))
                    .ok_or_else(|| unsupported("jv_get answered nothing")) // patchcov: coverage tolerate-line reason="defensive: the delegated `.[$k]` answers exactly one value or an error"
            }
        }
    }

    /// `jvp_array_slice`: an empty slice is a fresh `[]`; the whole root is the
    /// root itself; any other range is the one view of that range.
    fn array_slice(&self, items: &Rc<Vec<SVal>>, start: usize, end: usize) -> SVal {
        if start == end {
            return SVal::Arr(Rc::new(Vec::new()));
        }
        let (root, offset) = self
            .slice_roots
            .borrow()
            .get(&(Rc::as_ptr(items) as usize))
            .cloned()
            .unwrap_or_else(|| (items.clone(), 0));
        let (offset, size) = (offset + start, end - start);
        if offset == 0 && size == root.len() {
            return SVal::Arr(root);
        }
        let key = (Rc::as_ptr(&root) as usize, offset, size);
        if let Some(view) = self.slices.borrow().get(&key) {
            return SVal::Arr(view.clone());
        }
        let view = Rc::new(root[offset..offset + size].to_vec());
        self.slices.borrow_mut().insert(key, view.clone());
        self.slice_roots
            .borrow_mut()
            .insert(Rc::as_ptr(&view) as usize, (root, offset));
        SVal::Arr(view)
    }

    /// `EACH`/`EACH_OPT`: `path_intact` on the container (raising either
    /// way), then one output per element with the register appended.
    fn each(&self, container: SVal, reg: Reg, opt: bool, sink: &mut Sink<'_>) -> R {
        if !reg.intact(&container) {
            return Err(raise_msg(EvalError::invalid_path_expression_near_iterate(
                &container.to_owned_value(),
            )));
        }
        match &container {
            SVal::Arr(items) => {
                for (i, item) in items.iter().enumerate() {
                    let key = SVal::computed(OwnedValue::Int(i as i64));
                    sink(item.clone(), reg.clone().append(key, item.clone()))?;
                }
                Ok(())
            }
            SVal::Obj(map) => {
                for (k, item) in map.iter() {
                    let key = SVal::Str(k.clone());
                    sink(item.clone(), reg.clone().append(key, item.clone()))?;
                }
                Ok(())
            }
            _ if opt => Ok(()),
            _ => {
                let (_, control) =
                    delegate_eval(&self.prelude.iterate, &container.to_owned_value());
                Err(control.map_or_else(|| unsupported("iterate answered"), control_raise))
            }
        }
    }

    /// `gen_index`: the key is a subexp evaluated (outermost) on this node's
    /// input, then the target runs on the same input from the entry register.
    #[allow(clippy::too_many_arguments)]
    fn index_expr(
        &self,
        target: &'e Expr,
        key: &'e Expr,
        env: &Env<'e>,
        input: SVal,
        reg: Reg,
        opt: bool,
        sink: &mut Sink<'_>,
    ) -> R {
        self.eval(key, env, input.clone(), reg.sub(), &mut |k, _| {
            self.eval(target, env, input.clone(), reg.clone(), &mut |t, rt| {
                self.index(t, rt, k.clone(), opt, sink)
            })
        })
    }

    /// `gen_slice_index`: the `{"start":..,"end":..}` key is built from two
    /// subexps (start outer), then the target, then `INDEX`.
    #[allow(clippy::too_many_arguments)]
    fn slice_expr(
        &self,
        target: &'e Expr,
        start: &'e Option<Box<Expr>>,
        end: &'e Option<Box<Expr>>,
        env: &Env<'e>,
        input: SVal,
        reg: Reg,
        opt: bool,
        sink: &mut Sink<'_>,
    ) -> R {
        self.bound(start.as_deref(), env, &input, &reg, &mut |s| {
            self.bound(end.as_deref(), env, &input, &reg, &mut |e| {
                let key = slice_key(Some(s.clone()), Some(e));
                self.eval(target, env, input.clone(), reg.clone(), &mut |t, rt| {
                    self.index(t, rt, key.clone(), opt, sink)
                })
            })
        })
    }

    fn bound(
        &self,
        bound: Option<&'e Expr>,
        env: &Env<'e>,
        input: &SVal,
        reg: &Reg,
        k: &mut dyn FnMut(SVal) -> R,
    ) -> R {
        match bound {
            None => k(SVal::Null),
            Some(e) => self.eval(e, env, input.clone(), reg.sub(), &mut |v, _| k(v)),
        }
    }

    /// `Term '?'` forms: `.k?`, `.[k]?`, `.[]?` and slices compile to the
    /// `_OPT` opcodes, which suppress the get's own error but still raise a
    /// `path_intact` failure. Anything else is `try E`.
    fn optional(
        &self,
        inner: &'e Expr,
        env: &Env<'e>,
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        match inner {
            Expr::Field(name) => self.index(input, reg, SVal::string(name), true, sink),
            Expr::Index { idx, key } => {
                let key = index_key(*idx, key.as_ref());
                self.index(input, reg, key, true, sink)
            }
            Expr::Slice {
                start,
                end,
                start_key,
                end_key,
            } => {
                let key = static_slice_key(*start, *end, start_key.as_ref(), end_key.as_ref());
                self.index(input, reg, key, true, sink)
            }
            Expr::Iterate => self.each(input, reg, true, sink),
            Expr::IndexExpr { target, key } => {
                self.index_expr(target, key, env, input, reg, true, sink)
            }
            Expr::SliceExpr { target, start, end } => {
                self.slice_expr(target, start, end, env, input, reg, true, sink)
            }
            _ => self.try_catch(inner, None, env, input, reg, sink),
        }
    }

    /// `gen_collect`: a constant array is one `LOADK`; otherwise `E` runs
    /// tracked from the entry register, and the array carries the register
    /// at entry (the `FORK` restores it).
    fn collect(
        &self,
        node: &'e Expr,
        inner: &'e Expr,
        env: &Env<'e>,
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        if let Some(value) = self.constant(node) {
            return sink(value, reg);
        }
        let mut items = Vec::new();
        self.eval(inner, env, input, reg.clone(), &mut |v, _| {
            items.push(v);
            Ok(())
        })?;
        sink(SVal::Arr(Rc::new(items)), reg)
    }

    /// `gen_dictpair` per entry: key then value, both subexps, earlier
    /// entries outermost; the object is fresh and carries the entry register.
    fn object(
        &self,
        entries: &[(&'e ObjectKey, &'e Expr)],
        built: Vec<(Rc<str>, SVal)>,
        env: &Env<'e>,
        input: &SVal,
        reg: &Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        let Some(((key, value), rest)) = entries.split_first() else {
            return sink(SVal::Obj(Rc::new(built)), reg.clone());
        };
        let mut with_key = |k: SVal| -> R {
            let SVal::Str(name) = k else {
                return Err(raise_msg(EvalError::cannot_use_as_object_key(
                    &k.to_owned_value(),
                )));
            };
            self.eval(value, env, input.clone(), reg.sub(), &mut |v, _| {
                let mut next = built.clone();
                match next.iter_mut().find(|(n, _)| **n == *name) {
                    Some(slot) => slot.1 = v,
                    None => next.push((name.clone(), v)),
                }
                self.object(rest, next, env, input, reg, sink)
            })
        };
        match key {
            ObjectKey::Literal(s) => with_key(SVal::string(s)),
            ObjectKey::Expr(k) => {
                self.eval(k, env, input.clone(), reg.sub(), &mut |kv, _| with_key(kv))
            }
            ObjectKey::Bare => Err(unsupported("a bare object entry (yq only)")), // patchcov: coverage tolerate-line reason="unreachable: a bare object entry is yq syntax, never produced by the jq parser"
        }
    }

    /// A C binary builtin (`_plus`, `_equal`, ...): both operands are
    /// subexps, the right one the outer loop (`[(1,2)+(10,20)]` is
    /// `[11,12,21,22]`); the result keeps the entry register.
    #[allow(clippy::too_many_arguments)]
    fn binop(
        &self,
        left: &'e Expr,
        right: &'e Expr,
        env: &Env<'e>,
        input: &SVal,
        reg: &Reg,
        sink: &mut Sink<'_>,
        op: &mut dyn FnMut(&SVal, &SVal) -> Result<SVal, EvalError>,
    ) -> R {
        self.eval(right, env, input.clone(), reg.sub(), &mut |r, _| {
            self.eval(left, env, input.clone(), reg.sub(), &mut |l, _| {
                let value = op(&l, &r).map_err(raise_eval)?;
                sink(value, reg.clone())
            })
        })
    }

    /// `gen_and`/`gen_or`: `A` is tracked from the entry register; `B` runs
    /// on the duplicated input with the register `A` left; the bool carries
    /// the register of whichever side decided it.
    #[allow(clippy::too_many_arguments)]
    fn and_or(
        &self,
        a: &'e Expr,
        b: &'e Expr,
        is_and: bool,
        env: &Env<'e>,
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        self.eval(a, env, input.clone(), reg, &mut |av, ar| {
            if av.truthy() != is_and {
                return sink(SVal::Bool(!is_and), ar);
            }
            self.eval(b, env, input.clone(), ar, &mut |bv, br| {
                sink(SVal::Bool(bv.truthy()), br)
            })
        })
    }

    /// `gen_definedor`: `A`'s truthy outputs carry `A`'s register; if none
    /// was found, `B` starts from the entry register. An error in `A`
    /// propagates (`ON_BACKTRACK(FORK)` re-raises).
    fn defined_or(
        &self,
        a: &'e Expr,
        b: &'e Expr,
        env: &Env<'e>,
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        let mut found = false;
        self.eval(a, env, input.clone(), reg.clone(), &mut |v, r| {
            if v.truthy() {
                found = true;
                sink(v, r)
            } else {
                Ok(())
            }
        })?;
        if found {
            return Ok(());
        }
        self.eval(b, env, input, reg, sink)
    }

    /// `gen_try` (1.7.1's `TRY_BEGIN`/`TRY_END`): an error raised in the body
    /// runs the handler on its payload at the register of `try` entry; an
    /// error raised downstream of a body output passes through wrapped, and
    /// the `try` unwraps it once and re-raises it.
    fn try_catch(
        &self,
        body: &'e Expr,
        catch: Option<&'e Expr>,
        env: &Env<'e>,
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        let result = self.eval(body, env, input, reg.clone(), &mut |v, r| {
            sink(v, r).map_err(|e| Raise::Wrapped(Box::new(e)))
        });
        match result {
            Ok(()) => Ok(()),
            Err(Raise::Wrapped(inner)) => Err(*inner),
            Err(Raise::Error(payload)) => match catch {
                None => Ok(()),
                Some(handler) => self.eval(handler, env, payload, reg, sink),
            },
            Err(other) => Err(other),
        }
    }

    /// `gen_label`: `try E catch (if . == $label then empty else error)`, the
    /// label a fresh `{"__jq": n}` (`GENLABEL`).
    fn label(
        &self,
        name: &'e str,
        body: &'e Expr,
        env: &Env<'e>,
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        let n = self.next_label.get();
        self.next_label.set(n + 1);
        let label = SVal::Obj(Rc::new(vec![(
            Rc::from("__jq"),
            SVal::computed(OwnedValue::Int(n as i64)),
        )]));
        let env = Env::push(Frame::Label {
            name,
            value: label.clone(),
            parent: env.clone(),
        });
        let result = self.eval(body, &env, input, reg, &mut |v, r| {
            sink(v, r).map_err(|e| Raise::Wrapped(Box::new(e)))
        });
        match result {
            Ok(()) => Ok(()),
            Err(Raise::Wrapped(inner)) => Err(*inner),
            Err(Raise::Error(payload)) if value_eq(&payload, &label) => Ok(()),
            Err(other) => Err(other),
        }
    }

    /// `bind_alternation_matchers`: one pattern binds directly. With `?//`,
    /// every variable any alternative names starts `null`; each alternative
    /// but the last is a `DESTRUCTURE_ALT` fork, which on *any* error raised
    /// after it -- in the pattern, the body, or further downstream -- tries
    /// the next alternative from the register the source left.
    fn destructure(
        &self,
        patterns: &'e [Pattern],
        value: SVal,
        env: &Env<'e>,
        reg: Reg,
        k: &mut dyn FnMut(Env<'e>, Reg) -> R,
    ) -> R {
        if let [only] = patterns {
            return self.match_pattern(only, value, env.clone(), reg, k);
        }
        let mut names = Vec::new();
        for p in patterns {
            pattern_vars(p, &mut names);
        }
        let mut base = env.clone();
        for name in &names {
            base = base.var_ref(name, SVal::Null);
        }
        let (last, alternatives) = patterns.split_last().expect("at least one pattern");
        for p in alternatives {
            match self.match_pattern(p, value.clone(), base.clone(), reg.clone(), k) {
                Ok(()) => return Ok(()),
                Err(Raise::Error(_) | Raise::Wrapped(_)) => continue,
                Err(other) => return Err(other),
            }
        }
        self.match_pattern(last, value, base, reg, k)
    }

    /// One pattern: `$v` is a `STOREV`; `[p0, ..]` is `gen_array_matcher`,
    /// whose `INDEX`es run **last element first** on the bound value; and
    /// `{k: p, ..}` is `gen_object_matcher` per entry in order, its key a
    /// subexp of the bound value. Every `INDEX` checks and moves the register.
    fn match_pattern(
        &self,
        pattern: &'e Pattern,
        value: SVal,
        env: Env<'e>,
        reg: Reg,
        k: &mut dyn FnMut(Env<'e>, Reg) -> R,
    ) -> R {
        match pattern {
            Pattern::Var(name) => k(env.var_ref(name, value), reg),
            Pattern::Array(elems) => self.match_array(elems, elems.len(), value, env, reg, k),
            Pattern::Object(entries) => self.match_object(entries, value, env, reg, k),
        }
    }

    fn match_array(
        &self,
        elems: &'e [Pattern],
        remaining: usize,
        value: SVal,
        env: Env<'e>,
        reg: Reg,
        k: &mut dyn FnMut(Env<'e>, Reg) -> R,
    ) -> R {
        let Some(i) = remaining.checked_sub(1) else {
            return k(env, reg);
        };
        let key = SVal::computed(OwnedValue::Int(i as i64));
        self.index(value.clone(), reg, key, false, &mut |element, r| {
            self.match_pattern(&elems[i], element, env.clone(), r, &mut |env, r| {
                self.match_array(elems, i, value.clone(), env, r, k)
            })
        })
    }

    fn match_object(
        &self,
        entries: &'e [crate::jq::expr::PatternEntry],
        value: SVal,
        env: Env<'e>,
        reg: Reg,
        k: &mut dyn FnMut(Env<'e>, Reg) -> R,
    ) -> R {
        let Some((entry, rest)) = entries.split_first() else {
            return k(env, reg);
        };
        let mut with_key = |key: SVal| -> R {
            self.index(value.clone(), reg.clone(), key, false, &mut |field, r| {
                let env = match &entry.bind {
                    Some(name) => env.var_ref(name, field.clone()),
                    None => env.clone(),
                };
                self.match_pattern(&entry.pattern, field, env, r, &mut |env, r| {
                    self.match_object(rest, value.clone(), env, r, k)
                })
            })
        };
        match &entry.key {
            ObjectKey::Literal(s) => with_key(SVal::string(s)),
            ObjectKey::Expr(e) => {
                self.eval(e, &env, value.clone(), reg.sub(), &mut |kv, _| with_key(kv))
            }
            ObjectKey::Bare => Err(unsupported("a bare pattern key")), // patchcov: coverage tolerate-line reason="unreachable: a bare pattern key is yq syntax, never produced by the jq parser"
        }
    }

    /// `gen_reduce`: `DUP; INIT; STOREV; FORK [DUPN; SOURCE; P; LOADVN;
    /// UPDATE; STOREV; BACKTRACK]; LOADVN`. Each step's `SOURCE` runs on the
    /// fold's input from the register `INIT` left; `UPDATE` starts at the
    /// register the source emission and pattern left, and its movement is
    /// undone; the state is the last `UPDATE` output, or `null` if there was
    /// none (`LOADVN`); the result carries the register after `INIT`. Only
    /// the first `INIT` output's `SOURCE` sees the input ([`FoldInput`]).
    #[allow(clippy::too_many_arguments)]
    fn reduce(
        &self,
        source: &'e Expr,
        patterns: &'e [Pattern],
        init: &'e Expr,
        update: &'e Expr,
        env: &Env<'e>,
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        let fold_input = FoldInput::new(input.clone());
        self.eval(init, env, input, reg, &mut |initial, reg_init| {
            let state = RefCell::new(initial);
            self.eval(
                source,
                env,
                fold_input.take(),
                reg_init.clone(),
                &mut |s, reg_s| {
                    self.destructure(patterns, s, env, reg_s, &mut |env_p, reg_p| {
                        let current = state.replace(SVal::Null);
                        self.eval(update, &env_p, current, reg_p, &mut |u, _| {
                            *state.borrow_mut() = u;
                            Ok(())
                        })
                    })
                },
            )?;
            sink(state.into_inner(), reg_init)
        })
    }

    /// `gen_foreach`: as `reduce`, but `EXTRACT` runs straight after each
    /// `UPDATE` output, on that output and at the register `UPDATE` left.
    #[allow(clippy::too_many_arguments)]
    fn foreach(
        &self,
        source: &'e Expr,
        patterns: &'e [Pattern],
        init: &'e Expr,
        update: &'e Expr,
        extract: Option<&'e Expr>,
        env: &Env<'e>,
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        let fold_input = FoldInput::new(input.clone());
        self.eval(init, env, input, reg, &mut |initial, reg_init| {
            let state = RefCell::new(initial);
            self.eval(source, env, fold_input.take(), reg_init, &mut |s, reg_s| {
                self.destructure(patterns, s, env, reg_s, &mut |env_p, reg_p| {
                    let current = state.replace(SVal::Null);
                    self.eval(update, &env_p, current, reg_p, &mut |u, reg_u| {
                        *state.borrow_mut() = u.clone();
                        match extract {
                            None => sink(u, reg_u),
                            Some(x) => self.eval(x, &env_p, u, reg_u, sink),
                        }
                    })
                })
            })
        })
    }

    /// `range/2` (bytecoded): `start` and `end` are closure calls, so both
    /// are tracked, `end` from the register `start` left; `RANGE` emits the
    /// start value itself first, then computed successors.
    fn range(
        &self,
        from: &'e Expr,
        to: &'e Expr,
        env: &Env<'e>,
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        self.eval(from, env, input.clone(), reg, &mut |start, reg_s| {
            self.eval(to, env, input.clone(), reg_s, &mut |end, reg_e| {
                let (Some(mut cur), Some(max)) = (start.f64(), end.f64()) else {
                    return Err(Raise::Error(SVal::string("Range bounds must be numeric")));
                };
                let mut first = Some(start.clone());
                // `RANGE` stops only on `>=`, which a NaN bound never is.
                loop {
                    if cur >= max {
                        return Ok(());
                    }
                    let value = first
                        .take()
                        .unwrap_or_else(|| SVal::computed(number_owned(cur)));
                    sink(value, reg_e.clone())?;
                    cur += 1.0;
                }
            })
        })
    }

    /// `PATH_BEGIN … PATH_END`: a fresh register on the input; each output
    /// must be identical to where the register ended, and yields the path
    /// with the outer register restored.
    fn path_mode(
        &self,
        f: &'e Expr,
        env: &Env<'e>,
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        let inner = Reg {
            path: Some(PathList::default()),
            node: input.clone(),
            nest: 0,
        };
        self.eval(f, env, input, inner, &mut |v, r| {
            if !r.intact(&v) {
                return Err(raise_msg(EvalError::invalid_path_expression(
                    &v.to_owned_value(),
                )));
            }
            let path = r.path.as_ref().map_or(SVal::Null, PathList::to_array);
            sink(path, reg.clone())
        })
    }

    /// `getpath(p)`, the one register-aware C builtin: `p` is a subexp; on an
    /// input identical to the register's node the path is extended and the
    /// register moves to the result (`_jq_path_append`); otherwise the value
    /// is returned and the register is untouched.
    fn getpath(&self, p: &'e Expr, env: &Env<'e>, input: SVal, reg: Reg, sink: &mut Sink<'_>) -> R {
        self.eval(p, env, input.clone(), reg.sub(), &mut |path, _| {
            let SVal::Arr(keys) = &path else {
                return Err(raise_msg(EvalError::path_must_be_array()));
            };
            let mut value = input.clone();
            for key in keys.iter() {
                value = self.jv_get(&value, key)?;
            }
            if !reg.tracking() || !identical(&input, &reg.node) {
                return sink(value, reg.clone());
            }
            let mut moved = reg.clone();
            let mut list = moved.path.take().unwrap_or_default();
            for key in keys.iter() {
                list = list.push(key.clone());
            }
            moved.path = Some(list);
            moved.node = value.clone();
            sink(value, moved)
        })
    }

    fn builtin(
        &self,
        node: &'e Expr,
        builtin: &'e Builtin,
        env: &Env<'e>,
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        match builtin {
            Builtin::Path(f) => self.path_mode(f, env, input, reg, sink),
            Builtin::GetPath(p) => self.getpath(p, env, input, reg, sink),
            Builtin::Empty => Ok(()),
            _ => {
                if let Some((name, args)) = prelude_call(builtin) {
                    return self.call_prelude(name, &args, env, input, reg, sink);
                }
                if let Some(why) = unsupported_builtin(builtin) {
                    return Err(unsupported(why));
                }
                self.c_builtin(node, builtin, env, input, reg, sink)
            }
        }
    }

    /// A C builtin: its arguments are subexps the spike evaluates itself
    /// (so a closure parameter in one resolves here), then the call is
    /// delegated with the argument values substituted, and answers fresh.
    fn c_builtin(
        &self,
        node: &'e Expr,
        builtin: &'e Builtin,
        env: &Env<'e>,
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        if stub_risk(builtin) {
            self.note_stub(&variant_name(node));
        }
        if let Some(arg) = builtin.value_arg() {
            let call = builtin
                .with_value_arg(Expr::Var(A_VAR.to_string()))
                .expect("value_arg implies with_value_arg");
            let call = Expr::Builtin(call);
            return self.eval(arg, env, input.clone(), reg.sub(), &mut |a, _| {
                let expr = substitute_vars(&call, [(A_VAR, &a.to_owned_value())]);
                emit_delegated(&expr, &input, &reg, sink)
            });
        }
        if let Builtin::SetPath(p, v) = builtin {
            // Two C arguments: the last is the outer loop.
            let call = Expr::Builtin(Builtin::SetPath(
                Box::new(Expr::Var(A_VAR.to_string())),
                Box::new(Expr::Var(B_VAR.to_string())),
            ));
            return self.eval(v, env, input.clone(), reg.sub(), &mut |bv, _| {
                self.eval(p, env, input.clone(), reg.sub(), &mut |av, _| {
                    let expr = substitute_vars(
                        &call,
                        [(A_VAR, &av.to_owned_value()), (B_VAR, &bv.to_owned_value())],
                    );
                    emit_delegated(&expr, &input, &reg, sink)
                })
            });
        }
        delegate(node, env, input, reg, sink)
    }

    fn note_stub(&self, name: &str) {
        let mut stubbed = self.stubbed.borrow_mut();
        if !stubbed.iter().any(|s| s == name) {
            stubbed.push(name.to_string());
        }
    }

    /// The C functions the prelude's `builtin.jq` bodies call that succinctly
    /// has no builtin for. Each argument is a subexp; the sort, group and
    /// min/max helpers keep the input's elements (`jv_copy`), so their
    /// identity survives as it does in jq.
    fn hook(
        &self,
        name: &str,
        args: &'e [Expr],
        env: &Env<'e>,
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        let undefined = || unsupported(format!("{name}/{} is not defined", args.len()));
        let [arg] = args else {
            return Err(undefined());
        };
        let hook = match name {
            "_sp_hook_sort_by" => Hook::SortBy,
            "_sp_hook_group_by" => Hook::GroupBy,
            "_sp_hook_min_by" => Hook::MinBy,
            "_sp_hook_max_by" => Hook::MaxBy,
            "_sp_hook_strindices" => Hook::StrIndices,
            _ => return Err(undefined()),
        };
        self.eval(arg, env, input.clone(), reg.sub(), &mut |a, _| match hook {
            Hook::SortBy => sink(sort_by_keys(&input, &a, false)?, reg.clone()),
            Hook::GroupBy => sink(sort_by_keys(&input, &a, true)?, reg.clone()),
            Hook::MinBy => sink(min_max_by(&input, &a, true)?, reg.clone()),
            Hook::MaxBy => sink(min_max_by(&input, &a, false)?, reg.clone()),
            Hook::StrIndices => {
                let call = Expr::Builtin(Builtin::Indices(Box::new(Expr::Var(A_VAR.to_string()))));
                let expr = substitute_vars(&call, [(A_VAR, &a.to_owned_value())]);
                emit_delegated(&expr, &input, &reg, sink)
            }
        })
    }

    fn call_prelude(
        &self,
        name: &str,
        args: &[&'e Expr],
        env: &Env<'e>,
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        let callee = self
            .prelude_env
            .lookup_func(name, args.len())
            .ok_or_else(|| unsupported(format!("prelude {name}/{}", args.len())))?;
        self.call(callee, args, env, input, reg, sink)
    }

    /// `CALL_JQ`: closure parameters bind the argument expressions with the
    /// caller's environment; a `$name` parameter is jq's `name as $name`
    /// desugaring, its argument a subexp, earlier parameters outermost.
    fn call(
        &self,
        callee: Callee<'e>,
        args: &[&'e Expr],
        caller: &Env<'e>,
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        let args: Vec<(&'e Expr, Env<'e>)> = args.iter().map(|a| (*a, caller.clone())).collect();
        self.call_with(callee, &args, input, reg, sink)
    }

    /// [`Self::call`] with each argument closing over its own environment.
    fn call_with(
        &self,
        callee: Callee<'e>,
        args: &[(&'e Expr, Env<'e>)],
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        match callee {
            Callee::Arg { body, env } => self.eval(body, &env, input, reg, sink),
            Callee::Def { params, body, env } => {
                let mut body_env = env;
                for (param, (arg, arg_env)) in params.iter().zip(args) {
                    body_env = Env::push(Frame::Arg {
                        name: param.name(),
                        body: arg,
                        env: arg_env.clone(),
                        parent: body_env.clone(),
                    });
                }
                self.bind_dollars(params, body, body_env, input, reg, sink)
            }
        }
    }

    fn bind_dollars(
        &self,
        params: &'e [Param],
        body: &'e Expr,
        env: Env<'e>,
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        let Some((param, rest)) = params.split_first() else {
            return self.eval(body, &env, input, reg, sink);
        };
        let Param::Dollar(name) = param else {
            return self.bind_dollars(rest, body, env, input, reg, sink);
        };
        let callee = env
            .lookup_func(name, 0)
            .ok_or_else(|| unsupported(format!("${name} parameter")))?;
        self.call(callee, &[], &env, input.clone(), reg.sub(), &mut |v, _| {
            let env = env.var_ref(name, v);
            self.bind_dollars(rest, body, env, input.clone(), reg.clone(), sink)
        })
    }

    /// `gen_update`/`gen_definedor_assign`: `DUP; VALUE; STOREV tmp;
    /// _modify(path; . op $tmp)`. `VALUE` is tracked (not a subexp), so its
    /// register effect carries into the `_modify` call.
    #[allow(clippy::too_many_arguments)]
    fn update_with(
        &self,
        name: &str,
        path: &'e Expr,
        value: &'e Expr,
        env: &Env<'e>,
        input: SVal,
        reg: Reg,
        sink: &mut Sink<'_>,
    ) -> R {
        self.eval(value, env, input.clone(), reg, &mut |t, reg_t| {
            // `path` closes over the caller's environment; the re-read
            // `$__sp_t` over one that binds the value just computed.
            let env_t = env.var_ref(T_VAR, t);
            let callee = self
                .prelude_env
                .lookup_func(name, 2)
                .ok_or_else(|| unsupported(format!("prelude {name}/2")))?;
            let args = [(path, env.clone()), (&self.prelude.t_var, env_t)];
            self.call_with(callee, &args, input.clone(), reg_t, sink)
        })
    }
}

/// The fold's input as `DUPN` leaves it: `gen_reduce`/`gen_foreach` start
/// each loop with `DUPN`, which *moves* the input out of a stack slot that
/// `INIT`'s fork point shares, so every `INIT` output after the first runs
/// `SOURCE` on `null` (`{"a":1} | [reduce .a as $i ((1,2); [$i,.])]` is
/// `[[1,1],[null,2]]` in jq 1.7.1, which value mode already reproduces).
struct FoldInput(RefCell<Option<SVal>>);

impl FoldInput {
    fn new(input: SVal) -> Self {
        Self(RefCell::new(Some(input)))
    }

    fn take(&self) -> SVal {
        self.0.borrow_mut().take().unwrap_or(SVal::Null)
    }
}

/// The prelude's C helpers ([`Interp::hook`]).
#[derive(Clone, Copy)]
enum Hook {
    SortBy,
    GroupBy,
    MinBy,
    MaxBy,
    StrIndices,
}

/// `type_error2`: `"<kind> (<dump>) and <kind> (<dump>) <msg>"`.
fn type_error2(a: &SVal, b: &SVal, msg: &str) -> Raise {
    let (a, b) = (a.to_owned_value(), b.to_owned_value());
    Raise::Error(SVal::string(&format!(
        "{} ({}) and {} ({}) {msg}",
        a.type_name(),
        dump_truncated_with(EvalTag::Jq, &a),
        b.type_name(),
        dump_truncated_with(EvalTag::Jq, &b)
    )))
}

/// `_sort_by_impl`/`_group_by_impl`: a stable sort of the input's elements by
/// `keys` (`jv_sort`/`jv_group`), grouping runs of equal keys when `group`.
fn sort_by_keys(input: &SVal, keys: &SVal, group: bool) -> Result<SVal, Raise> {
    let (SVal::Arr(items), SVal::Arr(ks)) = (input, keys) else {
        return Err(type_error2(
            input,
            keys,
            "cannot be sorted, as they are not both arrays",
        ));
    };
    if items.len() != ks.len() {
        return Err(type_error2(
            input,
            keys,
            "cannot be sorted, as they are not both arrays",
        ));
    }
    let owned: Vec<OwnedValue> = ks.iter().map(SVal::to_owned_value).collect();
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| compare_values::<JqSemantics>(&owned[a], &owned[b]));
    if !group {
        return Ok(SVal::Arr(Rc::new(
            order.iter().map(|&i| items[i].clone()).collect(),
        )));
    }
    let mut groups: Vec<SVal> = Vec::new();
    let mut current: Vec<SVal> = Vec::new();
    let mut key: Option<usize> = None;
    for &i in &order {
        if let Some(k) = key {
            if compare_values::<JqSemantics>(&owned[k], &owned[i]) != core::cmp::Ordering::Equal {
                groups.push(SVal::Arr(Rc::new(core::mem::take(&mut current))));
                key = Some(i);
            }
        } else {
            key = Some(i);
        }
        current.push(items[i].clone());
    }
    if key.is_some() {
        groups.push(SVal::Arr(Rc::new(current)));
    }
    Ok(SVal::Arr(Rc::new(groups)))
}

/// `minmax_by`: the first element with the least key, or the last with the
/// greatest; `null` for an empty array.
fn min_max_by(values: &SVal, keys: &SVal, is_min: bool) -> Result<SVal, Raise> {
    let (SVal::Arr(items), SVal::Arr(ks)) = (values, keys) else {
        return Err(type_error2(values, keys, "cannot be iterated over"));
    };
    if items.len() != ks.len() {
        return Err(type_error2(values, keys, "have wrong length"));
    }
    let Some(first) = items.first() else {
        return Ok(SVal::Null);
    };
    let owned: Vec<OwnedValue> = ks.iter().map(SVal::to_owned_value).collect();
    let (mut best, mut best_key) = (first, 0);
    for i in 1..items.len() {
        let less =
            compare_values::<JqSemantics>(&owned[i], &owned[best_key]) == core::cmp::Ordering::Less;
        if less == is_min {
            best = &items[i];
            best_key = i;
        }
    }
    Ok(best.clone())
}

/// A builtin the fresh stub may answer differently from jq on identity: a C
/// builtin jq lets hand back its input (`ltrimstr`/`rtrimstr` with no match,
/// `tostring` of a string, `tonumber` of a number, `delpaths([])`,
/// `setpath([]; v)`, `min`/`max` and `sort`, whose elements are the input's),
/// or a jq-defined builtin the prelude does not run, whose body navigates its
/// input.
fn stub_risk(builtin: &Builtin) -> bool {
    use Builtin as B;
    matches!(
        builtin,
        B::Ltrimstr(_)
            | B::Rtrimstr(_)
            | B::ToString
            | B::ToNumber
            | B::DelPaths(_)
            | B::SetPath(..)
            | B::Min
            | B::Max
            | B::Sort
            | B::Match(_)
            | B::MatchFlags(..)
            | B::Capture(_)
            | B::CaptureFlags(..)
            | B::Scan(_)
            | B::ScanFlags(..)
            | B::Splits(_)
            | B::SplitsFlags(..)
            | B::SplitRegex(..)
            | B::Sub(..)
            | B::SubFlags(..)
            | B::Gsub(..)
            | B::GsubFlags(..)
            | B::Transpose
            | B::Combinations
            | B::CombinationsN(_)
            | B::ToStream
            | B::FromStream(_)
            | B::TruncateStream(_)
            | B::BSearch(_)
            | B::UpperIndex(_)
            | B::UpperIndexStream(..)
    )
}

/// The jq-defined builtins the prelude implements, as `(definition, args)`.
fn prelude_call(builtin: &Builtin) -> Option<(&'static str, Vec<&Expr>)> {
    use Builtin as B;
    let call: (&'static str, Vec<&Expr>) = match builtin {
        B::Select(f) => ("_sp_select", vec![f]),
        B::Map(f) => ("_sp_map", vec![f]),
        B::MapValues(f) => ("_sp_map_values", vec![f]),
        B::Recurse => ("_sp_recurse", vec![]),
        B::RecurseF(f) => ("_sp_recurse", vec![f]),
        B::RecurseCond(f, c) => ("_sp_recurse", vec![f, c]),
        B::Add => ("_sp_add", vec![]),
        B::Del(f) => ("_sp_del", vec![f]),
        B::ToEntries => ("_sp_to_entries", vec![]),
        B::FromEntries => ("_sp_from_entries", vec![]),
        B::WithEntries(f) => ("_sp_with_entries", vec![f]),
        B::Reverse => ("_sp_reverse", vec![]),
        B::Paths => ("_sp_paths", vec![]),
        B::PathsFilter(f) => ("_sp_paths", vec![f]),
        B::Arrays => ("_sp_arrays", vec![]),
        B::Objects => ("_sp_objects", vec![]),
        B::Iterables => ("_sp_iterables", vec![]),
        B::Booleans => ("_sp_booleans", vec![]),
        B::Numbers => ("_sp_numbers", vec![]),
        B::Normals => ("_sp_normals", vec![]),
        B::Finites => ("_sp_finites", vec![]),
        B::Strings => ("_sp_strings", vec![]),
        B::Nulls => ("_sp_nulls", vec![]),
        B::Values => ("_sp_values", vec![]),
        B::Scalars => ("_sp_scalars", vec![]),
        B::Join(x) => ("_sp_join", vec![x]),
        B::Flatten => ("_sp_flatten", vec![]),
        B::FlattenDepth(x) => ("_sp_flatten", vec![x]),
        B::Limit(n, g) => ("_sp_limit", vec![n, g]),
        B::FirstStream(g) => ("_sp_first", vec![g]),
        B::LastStream(g) => ("_sp_last", vec![g]),
        B::NthStream(n, g) => ("_sp_nth", vec![n, g]),
        B::IsEmpty(g) => ("_sp_isempty", vec![g]),
        B::Any => ("_sp_any", vec![]),
        B::All => ("_sp_all", vec![]),
        B::AnyF(c) => ("_sp_any", vec![c]),
        B::AllF(c) => ("_sp_all", vec![c]),
        B::AnyCond(g, c) => ("_sp_any", vec![g, c]),
        B::AllCond(g, c) => ("_sp_all", vec![g, c]),
        B::First => ("_sp_first", vec![]),
        B::Last => ("_sp_last", vec![]),
        B::Nth(n) => ("_sp_nth", vec![n]),
        B::In(xs) => ("_sp_in", vec![xs]),
        B::Inside(xs) => ("_sp_inside", vec![xs]),
        B::UpperIn(s) => ("_sp_IN", vec![s]),
        B::UpperInSrc(src, s) => ("_sp_IN", vec![src, s]),
        B::AsciiDowncase => ("_sp_ascii_downcase", vec![]),
        B::AsciiUpcase => ("_sp_ascii_upcase", vec![]),
        B::Walk(f) => ("_sp_walk", vec![f]),
        B::Pick(f) => ("_sp_pick", vec![f]),
        B::Abs => ("_sp_abs", vec![]),
        B::SortBy(f) => ("_sp_sort_by", vec![f]),
        B::GroupBy(f) => ("_sp_group_by", vec![f]),
        B::Unique => ("_sp_unique", vec![]),
        B::UniqueBy(f) => ("_sp_unique_by", vec![f]),
        B::MinBy(f) => ("_sp_min_by", vec![f]),
        B::MaxBy(f) => ("_sp_max_by", vec![f]),
        B::Indices(i) => ("_sp_indices", vec![i]),
        B::Index(i) => ("_sp_index", vec![i]),
        B::Rindex(i) => ("_sp_rindex", vec![i]),
        B::DebugMsg(m) => ("_sp_debug", vec![m]),
        _ => return None,
    };
    Some(call)
}

/// Builtins the spike refuses outright: input-reading ones, succinctly
/// extensions with no jq 1.7.1 model, and yq-only ones that reach path mode.
fn unsupported_builtin(builtin: &Builtin) -> Option<&'static str> {
    use Builtin as B;
    Some(match builtin {
        B::Input | B::Inputs | B::InputLineNumber | B::InputFilename => {
            "input-reading builtins (the spike has no input queue)"
        }
        B::HaltError | B::HaltErrorCode(_) => "halt_error (its message is not carried)",
        B::LeafPaths => "leaf_paths (a succinctly extension)",
        B::AtOffset(_) | B::AtPosition(..) => "at_offset/at_position (a succinctly extension)",
        B::PathNoArg | B::Parent | B::ParentN(_) => "a yq path builtin",
        B::IsValid(_) | B::Skip(..) | B::AnyC(..) | B::AllC(..) | B::RecurseDown => {
            "a builtin jq 1.7.1 does not define"
        }
        _ => return None,
    })
}

/// `parse_slice` for an array of `len` elements (or a string of `len`
/// codepoints): the `[start, end)` range a
/// `{"start": s, "end": e}` key selects (`null` bounds default; the start
/// rounds down and the end up). `None` when a bound is not a number, which
/// `jv_get` reports as an error.
fn slice_bounds(len: usize, bounds: &[(Rc<str>, SVal)]) -> Option<(usize, usize)> {
    let bound = |name: &str, default: f64| -> Option<f64> {
        match bounds.iter().find(|(k, _)| &**k == name).map(|(_, v)| v) {
            None | Some(SVal::Null) => Some(default),
            Some(v @ SVal::Num(..)) => v.f64(),
            Some(_) => None,
        }
    };
    let n = len as f64;
    let (mut dstart, mut dend) = (bound("start", 0.0)?, bound("end", n)?);
    if dstart.is_nan() {
        dstart = 0.0;
    }
    if dstart < 0.0 {
        dstart += n;
    }
    let start = dstart.clamp(0.0, n) as usize;
    if dend.is_nan() {
        dend = n;
    }
    if dend < 0.0 {
        dend += n;
    }
    let mut end = if dend < 0.0 {
        start
    } else {
        dend.min(f64::from(i32::MAX)) as usize
    };
    end = end.min(len);
    if end < len && (end as f64) < dend {
        end += 1;
    }
    Some((start, end.max(start)))
}

/// `-1` is `_negate` applied to the literal `1` at run time (`'-' Exp`), so a
/// negative spelling is computed, never a constant.
fn literal_value(lit: &Literal) -> Option<SVal> {
    Some(match lit {
        Literal::Null => SVal::Null,
        Literal::Bool(b) => SVal::Bool(*b),
        Literal::NumberLiteral(_, spelling) if spelling.starts_with('-') => return None,
        Literal::NumberLiteral(repr, spelling) => {
            SVal::literal(OwnedValue::NumberLiteral(*repr, spelling.as_str().into()))
        }
        Literal::Int(i) => SVal::literal(OwnedValue::Int(*i)),
        Literal::Float(f) => SVal::literal(OwnedValue::Float(*f)),
        Literal::String(s) => SVal::string(s),
    })
}

/// What jq compiles to one constant (`LOADK`): a literal, and arrays and
/// objects of constants that `gen_const_array`/`gen_const_object` fold, and a
/// binary operator over two constants (`constant_fold`).
fn constant_value(expr: &Expr) -> Option<SVal> {
    match expr {
        Expr::Literal(lit) => literal_value(lit),
        Expr::Paren(inner) => constant_value(inner),
        Expr::Loc { line, file } => Some(SVal::Obj(Rc::new(vec![
            (
                Rc::from("file"),
                SVal::string(file.as_deref().unwrap_or("<top-level>")),
            ),
            (
                Rc::from("line"),
                SVal::literal(OwnedValue::Int(*line as i64)),
            ),
        ]))),
        Expr::Array(inner) => {
            let elements: Vec<&Expr> = match &**inner {
                Expr::Comma(items) => items.iter().collect(),
                single => vec![single],
            };
            let values: Option<Vec<SVal>> = elements.into_iter().map(constant_value).collect();
            Some(SVal::Arr(Rc::new(values?)))
        }
        Expr::Object(entries) => {
            let mut built: Vec<(Rc<str>, SVal)> = Vec::new();
            for entry in entries {
                let ObjectKey::Literal(key) = &entry.key else {
                    return None;
                };
                let value = constant_value(&entry.value)?;
                match built.iter_mut().find(|(k, _)| **k == **key) {
                    Some(slot) => slot.1 = value,
                    None => built.push((Rc::from(key.as_str()), value)),
                }
            }
            Some(SVal::Obj(Rc::new(built)))
        }
        Expr::Arithmetic {
            op, left, right, ..
        } => {
            let (l, r) = (constant_value(left)?, constant_value(right)?);
            arith_combine::<JqSemantics>(*op, l.to_owned_value(), r.to_owned_value())
                .ok()
                .map(|v| SVal::from_owned(&v, NumOrigin::Fresh))
        }
        Expr::Compare { op, left, right } => {
            let (l, r) = (constant_value(left)?, constant_value(right)?);
            Some(SVal::Bool(apply_compare_op::<JqSemantics>(
                *op,
                &l.to_owned_value(),
                &r.to_owned_value(),
            )))
        }
        _ => None,
    }
}

fn pattern_vars<'e>(pattern: &'e Pattern, out: &mut Vec<&'e str>) {
    match pattern {
        Pattern::Var(name) => {
            if !out.contains(&name.as_str()) {
                out.push(name);
            }
        }
        Pattern::Array(items) => items.iter().for_each(|p| pattern_vars(p, out)),
        Pattern::Object(entries) => {
            for entry in entries {
                if let Some(name) = &entry.bind {
                    if !out.contains(&name.as_str()) {
                        out.push(name);
                    }
                }
                pattern_vars(&entry.pattern, out);
            }
        }
    }
}

/// `.[n]`'s key: a program literal, `_negate` of one for a negative index,
/// or the number it was spelled as (`.[1.0]` records `1.0`).
fn index_key(idx: i64, spelled: Option<&NumberKey>) -> SVal {
    spelled.map_or_else(|| literal_int(idx), number_key)
}

/// A spelled number key: a source literal keeps its spelling and is
/// allocated; one the parser computed (a negated literal) is a double.
fn number_key(key: &NumberKey) -> SVal {
    match key {
        NumberKey::Float(f) => SVal::computed(OwnedValue::Float(*f)),
        NumberKey::Literal(f, spelling) => SVal::literal(OwnedValue::NumberLiteral(
            NumberRepr::Float(*f),
            (**spelling).into(),
        )),
    }
}

/// A static slice's key as `gen_slice_index` builds it: each bound's own
/// spelling when it kept one, the raw value jq never parsed (a `null`
/// target), else the integer literal (`null` when absent); or, for a
/// descriptor the program named whole (`SliceBoundKey::Verbatim`), that
/// object as written.
fn static_slice_key(
    start: Option<i64>,
    end: Option<i64>,
    start_key: Option<&SliceBoundKey>,
    end_key: Option<&SliceBoundKey>,
) -> SVal {
    if let Some(SliceBoundKey::Verbatim(descriptor)) = start_key {
        return SVal::from_owned(descriptor, NumOrigin::Fresh);
    }
    let bound = |n: Option<i64>, key: Option<&SliceBoundKey>| match key {
        Some(SliceBoundKey::Number(k)) => Some(number_key(k)),
        Some(SliceBoundKey::Raw(value) | SliceBoundKey::Verbatim(value)) => {
            Some(SVal::from_owned(value, NumOrigin::Fresh))
        }
        None => n.map(literal_int),
    };
    slice_key(bound(start, start_key), bound(end, end_key))
}

fn literal_int(n: i64) -> SVal {
    if n < 0 {
        SVal::computed(OwnedValue::Int(n))
    } else {
        SVal::literal(OwnedValue::Int(n))
    }
}

/// `gen_slice_index`'s key, `{"start": s, "end": e}` (an absent bound is
/// `null`), fresh per evaluation.
fn slice_key(start: Option<SVal>, end: Option<SVal>) -> SVal {
    SVal::Obj(Rc::new(vec![
        (Rc::from("start"), start.unwrap_or(SVal::Null)),
        (Rc::from("end"), end.unwrap_or(SVal::Null)),
    ]))
}

fn number_owned(value: f64) -> OwnedValue {
    if value.fract() == 0.0 && value.abs() < 9.0e15 {
        OwnedValue::Int(value as i64)
    } else {
        OwnedValue::Float(value)
    }
}

/// Run `expr` through the existing evaluator with the variables it reads
/// substituted. Declines (unsupported) when it calls a function the
/// spike's environment holds, or breaks to a spike label.
fn delegate<'e>(expr: &'e Expr, env: &Env<'e>, input: SVal, reg: Reg, sink: &mut Sink<'_>) -> R {
    let mut names: Vec<String> = Vec::new();
    let mut blocked: Option<String> = None;
    any_subexpr(expr, &mut |e| {
        match e {
            Expr::Var(name) => names.push(name.clone()),
            Expr::FuncCall { name, .. } => {
                blocked = Some(format!("a closure call ({name}) inside a C builtin"));
            }
            Expr::Break(_) | Expr::Label { .. } => {
                blocked = Some("a label inside a C builtin".to_string());
            }
            _ => {}
        }
        blocked.is_some()
    });
    if let Some(why) = blocked {
        return Err(unsupported(why));
    }
    let bound: Vec<(&str, OwnedValue)> = names
        .iter()
        .filter_map(|n| env.lookup_var(n).map(|v| (n.as_str(), v.to_owned_value())))
        .collect();
    let expr = substitute_vars(expr, bound.iter().map(|(n, v)| (*n, v)));
    emit_delegated(&expr, &input, &reg, sink)
}

/// Emit what the existing evaluator answers for `expr` on `input`, each
/// output fresh and at `reg`.
fn emit_delegated(expr: &Expr, input: &SVal, reg: &Reg, sink: &mut Sink<'_>) -> R {
    let (outputs, control) = delegate_eval(expr, &input.to_owned_value());
    for v in outputs {
        sink(SVal::from_owned(&v, NumOrigin::Fresh), reg.clone())?;
    }
    match control {
        Some(control) => Err(control_raise(control)),
        None => Ok(()),
    }
}

/// Run `expr` on `input` through the existing evaluator: its outputs, and
/// the escape that ended it.
fn delegate_eval(expr: &Expr, input: &OwnedValue) -> (Vec<OwnedValue>, Option<Control>) {
    let result = eval_owned_unrouted::<Vec<u64>, JqSemantics>(expr, input, &[]);
    match result {
        QueryResult::One(v) => match crate::jq::eval_generic::to_owned::<JqSemantics, _>(&v) {
            Ok(o) => (vec![o], None),
            Err(e) => (vec![], Some(Control::Error(e))),
        },
        QueryResult::OneCursor(c) => {
            match crate::jq::eval_generic::to_owned::<JqSemantics, _>(&c.value()) {
                Ok(o) => (vec![o], None),
                Err(e) => (vec![], Some(Control::Error(e))),
            }
        }
        QueryResult::Many(vs) => {
            let mut out = Vec::new();
            for v in &vs {
                match crate::jq::eval_generic::to_owned::<JqSemantics, _>(v) {
                    Ok(o) => out.push(o),
                    Err(e) => return (out, Some(Control::Error(e))),
                }
            }
            (out, None)
        }
        QueryResult::None => (vec![], None),
        QueryResult::Error(e) => (vec![], Some(Control::Error(e))),
        QueryResult::Owned(v) => (vec![v], None),
        QueryResult::ManyOwned(vs) => (vs, None),
        QueryResult::Break(label) => (vec![], Some(Control::Break(label))),
        QueryResult::Halt(code) => (vec![], Some(Control::Halt(code))),
        QueryResult::Partial(vs, control) => (vs, Some(control)),
    }
}

fn control_raise(control: Control) -> Raise {
    match control {
        Control::Error(e) => raise_eval(e),
        Control::Break(label) => unsupported(format!("a delegated break to {label}")), // patchcov: coverage tolerate-line reason="unreachable: delegate() declines any expression holding a break before delegating it"
        Control::Halt(code) => Raise::Halt(code),
    }
}

fn variant_name(expr: &Expr) -> String {
    let debug = match expr {
        Expr::Builtin(builtin) => format!("{builtin:?}"),
        other => format!("{other:?}"), // patchcov: coverage tolerate-line reason="defensive: only reached for an Expr variant the spike does not implement"
    };
    debug
        .split(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap_or("?")
        .to_string()
}
