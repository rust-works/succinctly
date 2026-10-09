//! Compile-time resolution of function calls (#1473).
//!
//! Real jq resolves every function call *before* any input is read. A call to
//! an undefined function — or to an undefined arity of an existing one — is a
//! compile error: unconditional, unaffected by whether the branch containing
//! it is ever reached, and uncatchable by `try`/`catch` or `?`, since
//! compilation fails before evaluation (and thus before `try`) ever begins.
//! Exit code 3.
//!
//! succinctly resolves user-defined `def`s by static AST substitution
//! (`expand_func_calls`, `src/jq/eval.rs`), which runs *during* evaluation and
//! leaves an unresolvable call in place to be discovered lazily. Three
//! divergences followed, all confirmed live against jq 1.7.1:
//!
//! - an arity-mismatched call in a never-taken branch never errored at all;
//! - the error was swallowable by `try`/`?`;
//! - it surfaced as a runtime error (exit 5), not a compile error (exit 3).
//!
//! and one silently-wrong result, the severe case: substitution has no notion
//! of *lexical position*, so a body substituted into a later call site becomes
//! indistinguishable from a call genuinely written there, and a later `def`'s
//! own expansion pass resolves it. Both `def f(x): f(x; 99); def f(x; y): x +
//! y; f(1)` and `def f: g; def g: 42; f` are forward references real jq
//! rejects, and both computed a value instead.
//!
//! ## Why a *check* is enough
//!
//! The issue scoped this as needing a real resolution mechanism. It does not.
//! `src/jq/eval.rs` has exactly one evaluation arm for `Expr::FuncCall`, and it
//! routes unconditionally to `eval_func_call`, which always returns an error —
//! so *any* residual `Expr::FuncCall` reaching evaluation is already an error
//! today. This pass therefore introduces no new error class for a call that is
//! actually reached; it only moves the error earlier and extends it to the
//! unreached, caught and forward-referencing cases jq also rejects.
//!
//! `expand_func_calls`'s substitution model is left exactly as it was. The
//! programs it mis-resolves are simply rejected before it ever runs.
//!
//! ## Scope rules
//!
//! Only [`Expr::FuncDef`] changes function scope — `as`/`reduce`/`foreach`
//! patterns bind variables and `label` binds labels, never functions. Each rule
//! below was verified against jq 1.7.1 rather than taken from the manual:
//!
//! | program | jq 1.7.1 |
//! |---|---|
//! | `def f: 1; def g: f; g` | `1` — a def is visible to later siblings |
//! | `def f: g; def g: 42; f` | error — but *not* to earlier ones |
//! | `def f: def g: 1; g; g` | error — a nested def does not leak |
//! | `def f($a): a; f(1)` | `1` — a `$`-param binds the bare name too |
//! | `def f(g): g(1); f(.)` | error `g/1` — a param binds arity 0 only |
//! | `def f: 1; def g: f; def f: 2; g` | `1` — later same-arity def shadows |
//!
//! `Expr::FuncDef::params` distinguishes `def f($a)` from `def f(a)` (`Param`,
//! `src/jq/expr.rs` -- #2283), but this pass only ever needs
//! [`Param::name`](super::Param::name), the bare identifier either spelling
//! binds: jq's `def f($a): …` desugars to `def f(a): a as $a | …`, which
//! leaves `a` callable at arity 0 regardless of which spelling was written.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::eval::pattern_alternatives_var_names;
use super::parser::{join_expr_operands_mut, JOIN_IDX_VAR};
use super::walk::{any_subexpr, builtin_kids, map_builtin_subexprs, BuiltinKids};
use super::{Builtin, Expr, ObjectKey, Pattern, StringPart};

/// The module-level `def` a module-body diagnostic sits inside (#3085): the
/// scope its occurrence index is counted in.
///
/// The loader splices a module into the program once per `import`/`include`
/// that names it, and a dependency's link run keeps only the defs something
/// references. So neither "which copy" nor "which sibling defs are present"
/// is stable, and a per-file count would point different copies of one
/// module at different physical sites. A count restarted at each
/// module-level def is stable: every copy of a def holds the same body, and
/// the loader keeps every same-named def together, so `(name, arity,
/// ordinal)` names one def in the file (`jq::DefSite`'s table).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModuleDef {
    /// The def's name as written in the module, with any import alias or
    /// link-run prefix removed.
    pub name: String,
    /// How many parameters the def declares.
    pub arity: usize,
    /// How many earlier defs of the same `(name, arity)` the module holds.
    pub ordinal: usize,
}

/// A call this pass could not resolve to any in-scope `def`, parameter or
/// builtin — the compile error's payload.
///
/// Carries the name and arity rather than a formatted message: the two runners
/// word it differently (jq's `f/2 is not defined at <top-level>, line N:` with
/// the offending source line echoed, against yq's uniform `Error: …`), and
/// only the runner has the filter source to quote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedCall {
    /// The called name, as written (a module import arrives here already
    /// rewritten to its `namespace::name` form).
    pub name: String,
    /// How many arguments the call site passed.
    pub arity: usize,
    /// Which module run the failing call was written in (#2951), or `None`
    /// for the main filter and `~/.jq`-free top level.
    ///
    /// Real jq attributes a module body's compile error to that module --
    /// `sb/0 is not defined at /abs/path/sibA.jq, line 1:` with the source
    /// line echoed -- where succinctly could only ever say `<top-level>`,
    /// because by the time this pass runs every module has been inlined into
    /// one flat chain and the runner has only the main filter's text. This
    /// is the run id the loader assigned, which it can map back to a
    /// canonical path and source.
    ///
    /// Lives on this struct rather than on `Expr`: adding a field to
    /// `Expr::FuncDef` would cost 8 bytes on every `Expr` (see
    /// [`ModuleRun`]), whereas this is a diagnostic payload built only when
    /// a call actually fails to resolve.
    pub origin: Option<u32>,
    /// How many earlier calls to this exact `(name, arity)` pair, in the
    /// same module instance (or the top level) -- resolved *or* unresolved --
    /// the resolver had already visited, in source order, before this one
    /// (#2635).
    ///
    /// `jq::CallSite` (`parser.rs`) records every generic call's own source
    /// position, in the same order, regardless of whether it later resolves
    /// -- so this index is exactly what the runner needs to pick this
    /// call's own entry out of that table (`call_sites.iter().filter(same
    /// name/arity).nth(occurrence_index)`) instead of counting *unresolved*
    /// calls only, which silently cites an earlier, unrelated, *resolved*
    /// occurrence of the same name+arity when one comes first in the source
    /// (`(def f: 1; f) | f` cited the resolving `f` inside the parens, not
    /// the failing one after the pipe -- both are `f/0`, only one is a
    /// compile error).
    ///
    /// Inside a module body the count restarts at each module-level def;
    /// [`Self::module_def`] names which one (#3085).
    pub occurrence_index: usize,
    /// The module-level def this call sits inside, when `origin` is a
    /// module (#3085).
    pub module_def: Option<ModuleDef>,
}

impl core::fmt::Display for UnresolvedCall {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}/{} is not defined", self.name, self.arity)
    }
}

/// A `$name` reference this pass could not resolve to any in-scope binding —
/// [`UnresolvedCall`]'s sibling for variables (#2734).
///
/// Real jq resolves `$variable` references at compile time exactly as it does
/// function calls: `$nope` alone is a compile error (`$nope is not defined`,
/// exit 3, zero output), unconditional and uncatchable by `try`/`?`, same as
/// an unresolved call. succinctly's evaluator already refuses an unbound
/// `Expr::Var` at *runtime* (`eval.rs`'s `Expr::Var` arm) — this pass only
/// moves that refusal earlier, the same relationship [`UnresolvedCall`] has
/// to `eval_func_call`'s existing unconditional error arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnboundVar {
    /// The referenced name, without the `$` sigil — matching [`Expr::Var`]'s
    /// own storage.
    pub name: String,
    /// The module run the reference was written in, or `None` for the main
    /// filter -- [`UnresolvedCall::origin`]'s twin, so a module body's
    /// unbound variable is reported against that module's file (#2962).
    pub origin: Option<u32>,
    /// Index among same-named variable references, bound ones included, in
    /// source order -- counted per module-level def inside a module body
    /// ([`Self::module_def`]), per program otherwise (#3085).
    pub occurrence: usize,
    /// The module-level def this reference sits inside, when `origin` is a
    /// module (#3085).
    pub module_def: Option<ModuleDef>,
}

impl core::fmt::Display for UnboundVar {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "${} is not defined", self.name)
    }
}

/// A `break $name` this pass could not resolve to any lexically enclosing
/// `label $name` — [`UnboundVar`]'s sibling for labels (#2964).
///
/// Real jq rejects `break $name` at *compile* time unless it sits lexically
/// inside a `label $name | …` body: `def f: break $x; label $x | f` is
/// `$*label-x is not defined`, exit 3, zero output — even though `f` is
/// never called, because the `def`'s body is compiled at its own position
/// (where `$x` is not yet scoped). succinctly's evaluator resolves `break`
/// purely by name against whatever labels are on the dynamic call stack at
/// *runtime*, silently doing nothing when no matching label is active; this
/// pass moves that refusal earlier, the same relationship [`UnboundVar`] has
/// to evaluation's existing runtime refusal.
///
/// `name` is stored without the `$` sigil (`Expr::Break`'s own storage) —
/// note the `*` in jq's message: a label is compiled to a
/// `$*label-NAME`-named "sentinel" variable, distinct from the user-visible
/// `$name` a `Var` error quotes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedLabel {
    /// The label's name, without the `$` sigil — matching [`Expr::Break`]'s
    /// own storage.
    pub name: String,
    /// Which same-named `break $name` site this belongs to (0-based, in
    /// `super::parser::collect_break_sites`'s source order, scoped to
    /// whichever file `origin` names, and within it to `module_def`): when two
    /// same-named breaks differ only by lexical scope — one bound under an
    /// enclosing `label $name`, one genuinely unbound — the position
    /// recovery in `report_compile_errors` (the CLI) must cite the *failing*
    /// one, and this index is how it threads that decision through the AST,
    /// which carries no positions. Inside a module body the count restarts at
    /// each module-level def, so every copy of the module agrees (#3085).
    pub occurrence: usize,
    /// Which module run the failing break was written in (#2951), or `None`
    /// for the main filter — [`UnboundVar::origin`]'s twin: without this, a
    /// break inside an `import`/`include`d module's `def` body could only
    /// ever be reported as `<top-level>`, which is not merely imprecise but
    /// wrong (the position it would otherwise search for belongs to a
    /// different file's text entirely).
    pub origin: Option<u32>,
    /// The module-level def this break sits inside, when `origin` is a
    /// module (#3085).
    pub module_def: Option<ModuleDef>,
}

impl core::fmt::Display for UnresolvedLabel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "$*label-{} is not defined", self.name)
    }
}

/// One compile-time diagnostic this pass can produce: an unresolved function
/// call, an unbound `$variable` reference, or an out-of-scope `break $label`
/// (#2964).
///
/// All three kinds interleave in a single, source-ordered list rather than
/// three separate ones, because real jq's own diagnostics interleave them by
/// position rather than grouping by kind (confirmed live: `$bar, foo, $baz`
/// reports all three left to right) — only a single combined walk naturally
/// preserves that order. (A label error is reported *with* a call/variable
/// error at a later position, not instead of one; jq reports every compile
/// error it finds in one pass, and a missing label is independent of a
/// missing call.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    Call(UnresolvedCall),
    Var(UnboundVar),
    /// #2964: a `break $name` outside any lexically enclosing `label $name`.
    Break(UnresolvedLabel),
}

impl ResolveError {
    /// The module run this error was written in, or `None` for the main
    /// filter -- every kind's own `origin`. The jq CLI regroups a
    /// multi-module report by it (#3313).
    pub fn origin(&self) -> Option<u32> {
        match self {
            Self::Call(e) => e.origin,
            Self::Var(e) => e.origin,
            Self::Break(e) => e.origin,
        }
    }
}

/// Every builtin the pinned jq (1.7.1) defines, as `(name, arity)`.
///
/// succinctly lowers the builtins it implements to typed `Builtin::*`
/// variants at parse time, so an implemented name reaches this pass as an
/// `Expr::FuncCall` only when the parser declined to lower it -- a spelling
/// at an arity the dedicated parser does not take (`cbrt(1)`), or a name a
/// `def` shadows -- and this roster is what lets such a call be judged
/// "jq defines this" rather than "undefined". It was introduced for the
/// opposite set, the jq builtins succinctly did *not* implement, which
/// compiled in real jq when unreached and would otherwise have been compile
/// errors here; that set is empty since #3042 (the libm family) and #3046
/// (`JOIN`, `format`, `input_filename`, ...), and
/// `test_every_pinned_jq_builtin_is_implemented_1473` (`tests/jq_cli_tests.rs`)
/// sweeps the roster to keep it so.
///
/// Generated by `./scripts/sync-jq-builtin-names.sh` and checked entry for
/// entry against `tests/data/jq-builtin-names.txt` by this module's own
/// `jq_builtin_roster_matches_the_pinned_capture`, so the two cannot drift
/// apart silently.
const JQ_BUILTIN_ROSTER: &[(&str, usize)] = &[
    ("IN", 1),
    ("IN", 2),
    ("INDEX", 1),
    ("INDEX", 2),
    ("JOIN", 2),
    ("JOIN", 3),
    ("JOIN", 4),
    ("abs", 0),
    ("acos", 0),
    ("acosh", 0),
    ("add", 0),
    ("all", 0),
    ("all", 1),
    ("all", 2),
    ("any", 0),
    ("any", 1),
    ("any", 2),
    ("arrays", 0),
    ("ascii_downcase", 0),
    ("ascii_upcase", 0),
    ("asin", 0),
    ("asinh", 0),
    ("atan", 0),
    ("atan2", 2),
    ("atanh", 0),
    ("booleans", 0),
    ("bsearch", 1),
    ("builtins", 0),
    ("capture", 1),
    ("capture", 2),
    ("cbrt", 0),
    ("ceil", 0),
    ("combinations", 0),
    ("combinations", 1),
    ("contains", 1),
    ("copysign", 2),
    ("cos", 0),
    ("cosh", 0),
    ("debug", 0),
    ("debug", 1),
    ("del", 1),
    ("delpaths", 1),
    ("drem", 2),
    ("empty", 0),
    ("endswith", 1),
    ("env", 0),
    ("erf", 0),
    ("erfc", 0),
    ("error", 0),
    ("error", 1),
    ("exp", 0),
    ("exp10", 0),
    ("exp2", 0),
    ("explode", 0),
    ("expm1", 0),
    ("fabs", 0),
    ("fdim", 2),
    ("finites", 0),
    ("first", 0),
    ("first", 1),
    ("flatten", 0),
    ("flatten", 1),
    ("floor", 0),
    ("fma", 3),
    ("fmax", 2),
    ("fmin", 2),
    ("fmod", 2),
    ("format", 1),
    ("frexp", 0),
    ("from_entries", 0),
    ("fromdate", 0),
    ("fromdateiso8601", 0),
    ("fromjson", 0),
    ("fromstream", 1),
    ("gamma", 0),
    ("get_jq_origin", 0),
    ("get_prog_origin", 0),
    ("get_search_list", 0),
    ("getpath", 1),
    ("gmtime", 0),
    ("group_by", 1),
    ("gsub", 2),
    ("gsub", 3),
    ("halt", 0),
    ("halt_error", 0),
    ("halt_error", 1),
    ("has", 1),
    ("hypot", 2),
    ("implode", 0),
    ("in", 1),
    ("index", 1),
    ("indices", 1),
    ("infinite", 0),
    ("input", 0),
    ("input_filename", 0),
    ("input_line_number", 0),
    ("inputs", 0),
    ("inside", 1),
    ("isempty", 1),
    ("isfinite", 0),
    ("isinfinite", 0),
    ("isnan", 0),
    ("isnormal", 0),
    ("iterables", 0),
    ("j0", 0),
    ("j1", 0),
    ("jn", 2),
    ("join", 1),
    ("keys", 0),
    ("keys_unsorted", 0),
    ("last", 0),
    ("last", 1),
    ("ldexp", 2),
    ("length", 0),
    ("lgamma", 0),
    ("lgamma_r", 0),
    ("limit", 2),
    ("localtime", 0),
    ("log", 0),
    ("log10", 0),
    ("log1p", 0),
    ("log2", 0),
    ("logb", 0),
    ("ltrimstr", 1),
    ("map", 1),
    ("map_values", 1),
    ("match", 1),
    ("match", 2),
    ("max", 0),
    ("max_by", 1),
    ("min", 0),
    ("min_by", 1),
    ("mktime", 0),
    ("modf", 0),
    ("modulemeta", 0),
    ("nan", 0),
    ("nearbyint", 0),
    ("nextafter", 2),
    ("nexttoward", 2),
    ("normals", 0),
    ("not", 0),
    ("now", 0),
    ("nth", 1),
    ("nth", 2),
    ("nulls", 0),
    ("numbers", 0),
    ("objects", 0),
    ("path", 1),
    ("paths", 0),
    ("paths", 1),
    ("pick", 1),
    ("pow", 2),
    ("pow10", 0),
    ("range", 1),
    ("range", 2),
    ("range", 3),
    ("recurse", 0),
    ("recurse", 1),
    ("recurse", 2),
    ("remainder", 2),
    ("repeat", 1),
    ("reverse", 0),
    ("rindex", 1),
    ("rint", 0),
    ("round", 0),
    ("rtrimstr", 1),
    ("scalars", 0),
    ("scalb", 2),
    ("scalbln", 2),
    ("scan", 1),
    ("scan", 2),
    ("select", 1),
    ("setpath", 2),
    ("significand", 0),
    ("sin", 0),
    ("sinh", 0),
    ("sort", 0),
    ("sort_by", 1),
    ("split", 1),
    ("split", 2),
    ("splits", 1),
    ("splits", 2),
    ("sqrt", 0),
    ("startswith", 1),
    ("stderr", 0),
    ("strflocaltime", 1),
    ("strftime", 1),
    ("strings", 0),
    ("strptime", 1),
    ("sub", 2),
    ("sub", 3),
    ("tan", 0),
    ("tanh", 0),
    ("test", 1),
    ("test", 2),
    ("tgamma", 0),
    ("to_entries", 0),
    ("todate", 0),
    ("todateiso8601", 0),
    ("tojson", 0),
    ("tonumber", 0),
    ("tostream", 0),
    ("tostring", 0),
    ("transpose", 0),
    ("trunc", 0),
    ("truncate_stream", 1),
    ("type", 0),
    ("unique", 0),
    ("unique_by", 1),
    ("until", 2),
    ("utf8bytelength", 0),
    ("values", 0),
    ("walk", 1),
    ("while", 2),
    ("with_entries", 1),
    ("y0", 0),
    ("y1", 0),
    ("yn", 2),
];

/// A lexical function scope: the `(name, arity)` pairs visible at a point in
/// the tree.
///
/// A stack, not a map cloned per node: shadowing falls out of the order, and
/// pushing and truncating is cheaper than cloning. It is *indexed* by name
/// ([`FnScope`], #3455), because a module run can hold thousands of defs and a
/// lookup that scanned the stack was linear in the distance to the def it
/// reached.
type Scope = FnScope<()>;

/// The module-run bracket: how a *scope boundary* is encoded in the def
/// chain, so a module body cannot see names that are merely wrapped around
/// it (#2951).
///
/// # Why a def, and why this name
///
/// succinctly inlines every module's defs into one flat `Expr::FuncDef`
/// chain wrapped around the main filter, so each module body physically sits
/// nested inside every other unqualified source -- `~/.jq` and every sibling
/// `include`. Ordinary lexical scoping then resolves outward into them, and
/// resolves names real jq keeps out. The boundary has to be expressed
/// *somewhere* in the tree, and the options were costed in #2951's plan:
///
/// - A field on [`Expr::FuncDef`] costs 8 bytes on **every** `Expr` (the
///   discriminant stops riding a niche -- measured for #2283) and eats the
///   `MAX_EXPR_DEPTH` stack margin. Rejected.
/// - A side table keyed by node address cannot survive `substitute_vars`,
///   which rebuilds the tree between the loader and this pass. Rejected.
/// - Rewriting each module's own defs to a unique prefix (name mangling)
///   does not close the leak at all: an exported name must stay callable
///   bare from the main filter, so its bare wrapper stays outside the
///   sibling's body and is still found. Closing it that way requires
///   knowing which names are *free* in the module -- which is this pass's
///   job, not the loader's. Rejected as unsound, not merely expensive.
///
/// What is left is to encode the boundary in a value the chain already
/// carries: a def's own name. See `docs/adrs/adr-0023.md` for the full
/// decision, including why symbolic binding at load time is the eventual
/// answer and why nothing here blocks it. A marker is a real `Expr::FuncDef` with an
/// `Identity` body, whose name begins with a NUL byte. NUL cannot be lexed,
/// so no call anywhere -- in a module, in the main filter, in `~/.jq` --
/// can ever name one.
///
/// # The rule
///
/// The loader brackets every *run* of defs it wraps (each `include`, the
/// `~/.jq` block, each `import`, and each module linked once as some other
/// module's dependency, #2955) between a begin and an end marker:
///
/// ```text
/// def <NUL>run:begin:<id>[:<alias>]: .;   ...the run's defs...   def <NUL>run:end:<id>: .;
/// ```
///
/// Because markers are ordinary `FuncDef`s, both walks in this file already
/// push them onto their scope stack and truncate them at the right moment --
/// no walk state has to be threaded through for this to work. Only *lookup*
/// changes, and it changes once, in this module's own `scan_scope` (private,
/// so not linked here): scanning innermost-first,
/// an end marker opens a closed run (its defs are visible; they are wrapped
/// *around* the current point) and a begin marker with no matching end is the
/// **floor** -- everything below it belongs to some other module and is
/// invisible from here.
///
/// ### Link runs (#2955)
///
/// A module that another module depends on is emitted **once**, outermost,
/// as a run whose alias is [`Self::link_alias`] and whose defs carry the
/// qualified spelling [`Self::link_name`] -- `<NUL>link:<id>::<name>` -- so
/// the run exports nothing a user can spell. Inside it, a bare sibling call
/// misses those names, floors at the run's own begin marker, and is retried
/// under the alias exactly as a bare call inside an `import`ed module is
/// (#2989); the retry renames the call in place, so the evaluator binds it
/// with no change of its own. A consumer reaches the run through a
/// forwarding stub the loader wraps into its body, `def g: <NUL>link:<id>::g;`,
/// and that lookup is the one kind that **crosses floors**: the stub sits
/// inside some run of its own, and its target is outside every run. Nothing
/// lexable can spell a link name, so only a stub or a retried sibling call
/// ever makes such a lookup.
pub struct ModuleRun;

/// One parsed marker name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunMarker<'a> {
    /// Opens a run that is also a **scope floor**: nothing below it is
    /// visible from inside. Carries the import alias when the run is an
    /// `import`, so a bare sibling call inside it can be retried as
    /// `alias::name`.
    Begin { id: u32, alias: Option<&'a str> },
    /// Closes the run with this id.
    End { id: u32 },
}

/// The NUL-prefixed sigil no lexable identifier can start with.
const RUN_BEGIN: &str = "\u{0}run:begin:";
/// Sibling of [`RUN_BEGIN`].
const RUN_END: &str = "\u{0}run:end:";
/// The prefix of a linked module's alias and of every def inside its run
/// (#2955) -- see [`ModuleRun::link_alias`] and [`ModuleRun::link_name`]. Not
/// a marker: [`ModuleRun::parse`] reads a link name as an ordinary def, which
/// is what it is.
const LINK: &str = "\u{0}link:";

impl ModuleRun {
    /// The name of the def that opens run `id`, carrying `alias` when the
    /// run is an `import` (so a bare sibling call inside it can be retried
    /// as `alias::name` -- #2989).
    #[must_use]
    pub fn begin_marker(id: u32, alias: Option<&str>) -> String {
        match alias {
            Some(a) => alloc::format!("{RUN_BEGIN}{id}:{a}"),
            None => alloc::format!("{RUN_BEGIN}{id}"),
        }
    }

    /// The name of the def that closes run `id`.
    #[must_use]
    pub fn end_marker(id: u32) -> String {
        alloc::format!("{RUN_END}{id}")
    }

    /// Parse a def name back into a marker, or `None` for an ordinary def.
    ///
    /// One definition shared by the loader (which writes them) and this
    /// pass (which reads them), so the two can never drift on the spelling.
    #[must_use]
    pub fn parse(name: &str) -> Option<RunMarker<'_>> {
        // Cheapest possible rejection for the overwhelmingly common case:
        // an ordinary def name cannot start with NUL, so one byte decides.
        if !name.starts_with('\u{0}') {
            return None;
        }
        if let Some(rest) = name.strip_prefix(RUN_BEGIN) {
            let (id, alias) = match rest.split_once(':') {
                Some((id, a)) => (id, Some(a)),
                None => (rest, None),
            };
            return id.parse().ok().map(|id| RunMarker::Begin { id, alias });
        }
        name.strip_prefix(RUN_END)
            .and_then(|id| id.parse().ok())
            .map(|id| RunMarker::End { id })
    }

    /// The alias of the run that links module `id` once (#2955): what its
    /// begin marker carries, so a bare sibling call inside the run is retried
    /// as [`Self::link_name`] by the same code that retries a bare call inside
    /// an `import`ed module (#2989). The NUL prefix keeps it out of reach of
    /// anything a user can write, exactly as it does for the markers.
    #[must_use]
    pub fn link_alias(id: u32) -> String {
        alloc::format!("{LINK}{id}")
    }

    /// The name def `name` is emitted under inside module `id`'s link run,
    /// and the name a consumer's forwarding stub calls: `<alias>::<name>`,
    /// the same shape an `import "m" as a;` gives `a::name`.
    #[must_use]
    pub fn link_name(id: u32, name: &str) -> String {
        alloc::format!("{LINK}{id}::{name}")
    }

    /// Whether `name` is a [`Self::link_name`] -- the one lookup that is
    /// allowed to cross a scope floor (see `scan_scope`).
    #[must_use]
    pub fn is_link_name(name: &str) -> bool {
        name.starts_with(LINK)
    }

    /// `name` as a user wrote it: [`Self::link_name`] undone, and any other
    /// name returned unchanged. For messages that name a def, so an internal
    /// spelling never reaches the terminal.
    #[must_use]
    pub fn display_name(name: &str) -> &str {
        name.strip_prefix(LINK)
            .and_then(|rest| rest.split_once("::"))
            .map_or(name, |(_, written)| written)
    }
}

