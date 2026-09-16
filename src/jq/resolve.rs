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
use super::walk::{any_subexpr, builtin_kids, map_builtin_subexprs, BuiltinKids};
use super::{Expr, ObjectKey, Pattern, StringPart};

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
    /// same `origin` scope -- resolved *or* unresolved -- the resolver had
    /// already visited, in source order, before this one (#2635).
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
    pub occurrence_index: usize,
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
    /// Which same-named `break $name` site this diagnostic belongs to
    /// (0-based, in `super::parser::collect_break_sites`'s source order): when two
    /// same-named breaks differ only by lexical scope — one bound under an
    /// enclosing `label $name`, one genuinely unbound — the position
    /// recovery in `report_compile_errors` (the CLI) must cite the *failing*
    /// one, and this index is how it threads that decision through the AST,
    /// which carries no positions. Mirrors the `calls`/`vars` "which
    /// occurrence" counters in the CLI's own `report_compile_errors`
    /// (`calls_consumed`/`vars_consumed`).
    pub occurrence: usize,
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
/// A `Vec` used as a stack, not a map: scopes are small (a handful of `def`s
/// and parameters), shadowing falls out of searching from the top, and pushing
/// and truncating is cheaper than cloning a map per node.
type Scope = Vec<(String, usize)>;

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
/// `~/.jq` block, each `import`, and each per-origin group of dependencies
/// wrapped into a body) between a begin and an end marker:
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
/// The prefix of a dependency renamed out of a def's way (#2962) -- see
/// [`ModuleRun::renamed_dep`]. Not a marker: [`ModuleRun::parse`] reads it as
/// an ordinary def, which is what it is.
const DEP_RENAME: &str = "\u{0}dep:";

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

    /// The name a dependency is renamed to when it would otherwise collide
    /// with the def it is wrapped into (#2962): entry `index` of run `id`'s
    /// dependency group, originally called `name`.
    ///
    /// A dependency bound into a def's body sits *inside* that def's scope,
    /// so a dependency sharing the def's (name, arity), or named after one of
    /// its parameters, would shadow the def's own binding for the body. jq
    /// binds a module's block in its own scope and never has the clash. The
    /// NUL prefix keeps the new name out of reach of anything a user can
    /// write, exactly as it does for the markers.
    #[must_use]
    pub fn renamed_dep(id: u32, index: usize, name: &str) -> String {
        alloc::format!("{DEP_RENAME}{id}:{index}:{name}")
    }

    /// `name` as a user wrote it: [`Self::renamed_dep`] undone, and any other
    /// name returned unchanged. For messages that name a def, so an internal
    /// spelling never reaches the terminal.
    #[must_use]
    pub fn display_name(name: &str) -> &str {
        name.strip_prefix(DEP_RENAME)
            .and_then(|rest| rest.splitn(3, ':').nth(2))
            .unwrap_or(name)
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

/// The one place the module-scope floor is applied.
///
/// Walks `scope` innermost-first, handing each ordinary entry to `probe` and
/// stopping at the first begin marker that has no matching end -- the floor.
/// Returns whatever `probe` matched (if anything) together with the innermost
/// open run, which callers need for the `import` retry (#2989) and for
/// attributing a compile error to the module it came from.
///
/// Both walks in this file share it, rather than each spelling the marker
/// bookkeeping out: the file's own header already flags the two as the pair
/// that must stay in lock-step, and "which names are visible here" is exactly
/// the kind of duplicated predicate that diverges silently.
fn scan_scope<'a, E, T>(
    scope: &'a [E],
    name_of: impl Fn(&'a E) -> &'a str,
    mut probe: impl FnMut(&'a E) -> Option<T>,
) -> ScanResult<T> {
    // Counts end markers seen but not yet matched by an opener. A run whose
    // end we have already passed is *closed*: it is wrapped around this
    // point, so its defs are visible and its opener is not a floor.
    let mut closed = 0usize;
    for entry in scope.iter().rev() {
        match ModuleRun::parse(name_of(entry)) {
            Some(RunMarker::End { .. }) => closed += 1,
            Some(RunMarker::Begin { id, alias }) => {
                if closed > 0 {
                    closed -= 1;
                } else {
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

/// A lexical *variable* scope: the `$name`s (without the sigil) visible at a
/// point in the tree — [`Scope`]'s sibling for #2734, tracked in the same
/// walk rather than a separate pass (see [`check`]'s doc comment).
///
/// No arity component: unlike a function, a variable binding has no
/// overload-by-arity concept, so plain name equality is enough.
type VarScope = Vec<String>;

/// A lexical *label* scope (#2964): the `$label` names (without the sigil)
/// visible at a point in the tree — [`VarScope`]'s sibling for `break`'s
/// compile-time label-scope check, tracked in the same walk rather than a
/// separate pass (see [`check`]'s doc comment).
///
/// No arity or value component, same as [`VarScope`] but for `label`
/// bindings: only the name matters. Restored before returning so a sibling
/// never sees a label introduced by its neighbour, exactly like `var_scope`.
type LabelScope = Vec<String>;

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
type ReachScope = Vec<(String, usize, ScopeHit)>;

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
    scan_scope(
        scope,
        |(n, _, _)| n.as_str(),
        |(n, a, hit)| (*a == arity && n == name).then_some(*hit),
    )
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
        | Expr::Iterate
        | Expr::Literal(_)
        | Expr::RecursiveDescent
        | Expr::Not
        | Expr::Format(_)
        | Expr::Var(_)
        | Expr::TrackedVar(_)
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
            scope.push((name.clone(), params.len(), ScopeHit::Def(body_addr)));
            let with_self = scope.len();
            for p in params {
                scope.push((p.name().to_string(), 0, ScopeHit::Param));
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

        Expr::Pipe(exprs) | Expr::Comma(exprs) => {
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
            // #2964: breaks are checked only in jq mode. Real yq marshals
            // `break`/`label` through its own jq-superset evaluator, where
            // a break's label scope is resolved at *runtime* much like
            // succinctly's does — yq has no compile-time label check to
            // match (mirrors `ResolveError::Var`'s own yq-mode exclusion,
            // #2981), so folding these into yq's output would make
            // `succinctly yq` reject programs real yq accepts.
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
    // #2740: a `def` whose call never appears anywhere reachable is never
    // checked -- jq's own compiler never compiles such a body either, since
    // it only ever compiles a `def` at the call site substituting it in.
    // `build_call_graph` (read-only) computes which bodies are reachable
    // before `check` (below, `&mut`-mutating) walks the tree for real.
    let mut reach_scope = ReachScope::new();
    let mut graph = BTreeMap::new();
    let mut roots = Vec::new();
    build_call_graph(expr, &mut reach_scope, None, &mut graph, &mut roots);
    let reachable = compute_reachable(&graph, &roots);

    let mut scope = Scope::new();
    let mut var_scope = VarScope::new();
    let mut label_scope = LabelScope::new();
    // #2964: how many `break $name` sites the walk has already visited per
    // label name, so a diagnostic carries which *occurrence* of the name —
    // the same "which `$x` actually failed" question `var_scope` can't
    // answer by itself, resolved here by counting as we go (see
    // `UnresolvedLabel::occurrence`).
    let mut break_occurrences: BTreeMap<String, usize> = BTreeMap::new();
    let mut errors = Vec::new();
    let mut occurrences = BTreeMap::new();
    check(
        expr,
        &mut scope,
        &mut var_scope,
        &mut label_scope,
        &mut errors,
        &reachable,
        &mut occurrences,
        &mut break_occurrences,
    );
    errors
}

/// Whether `(name, arity)` is one of the pinned jq's own builtins.
fn is_jq_builtin(name: &str, arity: usize) -> bool {
    JQ_BUILTIN_ROSTER
        .iter()
        .any(|&(n, a)| a == arity && n == name)
}

/// Records one more visit to `(name, arity)` -- resolved or not -- and
/// returns how many earlier visits to that exact pair `occurrences` had
/// already recorded (#2635). Shared by every terminal arm of `check`'s
/// `Expr::FuncCall` handling that represents a genuine, distinct call site
/// (not the "restore the original parse and re-dispatch" arm, which revisits
/// the same position rather than a new one).
fn next_call_occurrence(
    occurrences: &mut BTreeMap<(Option<u32>, String, usize), usize>,
    origin: Option<u32>,
    name: &str,
    arity: usize,
) -> usize {
    let count = occurrences
        .entry((origin, name.to_string(), arity))
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
    scan_scope(
        scope,
        |(n, _)| n.as_str(),
        |(n, a)| (*a == arity && n == name).then_some(()),
    )
}

/// Whether `$name` resolves against `var_scope`, innermost first, stopping
/// at the same module-scope floor as [`in_scope`] (#2962): a `$`-parameter
/// of the def a dependency is spliced into is not the dependency's to see.
fn in_var_scope(var_scope: &VarScope, name: &str) -> ScanResult<()> {
    scan_scope(var_scope, String::as_str, |n| (n == name).then_some(()))
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
fn check_pattern_keys(
    pattern: &mut Pattern,
    scope: &mut Scope,
    var_scope: &mut VarScope,
    label_scope: &mut LabelScope,
    errors: &mut Vec<ResolveError>,
    reachable: &BTreeSet<usize>,
    occurrences: &mut BTreeMap<(Option<u32>, String, usize), usize>,
    break_occurrences: &mut BTreeMap<String, usize>,
) {
    match pattern {
        Pattern::Var(_) => {}
        Pattern::Object(entries) => {
            for entry in entries.iter_mut() {
                if let ObjectKey::Expr(k) = &mut entry.key {
                    check(
                        k,
                        scope,
                        var_scope,
                        label_scope,
                        errors,
                        reachable,
                        occurrences,
                        break_occurrences,
                    );
                }
                check_pattern_keys(
                    &mut entry.pattern,
                    scope,
                    var_scope,
                    label_scope,
                    errors,
                    reachable,
                    occurrences,
                    break_occurrences,
                );
            }
        }
        Pattern::Array(patterns) => {
            for p in patterns.iter_mut() {
                check_pattern_keys(
                    p,
                    scope,
                    var_scope,
                    label_scope,
                    errors,
                    reachable,
                    occurrences,
                    break_occurrences,
                );
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
    scope: &mut Scope,
    var_scope: &mut VarScope,
    label_scope: &mut LabelScope,
    errors: &mut Vec<ResolveError>,
    reachable: &BTreeSet<usize>,
    occurrences: &mut BTreeMap<(Option<u32>, String, usize), usize>,
    break_occurrences: &mut BTreeMap<String, usize>,
) -> Vec<String> {
    for pattern in patterns.iter_mut() {
        check_pattern_keys(
            pattern,
            scope,
            var_scope,
            label_scope,
            errors,
            reachable,
            occurrences,
            break_occurrences,
        );
    }
    pattern_alternatives_var_names(patterns)
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

/// Recurse into `expr` under `scope`, restoring `scope` before returning so a
/// sibling never sees a binding introduced by its neighbour. Appends every
/// unresolvable call to `errors` rather than stopping at the first, matching
/// how real jq's own compiler keeps going to report every compile error in
/// one pass (#2037).
fn check(
    expr: &mut Expr,
    scope: &mut Scope,
    var_scope: &mut VarScope,
    label_scope: &mut LabelScope,
    errors: &mut Vec<ResolveError>,
    reachable: &BTreeSet<usize>,
    occurrences: &mut BTreeMap<(Option<u32>, String, usize), usize>,
    break_occurrences: &mut BTreeMap<String, usize>,
) {
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
                let target = Rc::make_mut(inner);
                let rebased = rebase_reachable(&original, target, reachable);
                check(
                    target,
                    scope,
                    var_scope,
                    label_scope,
                    errors,
                    &rebased,
                    occurrences,
                    break_occurrences,
                );
            } else {
                check(Rc::make_mut(inner), scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            }
        }
        Expr::DefCall { args, .. } => {
            for arg in args.iter_mut() {
                check(arg, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            }
        }
        // Leaves: nothing nested to descend into. Mirrors `walk::any_subexpr`'s
        // own grouping so the two stay comparable arm for arm. `Var` is
        // checked separately below (#2734) rather than folded in here.
        Expr::Identity
        | Expr::Field(_)
        | Expr::Index { .. }
        | Expr::Slice { .. }
        | Expr::Iterate
        | Expr::Literal(_)
        | Expr::RecursiveDescent
        | Expr::Not
        | Expr::Format(_)
        | Expr::TrackedVar(_)
        | Expr::Loc { .. }
        | Expr::Env => {}

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
        // `break_occurrences` carries *which* same-named break site this
        // one is — the #2635-class limitation: a break inside an
        // unreachable `def` body that textually precedes the failing one
        // shifts the CLI's caret target by one (see the `UnresolvedLabel`
        // doc comment and the `BreakSite` doc comment for the full
        // accounting), the exact same class `CallSite`/`VarSite` already
        // record for calls/variables.
        Expr::Break(name) => {
            let n = break_occurrences.entry(name.clone()).or_insert(0);
            let occurrence = *n;
            *n += 1;
            // Label scope is determined at the break's *own* lexical
            // position — a `label $x` only covers nodes nested inside its
            // body, not siblings of that body. `label_scope` is a stack
            // pushed/popped by the `Label` arm below, so this check sees
            // only genuinely enclosing labels.
            if !label_scope.iter().rev().any(|l| l == name) {
                errors.push(ResolveError::Break(UnresolvedLabel {
                    name: name.clone(),
                    occurrence,
                }));
            }
        }

        // #2734: the leaf this whole pass exists to add. `$ENV`/`$__loc__`
        // never reach here at all -- the parser lowers them straight to
        // `Expr::Env`/`Expr::Loc` (see `UnboundVar`'s own doc comment) -- and
        // `TrackedVar` is an evaluation-time-only substitute for a `Var` that
        // already resolved, never present on the freshly parsed tree this
        // pass runs on (same reasoning as the `Shared`/`DefCall` arm above).
        Expr::Var(name) => {
            let found = in_var_scope(var_scope, name);
            if found.hit.is_none() {
                errors.push(ResolveError::Var(UnboundVar {
                    name: name.clone(),
                    origin: found.run.map(|(id, _)| id),
                }));
            }
        }

        Expr::Optional(inner)
        | Expr::Array(inner)
        | Expr::Paren(inner)
        | Expr::Negate(inner)
        | Expr::FirstExpr(inner)
        | Expr::LastExpr(inner)
        | Expr::Repeat(inner) => check(inner, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences),

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
            label_scope.push(name.clone());
            check(body, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            label_scope.pop();
            if in_scope(scope, "error", 0).hit.is_some() {
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

        Expr::Error(inner) => {
            if let Some(e) = inner.as_deref_mut() {
                check(e, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
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
            check(left, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            check(right, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
        }

        // #2734: binds `var` in `body` only, not `expr` -- `.foo as $x | ...`
        // evaluates `.foo` before `$x` exists.
        Expr::As { expr, var, body } => {
            check(expr, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            var_scope.push(var.clone());
            check(body, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            var_scope.pop();
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
            check(expr, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            let outer = var_scope.len();
            let bound = bind_patterns(patterns, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            var_scope.extend(bound);
            check(body, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            var_scope.truncate(outer);
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
            let outer = scope.len();
            scope.push((name.clone(), params.len()));
            let with_self = scope.len();

            // A parameter binds its bare name at arity 0 only: `def f(g):
            // g(1)` is `g/1 is not defined` in jq, not a call to the outer
            // `g`. Pushed after the function's own name so a parameter
            // shadowing it wins. A `$`-style parameter also binds `$name`
            // in the same pass over `params`, one loop for both namespaces.
            let var_outer = var_scope.len();
            for p in params.iter() {
                scope.push((p.name().to_string(), 0));
                if p.is_dollar() {
                    var_scope.push(p.name().to_string());
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
            if reachable.contains(&body_addr) {
                check(body, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            }
            var_scope.truncate(var_outer);
            scope.truncate(with_self);

            // A run marker floors variables as well as functions (#2962), so
            // it goes on the variable stack too -- for `then` only, since a
            // marker's own body is `.`.
            let is_marker = ModuleRun::parse(name).is_some();
            if is_marker {
                var_scope.push(name.clone());
            }
            check(then, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            var_scope.truncate(var_outer);
            scope.truncate(outer);
        }

        Expr::Try { expr, catch } => {
            check(expr, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            if let Some(c) = catch.as_deref_mut() {
                check(c, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            }
        }

        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            check(cond, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            check(then_branch, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            check(else_branch, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
        }

        Expr::SliceExpr { target, start, end } => {
            check(target, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            check_opt(start.as_deref_mut(), scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            check_opt(end.as_deref_mut(), scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
        }

        Expr::Range { from, to, step } => {
            check(from, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            check_opt(to.as_deref_mut(), scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            check_opt(step.as_deref_mut(), scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
        }

        // #2734: `patterns`' vars are bound in `update` only, not `init` --
        // `reduce .[] as $x (0; . + $x)` evaluates `init` before any element
        // has been bound. Same union-of-alternatives and computed-key
        // treatment as `Expr::AsPattern` above.
        Expr::Reduce {
            input,
            patterns,
            init,
            update,
        } => {
            // #2635 review: `patterns` (its own computed keys) is checked
            // before `init`, matching source-*text* order -- `SOURCE as
            // PATTERN (INIT; UPDATE)` writes the pattern before the
            // parenthesized part, and `collect_call_sites`' table is sorted
            // by byte offset, purely textual. `occurrence_index` needs this
            // order specifically for a name repeated across these
            // positions to land on the right table entry -- it does not
            // change which var_scope `init` sees (`bind_patterns` only
            // *checks* `patterns`'s own computed keys here; the names it
            // returns are not folded into `var_scope` until the explicit
            // `extend` below).
            //
            // Confirmed live this does not make multi-error *reporting*
            // order match jq exactly: real jq 1.7.1 prints a bare `init`
            // error before a `patterns` one (`h | reduce empty as {(h): $x}
            // (h; h)` split across lines reports line 4 before line 3),
            // its own compile-order artifact unrelated to text position --
            // an existing, orthogonal divergence class (`resolve_all`'s own
            // doc comment already notes reporting order is source-order-ish,
            // not full jq-compile-order fidelity) this reordering does not
            // newly introduce, since #2635's own scope is per-occurrence
            // line *correctness*, not inter-error print order. Every
            // individual citation still lands on its own correct line
            // either way (confirmed against the same live example).
            check(input, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            let outer = var_scope.len();
            let bound = bind_patterns(patterns, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            check(init, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            var_scope.extend(bound);
            check(update, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            var_scope.truncate(outer);
        }

        // #2734: same rule as `Expr::Reduce` above -- `init` sees no bound
        // vars, `update`/`extract` both do. #2635 review: same patterns-
        // before-init reordering, same reasoning.
        Expr::Foreach {
            input,
            patterns,
            init,
            update,
            extract,
        } => {
            check(input, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            let outer = var_scope.len();
            let bound = bind_patterns(patterns, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            check(init, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            var_scope.extend(bound);
            check(update, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            check_opt(extract.as_deref_mut(), scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            var_scope.truncate(outer);
        }

        Expr::Pipe(exprs) | Expr::Comma(exprs) => {
            for e in exprs.iter_mut() {
                check(e, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
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
            let scan = in_scope(scope, name, arity);
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
                    in_scope(scope, &qualified, arity).hit.map(|()| qualified)
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
                next_call_occurrence(occurrences, origin, name, arity);
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
                        let n = break_occurrences.entry(name.clone()).or_insert(0);
                        let occurrence = *n;
                        *n += 1;
                        if !label_scope.iter().rev().any(|l| l == name) {
                            errors.push(ResolveError::Break(UnresolvedLabel {
                                name: name.clone(),
                                occurrence,
                            }));
                        }
                    }
                    *args = builtin_fallback_into_args(*fallback);
                    cloned_from.map(|from| {
                        let to = args.iter().flat_map(def_body_addrs).collect();
                        translate_reachable(&from, to, reachable)
                    })
                });
                let reachable = rebased.as_ref().unwrap_or(reachable);
                for a in args.iter_mut() {
                    check(a, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
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
                check(expr, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            } else if is_jq_builtin(name, arity) {
                next_call_occurrence(occurrences, origin, name, arity); // omni-dev: coverage tolerate-line reason="unreachable in practice today: this arm needs builtin_fallback==None (the name was never a shadow candidate) yet is_jq_builtin==true (a real jq builtin at this arity) -- every implemented builtin's own dedicated parse already lowers that shape to Expr::Builtin before resolve.rs ever runs, and #3042/#3046 closed the once-real 'unimplemented builtin' gap this existed for (see JQ_BUILTIN_ROSTER's own doc comment)"
                for a in args.iter_mut() {
                    check(a, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences); // omni-dev: coverage tolerate-line reason="same unreachable arm as the line above"
                }
            } else {
                let occurrence_index = next_call_occurrence(occurrences, origin, name, arity);
                errors.push(ResolveError::Call(UnresolvedCall {
                    name: name.clone(),
                    arity,
                    origin,
                    occurrence_index,
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
                check(a, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            }
        }

        Expr::Object(entries) => {
            for entry in entries.iter_mut() {
                if let ObjectKey::Expr(k) = &mut entry.key {
                    check(k, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
                }
                check(&mut entry.value, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
            }
        }

        Expr::StringInterpolation(parts) => {
            for part in parts.iter_mut() {
                if let StringPart::Expr(e) = part {
                    check(e, scope, var_scope, label_scope, errors, reachable, occurrences, break_occurrences);
                }
            }
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
        Expr::Builtin(builtin) => {
            *builtin = map_builtin_subexprs(builtin, &mut |sub| {
                let mut copy = sub.clone();
                let rebased = rebase_reachable(sub, &copy, reachable);
                check(
                    &mut copy,
                    scope,
                    var_scope,
                    label_scope,
                    errors,
                    &rebased,
                    occurrences,
                    break_occurrences,
                );
                copy
            });
        }
    }
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
/// Built on [`any_subexpr`], plus the two places it does not look but
/// [`check`] does: a call's `builtin_fallback` (the alternate parse a
/// shadowed builtin name carries, which is where `select(def g: ..; g)` puts
/// its def once `select` is also user-defined) and a destructuring pattern's
/// computed object keys (#2734). Missing either left those defs unpaired.
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
            Expr::Reduce { patterns, .. }
            | Expr::Foreach { patterns, .. }
            | Expr::AsPattern { patterns, .. } => {
                for pattern in patterns {
                    collect_pattern_def_body_addrs(pattern, addrs);
                }
            }
            _ => {}
        }
        false
    });
}

fn collect_pattern_def_body_addrs(pattern: &Pattern, addrs: &mut Vec<usize>) {
    match pattern {
        Pattern::Var(_) => {}
        Pattern::Object(entries) => {
            for entry in entries {
                if let ObjectKey::Expr(key) = &entry.key {
                    collect_def_body_addrs(key, addrs);
                }
                collect_pattern_def_body_addrs(&entry.pattern, addrs);
            }
        }
        Pattern::Array(patterns) => {
            for pattern in patterns {
                collect_pattern_def_body_addrs(pattern, addrs);
            }
        }
    }
}

/// [`check`] over an optional sub-expression.
fn check_opt(
    expr: Option<&mut Expr>,
    scope: &mut Scope,
    var_scope: &mut VarScope,
    label_scope: &mut LabelScope,
    errors: &mut Vec<ResolveError>,
    reachable: &BTreeSet<usize>,
    occurrences: &mut BTreeMap<(Option<u32>, String, usize), usize>,
    break_occurrences: &mut BTreeMap<String, usize>,
) {
    if let Some(e) = expr {
        check(
            e,
            scope,
            var_scope,
            label_scope,
            errors,
            reachable,
            occurrences,
            break_occurrences,
        );
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

        let mut unique = Expr::Shared(Rc::new(program()));
        assert_eq!(
            resolve_func_calls(&mut unique).map_err(|e| format!("{e}")),
            Err("nosuchfn/0 is not defined".into()),
            "uniquely owned: make_mut does not clone"
        );

        let inner = Rc::new(program());
        let _keep = Rc::clone(&inner);
        let mut shared = Expr::Shared(inner);
        assert_eq!(
            resolve_func_calls(&mut shared).map_err(|e| format!("{e}")),
            Err("nosuchfn/0 is not defined".into()),
            "multi-owner: make_mut clones, which must not strand the def"
        );

        // And the gate still holds there: an uncalled def stays skipped.
        let inner = Rc::new(parse("def g: nosuchfn; 1").expect("filter must parse"));
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

        let mut scope = Scope::new();
        let mut var_scope = VarScope::new();
        let mut label_scope = LabelScope::new();
        let mut errors = Vec::new();
        let mut occurrences = BTreeMap::new();
        let mut break_occurrences = BTreeMap::new();
        check(
            &mut Expr::Shared(Rc::new(unresolved())),
            &mut scope,
            &mut var_scope,
            &mut label_scope,
            &mut errors,
            &BTreeSet::new(),
            &mut occurrences,
            &mut break_occurrences,
        );
        assert_eq!(
            errors,
            [ResolveError::Call(UnresolvedCall {
                name: "nosuchfn".into(),
                arity: 0,
                // Not inside any module run: these hand-built trees have no
                // markers, so the floor is 0 and there is nothing to
                // attribute the failure to (#2951).
                origin: None,
                occurrence_index: 0,
            })]
        );

        let mut scope = Scope::new();
        let mut var_scope = VarScope::new();
        let mut label_scope = LabelScope::new();
        let mut errors = Vec::new();
        let mut occurrences = BTreeMap::new();
        let mut break_occurrences = BTreeMap::new();
        check(
            &mut Expr::DefCall {
                def: Rc::new(crate::jq::FuncDefData {
                    name: "f".into(),
                    params: Vec::new(),
                    body: Expr::Identity,
                }),
                args: alloc::vec![unresolved()],
                frames: 0,
                bound: crate::jq::BoundBody::default(),
            },
            &mut scope,
            &mut var_scope,
            &mut label_scope,
            &mut errors,
            &BTreeSet::new(),
            &mut occurrences,
            &mut break_occurrences,
        );
        assert_eq!(
            errors,
            [ResolveError::Call(UnresolvedCall {
                name: "nosuchfn".into(),
                arity: 0,
                // Not inside any module run: these hand-built trees have no
                // markers, so the floor is 0 and there is nothing to
                // attribute the failure to (#2951).
                origin: None,
                occurrence_index: 0,
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

        let mut scope: ReachScope = alloc::vec![("f".to_string(), 0, ScopeHit::Def(42))];
        let mut graph = BTreeMap::new();
        let mut roots = Vec::new();
        build_call_graph(
            &Expr::Shared(Rc::new(call_to_f())),
            &mut scope,
            None,
            &mut graph,
            &mut roots,
        );
        assert_eq!(roots, alloc::vec![42]);

        let mut scope: ReachScope = alloc::vec![("f".to_string(), 0, ScopeHit::Def(42))];
        let mut graph = BTreeMap::new();
        let mut roots = Vec::new();
        build_call_graph(
            &Expr::DefCall {
                def: Rc::new(crate::jq::FuncDefData {
                    name: "f".into(),
                    params: Vec::new(),
                    body: Expr::Identity,
                }),
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
        /// `"begin:1@a"` are markers, anything else is a def of that name at
        /// arity 0.
        fn scope_of(entries: &[&str]) -> Scope {
            entries
                .iter()
                .map(|e| {
                    let name = if let Some(rest) = e.strip_prefix("begin:") {
                        match rest.split_once('@') {
                            Some((id, alias)) => {
                                ModuleRun::begin_marker(id.parse().expect("id"), Some(alias))
                            }
                            None => ModuleRun::begin_marker(rest.parse().expect("id"), None),
                        }
                    } else if let Some(id) = e.strip_prefix("end:") {
                        ModuleRun::end_marker(id.parse().expect("id"))
                    } else {
                        (*e).to_string()
                    };
                    (name, 0usize)
                })
                .collect()
        }

        fn visible(entries: &[&str], name: &str) -> bool {
            in_scope(&scope_of(entries), name, 0).hit.is_some()
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

        /// #2962: a renamed dependency is an ordinary def to the scan, and
        /// its display name is the one the user wrote -- including a name
        /// that itself contains `:` (an `alias::name`).
        #[test]
        fn renamed_deps_are_ordinary_defs_and_display_as_written() {
            let renamed = ModuleRun::renamed_dep(4, 2, "c");
            assert_eq!(ModuleRun::parse(&renamed), None);
            assert_eq!(ModuleRun::display_name(&renamed), "c");
            let renamed = ModuleRun::renamed_dep(4, 2, "ns::c");
            assert_eq!(ModuleRun::display_name(&renamed), "ns::c");
            for ordinary in ["c", "ns::c", ""] {
                assert_eq!(ModuleRun::display_name(ordinary), ordinary);
            }
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
}