/// What an innermost-first scope scan found, plus the boundary it stopped at.
pub(crate) struct ScanResult<T> {
    /// The matching entry, if any was visible from here.
    pub(crate) hit: Option<T>,
    /// The innermost open run at this point: its id, and its import alias
    /// if it has one. `None` at the top level and in the main filter, which
    /// sit below every end marker and so see everything.
    ///
    /// **Only meaningful when [`Self::hit`] is `None`.** The scan stops as
    /// soon as it matches, so a successful lookup reports no run rather than
    /// walking the rest of the stack to find one that no caller would read.
    pub(crate) run: Option<(u32, Option<String>)>,
}

// Scope entries the lookups have examined on this thread -- test-only, so a
// test can count work instead of timing it (#3455, as #3307's `INSTALL_VISITS`).
#[cfg(test)]
thread_local! {
    static SCOPE_PROBES: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

/// Count one scope entry examined (see `SCOPE_PROBES`); nothing in a real build.
#[cfg(test)]
fn note_scope_probe() {
    SCOPE_PROBES.with(|n| n.set(n.get() + 1));
}

#[cfg(not(test))]
#[inline(always)]
fn note_scope_probe() {}

/// The one place the module-scope floor is applied.
///
/// Walks `scope` innermost-first, handing each ordinary entry to `probe` and
/// stopping at the first begin marker that has no matching end -- the floor.
/// Returns whatever `probe` matched (if anything) together with the innermost
/// open run, which callers need for the `import` retry (#2989) and for
/// attributing a compile error to the module it came from.
///
/// `crosses_floors` is the one exception (#2955): a lookup for a
/// [`ModuleRun::link_name`] steps over an open begin marker instead of
/// stopping at it. The name it wants lives in a link run wrapped outside
/// every module run, and the stub or retried sibling call asking for it sits
/// inside one; nothing lexable spells such a name, so no user-written call
/// can ride this exception across a boundary. Callers compute it from the
/// name they look up rather than deciding it themselves, so the two lookups
/// that can meet a link name ([`in_scope`], [`reach_in_scope`]) cannot drift.
///
/// Both walks in this file share it, rather than each spelling the marker
/// bookkeeping out: the file's own header already flags the two as the pair
/// that must stay in lock-step, and "which names are visible here" is exactly
/// the kind of duplicated predicate that diverges silently.
///
/// Function scopes answer the same question from an index ([`FnScope`], #3455)
/// instead of calling this: [`in_scope`] and [`reach_in_scope`] share that one
/// lookup, so they still cannot drift from each other, and
/// `fn_scope_matches_scan_scope_3455` pins it to this scan over random
/// push/truncate histories. The variable and label stacks still call it.
fn scan_scope<'a, E, T>(
    scope: &'a [E],
    name_of: impl Fn(&'a E) -> &'a str,
    mut probe: impl FnMut(&'a E) -> Option<T>,
    crosses_floors: bool,
) -> ScanResult<T> {
    // Counts end markers seen but not yet matched by an opener. A run whose
    // end we have already passed is *closed*: it is wrapped around this
    // point, so its defs are visible and its opener is not a floor.
    let mut closed = 0usize;
    for entry in scope.iter().rev() {
        note_scope_probe();
        match ModuleRun::parse(name_of(entry)) {
            Some(RunMarker::End { .. }) => closed += 1,
            Some(RunMarker::Begin { id, alias }) => {
                if closed > 0 {
                    closed -= 1;
                } else if !crosses_floors {
                    // An open run: nothing below it is visible from inside
                    // this module body, and it names who wrote the code here.
                    return ScanResult {
                        hit: None,
                        run: Some((id, alias.map(ToString::to_string))),
                    };
                }
            }
            None => {
                if let Some(found) = probe(entry) {
                    // Both callers read `run` only when `hit` is `None`, and
                    // the contract on `ScanResult::run` says so -- returning
                    // here keeps the common case O(distance to the match)
                    // rather than O(scope). Scanning on regardless made every
                    // lookup walk the whole stack, which is quadratic over a
                    // long def chain.
                    return ScanResult {
                        hit: Some(found),
                        run: None,
                    };
                }
            }
        }
    }
    // No open run: the top level, or the main filter below every run.
    ScanResult {
        hit: None,
        run: None,
    }
}

/// What a [`FnScope`] entry is to the module-run bracket (see [`ModuleRun`]).
enum RunEntry {
    /// An ordinary def or parameter -- including a link name, which
    /// [`ModuleRun::parse`] reads as the def it is.
    Plain,
    /// A begin marker: it is the scope floor for as long as no end marker
    /// closes it.
    Begin { id: u32, alias: Option<String> },
    /// An end marker: it closes the innermost run still open.
    End,
}

/// One entry of a [`FnScope`].
struct FnEntry<T> {
    name: String,
    arity: usize,
    /// What a lookup that lands on this entry reports.
    payload: T,
    run: RunEntry,
    /// Where the floor is once this entry is on the stack: the position of
    /// the innermost begin marker no end marker has closed, if any.
    floor: Option<usize>,
}

/// A lexical function scope, indexed by name (#3455).
///
/// The question it answers is the one [`scan_scope`] answers -- which entry a
/// `(name, arity)` call reaches, innermost first, stopping at the module-scope
/// floor, plus the innermost open run on a miss -- without reading the stack.
/// A call to the outermost of `M` defs scanned all `M` entries, and a filter
/// naming every def did that `M` times.
///
/// Every entry that is not a marker is recorded under its name and arity, in
/// push order, so the innermost entry of a key is the last of its list. It is
/// *visible* when it sits above the floor, or when the lookup crosses floors
/// (#2955). That is all the scan decides: it probes every ordinary entry above
/// the floor and none below. A closed run's entries are above the floor too,
/// because its end marker lifts the floor back below them. The floor is kept
/// per entry, so [`truncate`](Self::truncate) restores it without replaying
/// the stack.
///
/// Pushing parses the entry's name once, where each scan parsed every entry
/// it passed.
struct FnScope<T> {
    entries: Vec<FnEntry<T>>,
    /// Positions into `entries`, ascending, by name and then arity. Markers
    /// are never listed: no call can name one.
    positions: BTreeMap<String, Vec<(usize, Vec<usize>)>>,
}

impl<T> Default for FnScope<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            positions: BTreeMap::new(),
        }
    }
}

impl<T: Copy> FnScope<T> {
    fn len(&self) -> usize {
        self.entries.len()
    }

    /// The floor the whole stack presents: the innermost open begin marker.
    fn floor(&self) -> Option<usize> {
        self.entries.last().and_then(|entry| entry.floor)
    }

    fn push(&mut self, name: String, arity: usize, payload: T) {
        let at = self.entries.len();
        let open = self.floor();
        let run = match ModuleRun::parse(&name) {
            Some(RunMarker::Begin { id, alias }) => RunEntry::Begin {
                id,
                alias: alias.map(ToString::to_string),
            },
            Some(RunMarker::End { .. }) => RunEntry::End,
            None => RunEntry::Plain,
        };
        let floor = match run {
            RunEntry::Begin { .. } => Some(at),
            // Closing the innermost open run puts the floor back where it was
            // when that run began: after the entry just below its begin marker.
            RunEntry::End => open
                .and_then(|begin| begin.checked_sub(1))
                .and_then(|below| self.entries[below].floor),
            RunEntry::Plain => open,
        };
        if matches!(run, RunEntry::Plain) {
            match self.positions.get_mut(name.as_str()) {
                Some(slots) => match slots.iter_mut().find(|(a, _)| *a == arity) {
                    Some((_, listed)) => listed.push(at),
                    None => slots.push((arity, alloc::vec![at])),
                },
                None => {
                    self.positions
                        .insert(name.clone(), alloc::vec![(arity, alloc::vec![at])]);
                }
            }
        }
        self.entries.push(FnEntry {
            name,
            arity,
            payload,
            run,
            floor,
        });
    }

    /// Drop every entry from `len` up, as [`Vec::truncate`] does.
    fn truncate(&mut self, len: usize) {
        if len >= self.entries.len() {
            return;
        }
        for entry in self.entries.drain(len..).rev() {
            if !matches!(entry.run, RunEntry::Plain) {
                continue;
            }
            // Every plain entry was listed under its name and arity when it was
            // pushed, and entries leave in reverse order, so this entry's
            // position is the last of its list. A mismatch means the index and
            // the stack have drifted, which a lookup would turn into a wrong
            // scope: stop here rather than carry on from a corrupt index.
            let Some(slots) = self.positions.get_mut(entry.name.as_str()) else {
                unreachable!("a plain entry is listed under its name"); // patchcov: coverage tolerate-line reason="unreachable by construction: every plain entry was listed under its name when it was pushed"
            };
            let Some(slot) = slots.iter().position(|(a, _)| *a == entry.arity) else {
                unreachable!("a plain entry is listed under its arity"); // patchcov: coverage tolerate-line reason="unreachable by construction: every plain entry was listed under its arity when it was pushed"
            };
            slots[slot].1.pop();
            if slots[slot].1.is_empty() {
                slots.swap_remove(slot);
            }
            if slots.is_empty() {
                self.positions.remove(entry.name.as_str());
            }
        }
    }

    /// What a call to `(name, arity)` reaches here, and the innermost open run
    /// if it reaches nothing -- [`scan_scope`]'s answer, from the index.
    /// `crosses_floors` is the same exception it takes (#2955).
    fn lookup(&self, name: &str, arity: usize, crosses_floors: bool) -> ScanResult<T> {
        note_scope_probe();
        let floor = self.floor();
        let reached = self
            .positions
            .get(name)
            .and_then(|slots| {
                slots.iter().find(|(a, _)| {
                    note_scope_probe();
                    *a == arity
                })
            })
            .and_then(|(_, listed)| listed.last().copied())
            // `map_or(true, ..)`, not `is_none_or`: the crate's MSRV (1.73)
            // predates it (1.82).
            .filter(|&at| crosses_floors || floor.map_or(true, |open| at > open));
        if let Some(at) = reached {
            // As `scan_scope`: a hit reports no run, which no caller reads.
            return ScanResult {
                hit: Some(self.entries[at].payload),
                run: None,
            };
        }
        // A lookup that crosses floors never stops at one, so it names none.
        let run = floor
            .filter(|_| !crosses_floors)
            .and_then(|open| self.entries[open].run.open());
        ScanResult { hit: None, run }
    }
}

impl RunEntry {
    /// The run this begin marker opens, as [`ScanResult::run`] reports it.
    fn open(&self) -> Option<(u32, Option<String>)> {
        let Self::Begin { id, alias } = self else {
            return None; // patchcov: coverage tolerate-line reason="unreachable by construction: `FnScope::floor` only ever names a begin marker"
        };
        Some((*id, alias.clone()))
    }
}

/// One definition of the name stack both [`VarScope`] and [`LabelScope`] are
/// (#3081). They stay two distinct types rather than aliases of one
/// `Vec<String>`: `check` threads both through every recursive call, and with
/// bare aliases a transposed pair of arguments type-checked, so a swap would
/// have been a silent wrong-scope lookup instead of a compile error.
macro_rules! name_scope {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Default)]
        struct $name(Vec<String>);

        impl $name {
            fn push(&mut self, name: String) {
                self.0.push(name);
            }

            fn pop(&mut self) {
                self.0.pop();
            }

            /// The names, innermost last -- what a scope scan reads.
            fn names(&self) -> &[String] {
                &self.0
            }
        }
    };
}

name_scope! {
    /// A lexical *variable* scope: the `$name`s (without the sigil) visible at a
    /// point in the tree — [`Scope`]'s sibling for #2734, tracked in the same
    /// walk rather than a separate pass (see [`check`]'s doc comment).
    ///
    /// No arity component: unlike a function, a variable binding has no
    /// overload-by-arity concept, so plain name equality is enough.
    VarScope
}

impl VarScope {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn truncate(&mut self, len: usize) {
        self.0.truncate(len);
    }

    fn extend(&mut self, names: impl IntoIterator<Item = String>) {
        self.0.extend(names);
    }
}

name_scope! {
    /// A lexical *label* scope (#2964): the `$label` names (without the sigil)
    /// visible at a point in the tree — [`VarScope`]'s sibling for `break`'s
    /// compile-time label-scope check, tracked in the same walk rather than a
    /// separate pass (see [`check`]'s doc comment).
    ///
    /// No arity or value component, same as [`VarScope`] but for `label`
    /// bindings: only the name matters. Restored before returning so a sibling
    /// never sees a label introduced by its neighbour, exactly like `var_scope`.
    LabelScope
}

/// [`Scope`]'s sibling for [`build_call_graph`] (#2740): each entry also
/// carries the identity of the `def` it resolves to -- a parameter has none
/// (`None`), since it has no body of its own to ever mark reachable.
///
/// A `def`'s identity is its `body`'s own heap address (`&**body as *const
/// Expr as usize`), not a sequential counter: computing it needs no walk
/// state threaded through this pass at all, and -- the property that
/// actually matters -- [`check`]'s own later, separate, `&mut`-based walk
/// over the *same* tree can recompute the identical address for the same
/// `Expr::FuncDef` node without this pass having to hand it anything, since
/// this pass never restructures the tree it walks. A sequential counter
/// would need the two passes to agree on visitation order down to the
/// exact node, forever, as a silent invariant nothing enforces; a
/// recomputed address needs the two passes to agree on nothing at all past
/// "this box hasn't moved since the last pass read its address".
type ReachScope = FnScope<ScopeHit>;

/// What a [`ReachScope`] entry resolves to: a real `def` (identified by its
/// body's address, for [`build_call_graph`]'s call graph) or a parameter,
/// which has no body of its own to ever mark reachable.
#[derive(Clone, Copy)]
enum ScopeHit {
    Def(usize),
    Param,
}

/// Whether `(name, arity)` resolves against `scope`, and if so, which
/// `def`'s body it reaches (or that it's a parameter). Innermost first,
/// mirroring [`in_scope`], and stopping at the module-scope floor
/// (see [`ModuleRun`]).
///
/// Also reports the innermost open run, so the caller can apply the same
/// `alias::name` retry [`check`] does -- the two passes have to agree on
/// which def a call reaches, or reachability and checking disagree about
/// which bodies are compiled at all (#2951).
fn reach_in_scope(scope: &ReachScope, name: &str, arity: usize) -> ScanResult<ScopeHit> {
    scope.lookup(name, arity, ModuleRun::is_link_name(name))
}

/// Read-only twin of [`builtin_fallback_into_args`]: yields `fallback`'s own
/// sub-expressions by reference rather than consuming it, for
/// [`build_call_graph`] (#2740), which only needs to see into them, not
/// resolve or replace the call itself. Kept in sync with that function by
/// construction, not just convention: both match the exact same shapes
/// `builtin_fallback_arity` names, one returning owned children, this one
/// borrowed ones.
fn builtin_fallback_children(fallback: &Expr) -> Vec<&Expr> {
    match fallback {
        Expr::Not => Vec::new(),
        Expr::Limit { n, expr } => alloc::vec![n.as_ref(), expr.as_ref()],
        Expr::Until { cond, update } | Expr::While { cond, update } => {
            alloc::vec![cond.as_ref(), update.as_ref()]
        }
        Expr::Repeat(inner) | Expr::FirstExpr(inner) | Expr::LastExpr(inner) => {
            alloc::vec![inner.as_ref()]
        }
        Expr::Paren(inner) if matches!(inner.as_ref(), Expr::Range { .. }) => {
            match inner.as_ref() {
                Expr::Range { to: Some(to), .. } => alloc::vec![to.as_ref()],
                Expr::Range { .. } => Vec::new(),
                _ => unreachable!("guarded by the outer match arm's pattern"),
            }
        }
        Expr::Range { from, to, step } => {
            let mut children = alloc::vec![from.as_ref()];
            children.extend(to.as_deref());
            children.extend(step.as_deref());
            children
        }
        Expr::Error(msg) => msg.iter().map(alloc::boxed::Box::as_ref).collect(),
        Expr::Break(_) => Vec::new(),
        Expr::Builtin(builtin) => match builtin_kids(builtin) {
            BuiltinKids::None => Vec::new(),
            BuiltinKids::One(a) => alloc::vec![a],
            BuiltinKids::Two(a, b) => alloc::vec![a, b],
            BuiltinKids::Three(a, b, c) => alloc::vec![a, b, c],
        },
        // Genuinely unreachable -- see `builtin_fallback_arity`'s own
        // identical fallback arm.
        _ => Vec::new(),
    }
}

/// Builds the call graph among `def` bodies (#2740): an edge from `def` `A`
/// (identified by its body's address, or the virtual root -- `None` --  for
/// code outside every `def` body, always reachable) to `def` `B` records
/// that `A`'s body (or the root) contains a call resolving to `B`. A call
/// resolving to a parameter contributes no edge -- a parameter is not a
/// `def` with a body to reach, and its actual argument flows from wherever
/// the enclosing function is itself called, a different question this
/// pass does not answer.
///
/// Mirrors [`check`]'s own structure arm for arm (both must stay
/// exhaustive over every [`Expr`] variant, so a missing arm here is a
/// compile error, not a silent gap), but does no error reporting and never
/// mutates: it only needs to know *which* calls exist and where they sit,
/// not to resolve or rewrite them, so `#2036`'s `builtin_fallback` shadowing
/// decision is made read-only here via [`reach_in_scope`]/
/// [`builtin_fallback_children`] rather than the consuming
/// [`builtin_fallback_into_args`].
fn build_call_graph(
    expr: &Expr,
    scope: &mut ReachScope,
    enclosing: Option<usize>,
    graph: &mut BTreeMap<usize, Vec<usize>>,
    roots: &mut Vec<usize>,
) {
    let mut record_edge = |target: usize, graph: &mut BTreeMap<usize, Vec<usize>>| match enclosing {
        Some(e) => graph.entry(e).or_default().push(target),
        None => roots.push(target),
    };

    match expr {
        Expr::Shared(inner) => build_call_graph(inner, scope, enclosing, graph, roots),
        Expr::DefCall { args, .. } => {
            for arg in args {
                build_call_graph(arg, scope, enclosing, graph, roots);
            }
        }
        Expr::Identity
        | Expr::Field(_)
        | Expr::Index { .. }
        | Expr::Slice { .. }
        | Expr::ArrayKey(_)
        | Expr::Iterate
        | Expr::Literal(_)
        | Expr::RecursiveDescent
        | Expr::RecursiveDescentWithKeys
        | Expr::Not
        | Expr::Format(_)
        | Expr::Var(_)
        | Expr::TrackedVar(_)
        | Expr::DeferredVar(_)
        | Expr::Loc { .. }
        | Expr::Env
        | Expr::Break(_) => {}

        Expr::Optional(inner)
        | Expr::Array(inner)
        | Expr::Paren(inner)
        | Expr::Negate(inner)
        | Expr::FirstExpr(inner)
        | Expr::LastExpr(inner)
        | Expr::Repeat(inner) => {
            build_call_graph(inner, scope, enclosing, graph, roots);
        }

        // #2840: this pass runs on the pristine, pre-`check` tree (see
        // `resolve_func_calls_all`'s doc comment), so `check`'s own
        // `Expr::Label` rewrite hasn't happened yet -- the synthetic
        // `error/0` call site it will insert has to be marked reachable
        // here independently, mirroring `Expr::FuncCall`'s own
        // `reach_in_scope` -> `record_edge` shape, so a `def error:` only
        // reachable through a shadowed label (`def error: bogus; label
        // $out | 1`) is still compiled and reported like any other call.
        Expr::Label { body: inner, .. } => {
            if let Some(ScopeHit::Def(target)) = reach_in_scope(scope, "error", 0).hit {
                record_edge(target, graph);
            }
            build_call_graph(inner, scope, enclosing, graph, roots);
        }

        Expr::Error(inner) => {
            if let Some(e) = inner.as_deref() {
                build_call_graph(e, scope, enclosing, graph, roots);
            }
        }

        Expr::Arithmetic { left, right, .. }
        | Expr::Compare { left, right, .. }
        | Expr::And(left, right)
        | Expr::Or(left, right)
        | Expr::Alternative(left, right)
        | Expr::IndexExpr {
            target: left,
            key: right,
        }
        | Expr::Limit {
            n: left,
            expr: right,
        }
        | Expr::NthExpr {
            n: left,
            expr: right,
        }
        | Expr::Until {
            cond: left,
            update: right,
        }
        | Expr::While {
            cond: left,
            update: right,
        }
        | Expr::As {
            expr: left,
            body: right,
            ..
        }
        | Expr::Assign {
            path: left,
            value: right,
        }
        | Expr::Update {
            path: left,
            filter: right,
        }
        | Expr::CompoundAssign {
            path: left,
            value: right,
            ..
        }
        | Expr::AlternativeAssign {
            path: left,
            value: right,
        }
        | Expr::MetaAssign {
            target: left,
            value: right,
            ..
        } => {
            build_call_graph(left, scope, enclosing, graph, roots);
            build_call_graph(right, scope, enclosing, graph, roots);
        }

        Expr::FuncDef {
            name,
            params,
            body,
            then,
            ..
        } => {
            let body_addr = body.as_ref() as *const Expr as usize;

            let outer = scope.len();
            scope.push(name.clone(), params.len(), ScopeHit::Def(body_addr));
            let with_self = scope.len();
            for p in params {
                scope.push(p.name().to_string(), 0, ScopeHit::Param);
            }
            build_call_graph(body, scope, Some(body_addr), graph, roots);
            scope.truncate(with_self);

            build_call_graph(then, scope, enclosing, graph, roots);
            scope.truncate(outer);
        }

        Expr::Try { expr, catch } => {
            build_call_graph(expr, scope, enclosing, graph, roots);
            if let Some(c) = catch.as_deref() {
                build_call_graph(c, scope, enclosing, graph, roots);
            }
        }

        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            build_call_graph(cond, scope, enclosing, graph, roots);
            build_call_graph(then_branch, scope, enclosing, graph, roots);
            build_call_graph(else_branch, scope, enclosing, graph, roots);
        }

        Expr::SliceExpr { target, start, end } => {
            build_call_graph(target, scope, enclosing, graph, roots);
            if let Some(s) = start.as_deref() {
                build_call_graph(s, scope, enclosing, graph, roots);
            }
            if let Some(e) = end.as_deref() {
                build_call_graph(e, scope, enclosing, graph, roots);
            }
        }

        Expr::Range { from, to, step } => {
            build_call_graph(from, scope, enclosing, graph, roots);
            if let Some(t) = to.as_deref() {
                build_call_graph(t, scope, enclosing, graph, roots);
            }
            if let Some(s) = step.as_deref() {
                build_call_graph(s, scope, enclosing, graph, roots);
            }
        }

        // #2971 review: `check` compile-checks every computed key in these
        // patterns (`bind_patterns`, #2734), so a call inside one is an edge
        // like any other. Skipping them left a def written in a key -- `. as
        // {(def g: nosuchfn; g): $x} | $x` -- permanently unreachable, and so
        // never checked, where jq refuses to compile it.
        Expr::AsPattern {
            expr,
            patterns,
            body,
        } => {
            build_call_graph(expr, scope, enclosing, graph, roots);
            build_call_graph_pattern_keys(patterns, scope, enclosing, graph, roots);
            build_call_graph(body, scope, enclosing, graph, roots);
        }

        Expr::Reduce {
            input,
            patterns,
            init,
            update,
        } => {
            build_call_graph_pattern_keys(patterns, scope, enclosing, graph, roots);
            build_call_graph(input, scope, enclosing, graph, roots);
            build_call_graph(init, scope, enclosing, graph, roots);
            build_call_graph(update, scope, enclosing, graph, roots);
        }

        Expr::Foreach {
            input,
            patterns,
            init,
            update,
            extract,
        } => {
            build_call_graph_pattern_keys(patterns, scope, enclosing, graph, roots);
            build_call_graph(input, scope, enclosing, graph, roots);
            build_call_graph(init, scope, enclosing, graph, roots);
            build_call_graph(update, scope, enclosing, graph, roots);
            if let Some(e) = extract.as_deref() {
                build_call_graph(e, scope, enclosing, graph, roots);
            }
        }

        Expr::Pipe(exprs) => {
            for e in exprs {
                build_call_graph(e, scope, enclosing, graph, roots);
            }
        }
        Expr::Comma(exprs) => {
            for e in exprs {
                build_call_graph(e, scope, enclosing, graph, roots);
            }
        }

        Expr::FuncCall {
            name,
            args,
            builtin_fallback,
        } => {
            let arity = call_arity(args, builtin_fallback.as_deref());
            // #2989: inside an `import`ed module a def calls its sibling
            // by the bare name it is written with, but the chain holds that
            // sibling under `alias::name`. Retry the bare miss exactly as
            // `check` does, so both passes agree on which def this call
            // reaches -- if only `check` retried, the edge would be missing
            // here and the target body would look unreachable and never be
            // compiled at all.
            let scan = reach_in_scope(scope, name, arity);
            let scan = match (&scan.hit, &scan.run) {
                (None, Some((_, Some(alias)))) => {
                    let qualified = alloc::format!("{alias}::{name}");
                    let retry = reach_in_scope(scope, &qualified, arity);
                    if retry.hit.is_some() {
                        retry
                    } else {
                        scan
                    }
                }
                _ => scan,
            };
            match scan.hit {
                Some(hit) => {
                    if let ScopeHit::Def(target) = hit {
                        record_edge(target, graph);
                    }
                    if !args.is_empty() {
                        for a in args {
                            build_call_graph(a, scope, enclosing, graph, roots);
                        }
                    } else if let Some(fallback) = builtin_fallback.as_deref() {
                        for child in builtin_fallback_children(fallback) {
                            build_call_graph(child, scope, enclosing, graph, roots);
                        }
                    }
                }
                None => {
                    if let Some(fallback) = builtin_fallback.as_deref() {
                        build_call_graph(fallback, scope, enclosing, graph, roots);
                    } else if is_jq_builtin(name, arity) {
                        for a in args {
                            build_call_graph(a, scope, enclosing, graph, roots);
                        }
                    }
                    // A genuinely unresolvable callee: `check`'s own matching
                    // branch doesn't check its arguments either (#2037 --
                    // real jq's compiler never compiles an unresolved call's
                    // arguments, having nothing to bind them to), so nothing
                    // in `args` can mark another `def` reachable through
                    // this call site. Confirmed live: `def h: nosuchfn2;
                    // nosuchfn(h)` reports only `nosuchfn/1 is not defined`
                    // in jq 1.7.1, not `nosuchfn2/0` too.
                }
            }
        }

        Expr::NamespacedCall { args, .. } => {
            for a in args {
                build_call_graph(a, scope, enclosing, graph, roots);
            }
        }

        Expr::Object(entries) => {
            for entry in entries {
                if let ObjectKey::Expr(k) = &entry.key {
                    build_call_graph(k, scope, enclosing, graph, roots);
                }
                build_call_graph(&entry.value, scope, enclosing, graph, roots);
            }
        }

        Expr::StringInterpolation(parts) => {
            for part in parts {
                if let StringPart::Expr(e) = part {
                    build_call_graph(e, scope, enclosing, graph, roots);
                }
            }
        }

        Expr::Builtin(builtin) => match builtin_kids(builtin) {
            BuiltinKids::None => {}
            BuiltinKids::One(a) => build_call_graph(a, scope, enclosing, graph, roots),
            BuiltinKids::Two(a, b) => {
                build_call_graph(a, scope, enclosing, graph, roots);
                build_call_graph(b, scope, enclosing, graph, roots);
            }
            BuiltinKids::Three(a, b, c) => {
                build_call_graph(a, scope, enclosing, graph, roots);
                build_call_graph(b, scope, enclosing, graph, roots);
                build_call_graph(c, scope, enclosing, graph, roots);
            }
        },
    }
}

/// [`build_call_graph`] over every computed key in `patterns` -- the mirror
/// of `check`'s `check_pattern_keys`, which compile-checks the same keys in
/// the same scope. A key is an ordinary expression, so a call inside one is
/// an ordinary edge from `enclosing`.
fn build_call_graph_pattern_keys(
    patterns: &[Pattern],
    scope: &mut ReachScope,
    enclosing: Option<usize>,
    graph: &mut BTreeMap<usize, Vec<usize>>,
    roots: &mut Vec<usize>,
) {
    for pattern in patterns {
        match pattern {
            Pattern::Var(_) => {}
            Pattern::Object(entries) => {
                for entry in entries {
                    if let ObjectKey::Expr(key) = &entry.key {
                        build_call_graph(key, scope, enclosing, graph, roots);
                    }
                    build_call_graph_pattern_keys(
                        core::slice::from_ref(&entry.pattern),
                        scope,
                        enclosing,
                        graph,
                        roots,
                    );
                }
            }
            Pattern::Array(elements) => {
                build_call_graph_pattern_keys(elements, scope, enclosing, graph, roots);
            }
        }
    }
}

/// Every `def` body address reachable from `roots` by following `graph`
/// -- iterative, not recursive, so a long call chain (or the pathological
/// self-recursive/mutually-recursive cases this graph handles trivially,
/// same as any graph reachability computation) never grows this pass's own
/// stack the way naive recursion would.
fn compute_reachable(graph: &BTreeMap<usize, Vec<usize>>, roots: &[usize]) -> BTreeSet<usize> {
    let mut reachable = BTreeSet::new();
    let mut stack: Vec<usize> = roots.to_vec();
    while let Some(id) = stack.pop() {
        if reachable.insert(id) {
            if let Some(callees) = graph.get(&id) {
                stack.extend(callees.iter().copied());
            }
        }
    }
    reachable
}

/// Check every function call in `expr` against the `def`s, parameters and
/// builtins in scope at its position, the way real jq's compiler does.
///
/// Returns the first unresolvable call in traversal order, if any. See
/// [`resolve_func_calls_all`] to collect every one, matching jq's own
/// `jq: N compile errors` behaviour.
///
/// Must run *after* `ModuleLoader::process_program`: that is what inlines
/// `include`/`import`/`~/.jq` definitions as `Expr::FuncDef` wrappers around
/// the program and rewrites `ns::f` into a `FuncCall` named `ns::f`, matching
/// the wrapper it also creates. Running earlier would report every module
/// function as undefined.
///
/// # Reachability survives this function's own clones
///
/// `build_call_graph`'s reachability graph (#2740) identifies a `def` by
/// its body's heap address, read once off the tree before this function's
/// own `&mut`-mutating walk runs. That walk reallocates subtrees in three
/// places, and each used to strand every `def` inside -- checking it
/// against an address the graph had never seen, and silently skipping it:
///
/// - `check`'s `Expr::Builtin` arm clones each operand. Ordinary programs
///   hit this through the CLI: `[1]|map(def g: nosuchfn; g)`.
/// - `check`'s `Expr::FuncCall` arm, when a builtin name is shadowed by a
///   user `def`, unpacks the `builtin_fallback` parse -- by moving, except
///   for its own `Builtin` arm, which clones.
/// - `check`'s `Expr::Shared` arm reaches its subtree through `Rc::make_mut`,
///   which clones when an external caller holds another handle on the `Rc`.
///
/// All three now re-key the reachable set onto the copy (`rebase_reachable`),
/// so a caller of this public API gets the same result whether or not it
/// shares the `Rc` it passes in (#2971).
///
/// The invariant that keeps this closed: **after a clone, check the copy
/// against the re-keyed set only, never the enclosing one.** The enclosing
/// set still holds the addresses of trees already replaced and freed, and
/// the allocator reuses them -- consulting it is what once made an uncalled
/// def look called. Any new clone of a subtree inside `check` must follow
/// the same rule; a move need not, since moving an `Expr` carries its `Box`
/// pointers along unchanged.
pub fn resolve_func_calls(expr: &mut Expr) -> Result<(), UnresolvedCall> {
    match resolve_func_calls_all(expr).into_iter().next() {
        Some(first) => Err(first),
        None => Ok(()),
    }
}

/// Like [`resolve_func_calls`], but keeps traversing past an unresolvable
/// call instead of stopping at the first.
///
/// Returns every unresolvable call it finds, in traversal order (which
/// follows source order for every existing `Expr` variant). Real jq reports
/// every unresolvable call in one compile pass (`jq: N compile errors`);
/// this is what lets the jq runner match that instead of always reporting
/// `jq: 1 compile error` (#2037).
///
/// A thin filter over [`resolve_all`] (#2734): unaffected in content or
/// order by variable resolution running in the same walk, since it only
/// keeps the [`ResolveError::Call`] entries. Exists as its own function
/// because `yq_runner.rs` calls it (via [`resolve_func_calls`]) and must
/// keep seeing function-only results — real yq has no compile-time variable
/// check to match (#2981), so folding variable errors into its output would
/// make succinctly yq reject programs real yq accepts.
///
/// This filter is about *variable* errors only, not a promise that every
/// `Call` entry predates #2734 unchanged — `check_pattern_keys`'s own doc
/// comment (private, this module) records the one place a `Call` entry can
/// newly appear here too (a pattern's computed key), and why that is the
/// correct extension of yq mode's own pre-existing function-call policy
/// rather than a new one.
pub fn resolve_func_calls_all(expr: &mut Expr) -> Vec<UnresolvedCall> {
    resolve_all(expr)
        .into_iter()
        .filter_map(|e| match e {
            ResolveError::Call(c) => Some(c),
            ResolveError::Var(_) => None,
            // #2964: breaks are checked only in jq mode. Confirmed live
            // against the pinned Homebrew yq (v4.53.3): its *lexer* rejects
            // `break`/`label` syntax outright (`echo null | yq 'label $x |
            // break $x'` -> `Error: 1:1: lexer: invalid input text "label
            // $x | break..."`), so there is no real-yq-accepted `break`
            // program for a compile-time (or runtime) label-scope check to
            // apply to in the first place -- this is not the same situation
            // `ResolveError::Var`'s yq-mode exclusion is in (#2981, where
            // `$nope` genuinely is valid, permissive yq syntax verified
            // against the oracle). `succinctly yq` accepting `label`/`break`
            // syntax at all, ungated by `--jq-extensions`, is itself a
            // separate, pre-existing divergence from real yq (tracked
            // separately, not introduced or fixed here) -- this filter just
            // keeps this PR's new compile-time check jq-only, matching every
            // other jq-only construct `--jq-extensions` already gates in yq
            // mode (see docs/reference/yq-language.md).
            ResolveError::Break(_) => None,
        })
        .collect()
}

/// Combined compile-time check: every unresolved function call and every
/// unbound `$variable` reference (#2734).
///
/// Found in one walk over the tree so pattern-binding and `def`-scope
/// machinery isn't duplicated across a second full `Expr` match — #2885
/// already flags the drift risk of a third hand-written exhaustive match in
/// this file (`check` and `build_call_graph` are the existing two); this
/// keeps that count at two by folding variable tracking into `check` itself
/// instead of adding a fourth.
///
/// Returns diagnostics in a single traversal-ordered list, matching real
/// jq's own reporting: a program with both an unresolved call and an
/// unbound variable reports them interleaved by source position, not
/// grouped by kind (confirmed live: `$bar, foo, $baz` reports all three
/// left to right).
///
/// jq-mode only in practice — see [`resolve_func_calls_all`]'s doc comment
/// for why `yq_runner.rs` must keep using the function-only view instead.
pub fn resolve_all(expr: &mut Expr) -> Vec<ResolveError> {
    walk_resolve(expr).errors
}

/// [`resolve_all`], reduced to the diagnostics real jq itself reports (#3391).
///
/// jq's compiler hides every error beneath a compile unit that has one of its
/// own -- `def t: topmissing; t, bodymissing` is `1 compile error` (only
/// `bodymissing`), not two -- where [`resolve_all`] reports every error in the
/// tree. The rule is documented on the private `Blocks`. jq mode only: yq has no such rule to
/// match, and `resolve_func_calls_all` (which yq mode filters) keeps the
/// full list.
///
/// The errors that remain are in jq's own order (#3583), not always the
/// walk's source order: [`resolve_all`] keeps source order.
pub fn resolve_all_jq(expr: &mut Expr) -> Vec<ResolveError> {
    walk_resolve(expr).into_jq_reported()
}

/// The walk behind [`resolve_all`] and [`resolve_all_jq`].
fn walk_resolve(expr: &mut Expr) -> CheckCtx {
    // #2740: a `def` whose call never appears anywhere reachable is never
    // checked -- jq's own compiler never compiles such a body either, since
    // it only ever compiles a `def` at the call site substituting it in.
    // `build_call_graph` (read-only) computes which bodies are reachable
    // before `check` (below, `&mut`-mutating) walks the tree for real.
    let mut reach_scope = ReachScope::default();
    let mut graph = BTreeMap::new();
    let mut roots = Vec::new();
    build_call_graph(expr, &mut reach_scope, None, &mut graph, &mut roots);
    let reachable = compute_reachable(&graph, &roots);

    let mut cx = CheckCtx::default();
    check(expr, &mut cx, &reachable);
    cx
}

/// Whether `(name, arity)` is one of the pinned jq's own builtins.
fn is_jq_builtin(name: &str, arity: usize) -> bool {
    JQ_BUILTIN_ROSTER
        .iter()
        .any(|&(n, a)| a == arity && n == name)
}

/// Per-site occurrence counters, bundled into one argument so `check` and its
/// siblings stay within the argument limit.
///
/// Each counter is keyed by a counting scope: `None` for the main filter,
/// which is counted as one whole file, or the id of one visit to a
/// module-level def body (#3085, see [`ModuleDef`]).
#[derive(Default)]
struct Occurrences {
    calls: BTreeMap<(Option<usize>, String, usize), usize>,
    vars: BTreeMap<(Option<usize>, String), usize>,
    breaks: BTreeMap<(Option<usize>, String), usize>,
    /// One frame per module run open at this point of the walk.
    runs: Vec<RunFrame>,
    next_scope: usize,
}

/// A module run's counting state: how many defs of each `(name, arity)` it
/// has declared so far, and the module-level def whose body is being checked.
#[derive(Default)]
struct RunFrame {
    ordinals: BTreeMap<(String, usize), usize>,
    def: Option<(usize, ModuleDef)>,
}

/// The mutable state [`check`] threads through its whole walk (#3081): the
/// three lexical scopes it pushes and pops, the diagnostics it appends, and the
/// per-name occurrence counters those diagnostics carry. One `&mut CheckCtx`
/// replaces what used to be five positional arguments at every one of the ~45
/// recursive call sites, so the next tracked-scope concern is one more field
/// here rather than another parameter on every call -- and [`VarScope`] and
/// [`LabelScope`] being distinct types means a field cannot be read as the
/// other one.
///
/// `reachable` stays a separate parameter: it is read-only and varies per call
/// (`rebase_reachable`), where these fields live for the whole walk.
#[derive(Default)]
struct CheckCtx {
    scope: Scope,
    var_scope: VarScope,
    label_scope: LabelScope,
    errors: Vec<ResolveError>,
    /// Bundles #2635's call-occurrence counter with #2964's break-occurrence
    /// counter -- how many `break $name` sites the walk has already visited
    /// per label name, so a diagnostic carries which *occurrence* of the name
    /// -- the same "which `$x` actually failed" question `var_scope` can't
    /// answer by itself, resolved here by counting as we go (see
    /// `UnresolvedLabel::occurrence`).
    occurrences: Occurrences,
    /// The compile-unit tree the walk is inside (#3391) and, parallel to
    /// `errors`, the unit each diagnostic was found in.
    blocks: Blocks,
    error_blocks: Vec<usize>,
}

/// The compile units real jq reports errors against (#3391).
///
/// jq's compiler resolves one *block* at a time: it reports every unresolved
/// call, `$variable` and `break` that sits directly in the block, and compiles
/// the closures the block owns -- each `def` body, and each argument of a call
/// to a jq-defined function -- only when that block raised nothing. So a block
/// with an error of its own hides every error beneath it, while sibling
/// blocks are each still reported (`def a: x; def b: y; a, b` is two errors,
/// `def a: x; a, y` is one). The main program is block 0.
///
/// A block is *created* by [`check_in_block`]; what counts as one is decided
/// at each arm of [`check`], from what jq itself compiles to a closure.
///
/// The walk visits the tree in source order, which keeps every occurrence
/// counter (and so every caret position) in source order too. jq's own order
/// differs for a few constructs (#3583), so a block's diagnostics and its
/// child blocks are kept in two lists the walk can re-sequence afterwards
/// ([`CheckCtx::reorder`]) instead of being read back off the walk's order.
struct Blocks {
    /// The block the walk is currently inside.
    current: usize,
    /// `children[b]`: the blocks created directly in block `b`, in the order
    /// jq compiles them. One entry per block, so its length is the block
    /// count.
    children: Vec<Vec<usize>>,
    /// `own[b]`: indices into [`CheckCtx::errors`] of the diagnostics found
    /// directly in block `b`, in the order jq reports them.
    own: Vec<Vec<usize>>,
}

impl Default for Blocks {
    fn default() -> Self {
        Self {
            current: 0,
            children: alloc::vec![Vec::new()],
            own: alloc::vec![Vec::new()],
        }
    }
}

/// How far the current block's two ordered lists have grown: a boundary
/// between the operands of a construct whose jq order is not its source order
/// (#3583). `check` takes one before the first operand and one after each.
#[derive(Clone, Copy)]
struct Mark {
    children: usize,
    own: usize,
}

impl CheckCtx {
    /// Records a diagnostic against the block the walk is in.
    fn push_error(&mut self, error: ResolveError) {
        let block = self.blocks.current;
        self.blocks.own[block].push(self.errors.len());
        self.error_blocks.push(block);
        self.errors.push(error);
    }

    /// Drops every diagnostic recorded after the first `len`.
    fn truncate_errors(&mut self, len: usize) {
        let mut touched: Vec<usize> = self.error_blocks[len..].to_vec();
        touched.sort_unstable();
        touched.dedup();
        for block in touched {
            self.blocks.own[block].retain(|&error| error < len);
        }
        self.errors.truncate(len);
        self.error_blocks.truncate(len);
    }

    /// The current block's list lengths, as the boundary of one operand.
    fn mark(&self) -> Mark {
        let block = self.blocks.current;
        Mark {
            children: self.blocks.children[block].len(),
            own: self.blocks.own[block].len(),
        }
    }

    /// Re-sequences the operands `marks` bound (`marks[i]..marks[i + 1]` is
    /// operand `i`) in the current block: its child blocks as `compiled`
    /// says, and its own diagnostics as `reported` says. Each is a
    /// permutation of the operand indices, or `None` to keep source order.
    fn reorder(&mut self, marks: &[Mark], compiled: Option<&[usize]>, reported: Option<&[usize]>) {
        let block = self.blocks.current;
        if let Some(order) = compiled {
            let bounds: Vec<usize> = marks.iter().map(|m| m.children).collect();
            permute_spans(&mut self.blocks.children[block], &bounds, order);
        }
        if let Some(order) = reported {
            let bounds: Vec<usize> = marks.iter().map(|m| m.own).collect();
            permute_spans(&mut self.blocks.own[block], &bounds, order);
        }
    }

    /// [`Self::reorder`] with the child blocks of the operands `marks` bounds
    /// last to first -- the rule for an operator, a C builtin and an
    /// interpolation. Fewer than two operands (`marks` empty, or one operand)
    /// have nothing to reverse.
    fn reorder_reversed(&mut self, marks: &[Mark]) {
        if marks.len() > 2 {
            let order: Vec<usize> = (0..marks.len() - 1).rev().collect();
            self.reorder(marks, Some(&order), None);
        }
    }

    /// The diagnostics real jq reports: those of every block with no
    /// ancestor block that raised one of its own (#3391), in the order jq
    /// reports them (#3583).
    fn into_jq_reported(self) -> Vec<ResolveError> {
        let mut order = Vec::with_capacity(self.errors.len());
        let mut pending = alloc::vec![0usize];
        while let Some(block) = pending.pop() {
            if self.blocks.own[block].is_empty() {
                pending.extend(self.blocks.children[block].iter().rev());
            } else {
                order.extend_from_slice(&self.blocks.own[block]);
            }
        }
        let mut errors: Vec<Option<ResolveError>> = self.errors.into_iter().map(Some).collect();
        order
            .into_iter()
            .filter_map(|index| errors[index].take())
            .collect()
    }
}

/// Rewrites `list[bounds[0]..bounds[n]]` so the `n` spans `bounds` delimits
/// come out in `order`, a permutation of `0..n`.
fn permute_spans(list: &mut [usize], bounds: &[usize], order: &[usize]) {
    debug_assert_eq!(order.len() + 1, bounds.len(), "one order entry per span");
    debug_assert!(
        bounds.windows(2).all(|w| w[0] <= w[1]),
        "span bounds only grow: a walk never shortens the list it is marking"
    );
    let start = bounds[0];
    let end = bounds[bounds.len() - 1];
    let original = list[start..end].to_vec();
    let mut at = start;
    for &operand in order {
        for &item in &original[bounds[operand] - start..bounds[operand + 1] - start] {
            list[at] = item;
            at += 1;
        }
    }
}

/// [`check`] `expr` as a compile unit of its own, a child of the one the walk
/// is in (#3391) -- see [`Blocks`].
fn check_in_block(expr: &mut Expr, cx: &mut CheckCtx, reachable: &BTreeSet<usize>) {
    let parent = cx.blocks.current;
    let block = cx.blocks.children.len();
    cx.blocks.children[parent].push(block);
    cx.blocks.children.push(Vec::new());
    cx.blocks.own.push(Vec::new());
    cx.blocks.current = block;
    check(expr, cx, reachable);
    cx.blocks.current = parent;
}

impl Occurrences {
    fn scope(&self) -> Option<usize> {
        self.runs.last()?.def.as_ref().map(|(scope, _)| *scope)
    }

    fn module_def(&self) -> Option<ModuleDef> {
        self.runs.last()?.def.as_ref().map(|(_, def)| def.clone())
    }

    /// Whether a `def` visited now is one of the open run's module-level
    /// defs rather than one nested inside a def body.
    fn at_module_level(&self) -> bool {
        self.runs.last().is_some_and(|run| run.def.is_none())
    }
}

/// The name a module-level def was written under: the loader prefixes an
/// imported def with its alias (`a::f`) and a linked one with its link-run
/// name, and neither appears in the module's own source.
fn written_def_name(name: &str) -> &str {
    name.rsplit_once("::").map_or(name, |(_, written)| written)
}

/// Records one more visit to `(name, arity)` -- resolved or not -- and
/// returns how many earlier visits to that exact pair `occurrences` had
/// already recorded (#2635). Shared by every terminal arm of `check`'s
/// `Expr::FuncCall` handling that represents a genuine, distinct call site
/// (not the "restore the original parse and re-dispatch" arm, which revisits
/// the same position rather than a new one).
fn next_call_occurrence(occurrences: &mut Occurrences, name: &str, arity: usize) -> usize {
    let scope = occurrences.scope();
    let count = occurrences
        .calls
        .entry((scope, name.to_string(), arity))
        .or_insert(0);
    let index = *count;
    *count += 1;
    index
}

/// Whether `(name, arity)` resolves against `scope`, innermost first,
/// stopping at the module-scope floor (see [`ModuleRun`]).
///
/// Returns the innermost open run alongside the hit, which the caller needs
/// twice: to retry a bare miss as `alias::name` inside an `import`ed module
/// (#2989), and to attribute the resulting compile error to the module the
/// call was written in rather than to `<top-level>` (#2951).
fn in_scope(scope: &Scope, name: &str, arity: usize) -> ScanResult<()> {
    scope.lookup(name, arity, ModuleRun::is_link_name(name))
}

/// Whether `name` resolves against a plain name-stack scope (`VarScope` or
/// `LabelScope`, both name stacks), innermost first, stopping at the
/// same module-scope floor as [`in_scope`] (#2962) — the shared lookup body
/// [`in_var_scope`] and [`in_label_scope`] both wrap, so a change to how
/// name-stack lookups work has exactly one definition to update.
fn in_name_scope(scope: &[String], name: &str) -> ScanResult<()> {
    scan_scope(scope, String::as_str, |n| (n == name).then_some(()), false)
}

/// [`in_name_scope`] for `$name` against `var_scope`: a `$`-parameter of the
/// def a dependency is spliced into is not the dependency's to see.
fn in_var_scope(var_scope: &VarScope, name: &str) -> ScanResult<()> {
    in_name_scope(var_scope.names(), name)
}

/// [`in_name_scope`] for `$*label-name` against `label_scope` —
/// [`in_var_scope`]'s sibling for labels (#2964).
fn in_label_scope(label_scope: &LabelScope, name: &str) -> ScanResult<()> {
    in_name_scope(label_scope.names(), name)
}

/// The innermost module run enclosing this point in `label_scope`,
/// regardless of whether any particular label name matches.
///
/// [`in_label_scope`]'s `run` field is a [`ScanResult`] contract detail:
/// meaningful only on a miss, because the scan stops as soon as it finds a
/// hit. That is enough to attribute an *unresolved* break to its module, but
/// [`UnresolvedLabel::occurrence`]'s counter needs a run id for every
/// visited break, bound or not, so same-named breaks in different files
/// don't share one counter and drift each other's occurrence index. This is
/// the same [`scan_scope`] walk with a probe that never matches, so it
/// always runs to the floor (or the top) instead of stopping early.
fn enclosing_run(label_scope: &LabelScope) -> Option<u32> {
    scan_scope(label_scope.names(), String::as_str, |_| None::<()>, false)
        .run
        .map(|(id, _)| id)
}

/// #2964: the compile-time label-scope check for one `break $name` site,
/// shared by the bare [`Expr::Break`] arm and the `FuncCall` shadowed-`error`
/// pre-check (which re-examines a break the shadowing branch would otherwise
/// discard) — both need the identical bump-the-occurrence-counter-then-check
/// sequence, and having two copies invites them drifting apart silently
/// (this file's own [`scan_scope`] doc comment warns about exactly that
/// failure mode, citing #106).
fn check_break(name: &str, cx: &mut CheckCtx) {
    // Both a bound and an unbound break consume a slot in this scope's
    // counter, mirroring the source's `collect_break_sites` table.
    let origin = enclosing_run(&cx.label_scope);
    let scope = cx.occurrences.scope();
    let n = cx
        .occurrences
        .breaks
        .entry((scope, name.to_string()))
        .or_insert(0);
    let occurrence = *n;
    *n += 1;
    // Label scope is determined at the break's *own* lexical position -- a
    // `label $x` only covers nodes nested inside its body, not siblings of
    // that body. `label_scope` is a stack pushed/popped by the `Label` arm,
    // so this check sees only genuinely enclosing labels.
    if in_label_scope(&cx.label_scope, name).hit.is_none() {
        cx.push_error(ResolveError::Break(UnresolvedLabel {
            name: name.to_string(),
            occurrence,
            origin,
            module_def: cx.occurrences.module_def(),
        }));
    }
}

/// A pattern's computed keys (`{(EXPR): P}`, #2677) are ordinary
/// sub-expressions, evaluated in the scope the pattern itself sits in —
/// *before* any of its own names are bound — so they need the same
/// [`check`] treatment as any other sub-expression, function calls and
/// variables both. `Pattern` binding names themselves are collected
/// separately, read-only, by the existing [`collect_pattern_var_names`]
/// (`eval.rs`) once every alternative's keys have been checked.
///
/// [`check`]'s own `AsPattern`/`Reduce`/`Foreach` arms did not visit
/// `patterns` at all before this pass existed (folded away by `..`), so a
/// computed key referencing an undefined function or variable was silently
/// unchecked — this closes that gap as a side effect of needing to visit the
/// same nodes for #2734's own purposes, not a separate fix.
///
/// **Reaches yq mode too, unlike the rest of #2734.** The *variable* half
/// stays jq-only (`resolve_func_calls_all` filters `ResolveError::Var` out
/// for `yq_runner.rs`, per that function's own doc comment), but the *call*
/// half does not: a computed key's undefined function call is a genuine
/// `ResolveError::Call`, indistinguishable here from one found anywhere else
/// in the tree, and `resolve_func_calls_all` keeps every `Call` entry
/// regardless of where `check` found it. This is intentional, not
/// collateral damage: `yq_runner.rs` already runs `resolve_func_calls`
/// unconditionally over every program (an undefined function anywhere else
/// already fails to compile in yq mode today, confirmed live:
/// `succinctly yq 'nosuchfn'` → exit 1, `nosuchfn/0 is not defined`) — the
/// pre-existing gap being closed here was that one specific position
/// (inside a pattern's computed key) silently escaped that same,
/// already-established policy, not that yq mode gained a new one.
fn check_pattern_keys(pattern: &mut Pattern, cx: &mut CheckCtx, reachable: &BTreeSet<usize>) {
    match pattern {
        Pattern::Var(_) => {}
        Pattern::Object(entries) => {
            for entry in entries.iter_mut() {
                if let ObjectKey::Expr(k) = &mut entry.key {
                    check(k, cx, reachable);
                }
                check_pattern_keys(&mut entry.pattern, cx, reachable);
            }
        }
        Pattern::Array(patterns) => {
            for p in patterns.iter_mut() {
                check_pattern_keys(p, cx, reachable);
            }
        }
    }
}

/// Checks every `?//`-alternative's computed keys, then returns the union of
/// every name any alternative could bind, deduped -- the shared shape
/// `Expr::AsPattern`/`Expr::Reduce`/`Expr::Foreach` each need before binding
/// their body/update/extract. Delegates the union-and-dedup step to
/// `eval.rs`'s existing [`pattern_alternatives_var_names`] rather than
/// re-deriving it a third time here — its own doc comment records #2180
/// already finding and closing five copies of exactly this shape.
fn bind_patterns(
    patterns: &mut [Pattern],
    cx: &mut CheckCtx,
    reachable: &BTreeSet<usize>,
) -> Vec<String> {
    check_pattern_keys_all(patterns, cx, reachable);
    pattern_alternatives_var_names(patterns)
}

/// Checks every `?//`-alternative's computed keys, in the scope the caller is
/// in. [`bind_patterns`] without the union of names, for a caller that has
/// something to check between the keys and the binding (`reduce`'s `init`).
fn check_pattern_keys_all(
    patterns: &mut [Pattern],
    cx: &mut CheckCtx,
    reachable: &BTreeSet<usize>,
) {
    for pattern in patterns.iter_mut() {
        check_pattern_keys(pattern, cx, reachable);
    }
}

/// The arity an `Expr::FuncCall` with these `args` and `builtin_fallback`
/// was written with.
///
/// A shadowable call (#2036) carries its real sub-expressions in the
/// fallback and leaves `args` empty until this pass resolves it, so `args`
/// alone reads 0 for it. Public for the module loader, which renames calls
/// by (name, arity) before this pass has run (#2962).
#[must_use]
pub fn call_arity(args: &[Expr], builtin_fallback: Option<&Expr>) -> usize {
    if args.is_empty() {
        builtin_fallback.map_or(0, builtin_fallback_arity)
    } else {
        args.len()
    }
}

/// #2036: the arity `fallback` -- a successfully-parsed builtin or
/// fixed-arity special form stashed on `Expr::FuncCall::builtin_fallback` --
/// represents. Read-only: never clones or allocates, so computing it to
/// decide `in_scope` costs nothing beyond the match itself, regardless of
/// how large `fallback`'s own children are.
fn builtin_fallback_arity(fallback: &Expr) -> usize {
    if let Some(arity) = super::parser::join_expr_arity(fallback) {
        return arity;
    }
    match fallback {
        Expr::Not => 0,
        Expr::Limit { .. } | Expr::Until { .. } | Expr::While { .. } => 2,
        Expr::Repeat(_) | Expr::FirstExpr(_) | Expr::LastExpr(_) => 1,
        // #2036 review round 2: `range(N)` -- the 1-arg sugar form --
        // desugars to the exact same `Range { from: Literal(0), to:
        // Some(_), step: None }` shape a genuine 2-arg `range(0; N)` call
        // produces, so the general `Expr::Range` arm below cannot tell them
        // apart. `parse_range_expr` (`parser.rs`) marks the 1-arg form by
        // wrapping it in `Expr::Paren` (otherwise unused for this purpose,
        // and a no-op everywhere else) precisely so this arm can report the
        // correct arity of 1 here instead of the wrong 2.
        Expr::Paren(inner) if matches!(**inner, Expr::Range { .. }) => 1,
        Expr::Range { to, step, .. } => 1 + usize::from(to.is_some()) + usize::from(step.is_some()),
        Expr::Error(msg) => usize::from(msg.is_some()),
        // #2687: `break $x` desugars to a call to `error/0` -- see
        // `parser.rs`'s `parse_break_expr`. The label name is not an
        // argument (it's not even in scope as a call argument would be),
        // so this is arity 0, same as a bare `error`.
        Expr::Break(_) => 0,
        Expr::Builtin(builtin) => match builtin_kids(builtin) {
            BuiltinKids::None => 0,
            BuiltinKids::One(_) => 1,
            BuiltinKids::Two(_, _) => 2,
            BuiltinKids::Three(_, _, _) => 3,
        },
        // Genuinely unreachable: `wrap_shadowable_call` (`parser.rs`) only
        // ever stores one of the shapes matched above in
        // `builtin_fallback` -- anything else (a plain `Expr::FuncCall`, or
        // any postfix wrapping of one, produced when a dedicated parser's
        // own #2110/#2237 wrong-arity rewind fires) is returned unwrapped,
        // with `builtin_fallback` left `None`, specifically so it can never
        // reach here. Kept as a defensive fallback rather than `unreachable!()`
        // since the two functions have no compiler-enforced link to
        // `wrap_shadowable_call`'s own match -- see that function's doc
        // comment for the bug this arm used to silently paper over.
        _ => 0,
    }
}

/// #2036: moves `fallback`'s own sub-expressions out as the argument list a
/// generic `NAME(args;args)` call over the same source span would have
/// produced -- called only once [`check`] has determined the call is
/// genuinely shadowed by an in-scope `def`. Takes `fallback` *by value* and
/// moves its children rather than cloning them: `fallback` is discarded
/// immediately after this returns (the caller already took it out of
/// `builtin_fallback` via `Option::take`), so there is nothing left that
/// would need its own independent copy -- cloning here, even though it
/// would only run once per confirmed-shadowed node rather than compounding
/// across every declined one, could still duplicate an arbitrarily large
/// not-yet-resolved subtree sitting in one of `fallback`'s own arguments.
fn builtin_fallback_into_args(fallback: Expr) -> Vec<Expr> {
    // #3046: `JOIN` desugars to an `as` binding; see `parser::join_expr`.
    let fallback = match super::parser::join_expr_into_args(fallback) {
        Ok(args) => return args,
        Err(fallback) => fallback,
    };
    match fallback {
        Expr::Not => Vec::new(),
        Expr::Limit { n, expr } => alloc::vec![*n, *expr],
        Expr::Until { cond, update } | Expr::While { cond, update } => {
            alloc::vec![*cond, *update]
        }
        Expr::Repeat(inner) | Expr::FirstExpr(inner) | Expr::LastExpr(inner) => {
            alloc::vec![*inner]
        }
        // #2036 review round 2: the 1-arg `range(N)` marker -- see
        // `builtin_fallback_arity`'s matching arm. The single real argument
        // the user wrote is `to` (the synthesized `from: Literal(0)` was
        // never written and must not be surfaced as a second argument).
        Expr::Paren(inner) if matches!(*inner, Expr::Range { .. }) => match *inner {
            Expr::Range { to: Some(to), .. } => alloc::vec![*to],
            // `parse_range_expr` only ever produces this marker with `to:
            // Some(_)` -- defensive, not reachable.
            Expr::Range { .. } => Vec::new(),
            _ => unreachable!("guarded by the outer match arm's pattern"),
        },
        Expr::Range { from, to, step } => {
            let mut args = alloc::vec![*from];
            args.extend(to.map(|b| *b));
            args.extend(step.map(|b| *b));
            args
        }
        Expr::Error(msg) => msg.into_iter().map(|b| *b).collect(),
        // #2687: `break $x`'s label name is not surfaced as an `error/0`
        // argument -- see `builtin_fallback_arity`'s matching arm. Once this
        // arm is reached, the shadowing `def error:` has already won (`check`
        // only calls this after confirming `in_scope`), so the original
        // `Expr::Break` -- label name included -- is simply discarded; it was
        // never going to reach the label-catching machinery either way.
        Expr::Break(_) => Vec::new(),
        // `builtin_kids` only ever borrows -- there is no by-value
        // counterpart, so this one case clones rather than moves. Bounded
        // even so: it can only fire once per node that check() has just
        // confirmed is genuinely shadowed, immediately followed by
        // recursing into (and thereby resolving/shrinking) each cloned
        // child -- it does not compound across the *declined* case above
        // (`*expr = *fallback`, always a pure move), which is the shape
        // nested-but-not-actually-shadowed candidates take and the one
        // this issue's own review found exponential before this fix.
        Expr::Builtin(builtin) => match builtin_kids(&builtin) {
            BuiltinKids::None => Vec::new(),
            BuiltinKids::One(a) => alloc::vec![a.clone()],
            BuiltinKids::Two(a, b) => alloc::vec![a.clone(), b.clone()],
            BuiltinKids::Three(a, b, c) => alloc::vec![a.clone(), b.clone(), c.clone()],
        },
        // Genuinely unreachable -- see `builtin_fallback_arity`'s own
        // identical fallback arm, which this must stay consistent with.
        _ => Vec::new(),
    }
}

/// Recurse into `expr` under `cx`'s scopes, restoring every scope in `cx` to
/// what it was on entry before returning so a sibling never sees a binding
/// introduced by its neighbour. Appends every unresolvable call, variable and
/// label to `cx.errors` rather than stopping at the first, matching how real
/// jq's own compiler keeps going to report every compile error in one pass
/// (#2037). `reachable` is the set of `def` bodies a call can reach (#2740).
fn check(expr: &mut Expr, cx: &mut CheckCtx, reachable: &BTreeSet<usize>) {
    match expr {
        // #1371: neither variant can occur here. This pass runs once, on the
        // freshly parsed program, before evaluation begins; both are built
        // *by* evaluation. They are still given real arms rather than being
        // folded into a leaf group, so that if a future caller ever runs this
        // check over an evaluation-time tree it reports honestly instead of
        // silently treating a whole subtree as having no calls in it.
        //
        // A `DefCall` is by construction already resolved -- it holds the
        // definition it resolved to -- so only its arguments can carry an
        // unresolved call, and they are checked in the scope this node sits
        // in. The definition's own body was checked at its `FuncDef`.
        //
        // `Shared` wraps an `Rc`. `resolve_func_calls`/`Expr::Shared` are
        // both public API, so an external caller can hand this a multi-owner
        // `Rc` even though this crate's own 3 CLI call sites never do (see
        // above) -- `Rc::make_mut` (clone-on-write if not uniquely owned,
        // unlike `Rc::get_mut`'s silent "return None, skip this subtree
        // entirely" on the same case) keeps recursion unconditional exactly
        // like the pre-`&mut Expr` version of this arm did, at the cost of
        // one clone in the (self-inflicted, still never hit by this crate's
        // own callers) multi-owner case.
        //
        // #2971: that clone reallocates every `def` body inside, exactly as
        // the builtin arm's does, so it is re-keyed the same way. A handle on
        // the pre-clone tree is kept only when `make_mut` is going to clone
        // -- the uniquely owned case, which is every case this crate's own
        // callers produce, pays nothing.
        Expr::Shared(inner) => {
            if Rc::strong_count(inner) > 1 || Rc::weak_count(inner) > 0 {
                // Holding `original` keeps the pre-clone tree alive while it is
                // paired with the copy -- and guarantees `make_mut` clones.
                let original = Rc::clone(inner);
                let target = Rc::make_mut(inner).expr_mut();
                let rebased = rebase_reachable(&original, target, reachable);
                check(target, cx, &rebased);
            } else {
                check(Rc::make_mut(inner).expr_mut(), cx, reachable);
            }
        }
        Expr::DefCall { args, .. } => {
            for arg in args.iter_mut() {
                check_in_block(arg, cx, reachable);
            }
        }
        // Leaves: nothing nested to descend into. Mirrors `walk::any_subexpr`'s
        // own grouping so the two stay comparable arm for arm. `Var` is
        // checked separately below (#2734) rather than folded in here.
        Expr::Identity
        | Expr::Field(_)
        | Expr::Index { .. }
        | Expr::Slice { .. }
        | Expr::ArrayKey(_)
        | Expr::Iterate
        | Expr::Literal(_)
        | Expr::RecursiveDescent
        | Expr::RecursiveDescentWithKeys
        | Expr::Not
        | Expr::Format(_)
        | Expr::TrackedVar(_)
        | Expr::DeferredVar(_)
        | Expr::Loc { .. } => {}

        // #3029: `$ENV` is an ordinary, shadowable binding in jq (and in
        // real yq -- both oracles answer `1` for `1 as $ENV | $ENV`,
        // confirmed live). The parser lowers *every* `$ENV` reference to
        // `Expr::Env` at parse time with no knowledge of enclosing bindings
        // (`dollar_var_expr` in `parser.rs`), so that decision is deferred
        // to here, the one scope-aware walk over the freshly parsed tree:
        // when the name `ENV` is bound in the lexical `var_scope`, the
        // reference becomes a plain `Expr::Var("ENV")`, which both
        // evaluators already resolve against the runtime scope -- no
        // evaluation rule needs its own shadowing case. An unbound
        // reference keeps `Expr::Env` and reads the environment object.
        // The lexical `def f: $ENV; 1 as $ENV | f` row stays on the
        // environment because a `def` body is descended at its own textual
        // scope (`var_scope` is truncated before `then`), so the body's
        // `$ENV` never sees the later `as` binding.
        Expr::Env => {
            if in_var_scope(&cx.var_scope, "ENV").hit.is_some() {
                *expr = Expr::Var("ENV".to_string());
            }
        }

        // #2964: real jq rejects `break $name` at *compile* time (exit 3)
        // when there is no lexically enclosing `label $name`, reporting
        // `$*label-x is not defined` at the `break` keyword's position --
        // confirmed live against the pinned oracle. The check is
        // unconditional with respect to the `def error:` shadowing that
        // #2687 handles: a bare `break $x` outside any label compiles even
        // when `error/0` is shadowed (oracle: `def error: "S"; break $x`
        // → `$*label-x is not defined`, rc=3), so this arm fires before
        // #2687's `wrap_shadowable_call` desugaring can convert the node
        // into a `FuncCall`. When `def error` *is* in scope at the
        // break's own position, the `FuncCall` arm's fallback-restore path
        // (`*expr = *fallback; check(expr, ...)`) re-enters this exact arm
        // with the bare `Expr::Break` — the same reason `parse_break_expr`
        // records the site unconditionally regardless of the `error` wrap.
        //
        // `occurrences.breaks` carries *which* same-named break site this
        // one is, keyed by counting scope (the main filter, or one
        // module-level def -- `ModuleDef`) so a label repeated across
        // module boundaries doesn't have one file's breaks consume
        // another's slots. Breaks inside unreferenced `def` bodies are
        // counted too (#3085), so the index lines up with the CLI's
        // `BreakSite` table.
        Expr::Break(name) => {
            check_break(name, cx);
        }

        // #2734: the leaf this whole pass exists to add. `$ENV`/`$__loc__`
        // never reach here at all -- the parser lowers them straight to
        // `Expr::Env`/`Expr::Loc` (see `UnboundVar`'s own doc comment) -- and
        // `TrackedVar` is an evaluation-time-only substitute for a `Var` that
        // already resolved, never present on the freshly parsed tree this
        // pass runs on (same reasoning as the `Shared`/`DefCall` arm above).
        Expr::Var(name) => {
            let count = cx.occurrences
                .vars
                .entry((cx.occurrences.scope(), name.clone()))
                .or_insert(0);
            let occurrence = *count;
            *count += 1;
            let found = in_var_scope(&cx.var_scope, name);
            if found.hit.is_none() {
                cx.push_error(ResolveError::Var(UnboundVar {
                    name: name.clone(),
                    origin: found.run.map(|(id, _)| id),
                    occurrence,
                    module_def: cx.occurrences.module_def(),
                }));
            }
        }

        Expr::Optional(inner)
        | Expr::Array(inner)
        | Expr::Paren(inner)
        | Expr::Negate(inner) => check(inner, cx, reachable),

        // #3391: `first(f)`, `last(f)` and `repeat(f)` are jq-defined, so their
        // operand is a closure -- a compile unit of its own.
        Expr::FirstExpr(inner) | Expr::LastExpr(inner) | Expr::Repeat(inner) => {
            check_in_block(inner, cx, reachable);
        }

        // #2840: real jq's `label $x | BODY` intercepts every escape that
        // isn't its own `{"__jq":N}` break sentinel and re-raises it
        // through a synthetic call to `error/0` (`gen_call("error",
        // gen_noop())` in `parser.y`), resolved by ordinary lexical scope
        // at the label's own position -- so a `def error:` in scope there
        // shadows the re-raise exactly like any other call. That is
        // already `try BODY catch error`'s own semantics, an expression
        // every evaluator arm that walks `Expr::Label` already evaluates
        // correctly -- so the fix is this AST rewrite at check time, not a
        // change to any evaluator. Gated on `in_scope` so a program with no
        // `def error` in scope here parses to the exact `Expr::Label` node
        // it always has, byte-identical AST, zero evaluator cost (mirrors
        // #2687's `break $x` -> `error/0` desugaring's own
        // shadowable-or-untouched gate).
        Expr::Label { name, body } => {
            cx.label_scope.push(name.clone());
            check(body, cx, reachable);
            cx.label_scope.pop();
            if in_scope(&cx.scope, "error", 0).hit.is_some() {
                let old_body = core::mem::replace(body.as_mut(), Expr::Identity);
                **body = Expr::Try {
                    expr: Box::new(old_body),
                    catch: Some(Box::new(Expr::FuncCall {
                        name: "error".to_string(),
                        args: Vec::new(),
                        builtin_fallback: None,
                    })),
                };
            }
        }

        // #3391: `error(msg)` is jq-defined; its operand is a closure.
        Expr::Error(inner) => {
            if let Some(e) = inner.as_deref_mut() {
                check_in_block(e, cx, reachable);
            }
        }

        // #3583: jq builds an arithmetic or comparison operator as a call to
        // a C function, whose operands it compiles right to left, so the
        // closure units under `map(ua) + map(ub)` come out `ub`, `ua`. Only
        // the units are reversed: an error of the block itself stays in
        // source order (`ua + ub` is `ua`, `ub`), and `and`, `or` and `//`
        // are not such calls.
        Expr::Arithmetic {
            left,
            right,
            settle,
            ..
        } => {
            // The pass below can rewrite an operand in place (#3997).
            settle.forget();
            check_operator_operands(left, right, cx, reachable);
        }
        Expr::Compare { left, right, .. } => check_operator_operands(left, right, cx, reachable),

        Expr::And(left, right) | Expr::Or(left, right) | Expr::Alternative(left, right) => {
            check(left, cx, reachable);
            check(right, cx, reachable);
        }

        // #3583: `(target)[key]` is compiled key first, so both its own
        // errors and the units beneath it come out `key`, then `target`
        // (`(ua)[ub]` is `ub`, `ua`).
        Expr::IndexExpr {
            target: left,
            key: right,
        } => {
            let start = cx.mark();
            check(left, cx, reachable);
            let middle = cx.mark();
            check(right, cx, reachable);
            let end = cx.mark();
            cx.reorder(&[start, middle, end], Some(&[1, 0]), Some(&[1, 0]));
        }

        // #3391: each of these is a call to a jq-defined function in real jq
        // (`limit`, `until`, `while`, and `=`/`|=` via `_assign`/`_modify`),
        // so both operands are closures -- compile units of their own, not
        // part of the enclosing block.
        Expr::Limit {
            n: left,
            expr: right,
        }
        // `Expr::NthExpr` has no parser construction site at all -- `nth(n; f)`
        // parses to `Builtin::NthStream` (see `eval_generic.rs`'s own
        // `Builtin::NthStream` arm, which records the same finding). It is
        // named here because this match is exhaustive with no wildcard, not
        // because a parsed program can reach it, so it shows as uncovered and
        // no test can change that.
        | Expr::NthExpr {
            n: left,
            expr: right,
        }
        | Expr::Until {
            cond: left,
            update: right,
        }
        | Expr::While {
            cond: left,
            update: right,
        }
        | Expr::Assign {
            path: left,
            value: right,
        }
        | Expr::Update {
            path: left,
            filter: right,
        }
        | Expr::MetaAssign {
            target: left,
            value: right,
            ..
        } => {
            check_in_block(left, cx, reachable);
            check_in_block(right, cx, reachable);
        }

        // #3391: `a op= b` and `a //= b` evaluate `b` once, up front, as an
        // ordinary operand (`b as $x | a |= . op $x`); only the path `a` is a
        // closure, handed to `_modify`.
        //
        // #3583: for the same reason `b` is compiled before `a`, so the units
        // under `(map(ua)) += (map(ub))` come out `ub`, `ua`.
        Expr::CompoundAssign {
            path: left,
            value: right,
            ..
        }
        | Expr::AlternativeAssign {
            path: left,
            value: right,
        } => {
            let start = cx.mark();
            check_in_block(left, cx, reachable);
            let middle = cx.mark();
            check(right, cx, reachable);
            let end = cx.mark();
            cx.reorder(&[start, middle, end], Some(&[1, 0]), None);
        }

        // #3391: `JOIN` is jq-defined, so what the program wrote to it are
        // closures -- compile units of their own -- even though the parser
        // has desugared the call into this binding (`parser::join_expr`). Its
        // `$idx` is not nameable, so none of them can see it.
        Expr::As { expr, var, body } if var == JOIN_IDX_VAR => {
            for operand in join_expr_operands_mut(expr, body) {
                check_in_block(operand, cx, reachable);
            }
        }

        // #2734: binds `var` in `body` only, not `expr` -- `.foo as $x | ...`
        // evaluates `.foo` before `$x` exists.
        Expr::As { expr, var, body } => {
            check(expr, cx, reachable);
            cx.var_scope.push(var.clone());
            check(body, cx, reachable);
            cx.var_scope.pop();
        }

        // #2734: `patterns` is every `?//`-alternative (usually just one);
        // jq requires each alternative bind the same variable set, so the
        // union of all of them is pushed once for `body` rather than
        // rechecking `body` per alternative. Each alternative's own computed
        // keys (`{(EXPR): P}`) are checked in the *pre-binding* scope via
        // `check_pattern_keys`, which also closes a pre-existing gap: this
        // arm previously ignored `patterns` entirely (folded into the
        // generic two-child group above by `..`), so a computed key
        // referencing an undefined function or variable was never checked
        // at all.
        Expr::AsPattern {
            expr,
            patterns,
            body,
        } => {
            check(expr, cx, reachable);
            let outer = cx.var_scope.len();
            let bound = bind_patterns(patterns, cx, reachable);
            cx.var_scope.extend(bound);
            check(body, cx, reachable);
            cx.var_scope.truncate(outer);
        }

        // The one arm that changes function scope. `body` sees the function
        // itself (self-recursion is legal) plus its parameters as arity-0
        // functions; `then` sees the function but *not* its parameters.
        // Neither sees a `def` that comes later — which is exactly the
        // forward reference `expand_func_calls` could not detect, and the
        // reason `def f: g; def g: 42; f` computed `42` instead of failing.
        //
        // #2734: also the one arm that changes *variable* scope via a
        // `$`-style parameter (`def f($a): $a; ...` desugars to `def f(a): a
        // as $a | ...` in real jq, binding `$a` in `body` only, same as
        // `Expr::As` above) -- a bare `Param::Bare` binds no variable.
        Expr::FuncDef {
            name,
            params,
            body,
            then,
            ..
        } => {
            let outer = cx.scope.len();
            cx.scope.push(name.clone(), params.len(), ());
            let with_self = cx.scope.len();

            // A parameter binds its bare name at arity 0 only: `def f(g):
            // g(1)` is `g/1 is not defined` in jq, not a call to the outer
            // `g`. Pushed after the function's own name so a parameter
            // shadowing it wins. A `$`-style parameter also binds `$name`
            // in the same pass over `params`, one loop for both namespaces.
            let var_outer = cx.var_scope.len();
            for p in params.iter() {
                cx.scope.push(p.name().to_string(), 0, ());
                if p.is_dollar() {
                    cx.var_scope.push(p.name().to_string());
                }
            }
            // #2740: a `def` whose call never appears anywhere reachable is
            // never compiled by real jq either -- its own substitution model
            // only ever compiles a body at the call site that references it.
            // `build_call_graph` (run once, up front, in
            // `resolve_func_calls_all`) already answered this for every
            // `def` in the program by the time this walk reaches it, keyed
            // by the same body-address identity recomputed here -- so
            // skipping the check costs nothing (this is not a second
            // graph walk, just one `BTreeSet` lookup) and never revisits
            // scope/`then`, which still need the same treatment either way.
            let body_addr = body.as_ref() as *const Expr as usize;
            let marker = ModuleRun::parse(name);
            // #3085: a module-level def of an open run is its own counting
            // scope, identified the way the CLI's `DefSite` table can find
            // it again (see `ModuleDef`).
            let module_level = marker.is_none() && cx.occurrences.at_module_level();
            if module_level {
                let scope_id = cx.occurrences.next_scope;
                cx.occurrences.next_scope += 1;
                if let Some(run) = cx.occurrences.runs.last_mut() {
                    let written = written_def_name(name);
                    let seen = run
                        .ordinals
                        .entry((written.to_string(), params.len()))
                        .or_insert(0);
                    let ordinal = *seen;
                    *seen += 1;
                    run.def = Some((
                        scope_id,
                        ModuleDef {
                            name: written.to_string(),
                            arity: params.len(),
                            ordinal,
                        },
                    ));
                }
            }
            // #3391: a def body is a compile unit of its own, a child of the
            // block the `def` is written in.
            if reachable.contains(&body_addr) {
                check_in_block(body, cx, reachable);
            } else if !module_level {
                // #3085: an unreferenced body is not diagnosed, but its
                // sites are still in the source tables the occurrence
                // indexes point into, so they are still counted. A
                // module-level def needs no such walk: it is a counting
                // scope of its own, so skipping it shifts nothing.
                let kept = cx.errors.len();
                check_in_block(body, cx, reachable);
                cx.truncate_errors(kept);
            }
            if module_level {
                if let Some(run) = cx.occurrences.runs.last_mut() {
                    run.def = None;
                }
            }
            cx.var_scope.truncate(var_outer);
            cx.scope.truncate(with_self);

            // A run marker floors variables as well as functions (#2962), so
            // it goes on the variable stack too -- for `then` only, since a
            // marker's own body is `.`. #2964: labels get the same floor, for
            // the same reason -- a `label $x` above an `import`/`include`
            // must not leak into the module's own `break`s, and a break
            // inside the module must be attributable back to it (see
            // `enclosing_run`), neither of which works unless the marker is
            // visible on `label_scope` too.
            let is_marker = marker.is_some();
            let mut closed_run = None;
            if is_marker {
                cx.var_scope.push(name.clone());
                cx.label_scope.push(name.clone());
                // #3085: a begin opens a fresh run frame, even when its file
                // shares a run id with another import of the same module. An
                // end closes it for `then`; on return the begin that opened
                // it still owns it.
                match &marker {
                    Some(RunMarker::Begin { .. }) => cx.occurrences.runs.push(RunFrame::default()),
                    Some(RunMarker::End { .. }) => closed_run = cx.occurrences.runs.pop(),
                    None => {} // patchcov: coverage tolerate-line reason="unreachable by construction: this match is only entered when is_marker (marker.is_some()) is true, and RunMarker has only Begin/End variants -- the None arm exists solely for exhaustiveness against Option<RunMarker>'s type"
                }
            }
            check(then, cx, reachable);
            cx.var_scope.truncate(var_outer);
            if is_marker {
                if matches!(marker, Some(RunMarker::Begin { .. })) {
                    cx.occurrences.runs.pop();
                } else if let Some(run) = closed_run {
                    cx.occurrences.runs.push(run);
                }
                cx.label_scope.pop();
            }
            cx.scope.truncate(outer);
        }

        Expr::Try { expr, catch } => {
            check(expr, cx, reachable);
            if let Some(c) = catch.as_deref_mut() {
                check(c, cx, reachable);
            }
        }

        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            check(cond, cx, reachable);
            check(then_branch, cx, reachable);
            check(else_branch, cx, reachable);
        }

        // #3583: `(target)[from:to]` is compiled `from`, `to`, then the target
        // (`(ua)[ub:uc]` is `ub`, `uc`, `ua`).
        Expr::SliceExpr { target, start, end } => {
            let first = cx.mark();
            check(target, cx, reachable);
            let after_target = cx.mark();
            check_opt(start.as_deref_mut(), cx, reachable);
            let after_start = cx.mark();
            check_opt(end.as_deref_mut(), cx, reachable);
            let last = cx.mark();
            let marks = [first, after_target, after_start, last];
            cx.reorder(&marks, Some(&[1, 2, 0]), Some(&[1, 2, 0]));
        }

        // #3391: `range/1..3` are jq-defined; every operand is a closure.
        Expr::Range { from, to, step } => {
            check_in_block(from, cx, reachable);
            if let Some(to) = to.as_deref_mut() {
                check_in_block(to, cx, reachable);
            }
            if let Some(step) = step.as_deref_mut() {
                check_in_block(step, cx, reachable);
            }
        }

        // #2734: `patterns`' vars are bound in `update` only, not `init` --
        // `reduce .[] as $x (0; . + $x)` evaluates `init` before any element
        // has been bound. Same union-of-alternatives and computed-key
        // treatment as `Expr::AsPattern` above.
        //
        // The walk visits the pieces in source order -- the source, the
        // pattern's computed keys, `init`, the update -- so each occurrence
        // counter lines up with `collect_var_sites`/`collect_break_sites`,
        // which are sorted by text offset (#3583; before it, `init` was
        // visited first and a same-named `$var`/`break` in a key and in
        // `init` had each caret attributed to the other, #2964). jq itself
        // reports `init` first -- `reduce (1,2) as {($x): $v} ($x; .)` prints
        // `init`'s `$x` (column 29) before the key's (column 19) -- so the
        // pieces are re-sequenced afterwards, for the block's own errors and
        // for its units alike. The keys are checked in the pre-binding scope,
        // `init` sees no bound vars, and the update sees them.
        //
        // #2734: `patterns`' vars are bound in `update` only, not `init` --
        // `reduce .[] as $x (0; . + $x)` evaluates `init` before any element
        // has been bound. Same union-of-alternatives treatment as
        // `Expr::AsPattern` above.
        Expr::Reduce {
            input,
            patterns,
            init,
            update,
        } => {
            let start = cx.mark();
            check(input, cx, reachable);
            let after_input = cx.mark();
            check_pattern_keys_all(patterns, cx, reachable);
            let after_keys = cx.mark();
            check(init, cx, reachable);
            let after_init = cx.mark();
            let outer = cx.var_scope.len();
            cx.var_scope
                .extend(pattern_alternatives_var_names(patterns));
            check(update, cx, reachable);
            let end = cx.mark();
            cx.var_scope.truncate(outer);
            let marks = [start, after_input, after_keys, after_init, end];
            cx.reorder(&marks, Some(&[2, 0, 1, 3]), Some(&[2, 0, 1, 3]));
        }

        // #2734: same rule as `Expr::Reduce` above -- `init` sees no bound
        // vars, `update`/`extract` both do -- and the same visit and report
        // order (#3583).
        Expr::Foreach {
            input,
            patterns,
            init,
            update,
            extract,
        } => {
            let start = cx.mark();
            check(input, cx, reachable);
            let after_input = cx.mark();
            check_pattern_keys_all(patterns, cx, reachable);
            let after_keys = cx.mark();
            check(init, cx, reachable);
            let after_init = cx.mark();
            let outer = cx.var_scope.len();
            cx.var_scope
                .extend(pattern_alternatives_var_names(patterns));
            check(update, cx, reachable);
            let after_update = cx.mark();
            check_opt(extract.as_deref_mut(), cx, reachable);
            let end = cx.mark();
            cx.var_scope.truncate(outer);
            let marks = [
                start,
                after_input,
                after_keys,
                after_init,
                after_update,
                end,
            ];
            cx.reorder(&marks, Some(&[2, 0, 1, 3, 4]), Some(&[2, 0, 1, 3, 4]));
        }

        Expr::Pipe(exprs) => {
            for e in exprs.iter_mut() {
                check(e, cx, reachable);
            }
        }
        Expr::Comma(exprs) => {
            for e in exprs.iter_mut() {
                check(e, cx, reachable);
            }
        }

        // The check itself. Arguments are checked in the *caller's* scope, not
        // the callee's — an argument is an expression written at the call
        // site — but only when the callee itself resolves. Real jq's compiler
        // binds a call's argument closures to the callee's parameter slots,
        // which requires already having found the callee; an unresolved
        // callee means there is no such binding to attempt, so jq never
        // compiles the arguments at all and reports only the callee
        // (verified live: `nosucha(nosuchb; nosuchc)` is `nosucha/2 is not
        // defined`, one error, not three). Checking the arguments anyway —
        // the pre-#2037 code did, via `?`-propagation that just happened to
        // discard the extra errors by stopping at the first one — would
        // report errors real jq's compiler never reaches once every call is
        // collected instead of just the first (#2037).
        //
        // #2036: `builtin_fallback` is this node's alternate parse -- the
        // builtin or special form the parser would have produced had the
        // name never been shadowable -- attached only when the lexical
        // prescan flagged `name` as possibly `def`'d somewhere in the
        // program (see `Expr::FuncCall`'s own doc comment). `args` starts
        // out *empty* whenever `builtin_fallback` is `Some` (the parser
        // deliberately does not clone the fallback's own children into it
        // -- see `wrap_shadowable_call`'s own doc comment for why that
        // duplication made nested shadow candidates an `O(2^depth)`
        // parser-level denial-of-service), so the arity to check scope
        // against has to come from the fallback's own shape
        // (`builtin_fallback_arity`) whenever `args` is still empty, not
        // from `args.len()` directly.
        //
        // Real shadowing (`in_scope`) is checked *first* and wins outright:
        // a `def` at this exact `(name, arity)` always shadows the
        // builtin, matching real jq. Only then is `args` actually
        // populated -- moved out of `fallback` (`builtin_fallback_into_args`),
        // not cloned, since `fallback` is discarded immediately after and
        // there is nothing left needing its own copy. If shadowing is
        // declined instead, the *whole* fallback is moved back into `expr`
        // in one piece -- again no cloning -- and re-checked so its own
        // nested children get the identical treatment. Whichever way it
        // resolves, `builtin_fallback` is always `None` and `args` always
        // populated by the time this function returns for this node -- so
        // no other pass over the tree, before or after this one runs to
        // completion, ever needs to know `builtin_fallback` exists, or
        // sees a node whose `args` is emptily lying about its real arity.
        Expr::FuncCall {
            name,
            args,
            builtin_fallback,
        } => {
            let arity = call_arity(args, builtin_fallback.as_deref());
            let scan = in_scope(&cx.scope, name, arity);
            // #2989: a def inside an `import`ed module calls its siblings by
            // the bare name it is written with, but the loader namespaced
            // them to `alias::name`. Retry the bare miss under the innermost
            // open run's alias, and on a hit rename the call in place -- this
            // is already the `&mut` walk that rewrites #2036's shadow
            // candidates, so evaluation then finds the namespaced def with no
            // second pass and no evaluator change.
            let aliased = match (&scan.hit, &scan.run) {
                (None, Some((_, Some(alias)))) => {
                    let qualified = alloc::format!("{alias}::{name}");
                    in_scope(&cx.scope, &qualified, arity).hit.map(|()| qualified)
                }
                _ => None,
            };
            let resolved = scan.hit.is_some() || aliased.is_some();
            if let Some(qualified) = aliased {
                *name = qualified;
            }
            // #2635 review: keyed by `origin` too, not just `(name, arity)`
            // -- `occurrences` is one shared map across the whole merged
            // tree (main filter plus every inlined module), but each
            // origin's own `call_sites` table is independently re-parsed
            // from that origin's own text alone (`jq_runner.rs`). Without
            // this, a module's own earlier occurrence of `(name, arity)`
            // inflated the main filter's own count for the same pair (or
            // vice versa), landing `.nth(occurrence_index)` on the wrong
            // table entirely.
            let origin = scan.run.map(|(id, _)| id);
            if resolved {
                // Counts this call towards `occurrences` (a resolved one
                // still needs to, so a *later* same-name-same-arity call in
                // the same origin that fails knows how many earlier
                // occurrences -- resolved or not -- came before it in the
                // source; see `UnresolvedCall::occurrence_index`'s own doc
                // comment).
                next_call_occurrence(&mut cx.occurrences, name, arity);
                // #2971: of `builtin_fallback_into_args`'s arms only the
                // `Builtin` one clones -- the rest move their operands, which
                // keeps every `def` body where `build_call_graph` found it.
                // Where it clones, the new args are re-keyed; both address
                // lists come from `builtin_kids` order, so they pair up.
                let rebased = builtin_fallback.take().and_then(|fallback| {
                    let cloned_from =
                        matches!(*fallback, Expr::Builtin(_)).then(|| def_body_addrs(&fallback));
                    // #2964 pre-check: when `error` IS in scope (def exists),
                    // the break fallback is silently discarded by
                    // `builtin_fallback_into_args`. But real jq still checks
                    // the break's label scope — `def error: "S"; break $x` →
                    // `$*label-x is not defined`, rc=3 (confirmed live).
                    // Check BEFORE discarding.
                    if let Expr::Break(ref name) = *fallback {
                        check_break(name, cx);
                    }
                    *args = builtin_fallback_into_args(*fallback);
                    cloned_from.map(|from| {
                        let to = args.iter().flat_map(def_body_addrs).collect();
                        translate_reachable(&from, to, reachable)
                    })
                });
                let reachable = rebased.as_ref().unwrap_or(reachable);
                // #3391: the callee is a `def` (or a parameter, which takes no
                // arguments), so each argument is a closure -- a compile unit
                // of its own.
                for a in args.iter_mut() {
                    check_in_block(a, cx, reachable);
                }
            } else if let Some(fallback) = builtin_fallback.take() {
                // Not shadowed after all -- restore the original parse in
                // one move, no cloning. Deliberately does *not* count towards
                // `occurrences` here: this re-dispatches on the exact same
                // source position (now a different `Expr` variant, typically
                // `Expr::Builtin`), not a second, distinct call site -- the
                // recursive `check` call below counts it exactly once, in
                // whichever arm the swapped-in `expr` actually matches.
                *expr = *fallback;
                check(expr, cx, reachable);
            } else if is_jq_builtin(name, arity) {
                next_call_occurrence(&mut cx.occurrences, name, arity); // patchcov: coverage tolerate-line reason="unreachable in practice today: this arm needs builtin_fallback==None (the name was never a shadow candidate) yet is_jq_builtin==true (a real jq builtin at this arity) -- every implemented builtin's own dedicated parse already lowers that shape to Expr::Builtin before resolve.rs ever runs, and #3042/#3046 closed the once-real 'unimplemented builtin' gap this existed for (see JQ_BUILTIN_ROSTER's own doc comment)"
                for a in args.iter_mut() {
                    check(a, cx, reachable); // patchcov: coverage tolerate-line reason="unreachable with the current roster: every `JQ_BUILTIN_ROSTER` entry of arity >= 1 already has a dedicated parser form (a `matches_keyword` special case or a `Libm1`/`Libm2`/`Libm3::ALL` entry -- confirmed by cross-referencing the full roster against both), so it is parsed straight to `Expr::Builtin` and never reaches here as a bare `FuncCall`. This arm exists for a roster name with no dedicated parse yet and a nonzero arity -- there is none today, so the loop body is reached with an empty `args` on every pinned-suite run (355 hits on the arm's own condition, 0 in the loop) and would only start executing if such a name were added (#2964)"
                }
            } else {
                let occurrence_index = next_call_occurrence(&mut cx.occurrences, name, arity);
                cx.push_error(ResolveError::Call(UnresolvedCall {
                    name: name.clone(),
                    arity,
                    origin,
                    occurrence_index,
                    module_def: cx.occurrences.module_def(),
                }));
            }
        }

        // `NamespacedCall` survives only when this pass runs without the jq
        // runner's `rewrite_namespaced_calls` (the yq runner has no module
        // system). Its arguments still need checking; the call itself is left
        // to `eval`'s own "module not loaded" reporting rather than claimed
        // undefined here.
        Expr::NamespacedCall { args, .. } => {
            for a in args.iter_mut() {
                check_in_block(a, cx, reachable);
            }
        }

        Expr::Object(entries) => {
            for entry in entries.iter_mut() {
                if let ObjectKey::Expr(k) = &mut entry.key {
                    check(k, cx, reachable);
                }
                check(&mut entry.value, cx, reachable);
            }
        }

        // #3583: jq folds the parts into a chain of `+`, so the units under
        // `"\(map(ua))\(map(ub))"` come out `ub`, `ua` -- the same right to
        // left order as any other `+`.
        Expr::StringInterpolation(parts) => {
            let mut marks = Vec::new();
            for part in parts.iter_mut() {
                if let StringPart::Expr(e) = part {
                    if marks.is_empty() {
                        marks.push(cx.mark());
                    }
                    check(e, cx, reachable);
                    marks.push(cx.mark());
                }
            }
            cx.reorder_reversed(&marks);
        }

        // No builtin introduces a function or variable binding, so its
        // sub-expressions inherit the current scope unchanged. Rebuilt via
        // `map_builtin_subexprs` rather than an in-place mutable walker: a
        // builtin's own operand can itself be a `FuncCall` this same #2036
        // rewrite needs to reach (`map(length)` where `length` is
        // shadowed, say), so each sub-expression is cloned, checked (which
        // may substitute it), and used to rebuild the builtin -- the same
        // clone-and-rebuild shape `jq_runner.rs`'s `rewrite_namespaced_calls`
        // already uses for its own builtin arm, reusing this function's
        // existing, tested infrastructure rather than hand-writing a
        // second, mutable 207-arm match beside `builtin_kids`.
        //
        // #2971: that clone is also what moved every `def` body inside the
        // operand to a fresh heap address, and `reachable` is keyed by the
        // addresses `build_call_graph` read off the *original* tree -- so
        // `check`'s `FuncDef` gate found none of them and skipped each body
        // as if it were never called. `[1]|map(def g: nosuchfn; g)` compiled,
        // and failed at runtime (exit 5) where jq refuses to compile it
        // (exit 3). The gate is right; only its key went stale, so the key is
        // carried across the clone rather than the gate being weakened.
        //
        // #3391: whether an operand is a compile unit of its own depends on
        // how jq itself builds the builtin -- see
        // `builtin_operands_are_inline`.
        Expr::Builtin(builtin) => {
            let inline = builtin_operands_are_inline(builtin);
            // #3583: the operands of a builtin implemented in C are compiled
            // right to left (`pow(map(ua); map(ub))` is `ub`, `ua`); a jq
            // `def`'s closure arguments are compiled in the order written, so
            // only an inline builtin records where each operand ends.
            let mut marks = Vec::new();
            if inline {
                marks.push(cx.mark());
            }
            *builtin = map_builtin_subexprs(builtin, &mut |sub| {
                let mut copy = sub.clone();
                let rebased = rebase_reachable(sub, &copy, reachable);
                if inline {
                    check(&mut copy, cx, &rebased);
                    marks.push(cx.mark());
                } else {
                    check_in_block(&mut copy, cx, &rebased);
                }
                copy
            });
            cx.reorder_reversed(&marks);
        }
    }
}

/// Whether `builtin`'s operands are evaluated inline in the block that calls
/// it, rather than being closures -- compile units of their own (#3391, see
/// [`Blocks`]).
///
/// jq builds a builtin one of two ways. The ones implemented in C (`has`,
/// `ltrimstr`, `getpath`, `setpath`, `strftime`, `pow` and the other libm
/// functions, ...) take their arguments as inline sub-expressions, so an
/// unresolved name in one is an error of the calling block itself. Everything
/// else -- `map`, `select`, `path`, `first`, `test`, `sub`, `limit`, ... -- is
/// a jq `def` whose arguments are closures, and an error in one is only
/// reported when the calling block has none of its own.
///
/// Captured argument by argument against the pinned jq, for every roster
/// entry of arity 1 or more; no builtin mixes the two. This lists the inline
/// ones, so a variant added later is a closure until it is named here -- the
/// side that never hides a sibling's error -- and
/// `builtin_operand_kinds_match_the_pinned_capture` fails if the roster and
/// this list disagree.
///
/// The same split decides the *order* of a builtin's operands (#3583): jq
/// compiles a C-implemented builtin's operands last to first and a `def`'s
/// closure arguments in the order written, and `check` reverses the former on
/// the strength of this list. The two properties coincide for the whole
/// roster (`closure_order_follows_the_operand_kind_across_the_roster`).
fn builtin_operands_are_inline(builtin: &Builtin) -> bool {
    matches!(
        builtin,
        Builtin::Has(_)
            | Builtin::Contains(_)
            | Builtin::Ltrimstr(_)
            | Builtin::Rtrimstr(_)
            | Builtin::Startswith(_)
            | Builtin::Endswith(_)
            | Builtin::Split(_)
            | Builtin::GetPath(_)
            | Builtin::DelPaths(_)
            | Builtin::SetPath(..)
            | Builtin::HaltErrorCode(_)
            | Builtin::FormatNamed(_)
            | Builtin::Strftime(_)
            | Builtin::Strflocaltime(_)
            | Builtin::Strptime(_)
            | Builtin::Pow(..)
            | Builtin::Atan2(..)
            | Builtin::Libm2(..)
            | Builtin::Libm3(..)
    )
}

/// `reachable`, re-keyed onto `copy` -- a fresh [`Clone`] of `original`
/// (#2971).
///
/// [`build_call_graph`] identifies a `def` by its body's heap address, and a
/// clone reallocates every `Box` it copies, so none of `original`'s keys
/// name anything in `copy`. `original` and `copy` are structurally identical,
/// so one deterministic walk ([`def_body_addrs`]) visits their defs in the
/// same order, and zipping the two lists pairs each body with its copy.
///
/// **The caller must use the result, never fall back to `reachable`.** That
/// is what makes this sound. `reachable` still holds the addresses of every
/// tree this pass has already replaced and freed, and the allocator hands
/// those addresses out again -- to exactly the copies this function is
/// called for. Consulting it after a clone let an *uncalled* def land on a
/// freed address that had belonged to a *called* one, and be rejected:
/// `def select: 1; [1]|map(def g: 1; g), [1]|map(select(def k: nosuchfn; 1))`
/// failed to compile where jq runs it. The set returned here holds only
/// addresses inside `copy`, each put there because its own original was
/// reachable, so a reused address cannot match by coincidence.
///
/// The same property makes any gap in the walk safe: a `def` it fails to
/// pair is simply absent, so it is treated as unreachable -- the pre-#2971
/// behaviour for it -- rather than wrongly checked. Completeness only decides
/// how many answers this fixes, never whether it can break one.
///
/// Empty (and unallocated) when `original` holds no `def`, the common case.
/// Resolution runs once per program, not per input, so the walk scales with
/// program size.
fn rebase_reachable(original: &Expr, copy: &Expr, reachable: &BTreeSet<usize>) -> BTreeSet<usize> {
    let from = def_body_addrs(original);
    if from.is_empty() {
        return BTreeSet::new();
    }
    translate_reachable(&from, def_body_addrs(copy), reachable)
}

/// The copies (`to`) of whichever `from` addresses are in `reachable`,
/// pairing the two lists by position -- see [`rebase_reachable`].
fn translate_reachable(
    from: &[usize],
    to: Vec<usize>,
    reachable: &BTreeSet<usize>,
) -> BTreeSet<usize> {
    debug_assert_eq!(
        from.len(),
        to.len(),
        "a clone must hold exactly its source's defs, in the same order"
    );
    from.iter()
        .zip(to)
        .filter(|(old, _)| reachable.contains(old))
        .map(|(_, new)| new)
        .collect()
}

/// Every `def` body's address in `expr`, in a deterministic pre-order -- the
/// identity [`build_call_graph`] keys reachability on.
///
/// Built on [`any_subexpr`], plus the one place it does not look but
/// [`check`] does: a call's `builtin_fallback` (the alternate parse a
/// shadowed builtin name carries, which is where `select(def g: ..; g)` puts
/// its def once `select` is also user-defined). A destructuring pattern's
/// computed object keys (#2734) no longer need a separate walk here --
/// `any_subexpr` descends `patterns` itself (#3017), so a `def` inside a key
/// is reached the same way as everywhere else. It must not be *also* reached
/// this way: a second, independent descent into `patterns` would push its
/// body address twice, and `translate_reachable`'s positional zip against a
/// cloned tree's own addresses requires exactly one per `def`.
fn def_body_addrs(expr: &Expr) -> Vec<usize> {
    let mut addrs = Vec::new();
    collect_def_body_addrs(expr, &mut addrs);
    addrs
}

fn collect_def_body_addrs(expr: &Expr, addrs: &mut Vec<usize>) {
    any_subexpr(expr, &mut |node| {
        match node {
            Expr::FuncDef { body, .. } => addrs.push(body.as_ref() as *const Expr as usize),
            Expr::FuncCall {
                builtin_fallback: Some(fallback),
                ..
            } => collect_def_body_addrs(fallback, addrs),
            _ => {}
        }
        false
    });
}

/// [`check`] over an optional sub-expression.
/// The operands of an arithmetic or comparison operator, checked in order
/// and reported in jq's reversed one (#3583).
fn check_operator_operands(
    left: &mut Expr,
    right: &mut Expr,
    cx: &mut CheckCtx,
    reachable: &BTreeSet<usize>,
) {
    let start = cx.mark();
    check(left, cx, reachable);
    let middle = cx.mark();
    check(right, cx, reachable);
    let end = cx.mark();
    cx.reorder(&[start, middle, end], Some(&[1, 0]), None);
}

fn check_opt(expr: Option<&mut Expr>, cx: &mut CheckCtx, reachable: &BTreeSet<usize>) {
    if let Some(e) = expr {
        check(e, cx, reachable);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jq::parse;
    use alloc::format;

    /// Resolve a filter, returning the failing `name/arity` if any. Every
    /// expectation in this module was captured from the pinned oracle
    /// (`/usr/bin/jq`, jq-1.7.1), never from succinctly's own output.
    fn resolve(filter: &str) -> Result<(), String> {
        let mut expr = parse(filter).expect("filter must parse");
        resolve_func_calls(&mut expr).map_err(|e| format!("{e}"))
    }

    /// The compiled-in roster must match what `./scripts/sync-jq-builtin-names.sh`
    /// captured from the pinned jq (1.7.1), entry for entry and in order.
    ///
    /// A stale table silently reintroduces the one false-positive class this
    /// pass has: a jq builtin succinctly does not implement, mentioned
    /// somewhere evaluation never reaches, would be rejected at compile time
    /// where real jq compiles it. Compared against the checked-in capture
    /// rather than a live `jq` -- CI runners have none, and the file *is* the
    /// pin's output.
    #[test]
    fn jq_builtin_roster_matches_the_pinned_capture() {
        let captured: Vec<(&str, usize)> = include_str!("../../tests/data/jq-builtin-names.txt")
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .map(|l| {
                let (name, arity) = l.rsplit_once('/').expect("malformed roster entry");
                (name, arity.parse().expect("malformed roster arity"))
            })
            .collect();

        assert_eq!(
            JQ_BUILTIN_ROSTER.len(),
            captured.len(),
            "roster size drifted from the capture -- rerun \
             ./scripts/sync-jq-builtin-names.sh and update JQ_BUILTIN_ROSTER"
        );
        for (compiled, captured) in JQ_BUILTIN_ROSTER.iter().zip(captured.iter()) {
            assert_eq!(compiled, captured, "roster entry drifted from the capture");
        }
    }

    #[test]
    fn accepts_a_call_to_a_preceding_def() {
        assert_eq!(resolve("def f: 1; def g: f; g"), Ok(()));
    }

    #[test]
    fn rejects_a_forward_reference_across_names() {
        // jq: `g/0 is not defined`. succinctly computed `42` before #1473.
        assert_eq!(
            resolve("def f: g; def g: 42; f"),
            Err("g/0 is not defined".into())
        );
    }

    #[test]
    fn rejects_a_forward_reference_to_a_later_arity_of_itself() {
        // jq: `f/2 is not defined`. succinctly computed `100` before #1473 --
        // the silently-wrong case #1376's review found.
        assert_eq!(
            resolve("def f(x): f(x; 99); def f(x; y): x + y; f(1)"),
            Err("f/2 is not defined".into())
        );
    }

    #[test]
    fn accepts_self_recursion() {
        assert_eq!(
            resolve("def f(n): if n == 0 then 0 else f(n-1) end; f(3)"),
            Ok(())
        );
    }

    #[test]
    fn accepts_arity_overloading_in_both_directions() {
        // #1376's own repro, plus the earlier arity called from inside the
        // later one's body -- neither is a forward reference.
        assert_eq!(
            resolve("def f(x): x + 1; def f(x; y): x + y; [f(1), f(2;3)]"),
            Ok(())
        );
        assert_eq!(
            resolve("def f(x): x+1; def f(x;y): (f(x)) + y; f(2;3)"),
            Ok(())
        );
    }

    #[test]
    fn rejects_an_arity_no_def_provides() {
        assert_eq!(
            resolve("def f(x): x+1; def f(x;y): x+y; f(1;2;3)"),
            Err("f/3 is not defined".into())
        );
    }

    #[test]
    fn rejects_an_unreached_branch_and_a_caught_call() {
        // The two laziness symptoms: jq rejects both unconditionally.
        assert_eq!(
            resolve("def f(x): x; if false then f(1;2;3) else 1 end"),
            Err("f/3 is not defined".into())
        );
        assert_eq!(
            resolve("def f(x): x; try f(1;2) catch \"caught\""),
            Err("f/2 is not defined".into())
        );
    }

    /// #2740: a `def` whose call never appears anywhere in the program is
    /// never checked -- jq's own substitution-based compiler never compiles
    /// such a body either, since it only ever compiles a `def` at the call
    /// site referencing it. Distinct from the *unreached-branch* case above:
    /// `if false then f(1;2;3) ...` still contains a real, textual call to
    /// `f` (jq rejects it regardless of which branch runs at evaluation
    /// time) -- this test's `h` has no call to it anywhere at all.
    #[test]
    fn accepts_a_def_whose_body_is_never_referenced_anywhere() {
        assert_eq!(resolve("def h: nosuchfn; 1"), Ok(()));
        // A nested, locally-scoped def that is likewise never called.
        assert_eq!(resolve("def h: def g: nosuchfn; 1; 1"), Ok(()));
        // Two mutually unreferenced defs.
        assert_eq!(resolve("def a: nosuchfn; def b: nosuchfn2; 1"), Ok(()));
    }

    /// #2740: a call reached only through a genuine (non-shadowed)
    /// `Expr::Range`'s own `to` bound still propagates reachability --
    /// `range(1; b)` parses straight to `Expr::Range`, not a `FuncCall`,
    /// since no `def range` exists anywhere in this program to trigger
    /// #2036's shadowable-call wrapping.
    #[test]
    fn a_call_reached_through_a_range_bound_still_raises() {
        assert_eq!(
            resolve("def b: nosuchfn; def a: range(1; b); a"),
            Err("nosuchfn/0 is not defined".into())
        );
    }

    /// #2740: a def that IS referenced still gets its body checked, even
    /// when the only reference is as a bare *argument* to another call
    /// that never actually invokes its parameter -- confirmed live against
    /// jq 1.7.1: the argument expression itself must still resolve/compile
    /// at the call site, independent of whether the callee's body ever
    /// uses the parameter it's bound to.
    #[test]
    fn rejects_a_def_referenced_only_as_an_unused_argument() {
        assert_eq!(
            resolve("def h: nosuchfn; def use(f): 1; use(h)"),
            Err("nosuchfn/0 is not defined".into())
        );
    }

    /// #2740 review: `build_call_graph`'s own arguments-of-an-unresolved-call
    /// handling must mirror `check`'s (#2037's rule: real jq's compiler
    /// never compiles an unresolved call's arguments, having no resolved
    /// callee to bind them to). An early version of this pass didn't
    /// distinguish that case from the `is_jq_builtin` one and walked `args`
    /// unconditionally, marking `h` -- and transitively its own
    /// `nosuchfn2` -- reachable through a call site jq itself never
    /// compiles. Oracle-verified: jq 1.7.1 reports only `nosuchfn/1`.
    #[test]
    fn an_unresolvable_callees_arguments_do_not_mark_anything_reachable() {
        assert_eq!(
            resolve("def h: nosuchfn2; nosuchfn(h)"),
            Err("nosuchfn/1 is not defined".into())
        );
    }

    /// #2740: mutual recursion between two otherwise-unreferenced defs stays
    /// unreachable as a pair -- reachability is computed over the whole
    /// call graph, not a single def in isolation, so a cycle with no
    /// incoming edge from outside it is still dead.
    #[test]
    fn a_mutually_recursive_pair_stays_unreachable_together() {
        assert_eq!(resolve("def a: b; def b: nosuchfn; 1"), Ok(()));
    }

    /// #2740: reachability must not weaken the *existing* checks -- a call
    /// that genuinely reaches an unresolvable name still raises, even when
    /// discovered only via a chain of several defs.
    #[test]
    fn a_call_reached_through_several_defs_still_raises() {
        // Dependency order matters here for a reason unrelated to
        // reachability: `def a: b; ...` would be a forward reference (#1473
        // rejects it before this pass's own reachability question is even
        // reached), so each def below only calls one already defined.
        assert_eq!(
            resolve("def c: nosuchfn; def b: c; def a: b; a"),
            Err("nosuchfn/0 is not defined".into())
        );
    }

    /// #2971: a `def` inside a builtin's argument is compile-checked when it
    /// is called. `check` clones each builtin operand before walking it,
    /// which reallocated the def's body away from the address
    /// `build_call_graph` recorded as reachable -- so the body was skipped
    /// and the unresolved call surfaced at runtime instead. One row per
    /// builtin family the issue confirmed, all oracle-verified against jq
    /// 1.7.1 (`nosuchfn/0 is not defined`, exit 3).
    #[test]
    fn a_called_def_inside_a_builtin_argument_is_checked() {
        for filter in [
            "[1]|map(def g: nosuchfn; g)",
            "[1]|select(def g: nosuchfn; g)",
            "[1]|with_entries(def g: nosuchfn; g)",
            "[1]|any(def g: nosuchfn; g)",
            "[1]|all(def g: nosuchfn; g)",
        ] {
            assert_eq!(
                resolve(filter),
                Err("nosuchfn/0 is not defined".into()),
                "{filter}"
            );
        }
    }

    /// #2971's negative control, and the reason the fix re-keys the
    /// reachability gate instead of dropping it: dropping it passes every
    /// row above and fails every row here. An *uncalled* def is never
    /// compiled by jq, wherever it sits (jq 1.7.1: exit 0 for each).
    #[test]
    fn an_uncalled_def_inside_a_builtin_argument_is_still_skipped() {
        for filter in [
            "[1]|map(def g: nosuchfn; 1)",
            "[1]|select(def g: nosuchfn; 1)",
            "[1]|with_entries(def g: nosuchfn; 1)",
            "[1]|any(def g: nosuchfn; 1)",
            "[1]|all(def g: nosuchfn; 1)",
        ] {
            assert_eq!(resolve(filter), Ok(()), "{filter}");
        }
    }

    /// #2971: the rebase has to compose, and has to carry across only what
    /// was reachable -- not every def it pairs. Each nested builtin clones
    /// again, so a def two builtins deep is re-keyed twice; and a def whose
    /// only caller is itself uncalled must stay skipped even though the
    /// rebase walks straight past it. All oracle-verified against jq 1.7.1.
    #[test]
    fn a_def_inside_a_builtin_argument_keeps_whole_program_reachability() {
        // Two builtins deep: re-keyed at each level.
        assert_eq!(
            resolve("[[1]]|map(map(def g: nosuchfn; g))"),
            Err("nosuchfn/0 is not defined".into())
        );
        assert_eq!(resolve("[[1]]|map(map(def g: nosuchfn; 1))"), Ok(()));
        // Reached only transitively, through another def in the same operand.
        assert_eq!(
            resolve("[1]|map(def g: nosuchfn; def h: g; h)"),
            Err("nosuchfn/0 is not defined".into())
        );
        assert_eq!(resolve("[1]|map(def g: nosuchfn; def h: g; 1)"), Ok(()));
        // The operand sits inside a def: reachable only if that def is.
        assert_eq!(resolve("def h: map(def g: nosuchfn; g); 1"), Ok(()));
        assert_eq!(
            resolve("def h: map(def g: nosuchfn; g); [1]|h"),
            Err("nosuchfn/0 is not defined".into())
        );
    }

    /// #2971's second site. `check`'s `Expr::Shared` arm reaches its
    /// subtree through `Rc::make_mut`, which clones when the `Rc` has another
    /// owner -- the same reallocation the builtin arm's clone causes, so a
    /// called def inside it was skipped the same way. This was the
    /// "multi-owner `Expr::Shared`" caveat documented on
    /// `resolve_func_calls`; it needs an external caller holding a second
    /// handle, which is what `_keep` is. The uniquely-owned spelling is the
    /// control: `make_mut` does not clone there, so it always worked.
    #[test]
    fn a_called_def_inside_a_multi_owner_shared_is_checked() {
        let program = || parse("def g: nosuchfn; g").expect("filter must parse");

        let mut unique = Expr::shared(program());
        assert_eq!(
            resolve_func_calls(&mut unique).map_err(|e| format!("{e}")),
            Err("nosuchfn/0 is not defined".into()),
            "uniquely owned: make_mut does not clone"
        );

        let inner = Rc::new(super::super::SharedArg::new(program()));
        let _keep = Rc::clone(&inner);
        let mut shared = Expr::Shared(inner);
        assert_eq!(
            resolve_func_calls(&mut shared).map_err(|e| format!("{e}")),
            Err("nosuchfn/0 is not defined".into()),
            "multi-owner: make_mut clones, which must not strand the def"
        );

        // And the gate still holds there: an uncalled def stays skipped.
        let inner = Rc::new(super::super::SharedArg::new(
            parse("def g: nosuchfn; 1").expect("filter must parse"),
        ));
        let _keep = Rc::clone(&inner);
        let mut shared = Expr::Shared(inner);
        assert_eq!(
            resolve_func_calls(&mut shared).map_err(|e| format!("{e}")),
            Ok(())
        );
    }

    /// #2971 review: the other two clone sites, reached only when a builtin's
    /// name is also user-defined. `select(def g: ..; g)` then parses to a
    /// call whose `builtin_fallback` holds the def -- a field the pairing
    /// walk first missed -- and unpacking that fallback clones the def again.
    /// All oracle-verified against jq 1.7.1.
    #[test]
    fn a_def_inside_a_shadowed_builtins_argument_is_checked() {
        for filter in [
            "def select: 1; [1]|map(select(def g: nosuchfn; g))",
            "def first: 1; [1]|map(first(def g: nosuchfn; g))",
            "def map(f): f; [1] | map(def g: nosuchfn; g)",
        ] {
            assert_eq!(
                resolve(filter),
                Err("nosuchfn/0 is not defined".into()),
                "{filter}"
            );
        }
        assert_eq!(
            resolve("def map(f): f; [1] | map(def g: nosuchfn; 1)"),
            Ok(())
        );
    }

    /// #2971 review: the regression the first version of this fix shipped.
    /// Once the first `map` has been resolved and its operand freed, the
    /// address of `g`'s body is free for reuse but still in the reachable
    /// set -- and `k`'s copy can be allocated exactly there. Consulting the
    /// enclosing set after a clone then called `k` reachable and rejected a
    /// program jq runs (exit 0).
    ///
    /// **This test does not catch that regression.** Whether `k`'s copy lands
    /// on the freed address depends on heap state, and inside this harness it
    /// does not: reintroducing the fallback leaves this test passing. It pins
    /// the correct answer for the shape; the guard is
    /// `test_uncalled_def_inside_builtin_argument_still_compiles_2971`
    /// (`jq_cli_tests.rs`), whose CLI binary does reproduce the reuse and
    /// fails under that mutation -- itself allocator-dependent, which is why
    /// the invariant is also written down on `resolve_func_calls` and at
    /// each clone site rather than left to a test to enforce.
    #[test]
    fn an_uncalled_def_is_not_mistaken_for_a_freed_called_one() {
        assert_eq!(
            resolve("def select: 1; [1]|map(def g: 1; g), [1]|map(select(def k: nosuchfn; 1))"),
            Ok(())
        );
    }

    /// #2971 review: a def written inside a destructuring pattern's computed
    /// key. `check` compile-checks those keys (#2734), but `build_call_graph`
    /// never walked them, so every such def was unreachable and skipped. Each
    /// pattern-carrying form -- `as`, `reduce`, `foreach` -- and an object
    /// pattern nested in an array one. All oracle-verified against jq 1.7.1;
    /// the last row is the uncalled control.
    #[test]
    fn a_def_inside_a_pattern_computed_key_is_checked() {
        for filter in [
            ". as {(def g: nosuchfn; g): $x} | $x",
            "[1]|map(. as {(def g: nosuchfn; g): $x} | $x)",
            "reduce ([{}]|.[]) as {(def g: nosuchfn; g): $x} (0; .)",
            "foreach ([{}]|.[]) as {(def g: nosuchfn; g): $x} (0; .)",
            ". as [{(def g: nosuchfn; g): $x}] | $x",
        ] {
            assert_eq!(
                resolve(filter),
                Err("nosuchfn/0 is not defined".into()),
                "{filter}"
            );
        }
        assert_eq!(resolve(". as {(def g: nosuchfn; \"a\"): $x} | $x"), Ok(()));
    }

    /// #3017: `collect_def_body_addrs` used to carry its own local walk into
    /// a pattern's computed keys (`collect_pattern_def_body_addrs`, added by
    /// #3014 for #2971's rebase specifically because `any_subexpr` didn't
    /// look there). Now that `any_subexpr` itself descends `patterns`
    /// (#3017), that local walk is gone -- if it had been left in place, a
    /// `def` inside a computed key would be pushed twice: once by
    /// `any_subexpr`'s own new descent hitting the `Expr::FuncDef` node,
    /// once by the (now-removed) explicit pattern walk.
    #[test]
    fn def_body_addrs_visits_a_pattern_key_def_exactly_once_3017() {
        for filter in [
            ". as {((def g: 1; g)): $x} | $x",
            "reduce .a as {((def g: 1; g)): $x} (.b; .c)",
            "foreach .a as {((def g: 1; g)): $x} (.b; .c; .d)",
        ] {
            let expr = parse(filter).expect("filter must parse");
            let addrs = def_body_addrs(&expr);
            assert_eq!(
                addrs.len(),
                1,
                "def body inside a computed key must be visited exactly once for {filter:?}: {addrs:?}"
            );
        }
    }

    /// The same double-push would also break [`rebase_reachable`]'s
    /// positional zip against a cloned tree: `translate_reachable`'s
    /// `debug_assert_eq!(from.len(), to.len())` requires `def_body_addrs`
    /// to visit `original` and `copy` in exactly the same order and count.
    #[test]
    fn rebase_reachable_pairs_a_pattern_key_def_through_a_clone_3017() {
        let expr = parse(". as {((def g: 1; g)): $x} | $x").expect("filter must parse");
        let copy = expr.clone();
        let from = def_body_addrs(&expr);
        assert_eq!(from.len(), 1, "sanity: exactly one def body in the source");
        let reachable: BTreeSet<usize> = from.iter().copied().collect();
        let rebased = rebase_reachable(&expr, &copy, &reachable);
        assert_eq!(rebased.len(), 1);
    }

    #[test]
    fn a_nested_def_does_not_leak_into_the_outer_scope() {
        assert_eq!(
            resolve("def f: def g: 1; g; g"),
            Err("g/0 is not defined".into())
        );
    }

    #[test]
    fn a_parameter_binds_its_bare_name_at_arity_zero_only() {
        assert_eq!(resolve("def f(g): g; f(1)"), Ok(()));
        // jq: `def f($a): a; f(1)` is `1` -- the `$` form binds both.
        assert_eq!(resolve("def f($a): a; f(1)"), Ok(()));
        // ...but only at arity 0.
        assert_eq!(
            resolve("def f(g): g(1); f(.)"),
            Err("g/1 is not defined".into())
        );
    }

    #[test]
    fn a_parameter_is_out_of_scope_outside_its_own_body() {
        assert_eq!(resolve("def f(g): g; g"), Err("g/0 is not defined".into()));
    }

    #[test]
    fn accepts_every_pinned_jq_builtin_when_unreached() {
        // Every roster entry, at its captured arity, compiles when it is
        // never reached -- as it does in real jq. Before #3042/#3046 this
        // was the roster's whole reason to exist (`cbrt` and `JOIN` arrived
        // here as bare, unimplemented calls); now every name is lowered by
        // the parser and the roster backs the spellings the parser declines.
        for &(name, arity) in JQ_BUILTIN_ROSTER {
            let call = if arity == 0 {
                name.to_string()
            } else {
                format!("{name}({})", vec!["."; arity].join("; "))
            };
            assert_eq!(
                resolve(&format!("if false then {call} else 1 end")),
                Ok(()),
                "{call}"
            );
        }
    }

    #[test]
    fn rejects_a_name_neither_defined_nor_a_builtin() {
        assert_eq!(resolve("nosuchfn"), Err("nosuchfn/0 is not defined".into()));
    }

    #[test]
    fn a_call_argument_is_checked_in_the_callers_scope() {
        // The argument `h` is written at the call site, where only `f/1` and
        // nothing else is in scope -- not inside `f`'s body, where `x` is.
        assert_eq!(
            resolve("def f(x): x; f(h)"),
            Err("h/0 is not defined".into())
        );
    }

    #[test]
    fn descends_into_builtin_sub_expressions() {
        // `map(f)` carries its argument inside a `Builtin`, not an
        // `Expr::FuncCall` -- the `builtin_kids` arm is what reaches it.
        assert_eq!(
            resolve("map(nosuchfn)"),
            Err("nosuchfn/0 is not defined".into())
        );
    }

    #[test]
    fn a_later_same_arity_def_shadows_but_the_earlier_one_stays_resolvable() {
        assert_eq!(resolve("def f: 1; def g: f; def f: 2; g"), Ok(()));
    }

    /// #1371: `Shared`/`DefCall` can never actually reach `check` -- this pass
    /// runs once, on the freshly parsed program, strictly before evaluation
    /// ever builds either variant. Named rather than folded into the leaf
    /// group regardless (see the arm's own comment), so exercised directly
    /// here via `check` itself rather than through `resolve_func_calls`'s
    /// parser-only entry point: each arm must actually recurse into its
    /// payload, not just be present and silently report "no unresolved
    /// calls" the way the leaf arms correctly do for real leaves.
    #[test]
    fn check_recurses_through_shared_and_defcall() {
        use alloc::rc::Rc;

        let unresolved = || Expr::FuncCall {
            name: "nosuchfn".into(),
            args: Vec::new(),
            builtin_fallback: None,
        };

        let mut cx = CheckCtx::default();
        check(&mut Expr::shared(unresolved()), &mut cx, &BTreeSet::new());
        assert_eq!(
            cx.errors,
            [ResolveError::Call(UnresolvedCall {
                name: "nosuchfn".into(),
                arity: 0,
                // Not inside any module run: these hand-built trees have no
                // markers, so the floor is 0 and there is nothing to
                // attribute the failure to (#2951).
                origin: None,
                occurrence_index: 0,
                module_def: None,
            })]
        );

        let mut cx = CheckCtx::default();
        check(
            &mut Expr::DefCall {
                def: Rc::new(crate::jq::FuncDefData::new(
                    "f".into(),
                    Vec::new(),
                    Expr::Identity,
                )),
                args: alloc::vec![unresolved()],
                frames: 0,
                bound: crate::jq::BoundBody::default(),
            },
            &mut cx,
            &BTreeSet::new(),
        );
        assert_eq!(
            cx.errors,
            [ResolveError::Call(UnresolvedCall {
                name: "nosuchfn".into(),
                arity: 0,
                // Not inside any module run: these hand-built trees have no
                // markers, so the floor is 0 and there is nothing to
                // attribute the failure to (#2951).
                origin: None,
                occurrence_index: 0,
                module_def: None,
            })]
        );
    }

    /// #1371 (mirroring `check_recurses_through_shared_and_defcall` just
    /// above, same reasoning): `build_call_graph` must also recurse into
    /// `Shared`/`DefCall` payloads, even though neither variant survives to
    /// reach it in practice -- this pass runs once, on the freshly parsed
    /// program, strictly before evaluation ever builds either.
    #[test]
    fn build_call_graph_recurses_through_shared_and_defcall() {
        use alloc::rc::Rc;

        let call_to_f = || Expr::FuncCall {
            name: "f".into(),
            args: Vec::new(),
            builtin_fallback: None,
        };

        let mut scope = ReachScope::default();
        scope.push("f".to_string(), 0, ScopeHit::Def(42));
        let mut graph = BTreeMap::new();
        let mut roots = Vec::new();
        build_call_graph(
            &Expr::shared(call_to_f()),
            &mut scope,
            None,
            &mut graph,
            &mut roots,
        );
        assert_eq!(roots, alloc::vec![42]);

        let mut scope = ReachScope::default();
        scope.push("f".to_string(), 0, ScopeHit::Def(42));
        let mut graph = BTreeMap::new();
        let mut roots = Vec::new();
        build_call_graph(
            &Expr::DefCall {
                def: Rc::new(crate::jq::FuncDefData::new(
                    "f".into(),
                    Vec::new(),
                    Expr::Identity,
                )),
                args: alloc::vec![call_to_f()],
                frames: 0,
                bound: crate::jq::BoundBody::default(),
            },
            &mut scope,
            None,
            &mut graph,
            &mut roots,
        );
        assert_eq!(roots, alloc::vec![42]);
    }

    /// #2687: `break $x` desugars to a resolvable `error/0` call -- a
    /// same-name, same-arity `def error:` in scope at the break's own
    /// position shadows it, same as any other call. Oracle: `jq-1.7.1`
    /// answers `1`, `"S"`, `3` for
    /// `def error: "S"; label $out | (1, break $out, 3)`.
    #[test]
    fn break_with_a_shadowing_arity_0_def_error_resolves_clean() {
        assert_eq!(
            resolve("def error: \"S\"; label $out | (1, break $out, 3)"),
            Ok(())
        );
    }

    /// #3997: a node whose operands the resolve pass rewrites in place does
    /// not keep what a settle walk remembered about the old ones.
    #[test]
    fn resolving_an_arithmetic_node_forgets_its_settle_memo_3997() {
        let mut expr = parse("def f: 1; f + f").unwrap();
        #[rustfmt::skip]
        let Expr::FuncDef { then, .. } = &mut expr else { panic!("expected a def") };
        #[rustfmt::skip]
        let Expr::Arithmetic { settle, .. } = &**then else { panic!("expected arithmetic") };
        settle.set(Some(true), 5);
        assert!(settle.get().is_some());
        resolve_func_calls(&mut expr).unwrap();
        #[rustfmt::skip]
        let Expr::FuncDef { then, .. } = &expr else { unreachable!() };
        #[rustfmt::skip]
        let Expr::Arithmetic { settle, .. } = &**then else { unreachable!() };
        assert!(settle.get().is_none());
    }

    /// #2687: an arity-1 `def error(m):` does not shadow a bare `break $x`
    /// (arity 0) -- real jq's own `is_jq_builtin`-style arity distinction
    /// applies here exactly as it does to a plain `error` call. Confirmed
    /// live: `def error(m): "S1"; label $out | 1, break $out` still answers
    /// `1` in jq 1.7.1.
    #[test]
    fn break_is_not_shadowed_by_a_different_arity_def_error() {
        assert_eq!(
            resolve("def error(m): \"S1\"; label $out | 1, break $out"),
            Ok(())
        );
    }

    /// #2687: with no `def error` anywhere in the program, `break $x` must
    /// parse to the exact same `Expr::Break` node it always has -- not a
    /// `FuncCall` that then falls back at resolve time. This is what keeps
    /// every existing break/label test byte-for-byte unaffected by this
    /// fix's parser change (`Parser::shadowable_defs`'s fast-reject, the
    /// same one every other shadowable special form already relies on).
    #[test]
    fn break_with_no_def_error_anywhere_stays_a_bare_break_node() {
        let mut expr = parse("label $out | break $out").expect("filter must parse");
        assert!(
            resolve_func_calls(&mut expr).is_ok(),
            "must resolve with no unresolved calls"
        );
        assert!(
            crate::jq::walk::any_subexpr(
                &expr,
                &mut |e| matches!(e, Expr::Break(name) if name == "out")
            ),
            "expected a bare Expr::Break(\"out\") node, found: {expr:?}"
        );
        assert!(
            !crate::jq::walk::any_subexpr(&expr, &mut |e| matches!(
                e,
                Expr::FuncCall { name, .. } if name == "error"
            )),
            "must not have been wrapped as a FuncCall when nothing shadows it: {expr:?}"
        );
    }

    /// #2687: the flip side of the above -- once a shadowing `def error:` is
    /// genuinely in scope, the break site must resolve to a real call
    /// (`DefCall`, after `eval::install_def_calls` runs) rather than staying
    /// an `Expr::Break`. `resolve_func_calls` alone leaves a shadowed site as
    /// an ordinary zero-arg `Expr::FuncCall { name: "error", .. }` -- the
    /// `DefCall` rewrite is a separate, later eval-time pass -- so this
    /// checks that intermediate shape directly.
    #[test]
    fn break_with_a_shadowing_def_error_becomes_an_error_call_node() {
        let mut expr =
            parse("def error: \"S\"; label $out | break $out").expect("filter must parse");
        assert!(resolve_func_calls(&mut expr).is_ok());
        assert!(
            crate::jq::walk::any_subexpr(&expr, &mut |e| matches!(
                e,
                Expr::FuncCall { name, args, .. } if name == "error" && args.is_empty()
            )),
            "expected a resolved zero-arg `error` FuncCall node, found: {expr:?}"
        );
        assert!(
            !crate::jq::walk::any_subexpr(&expr, &mut |e| matches!(e, Expr::Break(_))),
            "the original Expr::Break must not survive once shadowed: {expr:?}"
        );
    }

    /// #2734: unbound `$variable` resolution. Every expectation captured
    /// live against the pinned oracle (`/usr/bin/jq`, jq-1.7.1), same
    /// discipline as [`resolve`]'s own doc comment.
    /// #2951's floor, exercised directly on hand-built scope stacks.
    ///
    /// The CLI tests drive the same rule through real files, which is what
    /// proves the loader and the resolver agree; these pin the rule itself,
    /// where a failure says which half is wrong instead of just "the program
    /// compiled when it should not have".
    mod module_scope_floor {
        use super::*;

        /// Build a `Scope` from a compact spelling: `"begin:1"` / `"end:1"` /
        /// `"begin:1@a"` are markers, `"link:1"` opens module 1's link run
        /// (a begin marker carrying its link alias, #2955), `"1::g"` is the
        /// def `g` as emitted inside that run, and anything else is a def of
        /// that name at arity 0.
        fn scope_of(entries: &[&str]) -> Scope {
            let mut scope = Scope::default();
            for e in entries {
                let name = if let Some(rest) = e.strip_prefix("begin:") {
                    match rest.split_once('@') {
                        Some((id, alias)) => {
                            ModuleRun::begin_marker(id.parse().expect("id"), Some(alias))
                        }
                        None => ModuleRun::begin_marker(rest.parse().expect("id"), None),
                    }
                } else if let Some(id) = e.strip_prefix("end:") {
                    ModuleRun::end_marker(id.parse().expect("id"))
                } else if let Some(id) = e.strip_prefix("link:") {
                    let id = id.parse().expect("id");
                    ModuleRun::begin_marker(id, Some(&ModuleRun::link_alias(id)))
                } else if let Some((id, name)) = e.split_once("::") {
                    ModuleRun::link_name(id.parse().expect("id"), name)
                } else {
                    (*e).to_string()
                };
                scope.push(name, 0, ());
            }
            scope
        }

        fn visible(entries: &[&str], name: &str) -> bool {
            in_scope(&scope_of(entries), name, 0).hit.is_some()
        }

        /// #3455: [`FnScope::lookup`] answers what the [`scan_scope`] it
        /// replaced answers -- the same entry, and the same open run on a miss
        /// -- over random histories of pushes (defs at three arities, begin and
        /// end markers, link names) and truncations, for every name and arity,
        /// with and without crossing floors. Unbalanced markers are included:
        /// the stack a lookup sees is not always well bracketed. Three seeds,
        /// over a deliberately small alphabet so shadowing, floors and
        /// truncation meet constantly.
        #[test]
        fn fn_scope_matches_scan_scope_3455() {
            for seed in [
                0x9E37_79B9_7F4A_7C15_u64,
                0xD1B5_4A32_D192_ED03,
                0x2545_F491_4F6C_DD1D,
            ] {
                fn_scope_matches_scan_scope_over(seed);
            }
        }

        fn fn_scope_matches_scan_scope_over(seed: u64) {
            let mut state = seed;
            let mut roll = |n: u64| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state % n
            };
            let names: Vec<String> = ["a", "b", "c", "d"]
                .iter()
                .map(|n| (*n).to_string())
                .chain((0..3).map(|id| ModuleRun::link_name(id, "a")))
                .collect();
            for history in 0..300 {
                let mut scope = FnScope::<usize>::default();
                let mut mirror: Vec<(String, usize, usize)> = Vec::new();
                for step in 0..60 {
                    if roll(8) == 0 {
                        let keep = roll(mirror.len() as u64 + 1) as usize;
                        scope.truncate(keep);
                        mirror.truncate(keep);
                    } else {
                        let id = roll(3) as u32;
                        let name = match roll(8) {
                            0 => ModuleRun::begin_marker(id, None),
                            1 => ModuleRun::begin_marker(id, Some("alias")),
                            2 => ModuleRun::end_marker(id),
                            3 => ModuleRun::link_name(id, "a"),
                            _ => names[roll(4) as usize].clone(),
                        };
                        let arity = roll(3) as usize;
                        scope.push(name.clone(), arity, step);
                        mirror.push((name, arity, step));
                    }
                    assert_eq!(scope.len(), mirror.len());
                    for name in &names {
                        for arity in 0..3 {
                            for crosses in [false, true] {
                                let want = scan_scope(
                                    &mirror,
                                    |(n, _, _)| n.as_str(),
                                    |(n, a, at)| (*a == arity && n == name).then_some(*at),
                                    crosses,
                                );
                                let got = scope.lookup(name, arity, crosses);
                                assert_eq!(
                                    (got.hit, got.run),
                                    (want.hit, want.run),
                                    "history {history} step {step}: {name:?}/{arity} \
                                     (crosses floors: {crosses}) over {mirror:?}"
                                );
                            }
                        }
                    }
                }
            }
        }

        /// #3455: resolving costs scope work linear in the number of defs,
        /// counted in entries examined rather than timed. The filter calls the
        /// *outermost* of `M` defs `M` times, the shape a stack scan pays `M`
        /// entries for each time; doubling `M` must roughly double the
        /// entries examined, where the scan it replaced quadrupled them. Both
        /// passes over the tree (`build_call_graph` and `check`) are counted.
        ///
        /// The spine is built def by def, as the runner builds one, and
        /// resolved on a big stack: `check` recurses once per nested def, and
        /// the thread this runs on gets only 2 MiB.
        #[test]
        fn resolving_many_defs_examines_scope_entries_linearly_3455() {
            let (p500, p1000, p2000) = std::thread::Builder::new()
                .stack_size(1 << 30)
                .spawn(|| {
                    let probes = |m: usize| {
                        let calls = alloc::vec!["g0"; m].join(", ");
                        let mut expr = parse(&format!("[{calls}] | length")).expect("parses");
                        for i in (0..m).rev() {
                            expr = Expr::FuncDef {
                                name: format!("g{i}"),
                                params: Vec::new(),
                                body: Box::new(parse(&i.to_string()).expect("parses")),
                                then: Box::new(expr),
                                bound: crate::jq::FuncDefBound::default(),
                            };
                        }
                        let before = SCOPE_PROBES.with(core::cell::Cell::get);
                        let errors = resolve_all(&mut expr);
                        assert!(errors.is_empty(), "{m} defs resolve cleanly: {errors:?}");
                        SCOPE_PROBES.with(core::cell::Cell::get) - before
                    };
                    (probes(500), probes(1000), probes(2000))
                })
                .expect("spawns")
                .join()
                .expect("resolves");
            assert!(
                p1000 < 3 * p500 && p2000 < 3 * p1000,
                "probes must grow linearly in the number of defs: {p500} / {p1000} / {p2000}"
            );
        }

        /// #3455: truncating restores what the dropped entries changed -- the
        /// floor a begin marker raised or an end marker lifted, and the def a
        /// redefinition had shadowed -- and truncating past the end is a no-op.
        #[test]
        fn fn_scope_truncate_restores_floor_and_shadowed_defs_3455() {
            let mut scope = scope_of(&["outer", "begin:1", "own"]);
            assert!(in_scope(&scope, "outer", 0).hit.is_none());
            scope.truncate(1);
            assert!(in_scope(&scope, "outer", 0).hit.is_some());
            assert!(in_scope(&scope, "own", 0).hit.is_none());

            let mut scope = scope_of(&["outer", "begin:1", "own", "end:1"]);
            assert!(in_scope(&scope, "outer", 0).hit.is_some());
            scope.truncate(3);
            let scan = in_scope(&scope, "outer", 0);
            assert!(scan.hit.is_none());
            assert_eq!(scan.run, Some((1, None)));

            let mut scope = FnScope::<usize>::default();
            scope.push("f".to_string(), 0, 1);
            scope.push("f".to_string(), 0, 2);
            scope.push("f".to_string(), 1, 3);
            assert_eq!(scope.lookup("f", 0, false).hit, Some(2));
            assert_eq!(scope.lookup("f", 1, false).hit, Some(3));
            scope.truncate(2);
            assert_eq!(scope.lookup("f", 0, false).hit, Some(2));
            assert_eq!(scope.lookup("f", 1, false).hit, None);
            scope.truncate(1);
            assert_eq!(scope.lookup("f", 0, false).hit, Some(1));
            scope.truncate(9);
            assert_eq!(scope.len(), 1);
            scope.truncate(0);
            assert_eq!(scope.lookup("f", 0, false).hit, None);
        }

        /// A name below an *open* run's begin marker is invisible -- the
        /// whole point. `outer` is what `~/.jq` or a sibling `include`
        /// contributes; `own` is a def of the module we are inside.
        #[test]
        fn floor_hides_a_def_below_an_open_run() {
            assert!(!visible(&["outer", "begin:1", "own"], "outer"));
            assert!(visible(&["outer", "begin:1", "own"], "own"));
        }

        /// ...but an *earlier sibling in the same run* stays visible, which
        /// is how `def f: 1; def g: f;` inside one module keeps working.
        #[test]
        fn floor_keeps_an_earlier_sibling_of_the_same_run() {
            assert!(visible(&["outer", "begin:1", "f", "g"], "f"));
        }

        /// A *closed* run -- one whose end marker we have already passed
        /// scanning inward -- is wrapped around this point, so its defs are
        /// visible and its begin is not a floor. This is the main filter's
        /// case, and getting it wrong would hide every module from the
        /// program that included it.
        #[test]
        fn a_closed_run_is_visible_and_is_not_a_floor() {
            let chain = ["outer", "begin:1", "own", "end:1"];
            assert!(visible(&chain, "own"), "the run's own defs");
            assert!(visible(&chain, "outer"), "and everything below it");
        }

        /// Nested runs close in order: an inner closed run inside an outer
        /// open one must not cancel the outer one's floor. Counting end
        /// markers rather than matching ids is what makes this work, and it
        /// is the case that would break if the count were a boolean.
        #[test]
        fn a_closed_inner_run_does_not_cancel_an_outer_open_floor() {
            let chain = ["outer", "begin:1", "own", "begin:2", "dep", "end:2"];
            assert!(visible(&chain, "dep"), "the closed inner run");
            assert!(visible(&chain, "own"), "the open outer run's own defs");
            assert!(!visible(&chain, "outer"), "still floored by run 1");
        }

        /// The innermost open run is reported, so the caller can retry a
        /// bare miss under its alias (#2989) and attribute the error to the
        /// right module.
        #[test]
        fn reports_the_innermost_open_run_and_its_alias() {
            let run = in_scope(&scope_of(&["begin:7@ns", "own"]), "nope", 0).run;
            assert_eq!(run, Some((7, Some("ns".to_string()))));

            // Below every end marker there is no open run at all: the main
            // filter sees everything and carries no alias to retry under.
            let run = in_scope(&scope_of(&["begin:7@ns", "own", "end:7"]), "nope", 0).run;
            assert_eq!(run, None);
        }

        /// `run` is only meaningful when the lookup missed: the scan stops at
        /// the match rather than walking the rest of the stack, which is what
        /// keeps a long def chain from going quadratic.
        #[test]
        fn a_hit_short_circuits_and_reports_no_run() {
            let scan = in_scope(&scope_of(&["begin:1@ns", "own"]), "own", 0);
            assert!(scan.hit.is_some());
            assert_eq!(scan.run, None);
        }

        /// Marker names round-trip, and an ordinary def is never mistaken
        /// for one. The NUL prefix is the whole safety argument -- a name a
        /// user could write would let a filter reach across the boundary.
        #[test]
        fn marker_names_round_trip_and_ordinary_names_do_not_parse() {
            assert_eq!(
                ModuleRun::parse(&ModuleRun::begin_marker(3, None)),
                Some(RunMarker::Begin { id: 3, alias: None })
            );
            assert_eq!(
                ModuleRun::parse(&ModuleRun::begin_marker(3, Some("a"))),
                Some(RunMarker::Begin {
                    id: 3,
                    alias: Some("a")
                })
            );
            assert_eq!(
                ModuleRun::parse(&ModuleRun::end_marker(3)),
                Some(RunMarker::End { id: 3 })
            );
            for ordinary in ["run:begin:3", "f", "ns::f", "", "beginning"] {
                assert_eq!(ModuleRun::parse(ordinary), None, "{ordinary:?}");
            }
        }

        /// #2955: a link name is an ordinary def to the scan, its display
        /// name is the one the user wrote -- including a name that itself
        /// contains `::` -- and a begin marker carrying a link alias parses
        /// back to that alias (the `import` retry reads it from there).
        #[test]
        fn link_names_are_ordinary_defs_and_display_as_written() {
            let linked = ModuleRun::link_name(4, "c");
            assert_eq!(ModuleRun::parse(&linked), None);
            assert!(ModuleRun::is_link_name(&linked));
            assert_eq!(ModuleRun::display_name(&linked), "c");
            assert_eq!(
                ModuleRun::display_name(&ModuleRun::link_name(4, "ns::c")),
                "ns::c"
            );
            for ordinary in ["c", "ns::c", ""] {
                assert!(!ModuleRun::is_link_name(ordinary));
                assert_eq!(ModuleRun::display_name(ordinary), ordinary);
            }
            assert_eq!(
                format!("{}::c", ModuleRun::link_alias(4)),
                linked,
                "the retry spells `alias::name`, so the two must compose"
            );
            let alias = ModuleRun::link_alias(4);
            assert_eq!(
                ModuleRun::parse(&ModuleRun::begin_marker(4, Some(&alias))),
                Some(RunMarker::Begin {
                    id: 4,
                    alias: Some(alias.as_str())
                })
            );
        }

        /// #2955: a consumer's forwarding stub sits inside its own module's
        /// run and calls a def in a link run wrapped outside it. That one
        /// lookup crosses the floor; a bare lookup from the same point still
        /// does not, and the link run's defs are unreachable by their bare
        /// names from anywhere.
        #[test]
        fn a_link_name_lookup_crosses_the_floor_and_nothing_else_does() {
            // `link:2 { 2::g } end:2 { begin:1 { h, <stub body here> } }`
            let chain = ["link:2", "2::g", "end:2", "begin:1", "h"];
            let target = ModuleRun::link_name(2, "g");
            assert!(
                visible(&chain, &target),
                "the stub's target, across run 1's floor"
            );
            assert!(!visible(&chain, "g"), "never by its bare name");
            assert!(visible(&chain, "h"));

            // From the main filter, below every end marker: still not `g`.
            let chain = ["link:2", "2::g", "end:2", "begin:1", "h", "end:1"];
            assert!(!visible(&chain, "g"), "a link run re-exports nothing");
            assert!(visible(&chain, "h"));

            // Inside the link run itself, a bare sibling call misses and
            // reports the link alias, which is what the retry needs.
            let scan = in_scope(&scope_of(&["link:2", "2::g", "2::k"]), "g", 0);
            assert!(scan.hit.is_none());
            assert_eq!(scan.run, Some((2, Some(ModuleRun::link_alias(2)))));
            assert!(
                visible(&["link:2", "2::g", "2::k"], &target),
                "...and the retry hits"
            );

            // A dependency of a dependency: the inner link run's stub crosses
            // its own run's floor into the closed outer link run.
            let chain = ["link:3", "3::z", "end:3", "link:2", "2::g"];
            assert!(visible(&chain, &ModuleRun::link_name(3, "z")));
            assert!(!visible(&chain, "z"));
        }
    }

    mod unbound_vars {
        use super::*;

        /// Resolve a filter for *every* diagnostic ([`resolve_all`]), as
        /// `Kind(message)` -- unlike [`resolve`] above, which only ever needs
        /// the first.
        ///
        /// Renders each diagnostic's own `Display` (the text the runners
        /// actually print) behind its kind, rather than `{:?}` of the whole
        /// struct. The `Debug` form names every field, so it turned any
        /// addition to a diagnostic's payload into a failure of a test that
        /// is about *ordering and kind* and nothing else -- #2951's `origin`
        /// did exactly that. What this test pins is that calls and variables
        /// interleave by source position; the payload's shape is not part of
        /// the claim.
        fn resolve_all_strs(filter: &str) -> Vec<String> {
            let mut expr = parse(filter).expect("filter must parse");
            resolve_all(&mut expr)
                .into_iter()
                .map(|e| match e {
                    ResolveError::Call(c) => format!("Call({c})"),
                    ResolveError::Var(v) => format!("Var({v})"),
                    ResolveError::Break(b) => format!("Break({b})"),
                })
                .collect()
        }

        fn resolve_var(filter: &str) -> Result<(), String> {
            let mut expr = parse(filter).expect("filter must parse");
            match resolve_all(&mut expr).into_iter().next() {
                Some(ResolveError::Var(v)) => Err(format!("{v}")),
                Some(ResolveError::Call(c)) => {
                    panic!("expected a variable error, got a call error: {c}")
                }
                Some(ResolveError::Break(b)) => {
                    panic!("expected a variable error, got a break error: {b}")
                }
                None => Ok(()),
            }
        }

        #[test]
        fn rejects_a_bare_unbound_variable() {
            // jq: `$nope is not defined`, exit 3.
            assert_eq!(resolve_var("$nope"), Err("$nope is not defined".into()));
        }

        /// `resolve_var` is a *variable*-shaped helper: its `Break` arm
        /// exists only to fail loudly if a filter under test raises a
        /// break error instead, so a test writer immediately sees which
        /// kind of diagnostic actually fired rather than a confusing
        /// `Ok(())`/`Err` mismatch against the wrong string.
        #[test]
        #[should_panic(expected = "expected a variable error, got a break error")]
        fn resolve_var_panics_on_an_unexpected_break_error() {
            let _ = resolve_var("break $x");
        }

        #[test]
        fn accepts_a_variable_bound_by_as() {
            assert_eq!(resolve_var(".foo as $x | $x"), Ok(()));
        }

        #[test]
        fn as_does_not_bind_its_own_source_expression() {
            // `.foo as $x` evaluates `.foo` before `$x` exists -- a `$x`
            // reference in the source expression itself must stay unbound.
            assert_eq!(
                resolve_var("$x as $y | $x"),
                Err("$x is not defined".into())
            );
        }

        #[test]
        fn rejects_a_variable_only_bound_in_a_sibling_branch() {
            assert_eq!(
                resolve_var(".foo as $x | $x, $nope"),
                Err("$nope is not defined".into())
            );
        }

        #[test]
        fn accepts_object_and_array_destructuring_patterns() {
            assert_eq!(resolve_var(". as {a: $a, b: $b} | $a + $b"), Ok(()));
            assert_eq!(resolve_var(". as [$a, $b] | $a + $b"), Ok(()));
        }

        #[test]
        fn accepts_the_shorthand_bind_in_an_object_pattern() {
            // `{$a}` desugars to `{a: $a}` -- both spellings must bind.
            assert_eq!(resolve_var(". as {$a} | $a"), Ok(()));
        }

        #[test]
        fn accepts_qq_slash_slash_alternatives_binding_the_same_name() {
            assert_eq!(resolve_var(". as $a ?// $b | $a"), Ok(()));
        }

        #[test]
        fn a_computed_pattern_key_is_checked_in_the_outer_scope() {
            // jq: `$nope is not defined` -- the computed key `($nope)` is
            // evaluated before the pattern binds anything, so it never sees
            // its own pattern's `$a`, only whatever was already in scope.
            assert_eq!(
                resolve_var(". as {($nope): $a} | $a"),
                Err("$nope is not defined".into())
            );
        }

        #[test]
        fn accepts_a_pattern_var_referenced_from_a_computed_key() {
            // The outer scope, not the pattern's own bindings, is what a
            // computed key sees -- an *outer* `$x` is fine.
            assert_eq!(
                resolve_var("1 as $x | . as {($x|tostring): $a} | $a"),
                Ok(())
            );
        }

        #[test]
        fn reduce_binds_the_pattern_in_update_only() {
            assert_eq!(resolve_var("reduce .[] as $x (0; . + $x)"), Ok(()));
            // jq: `init` is evaluated before the pattern binds -- `$x` in
            // `init` refers to nothing.
            assert_eq!(
                resolve_var("reduce .[] as $x ($x; . + 1)"),
                Err("$x is not defined".into())
            );
        }

        #[test]
        fn foreach_binds_the_pattern_in_update_and_extract_but_not_init() {
            assert_eq!(resolve_var("foreach .[] as $x (0; . + $x; . + $x)"), Ok(()));
            assert_eq!(
                resolve_var("foreach .[] as $x ($x; . + 1)"),
                Err("$x is not defined".into())
            );
        }

        #[test]
        fn a_dollar_style_def_param_binds_the_variable_in_body_only() {
            assert_eq!(resolve_var("def f($a): $a; f(1)"), Ok(()));
            // A *bare* param binds only the call-site name, never the `$`
            // namespace -- `def f(a): $a; ...` is `$a is not defined` in jq.
            assert_eq!(
                resolve_var("def f(a): $a; f(1)"),
                Err("$a is not defined".into())
            );
        }

        #[test]
        fn a_dollar_style_param_does_not_leak_into_then() {
            assert_eq!(
                resolve_var("def f($a): $a; $a"),
                Err("$a is not defined".into())
            );
        }

        #[test]
        fn a_def_closes_over_an_outer_variable_it_is_lexically_nested_inside() {
            // Confirmed live against jq 1.7.1: unlike function scope
            // (`def`s never see a later sibling), variable scope through a
            // `def` is ordinary lexical capture -- a def *nested inside* a
            // binding sees it, including transitively through a further
            // nested def, and this pass's own `var_scope` needs no special
            // isolation at `FuncDef` boundaries to get this right: it
            // already inherits whatever the ambient stack holds at that
            // position, the same as any other nested expression.
            assert_eq!(resolve_var("1 as $x | def f: $x; f"), Ok(()));
            assert_eq!(resolve_var("1 as $x | def f: def g: $x; g; f"), Ok(()));
            // But a def positioned *before* the binding still doesn't see
            // it -- capture is purely lexical position, not "anywhere in
            // the program".
            assert_eq!(
                resolve_var("def f: $x; 1 as $x | f"),
                Err("$x is not defined".into())
            );
        }

        #[test]
        fn label_and_variable_are_separate_namespaces() {
            // jq: `label $out | ...` never binds a `$out` *variable* -- only
            // `break $out` resolves against the label namespace. Confirmed
            // live: `label $out | $out` is `$out is not defined` in real jq.
            assert_eq!(
                resolve_var("label $out | $out"),
                Err("$out is not defined".into())
            );
        }

        #[test]
        fn loc_and_env_never_reach_this_pass_as_a_var() {
            // `$__loc__`/`$ENV` lower straight to `Expr::Loc`/`Expr::Env` at
            // parse time (`parser.rs`'s `dollar_var_expr`) -- neither should
            // ever be reported as unbound.
            assert_eq!(resolve_var("$__loc__"), Ok(()));
            assert_eq!(resolve_var("$ENV"), Ok(()));
        }

        #[test]
        fn descends_into_builtin_arguments_for_both_as_and_bare_vars() {
            assert_eq!(resolve_var("map(. as $x | $x)"), Ok(()));
            assert_eq!(
                resolve_var("map($nope)"),
                Err("$nope is not defined".into())
            );
            assert_eq!(
                resolve_var("select($nope)"),
                Err("$nope is not defined".into())
            );
        }

        /// `Expr::NamespacedCall` (a bare `ns::name` the parser produces
        /// directly, before `jq_runner.rs`'s own `rewrite_namespaced_calls`
        /// ever sees it -- these tests call [`resolve_all`] straight off
        /// [`parse`], with no rewrite pass in between) still has its own
        /// arguments checked even though the call itself is left to
        /// evaluation's "module not loaded" error, not claimed undefined
        /// here.
        #[test]
        fn descends_into_a_namespaced_calls_arguments() {
            assert_eq!(resolve_var("ns::foo(1)"), Ok(()));
            assert_eq!(
                resolve_var("ns::foo($nope)"),
                Err("$nope is not defined".into())
            );
        }

        #[test]
        fn descends_into_array_and_object_constructors() {
            assert_eq!(resolve_var("[$nope]"), Err("$nope is not defined".into()));
            assert_eq!(
                resolve_var("{a: $nope}"),
                Err("$nope is not defined".into())
            );
            assert_eq!(
                resolve_var("{($nope): 1}"),
                Err("$nope is not defined".into())
            );
        }

        /// #2734: real jq interleaves undefined calls and undefined
        /// variables by source position, not grouped by kind. Confirmed
        /// live: `$bar, foo, $baz` reports all three left to right.
        #[test]
        fn calls_and_variables_interleave_in_source_order() {
            assert_eq!(
                resolve_all_strs("$bar, foo, $baz"),
                [
                    "Var($bar is not defined)",
                    "Call(foo/0 is not defined)",
                    "Var($baz is not defined)",
                ]
            );
        }

        /// #2964: a `break` error interleaves with call/variable errors the
        /// same way -- and exercises `UnresolvedLabel`'s own `Display` (the
        /// `$*label-NAME is not defined` text real runners print), which
        /// `resolve_all_strs`'s `Break` arm is the only caller of.
        #[test]
        fn breaks_interleave_with_calls_and_variables_too() {
            assert_eq!(
                resolve_all_strs("break $x, foo, $bar"),
                [
                    "Break($*label-x is not defined)",
                    "Call(foo/0 is not defined)",
                    "Var($bar is not defined)",
                ]
            );
        }

        /// A program can fail on both fronts at once and jq's compile-error
        /// gate (unconditional, before any input) applies uniformly --
        /// [`resolve_func_calls_all`] (the yq-facing, function-only view)
        /// must still see its own errors when a variable error is also
        /// present in the same program.
        #[test]
        fn resolve_func_calls_all_still_sees_call_errors_alongside_var_errors() {
            let mut expr = parse("$bar, foo").expect("filter must parse");
            assert_eq!(
                resolve_func_calls_all(&mut expr),
                [UnresolvedCall {
                    name: "foo".into(),
                    arity: 0,
                    origin: None,
                    occurrence_index: 0,
                    module_def: None,
                }]
            );
        }

        /// #2635: `occurrence_index` counts *every* earlier visit to
        /// `(name, arity)` -- resolved included -- so the failing `f/0`
        /// (after the pipe) reports index 1, not 0, even though it is the
        /// *first and only* one that ends up in `errors`. Confirmed live
        /// against jq 1.7.1: `f/0 is not defined at <top-level>, line 3:`,
        /// citing the second `f`, not the first (which resolves).
        #[test]
        fn occurrence_index_counts_resolved_calls_too_2635() {
            let mut expr = parse("(def f: 1;\nf)\n| f").expect("filter must parse");
            assert_eq!(
                resolve_func_calls_all(&mut expr),
                [UnresolvedCall {
                    name: "f".into(),
                    arity: 0,
                    origin: None,
                    occurrence_index: 1,
                    module_def: None,
                }]
            );
        }

        /// #3085: a call inside an unreferenced def body is counted but not
        /// reported, so the failing call after it is occurrence 1 -- the
        /// index of its own entry in the source's call-site table.
        #[test]
        fn occurrence_index_counts_calls_in_unreferenced_def_bodies_3085() {
            let mut expr = parse("def u: f; f").expect("filter must parse");
            assert_eq!(
                resolve_func_calls_all(&mut expr),
                [UnresolvedCall {
                    name: "f".into(),
                    arity: 0,
                    origin: None,
                    occurrence_index: 1,
                    module_def: None,
                }]
            );
        }
    }

    /// #2964: the compile-time `break $name` label-scope check.
    ///
    /// Every expectation here was captured from the pinned oracle
    /// (`/usr/bin/jq`, jq-1.7.1), never from succinctly's own output, and
    /// each pair of accept/reject cases is oracle-verified on both sides.
    mod breaks {
        use super::*;

        fn break_errs(filter: &str) -> Vec<(String, usize)> {
            let mut expr = parse(filter).expect("filter must parse");
            resolve_all(&mut expr)
                .into_iter()
                .filter_map(|e| match e {
                    ResolveError::Break(b) => Some((b.name, b.occurrence)),
                    _ => None,
                })
                .collect()
        }

        #[test]
        fn bare_break_outside_any_label_is_rejected() {
            assert_eq!(break_errs("break $x"), alloc::vec![("x".into(), 0)]);
            assert_eq!(break_errs("1, break $x, 2"), alloc::vec![("x".into(), 0)]);
        }

        /// `break_errs`'s `filter_map` discards every non-`Break` diagnostic
        /// -- a program that raises a call or variable error *alongside* an
        /// unresolved break exercises that discard arm, not just the `Break`
        /// one every other case here hits.
        #[test]
        fn a_var_error_alongside_a_break_error_is_filtered_out() {
            assert_eq!(break_errs("break $x, $nope"), alloc::vec![("x".into(), 0)]);
        }

        #[test]
        fn break_inside_an_enclosing_label_is_accepted() {
            assert_eq!(break_errs("label $x | break $x"), Vec::new());
            assert_eq!(break_errs("label $x | 1, (break $x)"), Vec::new());
        }

        #[test]
        fn label_name_is_a_separate_namespace_and_an_unbound_one_fails() {
            // `label $y | break $x` -- `$x` is not scoped.
            assert_eq!(
                break_errs("label $y | break $x"),
                alloc::vec![("x".into(), 0)]
            );
        }

        #[test]
        fn two_same_named_breaks_differ_only_by_scope_carry_distinct_occurrences() {
            // `(label $x | break $x), (label $y | break $x)`: the second is
            // the failing one, and its occurrence (1) is what `collect_break_sites`
            // indexes on to cite the right source position.
            assert_eq!(
                break_errs("(label $x | break $x), (label $y | break $x)"),
                alloc::vec![("x".into(), 1)]
            );
        }

        #[test]
        fn a_break_in_an_unreferenced_def_body_is_still_checked() {
            // `def f: break $x; label $x | f` -- jq compiles `f`'s body at
            // its own position, where no `label $x` is in scope, even though
            // `f` is only ever called in scope of one.
            assert_eq!(
                break_errs("def f: break $x; label $x | f"),
                alloc::vec![("x".into(), 0)]
            );
        }

        #[test]
        fn a_break_in_a_referenced_def_body_call_site_resolves() {
            // `label $x | (def f: break $x; f)` -- `f`'s body is lexically
            // inside the label at its definition, and the call at the label
            // site resolves. Oracle: rc=0.
            assert_eq!(break_errs("label $x | (def f: break $x; f)"), Vec::new());
        }

        #[test]
        fn def_error_shadowing_does_not_suppress_the_label_check() {
            // `def error: "S"; break $x` -- jq's label check fires regardless
            // of a shadowing `def error:`, because the `break`'s own scope is
            // established before the `error/0` substitution model sees it.
            assert_eq!(
                break_errs("def error: \"S\"; break $x"),
                alloc::vec![("x".into(), 0)]
            );
        }

        #[test]
        fn def_error_shadowing_with_an_in_scope_label_accepts() {
            assert_eq!(
                break_errs("def error: \"S\"; label $x | break $x"),
                Vec::new()
            );
        }

        #[test]
        fn break_is_filtered_from_the_function_only_view() {
            // yq-facing view: `break $x` is not a function-call error.
            let mut expr = parse("break $x").expect("filter must parse");
            assert_eq!(resolve_func_calls_all(&mut expr), Vec::new());
        }
    }

    /// #3391: which diagnostics real jq reports -- an error in a compile unit
    /// hides every error in the units beneath it ([`Blocks`]). Every
    /// expectation below was captured from the pinned oracle (`/usr/bin/jq`,
    /// jq-1.7.1), program by program, never from succinctly's own output.
    mod jq_reported {
        use super::*;

        /// The names `resolve_all_jq` reports, in order, as `name/arity`,
        /// `$name` or `$*label-name` -- the same spelling jq prints.
        fn reported(filter: &str) -> Vec<String> {
            let mut expr = parse(filter).expect("filter must parse");
            names(resolve_all_jq(&mut expr))
        }

        /// The same, for `resolve_all`, which keeps every error in the tree.
        fn all(filter: &str) -> Vec<String> {
            let mut expr = parse(filter).expect("filter must parse");
            names(resolve_all(&mut expr))
        }

        fn names(errors: Vec<ResolveError>) -> Vec<String> {
            errors
                .into_iter()
                .map(|e| match e {
                    ResolveError::Call(c) => format!("{}/{}", c.name, c.arity),
                    ResolveError::Var(v) => format!("${}", v.name),
                    ResolveError::Break(b) => format!("$*label-{}", b.name),
                })
                .collect()
        }

        #[test]
        fn an_error_in_the_main_body_hides_a_def_bodys() {
            // The issue's own row: jq prints `bodymissing` and `1 compile
            // error`, where `resolve_all` still sees both.
            let filter = "def t: topmissing; t, bodymissing";
            assert_eq!(reported(filter), ["bodymissing/0"]);
            assert_eq!(all(filter), ["topmissing/0", "bodymissing/0"]);
        }

        #[test]
        fn a_variable_or_break_in_the_main_body_hides_a_def_bodys_too() {
            // Every kind of error is an error of the unit it sits in.
            assert_eq!(reported("def t: topmissing; t | $nov"), ["$nov"]);
            assert_eq!(
                reported("def t: topmissing; t | break $nol"),
                ["$*label-nol"]
            );
            assert_eq!(reported("def t: $v1; t, bodymissing"), ["bodymissing/0"]);
            assert_eq!(
                reported("def t: break $l1; t, bodymissing"),
                ["bodymissing/0"]
            );
        }

        #[test]
        fn a_main_body_error_after_an_inline_construct_still_hides_a_def() {
            // `if`, `as`, `reduce`, `try` and the like are not compile units:
            // an error inside one is an error of the block around it.
            for filter in [
                "def t: tm; if t then bodymissing else 1 end",
                "def t: tm; t as $x | bodymissing",
                "def t: tm; reduce bodymissing as $x (0; 1), t",
                "def t: tm; try 1 catch bodymissing, t",
                "def t: tm; \"\\(bodymissing)\", t",
                "def t: tm; label $l | bodymissing, t",
            ] {
                assert_eq!(reported(filter), ["bodymissing/0"], "{filter}");
            }
        }

        #[test]
        fn sibling_defs_are_each_reported_in_declaration_order() {
            // No error of the main body itself, so each def is compiled.
            assert_eq!(
                reported("def a: undef_a; def b: undef_b; a, b"),
                ["undef_a/0", "undef_b/0"]
            );
            assert_eq!(
                reported("def b: ub; def a: ua; a, b"),
                ["ub/0", "ua/0"],
                "declaration order, not call order"
            );
            // ... but one error in the body hides both.
            assert_eq!(
                reported("def a: undef_a; def b: undef_b; a, b, undef_main"),
                ["undef_main/0"]
            );
        }

        #[test]
        fn a_def_nested_in_a_def_is_hidden_by_the_enclosing_defs_own_error() {
            assert_eq!(
                reported("def f: def g: undef_g; g, undef_f; f"),
                ["undef_f/0"]
            );
            assert_eq!(reported("def f: def g: gg; g; f"), ["gg/0"]);
            assert_eq!(
                reported("def f: def g: gg; g; def h: hh; f, h"),
                ["gg/0", "hh/0"]
            );
            // A def nested in the *body* is still a unit beneath it.
            assert_eq!(reported("1 | def f: undef1; f, undef2"), ["undef2/0"]);
        }

        #[test]
        fn the_whole_chain_below_a_blocked_unit_is_hidden() {
            // `w` calls `u`; both defs are children of the main block.
            assert_eq!(
                reported("def t: 1; def u: topmissing; def w: u; w, bodymissing"),
                ["bodymissing/0"]
            );
        }

        #[test]
        fn an_argument_of_a_def_call_is_a_unit_of_its_own() {
            // Hidden by an error in the calling block ...
            assert_eq!(
                reported("def f(x): x; f(undef_a), undef_main"),
                ["undef_main/0"]
            );
            assert_eq!(
                reported("def f(x): x; f(undef_a) | undef_main"),
                ["undef_main/0"]
            );
            // ... reported on its own, alongside a def's ...
            assert_eq!(
                reported("def f(x): undef_in_f, x; f(undef_a)"),
                ["undef_in_f/0", "undef_a/0"]
            );
            // ... and defs come before the arguments of calls after them.
            assert_eq!(
                reported("def f(x): x; def g: undef_g; f(undef_a), g"),
                ["undef_g/0", "undef_a/0"]
            );
            // `$param` is the same closure.
            assert_eq!(reported("def f($p): $p; f(ua), uz"), ["uz/0"]);
            assert_eq!(reported("def f($p; q): $p, q; f(ua; ub)"), ["ua/0", "ub/0"]);
        }

        #[test]
        fn an_argument_nested_in_an_argument_is_hidden_by_the_outer_ones_error() {
            assert_eq!(reported("def f(x): x; f(ua | map(ub))"), ["ua/0"]);
            assert_eq!(reported("def f(x): x; f(map(ub) | ua)"), ["ua/0"]);
            assert_eq!(reported("def f(x): x; f(def g: gg; g), uz"), ["uz/0"]);
            assert_eq!(reported("def f(x): x; f(def g: gg; g)"), ["gg/0"]);
        }

        #[test]
        fn a_def_in_a_later_pipe_stage_is_a_sibling_of_an_earlier_argument() {
            assert_eq!(reported("[1] | map(ua) | def g: ug; g"), ["ua/0", "ug/0"]);
            assert_eq!(reported("def g: ug; [1] | map(ua) | g"), ["ug/0", "ua/0"]);
            assert_eq!(
                reported("def t: topmissing; t | def g: inner; g"),
                ["topmissing/0", "inner/0"]
            );
        }

        #[test]
        fn a_jq_defined_builtins_argument_is_a_unit_of_its_own() {
            // `map` is a jq `def`: its argument does not hide the def's error,
            // and is itself hidden by one in the calling block.
            assert_eq!(
                reported("def t: topmissing; [t] | map(bodymissing)"),
                ["topmissing/0", "bodymissing/0"]
            );
            assert_eq!(
                reported("[undef_a] | map(undef_b), undef_c"),
                ["undef_a/0", "undef_c/0"]
            );
            assert_eq!(
                reported("def t: tm; (.a |= ua) | (.b |= ub), t"),
                ["tm/0", "ua/0", "ub/0"]
            );
        }

        #[test]
        fn a_c_implemented_builtins_argument_belongs_to_the_calling_block() {
            // `ltrimstr` is a cfunction: the argument is inline, so its error
            // hides the def's like any other error of the block.
            assert_eq!(reported("def t: tm; ltrimstr(ua), t"), ["ua/0"]);
            assert_eq!(
                reported("def t: tm; ltrimstr(ua), rtrimstr(ub), t"),
                ["ua/0", "ub/0"]
            );
            assert_eq!(reported("def t: tm; setpath(ua; ub), t"), ["ua/0", "ub/0"]);
            assert_eq!(reported("def t: tm; ltrimstr(ua | map(ub)), t"), ["ua/0"]);
        }

        #[test]
        fn assignment_operands_are_closures_except_a_compound_right_side() {
            // `=` and `|=` are `_assign`/`_modify` calls: both sides closures.
            assert_eq!(reported("def t: tm; .a |= ua, t"), ["tm/0", "ua/0"]);
            assert_eq!(reported("def t: tm; .a = ua, t"), ["tm/0", "ua/0"]);
            assert_eq!(reported("def t: tm; (ua |= 1), t"), ["tm/0", "ua/0"]);
            // `a op= b` evaluates `b` inline, and hands only the path `a` to
            // `_modify`.
            assert_eq!(reported("def t: tm; .a += ua, t"), ["ua/0"]);
            assert_eq!(reported("def t: tm; .a //= ua, t"), ["ua/0"]);
            assert_eq!(reported("def t: tm; (ua += 1), t"), ["tm/0", "ua/0"]);
            assert_eq!(reported("def t: tm; (ua //= 1), t"), ["tm/0", "ua/0"]);
            assert_eq!(reported("def t: tm; (.a += ua), ub"), ["ua/0", "ub/0"]);
            assert_eq!(reported("def t: tm; (ua += 1), ub"), ["ub/0"]);
        }

        #[test]
        fn the_forms_jq_defines_take_closures() {
            for filter in [
                "def t: tm; limit(1; ua), t",
                "def t: tm; first(ua), t",
                "def t: tm; last(ua), t",
                "def t: tm; repeat(ua), t",
                "def t: tm; error(ua), t",
                "def t: tm; range(ua), t",
                "def t: tm; path(ua), t",
                "def t: tm; del(ua), t",
                "def t: tm; .a[ua] = 1, t",
            ] {
                assert_eq!(reported(filter), ["tm/0", "ua/0"], "{filter}");
            }
            assert_eq!(
                reported("def t: tm; until(ua; ub), uc"),
                ["uc/0"],
                "the call's own error hides both closures"
            );
            assert_eq!(
                reported("def t: tm; while(ua; ub), t"),
                ["tm/0", "ua/0", "ub/0"]
            );
            assert_eq!(
                reported("def t: tm; range(1; ua; ub), t"),
                ["tm/0", "ua/0", "ub/0"]
            );
        }

        #[test]
        fn a_user_def_shadowing_a_builtin_takes_closures_whatever_the_builtin_does() {
            // `ltrimstr` is inline in jq, but a user `def ltrimstr(x)` is a
            // def like any other.
            assert_eq!(
                reported("def t: tm; def ltrimstr(x): x; ltrimstr(ua), t"),
                ["tm/0", "ua/0"]
            );
        }

        #[test]
        fn the_errors_of_every_ancestor_less_unit_are_kept_in_source_order() {
            // Two main-body errors and a def's: only the body's, in order.
            assert_eq!(reported("def t: tm; (ua | t), ub"), ["ua/0", "ub/0"]);
        }

        #[test]
        fn a_program_without_a_body_error_is_unchanged() {
            for filter in [
                "def t: topmissing; t",
                "def t: topmissing; def u: bodymissing; u",
                "def a: undef_a; def b: undef_b; a, b",
                "def f(x): x | ux; f(ua)",
            ] {
                assert_eq!(reported(filter), all(filter), "{filter}");
            }
        }

        /// Asserts each `(filter, expected)` row of `reported`.
        fn assert_reported(rows: &[(&str, &[&str])]) {
            for &(filter, expected) in rows {
                assert_eq!(reported(filter), expected, "{filter}");
            }
        }

        /// #3583: jq compiles the closure units of a block in the order of its
        /// instructions, which is the reverse of source order for the operands
        /// of an operator, of a C-implemented builtin and of an interpolation.
        /// Every row was captured from the pinned jq (1.7.1).
        #[test]
        fn closure_units_under_an_operator_come_out_right_to_left_3583() {
            assert_reported(&[
                ("map(ua) + map(ub)", &["ub/0", "ua/0"]),
                ("map(ua) == map(ub)", &["ub/0", "ua/0"]),
                ("map(ua) + map(ub) + map(uc)", &["uc/0", "ub/0", "ua/0"]),
                ("(1, map(ua)) + (2, map(ub))", &["ub/0", "ua/0"]),
                // Each operator reverses its own operands only; the comma
                // between the two stays in source order.
                (
                    "[map(ua) + map(ub), map(uc) + map(ud)]",
                    &["ub/0", "ua/0", "ud/0", "uc/0"],
                ),
                ("def f(x): x; f(map(ua)) + f(map(ub))", &["ub/0", "ua/0"]),
                // `and`, `or` and `//` are not such calls.
                ("map(ua) and map(ub)", &["ua/0", "ub/0"]),
                ("map(ua) // map(ub)", &["ua/0", "ub/0"]),
                // Nor is a call to a jq-defined function.
                ("def f(x; y): x + y; f(map(ua); map(ub))", &["ua/0", "ub/0"]),
                ("sub(map(ua); map(ub))", &["ua/0", "ub/0"]),
            ]);
        }

        /// #3583: only the units are reversed. A block's own errors are in
        /// source order under the same constructs.
        #[test]
        fn a_blocks_own_errors_under_an_operator_stay_in_source_order_3583() {
            assert_reported(&[
                ("ua + ub", &["ua/0", "ub/0"]),
                ("pow(ua; ub)", &["ua/0", "ub/0"]),
                ("\"\\(ua)\\(ub)\\(uc)\"", &["ua/0", "ub/0", "uc/0"]),
            ]);
        }

        #[test]
        fn c_implemented_builtin_operands_come_out_right_to_left_3583() {
            assert_reported(&[
                ("pow(map(ua); map(ub))", &["ub/0", "ua/0"]),
                ("setpath(map(ua); map(ub))", &["ub/0", "ua/0"]),
                ("fma(map(ua); map(ub); map(uc))", &["uc/0", "ub/0", "ua/0"]),
            ]);
        }

        #[test]
        fn string_interpolation_parts_come_out_right_to_left_3583() {
            assert_reported(&[
                (
                    "\"\\(map(ua))\\(map(ub))\\(map(uc))\"",
                    &["uc/0", "ub/0", "ua/0"],
                ),
                ("@base64 \"\\(map(ua)) \\(map(ub))\"", &["ub/0", "ua/0"]),
            ]);
        }

        /// #3583: `a op= b` compiles `b` before `a`; `=` and `|=` do not.
        #[test]
        fn a_compound_assignments_right_side_is_compiled_first_3583() {
            assert_reported(&[
                ("(map(ua)) += (map(ub))", &["ub/0", "ua/0"]),
                ("(map(ua)) //= (map(ub))", &["ub/0", "ua/0"]),
                ("(map(ua)) |= (map(ub))", &["ua/0", "ub/0"]),
                ("(map(ua)) = (map(ub))", &["ua/0", "ub/0"]),
            ]);
        }

        /// #3583: `reduce` and `foreach` compile `init`, then the source, then
        /// the pattern's computed keys, then the body -- for the block's own
        /// errors as much as for its units.
        #[test]
        fn reduce_and_foreach_compile_init_before_the_source_3583() {
            assert_reported(&[
                ("reduce ua as $x (ub; uc)", &["ub/0", "ua/0", "uc/0"]),
                (
                    "reduce ua as [$a, {(ub): $b}] (uc; .)",
                    &["uc/0", "ua/0", "ub/0"],
                ),
                (
                    "reduce map(ua) as $x (map(ub); map(uc))",
                    &["ub/0", "ua/0", "uc/0"],
                ),
                (
                    "foreach (ua) as $x (ub; uc; ud)",
                    &["ub/0", "ua/0", "uc/0", "ud/0"],
                ),
                (
                    "foreach map(ua) as $x (map(ub); map(uc); map(ud))",
                    &["ub/0", "ua/0", "uc/0", "ud/0"],
                ),
                // A reduce nested in a pipe keeps its place in the pipe.
                (
                    "ua as $x | reduce ub as $y (uc; ud)",
                    &["ua/0", "uc/0", "ub/0", "ud/0"],
                ),
            ]);
        }

        /// #3583: `(target)[key]` and `(target)[from:to]` compile the target
        /// last, whether the errors are the block's own or its units'.
        #[test]
        fn an_index_or_slice_target_is_compiled_last_3583() {
            assert_reported(&[
                ("(ua)[ub]", &["ub/0", "ua/0"]),
                ("(map(ua))[map(ub)]", &["ub/0", "ua/0"]),
                ("(ua)[ub:uc]", &["ub/0", "uc/0", "ua/0"]),
                ("(map(ua))[map(ub):map(uc)]", &["ub/0", "uc/0", "ua/0"]),
                ("(ua)[ub:]", &["ub/0", "ua/0"]),
                ("(ua)[:ub]", &["ub/0", "ua/0"]),
                // `.[from:to]` has no target to move.
                (".[ua:ub]", &["ua/0", "ub/0"]),
            ]);
        }

        /// #3583: the reordering reaches units nested under a `def`, and an
        /// unreferenced `def`'s discarded errors do not disturb it.
        #[test]
        fn the_order_holds_under_a_def_3583() {
            assert_reported(&[
                (
                    "def f: map(ua) + map(ub); f, map(uc) + map(ud)",
                    &["ub/0", "ua/0", "ud/0", "uc/0"],
                ),
                ("def t: tm; map(ua) + map(ub)", &["ub/0", "ua/0"]),
                ("def t: tm; map(ua) + map(ub), t", &["tm/0", "ub/0", "ua/0"]),
            ]);
        }

        /// #3583: the unfiltered view -- the one `succinctly yq` reads -- keeps
        /// the walk's source order.
        #[test]
        fn resolve_all_keeps_source_order_3583() {
            assert_eq!(all("map(ua) + map(ub)"), ["ua/0", "ub/0"]);
            assert_eq!(all("reduce ua as $x (ub; uc)"), ["ua/0", "ub/0", "uc/0"]);
            assert_eq!(all("(ua)[ub]"), ["ua/0", "ub/0"]);
            // `reduce`'s pieces are visited as written -- source, the
            // pattern's computed key, `init`, update -- not in jq's order.
            assert_eq!(
                all("reduce ua as {(ub): $v} (uc; ud)"),
                ["ua/0", "ub/0", "uc/0", "ud/0"]
            );
            assert_eq!(
                reported("reduce ua as {(ub): $v} (uc; ud)"),
                ["uc/0", "ua/0", "ub/0", "ud/0"]
            );
        }

        #[test]
        fn an_unreferenced_def_stays_silent_either_way() {
            // jq never compiles it; the walk already skips it, and its
            // discarded errors must not count as a unit's own.
            assert_eq!(
                reported("def t: topmissing; bodymissing"),
                ["bodymissing/0"]
            );
            assert_eq!(reported("def t: topmissing; 1"), Vec::<String>::new());
        }

        /// The yq-facing views keep every error: the rule is jq's, and yq
        /// has no compile-time diagnostic of this kind to match.
        #[test]
        fn resolve_func_calls_all_still_sees_every_call_error() {
            let mut expr = parse("def t: topmissing; t, bodymissing").expect("filter must parse");
            let names: Vec<String> = resolve_func_calls_all(&mut expr)
                .into_iter()
                .map(|c| c.name)
                .collect();
            assert_eq!(names, ["topmissing", "bodymissing"]);
        }

        /// The inline builtins, as `name/arity`, captured argument by argument
        /// from the pinned jq: `def t: tm; t, NAME(ua; ...)` prints only the
        /// `ua`s for these and `tm` as well for every other entry (no builtin
        /// mixes the two). Everything else in [`JQ_BUILTIN_ROSTER`] with
        /// arguments is a jq-defined function.
        const INLINE: &[&str] = &[
            "atan2/2",
            "contains/1",
            "copysign/2",
            "delpaths/1",
            "drem/2",
            "endswith/1",
            "fdim/2",
            "fma/3",
            "fmax/2",
            "fmin/2",
            "fmod/2",
            "format/1",
            "getpath/1",
            "halt_error/1",
            "has/1",
            "hypot/2",
            "jn/2",
            "ldexp/2",
            "ltrimstr/1",
            "nextafter/2",
            "nexttoward/2",
            "pow/2",
            "remainder/2",
            "rtrimstr/1",
            "scalb/2",
            "scalbln/2",
            "setpath/2",
            "split/1",
            "startswith/1",
            "strflocaltime/1",
            "strftime/1",
            "strptime/1",
            "yn/2",
        ];

        /// #3583: the order the closure units of a multi-argument builtin come
        /// out in follows its operand kind -- last to first for a
        /// C-implemented one, as written for a `def` -- for every roster entry
        /// of arity 2 or more. `NAME(map(ua1); map(ua2); ...)` was captured
        /// from the pinned jq for all of them (43 entries, no exception).
        #[test]
        fn closure_order_follows_the_operand_kind_across_the_roster() {
            let mut checked = 0;
            for &(name, arity) in JQ_BUILTIN_ROSTER.iter().filter(|&&(_, a)| a >= 2) {
                let args: Vec<String> = (1..=arity).map(|i| format!("ua{i}")).collect();
                let filter = format!(
                    "{name}({})",
                    args.iter()
                        .map(|a| format!("map({a})"))
                        .collect::<Vec<_>>()
                        .join("; ")
                );
                let mut expected: Vec<String> = args.iter().map(|a| format!("{a}/0")).collect();
                if INLINE.contains(&format!("{name}/{arity}").as_str()) {
                    expected.reverse();
                }
                assert_eq!(reported(&filter), expected, "{filter}");
                checked += 1;
            }
            assert_eq!(checked, 43, "the roster's entries of arity 2 or more");
        }

        #[test]
        fn builtin_operand_kinds_match_the_pinned_capture() {
            let mut checked = 0;
            for &(name, arity) in JQ_BUILTIN_ROSTER.iter().filter(|&&(_, a)| a >= 1) {
                let args: Vec<String> = (1..=arity).map(|i| format!("ua{i}")).collect();
                let filter = format!("def t: tm; t, {name}({})", args.join("; "));
                let mut expected: Vec<String> = args.iter().map(|a| format!("{a}/0")).collect();
                if !INLINE.contains(&format!("{name}/{arity}").as_str()) {
                    expected.insert(0, "tm/0".into());
                }
                assert_eq!(reported(&filter), expected, "{filter}");
                checked += 1;
            }
            assert_eq!(checked, 100, "the roster's entries of arity 1 or more");
        }
    }
}
