//! jq-compatible command runner for succinctly.
//!
//! This module implements a jq-compatible CLI interface using the succinctly
//! JSON semi-indexing and jq expression evaluator.

use anyhow::{Context, Result};
use indexmap::IndexMap;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use succinctly::dsv::{build_index as build_dsv_index, DsvConfig, DsvRows};
use succinctly::jq::document::{
    effective_keys, key_hash, key_span_fingerprint, DistinctKeyCursors, DocumentCursor,
    DocumentValue, IndentSpec, JsonConvention, PAIRWISE_SPAN_SCAN_LIMIT,
};
use succinctly::jq::eval_generic::{
    check_nesting_depth, eval_with_cursor, to_owned as generic_to_owned,
    to_owned_checked as generic_to_owned_checked, validate_cursor, GenericResult, LazyElem,
    MAX_NESTING_DEPTH,
};
use succinctly::jq::walk::{map_builtin_subexprs, map_pattern_subexprs, stamp_loc_file};
use succinctly::jq::{
    self, format_number_jq_compat, jq_bare_float_display, nonfinite_display_string, Builtin,
    ErrorKind, EvalError, EvalErrorPayload, Expr, FuncDefBound, JqSemantics, JqValue, OwnedValue,
    Param, Pattern, Program, StreamStats, MAX_VALUE_TREE_DEPTH,
};
use succinctly::json::light::{preceding_gap_ok, JsonCursor, JsonString, StandardJson};
use succinctly::json::validate::{self, ValidationError};
use succinctly::json::JsonIndex;

use super::m2_gate::can_use_m2_streaming;
use super::JqCommand;
use crate::output::{
    self, escape_json_string, escape_json_string_ascii, exit_codes, flush_then_err, ColorScheme,
    DiagStyle, ErrorSink, InputLocation, JsonFormatOpts, LoudFlushWriter, Terminator,
};

/// Evaluation context for passing variables to the jq evaluator.
#[derive(Debug, Default)]
pub struct EvalContext {
    /// Named arguments from --arg, --argjson, --slurpfile, --rawfile
    pub named: IndexMap<String, OwnedValue>,
    /// Positional arguments from --args or --jsonargs
    pub positional: Vec<OwnedValue>,
}

/// A module or filter's function definitions, as extracted from its parsed
/// `Expr`: name, parameters, body. Named (#2703) so the two-parameter
/// `Result<_, ModuleLoadError>` signatures this module needs don't also
/// spell out this tuple-of-a-tuple inline, which clippy's `type_complexity`
/// lint flags once it stops being folded into `anyhow::Result`'s single-
/// parameter alias.
type FuncDefList = Vec<(String, Vec<Param>, Expr)>;

/// One module's dependencies, grouped by the module each group came from
/// (#2951), in declaration order, with the alias an `import` group was
/// brought in under (`None` for an `include`). Only each dependency's
/// signature (name, params) travels here: the bodies stay in the cache and
/// are linked once, as a run of their own, by [`ModuleLoader::process_program`]
/// (#2955); a consuming def reaches them through forwarding stubs built from
/// these signatures by [`dep_stubs_for`].
type DepGroups = Vec<(u32, Option<String>, Vec<(String, Vec<Param>)>)>;

/// The run id reserved for `~/.jq`'s own defs (#2951) -- assigned in
/// [`ModuleLoader::new`] before any module can claim one.
const AUTO_LOAD_RUN_ID: u32 = 0;

/// Module loader for resolving and loading jq modules.
#[derive(Debug)]
pub struct ModuleLoader {
    /// Search path for modules (in order of priority)
    search_path: Vec<PathBuf>,
    /// Loaded modules, keyed by [`Self::run_key`] -- the same canonical-file
    /// key [`Self::run_ids`]/[`Self::run_origins`] intern by, not the literal
    /// path a caller happened to write (function definitions: name, params,
    /// body). Two different literal spellings of one module (`"dep"` from one
    /// includer, `"./dep"` from another) canonicalize to the same key here,
    /// so the module loads once regardless of how many spellings reach it
    /// (#2955) -- keying on the literal path used to let each spelling load
    /// and cache its own copy.
    loaded_modules: BTreeMap<String, FuncDefList>,
    /// Auto-loaded ~/.jq file definitions (if file exists): name, params, body
    auto_loaded_defs: FuncDefList,
    /// The modules whose own dependencies are currently being loaded, as
    /// `(resolved canonical file, module path as written)`, outermost first
    /// (#2865).
    ///
    /// The cycle guard, and it has to be separate from `loaded_modules`: a
    /// module is *absent* from that cache for exactly as long as its own
    /// dependencies are loading, which is precisely the window in which a
    /// cycle closes. Keyed on the resolved file rather than the written path
    /// so `include "./m"` inside `m.jq` is still caught.
    loading: Vec<(PathBuf, String)>,
    /// Run id per origin, keyed by canonical file (#2951). One id per
    /// *source*, not per directive: a module included once and imported twice
    /// is one id wrapped in three runs, which is right -- the id names where
    /// a def was written, and each run's own alias is what distinguishes the
    /// copies.
    run_ids: BTreeMap<String, u32>,
    /// `run id -> canonical path`, so a compile error raised inside a module
    /// body can say which file it came from instead of `<top-level>`.
    run_origins: BTreeMap<u32, String>,
    /// Next unused run id. `0` is reserved for `~/.jq`.
    next_run_id: u32,
    /// `run id -> (decl_index, run id)` of that module's own dependencies
    /// (#2955), `decl_index` shared across `include`/`import` exactly as
    /// [`Import::decl_index`]/[`Include::decl_index`] number them. Read by
    /// [`Self::hoist_order`], which sorts by `decl_index` before recursing --
    /// insertion order alone is *not* declaration order here, because
    /// [`Self::module_dep_defs`] records all of a module's `include`s before
    /// any of its `import`s, regardless of how the two are interleaved in the
    /// source.
    deps_of: BTreeMap<u32, Vec<(usize, u32)>>,
}

/// A [`ModuleLoader`] failure, structured enough to report in jq's own
/// per-case shape (#2703) -- unlike a flattened `anyhow::Error`, which loses
/// the distinction [`report_module_load_error`] needs.
#[derive(Debug)]
pub(crate) enum ModuleLoadError {
    /// No `{module_path}.jq` was found anywhere in the search path. jq's own
    /// shape has no source location to show for this case: `module not
    /// found: {module_path}`, a blank line standing in for the missing echo,
    /// then the usual `jq: 1 compile error` trailer. Confirmed live against
    /// jq 1.7.1, byte-for-byte.
    NotFound { module_path: String },
    /// A module's own `include`/`import` chain leads back to a module already
    /// being loaded (`ca` includes `cb` includes `ca`, or a module including
    /// itself).
    ///
    /// **A deliberate ADR-0018 rule-4 divergence** (#2865), under the explicit
    /// "matching would take the host process down" carve-out: real jq 1.7.1
    /// does not diagnose this at all, it recurses until it dies --
    /// `jq -L . -n 'include "ca"; a'` exits **139** (SIGSEGV) with no output on
    /// either stream, and a self-including module does the same. Reproducing
    /// that faithfully is not an option, so succinctly reports the cycle and
    /// leaves through the same `jq: 1 compile error` / exit 3 door as the other
    /// two compile-error kinds.
    ///
    /// `chain` is the module paths **as written in the `include`/`import`
    /// directives**, from the outermost module in the cycle through to the
    /// repeat that closed it (`["ca", "cb", "ca"]`); detection itself keys on
    /// the *resolved* file, so two spellings of one module still close a cycle.
    Cycle { chain: Vec<String> },
    /// The module's own contents failed to parse -- jq's fourth compile-error
    /// kind (#2703). Carries everything [`report_module_load_error`] needs to
    /// build jq's shape: the module's *canonical absolute* path (jq names the
    /// module by it, and canonicalizing turns `/tmp/...` into
    /// `/private/tmp/...`), the module's text for the line echo, and the
    /// parser error holding the byte position. The message wording stays
    /// succinctly's own (jq's `syntax error, unexpected ...` phrasing is
    /// per-parse-state -- see [`report_syntax_error`]), and jq's source-echo
    /// padding is not a fixed formula (the issue's own follow-up note), so
    /// the padding reuses the same column rule the undefined-name path uses.
    Parse {
        path: PathBuf,
        contents: String,
        error: jq::ParseError,
    },
    /// The module could not be read (as opposed to parsed). A *parse*
    /// failure now has its own [`Self::Parse`] variant; this is the residue
    /// of the pre-#2703 opaque `anyhow::Error`.
    Other(anyhow::Error),
}

/// Report a single compile-time syntax error in jq 1.7.1's report shape
/// (#2703), the same family the undefined-name path
/// ([`report_compile_errors`]) builds inline:
///
/// ```text
/// jq: error: unexpected end of input at <top-level>, line 1:
/// 1 +
/// jq: 1 compile error
/// ```
///
/// The `at ...` label is `<top-level>` for the main filter and a module's
/// canonical absolute path for a module-body parse failure. `source` is the
/// text the error lives in and `offset` its byte position; the line the
/// error is on, and the echoed line's trailing padding, come from
/// [`line_at_offset`] -- the "existing reporter's own helper" the issue
/// points Path 2/Path 4 at.
///
/// Two fidelity limits, both recorded in
/// `docs/compliance/jq/limitations.md`:
///
/// - **Wording is succinctly's own, not jq's.** jq's `syntax error,
///   unexpected ...` phrasing -- which token name, which "expecting ..."
///   list, and the `(Unix shell quoting issues?)` suffix -- is a function of
///   its bison parse state, and a lone [`jq::ParseError`] reason cannot
///   reproduce it (`1 +` and `."` are both "end of input" here, but jq
///   distinguishes them, the latter appending a QQSTRING expect-list).
/// - **Padding is the column rule, not a formula.** jq's own
///   `locfile_locate` `%*s` width is not formula-derived (probing `1 +`,
///   `[1,]`, `def f: ;`, `1 2`, `if then`, `?` gives no consistent rule), so
///   this points at the error column and differs from jq, where it differs
///   at all, only in trailing whitespace.
///
/// Whether a blank line sits between the echoed line and the trailer is
/// per-path in real jq (measured live against 1.7.1): a top-level syntax
/// error has none, while a module-body syntax error leaves one. `blank_line`
/// reproduces that.
fn report_syntax_error(
    message: &str,
    source: &str,
    offset: usize,
    location: &str,
    blank_line: bool,
) {
    let (line_no, line_text, column) = line_at_offset(source, offset);
    print_position_error(
        format_args!("{message}"),
        location,
        line_no,
        &line_text,
        column,
    );
    if blank_line {
        eprintln!();
    }
    eprintln!("jq: 1 compile error");
}

/// Print `"{message} at {location}, line {line_no}:"` followed by the
/// offending source line and its caret-padding -- the two-line position+caret
/// shape every "compile error at a known byte position" diagnostic in this
/// file needs ([`report_syntax_error`] and, through [`report_site_error`],
/// every "name is not defined" report), so a caret-format change (added
/// column info, a third line, a different style) has exactly one definition
/// to update. `message` is `std::fmt::Arguments` rather than an owned
/// `String` so building it (often just a name with a sigil glued on) never
/// allocates.
fn print_position_error(
    message: std::fmt::Arguments,
    location: &str,
    line_no: usize,
    line_text: &str,
    column: usize,
) {
    eprintln!("jq: error: {message} at {location}, line {line_no}:");
    eprintln!("{line_text}{}", " ".repeat(column));
}

impl From<anyhow::Error> for ModuleLoadError {
    fn from(e: anyhow::Error) -> Self {
        Self::Other(e)
    }
}

/// Print a [`ModuleLoadError`] the way `run_jq`'s two call sites both need
/// to (#2703): one shared place so the not-found case's jq-matching shape
/// can't drift between them the way the two `eprintln!("jq: module error:
/// {e}")` sites used to have to be kept in step by hand.
fn report_module_load_error(e: &ModuleLoadError) {
    match e {
        ModuleLoadError::NotFound { module_path } => {
            eprintln!("jq: error: module not found: {module_path}");
            eprintln!();
            eprintln!("jq: 1 compile error");
        }
        ModuleLoadError::Cycle { chain } => {
            eprintln!("jq: error: module cycle detected: {}", chain.join(" -> "));
            eprintln!();
            eprintln!("jq: 1 compile error");
        }
        ModuleLoadError::Parse {
            path,
            contents,
            error,
        } => {
            // jq 1.7.1 names the module by its resolved *absolute* path and
            // leaves a blank line between the echoed source and the trailer;
            // both measured live (see [`report_syntax_error`]).
            report_syntax_error(
                &error.message,
                contents,
                error.position,
                &path.display().to_string(),
                true,
            );
        }
        ModuleLoadError::Other(inner) => {
            eprintln!("jq: module error: {inner}");
        }
    }
}

/// A [`build_context`] (or [`get_filter`]) failure that jq treats as a
/// *usage* error -- `--argjson`/`--jsonargs` given a bad value,
/// `--slurpfile`/`--rawfile` given an unreadable file or malformed JSON,
/// `-f`/`--from-file` given an unreadable filter file -- as opposed to some
/// other, unrelated `anyhow::Error` the two might still propagate (#3051,
/// #3096, #3098).
///
/// Distinguished the same way [`ModuleLoadError`] is: an opaque
/// `anyhow::Error` routed through `main`'s own top-level `?` prints a
/// generic two-part `Error: .../Caused by:` block and exits 1, where jq
/// prints one line prefixed `jq:` (plus, for `--argjson`/`--jsonargs`, a
/// usage-hint trailer) and exits 2 (`USAGE_ERROR`) -- each `message` below
/// confirmed live against jq 1.7.1. `message` is pre-formatted by each call
/// site below so [`report_usage_error`] only has to print it.
enum BuildContextError {
    Usage(String),
    Other(anyhow::Error),
}

impl From<anyhow::Error> for BuildContextError {
    fn from(e: anyhow::Error) -> Self {
        Self::Other(e)
    }
}

/// Print a [`BuildContextError::Usage`] message the way `run_jq`'s call
/// site needs to (mirrors [`report_module_load_error`]'s own one-line-vs-
/// scattered-`eprintln!` reasoning): a bare `jq:` prefix, no `error:`
/// (unlike [`report_module_load_error`]'s own compile-error shape) --
/// jq's own usage-error wording never includes it.
fn report_usage_error(message: &str) {
    eprintln!("jq: {message}");
}

/// Strip Rust's `std::io::Error` Display's own `" (os error N)"` suffix,
/// which has no jq equivalent -- jq's C `strerror()` call never appends an
/// errno number. Leaves any other message untouched: the one `std::io::Error`
/// shape with no such suffix to strip is `ErrorKind::InvalidData` (invalid
/// UTF-8 in a `--slurpfile`/`--rawfile` argument, constructed from a plain
/// string rather than an OS errno) -- an unverified-against-jq wording for a
/// failure class real jq's own byte-oriented reader never hits at all (it
/// accepts arbitrary bytes in `--rawfile`; confirmed live), so there is
/// nothing to match there, only to not silently corrupt.
fn strerror_only(e: &std::io::Error) -> String {
    let full = e.to_string();
    match full.rsplit_once(" (os error ") {
        Some((message, _)) => message.to_string(),
        None => full,
    }
}

/// Read `--slurpfile`/`--rawfile`'s file argument, wrapped in jq's own
/// `Bad JSON in --<flag> <name> <file>: Could not open <file>: <detail>`
/// shape on a read failure (#3051, confirmed live for both flags) -- one
/// place so a future wording tweak can't land on one flag and not the
/// other the way the two near-identical inline blocks it replaces could
/// have drifted.
fn read_arg_file(flag: &str, name: &str, file: &str) -> Result<String, BuildContextError> {
    std::fs::read_to_string(file).map_err(|e| {
        BuildContextError::Usage(format!(
            "Bad JSON in --{flag} {name} {file}: Could not open {file}: {}",
            strerror_only(&e)
        ))
    })
}

/// Resolve a module path to a file path within `search_path`.
///
/// A free function rather than a `ModuleLoader` method (#2395). The original
/// reason -- its caller holding a mutable borrow of the module cache across
/// the call -- went away with #2865, which had to drop that `entry()`
/// spelling to make the loader re-entrant; it stays a free function because
/// it reads nothing but the search path, and its caller
/// ([`ModuleLoader::load_and_bind_module`]) does re-enter `&mut self` around
/// it to load the module's own dependencies.
fn resolve_module_in(search_path: &[PathBuf], module_path: &str) -> Option<PathBuf> {
    // #2702: real jq appends `.jq` unconditionally -- `include "m.jq"` looks
    // for `m.jq.jq`, never `m.jq` itself. Confirmed live against jq 1.7.1.
    let module_file = format!("{module_path}.jq");

    // Search in each path
    for base in search_path {
        let full_path = base.join(&module_file);
        if full_path.is_file() {
            return Some(full_path);
        }
    }

    None
}

/// Whether a **data** import's file (`{module_path}.json`) exists anywhere on
/// the search path (#2865).
///
/// `.json`, not `.jq`: a data import reads a JSON file, and the unconditional
/// `.jq` suffix rule (#2702) is a module-import rule. Confirmed live against
/// jq 1.7.1, which resolves `import "data" as $d;` to `data.json` and reports
/// `module not found: data` when there is none.
fn data_file_exists(search_path: &[PathBuf], module_path: &str) -> bool {
    let data_file = format!("{module_path}.json");
    search_path
        .iter()
        .any(|base| base.join(&data_file).is_file())
}

/// Merge one directive's load failure into a "last failing directive"
/// selection, keeping whichever carries the higher [`Import::decl_index`]
/// (#2857).
///
/// jq's module resolution reports the *last* `include`/`import` directive
/// (in true source order) that it cannot honour, not the first: with all of
/// `AAA`, `BBB`, `CCC` missing, `include "AAA"; include "BBB"; include
/// "CCC"` reports `module not found: CCC`. The loader stores the two kinds
/// in separate lists, so the only way to compare an `include` failure
/// against an `import` failure is the shared `decl_index` both carry from
/// the parser. Each load pass feeds every failure it hits through here and
/// returns the survivor at the end, rather than bailing on the first one.
fn keep_last_decl_failure(
    last: &mut Option<(usize, ModuleLoadError)>,
    decl_index: usize,
    e: ModuleLoadError,
) {
    if last.as_ref().map_or(true, |(i, _)| decl_index >= *i) {
        *last = Some((decl_index, e));
    }
}

/// Extract a module's function definitions and stamp every `$__loc__` in
/// each def's body with that module's own canonical (symlink-resolved)
/// path (#2774) -- confirmed live against jq 1.7.1, and the same shape a
/// `resolved_path` reaches this function through either way: an
/// `include`d/`import`ed module (via [`resolve_module_in`]) or `~/.jq`
/// (found directly, never searched for).
///
/// A free function rather than a `ModuleLoader` method for the same reason
/// [`resolve_module_in`] is: it reads nothing off the loader, and its caller
/// re-enters `&mut self` around it.
///
/// `canonicalize` can fail (a module deleted between resolving and reading
/// it, a race no real program depends on); fall back to `resolved_path`
/// as-is rather than failing a load that already succeeded.
fn extract_and_stamp_func_defs(expr: &Expr, resolved_path: PathBuf) -> FuncDefList {
    let canonical_path = std::fs::canonicalize(&resolved_path).unwrap_or(resolved_path);
    let loc_file: std::rc::Rc<str> = canonical_path.to_string_lossy().into();
    extract_func_defs(expr)
        .into_iter()
        .map(|(name, params, body)| (name, params, stamp_loc_file(&body, &loc_file)))
        .collect()
}

/// Wrap `expr` in one `Expr::FuncDef` per entry of `defs`, so that `defs`'
/// **last** entry ends up innermost -- the one jq's innermost-first scoping
/// resolves a name to -- and its first entry outermost.
///
/// One definition of that ordering rule (#2865). It used to be written out
/// three times in [`ModuleLoader::process_program`] (includes, `~/.jq`,
/// imports) and this fix adds a fourth site inside the module loader itself;
/// four copies of "which end wins" is exactly the duplicated-predicate trap
/// `CLAUDE.md` calls out, and the copies are individually correct only by
/// inspection.
fn wrap_defs(mut expr: Expr, defs: FuncDefList) -> Expr {
    for (name, params, body) in defs.into_iter().rev() {
        expr = Expr::FuncDef {
            name,
            params,
            body: Box::new(body),
            then: Box::new(expr),
            bound: FuncDefBound::default(),
        };
    }
    expr
}

/// [`wrap_defs`], with the run bracketed by a begin/end marker pair so the
/// wrapped bodies cannot see anything the chain wraps *around* them (#2951).
///
/// This is the whole loader side of the module-scope boundary. See
/// [`jq::ModuleRun`] for why the boundary is encoded as two extra defs whose
/// names begin with a NUL byte, and [`jq::ModuleRun::parse`]'s callers in
/// `resolve.rs` for the one rule that reads them. A module linked once as
/// another module's dependency is the same shape with a hidden alias
/// (#2955); see [`ModuleLoader::process_program`].
///
/// An empty run is not bracketed: a boundary with nothing inside it can only
/// hide names from the code below, never reveal any, and `~/.jq` is usually
/// absent entirely.
fn wrap_run(expr: Expr, defs: FuncDefList, id: u32, alias: Option<&str>) -> Expr {
    if defs.is_empty() {
        return expr;
    }
    let marker = |name: String| (name, Vec::new(), Expr::Identity);
    let mut bracketed: FuncDefList = Vec::with_capacity(defs.len() + 2);
    bracketed.push(marker(jq::ModuleRun::begin_marker(id, alias)));
    bracketed.extend(defs);
    bracketed.push(marker(jq::ModuleRun::end_marker(id)));
    wrap_defs(expr, bracketed)
}

/// Every function name called anywhere inside `expr`, including inside nested
/// def bodies (#2865).
///
/// **Names only, deliberately not (name, arity).** A module's own source is
/// parsed by plain `jq::parse_program` with no shadow-candidate seeding, so a
/// call site inside it can still be carrying #2036's un-resolved
/// `shadow_fallback`, and such a node holds an empty `args` by construction --
/// its arity reads 0 whatever it really is. Keying on the name alone
/// over-keeps a little (a stub for `g/1` is emitted when only `g/0` is
/// called) where keying on arity could silently *drop* a dependency a call
/// genuinely needs, turning a program that compiles into a compile error.
/// The clash test in [`dep_stubs_for`] still keys on (name, arity), where it
/// has to: there the exact pair is the semantics.
///
/// `any_subexpr` with a predicate that never answers `true` is a full
/// traversal -- the "must not rely on visiting every node" caveat in its doc
/// comment is about short-circuiting, which cannot happen here.
fn called_func_names(expr: &Expr) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    succinctly::jq::walk::any_subexpr(expr, &mut |node| {
        match node {
            Expr::FuncCall {
                name,
                builtin_fallback,
                ..
            } => {
                names.insert(name.clone());
                // `any_subexpr`'s own `FuncCall` arm does not descend into
                // `builtin_fallback`, which is sound only *after*
                // `resolve::check` has run -- and a module's source has just
                // been parsed here, so it has not. A module that defines a
                // builtin's name turns every call to that builtin into a
                // shadowable-call node whose real sub-expressions live in the
                // fallback with `args` left empty, so skipping it hides them:
                // `def limit: "s"; def h: [limit(1; g)];` would not see `g` at
                // all and would drop the dependency that defines it, turning a
                // program jq compiles into a compile error.
                if let Some(fallback) = builtin_fallback {
                    names.extend(called_func_names(fallback));
                }
            }
            Expr::NamespacedCall {
                namespace, name, ..
            } => {
                names.insert(format!("{namespace}::{name}"));
            }
            // A destructuring pattern's computed key (`. as {(kf): $v} | $v`,
            // #2734) no longer needs a separate walk here -- `any_subexpr`
            // descends `patterns` itself (#3017), so a call reached only
            // from such a key is already visited by this same traversal.
            _ => {}
        }
        false
    });
    names
}

/// The forwarding stubs one of a module's own defs is wrapped in for one of
/// its dependency groups, in [`wrap_defs`] order (#2955): one per dependency
/// (name, arity) the body calls directly, each a def of that name and arity
/// whose body calls the dependency where it was linked, by its
/// [`jq::ModuleRun::link_name`], forwarding every parameter as written.
///
/// `group` is one origin module's signatures, linked as run `run` (with
/// `alias` when it is an `import`, so its stubs are `alias::name`, exactly
/// as the run's own defs would be named at the top level). `called` is every
/// name the def's own body calls, as written.
///
/// ### Why stubs, not copies
///
/// Before #2955 the dependency *bodies* were copied into every def that
/// reached them, each copy carrying its own module's dependencies in turn, so
/// a chain of modules compounded as the fan-out to the power of the depth.
/// A stub is a fixed size, and the body it forwards to exists once. Nothing in
/// a stub is lexable: no free name a consumer could capture (#2962's whole
/// family), no `$variable`, no `$__loc__`, so it needs no run of its own.
///
/// ### Direct calls only
///
/// A dependency's own free names -- its module's siblings, and the modules
/// *it* includes -- are its own run's business: they resolve there, under the
/// run's alias retry and its stubs, so the transitive closure the copying
/// loader had to compute is gone. What remains is jq's `block_bind_referenced`
/// rule at the call site: a def's body is wrapped only in what it names, and
/// dropping an unreferenced dependency is unobservable, since nothing resolves
/// to it and jq agrees an unreferenced dependency whose own body calls an
/// undefined function is an error in neither tool.
///
/// **Names only, not (name, arity)**: `called` comes from
/// [`called_func_names`], so a called name gets a stub at every arity the
/// group defines it at. The resolver picks the right one; an unused stub
/// costs one node and can capture nothing.
///
/// ### Dependencies only -- a module's own siblings are deliberately absent
///
/// A module's defs are emitted as siblings in the chain they are exported
/// into, exactly as a filter's own defs are, so they already see each other
/// there with jq's lexical rule. Nesting a copy of one inside another's body
/// instead puts it under scopes it was never written in, and anything free
/// in it is then captured by them -- not just module-level names, but
/// **builtins**:
///
/// ```text
/// def a: length;                 jq: [1,2] | h(9) is 2
/// def h(length): a;              nested: 9, the parameter captured a's call
///
/// def a: b;                      jq: b/0 is not defined (exit 3)
/// def h(b): a;                   nested: 99, the error silently swallowed
/// ```
///
/// ### Two names never get a stub
///
/// The stubs are wrapped *inside* the def, so a stub would shadow two of the
/// def's own bindings for the body:
///
/// - **The def's own (name, arity).** jq binds a def's own recursive call to
///   itself: with `inner`'s `g/0` in scope, `def g: if . == 0 then "base"
///   else (. - 1 | g) end;` answers `"base"`.
/// - **A parameter's name, at arity 0.** A parameter binds the bare call-site
///   namespace (`Param::Dollar`'s `$g` binds `g` too) and wins: `def f(g): g;
///   def q: f(7);` answers `7` in jq even with a dependency `g` in scope.
///
/// The body's own calls to such a name are therefore never the dependency's.
/// Another dependency's calls to it are answered inside that dependency's
/// own run, where the clash does not exist -- with `inner.jq` = `def c: 7;
/// def g: c;` and `mid.jq` = `include "inner"; def c: if . == 0 then g else
/// (. - 1 | c) end;`, `0 | c` is `7`, and `g`'s `c` is `inner`'s because `g`
/// was linked next to it. The copying loader had to *rename* such a
/// dependency to get that row right (#2962); linking makes the rename moot.
fn dep_stubs_for(
    group: &[(String, Vec<Param>)],
    run: u32,
    alias: Option<&str>,
    name: &str,
    params: &[Param],
    called: &BTreeSet<String>,
) -> FuncDefList {
    let param_names: BTreeSet<&str> = params.iter().map(Param::name).collect();
    let arity = params.len();
    let mut seen: BTreeSet<(String, usize)> = BTreeSet::new();
    let mut stubs: FuncDefList = Vec::new();
    for (dep_name, dep_params) in group {
        let stub_name = match alias {
            Some(alias) => format!("{alias}::{dep_name}"),
            None => dep_name.clone(),
        };
        // A qualified `alias::name` call can never collide with the
        // consuming def's own bare (name, arity) or a bare parameter name --
        // only an `include`d (unaliased) dependency's bare spelling can.
        let clashes = alias.is_none()
            && ((dep_name == name && dep_params.len() == arity)
                || (dep_params.is_empty() && param_names.contains(dep_name.as_str())));
        if clashes || !called.contains(&stub_name) {
            continue;
        }
        // Two same-(name, arity) entries in one module are one export -- the
        // later one, innermost in its own run -- so one stub serves both.
        if !seen.insert((stub_name.clone(), dep_params.len())) {
            continue;
        }
        stubs.push(forwarding_stub(
            stub_name,
            dep_params,
            jq::ModuleRun::link_name(run, dep_name),
        ));
    }
    stubs
}

/// `def <name>(p1; ...; pn): <target>(p1; ...; pn);` -- a def that forwards
/// every argument to `target` as a closure (#2955).
///
/// The parameters are the dependency's own names, spelt bare even where the
/// dependency spells them `$p`: a bare parameter forwards the caller's
/// argument expression unevaluated, and the dependency does its own `$p`
/// binding on arrival, exactly as it would for a direct call. A parameter
/// named like the target cannot clash with it -- the
/// target is a NUL-prefixed link name -- and one named like the stub itself
/// binds only at arity 0, while the forwarded call is at the stub's arity.
fn forwarding_stub(name: String, params: &[Param], target: String) -> (String, Vec<Param>, Expr) {
    let forwarded: Vec<Param> = params
        .iter()
        .map(|p| Param::Bare(p.name().to_string()))
        .collect();
    let args = forwarded
        .iter()
        .map(|p| Expr::FuncCall {
            name: p.name().to_string(),
            args: Vec::new(),
            builtin_fallback: None,
        })
        .collect();
    let body = Expr::FuncCall {
        name: target,
        args,
        builtin_fallback: None,
    };
    (name, forwarded, body)
}

/// `path` resolved through the filesystem, falling back to `path` itself.
///
/// `canonicalize` can fail (a module deleted between resolving and reading it,
/// a race no real program depends on); a non-canonical key only weakens the
/// cycle guard's aliasing coverage, it never turns a legal program into an
/// error, so falling back is strictly better than failing a load that already
/// succeeded.
fn canonical_or_self(path: &std::path::Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

impl ModuleLoader {
    /// Create a new module loader with the given search paths.
    pub fn new(library_paths: &[PathBuf]) -> Self {
        let mut search_path = Vec::new();
        let mut auto_loaded_defs = Vec::new();

        // Add command-line -L paths first (highest priority)
        for path in library_paths {
            if path.is_dir() {
                search_path.push(path.clone());
            }
        }

        // Add JQ_LIBRARY_PATH environment variable paths
        if let Ok(jq_lib_path) = std::env::var("JQ_LIBRARY_PATH") {
            for path_str in jq_lib_path.split(':') {
                let path = PathBuf::from(path_str);
                if path.is_dir() {
                    search_path.push(path);
                }
            }
        }

        // Handle ~/.jq - can be either a file or directory
        let mut run_origins: BTreeMap<u32, String> = BTreeMap::new();
        if let Some(home) = std::env::var_os("HOME") {
            let jq_path = PathBuf::from(home).join(".jq");
            if jq_path.is_file() {
                // Auto-load ~/.jq file - functions defined here are always available
                if let Ok(contents) = std::fs::read_to_string(&jq_path) {
                    if let Ok(program) = jq::parse_program(&contents) {
                        // #2774: same stamping as an `include`d module's
                        // defs, using `~/.jq`'s own canonical path.
                        // Run id 0 is reserved for `~/.jq` (#2951): it is
                        // wrapped as its own run like any module, so its
                        // bodies cannot see an `include`d or `import`ed name
                        // either -- real jq keeps both out.
                        run_origins.insert(
                            AUTO_LOAD_RUN_ID,
                            canonical_or_self(&jq_path).to_string_lossy().into_owned(),
                        );
                        auto_loaded_defs = extract_and_stamp_func_defs(&program.expr, jq_path);
                    }
                }
            } else if jq_path.is_dir() {
                // Add ~/.jq directory to search path
                search_path.push(jq_path);
            }
        }

        Self {
            search_path,
            loaded_modules: BTreeMap::new(),
            auto_loaded_defs,
            loading: Vec::new(),
            run_ids: BTreeMap::new(),
            run_origins,
            next_run_id: AUTO_LOAD_RUN_ID + 1,
            deps_of: BTreeMap::new(),
        }
    }

    /// The run id for `module_path`, interned by canonical file so the same
    /// module included and imported shares one id (#2951).
    ///
    /// Falls back to the path as written when the module cannot be resolved:
    /// the caller is about to fail the load anyway, and a run id is only ever
    /// a diagnostic key, never a correctness one -- `scan_scope` pairs a
    /// begin with an end by nesting, not by id.
    fn run_id_for(&mut self, module_path: &str) -> u32 {
        let key = self.run_key(module_path);
        if let Some(&id) = self.run_ids.get(&key) {
            return id;
        }
        let id = self.next_run_id;
        self.next_run_id += 1;
        self.run_ids.insert(key.clone(), id);
        self.run_origins.insert(id, key);
        id
    }

    /// The key [`Self::run_id_for`] interns `module_path` under: its
    /// canonical file, or the path as written when it cannot be resolved.
    fn run_key(&self, module_path: &str) -> String {
        resolve_module_in(&self.search_path, module_path).map_or_else(
            || module_path.to_string(),
            |p| canonical_or_self(&p).to_string_lossy().into_owned(),
        )
    }

    /// The canonical file a run id names, for compile-error attribution.
    #[must_use]
    pub fn run_origin(&self, id: u32) -> Option<&str> {
        self.run_origins.get(&id).map(String::as_str)
    }

    /// Load a module if it is not already cached, and borrow its function
    /// definitions (name, params, body) in place.
    ///
    /// The borrowing form exists so a caller that only needs to *read* the
    /// defs -- [`Self::unqualified_def_names`], which wants their names --
    /// does not pay for a deep clone of every def body it is about to drop
    /// (#2395). [`Self::load_module`] is this plus that clone, for callers
    /// that need an owned copy.
    ///
    /// The defs handed back are the module's **own** defs only, each with
    /// forwarding stubs for its module's dependencies already wrapped around
    /// its body (#2865, #2955) -- so a transitively included name is visible
    /// to the module that included it and to nobody else. See
    /// [`Self::load_and_bind_module`] for why that shape, rather than
    /// splicing the dependencies into the exported chain, is what real jq
    /// does.
    fn ensure_module_loaded(&mut self, module_path: &str) -> Result<&FuncDefList, ModuleLoadError> {
        // Keyed by the canonical file (#2955), not `module_path` as written:
        // two spellings of the same module (`"dep"`, `"./dep"`) must share one
        // cache entry, or each distinct spelling pays its own full
        // parse-and-bind pass. `run_key` is the same canonicalize-or-fall-back
        // helper `run_id_for` interns run ids by, so this map and `run_ids`
        // agree on what "the same module" means.
        let key = self.run_key(module_path);

        // `contains_key` -> load -> `insert` -> re-`get`, rather than the
        // `entry` spelling this used to have (#2865). `entry` holds a mutable
        // borrow of `loaded_modules` across the whole load, which the loader
        // now has to be able to re-enter to pull in the module's own
        // dependencies -- so the `expect` on the re-lookup that the old doc
        // comment here was written to avoid comes back, as the cost of
        // recursion being possible at all. It is genuinely unreachable: the
        // insert immediately above it is unconditional on this path.
        if !self.loaded_modules.contains_key(&key) {
            let defs = self.load_and_bind_module(module_path)?;
            self.loaded_modules.insert(key.clone(), defs);
        }

        Ok(self
            .loaded_modules
            .get(&key)
            .expect("just inserted above, or already present"))
    }

    /// Read, parse and bind one module: its own defs, each wrapped in
    /// forwarding stubs for whatever its own `include`/`import` directives
    /// bring into scope (#2865, #2955).
    ///
    /// ### Why wrap each *body* rather than splice into the exported chain
    ///
    /// The issue's own suggested fix ("apply `process_program` recursively")
    /// gets three of jq's rows wrong at once. Wrapping the bodies buys all of
    /// them from one mechanism, with no special-casing (every row captured
    /// live against jq 1.7.1, fixtures `inner.jq` = `def g: 42;`):
    ///
    /// - `include "outer"; h` where `outer.jq` is `include "inner"; def h: g;`
    ///   answers `42` -- the dependency is visible inside the module.
    /// - `include "outer"; g` is a **compile error**: the dependency is *not*
    ///   re-exported to the includer, which splicing into the chain would do.
    /// - where the module is `include "inner"; def g: 7; def h: g;`, `h`
    ///   answers **`42`, not `7`** -- the dependency is innermost, so it beats
    ///   the module's own same-name sibling. (At the top level the same
    ///   collision goes the *other* way, since a filter's own defs bind at
    ///   parse time; that row already passes and is unchanged.)
    /// - ...while that module's own `g` is still what it exports: `7`.
    ///
    /// ### What each body is wrapped in
    ///
    /// [`dep_stubs_for`] decides that per def and per origin group: one
    /// forwarding stub per dependency (name, arity) the body calls directly,
    /// except the def's own name and its parameters (#2962). The dependency
    /// bodies themselves are not here at all: each dependency module is
    /// linked once, as a run of its own, by [`Self::process_program`], and
    /// the stubs call into it by an unlexable name. That is what keeps a
    /// chain of modules linear in size (#2955), and it is also what binds a
    /// dependency's own free names where jq binds them -- inside its own
    /// module -- rather than inside whichever def happened to copy it. The
    /// module's *own* defs are deliberately not wrapped in either -- they
    /// are emitted as siblings in the chain they are exported into, where
    /// jq's lexical rule already relates them. That function's doc comment
    /// carries the oracle row behind each rule.
    ///
    /// ### Search-path resolution
    ///
    /// A nested `include` resolves against the global search path only, never
    /// the including module's own directory -- `sub/usedeep.jq` saying
    /// `include "deep"` reports `module not found: deep` even with `deep.jq`
    /// sitting next to it. So reusing the search path verbatim is both the
    /// simple implementation and the faithful one.
    fn load_and_bind_module(&mut self, module_path: &str) -> Result<FuncDefList, ModuleLoadError> {
        // Resolve the module path
        let file_path = resolve_module_in(&self.search_path, module_path).ok_or_else(|| {
            ModuleLoadError::NotFound {
                module_path: module_path.to_string(),
            }
        })?;

        let canonical = canonical_or_self(&file_path);
        if let Some(at) = self.loading.iter().position(|(seen, _)| *seen == canonical) {
            // From the repeat, not from the bottom of the stack: the modules
            // that merely *led* to the cycle are not part of it, and naming
            // them makes the chain read as a longer cycle than it is. Loading
            // `x` (which includes `ca`, which includes `cb`, which includes
            // `ca`) reports `ca -> cb -> ca`, not `x -> ca -> cb -> ca`.
            let mut chain: Vec<String> = self.loading[at..]
                .iter()
                .map(|(_, as_written)| as_written.clone())
                .collect();
            chain.push(module_path.to_string());
            return Err(ModuleLoadError::Cycle { chain });
        }

        // Read and parse the module
        let contents = std::fs::read_to_string(&file_path)
            .with_context(|| format!("failed to read module: {}", file_path.display()))?;

        let program = jq::parse_program(&contents).map_err(|e| ModuleLoadError::Parse {
            path: canonical.clone(),
            contents,
            error: e,
        })?;

        // Stamp `$__loc__` BEFORE wrapping, never after: `stamp_loc_file`
        // overwrites `file` unconditionally, so a wrap-then-stamp order would
        // silently re-stamp an inner module's already-correct `$__loc__` with
        // *this* module's path, regressing #2774.
        let own = extract_and_stamp_func_defs(&program.expr, file_path);

        let own_id = self.run_id_for(module_path);
        self.loading.push((canonical, module_path.to_string()));
        let deps = self.module_dep_defs(&program, own_id);
        self.loading.pop();
        let deps = deps?;

        // Each def wrapped in stubs for the dependencies it names. The
        // module's own defs are left to the chain it is exported into -- see
        // [`dep_stubs_for`] for why nesting them here is unsound.
        Ok(own
            .into_iter()
            .map(|(name, params, body)| {
                // Groups keep declaration order, and each is wrapped in turn
                // so the last-declared ends up innermost, exactly as the flat
                // `wrap_defs` did before the grouping: `include "pa";
                // include "pb";` with both defining `foo` resolves `foo` to
                // `pb`'s, as at the top level.
                let called = called_func_names(&body);
                let mut wrapped = body;
                for (origin, alias, group) in deps.iter().rev() {
                    let stubs =
                        dep_stubs_for(group, *origin, alias.as_deref(), &name, &params, &called);
                    wrapped = wrap_defs(wrapped, stubs);
                }
                (name, params, wrapped)
            })
            .collect())
    }

    /// The signature of every def one module's own `include`/`import`
    /// directives bring into that module's scope, in [`wrap_defs`] order
    /// (last entry innermost, so last-declared wins), recording on the way
    /// that `own_id` depends on each of them (#2955).
    ///
    /// Declaration order is what produces that: a later `include` is appended
    /// later, so it lands nearer the end and therefore nearer the body --
    /// matching jq, where a module with `include "pa"; include "pb";` and both
    /// defining `foo` resolves `foo` to `pb`'s. That is the same rule
    /// [`Self::process_program`] documents for the top level, which is why
    /// both go through `wrap_defs`.
    ///
    /// `~/.jq` is deliberately **not** included here: its defs are not visible
    /// inside a module body in real jq (`def uh: hj;` in a module reports
    /// `hj/0 is not defined` even with `def hj: 1234;` in `~/.jq`). They still
    /// leak in today through the top-level chain -- a separate, pre-existing
    /// gap this fix neither widens nor closes.
    fn module_dep_defs(
        &mut self,
        program: &Program,
        own_id: u32,
    ) -> Result<DepGroups, ModuleLoadError> {
        let mut defs: DepGroups = Vec::new();

        for include in &program.includes {
            let id = self.run_id_for(&include.path);
            let sigs = self.dependency_signatures(&include.path, id, own_id, include.decl_index)?;
            defs.push((id, None, sigs));
        }

        // A module `import`ed by a module keeps its bare signatures here;
        // [`dep_stubs_for`] spells the stubs `ns::name`, as `process_program`
        // spells a top-level import's defs, and leaves them for the single
        // `rewrite_namespaced_calls` pass at the end of `process_program`,
        // which rewrites the `NamespacedCall`s inside module bodies along
        // with every other one.
        //
        // A *data* import (`import "f" as $d;`, which binds a `$`-variable to
        // the file's parsed JSON rather than a namespace of defs) contributes
        // no defs, and resolving one as a module reports `module not found`
        // where jq reads `<path>.json`. Before #2865 a module's own imports
        // were never looked at, so declaring one was harmless; processing them
        // transitively made it fatal. `Import::data` (#2865) is what separates
        // the two -- skipping on *resolvability* instead would have silently
        // swallowed a genuinely missing module, which jq reports and exits 3
        // for. Data imports themselves remain unimplemented (#2956).
        for import in &program.imports {
            if import.data {
                // A data import contributes no defs, but jq still *resolves*
                // it: with no `nodatafile.json` anywhere on the search path,
                // `import "nodatafile" as $d;` reports `module not found:
                // nodatafile` and exits 3, exactly as a missing module does.
                // Check the same thing so a typo is not silently swallowed,
                // without implementing the binding itself (#2956).
                if !data_file_exists(&self.search_path, &import.path) {
                    return Err(ModuleLoadError::NotFound {
                        module_path: import.path.clone(),
                    });
                }
                continue;
            }
            let id = self.run_id_for(&import.path);
            let sigs = self.dependency_signatures(&import.path, id, own_id, import.decl_index)?;
            defs.push((id, Some(import.alias.clone()), sigs));
        }

        Ok(defs)
    }

    /// Load `module_path` as a dependency of the module with run id `own_id`
    /// and borrow out its defs' signatures (#2955): the names and parameters
    /// the stubs are built from. The bodies stay in the cache, to be linked
    /// once by [`Self::process_program`], which is what this also records:
    /// that `id` needs a link run, and that `own_id` depends on it at
    /// `decl_index` -- the position [`Self::hoist_order`] sorts by, since
    /// `own_id`'s dependencies are recorded include-block-then-import-block
    /// here (see [`Self::module_dep_defs`]), not in true source order.
    fn dependency_signatures(
        &mut self,
        module_path: &str,
        id: u32,
        own_id: u32,
        decl_index: usize,
    ) -> Result<Vec<(String, Vec<Param>)>, ModuleLoadError> {
        let sigs = self
            .ensure_module_loaded(module_path)?
            .iter()
            .map(|(name, params, _)| (name.clone(), params.clone()))
            .collect();
        self.deps_of
            .entry(own_id)
            .or_default()
            .push((decl_index, id));
        Ok(sigs)
    }

    /// The modules that need a link run (#2955), outermost first: a module
    /// that some other module depends on, placed outside everything that
    /// depends on it, each once, a diamond included.
    ///
    /// The walk visits each module's directives in **reverse** declaration
    /// order and records a module after its own dependencies, so the runs are
    /// wrapped -- and their bodies compiled, and any compile errors in them
    /// reported -- in the order jq 1.7.1 reports them: a dependency's errors
    /// before its includer's, the last-declared dependency's first, and a
    /// chain's deepest module first (captured live for all three shapes). A
    /// module that is also a top-level `include`/`import` is walked for its
    /// dependencies but not recorded for itself unless something depends on
    /// it: its top-level run already carries its defs.
    ///
    /// `top_ids` is each top-level `include`/non-data `import`'s
    /// `(decl_index, run id)`, exactly as [`Self::process_program`] already
    /// resolved them via [`Self::run_id_for`] while wrapping their runs --
    /// passed in rather than re-derived from `Program`'s paths so this does
    /// not repeat the same `resolve_module_in`/`canonicalize` filesystem
    /// lookups a second time for every call.
    fn hoist_order(&self, top_ids: &[(usize, u32)]) -> Vec<u32> {
        fn visit(
            loader: &ModuleLoader,
            id: u32,
            as_dependency: bool,
            expanded: &mut BTreeSet<u32>,
            recorded: &mut BTreeSet<u32>,
            order: &mut Vec<u32>,
        ) {
            if expanded.insert(id) {
                if let Some(deps) = loader.deps_of.get(&id) {
                    // Sorted here rather than relying on `deps`' insertion
                    // order: `dependency_signatures` records a module's
                    // includes before its imports (see `module_dep_defs`),
                    // which is not source order when the two are
                    // interleaved. `decl_index` is the true order; last
                    // declared first, same rule as `top_ids` below.
                    let mut deps = deps.clone();
                    deps.sort_by_key(|(decl, _)| core::cmp::Reverse(*decl));
                    for (_, dep) in deps {
                        visit(loader, dep, true, expanded, recorded, order);
                    }
                }
            }
            if as_dependency && recorded.insert(id) {
                order.push(id);
            }
        }

        // Last declared first, exactly as their runs nest (`process_program`
        // wraps the last-declared include innermost).
        let mut top: Vec<(usize, u32)> = top_ids.to_vec();
        top.sort_by_key(|(decl, _)| core::cmp::Reverse(*decl));

        let mut expanded = BTreeSet::new();
        let mut recorded = BTreeSet::new();
        let mut order = Vec::new();
        for (_, id) in top {
            visit(self, id, false, &mut expanded, &mut recorded, &mut order);
        }
        order
    }

    /// Load a module and return an owned copy of its function definitions
    /// (name, params, body).
    pub fn load_module(&mut self, module_path: &str) -> Result<FuncDefList, ModuleLoadError> {
        self.ensure_module_loaded(module_path).cloned()
    }

    /// Every def name this program's modules will put into the main filter's
    /// scope *unqualified*, for seeding the parser's shadow-candidate set
    /// (#2395).
    ///
    /// That is `~/.jq`'s own defs plus every `include`d module's defs --
    /// exactly the two sources [`Self::process_program`] inlines under their
    /// bare names. `program.imports` is deliberately excluded: those defs are
    /// inlined as `ns::name`, and real jq agrees they do not shadow
    /// (`import "m" as m; length` is the builtin `length` even when the module
    /// defines one; only `m::length` reaches the module's).
    ///
    /// Loading here rather than in `process_program` costs nothing:
    /// [`Self::ensure_module_loaded`] memoizes on the module path and this
    /// only borrows the names out of it, so the later `process_program`
    /// re-reads and re-parses nothing. A module that cannot be resolved fails
    /// here instead of there, one step earlier but still after the main filter
    /// has parsed, so the caller's error and exit code are unchanged.
    pub fn unqualified_def_names(
        &mut self,
        program: &Program,
    ) -> Result<BTreeSet<String>, ModuleLoadError> {
        let mut names: BTreeSet<String> = self
            .auto_loaded_defs
            .iter()
            .map(|(name, _, _)| name.clone())
            .collect();

        // #2857: a failing directive must not short-circuit the pass here --
        // jq reports the *last* unresolvable `include`/`import` in source
        // order, so every directive gets a chance to load (into the
        // `loaded_modules` memo, which `process_program` then reuses) and the
        // failures are kept only if they are the latest seen. Only
        // `includes` contribute *names*; `imports` are probed purely for
        // their failures so the whole program, not just the include block,
        // decides what gets reported.
        let mut last_err: Option<(usize, ModuleLoadError)> = None;
        for include in &program.includes {
            match self.ensure_module_loaded(&include.path) {
                Ok(defs) => names.extend(defs.iter().map(|(name, _, _)| name.clone())),
                Err(e) => keep_last_decl_failure(&mut last_err, include.decl_index, e),
            }
        }
        for import in &program.imports {
            // Same data-import split the real loading path uses (#2865):
            // a data import reads `{path}.json`, not `{path}.jq`.
            if import.data {
                if !data_file_exists(&self.search_path, &import.path) {
                    keep_last_decl_failure(
                        &mut last_err,
                        import.decl_index,
                        ModuleLoadError::NotFound {
                            module_path: import.path.clone(),
                        },
                    );
                }
                continue;
            }
            if let Err(e) = self.ensure_module_loaded(&import.path) {
                keep_last_decl_failure(&mut last_err, import.decl_index, e);
            }
        }

        match last_err {
            Some((_, e)) => Err(e),
            None => Ok(names),
        }
    }

    /// Process imports and includes, returning the modified expression with all functions defined.
    pub fn process_program(&mut self, program: &Program) -> Result<Expr, ModuleLoadError> {
        let mut expr = program.expr.clone();

        // #2857: as in `unqualified_def_names`, a failing directive must not
        // short-circuit here -- jq reports the *last* unresolvable directive
        // in source order, across both kinds. Every directive is still given
        // its turn to load (the ones that fail simply wrap nothing), failures
        // are merged against the running last via `decl_index`, and the
        // survivor is returned after both loops. On success the wrapping
        // order below is bit-for-bit what it was before this issue.

        let mut last_err: Option<(usize, ModuleLoadError)> = None;
        // Every top-level `include`/non-data `import`'s (decl_index, run id),
        // collected as each is resolved below so `link_dependency_runs` ->
        // `hoist_order` can place them without re-resolving the same paths
        // through the filesystem a second time.
        let mut top_ids: Vec<(usize, u32)> = Vec::new();

        // #2682: each `expr = FuncDef { .., then: expr }` wraps the *previous*
        // `expr` one layer further in, so whichever source is processed
        // FIRST ends up nearest the original body -- the innermost def, and
        // therefore the one jq's own innermost-first scoping resolves a
        // name to. The two facts this loop order has to get right, both
        // confirmed live against jq 1.7.1:
        //
        // - `include` outranks `~/.jq`: a name defined in both resolves to
        //   the `include`d one (`~/.jq`'s own def is still visible, and
        //   still outranks the same name in a module that was never
        //   included -- just not one that was).
        // - among multiple `include`s defining the same name, the *last*
        //   declared one wins, not the first -- `include "pa"; include
        //   "pb"; foo` answers `pb`'s `foo`, and reversing the two
        //   `include`s reverses the answer.
        //
        // Hence `.rev()` below (last-declared include processed first, so
        // it ends up innermost) and the whole includes block running before
        // the `~/.jq` block that follows it.
        for include in program.includes.iter().rev() {
            let defs = match self.load_module(&include.path) {
                Ok(defs) => defs,
                Err(e) => {
                    keep_last_decl_failure(&mut last_err, include.decl_index, e);
                    continue;
                }
            };
            // #2951: bracketed as a run, so these defs' own bodies cannot
            // see the sibling `include`s and `~/.jq` block wrapped around
            // them. Before this, `def sa: sb;` in one module resolved `sb`
            // from an unrelated module that merely happened to be included
            // first -- and reversing the two `include`s made it agree with
            // jq again, by accident.
            let id = self.run_id_for(&include.path);
            top_ids.push((include.decl_index, id));
            expr = wrap_run(expr, defs, id, None);
        }

        // `~/.jq`'s own defs: lowest priority of the two unqualified
        // sources (loses to any `include`d module of the same name, but
        // still beats a name that was never `include`d at all).
        expr = wrap_run(expr, self.auto_loaded_defs.clone(), AUTO_LOAD_RUN_ID, None);

        // Process imports (definitions available under namespace::)
        // Load modules and add their functions with namespace prefixes
        for import in &program.imports {
            // The same data-import handling the module path uses (#2865), so
            // the two agree: a data import binds a `$`-variable rather than a
            // namespace of defs, so it contributes nothing here, but jq still
            // resolves its `<path>.json` and reports `module not found` when
            // there is none. Binding the variable itself is #2956.
            if import.data {
                if !data_file_exists(&self.search_path, &import.path) {
                    keep_last_decl_failure(
                        &mut last_err,
                        import.decl_index,
                        ModuleLoadError::NotFound {
                            module_path: import.path.clone(),
                        },
                    );
                }
                continue;
            }
            let defs = match self.load_module(&import.path) {
                Ok(defs) => defs,
                Err(e) => {
                    keep_last_decl_failure(&mut last_err, import.decl_index, e);
                    continue;
                }
            };
            let namespace = &import.alias;

            // Add each function with a namespaced name (namespace::funcname)
            let defs = defs
                .into_iter()
                .map(|(name, params, body)| (format!("{namespace}::{name}"), params, body))
                .collect();
            // The alias rides the begin marker (#2989): only the defs of
            // *this* run were namespaced, so a bare sibling call inside one
            // of their bodies is retried as `alias::name` by the resolver,
            // and only within this run.
            let id = self.run_id_for(&import.path);
            top_ids.push((import.decl_index, id));
            expr = wrap_run(expr, defs, id, Some(namespace));
        }

        if let Some((_, e)) = last_err {
            return Err(e);
        }

        expr = self.link_dependency_runs(program, &top_ids, expr);

        // Transform NamespacedCall expressions to regular FuncCall expressions
        expr = rewrite_namespaced_calls(expr);

        Ok(expr)
    }

    /// Wrap `expr` in one link run per module some module depends on
    /// (#2955): each such module's defs, once, under their
    /// [`jq::ModuleRun::link_name`]s, in a run whose alias is the module's
    /// [`jq::ModuleRun::link_alias`], outermost of everything -- outside the
    /// imports, `~/.jq` and includes already wrapped around `expr` -- and in
    /// [`Self::hoist_order`], so every module sits outside the modules that
    /// depend on it.
    ///
    /// A consuming def reaches a linked def through the forwarding stub
    /// [`dep_stubs_for`] wrapped into its body; a linked def's own bare calls
    /// to its siblings miss the link names, floor at the run's begin marker,
    /// and are retried under the alias by the resolver, exactly as a bare
    /// sibling call inside an `import`ed module is (#2989). The names are
    /// unlexable, so the run exports nothing: `include "mid"; g` where only
    /// `mid`'s dependency defines `g` is still `g/0 is not defined`.
    ///
    /// ### Only what is referenced is linked
    ///
    /// jq's `block_bind_referenced` rule, applied per module: a def is
    /// emitted only if some body that is itself reached names it -- a stub
    /// in a module that depends on this one, or a kept sibling of its own.
    /// "Reached" is by name from the main filter outward: the filter's own
    /// calls, then the top-level runs' defs those name and everything *they*
    /// name, and only then each linked module, dependents first (the reverse
    /// of the wrapping order), so it is a single pass. Dropping the rest is
    /// unobservable, since nothing resolves to a def nobody reached names --
    /// the resolver never checks an unreached body either (#2740). What it
    /// buys is that a large utility module used for one function costs one
    /// def in the chain, not all of them, and a wide module chain of which
    /// the filter uses one def costs one def per level: every chain def is
    /// installed over the whole program below it at evaluation, so the
    /// chain's length is the cost that matters (seeding from *every*
    /// top-level def instead measured 22 MB against 12 MB for a six-level
    /// forty-def chain the filter walks one def of).
    ///
    /// Every same-name entry is kept together, in declaration order, so the
    /// innermost-first rule among them is the module's own (`def c: 7; def
    /// c: 8; def g: c;` exports `g` as 8, and `def c: 7; def g: c; def c: 8;
    /// def k: [g, c];` as `[7, 8]`, both as jq answers).
    fn link_dependency_runs(
        &self,
        program: &Program,
        top_ids: &[(usize, u32)],
        mut expr: Expr,
    ) -> Expr {
        let order = self.hoist_order(top_ids);
        if order.is_empty() {
            return expr;
        }

        // Every name reached so far, as the reaching call spells it: bare
        // for an `include`d or `~/.jq` def, `alias::name` for an `import`ed
        // one, and a link name once a stub is reached. Seeded from the main
        // filter, then closed over the top-level runs' defs -- all of which
        // are emitted regardless; this only decides what they pull in.
        //
        // An `import`ed def calls its siblings bare, and the resolver retries
        // that call as `alias::name` (#2989); the closure has to retry it the
        // same way, or a sibling reached only that way looks unreached and
        // what *it* depends on is never linked (a compile error naming the
        // missing link, in a program jq runs).
        //
        // This is the top-level twin of the per-linked-module fixed point
        // below: both share `grow_to_fixed_point` for the "grow until
        // nothing new resolves" iteration itself, but what a visit *does*
        // still differs, over a different candidate list (`top`'s
        // alias-qualified entries here, `defs`'s bare-named ones there)
        // because a module can be imported under several different aliases
        // at the top level but a hoisted link run is keyed by one globally
        // unique id. A retry rule fixed in one almost certainly needs the
        // same fix in the other.
        let mut wanted: BTreeSet<String> = called_func_names(&program.expr);
        let mut top: Vec<(String, &Expr, Option<&str>, bool)> = Vec::new();
        for include in &program.includes {
            // `loaded_modules` is keyed canonically (#2955); `include.path` is
            // the literal spelling as written, so it has to go through
            // `run_key` the same way `ensure_module_loaded` does, or a
            // spelling that differs from whichever one populated the cache
            // would miss here even though the module is loaded.
            if let Some(defs) = self.loaded_modules.get(&self.run_key(&include.path)) {
                top.extend(defs.iter().map(|(n, _, b)| (n.clone(), b, None, false)));
            }
        }
        for import in program.imports.iter().filter(|i| !i.data) {
            if let Some(defs) = self.loaded_modules.get(&self.run_key(&import.path)) {
                let ns = import.alias.as_str();
                top.extend(
                    defs.iter()
                        .map(|(n, _, b)| (format!("{ns}::{n}"), b, Some(ns), false)),
                );
            }
        }
        top.extend(
            self.auto_loaded_defs
                .iter()
                .map(|(n, _, b)| (n.clone(), b, None, false)),
        );
        grow_to_fixed_point(top.len(), |i| {
            let (name, body, alias, kept) = &mut top[i];
            if *kept || !wanted.contains(name.as_str()) {
                return false;
            }
            *kept = true;
            let before = wanted.len();
            for called in called_func_names(body) {
                if let Some(alias) = alias {
                    if !called.contains("::") {
                        wanted.insert(format!("{alias}::{called}"));
                    }
                }
                wanted.insert(called);
            }
            wanted.len() != before
        });

        for &id in order.iter().rev() {
            // `run_origin` and `loaded_modules` are both keyed by the same
            // canonical file (#2955), so `id`'s origin is directly a
            // `loaded_modules` key -- no separate `id -> loaded_modules key`
            // map is needed once both agree on canonicalization.
            let Some(defs) = self.run_origin(id).and_then(|k| self.loaded_modules.get(k)) else {
                continue;
            };

            // Name -> index, built once so the fixed point below is O(log D)
            // per sibling call rather than an O(D) scan of the whole module.
            let name_index: BTreeMap<&str, usize> = defs
                .iter()
                .enumerate()
                .map(|(i, (name, _, _))| (name.as_str(), i))
                .collect();

            // Seeds: this module's defs some stub already names. Then the
            // fixed point over its own bare sibling calls, via the same
            // `grow_to_fixed_point` the top-level closure above uses -- what
            // a visit *does* still differs (this one grows a local
            // `kept_names` by sibling name, and only forwards a link name
            // into the shared `wanted`, where the top-level one grows
            // `wanted` itself and retries an alias), because a module can be
            // `import`ed under several different aliases at the top level
            // but a hoisted link run is keyed by one globally unique id --
            // see the top-level closure's comment for the oracle rows this
            // asymmetry is checked against. A retry rule fixed in one almost
            // certainly needs the same fix in the other.
            let mut kept_names: BTreeSet<&str> = defs
                .iter()
                .map(|(name, _, _)| name.as_str())
                .filter(|name| wanted.contains(&jq::ModuleRun::link_name(id, name)))
                .collect();
            let mut keep = vec![false; defs.len()];
            grow_to_fixed_point(defs.len(), |i| {
                let (name, _, body) = &defs[i];
                if keep[i] || !kept_names.contains(name.as_str()) {
                    return false;
                }
                keep[i] = true;
                let mut grew = false;
                for called in called_func_names(body) {
                    if jq::ModuleRun::is_link_name(&called) {
                        wanted.insert(called);
                    } else if let Some(&sibling_i) = name_index.get(called.as_str()) {
                        grew |= kept_names.insert(defs[sibling_i].0.as_str());
                    }
                }
                grew
            });

            let linked: FuncDefList = defs
                .iter()
                .zip(&keep)
                .filter(|(_, keep)| **keep)
                .map(|((name, params, body), _)| {
                    (
                        jq::ModuleRun::link_name(id, name),
                        params.clone(),
                        body.clone(),
                    )
                })
                .collect();
            let alias = jq::ModuleRun::link_alias(id);
            expr = wrap_run(expr, linked, id, Some(&alias));
        }
        expr
    }
}

/// Visit every index in `0..len` via `expand`, repeating the full pass until
/// one changes nothing -- the "grow until nothing new resolves" fixed point
/// [`ModuleLoader::link_dependency_runs`]'s two reachability closures both
/// need, pulled out once so a fix to the iteration itself (when to stop,
/// what order a pass visits) is made in one place rather than two. `expand`
/// reports whether visiting `i` changed anything it manages; the two callers
/// still decide, separately, what a visit *does* -- see each call site's own
/// comment for why that differs.
fn grow_to_fixed_point(len: usize, mut expand: impl FnMut(usize) -> bool) {
    loop {
        let mut grew = false;
        for i in 0..len {
            grew |= expand(i);
        }
        if !grew {
            break;
        }
    }
}

/// Rewrite every computed-key `Expr` inside a `reduce`/`foreach`/`as {...}`
/// pattern list (#2677's `ObjectKey::Expr`) the same way
/// [`rewrite_namespaced_calls`] rewrites everywhere else -- see #2957:
/// `Expr::Reduce`/`Expr::Foreach`/`Expr::AsPattern` carry a `Vec<Pattern>`
/// that a hand-rolled match previously passed through untouched, so a
/// namespaced call reachable only from a destructuring key
/// (`. as {(m::f): $v}`) reached evaluation as a raw `NamespacedCall`.
/// `map_pattern_subexprs` is the one exhaustive definition of what a pattern
/// contains, matching the fix `called_func_names` already applies to the same
/// blind spot.
fn rewrite_namespaced_calls_in_patterns(patterns: Vec<Pattern>) -> Vec<Pattern> {
    patterns
        .iter()
        .map(|p| map_pattern_subexprs(p, &mut |key| rewrite_namespaced_calls(key.clone())))
        .collect()
}

/// Recursively rewrite NamespacedCall expressions to regular FuncCall expressions
/// by transforming `namespace::func(args)` to `namespace::func(args)` as a regular call
fn rewrite_namespaced_calls(expr: Expr) -> Expr {
    match expr {
        // #1371: parse-time only, so neither can occur -- both are built by
        // evaluation, which happens after this rewrite. Named rather than
        // wildcarded so a future evaluation-time caller gets a compile error
        // here instead of silently keeping a `NamespacedCall` inside one.
        Expr::Shared(_) | Expr::DefCall { .. } => expr,
        Expr::NamespacedCall {
            namespace,
            name,
            args,
        } => {
            // Convert to a regular function call with the namespaced name
            let full_name = format!("{namespace}::{name}");
            let rewritten_args: Vec<Expr> =
                args.into_iter().map(rewrite_namespaced_calls).collect();
            Expr::FuncCall {
                name: full_name,
                args: rewritten_args,
                // A `namespace::name` call is never itself a candidate for
                // #2036's shadow-detection (that only wraps bare-identifier
                // keyword dispatch, never `::`-qualified syntax).
                builtin_fallback: None,
            }
        }
        // Recursively process all other expression types
        Expr::Pipe(exprs) => Expr::Pipe(exprs.into_iter().map(rewrite_namespaced_calls).collect()),
        Expr::Comma(exprs) => {
            Expr::Comma(exprs.into_iter().map(rewrite_namespaced_calls).collect())
        }
        Expr::Optional(inner) => Expr::Optional(Box::new(rewrite_namespaced_calls(*inner))),
        Expr::Paren(inner) => Expr::Paren(Box::new(rewrite_namespaced_calls(*inner))),
        Expr::Array(inner) => Expr::Array(Box::new(rewrite_namespaced_calls(*inner))),
        Expr::Object(entries) => {
            let new_entries = entries
                .into_iter()
                .map(|entry| jq::ObjectEntry {
                    key: match entry.key {
                        jq::ObjectKey::Expr(e) => {
                            jq::ObjectKey::Expr(Box::new(rewrite_namespaced_calls(*e)))
                        }
                        other => other,
                    },
                    value: rewrite_namespaced_calls(entry.value),
                })
                .collect();
            Expr::Object(new_entries)
        }
        Expr::FuncCall {
            name,
            args,
            builtin_fallback,
        } => {
            let new_args: Vec<Expr> = args.into_iter().map(rewrite_namespaced_calls).collect();
            Expr::FuncCall {
                name,
                args: new_args,
                // #2036: preserved and recursed into -- this pass runs
                // *before* `resolve.rs`'s own rewrite (see
                // `process_program`'s ordering doc comment), so a
                // shadow-candidate's fallback can itself contain a
                // `namespace::f(...)` call (e.g. inside a shadowed
                // `limit(n; ns::f)`) that still needs rewriting here too.
                builtin_fallback: builtin_fallback.map(|b| Box::new(rewrite_namespaced_calls(*b))),
            }
        }
        Expr::FuncDef {
            name,
            params,
            body,
            then,
            ..
        } => Expr::FuncDef {
            name,
            params,
            body: Box::new(rewrite_namespaced_calls(*body)),
            then: Box::new(rewrite_namespaced_calls(*then)),
            bound: FuncDefBound::default(),
        },
        Expr::Arithmetic { op, left, right } => Expr::Arithmetic {
            op,
            left: Box::new(rewrite_namespaced_calls(*left)),
            right: Box::new(rewrite_namespaced_calls(*right)),
        },
        Expr::Negate(inner) => Expr::Negate(Box::new(rewrite_namespaced_calls(*inner))),
        Expr::Compare { op, left, right } => Expr::Compare {
            op,
            left: Box::new(rewrite_namespaced_calls(*left)),
            right: Box::new(rewrite_namespaced_calls(*right)),
        },
        Expr::And(left, right) => Expr::And(
            Box::new(rewrite_namespaced_calls(*left)),
            Box::new(rewrite_namespaced_calls(*right)),
        ),
        Expr::Or(left, right) => Expr::Or(
            Box::new(rewrite_namespaced_calls(*left)),
            Box::new(rewrite_namespaced_calls(*right)),
        ),
        Expr::Alternative(left, right) => Expr::Alternative(
            Box::new(rewrite_namespaced_calls(*left)),
            Box::new(rewrite_namespaced_calls(*right)),
        ),
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => Expr::If {
            cond: Box::new(rewrite_namespaced_calls(*cond)),
            then_branch: Box::new(rewrite_namespaced_calls(*then_branch)),
            else_branch: Box::new(rewrite_namespaced_calls(*else_branch)),
        },
        Expr::Try { expr, catch } => Expr::Try {
            expr: Box::new(rewrite_namespaced_calls(*expr)),
            catch: catch.map(|e| Box::new(rewrite_namespaced_calls(*e))),
        },
        Expr::Error(inner) => Expr::Error(inner.map(|e| Box::new(rewrite_namespaced_calls(*e)))),
        Expr::As { expr, var, body } => Expr::As {
            expr: Box::new(rewrite_namespaced_calls(*expr)),
            var,
            body: Box::new(rewrite_namespaced_calls(*body)),
        },
        Expr::Reduce {
            input,
            patterns,
            init,
            update,
        } => Expr::Reduce {
            input: Box::new(rewrite_namespaced_calls(*input)),
            patterns: rewrite_namespaced_calls_in_patterns(patterns),
            init: Box::new(rewrite_namespaced_calls(*init)),
            update: Box::new(rewrite_namespaced_calls(*update)),
        },
        Expr::Foreach {
            input,
            patterns,
            init,
            update,
            extract,
        } => Expr::Foreach {
            input: Box::new(rewrite_namespaced_calls(*input)),
            patterns: rewrite_namespaced_calls_in_patterns(patterns),
            init: Box::new(rewrite_namespaced_calls(*init)),
            update: Box::new(rewrite_namespaced_calls(*update)),
            extract: extract.map(|e| Box::new(rewrite_namespaced_calls(*e))),
        },
        Expr::Limit { n, expr } => Expr::Limit {
            n: Box::new(rewrite_namespaced_calls(*n)),
            expr: Box::new(rewrite_namespaced_calls(*expr)),
        },
        Expr::FirstExpr(inner) => Expr::FirstExpr(Box::new(rewrite_namespaced_calls(*inner))),
        Expr::LastExpr(inner) => Expr::LastExpr(Box::new(rewrite_namespaced_calls(*inner))),
        Expr::NthExpr { n, expr } => Expr::NthExpr {
            n: Box::new(rewrite_namespaced_calls(*n)),
            expr: Box::new(rewrite_namespaced_calls(*expr)),
        },
        Expr::Until { cond, update } => Expr::Until {
            cond: Box::new(rewrite_namespaced_calls(*cond)),
            update: Box::new(rewrite_namespaced_calls(*update)),
        },
        Expr::While { cond, update } => Expr::While {
            cond: Box::new(rewrite_namespaced_calls(*cond)),
            update: Box::new(rewrite_namespaced_calls(*update)),
        },
        Expr::Repeat(inner) => Expr::Repeat(Box::new(rewrite_namespaced_calls(*inner))),
        Expr::Range { from, to, step } => Expr::Range {
            from: Box::new(rewrite_namespaced_calls(*from)),
            to: to.map(|e| Box::new(rewrite_namespaced_calls(*e))),
            step: step.map(|e| Box::new(rewrite_namespaced_calls(*e))),
        },
        Expr::AsPattern {
            expr,
            patterns,
            body,
        } => Expr::AsPattern {
            expr: Box::new(rewrite_namespaced_calls(*expr)),
            patterns: rewrite_namespaced_calls_in_patterns(patterns),
            body: Box::new(rewrite_namespaced_calls(*body)),
        },
        Expr::StringInterpolation(parts) => {
            let new_parts = parts
                .into_iter()
                .map(|part| match part {
                    jq::StringPart::Literal(s) => jq::StringPart::Literal(s),
                    jq::StringPart::Expr(e) => {
                        jq::StringPart::Expr(Box::new(rewrite_namespaced_calls(*e)))
                    }
                })
                .collect();
            Expr::StringInterpolation(new_parts)
        }
        // Assignment operators
        Expr::Assign { path, value } => Expr::Assign {
            path: Box::new(rewrite_namespaced_calls(*path)),
            value: Box::new(rewrite_namespaced_calls(*value)),
        },
        Expr::Update { path, filter } => Expr::Update {
            path: Box::new(rewrite_namespaced_calls(*path)),
            filter: Box::new(rewrite_namespaced_calls(*filter)),
        },
        Expr::CompoundAssign { op, path, value } => Expr::CompoundAssign {
            op,
            path: Box::new(rewrite_namespaced_calls(*path)),
            value: Box::new(rewrite_namespaced_calls(*value)),
        },
        Expr::AlternativeAssign { path, value } => Expr::AlternativeAssign {
            path: Box::new(rewrite_namespaced_calls(*path)),
            value: Box::new(rewrite_namespaced_calls(*value)),
        },
        // #798: yq-mode-only grammar (the parser never produces this in jq
        // mode, where namespaced calls/imports live), but still rewritten
        // for consistency with every other assignment-family variant above.
        // omni-dev: coverage tolerate reason="unreachable: `try_parse_meta_op` only fires under `ParserMode::Yq` (src/jq/parser.rs), and `rewrite_namespaced_calls` is only reached via `ModuleProcessor::process_program`, which jq_runner's own jq-mode `run` is the sole caller of -- so a `MetaAssign` node can never reach this function (#798)"
        Expr::MetaAssign {
            target,
            slot,
            value,
            is_update,
        } => Expr::MetaAssign {
            target: Box::new(rewrite_namespaced_calls(*target)),
            slot,
            value: Box::new(rewrite_namespaced_calls(*value)),
            is_update,
        },
        // omni-dev: coverage end
        // Label-break
        Expr::Label { name, body } => Expr::Label {
            name,
            body: Box::new(rewrite_namespaced_calls(*body)),
        },
        Expr::Break(name) => Expr::Break(name),
        // Both halves hold sub-expressions, so a namespaced call can appear in
        // either — `.[ns::f]` as much as `(ns::f)[0]`.
        Expr::IndexExpr { target, key } => Expr::IndexExpr {
            target: Box::new(rewrite_namespaced_calls(*target)),
            key: Box::new(rewrite_namespaced_calls(*key)),
        },
        // Same reasoning as `IndexExpr`: a namespaced call can appear in the
        // target or either bound — `.[ns::f():ns::g()]`.
        Expr::SliceExpr { target, start, end } => Expr::SliceExpr {
            target: Box::new(rewrite_namespaced_calls(*target)),
            start: start.map(|e| Box::new(rewrite_namespaced_calls(*e))),
            end: end.map(|e| Box::new(rewrite_namespaced_calls(*e))),
        },
        // A namespaced call can appear inside any of the 82
        // sub-expression-carrying builtins (`map(ns::f)`, `limit(1; ns::f)`,
        // `sub("a"; "b"; ns::f)`, ...) -- #1505. Without this arm, the
        // catch-all below returned `expr` unchanged, so a call nested this
        // way never got rewritten from `Expr::NamespacedCall` to
        // `Expr::FuncCall`, and evaluation failed with "module not loaded".
        // `map_builtin_subexprs` (`jq::walk`) is `builtin_kids`'s mapping
        // twin: exhaustive over every `Builtin` variant with no wildcard, so
        // a future variant that carries a sub-expression is a compile error
        // here until it's declared there, the same discipline this file's
        // own manual `Expr` recursion already follows above.
        //
        // A call to a namespace that was never actually `import`ed now
        // fails the same way in this position as it already did bare
        // (`nonexistent_ns::foo` outside any builtin) or piped -- rewritten
        // to `Expr::FuncCall` unconditionally, regardless of import status,
        // reporting "undefined function" from the general `FuncCall`
        // resolution path rather than `eval.rs`'s own `Expr::NamespacedCall`
        // arm's "module not loaded". Not a regression this fix introduces:
        // confirmed live that the bare/piped case already answered
        // "undefined function" before this change (`rewrite_namespaced_calls`
        // never checked import status anywhere), and that real jq's own
        // message for this shape is closer to "X/0 is not defined" (a
        // compile error) than to either succinctly wording -- this arm
        // brings the builtin-argument position in line with what every
        // other position already did, rather than diverging from it.
        Expr::Builtin(builtin) => Expr::Builtin(map_builtin_subexprs(&builtin, &mut |sub| {
            rewrite_namespaced_calls(sub.clone())
        })),
        // Expressions that don't contain sub-expressions - return as-is
        Expr::Identity
        | Expr::Field(_)
        | Expr::Index { .. }
        | Expr::Slice { .. }
        | Expr::Iterate
        | Expr::RecursiveDescent
        | Expr::Literal(_)
        | Expr::Var(_)
        | Expr::TrackedVar(_)
        | Expr::Loc { .. }
        | Expr::Env
        | Expr::Not
        | Expr::Format(_) => expr,
    }
}

/// Extract function definitions from an expression, preserving parameters.
fn extract_func_defs(expr: &Expr) -> FuncDefList {
    let mut defs = Vec::new();

    fn extract_inner(expr: &Expr, defs: &mut FuncDefList) {
        if let Expr::FuncDef {
            name,
            params,
            body,
            then,
            ..
        } = expr
        {
            defs.push((name.clone(), params.clone(), (**body).clone()));
            extract_inner(then, defs);
        }
    }

    extract_inner(expr, &mut defs);
    defs
}

/// Output formatting configuration
struct OutputConfig {
    compact: bool,
    raw_output: bool,
    join_output: bool,
    raw_output0: bool,
    ascii_output: bool,
    color_output: bool,
    color_scheme: ColorScheme,
    sort_keys: bool,
    indent_string: String,
    unbuffered: bool,
    seq: bool,
    /// The active output convention: whether a number literal is reformatted
    /// to jq's own spelling (`JqCompat`, the default) or echoed verbatim
    /// (`JqPreserveInput`, `--preserve-input`/`SUCCINCTLY_PRESERVE_INPUT=1`),
    /// and which tool's escape table strings go through.
    ///
    /// #2874: was `jq_compat: bool`, one layer below where `JsonConvention`
    /// was finally constructed — so the enum was built *late*, from the
    /// bool, and five other consumers each re-derived the same split from
    /// that bool instead. Computing it once here makes this the single
    /// source of truth every consumer reads, via
    /// [`JsonConvention::preserves_source_values`] /
    /// [`JsonConvention::uses_jq_escape_table`] rather than a raw
    /// two-variant pattern (#2209).
    ///
    /// This is jq mode, so it is never `Preserve` — that is yq's whole
    /// bundle, escape table included; see `JsonConvention`'s own doc comment.
    convention: JsonConvention,
}

impl OutputConfig {
    fn from_args(args: &JqCommand) -> Self {
        let indent_string = if args.tab {
            "\t".to_string()
        } else if let Some(n) = args.indent {
            " ".repeat(n as usize)
        } else if args.compact_output {
            String::new()
        } else {
            "  ".to_string() // Default: 2 spaces
        };

        // Priority is documented on `resolve_color`.
        let color_output = crate::env_config::resolve_color(
            crate::env_config::ColorChoice::from_flags(args.monochrome_output, args.color_output),
            crate::env_config::no_color_from_env(),
            std::io::stdout().is_terminal(),
        );

        // Get color scheme from JQ_COLORS env var (or defaults)
        let color_scheme = ColorScheme::from_env();

        // Determine the output convention with priority:
        // 1. --preserve-input flag forces preserve (keep original formatting)
        // 2. SUCCINCTLY_PRESERVE_INPUT=1 env var does the same
        // 3. Default: jq-compatible formatting
        //
        // #2209: `JqPreserveInput`, never `Preserve` -- this is jq mode, and
        // `Preserve` is yq's whole bundle, escape table included. Selecting
        // that here made `--preserve-input` silently render strings through
        // yq's table on the cursor-streaming path, a divergence from real jq
        // that `--preserve-input` was never meant to cause (it is documented
        // to affect number spelling and duplicate keys only).
        let jq_compat = !args.preserve_input
            && !std::env::var("SUCCINCTLY_PRESERVE_INPUT")
                .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        let convention = if jq_compat {
            JsonConvention::JqCompat
        } else {
            JsonConvention::JqPreserveInput
        };

        Self {
            compact: args.compact_output,
            raw_output: args.raw_output || args.join_output || args.raw_output0,
            join_output: args.join_output,
            raw_output0: args.raw_output0,
            ascii_output: args.ascii_output,
            color_output,
            color_scheme,
            sort_keys: args.sort_keys,
            indent_string,
            unbuffered: args.unbuffered,
            seq: args.seq,
            convention,
        }
    }

    /// Returns true if raw identity output can be used (no formatting transformations needed).
    ///
    /// When this returns true for identity queries, we can output the original JSON bytes
    /// directly without parsing or materializing values, saving significant memory.
    fn can_use_raw_identity(&self) -> bool {
        // Raw output is safe when:
        // - Compact mode (output matches compact input format)
        // - No color (would need to inject ANSI codes)
        // - No sort_keys (would need to reorder object keys)
        // - No ascii_output (would need to escape non-ASCII)
        // - No raw_output (would strip quotes from strings)
        // - No seq mode (would need to add RS characters)
        // - Source-preserving convention (jq's own would need to reformat
        //   numbers like 4e4 → 4E+4)
        //
        // This only decides whether the *raw-echo path itself* is taken at
        // all -- DEL-escaping (#2985) is handled separately, inside
        // `write_identity_bytes`, once a document's actual bytes are known.
        // An earlier draft folded `&& !uses_jq_escape_table()` into this
        // function instead, which disabled the fast path for *every*
        // jq-mode document (not just DEL-bearing ones), since
        // `JqCompat`/`JqPreserveInput` are the only two conventions this
        // binary ever constructs -- caught by review as a 2.7x-4.8x
        // slowdown on ordinary input with no DEL at all.
        //
        // `--indent 0` counts as compact here for the same reason it does in
        // both streaming printers (#3155): its output *is* `-c`'s, so it can
        // take the same echo.
        (self.compact || self.indent_string.is_empty())
            && !self.color_output
            && !self.sort_keys
            && !self.ascii_output
            && !self.raw_output
            && !self.seq
            && self.convention.preserves_source_values()
    }

    /// Writes `json_bytes` on `can_use_raw_identity`'s fast path, escaping
    /// any raw DEL byte (`0x7f`) a jq-escape-table convention requires
    /// (#2985). `Preserve` (yq's own) never re-encodes DEL, so its
    /// documents always take the plain byte-for-byte write.
    ///
    /// This substitutes in place rather than falling back to the
    /// slower, validating write path when DEL is found: this whole path
    /// exists specifically for `--preserve-input`'s lenient, non-validating
    /// echo (e.g. a trailing comma still round-trips), and falling back to
    /// a real parse-and-reencode would make that leniency depend on
    /// whether an unrelated DEL byte happens to also be in the document --
    /// caught by review on the first draft, which did exactly that.
    fn write_identity_bytes(&self, out: &mut impl Write, json_bytes: &[u8]) -> std::io::Result<()> {
        if !self.convention.del_needs_escaping(json_bytes) {
            return out.write_all(json_bytes);
        }
        let mut rest = json_bytes;
        while let Some(pos) = rest.iter().position(|&b| b == 0x7f) {
            out.write_all(&rest[..pos])?;
            out.write_all(b"\\u007f")?;
            rest = &rest[pos + 1..];
        }
        out.write_all(rest)
    }
}

use std::borrow::Cow as PreparedCow;
use std::collections::HashMap;

/// One object field, recorded on the single walk `print_json` makes over
/// the field list (#1385).
///
/// Holds BP positions rather than whole `JsonCursor`s. A cursor is 32 bytes
/// of which 24 -- its `text` slice and its `&JsonIndex` -- are identical for
/// every field in the document, so a pair of them per field was storing the
/// same two pointers over and over. Hoisting them to `Frame` and keeping
/// only the two `bp_pos` takes the buffer from 88 bytes per field to 40:
/// on a 10 MB document with a 54K-field object it is the difference between
/// +6.0 MB and +2.7 MB of peak RSS, and on a 1M-field object between
/// +100 MB and +45 MB.
#[derive(Clone, Copy)]
struct PreparedField<'a> {
    /// BP position of the key cursor.
    key_bp: usize,
    /// BP position of the value cursor.
    value_bp: usize,
    /// The value cursor's own `text_position()`, if the #1643 delimiter
    /// check already resolved one -- `usize::MAX` otherwise (a sentinel
    /// rather than `Option<usize>` for the same reason this struct hoists
    /// `text`/`index` into `Frame` above: an `Option<usize>` doubles this
    /// field's size to 16 bytes on a type with no spare bit pattern to
    /// exploit, which the same 1M-field-object memory math this struct's
    /// own doc comment cites would double again. The write loop passes it
    /// to `print_json` as `known_text_pos` so that recursive call doesn't
    /// redo the same rank/select lookup the check already paid for.
    value_start: usize,
    /// The key's raw source span, quotes included.
    raw: &'a [u8],
    /// Whether that span contains a backslash escape.
    escaped: bool,
    /// Whether that span contains a raw DEL byte (`0x7f`) -- #2591: legal
    /// unescaped in JSON source, but jq's own escape table still escapes it
    /// on output, unlike a plain backslash-free span otherwise. This file is
    /// jq mode only (`succinctly yq` has its own separate runner), but it is
    /// *not* jq-convention only: `--preserve-input`/`SUCCINCTLY_PRESERVE_INPUT=1`
    /// make this same runner select `JsonConvention::JqPreserveInput`, and
    /// key escaping must agree with whatever the value side does -- so
    /// `write_object_key` gates this on the active `JsonConvention` exactly
    /// like `write_json_string_pretty`'s twin fix in `src/json/light.rs`,
    /// not unconditionally: `JsonConvention::uses_jq_escape_table()`, not
    /// `preserves_source_values()` (#2985 -- the two happened to agree
    /// before `JqPreserveInput` existed, but that convention preserves
    /// source *values* while still using jq's own escape table).
    has_del: bool,
}

/// The `(text, index)` pair every cursor in one document shares, hoisted out
/// of [`PreparedField`] so it is stored once instead of twice per field.
struct Frame<'a, W> {
    text: &'a [u8],
    index: &'a succinctly::json::JsonIndex<W>,
}

impl<'a, W: Clone + AsRef<[u64]>> Frame<'a, W> {
    #[inline]
    fn cursor(&self, bp: usize) -> JsonCursor<'a, W> {
        JsonCursor::from_bp_position(self.index, self.text, bp)
    }
}

/// Whether any two key spans *may* be byte-identical.
///
/// Small objects -- nearly every object in a real document -- take the
/// pairwise branch, which allocates nothing. Above the threshold the
/// pairwise loop is quadratic, so the wide case sorts one 64-bit hash per
/// key and looks for an adjacent pair.
///
/// It sorts *hashes*, not the spans it used to (#1514). Sorting `&[u8]`
/// meant every comparison chased a pointer into a random offset of the
/// document text, which cost 59-85 ns per key on a 10 MB document with a
/// wide root object. Sorting `u64`s compares registers over one contiguous
/// array, half the width of the fat pointers it replaces.
///
/// An open-addressed table was tried here and is *not* what shipped: it
/// beat the sort on an M4 Pro and lost to it by 24% on a 7950X at 100 MB,
/// where 7.1M keys make the table 134 MB against 32 MB of L3 per CCD. A
/// sort streams; a table does not. See `docs/plan/jq-duplicate-key-collapse.md`.
///
/// The answer above the threshold is conservative: two distinct keys
/// sharing a 64-bit hash report `true`. [`collapse_duplicate_fields`]
/// resolves it on the spans themselves and returns `None` when nothing
/// actually collapsed, so a collision costs one exact rebuild.
fn spans_repeat(prepared: &[PreparedField<'_>]) -> bool {
    if prepared.len() <= PAIRWISE_SPAN_SCAN_LIMIT {
        let mut marks = [0u64; PAIRWISE_SPAN_SCAN_LIMIT];
        for (slot, field) in marks.iter_mut().zip(prepared) {
            *slot = key_span_fingerprint(field.raw);
        }
        return (0..prepared.len()).any(|i| {
            ((i + 1)..prepared.len())
                .any(|j| marks[i] == marks[j] && prepared[i].raw == prepared[j].raw)
        });
    }
    let mut hashes: Vec<u64> = prepared.iter().map(|field| key_hash(field.raw)).collect();
    hashes.sort_unstable();
    hashes.windows(2).any(|pair| pair[0] == pair[1])
}

/// jq's duplicate-key rule over an object's already-walked fields (#1385):
/// a repeated key collapses to its *first* position holding its *last*
/// value.
///
/// Returns `None` when no key repeats, which lets the caller print straight
/// from the list it already has. Only an object that genuinely carries a
/// duplicate allocates.
///
/// Keys compare by raw source span while nothing is escaped -- two
/// escape-free spans are equal exactly when their decoded values are. A
/// document that escapes any key falls back to comparing decoded strings, so
/// `{"a\/b":1,"a/b":2}` still collapses even though its two spans differ.
///
/// Linear in the field count: the surviving slot for each key comes from a
/// `HashMap`, never from scanning the keys accepted so far. Scanning made a
/// single duplicate in a 100K-field object take 8.5 s where not collapsing
/// at all took 0.02 s and real jq took 0.03 s.
fn collapse_duplicate_fields<'a>(
    prepared: &[PreparedField<'a>],
    frame: &Frame<'a, impl Clone + AsRef<[u64]>>,
) -> Option<Vec<PreparedField<'a>>> {
    if prepared.len() < 2 {
        return None;
    }
    let any_escaped = prepared.iter().any(|field| field.escaped);
    if !any_escaped && !spans_repeat(prepared) {
        return None;
    }

    let decoded: Vec<Option<PreparedCow<'a, str>>> = prepared
        .iter()
        .map(|field| match frame.cursor(field.key_bp).value() {
            StandardJson::String(k) => k.as_str().ok(),
            _ => None,
        })
        .collect();

    let mut slot_of: HashMap<&str, usize> = HashMap::with_capacity(prepared.len());
    let mut chosen: Vec<PreparedField<'a>> = Vec::with_capacity(prepared.len());
    for (i, field) in prepared.iter().enumerate() {
        // A key that does not decode has no name to collapse on, so it is
        // kept where it stands. Skipping it instead deleted the field from
        // the output entirely -- `{"a\q":1,"b":2}` printed as `{"b":2}`
        // (#1385 review); the write loop below still has its raw span and
        // echoes it verbatim, which is what this printer did before #1385.
        let Some(name) = decoded[i].as_deref() else {
            chosen.push(*field);
            continue;
        };
        match slot_of.get(name) {
            Some(&at) => chosen[at] = *field,
            None => {
                slot_of.insert(name, chosen.len());
                chosen.push(*field);
            }
        }
    }
    if chosen.len() == prepared.len() {
        return None;
    }
    Some(chosen)
}

/// Write one object key and its colon, honouring `-a`/`--ascii-output`.
///
/// Shared by the compact and pretty loops, which differ only in what
/// `space_after_colon` holds. The escape-free span goes out verbatim -- that
/// is the zero-copy path this printer exists for -- and a key that will not
/// decode falls back to echoing its raw span rather than vanishing.
fn write_object_key<Out: Write, W: Clone + AsRef<[u64]>>(
    out: &mut Out,
    frame: &Frame<'_, W>,
    field: &PreparedField<'_>,
    config: &OutputConfig,
    space_after_colon: &str,
) -> Result<()> {
    // A key that is not a string is not a key: the document is malformed and
    // only bracket-matching let it through (`{invalid: 1}`, `{123: 1}`).
    //
    // This used to `return Ok(())`, skipping the `:` below as well while the
    // caller went on to print the value -- so `{invalid: 1}` came out as
    // `{1}`, which no JSON parser can read, at exit 0 (#1194). Emitting
    // unparseable output is strictly worse than the silent drop it was
    // reasoned about as; raise instead.
    let StandardJson::String(key) = frame.cursor(field.key_bp).value() else {
        return Err(MalformedJsonError::new(EvalError::malformed_json_text(frame.text)).into());
    };
    write_json_string_zero_copy(out, field.raw, field.escaped, field.has_del, key, config)?;
    out.write_all(b":")?;
    out.write_all(space_after_colon.as_bytes())?;
    Ok(())
}

/// Writes a JSON string under `config`'s escaping convention, taking the
/// zero-copy fast path when the source span needs no re-encoding.
///
/// `raw`/`escaped`/`has_del` are `s.raw_and_escaped()`'s own answer, taken as
/// separate parameters (rather than recomputed here from `s`) because
/// `write_object_key`'s caller already has to consult them a second time
/// earlier, for duplicate-key collapse (#1385) -- rescanning here would pay
/// for that a third time. Every other caller has nothing else to hoist them
/// for and just passes `s.raw_and_escaped()` straight through.
///
/// #2591/#2592: a raw DEL byte is the one extra case the zero-copy path
/// cannot take under jq's own escape convention -- `0x7f` is legal
/// unescaped JSON source, but jq's escape table still re-encodes it to
/// `\u007f` on output. Shared by four call sites (this one plus three
/// sibling ones in `print_json`/`keys_unsorted`, #2592) that used to each
/// hand-roll this gate-and-branch shape independently.
///
/// #2985: the gate is keyed on the escape-table axis, not the preserve axis
/// -- `--preserve-input` must not turn off jq's own DEL escaping (#2209).
/// #2874 translated this gate behaviour-preservingly from `config.jq_compat`
/// to `!preserves_source_values()`, which happened to agree with
/// `uses_jq_escape_table()` for the two conventions that existed then
/// (`Preserve`/`JqCompat`) but diverges now that `JqPreserveInput` sits
/// between them: it preserves source *values* (#2209) but still uses jq's
/// escape table. Real jq 1.7.1 escapes a raw DEL on every route
/// (`-c`/`-S`/pretty/`-a`/`-s`/`-C`) under `--preserve-input`; before this
/// fix, only the non-zero-copy routes (`-a`/`-s`/`-C`, which go through
/// `escape_json_body`'s own already-correct `uses_jq_escape_table()` check)
/// agreed with it.
fn write_json_string_zero_copy<Out: Write>(
    out: &mut Out,
    raw: &[u8],
    escaped: bool,
    has_del: bool,
    s: JsonString<'_>,
    config: &OutputConfig,
) -> Result<()> {
    if !(config.ascii_output || escaped || has_del && config.convention.uses_jq_escape_table()) {
        out.write_all(raw)?;
    } else if let Ok(decoded) = s.as_str() {
        out.write_all(b"\"")?;
        let text = if config.ascii_output {
            escape_json_string_ascii(&decoded)
        } else {
            escape_json_string(&decoded)
        };
        out.write_all(text.as_bytes())?;
        out.write_all(b"\"")?;
    } else {
        out.write_all(raw)?;
    }
    Ok(())
}

/// Trim leading and trailing ASCII whitespace from a byte slice.
///
/// Equivalent to `<[u8]>::trim_ascii` but usable on our MSRV (1.73), which
/// predates that method's stabilization (1.80).
fn trim_ascii_ws(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(start, |p| p + 1);
    &bytes[start..end]
}

/// Determine the `-e`/`--exit-status` value for a raw JSON token emitted on the
/// identity fast path. Only the bare `null` and `false` literals are falsy; a
/// quoted `"false"`/`"null"` string or any number stays truthy.
fn identity_exit_status_value(json_bytes: &[u8]) -> OwnedValue {
    match trim_ascii_ws(json_bytes) {
        b"null" => OwnedValue::Null,
        b"false" => OwnedValue::Bool(false),
        _ => OwnedValue::Bool(true),
    }
}

/// A document the semi-index accepted that is not, in fact, valid JSON (#1194).
///
/// The printer's failures are otherwise all I/O, which `anyhow` carries to the
/// top and aborts on. This one is a *data* error and belongs in jq's own
/// diagnostic channel -- exit 5 through the [`ErrorSink`], with the rest of
/// the input stream still processed (#355). Giving it a distinct type lets the
/// single place that owns the sink tell the two apart by `downcast_ref`
/// instead of matching on message text.
/// Raise if a finished key walk ran out on an unpaired child (#1194).
///
/// `DistinctKeyCursors` records this as it walks, so asking costs a field
/// read rather than a second pass over the keys -- which matters, because
/// `keys_unsorted` over a 2 MB `wide` document is one of the workloads
/// `scripts/perf-guard.py` pins.
///
/// `doc_text` is the document the last key came from. It is `None` only when
/// the walk yielded nothing, and an object that yields no keys *and* ends
/// unpaired is `{invalid}` -- already refused before the opening bracket by
/// the `unpaired_tail` check, so this arm cannot be the one to report it.
///
/// Also checks [`DistinctKeyCursors::delimiter_fault`] (#1677), the sibling
/// fault the same walk can find: a missing/doubled `,`/`:`. Both share the
/// "ask only once the walk is done" contract -- this writer cannot rewind,
/// so either fault surfaces only here, potentially behind an
/// already-written partial `[`/array -- see `JqValue::LazyKeysArray`'s own
/// doc comment for why that trade is accepted.
///
/// #2261: also checks [`DistinctKeyCursors::trailing_gap_ok`] -- a trailing
/// stray `,` after the object's own real last key (`{"a":1,}`), same "ask
/// only once the walk is done" contract, same O(1) `next_sibling()` cost.
fn bail_if_keys_malformed<F: succinctly::jq::document::DocumentFields>(
    keys: &DistinctKeyCursors<F>,
    doc_text: Option<&[u8]>,
) -> Result<()> {
    match (
        (keys.is_malformed() || !keys.trailing_gap_ok(b'}')),
        doc_text,
    ) {
        (true, Some(text)) => {
            Err(MalformedJsonError::new(EvalError::malformed_json_text(text)).into())
        }
        _ => Ok(()),
    }
}

/// A malformed-document error travelling through an `anyhow::Result`, to be
/// `downcast` back out at the reporting boundary (#1194).
///
/// It carries the [`EvalError`]'s message and classification, not the
/// `EvalError` itself: `anyhow::Error` needs `Send + Sync`, and since #2999
/// `OwnedValue` holds `Rc`s, so an `EvalError` -- whose payload can be the
/// raw value of `error(v)` -- no longer is either. Every error that reaches
/// this wrapper today is a decode or nesting-depth failure the evaluator
/// raised itself, which never carries a value payload; should one ever
/// arrive, its payload is rendered into the message the way jq prints an
/// uncaught `error(v)` (the string itself, or the value's JSON), so nothing
/// is dropped -- only the payload's *type* is lost, so jq's `(not a string)`
/// suffix would not be appended on this route.
#[derive(Debug)]
pub struct MalformedJsonError {
    message: String,
    kind: Option<ErrorKind>,
}

impl MalformedJsonError {
    /// Wrap `err` for the `anyhow` channel.
    pub fn new(err: EvalError) -> Self {
        let (message, kind) = match err.value {
            EvalErrorPayload::Kind(kind) => (err.message, Some(kind)),
            EvalErrorPayload::None => (err.message, None),
            EvalErrorPayload::Value(OwnedValue::String(text)) => (text, None), // omni-dev: coverage tolerate-line reason="unreachable by construction: every error this wrapper receives today is a decode or nesting-depth failure the evaluator raised itself, never error(v); kept so a future one is rendered rather than dropped (#2999)"
            EvalErrorPayload::Value(value) => (value.to_json(), None), // omni-dev: coverage tolerate-line reason="see the arm above (#2999)"
        };
        Self { message, kind }
    }

    /// The [`EvalError`] this was built from.
    pub fn to_eval_error(&self) -> EvalError {
        EvalError {
            message: self.message.clone(),
            value: match self.kind {
                Some(kind) => EvalErrorPayload::Kind(kind),
                None => EvalErrorPayload::None,
            },
        }
    }
}

impl std::fmt::Display for MalformedJsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for MalformedJsonError {}

/// Adapter to use `std::io::Write` with `core::fmt::Write` methods, for the
/// M2 fast path's non-`LazySeq` writes (#1576) -- mirrors `yq_runner.rs`'s
/// identical local type.
struct FmtWriter<W>(W);

impl<W: Write> core::fmt::Write for FmtWriter<W> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.0.write_all(s.as_bytes()).map_err(|_| core::fmt::Error)
    }
}

/// Write one result and route a failure into jq's own diagnostic
/// channel, or propagate it as a genuine I/O/internal failure -- the same
/// `MalformedJsonError`-downcast dance `run_jq`'s own result-emission
/// loops each used to hand-copy independently (code review, #1830: this
/// PR took that copy from 1 site to 5, so it earned the extraction it
/// didn't have before).
///
/// A malformed-document error (`#1194`, and now `#1830`'s NUL-content
/// check) is a data error, not an I/O one: it belongs in jq's diagnostic
/// channel (exit 5) rather than aborting the process through `anyhow`.
/// Returns `Ok(true)` when the caller's per-document loop should `break`
/// (stop emitting results for *this* document, but fall through to the
/// halt check and carry on with the rest of the stream, #355).
///
/// Both `write` and `at` are closures, not already-produced values (code
/// review, second pass): `write` receives `out` as its own parameter
/// rather than capturing it, avoiding the double-`&mut out` borrow a
/// plain `fn(...) -> Result<()>` argument alongside this function's own
/// use of `out` in the `flush_then_err` arm would hit; `at` is called at
/// most once, only in the (rare) error arm, so a caller whose location is
/// only cheap to resolve lazily (`ErrorAt::Live`'s `current_input_location`
/// read, or an `InputLocation` clone) no longer pays for it on every
/// successful write -- confirmed by three independent code-review
/// passes as a real per-result cost on the success path, not just the
/// error one.
fn route_write_error<W: Write>(
    sink: &mut ErrorSink,
    out: &mut W,
    at: impl FnOnce() -> InputLocation,
    write: impl FnOnce(&mut W) -> Result<()>,
) -> Result<bool> {
    match write(out) {
        Ok(()) => Ok(false),
        Err(e) => match e.downcast_ref::<MalformedJsonError>() {
            Some(malformed) => {
                let err = malformed.to_eval_error();
                sink.report(DiagStyle::Jq, &err, &at());
                Ok(true)
            }
            // Same reasoning as the `--validate` early return elsewhere
            // in this file (#1563): `out` can already hold buffered
            // output from earlier documents/files in this same run, and
            // a genuine (non-malformed-document) error here shouldn't
            // leave that relying on `Drop`'s own best-effort,
            // error-swallowing flush. `flush_then_err` (review of #1673)
            // keeps `e` as the reported error even if the flush also
            // fails, instead of the flush error silently displacing it.
            None => flush_then_err(out, e),
        },
    }
}

/// Validate JSON bytes and print a formatted error message if invalid.
/// Returns Ok(()) if valid, Err with exit code if invalid.
fn validate_json_input(input: &[u8], filename: Option<&str>) -> Result<(), i32> {
    if let Err(err) = validate::validate(input) {
        print_validation_error(&err, input, filename);
        Err(exit_codes::COMPILE_ERROR) // Use compile error exit code for validation failures
    } else {
        Ok(())
    }
}

/// Print a formatted validation error message.
fn print_validation_error(err: &ValidationError, input: &[u8], filename: Option<&str>) {
    let pos = &err.position;

    // Print error header
    eprintln!("jq: validation error: {}", err.kind);

    // Print location
    let location = match filename {
        Some(f) => format!("{}:{}:{}", f, pos.line, pos.column),
        None => format!("<stdin>:{}:{}", pos.line, pos.column),
    };
    eprintln!("  --> {location}");

    // Print context snippet if possible
    if let Some((line_content, caret_offset)) = get_error_line(input, pos.line, pos.column) {
        let line_num_width = pos.line.to_string().len().max(3);
        let blank_padding = " ".repeat(line_num_width + 2);

        eprintln!("{blank_padding}|");
        eprintln!(
            " {:>width$} | {}",
            pos.line,
            line_content,
            width = line_num_width
        );
        eprintln!("{}| {}^", blank_padding, " ".repeat(caret_offset));
    }

    eprintln!();
}

/// Report every diagnostic the compile-time resolution pass could not
/// resolve, in jq's own compile-error shape (#1473, extended to all of them
/// by #2037, extended again to unbound `$variable`s by #2734):
///
/// ```text
/// jq: error: f/3 is not defined at <top-level>, line 1:
/// def f(x): x; if false then f(1;2;3) else 1 end
/// jq: 1 compile error
/// ```
///
/// **Each line is read off a real call-site position where one exists**
/// (#2085). `Expr::FuncCall` still carries no source position — adding one
/// would grow `Expr` and perturb its `Debug`/`PartialEq` — so `call_sites`
/// is the parallel table [`jq::collect_call_sites`] builds during a parse
/// instead, holding the byte offset of every *call*'s own identifier. Because
/// only the generic call-parsing path records into it, an object key, a
/// `$`-variable, or a string that merely spells the same identifier is absent
/// from it by construction. A `$variable` diagnostic has no equivalent table
/// (there is no call-site-style scan for variable references) — it always
/// uses the text-search fallback below, on `${name}` rather than the bare
/// name, so it cannot mistake an unrelated `nope` occurrence (a field, a
/// call) for the `$nope` reference that actually failed.
///
/// That closes the misfire this used to have. Before #2085 every line was
/// found by searching `filter` for the offending identifier, which matched any
/// occurrence of the spelling rather than a call specifically — so
/// `{nosuch: 1} | nosuch` cited the harmless object key on line 1 instead of
/// the failing call on line 2 (jq 1.7.1 says line 2).
///
/// **The text search is still the fallback**, for the cases the table cannot
/// answer: a call inlined from an `include`d module or `~/.jq` has no
/// occurrence in `filter`, and a filter whose own parse differed from the one
/// `collect_call_sites` performs may under-record. Repeated undefined names
/// are still matched positionally against the table in source order, and the
/// fallback keeps its own resume-after-previous-match behaviour so a repeated
/// name still finds its own occurrence rather than repeating the first.
///
/// A filter whose failing call came from an `include`d module or `~/.jq` has
/// no occurrence in `filter` at all, and drops the line marker and source
/// echo rather than inventing a position.
///
/// jq pads the echoed line with trailing spaces (a `%*s` in its own
/// `locfile_locate`); the padding width follows the failing node's start
/// column for a simple undefined name but points elsewhere for an
/// arity mismatch, so this reproduces the column rule rather than every case.
/// It is trailing whitespace either way.
///
/// Diagnostics are reported in `errors`' own order — [`jq::resolve_all`]
/// already walks the tree in source order, and real jq interleaves a
/// program's undefined calls and undefined variables by position rather than
/// grouping by kind (confirmed live: `$bar, foo, $baz` reports all three left
/// to right), so this function must not re-sort or re-group them.
/// Report one unresolved call, in jq's own `name/arity is not defined at
/// {location}[, line N:]` shape. Tries `source`'s own call-site table first
/// (the real, parser-recorded position -- see [`jq::CallSite`], and since
/// #3010 this includes a namespaced call, recorded under its joined
/// `namespace::name` spelling), then a text-search fallback for a call the
/// table does not record -- an `include`d module or `~/.jq` call with no
/// occurrence in `source` at all, or a filter whose own parse differed from
/// the one that built the table -- and finally the bare `at {location}` form
/// if even that fails.
///
/// `taken`/`resume_from` are the caller's own per-name counters (each
/// caller keys its own map, since `filter` and a given module are unrelated
/// namespaces of occurrences), kept in step across *both* the table and
/// text-search paths so a repeated name walks its own successive
/// occurrences rather than re-citing a position already used (#2085
/// review, finding 1) -- shared by both callers below so that discipline
/// cannot drift between them the way it did before this was one function.
fn report_unresolved_call(
    name: &str,
    arity: usize,
    location: &str,
    source: &str,
    call_sites: &[jq::CallSite],
    occurrence_index: usize,
    resume_from: &mut usize,
) {
    // #2955: a link-run pruning gap could in principle leave a stub's target
    // unresolved, surfacing its internal `\0link:<id>::name` spelling here.
    // `display_name` is a no-op for every ordinary name, so this costs
    // nothing on the common path.
    let name = jq::ModuleRun::display_name(name);
    // #2635: `occurrence_index` (from `resolve::UnresolvedCall`, computed
    // while walking the same tree in the same source order) counts *every*
    // earlier call to this `(name, arity)` pair, resolved or not -- not
    // just the earlier ones that also happened to fail, which is what a
    // simple "how many times have *we* reported this name" counter would
    // give. That distinction is exactly what made `(def f: 1; f) | f` cite
    // the resolving `f` inside the parens instead of the failing one after
    // the pipe: both are `f/0`, but only the second is actually this
    // error's own occurrence.
    let from_table = call_sites
        .iter()
        .filter(|c| c.name == name && c.arity == arity)
        .nth(occurrence_index)
        .map(|c| c.offset);
    if let Some(offset) = from_table {
        *resume_from = offset + name.len();
        let (line_no, line_text, column) = line_at_offset(source, offset);
        report_site_error(
            format_args!("{name}/{arity}"),
            location,
            Some((line_no, &line_text, column)),
        );
        return;
    }

    match locate_identifier_from(source, name, *resume_from) {
        Some((line_no, line_text, column, end)) => {
            *resume_from = end;
            report_site_error(
                format_args!("{name}/{arity}"),
                location,
                Some((line_no, &line_text, column)),
            );
        }
        None => {
            report_site_error(format_args!("{name}/{arity}"), location, None);
        }
    }
}

/// Print one "`header` is not defined"-shaped compile-error line against
/// `location`, given the site's resolved `(line_no, line_text, column)` if a
/// lookup found one -- the print body [`report_unresolved_call`],
/// [`report_unbound_var`], and [`report_unresolved_label`] all need, on top
/// of [`print_position_error`]'s own shared caret rendering, so a format
/// change (added column info, a third line, a different caret style) has
/// exactly one definition to update. `header` is `std::fmt::Arguments`
/// rather than an owned `String` so a call site never allocates just to glue
/// a sigil onto a name.
fn report_site_error(
    header: std::fmt::Arguments,
    location: &str,
    site: Option<(usize, &str, usize)>,
) {
    match site {
        Some((line_no, line_text, column)) => {
            print_position_error(
                format_args!("{header} is not defined"),
                location,
                line_no,
                line_text,
                column,
            );
        }
        None => {
            eprintln!("jq: error: {header} is not defined at {location}");
        }
    }
}

/// Report one unbound `$name` against `source` (the main filter, or a
/// module's own text), at its `site_index`-th recorded site -- the variable
/// twin of [`report_unresolved_call`].
///
/// Unlike calls, no text-search fallback: `collect_var_sites` records every
/// `$name` a parse sees (a `$name` token has none of the retry, shadow or
/// builtin-fallback machinery that can make the call-site table diverge from
/// the real parse). A text search could land on a coincidental `$name` in a
/// string or comment, so a missing table entry gets a file-only report.
/// `site_index` is always a resolver-computed occurrence count (#3107), so
/// callers have nothing left to do with whether the table had that site --
/// unlike `report_unresolved_call`'s own fallback-driving `bool`, this one
/// had no remaining reader once that was true for both call sites.
fn report_unbound_var(
    name: &str,
    location: &str,
    source: &str,
    var_sites: &[jq::VarSite],
    site_index: usize,
) {
    let offset = var_sites
        .iter()
        .filter(|v| v.name == name)
        .nth(site_index)
        .map(|v| v.offset);
    let site = offset.map(|offset| line_at_offset(source, offset));
    report_site_error(
        format_args!("${name}"),
        location,
        site.as_ref()
            .map(|(line_no, line_text, column)| (*line_no, line_text.as_str(), *column)),
    );
}

/// Report one out-of-scope `break $name` against `source` (the main filter,
/// or a module's own text), at its `occurrence`-th recorded site --
/// [`report_unbound_var`]'s sibling for labels (#2964).
///
/// `occurrence` is not a counter this function advances: `resolve::check`
/// already counted it while walking the tree (`UnresolvedLabel`'s own doc
/// comment), and a module caller has already narrowed `break_sites` to the
/// def it was counted in, so a single lookup is enough.
fn report_unresolved_label(
    name: &str,
    location: &str,
    source: &str,
    break_sites: &[jq::BreakSite],
    occurrence: usize,
) {
    let site = break_sites
        .iter()
        .filter(|b| b.name == name)
        .nth(occurrence)
        .map(|b| b.offset)
        .map(|offset| line_at_offset(source, offset));
    report_site_error(
        format_args!("$*label-{name}"),
        location,
        site.as_ref()
            .map(|(line_no, line_text, column)| (*line_no, line_text.as_str(), *column)),
    );
}

/// Which kind of compile error a module-site key names, so a call, a variable
/// and a break that happen to share a name and index stay distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SiteKind {
    Call,
    Var,
    Break,
}

/// One module's source, re-read on the compile-error path, with its site
/// tables parsed on first use (#2991, #2962, #2964) and its module-level def
/// spans (#3085).
struct ModuleSource {
    source: String,
    defs: Vec<jq::DefSite>,
    calls: std::cell::OnceCell<Vec<jq::CallSite>>,
    vars: std::cell::OnceCell<Vec<jq::VarSite>>,
    breaks: std::cell::OnceCell<Vec<jq::BreakSite>>,
}

impl ModuleSource {
    fn read(path: &str) -> Option<Self> {
        let source = std::fs::read_to_string(path).ok()?;
        let defs = jq::collect_def_sites(&source, jq::ParserMode::Jq, true);
        Some(Self {
            source,
            defs,
            calls: std::cell::OnceCell::new(),
            vars: std::cell::OnceCell::new(),
            breaks: std::cell::OnceCell::new(),
        })
    }

    fn calls(&self) -> &[jq::CallSite] {
        self.calls
            .get_or_init(|| jq::collect_call_sites(&self.source, jq::ParserMode::Jq, true))
    }

    fn vars(&self) -> &[jq::VarSite] {
        self.vars
            .get_or_init(|| jq::collect_var_sites(&self.source, jq::ParserMode::Jq, true))
    }

    fn breaks(&self) -> &[jq::BreakSite] {
        self.breaks
            .get_or_init(|| jq::collect_break_sites(&self.source, jq::ParserMode::Jq, true))
    }

    /// The part of `sites` (sorted by offset) inside module-level def `def`,
    /// which is what a module diagnostic's occurrence index counts within
    /// (`jq::ModuleDef`). The whole table when the def cannot be found.
    fn within<'s, T>(
        &self,
        sites: &'s [T],
        offset: impl Fn(&T) -> usize,
        def: Option<&jq::ModuleDef>,
    ) -> &'s [T] {
        let span = def.and_then(|def| {
            self.defs
                .iter()
                .filter(|d| d.name == def.name && d.arity == def.arity)
                .nth(def.ordinal)
        });
        match span {
            Some(span) => {
                let lo = sites.partition_point(|t| offset(t) < span.offset);
                let hi = sites.partition_point(|t| offset(t) < span.end);
                &sites[lo..hi]
            }
            None => sites, // omni-dev: coverage tolerate-line reason="unreachable in practice today: `def` is only `None` when `occurrences.module_def()` is `None`, which (given `origin` is `Some`) would require an open run whose innermost frame has no def set at a `check` point that isn't itself inside a module-level def body -- but every module run is a strict chain of `def` nodes (the loader's own defs, each wrapping its dependency stubs INSIDE its own body via `wrap_defs`/`dep_stubs_for`) terminated by the run's own end marker, so a diagnosable call/var/break site is always reached either inside a module-level def's body (module_def Some) or outside every run (origin None) -- and even when `def` is `Some`, `self.defs` is always `ModuleSource::read`'s own re-parse of the exact same file `run_id_for` interned this origin's id from, so `collect_def_sites` always finds the matching (name, arity, ordinal) span. Kept as a defensive fallback rather than a panic/unwrap in case that invariant is ever violated (#3085)"
        }
    }
}

fn report_compile_errors(errors: &[jq::ResolveError], filter: &str, loader: &ModuleLoader) {
    // #3085: a file may be spliced in by several import/include directives,
    // and the resolver visits every copy, but jq diagnoses each physical
    // module site once. A module diagnostic's def and occurrence identify
    // its site the same way in every copy (`jq::ModuleDef`), so a repeat of
    // an already reported key is another copy's report of the same site.
    type ModuleSiteKey = (u32, SiteKind, String, usize, Option<jq::ModuleDef>, usize);
    let mut reported_module_sites: BTreeSet<ModuleSiteKey> = BTreeSet::new();
    let mut duplicates = 0;
    // Byte offset to resume searching from, per name, so a second call to the
    // same undefined name finds its own occurrence rather than repeating the
    // first one's. Used only by the text-search fallback below, which only
    // calls have (see `report_unbound_var`).
    let mut call_resume_from: HashMap<&str, usize> = HashMap::new();

    // #2991: a module-body error's `origin` names a *different* source than
    // `filter`, so it needs its own text and its own site tables -- built
    // lazily, once per module, and cached here rather than up front, since
    // most runs raise no module-body errors at all. `None` marks a module
    // whose file could not be re-read (deleted between load and this error
    // path, or similar); such an error keeps today's file-name-only report
    // rather than panicking or guessing a line.
    let mut module_sources: HashMap<u32, Option<ModuleSource>> = HashMap::new();
    // The module-body counterpart of `call_resume_from` above, for the same
    // text-search fallback -- see the `origin` branch's own doc comment for
    // why a module needs one too.
    let mut module_call_resume_from: HashMap<(u32, &str), usize> = HashMap::new();

    // #2085: real positions for the calls this filter's own text contains.
    // Only consulted on this error path, so the extra parse is never on
    // anyone's hot path -- see `jq::collect_call_sites`.
    let call_sites = jq::collect_call_sites(filter, jq::ParserMode::Jq, true);
    // #2734: the same, for `$name` variable references -- see
    // `jq::collect_var_sites`.
    let var_sites = jq::collect_var_sites(filter, jq::ParserMode::Jq, true);
    // #2964: the same, for `break $name` sites written directly in `filter`
    // -- see `jq::collect_break_sites`. A module-body break uses its
    // module's own table instead, built lazily from that module's source.
    let break_sites = jq::collect_break_sites(filter, jq::ParserMode::Jq, true);

    for error in errors {
        match error {
            jq::ResolveError::Call(jq::UnresolvedCall {
                name,
                arity,
                origin,
                occurrence_index,
                module_def,
            }) => {
                // #2951: a call that failed inside a *module* body has no
                // occurrence in `filter` at all, so neither the call-site
                // table nor the text-search fallback below can honestly
                // place it -- and the fallback can actively mislead, by
                // finding a coincidental occurrence of the same name in the
                // main filter and citing that line. `origin` is the first
                // thing this pass has ever had that distinguishes the two,
                // so a module-body error names the right file instead of
                // guessing.
                //
                // #2991: real jq also names the line and echoes the source,
                // which needs the module's *own* text and its *own*
                // call-site table -- re-derived here (never at load time:
                // this is a cold error path, and most runs never take it) by
                // re-reading the file `run_origin` already resolved.
                // `report_unresolved_call` gives it the same lookup the main
                // filter's own diagnostic uses below, so `def f: ns::g;`
                // inside a module finds `ns::g`'s real occurrence the same
                // way: from the table directly since #3010 (its own
                // `namespace::name`-joined `CallSite`), falling back to a
                // text search only for what the table still cannot record.
                if let Some(id) = origin {
                    let key = (
                        *id,
                        SiteKind::Call,
                        name.clone(),
                        *arity,
                        module_def.clone(),
                        *occurrence_index,
                    );
                    if !reported_module_sites.insert(key) {
                        duplicates += 1;
                        continue;
                    }
                    let at = loader
                        .run_origin(*id)
                        .map_or_else(|| "<module>".to_string(), ToString::to_string);
                    let cached = module_sources
                        .entry(*id)
                        .or_insert_with(|| ModuleSource::read(&at));

                    match cached {
                        Some(module) => {
                            let sites =
                                module.within(module.calls(), |c| c.offset, module_def.as_ref());
                            let resume = module_call_resume_from
                                .entry((*id, name.as_str()))
                                .or_insert(0);
                            report_unresolved_call(
                                name,
                                *arity,
                                &at,
                                &module.source,
                                sites,
                                *occurrence_index,
                                resume,
                            );
                        }
                        None => {
                            let name = jq::ModuleRun::display_name(name);
                            eprintln!("jq: error: {name}/{arity} is not defined at {at}");
                        }
                    }
                    continue;
                }
                let resume = call_resume_from.entry(name.as_str()).or_insert(0);
                report_unresolved_call(
                    name,
                    *arity,
                    "<top-level>",
                    filter,
                    &call_sites,
                    *occurrence_index,
                    resume,
                );
            }
            jq::ResolveError::Var(jq::UnboundVar {
                name,
                origin,
                occurrence,
                module_def,
            }) => {
                // #2962: a module body's unbound variable is reported against
                // that module's own file, as jq does -- the same route the
                // `Call` arm's `origin` branch takes, with the module's own
                // variable sites.
                if let Some(id) = origin {
                    let key = (
                        *id,
                        SiteKind::Var,
                        name.clone(),
                        0,
                        module_def.clone(),
                        *occurrence,
                    );
                    if !reported_module_sites.insert(key) {
                        duplicates += 1;
                        continue;
                    }
                    let at = loader
                        .run_origin(*id)
                        .map_or_else(|| "<module>".to_string(), ToString::to_string);
                    let cached = module_sources
                        .entry(*id)
                        .or_insert_with(|| ModuleSource::read(&at));
                    match cached {
                        Some(module) => {
                            let sites =
                                module.within(module.vars(), |v| v.offset, module_def.as_ref());
                            report_unbound_var(name, &at, &module.source, sites, *occurrence);
                        }
                        None => {
                            eprintln!("jq: error: ${name} is not defined at {at}");
                        }
                    }
                    continue;
                }
                // #2734: the real reference position from `var_sites`, not a
                // text search -- a blind search for
                // `${name}` can land inside a string literal, or on an
                // earlier, genuinely *bound* occurrence of the same name,
                // neither of which is the failing reference. `occurrence`
                // (from `resolve::check`'s `Expr::Var` arm, #3085) already
                // counts every reference to this name in source order, bound
                // ones included, the same way `occurrence_index` does for
                // calls (#2635) -- so this indexes `var_sites` directly,
                // exactly like the module-body branch above already does,
                // rather than approximating it with a failures-only counter
                // that could pick a genuinely bound occurrence instead of
                // the actually-unbound one (#3107; confirmed live:
                // `(. as $x | $x),\n$x` cited line 1 -- the bound occurrence
                // -- instead of jq's own line 2).
                report_unbound_var(name, "<top-level>", filter, &var_sites, *occurrence);
            }
            jq::ResolveError::Break(jq::UnresolvedLabel {
                name,
                occurrence,
                origin,
                module_def,
            }) => {
                // #2964: a break inside a module body is reported against
                // that module's own file, the same route `Call`/`Var` take
                // above -- `occurrence` was counted within `module_def` (see
                // `UnresolvedLabel`'s own doc comment), so it indexes
                // straight into that def's part of the module's break-site
                // table with no separate resume counter needed.
                if let Some(id) = origin {
                    let key = (
                        *id,
                        SiteKind::Break,
                        name.clone(),
                        0,
                        module_def.clone(),
                        *occurrence,
                    );
                    if !reported_module_sites.insert(key) {
                        duplicates += 1;
                        continue;
                    }
                    let at = loader
                        .run_origin(*id)
                        .map_or_else(|| "<module>".to_string(), ToString::to_string);
                    let cached = module_sources
                        .entry(*id)
                        .or_insert_with(|| ModuleSource::read(&at));
                    match cached {
                        Some(module) => {
                            let sites =
                                module.within(module.breaks(), |b| b.offset, module_def.as_ref());
                            report_unresolved_label(name, &at, &module.source, sites, *occurrence);
                        }
                        // omni-dev: coverage tolerate reason="unreachable in a single-process run by construction: `run_id_for` (the sole source of an `origin` id) always inserts a `run_origins` entry for the id it hands back -- from a real load's canonical path, or its own literal-path fallback on a resolve failure -- and a def body only ever gets stamped with an `origin` after its module loaded successfully, so `at` always names a file that existed and was readable moments earlier. Reaching this arm needs that same file to vanish (or become unreadable) in the narrow window between that load and this re-read, entirely outside this process's control (#2964)"
                        None => {
                            eprintln!("jq: error: $*label-{name} is not defined at {at}");
                        } // omni-dev: coverage end
                    }
                    continue;
                }
                // #2964: the real `break` keyword position from
                // `break_sites`, not a text search. The resolver counts the
                // breaks inside unreferenced `def` bodies too (#3085), so
                // the index lines up with this table; see `jq::BreakSite`'s
                // own doc comment for what it still cannot place.
                report_unresolved_label(name, "<top-level>", filter, &break_sites, *occurrence);
            }
        }
    }

    let count = errors.len() - duplicates;
    let noun = if count == 1 { "error" } else { "errors" };
    eprintln!("jq: {count} compile {noun}");
}

/// Find the first occurrence of `name` in `filter`, at or after byte offset
/// `start_from`, that stands alone as an identifier — returning its 1-based
/// line, that line's text, its 0-based column, and the byte offset just past
/// the match (so a caller can resume from there to find the *next*
/// occurrence).
///
/// "Stands alone" means neither neighbour is an identifier character, so a
/// search for `f` does not match the `f` inside `first` — but `::` is allowed
/// on the left, since a namespaced call arrives here as `ns::f` while the
/// source spells the two halves either side of the separator.
fn locate_identifier_from(
    filter: &str,
    name: &str,
    start_from: usize,
) -> Option<(usize, String, usize, usize)> {
    fn is_ident_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }

    let bytes = filter.as_bytes();
    let mut search_from = start_from;
    let start = loop {
        let hit = search_from + filter[search_from..].find(name)?;
        let before_ok = hit == 0 || !is_ident_byte(bytes[hit - 1]);
        let after = hit + name.len();
        let after_ok = after >= bytes.len() || !is_ident_byte(bytes[after]);
        if before_ok && after_ok {
            break hit;
        }
        // Advance by one byte, not by `name.len()`: overlapping candidates
        // (`ff` searched for in `fff`) would otherwise be skipped. `find`
        // returns a char boundary and `name` is a non-empty identifier, so
        // `hit + 1` cannot land mid-character.
        search_from = hit + 1;
    };

    let line_start = filter[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_no = filter[..line_start].matches('\n').count() + 1;
    let line_end = filter[line_start..]
        .find('\n')
        .map_or(filter.len(), |i| line_start + i);

    Some((
        line_no,
        filter[line_start..line_end].to_string(),
        start - line_start,
        start + name.len(),
    ))
}

/// The 1-based line number, that line's text, and the 0-based column, for a
/// byte `offset` into `filter` (#2085).
///
/// The positional counterpart of [`locate_identifier_from`]'s own line
/// arithmetic, for an offset that is already known to be a real call site
/// rather than one that had to be searched for.
fn line_at_offset(filter: &str, offset: usize) -> (usize, String, usize) {
    let line_start = filter[..offset].rfind('\n').map_or(0, |i| i + 1);
    let line_no = filter[..line_start].matches('\n').count() + 1;
    let line_end = filter[line_start..]
        .find('\n')
        .map_or(filter.len(), |i| line_start + i);
    (
        line_no,
        filter[line_start..line_end].to_string(),
        offset - line_start,
    )
}

/// Extract the line containing an error for display.
fn get_error_line(input: &[u8], line: usize, column: usize) -> Option<(String, usize)> {
    let text = String::from_utf8_lossy(input);
    let mut current_line = 1;
    let mut line_start = 0;

    for (i, ch) in text.char_indices() {
        if current_line == line {
            line_start = i;
            break;
        }
        if ch == '\n' {
            current_line += 1;
        }
    }

    if current_line != line && line > 1 {
        return None;
    }

    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |i| line_start + i);

    let line_content = &text[line_start..line_end];

    // Truncate long lines
    let max_width = 80;
    let (display_content, caret_offset) = if line_content.len() > max_width {
        let error_col = column.saturating_sub(1);
        if error_col < max_width / 2 {
            let truncated = &line_content[..max_width.min(line_content.len())];
            (format!("{truncated}..."), error_col)
        } else {
            let start = error_col.saturating_sub(max_width / 2);
            let end = (start + max_width).min(line_content.len());
            let truncated = &line_content[start..end];
            let pos_in_truncated = error_col.saturating_sub(start);
            (format!("...{truncated}..."), pos_in_truncated + 3)
        }
    } else {
        (line_content.to_string(), column.saturating_sub(1))
    };

    Some((display_content, caret_offset))
}

/// Run the jq command with the given arguments.
/// Returns the exit code (0 for success, non-zero for various errors).
pub fn run_jq(args: JqCommand) -> Result<i32> {
    // Handle --version flag
    if args.version {
        println!(
            "succinctly jq - JSON processor [version {}]",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(exit_codes::SUCCESS);
    }

    // Handle --build-configuration flag
    if args.build_configuration {
        output::print_build_configuration("jq");
        return Ok(exit_codes::SUCCESS);
    }

    jq::cli_context::set_program(program_context(&args));

    // Build evaluation context from arguments. A bad --argjson/--slurpfile/
    // --rawfile is jq's own usage error (exit 2, a single `jq: ...` line),
    // not the generic `anyhow`-reported exit 1 every other `Other` failure
    // here still takes (#3051).
    let context = match build_context(&args) {
        Ok(context) => context,
        Err(BuildContextError::Usage(message)) => {
            report_usage_error(&message);
            return Ok(exit_codes::USAGE_ERROR);
        }
        Err(BuildContextError::Other(e)) => return Err(e),
    };

    // Get the filter expression. An unreadable `-f`/`--from-file` file is
    // jq's own usage error (exit 2), so it gets the same two-arm treatment
    // as the build_context failure above -- mirror that match, don't drift
    // from it (#3098).
    let filter_str = match get_filter(&args) {
        Ok(filter_str) => filter_str,
        Err(BuildContextError::Usage(message)) => {
            report_usage_error(&message);
            return Ok(exit_codes::USAGE_ERROR);
        }
        Err(BuildContextError::Other(e)) => return Err(e),
    };

    // The main filter is parsed *twice* (#2395 -- see the re-parse below), and
    // the second result is the one that runs, so a drift between the two in
    // mode or extensions would be silent: the first parse's shape would simply
    // be discarded. One closure therefore owns that choice for both calls
    // rather than each spelling it out. It reproduces `jq::parse_program`'s own
    // defaults (`ParserMode::Jq`, extensions off -- `jq_extensions` is ignored
    // in jq mode regardless), which is what this call site used before the
    // second parse existed.
    let parse_filter = |extra: &BTreeSet<String>| {
        jq::parse_program_with_extra_shadowable_defs(&filter_str, jq::ParserMode::Jq, false, extra)
    };

    // Parse the filter as a full program (with module directives).
    //
    // Returns the exit code rather than an `anyhow` error (#1473): a failed
    // parse is jq's compile error, exit 3, and routing it through `anyhow`
    // gave exit 1 *and* printed a second, stray `Error: compile error` line
    // from `main`'s own reporting. Both were confirmed live against jq 1.7.1,
    // which exits 3 with no such line. Nothing else distinguished this arm
    // from a resolution failure below, so the two now answer identically.
    let program = match parse_filter(&BTreeSet::new()) {
        Ok(program) => program,
        Err(e) => {
            report_syntax_error(&e.message, &filter_str, e.position, "<top-level>", false);
            return Ok(exit_codes::COMPILE_ERROR);
        }
    };

    // Create module loader and process imports/includes.
    //
    // The third compile-error kind, and it left the same way as the parse
    // failure above (#1473): jq exits 3 for an unresolvable `include`/`import`
    // (verified against 1.7.1), where routing through `anyhow` gave exit 1 and
    // a stray `Error: module error` line. Leaving this one behind would make
    // the runner disagree with itself -- a filter that fails to parse, one
    // that names an undefined function, and one that includes a missing module
    // are all "jq could not compile this program".
    let mut module_loader = ModuleLoader::new(&args.library_path);

    // #2395: re-parse the filter once the module-sourced def names are known,
    // so a `def` arriving through `include` or `~/.jq` can shadow a builtin
    // exactly as one written in the filter itself already does (#2036).
    //
    // The shadow-candidate set is a scan of the text being parsed, so the
    // parse above -- the only one that can tell us *which* modules to load --
    // is necessarily blind to their contents. Rather than rewrite the
    // assembled tree afterwards (which would need a `Builtin` -> name map the
    // crate does not have, and could not recover `range`'s arity, whose sugar
    // marker is itself emitted only for a shadow candidate), feed the names
    // back into a second parse.
    //
    // Skipped entirely when no module contributes a name -- including every
    // filter with no `include` and no `~/.jq`, which is the overwhelmingly
    // common case and pays nothing.
    let module_def_names = match module_loader.unqualified_def_names(&program) {
        Ok(names) => names,
        Err(e) => {
            report_module_load_error(&e);
            return Ok(exit_codes::COMPILE_ERROR);
        }
    };
    let program = if module_def_names.is_empty() {
        program
    } else {
        // Widening the candidate set cannot make a program stop parsing, so
        // this second parse succeeds whenever the first did.
        //
        // At a name the set newly covers, the parser's dedicated parse still
        // runs first and, on success, is merely wrapped as a fallback -- no
        // token is consumed differently and `pos` does not move. The only
        // other outcome is that the dedicated parse *fails*, and there the
        // widening is what rescues it: a candidate retries as a generic call
        // (`retry_shadow_candidate_as_generic_call`) where a non-candidate
        // propagates the error outright -- meaning the first parse would have
        // failed at that same site. `SHADOW_RETRY_BUDGET` cannot flip the
        // conclusion either, since it is charged only at those failing sites,
        // which are therefore the identical set in both parses.
        //
        // The arm below is kept rather than unwrapped anyway: if that
        // reasoning is ever wrong, falling back to the already-parsed program
        // costs only this filter's module-sourced shadowing -- exactly the
        // behavior before this fix -- where an error would reject a filter
        // that compiles today and that real jq accepts.
        parse_filter(&module_def_names).unwrap_or(program) // omni-dev: coverage tolerate-line reason="unreachable: widening the shadow-candidate set never rejects a program the first parse accepted -- a newly covered name only wraps an already-successful dedicated parse, and a failing one would have propagated its error in the first parse too, so the retry budget is charged at the identical sites in both (#2395)"
    };

    let expr = match module_loader.process_program(&program) {
        Ok(expr) => expr,
        Err(e) => {
            report_module_load_error(&e);
            return Ok(exit_codes::COMPILE_ERROR);
        }
    };

    // Build the $ARGS special variable
    let args_value = build_args_var(&context);

    // Substitute variables from context into the expression
    // First substitute regular named variables, then add $ARGS
    let mut all_vars: Vec<(&str, &OwnedValue)> =
        context.named.iter().map(|(k, v)| (k.as_str(), v)).collect();
    all_vars.push(("ARGS", &args_value));

    let mut expr = jq::substitute_vars(&expr, all_vars);

    // #1473: resolve every function call against the `def`s, parameters and
    // builtins in scope at its position, exactly as real jq's compiler does —
    // before any input is read, unconditionally, and beyond the reach of any
    // `try`/`?` in the filter.
    //
    // Placed *after* `process_program`, which is what inlines
    // `include`/`import`/`~/.jq` definitions as `FuncDef` wrappers and rewrites
    // `ns::f` into a matching `FuncCall`; running it earlier would report every
    // module function as undefined. `substitute_vars` above substitutes
    // `OwnedValue`s, never sub-expressions, so it cannot introduce a call and
    // running after it rather than before is equivalent.
    //
    // #2734: `resolve_all` also rejects an unbound `$variable` reference at
    // this same compile stage, in the same walk -- real jq resolves `$name`
    // at compile time exactly like a function call (`$nope` alone is `jq: 1
    // compile error`, exit 3, zero output; `succinctly jq` previously let it
    // reach evaluation and error there instead, at exit 5, after any earlier
    // output had already been written). yq mode does not go through this
    // function at all (`yq_runner.rs` calls `resolve_func_calls`, the
    // call-only view) -- see that function's own doc comment for why: real
    // yq is fully permissive about unbound variables (#2981), the opposite
    // direction of divergence from what this fixes here.
    let compile_errors = jq::resolve_all(&mut expr);
    if !compile_errors.is_empty() {
        report_compile_errors(&compile_errors, &filter_str, &module_loader);
        return Ok(exit_codes::COMPILE_ERROR);
    }

    // Whether the filter references `input`/`inputs`/`input_line_number`
    // (#723), which decides below whether the shared input queue gets seeded
    // at all.
    //
    // Deliberately computed *here* rather than at the top of this function:
    // it must see the expanded tree. `module_loader.process_program` above is
    // what inlines a `-L`/`import`/`include`-loaded module body, so a call
    // that exists only inside an imported module's own function -- never
    // spelled out in `filter_str` -- becomes visible only at this point. The
    // substring scan of `filter_str` this replaced missed exactly that case,
    // and the miss was not benign: the unseeded queue made every document
    // report spurious exhaustion (#1309, oracle-confirmed against jq 1.7.1).
    //
    // `substitute_vars` cannot introduce one of these builtins -- it
    // substitutes `OwnedValue`s, not sub-expressions -- so walking after it
    // rather than immediately after `process_program` is equivalent, and
    // keeps this to a single `expr` binding.
    //
    // Exact in both directions now, where the substring scan over-reported as
    // well as under-reported: `.input`, `.inputs` and an `"input"` string
    // literal no longer force the non-lazy read path.
    let uses_input_builtins = jq::walk::uses_input_builtins(&expr);

    // Configure output
    let output_config = OutputConfig::from_args(&args);

    // #1576: the jq-side M2 fast path -- `can_use_m2_streaming` (shared
    // with `yq_runner.rs`, `m2_gate.rs`) is the AST-shape half of the gate;
    // the flag exclusions below are its own, scoped narrower than yq's for
    // this first slice. `-S`/color/ascii/`--unbuffered` aren't excluded
    // because the underlying writer can't do them (it can: `JsonCursor`'s
    // new writer honors `sort_keys`, and `-C`/`-a`/`--unbuffered` are
    // orthogonal) -- they're excluded because this path only ever runs
    // nested inside `can_use_lazy_path`'s own file-reading loop below,
    // which already excludes `sort_keys`/`color_output`/`ascii_output` for
    // reasons unrelated to this issue; breaking those out into their own
    // top-level branch (duplicating that loop's file/multi-document/
    // line-counting machinery) is a well-scoped, low-risk follow-up rather
    // than something this change needs to do in one PR. `-r`/`-j`/
    // `--raw-output0` are excluded because no M2 JSON streamer here
    // implements raw-string unquoting, matching `yq_runner.rs`'s own
    // identical exclusion (#1715); `--unbuffered` because this path's own
    // per-document write doesn't replicate `write_terminator`'s per-value
    // flush. Named variables (`--arg`/`--argjson`) are excluded because
    // `eval_with_cursor` only takes an expression and a cursor, with no
    // variable-binding context threaded through the way the DOM path's
    // `context.named` machinery provides.
    //
    // `m2_json_fallback_safe` narrows `can_use_m2_streaming`'s own AST
    // whitelist further, jq-only (yq's own gate in `yq_runner.rs` is
    // unaffected): only an expression guaranteed to produce *at most one*
    // top-level result may take this path, because `evaluate_m2_fast_path`
    // falls back to the general path on any detected malformation rather
    // than trying to replicate jq's own issue-by-issue-tuned malformed-
    // input behavior (see that function's own doc comment) -- safe only
    // when nothing has been written to `out` yet for *this* document. For
    // `.[]`/`Iterate` (excluded here) or `select` with a fan-out `cond`
    // (also excluded, conservatively), a later result's failure could
    // follow already-written earlier ones, and falling back would
    // duplicate them.
    // #2662: `evaluate_m2_fast_path` takes `sort_keys` directly -- its
    // handling was already correct, just newly *exercised* here for the
    // first time, since `sort_keys` could never reach this gate before
    // (the outer `can_use_lazy_path` excluded it outright). It has no
    // `color`/`ascii` handling of its own, though: `color_output` and
    // `ascii_output` route to the general lazy path below instead, which
    // applies both by materializing the one output value and reusing
    // `format_json`'s existing `ascii`/color options (see
    // `write_output_jq_value`'s own comment on that branch).
    let can_json_fast_path = can_use_m2_streaming(&expr)
        && m2_json_fallback_safe(&expr)
        && !output_config.raw_output
        && !output_config.join_output
        && !output_config.raw_output0
        && !output_config.unbuffered
        && !output_config.color_output
        && !output_config.ascii_output
        && context.named.is_empty();

    // Indent width/unit for the fast path's streamer -- built directly from
    // `args`, not `output_config.indent_string` (a pre-rendered string),
    // mirroring `yq_runner.rs`'s own M2 setup and its own reasoning: a
    // streamer needs the semantic width/unit pair, not rendered text.
    // Priority matches `OutputConfig::from_args`'s `indent_string`
    // construction exactly (tab > explicit `--indent N` > `-c` > default 2).
    let json_indent = if args.tab {
        IndentSpec {
            width: 1,
            unit: '\t',
        }
    } else if let Some(n) = args.indent {
        IndentSpec::spaces(n as usize)
    } else if args.compact_output {
        IndentSpec::COMPACT
    } else {
        IndentSpec::spaces(2)
    };
    // #2874: was this same two-way derive-from-bool, one of the four
    // independent encodings of the axis. `OutputConfig::from_args` now owns
    // the decision (including the #2209 `JqPreserveInput`-never-`Preserve`
    // rule, recorded there) and this site just reads it.
    let json_numbers = output_config.convention;

    // Set up output writer
    let stdout = std::io::stdout();
    let mut out = LoudFlushWriter::new(stdout.lock());

    // Track last output for exit status
    let mut last_output: Option<OwnedValue> = None;
    let mut had_output = false;
    // Uncaught evaluation errors. Evaluation continues past one (as jq does),
    // so the failure is remembered here and turned into exit 5 below (#355).
    let mut sink = ErrorSink::default();

    // Validate DSV delimiter if provided
    if let Some(delim) = args.input_dsv {
        validate_dsv_delimiter(delim)?;
    }

    // Streaming DSV path: process DSV without materializing all rows into memory.
    // This uses the DSV cursor to iterate rows and writes JSON arrays directly to output.
    // Memory usage: file bytes + DSV index (~3-4% overhead) + small output buffer.
    if let Some(delimiter) = args.input_dsv {
        if !args.slurp && !args.null_input && !uses_input_builtins {
            // Streaming mode: process each row independently
            let files = get_input_files(&args);
            // `input_filename` on this route too (#3046 review).
            jq::cli_context::set_input_names(input_names(&files));
            let raw_inputs: Vec<Vec<u8>> = if files.is_empty() {
                vec![read_stdin_bytes()?]
            } else {
                match files
                    .iter()
                    .map(|path| read_file_bytes(path))
                    .collect::<std::result::Result<Vec<_>, InputFileOpenError>>()
                {
                    Ok(raw) => raw,
                    Err(unopenable) => return Ok(report_unopenable_input(&unopenable)),
                }
            };

            for (file_idx, raw) in raw_inputs.into_iter().enumerate() {
                jq::cli_context::set_current_source(u32::try_from(file_idx).ok());
                let file = files.get(file_idx).map(|p| p.to_string_lossy().to_string());

                // Build DSV index (memory-efficient with SIMD)
                let config = DsvConfig::default().with_delimiter(delimiter as u8);
                let index = build_dsv_index(&raw, &config);

                // Stream rows using the cursor - no materialization of all rows
                let rows = DsvRows::new(&raw, &index);

                for (row_idx, row) in rows.enumerate() {
                    // One row per line, so the row index is the line. Approximate
                    // for fields containing an embedded newline; jq has no DSV
                    // input mode, so there is no oracle to match here anyway.
                    // This path is gated on `!uses_input_builtins`, so nothing
                    // here can move the shared queue's position: fixed.
                    let at = ErrorAt::Fixed(InputLocation::at(file.as_deref(), row_idx + 1));
                    // Build JSON array for this row and write directly
                    let fields: Vec<OwnedValue> = row
                        .fields()
                        .map(|field| {
                            let field_str = strip_quotes_and_decode(field);
                            OwnedValue::String(field_str)
                        })
                        .collect();

                    let row_value = OwnedValue::array_from(fields);

                    // Evaluate expression on this row, streaming (#1653):
                    // each output must reach stdout before the next one is
                    // evaluated, or a mid-stream `debug`/`stderr`/`error`
                    // side effect lands ahead of output that preceded it.
                    evaluate_input_streaming(
                        &row_value,
                        &expr,
                        &context,
                        &at,
                        &mut sink,
                        &output_config,
                        &mut |sink, result| {
                            had_output = true;
                            if args.exit_status {
                                last_output = Some(result.clone());
                            }
                            let stop = route_write_error(
                                sink,
                                &mut out,
                                || at.resolve(),
                                |o| write_output_owned_value(o, &result, &output_config),
                            )?;
                            Ok(!stop)
                        },
                    )?;
                    // halt/halt_error (#791) outranks everything else,
                    // including remaining rows/files still to process.
                    if let Some(code) = sink.halted() {
                        out.flush()?;
                        return Ok(code);
                    }
                    // row_value is dropped here, freeing memory for this row
                }
            }

            out.flush()?;

            // Determine exit code. An uncaught error outranks -e: jq's 5 says
            // the filter failed, -e's 1/4 describe an otherwise-successful
            // result that happened to be falsy (#355 vs #178).
            if sink.hit() {
                return Ok(DiagStyle::Jq.error_exit_code());
            }
            if args.exit_status {
                if !had_output {
                    return Ok(exit_codes::NO_OUTPUT);
                }
                if let Some(last) = last_output {
                    if matches!(last, OwnedValue::Null | OwnedValue::Bool(false)) {
                        return Ok(exit_codes::FALSE_OR_NULL);
                    }
                }
            }

            return Ok(exit_codes::SUCCESS);
        }
        // Fall through to original path for slurp mode
    }

    // The lazy path preserves number formatting and uses less memory.
    // It's available when:
    // - Not using features that require serde_json parsing (slurp, raw_input, seq input, dsv)
    // - Not using input/inputs/input_line_number (#723): those need the
    //   "original" path's own already-materialized Vec<OwnedValue> to share
    //   with the shared input queue below; the lazy path never builds one.
    // Both the jq convention (reformatting numbers) and preserve mode (keeping original formatting)
    // use the lazy path for correctness.
    //
    // #2662: `sort_keys`/`color_output`/`ascii_output` used to be excluded
    // here too, sending every `-S`/`-C`/`-a` invocation to the
    // materializing path below (which validates the whole document before
    // the filter runs, #2103's own divergence from the default route).
    // None of the three actually need materialization of the *input*:
    // `write_output_jq_value` materializes just the one *output* value
    // when any of the three is set, reusing `format_json`'s existing
    // `sort_keys`/`ascii`/color options (already correct on the
    // pre-existing DOM route) rather than a separate streaming-writer
    // mechanism -- unlike `yq_runner.rs`'s own `AsciiEscapeWriter`
    // wrapping (#1700) for its M2 streamers, which this file does not use
    // at all (`print_json`'s `Out` is bound to `std::io::Write`, not the
    // `core::fmt::Write` `AsciiEscapeWriter` implements, so reusing it here
    // would need its own adapter -- not attempted by this change).
    let can_use_lazy_path = !args.slurp
        && !args.raw_input
        && args.input_dsv.is_none()
        && !args.seq // seq input mode parses differently
        && !uses_input_builtins;

    if can_use_lazy_path && !args.null_input {
        // Lazy path: read files as raw bytes and process directly
        // This preserves original number formatting like "4e4"
        let files = get_input_files(&args);
        // `input_filename`'s names for this route's source tags, which are the
        // indexes into `raw_inputs` below (#3046).
        jq::cli_context::set_input_names(input_names(&files));
        let raw_inputs: Vec<Vec<u8>> = if files.is_empty() {
            vec![read_stdin_bytes()?]
        } else {
            match files
                .iter()
                .map(|path| read_file_bytes(path))
                .collect::<std::result::Result<Vec<_>, InputFileOpenError>>()
            {
                Ok(raw) => raw,
                Err(unopenable) => return Ok(report_unopenable_input(&unopenable)),
            }
        };
        // Substitution is skipped under `--validate` so the strict validator
        // in the loop below still sees the *original* bytes (#1247).
        // Substituting first silently repaired a non-UTF-8 document, leaving
        // the one check `--validate` exists to perform with nothing to find:
        // `sjq --validate` exited 0 where `succinctly json validate` on the
        // same file still exits 1. Nothing is lost by skipping it here --
        // any document that would have been substituted is a document
        // `validate_json_input` rejects.
        let raw_inputs: Vec<Vec<u8>> = if args.validate {
            raw_inputs
        } else {
            raw_inputs.into_iter().map(utf8_lossy_document).collect()
        };

        // Check if we can use the identity fast path (raw bytes output, no materialization)
        let use_identity_fast_path = expr.is_identity() && output_config.can_use_raw_identity();
        // #1653 introduced streaming, gated so only a cursor-transparent
        // filter took it; #2103 dropped the gate, so every M2 filter streams
        // unconditionally now. That means a filter validates only what it
        // reads: `1+1` on a malformed document answers `2` at exit 0 where
        // jq 1.7.1 exits 5. See "Every M2 filter streams" in
        // `docs/compliance/jq/limitations.md` for the full list of affected
        // rows and the reasoning, and `scripts/jq-m2-streaming-sweep.sh` for
        // the sweep that measures them.

        for (idx, raw) in raw_inputs.iter().enumerate() {
            jq::cli_context::set_current_source(u32::try_from(idx).ok());
            let filename: Option<String> = files.get(idx).map(|p| p.to_string_lossy().to_string());
            // Validate JSON if --validate flag is set
            if args.validate {
                if let Err(exit_code) = validate_json_input(raw, filename.as_deref()) {
                    // Every other return from this loop that can run after
                    // `out` has already buffered real output flushes
                    // explicitly first (#1563; review: not a blanket claim
                    // about the whole function -- the separate materializing
                    // branch below has its own early returns that provably
                    // run before anything is ever written to `out`, so
                    // nothing to flush there). This one used to rely on
                    // `out`'s own `Drop` impl to flush any already-buffered
                    // output from files processed before this one, which
                    // works today but silently swallows a flush error (e.g.
                    // a closed stdout) instead of propagating it, unlike
                    // every sibling return path in this loop.
                    out.flush()?;
                    return Ok(exit_code);
                }
            }
            // Process as JSON stream (handle multiple JSON values in one input).
            // Every value before a malformed one is still processed, and the
            // parse error is reported after them, as jq's incremental parser
            // does (#2961) -- see the report below the per-value loop.
            let (values, split_error) = split_json_values(raw);
            // `values`' end offsets are non-decreasing (find_json_values is
            // a single left-to-right scan), so one LineCounter shared across
            // every value in this file keeps the whole loop O(n) (#1213).
            let mut line_counter = LineCounter::new(raw);
            for (start, end) in values {
                let json_bytes = &raw[start..end];

                // Fast path for identity query: output raw bytes directly without materialization.
                // This avoids building the index and materializing JqValue, saving significant memory.
                if use_identity_fast_path {
                    had_output = true;
                    // For exit_status, inspect the raw JSON token so that `null`
                    // and `false` inputs still produce the falsy exit code (jq: 1)
                    // on the identity fast path. Only these two literals are falsy;
                    // a quoted "false"/"null" string or any number stays truthy.
                    if args.exit_status {
                        last_output = Some(identity_exit_status_value(json_bytes));
                    }
                    output_config.write_identity_bytes(&mut out, json_bytes)?;
                    out.write_all(b"\n")?;
                    continue;
                }

                // Slow path: build index and evaluate expression
                let index = JsonIndex::build(json_bytes);
                // jq names the line the input value ends on, counted in the
                // whole file rather than in this value's slice.
                let at = InputLocation::at(filename.as_deref(), line_counter.advance_to(end));

                // #1576: the M2 fast path, mirroring `yq_runner.rs`'s own
                // (`can_use_m2_streaming`/`GenericResult::stream_json`) but
                // scoped to jq's own atomicity contract for array
                // construction (see `evaluate_m2_fast_path`'s own doc
                // comment for why `map`/`sort`/etc. buffer while plain
                // navigation streams straight to `out`). Wrapped in the
                // same `catch_unwind` as the general path below, for the
                // same #1793 reason -- `eval_with_cursor`'s own fallback
                // arms (`LazyKeys`'s sorted branch, `sequence_streamable_
                // cursors`'s `None` fallback) can still reach
                // `to_owned_cursor_at_depth`'s panic guard.
                if can_json_fast_path {
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        evaluate_m2_fast_path(
                            json_bytes,
                            &expr,
                            &index,
                            &mut sink,
                            &mut out,
                            json_indent,
                            args.sort_keys,
                            json_numbers,
                            args.exit_status,
                            &mut had_output,
                            &mut last_output,
                        )
                    }));
                    // `Ok(true)`: written (or halted), move on to the next
                    // document. `Ok(false)`: this document was malformed --
                    // `evaluate_m2_fast_path`'s own doc comment explains why
                    // nothing was written and why falling through to the
                    // general path below (rather than reporting from here)
                    // is the right way to handle it (#1576 review). The
                    // panic arm is unrelated to that distinction -- an
                    // adversarial-depth panic is already fully handled here,
                    // same as before this fast path existed.
                    let handled = match outcome {
                        Ok(handled) => handled?,
                        Err(payload) => {
                            let Some(message) = nesting_depth_panic_message(&*payload) else {
                                std::panic::resume_unwind(payload);
                            };
                            sink.report(DiagStyle::Jq, &EvalError::new(message), &at);
                            true
                        }
                    };
                    if let Some(code) = sink.halted() {
                        out.flush()?;
                        return Ok(code);
                    }
                    if handled {
                        continue;
                    }
                    // Else: fall through to the general path just below,
                    // for this one document only.
                }

                // A builtin with no native lazy fast path (`sort`, `join`,
                // ...) falls back to a full `to_owned_cursor` materialization
                // of whatever value it's handed -- for a bare `sort`/`join`
                // piped directly off `.`, that's the whole document. Unlike
                // `#1194`-style malformed-document errors (handled a few
                // lines below via `sink.report` + `continue`, isolating just
                // this one document), an adversarially/accidentally deep
                // document here used to escape as an uncaught panic instead,
                // aborting the whole run and silently dropping every
                // subsequent document in the stream (#1793). `catch_unwind`
                // isolates it the same way #1194's own error already is,
                // without touching `to_owned_cursor_at_depth`'s panic itself
                // -- that guard's docs (`eval_generic.rs`, above
                // `assert_nesting_depth`) explain why threading this through
                // `EvalError` at its 58+ interior call sites was tried and
                // reverted (#1021): only the two outermost call sites
                // reachable from ordinary CLI usage are wrapped here, not
                // the guard itself.
                //
                // The write now happens *inside* the guard, because a
                // streaming filter panics from within the sink callback
                // rather than before the caller's loop starts. That puts
                // `out`, `sink`, `had_output` and `last_output` under
                // `AssertUnwindSafe`, which stays sound: `write_output_jq_value`
                // below is panic-free for depth as of #2850 (`try_materialize`,
                // not the panicking `materialize`), but `evaluate_bytes_streaming`
                // above it is not -- `to_owned_cursor_at_depth`'s own panic
                // (sort/join, described just above) and `eval_generic.rs`'s
                // path-tracking `assert_nesting_depth` calls (`path()`/
                // `setpath`/`del()`/assignment operators on deep input) both
                // still reach the panicking guard during *evaluation*, ahead
                // of this closure's write. `out`'s writer is not mid-record
                // when either unwinds, and the other three are plain data
                // regardless.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut on_value = |sink: &mut ErrorSink,
                                        result: OutputItem<'_>|
                     -> Result<bool> {
                        had_output = true;
                        // For exit_status tracking, we need to check the last value
                        if args.exit_status {
                            match &result {
                                // `-e` is the flag that forces materialization at
                                // all, so it is where a decode failure first
                                // becomes observable here (#1247). Report and skip
                                // the value rather than letting an undecodable
                                // string count as a truthiness answer; `sink`
                                // drives the exit code. `try_materialize`, not
                                // `materialize` (#2850): this call used to leak
                                // Rust's raw panic backtrace to stderr past
                                // `MAX_NESTING_DEPTH`/`MAX_VALUE_TREE_DEPTH`
                                // levels of nesting, ahead of the clean,
                                // correctly exit-5'd diagnostic this `match`
                                // already produces below -- a CLI-output
                                // boundary has no business panicking for input
                                // depth alone.
                                OutputItem::Lazy(v) => match v.try_materialize() {
                                    Ok(owned) => last_output = Some(owned),
                                    Err(e) => {
                                        sink.report(DiagStyle::Jq, &e, &at);
                                        return Ok(true);
                                    }
                                },
                                // An owned result records its *truthiness*, not
                                // itself (#3009). Both readers of `last_output`
                                // only ever ask `matches!(last, Null |
                                // Bool(false))`, which is why the M2 path
                                // already synthesizes a `Bool` here rather than
                                // materializing (`record_exit_status`). Cloning
                                // the value instead would be a refcount bump in
                                // the shipped build but a deep copy under the
                                // `unshared-containers` holdout, i.e. a cost
                                // that shows up only in the configuration used
                                // to measure this change. There is no `Err`
                                // counterpart: an owned value cannot fail to
                                // materialize, and its depth was settled by
                                // `to_jq_values` before it got here.
                                OutputItem::Owned(v) => {
                                    last_output = Some(OwnedValue::Bool(!matches!(
                                        v,
                                        OwnedValue::Null | OwnedValue::Bool(false)
                                    )));
                                }
                            }
                        }
                        // A malformed document is a data error, not an I/O
                        // one: it belongs in jq's diagnostic channel (exit 5)
                        // rather than aborting the process through `anyhow`
                        // (#1194). Stop emitting results for *this* document,
                        // but fall through to the halt check below and carry
                        // on with the rest of the stream (#355) -- real jq
                        // stops at the first parse error instead, a
                        // divergence recorded in
                        // `docs/compliance/jq/limitations.md`.
                        let stop = route_write_error(
                            sink,
                            &mut out,
                            || at.clone(),
                            |o| match &result {
                                OutputItem::Lazy(v) => write_output_jq_value(o, v, &output_config),
                                OutputItem::Owned(v) => {
                                    write_output_owned_value(o, v, &output_config)
                                }
                            },
                        )?;
                        Ok(!stop)
                    };
                    evaluate_bytes_streaming(
                        json_bytes,
                        &expr,
                        &index,
                        &at,
                        &mut sink,
                        &mut on_value,
                    )
                }));
                match outcome {
                    Ok(written) => written?,
                    Err(payload) => {
                        // `&*payload`, not `&payload` -- `payload` is a
                        // `Box<dyn Any + Send>`, and a bare `&payload`
                        // coerces to `&(dyn Any + Send)` by treating the
                        // *Box* as the concrete type under test (`Box<T>`
                        // is itself `Any` via a blanket impl), not the value
                        // it holds -- so every `downcast_ref` inside the
                        // helper would silently miss regardless of the
                        // panic's real payload type. Confirmed live: `&payload`
                        // here made `nesting_depth_panic_message` return
                        // `None` even for this exact panic.
                        let Some(message) = nesting_depth_panic_message(&*payload) else {
                            // Not the specific guard this catch exists for --
                            // an unexpected panic must still crash loudly,
                            // not be silently absorbed as if it were #1793.
                            std::panic::resume_unwind(payload);
                        };
                        // Reported through the same channel, and so with
                        // the same exit code (5), as any other uncaught
                        // EvalError on this loop -- deliberately: this is an
                        // internal architectural ceiling, not a jq-level
                        // type/arity error, but the alternative (keeping the
                        // panic's own exit 101) would still be a hard,
                        // uncontrolled process exit for what #1793 exists to
                        // turn into an ordinary, recoverable diagnostic.
                        sink.report(DiagStyle::Jq, &EvalError::new(message), &at);
                        // Mirrors the same check a few lines below, after the
                        // ordinary per-result loop -- halt/halt_error (#791)
                        // outranks everything else, including remaining
                        // values/files, and this `continue` would otherwise
                        // skip straight past that check for this iteration
                        // (review: every other early-exit in this loop still
                        // reaches it on the same iteration; this one didn't).
                    }
                }
                // halt/halt_error (#791) outranks everything else, including
                // remaining values/files still to process. The panic arm above
                // used to `continue` past this check and repeat it itself;
                // with the write loop folded into the guard there is nothing
                // left between the two, so the single check below now covers
                // both paths.
                if let Some(code) = sink.halted() {
                    out.flush()?;
                    return Ok(code);
                }
            }
            // The malformed value, reported only once everything before it
            // has been processed. Moving on to the next file afterwards is
            // the recorded #355 continue-past-error divergence (jq stops the
            // whole stream); only the lost prefix was #2961.
            if let Some(offset) = split_error {
                let at = InputLocation::at(filename.as_deref(), line_at(raw, offset));
                sink.report(DiagStyle::Jq, &EvalError::new("Invalid JSON text"), &at);
            }
        }
    } else {
        // The materializing path: reads every document up front into a
        // `Vec<OwnedValue>`, which is what the input-builtin queue below needs
        // to seed from and what `--slurp`/`-R`/`--seq` need in order to
        // combine or reshape the whole stream.
        //
        // This comment used to say "parse through serde_json (loses number
        // formatting)". Both halves went stale: `parse_json_stream` routes
        // through the crate's own fidelity-preserving semi-indexer since
        // #1058/#1093, so this path preserves number literals exactly as the
        // lazy path does. Verified rather than assumed while removing #1309's
        // false-positive detection: a field access or string literal that
        // merely spells "input" (`.input`, `.inputs`, `"input"`) no longer
        // gets misrouted into this materializing path by that stale
        // substring scan -- it now correctly stays on the lazy path above,
        // where `test_jq_field_named_input_is_not_an_input_builtin_1309`
        // pins `1.10`, `4E+4` and `2.50` round-tripping unchanged. This
        // path's own fidelity (verified separately, not by that test) is
        // what makes the routing change itself a non-issue either way.
        //
        // `force_read_under_null_input` narrows `uses_input_builtins` by one
        // safety check: never force a real read under `-n` when stdin is an
        // interactive terminal with no files given. Since #1309 the check
        // itself is exact -- an AST walk, not the substring scan that used to
        // fire on a `.input` field -- so this narrowing no longer exists to
        // contain false positives. It covers the genuine case: `-n 'inputs'`
        // typed at a bare prompt would otherwise block reading a TTY the user
        // may not have meant to feed. `input`/`inputs` then correctly report
        // exhausted against an empty queue instead of hanging; the only data
        // lost is data the user intended to type in live rather than
        // pipe/redirect, an already-unusual way to invoke `-n`.
        let force_read_under_null_input = should_force_read_under_null_input(
            uses_input_builtins,
            args.null_input,
            get_input_files(&args).is_empty(),
            std::io::stdin().is_terminal(),
        );
        let (inputs, locations, trailing_error) =
            match get_inputs(&args, force_read_under_null_input) {
                Ok(Ok(inputs)) => inputs,
                // A malformed or undecodable document is a data error, so it goes
                // out in jq's own diagnostic shape at exit 5 rather than through
                // `anyhow` at exit 1 with an `Error:` prefix jq never prints
                // (#1194). Everything else here really is I/O and keeps the
                // `anyhow` path.
                //
                // Line 0, the same placeholder the lazy path prints when it has
                // no better position. Only `--slurp` still fails here: it reads
                // the whole stream into one value, so there is no document
                // position to name. A non-slurped stream keeps its clean prefix
                // and reports its parse error after it instead (#2961).
                Ok(Err(e)) => {
                    // A main input `get_inputs` couldn't open is jq's own
                    // exit-2 usage error, not a data error (#3098) -- checked
                    // before the `MalformedJsonError` branch below, which
                    // drains the `anyhow` channel by consuming `e`.
                    if let Some(unopenable) = e.downcast_ref::<InputFileOpenError>() {
                        return Ok(report_unopenable_input(unopenable));
                    }
                    let err = e.downcast::<MalformedJsonError>()?.to_eval_error();
                    let files = get_input_files(&args);
                    let file = files.first().map(|p| p.to_string_lossy().to_string());
                    sink.report(DiagStyle::Jq, &err, &InputLocation::at(file.as_deref(), 0));
                    return Ok(DiagStyle::Jq.error_exit_code());
                }
                Err(exit_code) => return Ok(exit_code), // Validation error
            };
        // `input_filename`'s names for this route's source tags (#3046). Stdin
        // is one unnamed source, which this table records as no sources at
        // all.
        jq::cli_context::set_input_names(if locations.files().is_empty() {
            vec![None]
        } else {
            locations.files().to_vec()
        });

        // Read before the queue takes `trailing_error` over (#3201). The
        // queue holds at most this one trailing error, so whatever
        // `InputPop::ParseError` the driver pops *is* it.
        let trailing_is_seq_warning = trailing_error.as_ref().is_some_and(|t| t.seq_warning);
        if uses_input_builtins {
            // Seed `input`/`inputs`/`input_line_number`'s shared queue
            // (#723) with every document `get_inputs` just read -- under
            // `-n`, `force_read_under_null_input` (passed above) made it
            // read the real documents instead of faking `[null]`, precisely
            // so they're available here.
            //
            // `!force_read_under_null_input` (TTY-safety suppressed the real
            // read) is the one case where `inputs` isn't real data -- it's
            // `get_inputs`'s own `[null]` placeholder for plain `-n`
            // (review catch: seeding straight from `inputs` unconditionally
            // fed that placeholder into the queue as if it were a genuine
            // document, so `input` silently returned `null` instead of
            // reporting exhausted). Seed nothing in that case instead --
            // `input`/`inputs` then correctly see an empty queue, matching
            // this function's own stated safety goal.
            if args.null_input && !force_read_under_null_input {
                jq::seed_remaining_inputs(Vec::new(), locations.exhausted(args.slurp));
            } else {
                // Moves rather than clones: `inputs` isn't read again on
                // this branch (the null-input arm below uses `OwnedValue::
                // Null` directly; the non-null arm pops from the queue this
                // seeds, not from `inputs`) -- review catch: an earlier
                // version cloned every document here for no reason, doubling
                // peak memory for the whole input set.
                //
                // `locations` itself is *not* consumed: it stays alive as the
                // file table `ErrorAt::Live` resolves the queue's opaque
                // source tags against (#1309).
                debug_assert_eq!(
                    inputs.len(),
                    locations.per_value().len(),
                    "one location per queued document"
                );
                let queue: Vec<(OwnedValue, u32, u32)> = inputs
                    .into_iter()
                    .zip(locations.per_value().iter().copied())
                    .map(|(v, (src, line))| (v, src, line))
                    .collect();
                // A stream that turned malformed partway queues its parse
                // error behind the documents before it (#2961).
                jq::seed_remaining_inputs_with_error(
                    queue,
                    locations.exhausted(args.slurp),
                    // As a plain error: jq's `try input catch .` catches a
                    // parse error like any other, where the materializer's
                    // own error is tagged as an uncatchable decode failure.
                    trailing_error.map(|t| (EvalError::new(t.error.message), (t.source, t.line))),
                );
            }

            if args.null_input {
                // `.` is null exactly once, matching `-n`'s own existing
                // contract -- `input`/`inputs` inside the filter draw from
                // the queue just seeded above, not from this invocation.
                //
                // `Live`, not `unknown()`: `-n` starts with nothing read, so
                // the marker *is* `<unknown>` until the filter's own `input`
                // moves it -- and once it does, jq names where the parser
                // ended up. `printf '' | jq -n 'input'` reports `<stdin>:0`,
                // not `<unknown>` (#1309, item 5).
                // Streaming, not collect-then-write (#1653) -- see
                // `evaluate_input_streaming`.
                evaluate_input_streaming(
                    &OwnedValue::Null,
                    &expr,
                    &context,
                    &ErrorAt::Live(&locations),
                    &mut sink,
                    &output_config,
                    &mut |sink, result| {
                        had_output = true;
                        last_output = Some(result.clone());
                        let stop = route_write_error(
                            sink,
                            &mut out,
                            || ErrorAt::Live(&locations).resolve(),
                            |o| write_output_owned_value(o, &result, &output_config),
                        )?;
                        Ok(!stop)
                    },
                )?;
                if let Some(code) = sink.halted() {
                    out.flush()?;
                    return Ok(code);
                }
            } else {
                // The outer loop and `input`/`inputs` draw from the exact
                // same queue (#723): a document a filter's own `input` call
                // consumes mid-evaluation is never also re-processed here as
                // a fresh top-level invocation, and vice versa -- one shared
                // cursor, not two kept in sync by hand.
                //
                // `ErrorAt::Live`, not a location captured here: the filter's
                // own `input`/`inputs` calls move jq's input position during
                // the very evaluation this loop kicks off, and jq's marker
                // names where the parser ended up, not where this document
                // started (#1309, item 4).
                //
                // A parse error the stream ended in reaches this loop only if
                // the filter's own `input` calls did not read it first: the
                // loop reports it uncaught, at exit 5, and stops -- the prefix
                // before it already ran, and nothing after it exists (#2961).
                loop {
                    let input = match jq::pop_input() {
                        jq::InputPop::Document(input) => input,
                        // jq's main loop prints a `--seq` parse error as
                        // ignored, and reads on to the end (#3201).
                        jq::InputPop::ParseError(error) if trailing_is_seq_warning => {
                            eprintln!(
                                "{}",
                                crate::jq_seq_reader::ignored_parse_error(&error.message)
                            );
                            break;
                        }
                        jq::InputPop::ParseError(error) => {
                            sink.report(
                                DiagStyle::Jq,
                                &error,
                                &ErrorAt::Live(&locations).resolve(),
                            );
                            break;
                        }
                        jq::InputPop::Exhausted => break,
                    };
                    // Streaming, not collect-then-write (#1653) -- see
                    // `evaluate_input_streaming`.
                    evaluate_input_streaming(
                        &input,
                        &expr,
                        &context,
                        &ErrorAt::Live(&locations),
                        &mut sink,
                        &output_config,
                        &mut |sink, result| {
                            had_output = true;
                            last_output = Some(result.clone());
                            let stop = route_write_error(
                                sink,
                                &mut out,
                                || ErrorAt::Live(&locations).resolve(),
                                |o| write_output_owned_value(o, &result, &output_config),
                            )?;
                            Ok(!stop)
                        },
                    )?;
                    if let Some(code) = sink.halted() {
                        out.flush()?;
                        return Ok(code);
                    }
                }
            }
        } else {
            for (idx, input) in inputs.iter().enumerate() {
                // `UNKNOWN_LINE`, not just any missing entry, must clear the
                // source too (#3202): `input_filename` can't otherwise tell
                // "this value's file is genuinely stdin" apart from "no
                // position at all" -- both read back as `INPUT_NAMES[0] ==
                // None` once `InputLocations::single` has folded a lost
                // slurp position (`--seq -s`) down to source index `0`
                // unconditionally (#1542's own invariant, kept for
                // `input`/`inputs`). Mirrors `remaining_inputs::last_line`'s
                // own `UNKNOWN_LINE` check for `input_line_number` (#1549).
                // `source_at` shares `resolve`'s own gate rather than
                // re-deriving it here.
                jq::cli_context::set_current_source(locations.source_at(idx));
                // Nothing on this branch can consume an input document, so
                // the per-value location is fixed before evaluation.
                let at = ErrorAt::Fixed(locations.get(idx));
                // Streaming, not collect-then-write (#1653): each output has
                // to reach stdout before the *next* one is evaluated, or a
                // mid-stream `debug`/`stderr`/`error` side effect lands ahead
                // of output that logically preceded it.
                evaluate_input_streaming(
                    input,
                    &expr,
                    &context,
                    &at,
                    &mut sink,
                    &output_config,
                    &mut |sink, result| {
                        had_output = true;
                        last_output = Some(result.clone());
                        let stop = route_write_error(
                            sink,
                            &mut out,
                            || at.resolve(),
                            |o| write_output_owned_value(o, &result, &output_config),
                        )?;
                        Ok(!stop)
                    },
                )?;
                if let Some(code) = sink.halted() {
                    out.flush()?;
                    return Ok(code);
                }
            }
            // The parse error the stream ended in, reported after the
            // documents before it (#2961) -- or, for `--seq -s`, the warning
            // jq defers past the filter (#3201).
            if let Some(t) = trailing_error {
                if t.seq_warning {
                    eprintln!(
                        "{}",
                        crate::jq_seq_reader::ignored_parse_error(&t.error.message)
                    );
                } else {
                    sink.report(
                        DiagStyle::Jq,
                        &t.error,
                        &locations.resolve(t.source, t.line),
                    );
                }
            }
        }
    }

    out.flush()?;

    // Determine exit code. An uncaught error outranks -e: jq's 5 says the
    // filter failed, -e's 1/4 describe an otherwise-successful result that
    // happened to be falsy (#355 vs #178).
    if sink.hit() {
        return Ok(DiagStyle::Jq.error_exit_code());
    }
    if args.exit_status {
        if !had_output {
            return Ok(exit_codes::NO_OUTPUT);
        }
        if let Some(last) = last_output {
            if matches!(last, OwnedValue::Null | OwnedValue::Bool(false)) {
                return Ok(exit_codes::FALSE_OR_NULL);
            }
        }
    }

    Ok(exit_codes::SUCCESS)
}

/// Build the evaluation context from command-line arguments.
fn build_context(args: &JqCommand) -> Result<EvalContext, BuildContextError> {
    let mut context = EvalContext::default();

    // Process --arg name value pairs
    for chunk in args.arg.chunks(2) {
        if let [name, value] = chunk {
            context
                .named
                .insert(name.clone(), OwnedValue::String(value.clone()));
        }
    }

    // Process --argjson name value pairs. jq's own wording and usage-hint
    // trailer, confirmed live (#3051) -- unlike --slurpfile/--rawfile below,
    // there's no file read here to report a `strerror` detail for.
    for chunk in args.argjson.chunks(2) {
        if let [name, value] = chunk {
            // jq's own wording doesn't name the flag's variable (`x`) at all.
            let json_value = parse_json_value(value).map_err(|_| {
                BuildContextError::Usage(
                    "invalid JSON text passed to --argjson\n\
                     Use jq --help for help with command-line options,\n\
                     or see the jq manpage, or online docs  at https://jqlang.github.io/jq"
                        .to_string(),
                )
            })?;
            context.named.insert(name.clone(), json_value);
        }
    }

    // Process --slurpfile name file pairs. jq wraps both an unreadable file
    // and malformed JSON in it under the identical "Bad JSON in --slurpfile
    // ..." wording (#3051, confirmed live) -- succinctly's own detail text
    // after the colon is not jq's (see `docs/compliance/jq/limitations.md`),
    // but the flag, exit code (2) and single-line shape now match. `{e:#}`,
    // not `{e}`: `parse_json_stream`'s own `.context("Invalid JSON in
    // stream")` call replaces (not chains onto) the wrapped `serde_json`
    // error's own Display, so a bare `{e}` here would silently drop its
    // line/column detail entirely rather than merely wording it differently
    // from jq's own diagnostic -- `anyhow::Error`'s alternate Display joins
    // the whole context chain instead.
    for chunk in args.slurpfile.chunks(2) {
        if let [name, file] = chunk {
            let contents = read_arg_file("slurpfile", name, file)?;
            let values = parse_json_stream(&contents).map_err(|e| {
                BuildContextError::Usage(format!("Bad JSON in --slurpfile {name} {file}: {e:#}"))
            })?;
            context
                .named
                .insert(name.clone(), OwnedValue::array_from(values));
        }
    }

    // Process --rawfile name file pairs. Same "Bad JSON in --rawfile ..."
    // wrapper as --slurpfile above even though a raw file is never actually
    // parsed as JSON -- jq's own generic arg-file reader uses this wording
    // unconditionally (#3051, confirmed live).
    for chunk in args.rawfile.chunks(2) {
        if let [name, file] = chunk {
            let contents = read_arg_file("rawfile", name, file)?;
            context
                .named
                .insert(name.clone(), OwnedValue::String(contents));
        }
    }

    // Process --args: values become string positional args
    for arg in &args.args {
        context.positional.push(OwnedValue::String(arg.clone()));
    }

    // Process --jsonargs: values become JSON positional args. A bad value is
    // jq's own usage error (exit 2), same wording as --argjson above (#3096):
    // both flags' failures route through `parse_json_value`, so the two
    // share the identical `invalid JSON text passed to --<flag>` message and
    // usage-hint trailer -- confirmed live against jq 1.7.1.
    for arg in &args.jsonargs {
        // jq's own wording doesn't name the offending value or position.
        let json_value = parse_json_value(arg).map_err(|_| {
            BuildContextError::Usage(
                "invalid JSON text passed to --jsonargs\n\
                 Use jq --help for help with command-line options,\n\
                 or see the jq manpage, or online docs  at https://jqlang.github.io/jq"
                    .to_string(),
            )
        })?;
        context.positional.push(json_value);
    }

    Ok(context)
}

/// Build the $ARGS special variable containing named and positional args.
fn build_args_var(context: &EvalContext) -> OwnedValue {
    let mut args_obj = IndexMap::new();

    // Build named object from context.named
    let named_obj: IndexMap<String, OwnedValue> = context.named.clone();
    args_obj.insert("named".to_string(), OwnedValue::Object(named_obj.into()));

    // Build positional array from context.positional
    args_obj.insert(
        "positional".to_string(),
        OwnedValue::Array(context.positional.clone().into()),
    );

    OwnedValue::Object(args_obj.into())
}

/// Get the filter expression from arguments.
///
/// An unreadable `-f`/`--from-file` filter file is jq's *own* usage error
/// (exit 2, `jq: Could not open <path>: <detail>`), not a generic `anyhow`
/// exit-1 failure (#3098) -- and its wording deliberately lacks both the
/// `error:` prefix and the `file` noun the main input file's own
/// `jq: error: Could not open file ...` uses (confirmed live against jq
/// 1.7.1), so it flows through [`BuildContextError::Usage`] like
/// [`build_context`]'s flag errors rather than sharing the main-input
/// wording.
fn get_filter(args: &JqCommand) -> Result<String, BuildContextError> {
    if let Some(ref path) = args.from_file {
        // Filter comes from file.
        let contents = std::fs::read_to_string(path).map_err(|e| {
            BuildContextError::Usage(format!(
                "Could not open {}: {}",
                path.display(),
                strerror_only(&e)
            ))
        })?;
        Ok(contents.trim().to_string())
    } else if let Some(ref filter) = args.filter {
        Ok(filter.clone())
    } else {
        Ok(".".to_string()) // Default: identity filter
    }
}

/// Get input files from arguments.
fn get_input_files(args: &JqCommand) -> Vec<std::path::PathBuf> {
    // With --args or --jsonargs, files are not used (they would have been consumed)
    if !args.args.is_empty() || !args.jsonargs.is_empty() {
        return vec![];
    }

    // When -f is used, the 'filter' field becomes the first input file
    // because the filter comes from a file instead of command line
    let mut files: Vec<std::path::PathBuf> = Vec::new();

    if args.from_file.is_some() {
        // When -f is used, the first positional arg (if any) is an input file
        if let Some(ref first_file) = args.filter {
            files.push(std::path::PathBuf::from(first_file));
        }
    }

    // Add remaining files
    files.extend(args.files.iter().map(std::path::PathBuf::from));

    files
}

/// Whether to force a real read under `-n` for `input`/`inputs`/
/// `input_line_number` (#723), given whether the filter actually references
/// one of them (`jq::walk::uses_input_builtins`, exact since #1309).
/// Narrowed by one safety check, pulled out as a pure function so it's
/// unit-testable without a real terminal: never force the read when `-n` is
/// set, no files were given, and stdin is an interactive terminal -- that
/// would block on a TTY the user may not have meant to feed.
///
/// Four bools, each independently meaningful and named at every call site
/// (no adjacent pair is ever confusable) -- a two-variant-enum refactor
/// would add ceremony without adding clarity for this single-call-site,
/// private helper.
#[allow(clippy::fn_params_excessive_bools)]
fn should_force_read_under_null_input(
    uses_input_builtins: bool,
    null_input: bool,
    no_files_given: bool,
    stdin_is_terminal: bool,
) -> bool {
    uses_input_builtins && !(null_input && no_files_given && stdin_is_terminal)
}

/// A parse error that ended a JSON input stream after a clean prefix of
/// documents (#2961): the error, and the `(source, line)` jq's marker names
/// once its parser has stopped there.
struct TrailingParseError {
    error: EvalError,
    source: u32,
    line: u32,
    /// A `--seq -s` warning jq raises only on the `next_input` call after
    /// the slurped array (#3201): the driver prints it as an ignored parse
    /// error, not an uncaught one, and it never costs exit 5. `input`/
    /// `inputs` still receive it as an error.
    seq_warning: bool,
}

/// What [`get_inputs`] read: every document, their locations, and -- when a
/// JSON input stream turned malformed partway -- the parse error that ended
/// it, to be raised only after the documents before it.
type Inputs = (Vec<OwnedValue>, InputLocations, Option<TrailingParseError>);

/// Get input values based on arguments.
/// Returns Err(i32) for validation failures (exit code), Ok(Err) for other errors.
fn get_inputs(
    args: &JqCommand,
    force_read_under_null_input: bool,
) -> std::result::Result<Result<Inputs>, i32> {
    // Null input mode: use null as the single input -- unless the filter
    // itself uses `input`/`inputs`/`input_line_number` (#723), in which case
    // the caller passes `force_read_under_null_input: true` to fall through
    // to the real read below instead: `-n` makes the top-level `.` null, but
    // `input`/`inputs` must still see the real stdin/files (`jq -n 'reduce
    // inputs as $x (0;.+$x)'` is jq's own idiomatic streaming-aggregation
    // pattern, oracle-confirmed). The caller is responsible for still
    // presenting `.` as `null` to the filter itself in that case -- this
    // function only controls what gets *read*, not what the top-level
    // invocation's input value is.
    if args.null_input && !force_read_under_null_input {
        // jq prints `(at <unknown>)` under -n: there is no input to point at.
        return Ok(Ok((
            vec![OwnedValue::Null],
            InputLocations::unknown(),
            None,
        )));
    }

    // Get input files
    let files = get_input_files(args);

    // Collect raw input from files or stdin. Read as bytes and decode below
    // rather than through `read_to_string`, which refused the whole input on
    // a stray byte and reported it as a *read* failure when the read had in
    // fact succeeded (#1247).
    let mut raw_bytes: Vec<(Option<usize>, Vec<u8>)> = if files.is_empty() {
        match read_stdin_bytes() {
            Ok(b) => vec![(None, b)],
            Err(e) => return Ok(Err(e)),
        }
    } else {
        let mut inputs = Vec::new();
        for (idx, path) in files.iter().enumerate() {
            match read_file_bytes(path) {
                Ok(b) => inputs.push((Some(idx), b)),
                // A main input `read_file_bytes` couldn't open is jq's own
                // exit-2 usage error (#3098); the caller downcasts it back
                // out of the `anyhow` channel here. `e.into()` converts the
                // concrete [`InputFileOpenError`] into the `anyhow::Error`
                // this `Result<Inputs, _>` carries.
                Err(e) => return Ok(Err(anyhow::Error::from(e))),
            }
        }
        inputs
    };

    // JSON input mode is the only mode that runs the strict validator at all
    // -- `-R`, `--seq` and DSV never do, and must not start now.
    let json_input_mode = args.input_dsv.is_none() && !args.raw_input && !args.seq;

    // #1525: real jq warns on stderr when it drops a malformed --seq
    // record; succinctly silently ignored malformed records entirely
    // (RFC 7464's own recommended failure mode, #1243, still correct for
    // *output*) with no diagnostic. `!args.raw_input` and
    // `args.input_dsv.is_none()` matter: `-R` takes over raw-text
    // handling entirely (matching `slurp_eof_line` below's identical
    // priority), and DSV content read via `-s`/`-n 'inputs'` (the only
    // way DSV input reaches this function -- non-slurp DSV has its own
    // streaming path that bypasses `get_inputs` entirely) never contains
    // RFC 7464 records at all, so neither combination should ever trigger
    // this. `!args.null_input` matters too: under `-n` combined with a
    // filter that forces a real read (`force_read_under_null_input`, the
    // only way this function is even reached with `args.null_input` true),
    // real jq treats the condition `seq_no_rs_byte_warning` checks for as
    // a *fatal* error (exit 5), not a warning -- printing the softer
    // "ignoring parse error" wording there would misrepresent succinctly's
    // separate, still-unfixed non-fatal behavior as though it now matched
    // jq; left silent instead until that's fixed properly.
    //
    // The two arms are exactly disjoint: `seq_no_rs_byte_warning` answers
    // `Some` only for a stream with no RS byte anywhere, which is the one
    // case `jq_seq_reader` is not asked about (#1723). Raw bytes, before
    // the UTF-8 substitution below, because jq counts columns in bytes.
    // That same reader walk also answers `--seq -s`'s EOF-location
    // question, so it runs once here and is used twice -- a second pass
    // over the whole stream measured +15% on a 12 MB `--seq -s -c length`
    // (interleaved A/B, release, output-identity gated).
    let mut seq_slurp_position_lost = false;
    // `jq_seq_reader::SeqStreamWalk::deferred_warning` (#3201).
    let mut seq_deferred_warning: Option<String> = None;
    // The raw walk's value ranges (`jq_seq_reader::SeqStreamWalk::values`,
    // #3247): the same pass that decides jq's warnings decides its values,
    // over the same raw bytes, already cut at #2998's end-of-stream drop.
    // Empty when no walk runs, which is only #1525's no-RS-byte stream,
    // where nothing ever leaves `WAITING_FOR_RS`.
    let mut seq_values: Vec<(usize, usize)> = Vec::new();
    // And each value's `(source index, line)` from the same walk (#3250).
    let mut seq_value_locations: Vec<(usize, usize)> = Vec::new();
    // Shared by `seq_warnings_apply` below, the `-n`-forced-read fallback
    // just after it, and the `build_seq_values` gate further down -- one
    // definition rather than three hand-copies of the same shape, so a
    // future change to it can't desynchronize them (CLAUDE.md: "Duplicated
    // predicates diverge silently").
    let seq_stream_shape = args.seq && !args.raw_input && args.input_dsv.is_none();
    let seq_warnings_apply = seq_stream_shape && !args.null_input;
    if seq_warnings_apply {
        // A *malformed* BOM is the one case where "no RS byte anywhere"
        // does not imply #1525's template: it costs jq a `parser_reset`
        // that leaves the parser reading rather than waiting for an RS, so
        // the reader owns that case too (#1723).
        let bom_malformed = crate::jq_seq_reader::bom_prefix(&raw_bytes).malformed;
        seq_slurp_position_lost =
            match seq_no_rs_byte_warning(&raw_bytes).filter(|_| !bom_malformed) {
                Some(warning) => {
                    eprintln!("{warning}");
                    // #1525's arm prints its own template instead of the
                    // reader's, but the location still needs the walk. A
                    // stream with no RS byte anywhere yields nothing, so
                    // this is the degenerate input, never the hot path --
                    // and `seq_values` stays empty, correctly: nothing ever
                    // leaves `WAITING_FOR_RS`, so no value can exist. Only
                    // read under `-s` below, so the walk is skipped
                    // otherwise.
                    args.slurp && crate::jq_seq_reader::slurp_eof_position_lost(&raw_bytes)
                }
                None => {
                    let walk =
                        crate::jq_seq_reader::walk_stream(&raw_bytes, args.slurp, &mut |warning| {
                            eprintln!("{warning}");
                        });
                    seq_values = walk.values;
                    seq_value_locations = walk.locations;
                    seq_deferred_warning = walk.deferred_warning;
                    walk.slurp_position_lost
                }
            };
    } else if seq_stream_shape {
        // The only way this function still runs `build_seq_values` with
        // `seq_warnings_apply` false is `-n` forcing a real read: DSV and
        // `-R` are excluded from both gates identically. That combination
        // skips the warnings but still needs the values and `-s`'s
        // location, so it runs the walk here with a no-op sink.
        let walk = crate::jq_seq_reader::walk_stream(&raw_bytes, args.slurp, &mut |_| {});
        seq_values = walk.values;
        seq_value_locations = walk.locations;
        // `-n -s`'s `input`/`inputs` read the deferred warning too.
        seq_deferred_warning = walk.deferred_warning;
        seq_slurp_position_lost = walk.slurp_position_lost;
    }

    // Answered here, before the UTF-8 substitution below consumes
    // `raw_bytes`: the rule reads jq's own byte stream, and a substituted
    // byte changes both the offsets and the reader's own BOM verdict (see
    // the function's docs). `-R` takes over entirely from `--seq` for
    // raw-text mode, keeping the same priority the location has below.
    let seq_trailing_record_dropped = args.slurp && args.seq && !args.raw_input && {
        // Every `--seq` stream shape already walked above, `-n` included;
        // `--input-dsv` skips that walk but can still reach the location,
        // so it pays for its own here. The rule itself lives with the
        // reader:
        // `jq_seq_reader::SeqStreamWalk::slurp_position_lost`.
        if seq_stream_shape {
            seq_slurp_position_lost
        } else {
            crate::jq_seq_reader::slurp_eof_position_lost(&raw_bytes)
        }
    };

    // `--seq` keeps its raw bytes: its values are ranges into them, and
    // each is substituted on its own in `build_seq_values` (#3247). The
    // whole-document substitution below would rewrite bytes the reader
    // already decided on. Nothing else below reads a `--seq` source.
    let seq_raw = if seq_stream_shape {
        std::mem::take(&mut raw_bytes)
    } else {
        Vec::new()
    };

    // All reads happen first, then decoding: a later file's read error still
    // outranks an earlier file's content error, as it did before.
    let mut raw_inputs: Vec<(Option<usize>, String)> = Vec::with_capacity(raw_bytes.len());
    for (file_idx, raw) in raw_bytes {
        // `String::from_utf8`, not `String::from_utf8_lossy(&raw).into_owned()`:
        // the latter allocates and copies the whole document even when it is
        // valid, because the `Cow` it returns is `Borrowed` and `into_owned`
        // must then clone it -- measured (pinned hardware, interleaved A/B)
        // as the dominant cost of #1247's whole diff, +9.47% median on a
        // cheap navigation query on x86_64. Taking ownership of the buffer
        // that already exists makes the valid path allocation-free.
        let raw = match String::from_utf8(raw) {
            Ok(s) => s,
            Err(e) => {
                // The substitution below is jq's own behaviour for a
                // non-UTF-8 document (see `utf8_lossy_document`), but it must
                // not run *before* `--validate` (#1247): repairing the
                // document first left the strict validator -- whose whole job
                // is to reject exactly this -- with nothing to find, so
                // `sjq --validate` exited 0 where `succinctly json validate`
                // on the same file still exits 1. `validate_json_input` fails
                // on any input that reaches here, so the substitution after
                // it is unreachable in JSON mode; it stays for the modes that
                // never validate.
                if args.validate && json_input_mode {
                    let filename = file_idx.map(|idx| files[idx].to_string_lossy().to_string());
                    validate_json_input(e.as_bytes(), filename.as_deref())?;
                }
                if args.raw_input && !args.slurp {
                    // #1742: real jq's non-slurp `-R` applies this
                    // end-of-buffer-relative fixup (#1717) per *line*, not
                    // once over the whole document -- splitting the raw
                    // bytes on `b'\n'` first, before substituting, is safe
                    // even amid invalid UTF-8: `\n` (0x0A) can never appear
                    // as a multi-byte sequence's own continuation byte, so
                    // this never risks splitting one mid-sequence. Joining
                    // back on `"\n"` reproduces the original byte layout
                    // exactly (including a trailing newline, which
                    // `split`'s own trailing-empty-segment already accounts
                    // for) -- `raw.lines()` further down re-splits this
                    // same way, so it sees corrected content at unchanged
                    // line boundaries. Slurp mode (`-s`) keeps whole-buffer
                    // substitution below, matching real jq there too.
                    e.into_bytes()
                        .split(|&b| b == b'\n')
                        .map(succinctly::text::utf8::substitute_invalid_utf8_jq_style)
                        .collect::<Vec<_>>()
                        .join("\n")
                } else if args.input_dsv.is_none() && !args.raw_input {
                    // #1743: JSON-shaped input (plain documents, `--slurp`
                    // and `--seq` alike) gets jq's own per-JSON-string
                    // substitution scope. Deliberately *not* gated on
                    // `json_input_mode`, which excludes `--seq` -- `--seq`
                    // input is still JSON text, just RS-separated, and real
                    // jq scopes the substitution to each string there too
                    // (oracle-verified). The two cases this branch must not
                    // capture are the ones left in the `else` below.
                    succinctly::jq::utf8_document::substitute_invalid_utf8_jq_document(e.as_bytes())
                } else {
                    // `--raw-input --slurp` (the non-slurp `-R` took the
                    // per-line branch above) is genuinely whole-buffer in
                    // real jq -- the entire input is one string, so the
                    // buffer's own end *is* that string's end
                    // (oracle-verified). DSV input is not JSON at all: its
                    // strings are `""`-escaped, not backslash-escaped, so a
                    // JSON string scanner would mis-segment it.
                    succinctly::text::utf8::substitute_invalid_utf8_jq_style(e.as_bytes())
                }
            }
        };
        raw_inputs.push((file_idx, raw));
    }

    let mut locations = InputLocations::new(
        files
            .iter()
            .map(|p| Some(p.to_string_lossy().to_string()))
            .collect(),
    );

    // `--slurp`'s single combined value has no content of its own to name a
    // line in -- jq instead names the *last source*'s own newline count at
    // EOF (#1520), computed here from the last source (`seq_raw`'s under
    // `--seq`, `raw_inputs`' otherwise) while it's still whole,
    // before either branch below consumes it, and only when slurping: every
    // ordinary invocation would otherwise pay this O(n) scan of the last
    // input for a value neither branch below ever reads. `line_at(bytes,
    // bytes.len())` is exactly this count: its trailing-lookahead byte is
    // always out of bounds at `end == bytes.len()`, so it degenerates to a
    // plain newline count -- the same one-off, single-lookup use its own
    // doc comment describes, not `LineCounter`'s repeated-increasing-offset
    // case. `None` when `--seq`'s trailing record was truncated/malformed
    // and silently dropped (#1542): real jq's own incremental parser loses
    // its EOF position entirely for a record it never finished reading,
    // where a malformed record earlier in the stream (one a later valid
    // record resyncs after) still reports normally. `-R` takes over
    // entirely from `--seq` for raw-text mode (matching the `-R -s` branch
    // below, which never RS-splits either) -- `!args.raw_input` here keeps
    // that same priority for the location, oracle-verified: `-R --seq -s`
    // on a truncated trailing record still reports its plain newline count,
    // not `<unknown>`.
    let last_source: Option<&[u8]> = if seq_stream_shape {
        seq_raw.last().map(|(_, raw)| raw.as_slice())
    } else {
        raw_inputs.last().map(|(_, raw)| raw.as_bytes())
    };
    let slurp_eof_line: Option<usize> = if args.slurp {
        last_source.and_then(|raw| {
            // #1550: the drop check reads across every file as one stream,
            // not just `raw_inputs.last()` alone -- a truncated record's
            // own opening RS byte and its closing/disambiguating bytes can
            // live in different files. Computed above, where the raw bytes
            // still exist.
            if seq_trailing_record_dropped {
                None
            } else {
                Some(line_at(raw, raw.len()))
            }
        })
    } else {
        None
    };

    // jq -R -s: the entire input (all files concatenated) becomes a single
    // string; no line splitting and no array wrap.
    if args.raw_input && args.slurp && args.input_dsv.is_none() {
        let mut combined = String::new();
        for (_, raw) in &raw_inputs {
            combined.push_str(raw);
        }
        // `slurp_eof_line`'s only `None` case is gated on `!args.raw_input`
        // above, which this branch's own `args.raw_input` guard rules out
        // here -- so `slurp_eof` always returns `Some` in this branch, but
        // the `match` still handles `None` explicitly (rather than
        // `.unwrap()`) to stay correct if that gate's condition ever
        // changes, matching this codebase's convention for exhaustive-but-
        // currently-dead defensive arms (#1064).
        let at = match locations.slurp_eof(slurp_eof_line) {
            Some((src, line)) => locations.resolve(src, line),
            None => InputLocation::unknown(),
        };
        return Ok(Ok((
            vec![OwnedValue::String(combined)],
            InputLocations::single(at),
            None,
        )));
    }

    // Process based on input mode
    let mut values = Vec::new();
    // A `--seq -s` warning jq defers past the filter rides the same slot
    // (#3201): delivered after the slurped array, to the driver or to the
    // filter's own `input`. jq has lost its position by then -- the call
    // that raises it closed the stream -- so it names `<unknown>`.
    let mut trailing_error: Option<TrailingParseError> =
        seq_deferred_warning.map(|message| TrailingParseError {
            error: EvalError::new(message),
            source: NO_SOURCE,
            line: UNKNOWN_LINE,
            seq_warning: true,
        });

    // `--seq` (RFC 7464, #1571) and raw-input (`-R`, #1809): both can
    // genuinely join content across a file boundary -- real jq's own
    // reader treats every file as one continuous byte stream for parsing
    // purposes (unrelated to whether `-s` is also passed: `-s` only
    // changes what happens to the *values* afterward, not how records/lines
    // are delimited) -- so both handle the whole file list at once via
    // [`build_seq_values`]/[`build_raw_input_values`] rather than per file
    // inside the loop below. Plain JSON keeps the per-file loop unchanged:
    // `find_json_values`/`parse_json_stream` never had a record delimiter
    // to lose in the first place (multiple JSON files are just independent
    // value streams concatenated in the output, with no
    // boundary-spanning-value concept to get wrong). DSV rows are
    // line-oriented and unverified either way (no jq DSV oracle to check
    // against), so they also keep the per-file loop -- including when
    // combined with `-R` (`args.raw_input && args.input_dsv.is_some()`),
    // which the DSV branch inside the loop already takes over first.
    if seq_stream_shape {
        values = build_seq_values(
            &seq_raw,
            &seq_values,
            &seq_value_locations,
            &mut locations,
            args.slurp,
        );
        if !args.slurp {
            debug_assert_eq!(locations.len(), values.len(), "one location per value");
        }
    } else if args.raw_input && args.input_dsv.is_none() {
        // Never reached with `args.slurp`: `-R -s` without `--input-dsv`
        // already returned early above this point.
        values = build_raw_input_values(&raw_inputs, &mut locations);
        debug_assert_eq!(locations.len(), values.len(), "one location per value");
    } else {
        for (file_idx, raw) in raw_inputs {
            let src = file_idx.unwrap_or(0);

            if let Some(delimiter) = args.input_dsv {
                // DSV input: each row becomes a JSON array of strings
                let parsed = parse_dsv_input(&raw, delimiter);
                // One row per line (approximate for embedded newlines; jq has no
                // DSV input mode, so there is no oracle to match here). Skipped
                // under `--slurp` (#1541): the combined array's own location
                // comes from `slurp_eof_line` above, not from any of these
                // per-value entries, which `get_inputs` discards wholesale when
                // it replaces `locations` with `InputLocations::single` below.
                if !args.slurp {
                    for line in 1..=parsed.len() {
                        locations.push(src, line);
                    }
                }
                values.extend(parsed);
            } else {
                // JSON input: validate first if --validate is set
                if args.validate {
                    let filename = file_idx.map(|idx| files[idx].to_string_lossy().to_string());
                    validate_json_input(raw.as_bytes(), filename.as_deref())?;
                }
                // Parse as JSON stream: the clean prefix, and the parse error
                // that ended it if the stream turned malformed (#2961).
                let (parsed, parse_error) = parse_json_stream_prefix(&raw);
                // `--slurp` stays all-or-nothing: jq prints nothing for a
                // slurped stream it could not finish parsing. The error goes
                // out in jq's channel at exit 5, like every other malformed
                // document, rather than through `anyhow` at exit 1.
                if args.slurp {
                    if let Some(error) = parse_error {
                        return Ok(Err(anyhow::Error::from(MalformedJsonError::new(error))));
                    }
                }
                // Skipped under `--slurp` (#1541) -- see the DSV branch above.
                // The splitter exists here only to feed
                // `locations.extend_from_ends`, so slurp mode skips that scan
                // entirely rather than running it and discarding the result.
                let mut error_line = 0;
                if !args.slurp {
                    // The splitter recognizes every span the parse above
                    // produced a value for (it is the more lenient of the two,
                    // and the prefix parse materializes from its spans), so
                    // it cannot come up short here; unreachable through this
                    // crate's own public CLI surface, and surfaced as an
                    // internal error rather than silently reusing a stale or
                    // wrong offset list if that ever stops holding (#1064).
                    let (spans, split_error) = split_json_values(raw.as_bytes());
                    if spans.len() < parsed.len() {
                        return Ok(Err(anyhow::anyhow!(
                            "internal error: the JSON splitter found {} values where the \
                             stream parse found {}",
                            spans.len(),
                            parsed.len()
                        )));
                    }
                    let ends: Vec<usize> = spans
                        .iter()
                        .take(parsed.len())
                        .map(|&(_, end)| end)
                        .collect();
                    locations.extend_from_ends(src, &raw, &ends, parsed.len());
                    if parse_error.is_some() {
                        // The malformed value starts at the first span the
                        // parse did not produce a value for, or where the
                        // splitter itself gave up.
                        let start = spans
                            .get(parsed.len())
                            .map(|&(start, _)| start)
                            .or(split_error)
                            .unwrap_or(raw.len());
                        error_line = parse_error_line(raw.as_bytes(), start);
                    }
                }
                values.extend(parsed);
                // jq's parser stops at the first malformed value: nothing
                // after it is read, this file or any later one, and the error
                // is raised once the documents before it have been (#2961).
                if let Some(error) = parse_error {
                    trailing_error = Some(TrailingParseError {
                        error,
                        source: u32::try_from(src).unwrap_or(u32::MAX),
                        line: u32::try_from(error_line).unwrap_or(u32::MAX),
                        seq_warning: false,
                    });
                    break;
                }
            }

            // Scoped to `!args.slurp` (#1541): under slurp, `locations` is left
            // empty by design (the branches above skip their pushes), so this
            // invariant no longer holds there and isn't meaningful to check --
            // the real slurp-mode invariant is `InputLocations::single`'s own
            // unconditional single push, asserted independently by `run_jq`'s
            // `debug_assert_eq!(inputs.len(), locations.per_value().len())`.
            if !args.slurp {
                debug_assert_eq!(locations.len(), values.len(), "one location per value");
            }
        }
    }

    // Slurp mode: wrap all inputs in an array
    if args.slurp {
        let at = match locations.slurp_eof(slurp_eof_line) {
            Some((src, line)) => locations.resolve(src, line),
            None => InputLocation::unknown(),
        };
        Ok(Ok((
            vec![OwnedValue::array_from(values)],
            InputLocations::single(at),
            // A plain-JSON parse error fails `--slurp` outright instead
            // (above); only `--seq`'s deferred warning follows the array
            // (#3201).
            {
                debug_assert!(trailing_error.as_ref().map_or(true, |t| t.seq_warning));
                trailing_error
            },
        )))
    } else {
        Ok(Ok((values, locations, trailing_error)))
    }
}

/// `input_filename`'s name for each input source tag (#3046): one per file,
/// or a single unnamed source (stdin) when no file was given.
fn input_names(files: &[std::path::PathBuf]) -> Vec<Option<String>> {
    if files.is_empty() {
        vec![None]
    } else {
        files
            .iter()
            .map(|p| Some(p.to_string_lossy().to_string()))
            .collect()
    }
}

/// What `get_search_list`/`get_jq_origin`/`get_prog_origin` report for this
/// run (#3046), each as jq 1.7.1 computes it:
///
/// - the search list is the `-L` directories, each resolved to its real path
///   when it exists and kept as given when it does not (`-L lib` is
///   `["/abs/cwd/lib"]`, `-L nope` is `["nope"]`), or jq's own default list,
///   which [`jq::cli_context`] supplies;
/// - the jq origin is the directory part of the path the binary was invoked
///   by, as typed and not resolved -- `.` for a bare name found on `PATH`, the
///   symlink's own directory for a symlink (jq 1.7.1: `(cd / && jq -n
///   get_jq_origin)` is `"."`);
/// - the program origin is the resolved directory of the `-f` file, or the
///   working directory.
fn program_context(args: &JqCommand) -> jq::cli_context::ProgramContext {
    let jq_origin =
        std::env::args_os()
            .next()
            .map(|argv0| match std::path::Path::new(&argv0).parent() {
                Some(dir) if !dir.as_os_str().is_empty() => dir.to_string_lossy().to_string(),
                _ => ".".to_string(),
            });
    let prog_origin = match &args.from_file {
        Some(path) => std::fs::canonicalize(path)
            .ok()
            .and_then(|p| p.parent().map(|dir| dir.to_string_lossy().to_string())),
        None => std::env::current_dir()
            .ok()
            .map(|dir| dir.to_string_lossy().to_string()),
    };
    jq::cli_context::ProgramContext {
        search_list: args
            .library_path
            .iter()
            .map(|p| {
                std::fs::canonicalize(p)
                    .unwrap_or_else(|_| p.clone())
                    .to_string_lossy()
                    .to_string()
            })
            .collect(),
        jq_origin,
        prog_origin,
    }
}

/// 1-based number of the last line carrying content.
///
/// Used only as `extend_from_ends`'s ends/values-mismatch fallback -- *not*
/// what `--slurp` reports at EOF (#1520 found that assumption wrong; see
/// `line_at`'s use in the `slurp_eof_line` computation above instead, which
/// is what jq's own marker actually counts there -- an empty input and one
/// with content but no trailing newline both report line `0`, where
/// `content_lines` would report `1` for the latter).
fn content_lines(raw: &str) -> usize {
    raw.lines().count().max(1)
}

/// Sentinel `per_value` line meaning "no real position" (#1542 -- a
/// `--seq` trailing record real jq itself never resolves). Never a real
/// line number: every genuine line comes from an actual newline count or
/// 1-based line index, bounded by the input's own byte length, and no real
/// input has close to `u32::MAX` lines. [`InputLocations::resolve`] checks
/// for it once, centrally, so every consumer of a `(source, line)` pair --
/// [`get`](InputLocations::get)'s direct lookup *and* the shared
/// `input`/`inputs` queue's `#1309` `ErrorAt::Live` path, which reads
/// straight from [`per_value`](InputLocations::per_value) -- answers
/// `<unknown>` the same way, rather than only the direct-lookup path
/// checking a side flag `resolve` itself didn't know about.
///
/// Value comes from the library crate's own `jq::UNKNOWN_INPUT_LINE` (#1549)
/// rather than an independently-picked `u32::MAX` here: this same raw value
/// also crosses into the shared `input`/`inputs`/`input_line_number` queue
/// (`seed_remaining_inputs`, below), so `builtin_input_line_number` needs to
/// recognize the exact sentinel this side emits -- previously it didn't,
/// and reported the raw `u32::MAX` (`4294967295`) instead of real jq's own
/// `0` for a dropped trailing `--seq -s` record.
const UNKNOWN_LINE: u32 = jq::UNKNOWN_INPUT_LINE;

/// A source tag no input has: once jq has closed its stream it names no file,
/// so `input_filename` answers `null` after reading a location tagged with it
/// (#3201's deferred `--seq -s` warning). Paired with [`UNKNOWN_LINE`], which
/// renders the `(at <unknown>)` marker before the tag is ever looked up.
const NO_SOURCE: u32 = u32::MAX;

/// Source locations for the values returned by [`get_inputs`].
///
/// Kept apart from the values and stored as `(source, line)` pairs: an owned
/// [`InputLocation`] per value would outweigh the values themselves on
/// line-oriented modes like `-R`, where every value is one short string.
#[derive(Debug, Default)]
pub struct InputLocations {
    /// File name per source, `None` for stdin.
    files: Vec<Option<String>>,
    /// `(source index, 1-based line)` per value. Empty means there is no input
    /// to point at (`-n`), which jq renders as `<unknown>`. A line of
    /// [`UNKNOWN_LINE`] means the same thing for one specific value within
    /// an otherwise-populated table (#1542).
    per_value: Vec<(u32, u32)>,
}

impl InputLocations {
    fn new(files: Vec<Option<String>>) -> Self {
        Self {
            files,
            per_value: Vec::new(),
        }
    }

    /// Locations for an input with nothing to point at (`-n`).
    fn unknown() -> Self {
        Self::default()
    }

    /// Locations for a single value at an already-resolved location, or at
    /// no location at all (`at.line.is_none()`, #1542 -- stored as
    /// [`UNKNOWN_LINE`]).
    ///
    /// Always pushes exactly one `per_value` entry regardless of `at`:
    /// slurp mode always produces exactly one value, so `get_inputs`'s
    /// `values`/`locations` invariant ("one location per value") must hold
    /// here unconditionally -- the seeding `.zip()` in `run_jq` silently
    /// truncates to the shorter side on a mismatch instead of erroring, so
    /// a skipped push here previously lost the whole slurped document to
    /// `input`/`inputs` (debug-build panic on the `debug_assert_eq!`
    /// guarding that zip, release-build silent empty output instead of
    /// jq's own `[]`) -- confirmed live against jq 1.7.1: `printf '' | jq
    /// -c -s '., inputs'` prints `[]`.
    fn single(at: InputLocation) -> Self {
        let mut locations = Self::new(vec![at.file.clone()]);
        locations.push(0, at.line.unwrap_or(UNKNOWN_LINE as usize));
        locations
    }

    fn push(&mut self, src: usize, line: usize) {
        self.per_value.push((src as u32, line as u32));
    }

    fn len(&self) -> usize {
        self.per_value.len()
    }

    /// Record one location per value from the values' end offsets in `raw`.
    ///
    /// Falls back to the last content line for every value when the counts
    /// disagree — modes that skip unparsable records can produce fewer values
    /// than the scan found offsets, and a wrong line is worse than a vague one.
    ///
    /// `ends` is already non-decreasing (this crate's own caller,
    /// `find_json_values`, is a single left-to-right scan), so one shared
    /// `LineCounter` keeps this whole loop O(n) rather than the O(n^2) a
    /// per-value `line_at` rescan from byte 0 produced (#1213). `--seq`'s
    /// own per-value locations no longer go through this helper at all
    /// (#1808): a boundary-spanning record's own file can't be recovered
    /// from an isolated `raw`, so `build_seq_values`/`seq_values_with_indices`
    /// track offsets across the whole multi-file stream directly instead.
    fn extend_from_ends(&mut self, src: usize, raw: &str, ends: &[usize], values: usize) {
        if ends.len() == values {
            let mut line_counter = LineCounter::new(raw.as_bytes());
            for &end in ends {
                self.push(src, line_counter.advance_to(end));
            }
        } else {
            let line = content_lines(raw);
            for _ in 0..values {
                self.push(src, line);
            }
        }
    }

    /// Location of the value at `idx`.
    pub fn get(&self, idx: usize) -> InputLocation {
        match self.per_value.get(idx) {
            Some(&(src, line)) => self.resolve(src, line),
            None => InputLocation::unknown(),
        }
    }

    /// The `(source, line)` pairs, in input order, for seeding the shared
    /// input queue (#1309). The queue carries the tag rather than the file
    /// name; [`resolve`](Self::resolve) turns it back -- including a
    /// [`UNKNOWN_LINE`] tag, so a value with no real position (#1542) still
    /// answers `<unknown>` once it's popped back off the queue and resolved
    /// via `ErrorAt::Live`, the same as it would through [`get`](Self::get).
    /// File name per source tag, `None` for stdin.
    fn files(&self) -> &[Option<String>] {
        &self.files
    }

    fn per_value(&self) -> &[(u32, u32)] {
        &self.per_value
    }

    /// The value at `idx`'s own source index, gated the same way
    /// [`resolve`](Self::resolve) gates it for the `(at <file>:<line>)`
    /// marker: `None` when that row's line is [`UNKNOWN_LINE`] (#3202), not
    /// just when `idx` is out of range. Shares `resolve`'s own
    /// `line == UNKNOWN_LINE` check rather than re-deriving it, so
    /// `jq::cli_context::set_current_source`'s caller can't drift from what
    /// `get`/`resolve` already treat as "no real position" the way #1549
    /// found `input_line_number` had.
    fn source_at(&self, idx: usize) -> Option<u32> {
        let &(src, line) = self.per_value.get(idx)?;
        (line != UNKNOWN_LINE).then_some(src)
    }

    /// Turn a raw `(source, line)` -- as handed back by
    /// `jq::current_input_location` -- into a printable location.
    fn resolve(&self, src: u32, line: u32) -> InputLocation {
        if line == UNKNOWN_LINE {
            return InputLocation::unknown();
        }
        InputLocation::at(
            self.files.get(src as usize).and_then(Option::as_deref),
            line as usize,
        )
    }

    /// Index of the last source on the command line (0 for stdin, or when
    /// there is exactly one source), shared by [`exhausted`](Self::exhausted)
    /// and [`slurp_eof`](Self::slurp_eof) -- both encode the same "jq's
    /// parser ends up at the last source" rule, just at different moments.
    fn last_src(&self) -> u32 {
        self.files.len().saturating_sub(1) as u32
    }

    /// Where jq's `(at ...)` marker settles once every input is consumed
    /// (#1309, item 5).
    ///
    /// jq names the file its parser has open at EOF, which is the *last* file
    /// on the command line -- at line 0 when that file contributed no document
    /// of its own, and at its last document's line otherwise. Oracle-verified
    /// against jq 1.7.1:
    ///
    /// ```text
    /// jq -n 'input,input'       one.json empty.json  => empty.json:0
    /// jq -n 'input,input'       empty.json one.json  => one.json:1
    /// jq -n 'input,input,input' one.json two.json    => two.json:1
    /// printf '' | jq -n 'input'                      => <stdin>:0
    /// ```
    ///
    /// No files at all means stdin, whose tag is 0 and whose name is `None`,
    /// so the same arithmetic yields `<stdin>`.
    ///
    /// **`--slurp` is the exception, and it is total:** slurping consumes the
    /// entire input to build one value, so jq has no file position left to
    /// name and reports `<unknown>` regardless of how many files were given or
    /// what they held. `slurping` therefore short-circuits to `None`. Note
    /// this reaches only *exhaustion* — a non-input error under `-s` still
    /// reports the slurped value's own location, which is the ordinary
    /// `Fixed`/`resolve` path and needs nothing special here. Oracle-verified:
    ///
    /// ```text
    /// jq -s '., input'    a.json b.json  => <unknown>   (not b.json:1)
    /// jq -R -s '., input' < two-lines    => <unknown>
    /// jq -s 'error("x")'  a.json b.json  => b.json:1    (unchanged)
    /// ```
    fn exhausted(&self, slurping: bool) -> Option<(u32, u32)> {
        if slurping {
            return None;
        }
        let last_src = self.last_src();
        Some(match self.per_value.last() {
            Some(&(src, line)) if src == last_src => (last_src, line),
            _ => (last_src, 0),
        })
    }

    /// Where `--slurp`'s single combined value's `(at ...)` marker points
    /// (#1520), as a raw `(source, line)` tag -- the same `Option` shape
    /// [`exhausted`](Self::exhausted) returns, for the same reason (`None`
    /// means "no position to point at", i.e. `<unknown>`; the caller
    /// resolves `Some` to a printable name only once it's actually needed):
    /// the last source on the command line, at `eof_line` -- that source's
    /// own newline count (`line_at(bytes, bytes.len())` at the call site
    /// above), or `None` when the caller found `--seq`'s trailing record
    /// truncated/malformed (#1542).
    ///
    /// The same "last source" rule as `exhausted`, but slurp collapses every
    /// input into one value up front, so there is no per-value table
    /// afterward to fall back on the way `exhausted` does -- the caller must
    /// compute `eof_line` directly from the last source's raw text before
    /// it's consumed, and pass it in.
    fn slurp_eof(&self, eof_line: Option<usize>) -> Option<(u32, u32)> {
        let line = eof_line?;
        Some((self.last_src(), line as u32))
    }
}

/// Where an evaluation's `(at ...)` marker should point.
///
/// Two shapes because the input-builtin path cannot know the answer up front.
/// `input`/`inputs` move jq's current input position *during* the evaluation
/// they are called from, and jq reports where the parser ended up rather than
/// where the value in hand came from -- oracle-verified:
/// `jq '[inputs] | .[0] | error("boom")' a b c` names **c**, not `.[0]`'s own
/// **b** (#1309, item 4).
///
/// Reading the position after evaluation returns is sound because
/// `evaluate_input_streaming` reports the same *set* of diagnostics the
/// eager `evaluate_input` it replaced did (#1653) -- its `Error`, `Break`,
/// `Halt` and `Partial` arms are mutually exclusive variants of one returned
/// value, and an uncaught control ends the evaluation, so nothing can consume
/// another document between the raise and the report.
enum ErrorAt<'a> {
    /// A position fixed before evaluation -- every path that cannot consume
    /// input documents.
    Fixed(InputLocation),
    /// Resolved from the shared queue's live position at report time, against
    /// the file table that owns the source tags.
    Live(&'a InputLocations),
}

impl ErrorAt<'_> {
    fn resolve(&self) -> InputLocation {
        match self {
            Self::Fixed(at) => at.clone(),
            Self::Live(locations) => match jq::current_input_location() {
                Some((src, line)) => locations.resolve(src, line),
                // Nothing has been read yet, so there is no position to name.
                None => InputLocation::unknown(),
            },
        }
    }
}

/// Replace every invalid UTF-8 sequence in a *document* with U+FFFD, the
/// way real jq does (#1247).
///
/// jq is the odd one out here: `yq` rejects a non-UTF-8 document outright
/// (see `yq_runner::yaml_validate_guard`), but jq accepts it, substitutes
/// the replacement character and exits 0 -- `{"a":"\xff\xfe"}` prints as
/// `"\u{fffd}\u{fffd}"`. succinctly used to echo the raw bytes instead,
/// which means it wrote invalid UTF-8 to stdout; the non-lazy path was
/// worse still, refusing the file with `Failed to read file` when the read
/// had in fact succeeded.
///
/// Valid input is returned untouched and unallocated -- the check is a
/// whole-input SIMD pass (~1.1 ms on 8.4 MB) and only a document that
/// actually fails it pays for a copy, via
/// [`substitute_invalid_utf8_jq_document`](succinctly::jq::utf8_document::substitute_invalid_utf8_jq_document),
/// which scopes
/// [`substitute_invalid_utf8_jq_style`](succinctly::text::utf8::substitute_invalid_utf8_jq_style)'s
/// rule (#1617/#1717) to each JSON string the way real jq's own lexer does
/// (#1743) rather than to the whole file. Both of those carry the
/// substitution rule and the scoping rationale respectively.
///
/// Document input only, and only when `--validate` is off: the strict
/// validator has to see the original bytes, or the substitution repairs the
/// document out from under the one check that is supposed to reject it
/// (#1247). `--raw-input` gets the same substitution (jq substitutes there
/// too) via `get_inputs`' own decode; DSV input, `--arg`/`--argjson` and
/// `--rawfile` get none.
fn utf8_lossy_document(raw: Vec<u8>) -> Vec<u8> {
    match succinctly::text::utf8::validate_utf8(&raw) {
        Ok(()) => raw,
        Err(_) => {
            succinctly::jq::utf8_document::substitute_invalid_utf8_jq_document(&raw).into_bytes()
        }
    }
}

/// Read stdin to bytes.
fn read_stdin_bytes() -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    std::io::stdin()
        .read_to_end(&mut buf)
        .context("Failed to read from stdin")?;
    Ok(buf)
}

/// Read a file to bytes.
///
/// A file `std::fs::read` cannot open is an [`InputFileOpenError`], not a
/// generic `anyhow` failure: jq reports a main input it cannot open with its
/// own one-line diagnostic at exit 2, so the call sites convert it back
/// (via [`report_unopenable_input`], or `get_inputs`' caller's downcast)
/// rather than letting it escape as `Error: .../Caused by:` at exit 1
/// (#3098).
fn read_file_bytes(path: &Path) -> std::result::Result<Vec<u8>, InputFileOpenError> {
    std::fs::read(path).map_err(|e| InputFileOpenError::new(path, strerror_only(&e)))
}

/// A main input file [`read_file_bytes`] couldn't open (#3098).
///
/// jq reports an unreadable main input with `jq: error: Could not open file
/// <path>: <detail>` at exit 2 (`USAGE_ERROR`) -- not the generic exit-1
/// `Error: .../Caused by:` block -- confirmed live against jq 1.7.1 for
/// both a plain `jq '.' file` invocation and `jq 'inputs' file`. Modeled on
/// [`MalformedJsonError`]: a concrete `std::error::Error` the `anyhow`
/// channels in `run_jq`/`get_inputs` carry so the call sites can `downcast`
/// it back into jq's own diagnostic shape. The `error:` prefix (present
/// here, absent from `-f`'s own "Could not open" wording -- see
/// [`get_filter`]) lives in `Display`, so the same string paints the whole
/// [`report_usage_error`] line.
#[derive(Debug)]
pub struct InputFileOpenError {
    path: PathBuf,
    detail: String,
}

impl InputFileOpenError {
    fn new(path: &Path, detail: String) -> Self {
        Self {
            path: path.to_path_buf(),
            detail,
        }
    }
}

impl std::fmt::Display for InputFileOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "error: Could not open file {}: {}",
            self.path.display(),
            self.detail
        )
    }
}

impl std::error::Error for InputFileOpenError {}

/// Report an [`InputFileOpenError`] the way jq does and answer with the exit
/// code jq takes for it: `jq: error: Could not open file <path>: <detail>`,
/// exit 2 (`USAGE_ERROR`) (#3098).
fn report_unopenable_input(e: &InputFileOpenError) -> i32 {
    report_usage_error(&e.to_string());
    exit_codes::USAGE_ERROR
}

/// Incremental version of [`line_at`] for a caller visiting a monotonically
/// increasing sequence of `end` offsets into the same `bytes` (#1213) --
/// every per-value location in a multi-value document or `--seq`/`--slurp`
/// stream, not just a single error report. `line_at` itself scans from byte
/// 0 on every call; calling it once per value in an N-value input makes the
/// whole loop O(N^2) (confirmed: 80k JSON-lines records took ~27s wall time
/// against a real-jq baseline under half a second). This type instead scans
/// each byte of `bytes` at most once across the whole sequence of calls, by
/// remembering how far the previous call already counted.
///
/// `advance_to` must be called with non-decreasing `end` values -- the same
/// order [`find_json_values`] and the `--seq` reader already produce
/// their offsets in, since both are themselves single left-to-right scans.
struct LineCounter<'a> {
    bytes: &'a [u8],
    pos: usize,
    newlines_before_pos: usize,
}

impl<'a> LineCounter<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            pos: 0,
            newlines_before_pos: 0,
        }
    }

    /// Same result [`line_at`] would return for this `end`, given every
    /// prior call passed a `end` no greater than this one.
    fn advance_to(&mut self, end: usize) -> usize {
        let end = end.min(self.bytes.len());
        debug_assert!(
            end >= self.pos,
            "LineCounter::advance_to called with a smaller end than a previous call"
        );
        self.newlines_before_pos += self.bytes[self.pos..end]
            .iter()
            .filter(|&&b| b == b'\n')
            .count();
        self.pos = end;
        let mut count = self.newlines_before_pos;
        if self.bytes.get(end) == Some(&b'\n') {
            count += 1;
        }
        count
    }
}

/// jq's line number for the value whose exclusive end offset is `end` within
/// `bytes`.
///
/// jq's `(at <file>:<line>)` marker names the line on which the input value
/// *ends*, so callers pass the exclusive end offset from [`find_json_values`].
/// jq's counter is the number of `\n` bytes its lexer has consumed by the
/// time the value's boundary is confirmed: every newline strictly before
/// `end`, plus exactly one byte of trailing lookahead if it exists and is a
/// newline. It is zero-based, not one-based — a value ending before any `\n`
/// reports line 0. Only for a single, one-off lookup (a parse-error report,
/// reached at most once per input) -- a caller visiting many `end` values
/// for the same `bytes` in increasing order (one location per value in a
/// multi-value document) must use [`LineCounter`] instead, or an O(n) scan
/// per call becomes an O(n^2) loop (#1213).
fn line_at(bytes: &[u8], end: usize) -> usize {
    let end = end.min(bytes.len());
    let mut count = bytes[..end].iter().filter(|&&b| b == b'\n').count();
    if bytes.get(end) == Some(&b'\n') {
        count += 1;
    }
    count
}

/// jq's `(at <file>:<line>)` line once its parser has stopped on a malformed
/// value starting at `start` (#2961): every newline up to and including the
/// one ending that value's line, since jq's lexer reads to the next delimiter
/// before it raises. On a document with one value per line that is exact --
/// `1\n2 }\n\n\n` reports line 2, `1\n2\n}\n3\n` line 3. A malformed value
/// spanning several lines, or cut off at end of input, is where it is not:
/// jq names wherever its parser gave up inside it, or `<unknown>` at EOF, and
/// this names the end of its first line instead. Recorded in
/// `docs/compliance/jq/limitations.md`.
fn parse_error_line(bytes: &[u8], start: usize) -> usize {
    let start = start.min(bytes.len());
    let line_end = bytes[start..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |offset| start + offset);
    line_at(bytes, line_end)
}

/// Find the byte ranges of JSON values in a byte slice.
///
/// This is a simple heuristic that finds the boundaries of top-level JSON values
/// by tracking brace/bracket nesting and handling strings.
///
/// Returns `Err(offset)` -- the byte offset the unparseable value started at
/// -- for content it can't recognize as any JSON value shape (a truncated
/// container/string, or a byte that starts none of the recognized shapes),
/// rather than silently skipping it. An earlier version of this function
/// skipped to the next whitespace and kept going, so `{invalid}`/`[1,2,`
/// produced `{}`/no output at all instead of an error (#1171) -- real jq
/// itself stops at the first parse failure, not skip-and-continue.
fn find_json_values(bytes: &[u8]) -> core::result::Result<Vec<(usize, usize)>, usize> {
    match split_json_values(bytes) {
        (values, None) => Ok(values),
        (_, Some(offset)) => Err(offset),
    }
}

/// [`find_json_values`] without discarding what it found before a failure:
/// every value span up to the first unrecognizable one, and that one's start
/// offset if there was one.
///
/// jq's parser is incremental, so a malformed *later* value cannot retract
/// the ones before it: `printf '{"ok":1}\n[1,2,' | jq -c .` prints
/// `{"ok":1}` and then reports the parse error (#2961). The callers that
/// emit values use this to do the same; the scan itself is unchanged, so the
/// hot input path does exactly the work it did before.
fn split_json_values(bytes: &[u8]) -> (Vec<(usize, usize)>, Option<usize>) {
    let mut values = Vec::new();
    let mut pos = 0;

    while pos < bytes.len() {
        // Skip whitespace
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }

        let start = pos;

        match scan_one_json_token(bytes, pos) {
            Some(end) => {
                values.push((start, end));
                pos = end;
            }
            None => return (values, Some(start)),
        }
    }

    (values, None)
}

/// Where the JSON token starting at `pos` ends, or `None` if no token can
/// start there.
///
/// Extracted from [`find_json_values`] so `--seq`'s own lenient scanner
/// (#1723) can ask the identical per-token question without a second copy of
/// this dispatch drifting from it.
///
/// "Ends" is *mostly* structural rather than a validity verdict:
/// `{"a":1 xyz}` scans as one token because its braces match, and only
/// validation rejects it. Keeping the two separate is what lets the `--seq`
/// caller reject a whole record without ever reading a value out of the
/// middle of a malformed one.
///
/// Two arms are deliberate exceptions, both because the scan that finds the
/// token's end is already the scan that would validate it, so splitting them
/// would cost a second pass for nothing: `number_literal_end` rejects a
/// number-*shaped* span that is not a number (`-e5`, `1e`), and
/// `string_literal_end` rejects a string holding a raw `U+0000`-`U+001F`
/// (#2878). Both were verified not to move `--seq`, whose own validity
/// question (`seq_value_is_valid`) already rejected these inputs -- so the
/// two routes still agree, they just now agree earlier.
fn scan_one_json_token(bytes: &[u8], pos: usize) -> Option<usize> {
    match bytes[pos] {
        // Object or array - find matching close
        b'{' | b'[' => find_matching_close(bytes, pos),
        // String. `string_literal_end` (shared with `light.rs`, same
        // one-validated-implementation reasoning as `number_literal_end`
        // below) both finds the end of and validates the token: it rejects
        // a raw, unescaped control character `U+0000`-`U+001F` outright,
        // matching real jq on every input path, while still accepting
        // `0x7F` the way jq's own signed-char check does (#2878).
        b'"' => succinctly::json::light::string_literal_end(bytes, pos),
        // true, false, null. Only `t…`, `f…` and `nu…` take the keyword
        // path -- jq's own `check_literal` (`jv_parse.c`) reads exactly
        // that, and sends a bare `n` to its number parser, which is how
        // `nan` and `nan12` are numbers there (#2877; `jq_seq_reader.rs`
        // models the same split for `--seq`).
        b't' | b'f' => find_literal_end(bytes, pos),
        b'n' if bytes.get(pos + 1) == Some(&b'u') => find_literal_end(bytes, pos),
        // Number. `jq_number_token_end` (shared with `light.rs`'s own
        // materializer, #1171 review -- one validated implementation
        // instead of independently-maintained copies) both finds the
        // end of and validates the token: a `.` is accepted as a
        // leading byte when at least one digit follows (`.5` -> `0.5`,
        // matching real jq's own leniency beyond strict JSON), a leading
        // `+` is peeled like `-`, decNumber's special words (`nan`,
        // `sNaN12`, `-Infinity`, `inf`) are numbers (#2877), and a byte
        // sequence that only *looks* number-shaped (`-e5`, `1e`, a bare
        // `.`, `nanx`, `+-1`) is rejected outright rather than silently
        // accepted as a truncated or zero-length span.
        b'+' | b'-' | b'.' | b'0'..=b'9' | b'n' | b'N' | b'i' | b'I' | b's' | b'S' => {
            succinctly::json::light::jq_number_token_end(bytes, pos)
        }
        _ => None,
    }
}

/// Find the end of an object or array starting at `pos`.
fn find_matching_close(bytes: &[u8], pos: usize) -> Option<usize> {
    let open = bytes[pos];
    let close = if open == b'{' { b'}' } else { b']' };
    let mut depth = 1;
    let mut i = pos + 1;

    while i < bytes.len() && depth > 0 {
        match bytes[i] {
            b'"' => {
                // Skip string. Same scanner as the top-level arm, so a raw
                // control character is rejected just as readily nested
                // inside a container -- in a key as well as a value (#2878).
                let end = succinctly::json::light::string_literal_end(bytes, i)?;
                i = end;
                continue;
            }
            c if c == open => depth += 1,
            c if c == close => depth -= 1,
            _ => {}
        }
        i += 1;
    }

    if depth == 0 {
        Some(i)
    } else {
        None
    }
}

/// Find the end of a literal (true, false, null) starting at `pos`.
/// Where a `true`/`false`/`null` keyword token starting at `pos` ends, under
/// jq 1.7.1's own reader, for the **top-level document splitter** (#3035).
///
/// The answer is [`succinctly::json::light::keyword_span`]'s whole-token
/// rule: the *entire* run up to jq's literal boundary
/// (`jq_literal_run_end` -- whitespace and `"[{,:]}`) must be exactly the
/// keyword. Anything else -- a longer token (`null1`, `truex`, `nully`, a
/// punctuation-suffixed `null-`, `true!`) or a truncated one (`nul`,
/// `tru`, `fals`) -- returns `None`, so the splitter does not carve a valid
/// `null` out of the middle of a token jq reports `Invalid literal` for, the
/// way an alphabetic-only run used to split `null1` into `null` + `1`.
fn find_literal_end(bytes: &[u8], pos: usize) -> Option<usize> {
    let kw: &[u8] = match bytes.get(pos) {
        Some(b't') => b"true",
        Some(b'f') => b"false",
        Some(b'n') => b"null",
        _ => return None,
    };
    succinctly::json::light::keyword_span(bytes, pos, kw)
}

/// Parse a JSON value from a string (`--argjson`/`--jsonargs`), preserving
/// the original number-literal spelling the way document-sourced numbers
/// already do (#1058) -- unlike `serde_json::Value::Number`, which
/// round-trips only through Rust's own `f64`/`i64` `Display` and loses e.g.
/// trailing zeros (`1.500` -> `1.5`) or exponent notation (`1e100` -> the
/// fully-expanded digit string), the way a filter-literal number
/// (`Literal::NumberLiteral`, #1035) or a document-sourced one
/// (`OwnedValue::from_number_bytes`) does not.
///
/// #2052: the gate is [`validate::validate_jq_lenient`] -- this crate's own
/// RFC 8259 validator in jq's accept-set -- then
/// [`json_bytes_to_owned_value_checked`] over the same, now-known-valid
/// text. It used to be `serde_json::from_str::<serde_json::Value>` followed,
/// on failure, by a retry against a text-rewriting normalizer chain, one
/// normalizer per leniency jq allows and `serde_json` does not
/// (#1094/#2012/#2240). That shape cost a fourth normalizer per new
/// leniency and had already shipped one composition bug (#2012: two
/// normalizers applied independently against the untouched original
/// rejected `[007,"\udc00"]`, which needs both); stating the accept-set
/// once, in the validator, replaces the chain, the retry and the second
/// validation pass together.
///
/// **Dropping `serde_json` here is a fidelity gain, not a trade.** #1095
/// chose `serde_json::Value` over the cheaper `IgnoredAny` to reject a
/// magnitude-overflowing literal, on the reasoning that `1e400` would
/// otherwise materialize as `null`. Both halves have since stopped holding:
/// the materializer preserves the literal (`1E+400`, the same answer the
/// `--slurpfile` and primary-input paths already give), and jq 1.7.1 does
/// not reject it either -- `jq -nc --argjson x 1e400 '$x'` is `1E+400`, so
/// the rejection was a divergence only `--argjson`/`--jsonargs` still
/// carried.
fn parse_json_value(s: &str) -> Result<OwnedValue> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(OwnedValue::Null);
    }

    validate::validate_jq_lenient(s.as_bytes())
        .map_err(|e| anyhow::anyhow!("Invalid JSON: {s}: {e}"))?;
    // #2295: the checked materializer, not the bare panicking one -- `s` is
    // genuinely external text, and the depth guard has to be this call's
    // own rather than an incidental by-product of whatever validated it.
    json_bytes_to_owned_value_checked(s.as_bytes())
        .map_err(|e| anyhow::anyhow!("Invalid JSON: {s}: {e}"))
}

/// Parse a JSON stream (multiple JSON values) from a string. Backs both
/// `--slurpfile` and, more heavily, the crate's own default/primary JSON
/// document-input path (see the `evaluate_input_streaming` call site above, reached
/// on every ordinary `sjq`/`succinctly jq` invocation that isn't
/// `--seq`/`--raw-input`/`--input-dsv`).
///
/// Preserves number-literal source fidelity the same way `parse_json_value`
/// does for `--argjson` (#1058, extended here to `--slurpfile`, #1093):
/// `serde_json::Deserializer::byte_offset()` delimits each value's own
/// span within the stream, and [`json_bytes_to_owned_value_checked`]
/// materializes the real result from that span.
///
/// Doesn't *primarily* share span-finding with `find_json_values` (above,
/// #1163 follow-up question), despite both walking a byte string to
/// delimit consecutive JSON values: `find_json_values` is a permissive,
/// jq-compatible scanner (accepts a leading `.` as a number start, doesn't
/// reject trailing garbage) with no `serde_json` dependency at all, while
/// this function's own validation strictness is load-bearing for more than
/// just `--slurpfile`'s CLI-arg error message -- the main input path's
/// [`parse_json_stream_prefix`], which shares this function's accept-set
/// value by value, depends on rejecting everything the splitter would
/// reject, so `get_inputs`' span cross-check never diverges.
///
/// It *does* fall back to `find_json_values` on a `serde_json` failure,
/// though (#1243): real jq's own number parser tolerates a leading zero
/// (`007`) that strict JSON doesn't (#1094), and unlike `parse_json_value`,
/// which since #2052 states that leniency directly in
/// `validate::validate_jq_lenient`'s accept-set, this function also backs
/// plain `--slurp` on the crate's own *primary* document-input path, where
/// stripping leading zeros from a re-validated copy but still needing to
/// materialize spans from the *original* text hits the same
/// offset-doesn't-shrink-in-lockstep problem `find_json_values` was built
/// to solve for a single document already. `find_json_values` already
/// tolerates a leading zero on its own (`number_literal_end` has no
/// leading-zero rejection), so retrying through it instead, only on
/// failure, fixes the divergence without weakening the happy path's
/// validation: this function's accepted-input set only ever *grows* to
/// match `find_json_values`'s own (never shrinks below `serde_json`'s), so
/// the main input path's cross-check invariant above still holds -- it
/// still never diverges, just via equality now instead of strict subset.
fn parse_json_stream(s: &str) -> Result<Vec<OwnedValue>> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(vec![]);
    }

    match parse_json_stream_strict(s) {
        Ok(values) => Ok(values),
        Err(e) => {
            let bytes = s.as_bytes();
            match find_json_values(bytes) {
                Ok(spans) => spans
                    .into_iter()
                    .map(|(start, end)| json_bytes_to_owned_value_checked(&bytes[start..end]))
                    // Wrapped rather than flattened, for the same reason as
                    // the sibling site in `parse_json_stream_strict`: the
                    // caller reports a document error in jq's channel at
                    // exit 5, and can only recognise it by type (#1194).
                    .collect::<core::result::Result<Vec<_>, _>>()
                    .map_err(|de| anyhow::Error::from(MalformedJsonError::new(de))),
                Err(_) => Err(e),
            }
        }
    }
}

/// [`parse_json_stream`] for a document input stream: every value before the
/// first malformed one, and that one's error, rather than all or nothing
/// (#2961).
///
/// Accepts exactly what [`parse_json_stream`] accepts value by value: the
/// strict pass first, and on its failure the same splitter-plus-checked-
/// materializer fallback. The only difference is what a failure keeps.
/// A splitter failure reads `Invalid JSON text`, the wording the lazy path
/// already reports for the same failure.
fn parse_json_stream_prefix(s: &str) -> (Vec<OwnedValue>, Option<EvalError>) {
    let s = s.trim();
    if s.is_empty() {
        return (vec![], None);
    }
    if let Ok(values) = parse_json_stream_strict(s) {
        return (values, None);
    }
    let bytes = s.as_bytes();
    let (spans, split_error) = split_json_values(bytes);
    let mut values = Vec::with_capacity(spans.len());
    for (start, end) in spans {
        match json_bytes_to_owned_value_checked(&bytes[start..end]) {
            Ok(value) => values.push(value),
            Err(error) => return (values, Some(error)),
        }
    }
    let error = split_error.map(|_| EvalError::new("Invalid JSON text"));
    (values, error)
}

/// The `serde_json`-validated core of [`parse_json_stream`], split out so
/// its own doc comment can describe the fallback wrapping it without also
/// re-explaining this half's mechanics inline.
fn parse_json_stream_strict(s: &str) -> Result<Vec<OwnedValue>> {
    // Validate the whole stream via `serde_json::Deserializer` as before
    // (for its own error message and its rejection of anything that isn't
    // valid, whitespace-or-self-delineated-separated JSON) -- but discard
    // each parsed `Value` rather than converting it, and materialize the
    // real result from the same byte span instead, via this crate's own
    // fidelity-preserving semi-indexer (#1058's fix for `--argjson`,
    // extended here to `--slurpfile`, #1093). `byte_offset()` gives each
    // value's own end position after a successful `.next()`; its start is
    // the first non-whitespace byte after the previous value's end -- JSON
    // ASCII whitespace specifically (`is_ascii_whitespace`, matching
    // `find_json_values`'s own convention below, not Rust's broader
    // Unicode `char::is_whitespace`), since that's the same set
    // `serde_json`'s own separator-skipping recognizes between stream
    // values.
    let mut values = Vec::new();
    let mut deserializer = serde_json::Deserializer::from_str(s).into_iter::<serde_json::Value>();
    let mut prev_end = 0;
    let bytes = s.as_bytes();

    while let Some(result) = deserializer.next() {
        result.context("Invalid JSON in stream")?;
        let end = deserializer.byte_offset();
        let start = bytes[prev_end..end]
            .iter()
            .position(|b| !b.is_ascii_whitespace())
            .map_or(prev_end, |offset| prev_end + offset);
        values.push(
            // Wrapped, not flattened to an `anyhow!` string: this is a data
            // error about the document, so the caller has to be able to tell
            // it apart from an I/O failure and report it in jq's own channel
            // at exit 5. Flattening lost that, and the run exited 1 with an
            // `Error:` prefix jq never prints (#1194).
            //
            // #2295: checked, not the bare materializer -- `serde_json`'s
            // own `Deserializer` above already rejected anything past its
            // default 128-deep recursion limit before this line runs, but
            // that's the same incidental (not designed-for-this-purpose)
            // protection #2295 found fragile elsewhere in this file --
            // this is the primary `--slurp`/`--slurpfile` document-input
            // path, worth the same defense-in-depth swap.
            json_bytes_to_owned_value_checked(&bytes[start..end])
                .map_err(|e| anyhow::Error::from(MalformedJsonError::new(e)))?,
        );
        prev_end = end;
    }

    Ok(values)
}

/// Build the values (and, when `!slurp`, one `(source, line)` location per
/// value) for `--seq` input across the whole file list at once (#1571).
///
/// Concatenates every file's raw bytes, in order, and materializes the
/// value ranges the reader's single raw-byte walk found over that same
/// concatenation (`jq_seq_reader::SeqStreamWalk::values`, #3247) -- matching real
/// jq's own `-s` reader, which treats the entire multi-file input as one
/// continuous RFC 7464 byte stream and doesn't stop scanning a record at a
/// file boundary. The reader's own EOF-location rule
/// (`jq_seq_reader::SeqStreamWalk::slurp_position_lost`) walks the same
/// full stream for the same reason: it can only ever see genuine
/// end-of-input, never a false EOF at some earlier file's own end -- no
/// separate per-file special-casing needed for that interaction.
///
/// `!slurp` locations come from the same walk, one per value
/// (`jq_seq_reader::SeqStreamWalk::locations`, #3250): the file and line
/// jq's reader stood at when it *yielded* the value, not the file holding
/// its end offset. A value dropped by the materializer drops its location
/// with it, since both are looked up by the value's index.
fn build_seq_values(
    // The raw sources, before any UTF-8 substitution (#3247).
    raw_sources: &[(Option<usize>, Vec<u8>)],
    // `jq_seq_reader::SeqStreamWalk::values` for these same sources: raw
    // offsets into their concatenation.
    ranges: &[(usize, usize)],
    // `jq_seq_reader::SeqStreamWalk::locations`, parallel to `ranges`: each
    // value's `(source index, line)` as jq reports it (#3250).
    value_locations: &[(usize, usize)],
    locations: &mut InputLocations,
    slurp: bool,
) -> Vec<OwnedValue> {
    // One source (stdin, or a single file) is borrowed as is; only a
    // multi-file stream, whose records can span a boundary, is copied into
    // one buffer.
    let combined: std::borrow::Cow<'_, [u8]> = match raw_sources {
        [(_, only)] => std::borrow::Cow::Borrowed(only),
        _ => {
            let mut all = Vec::with_capacity(raw_sources.iter().map(|(_, raw)| raw.len()).sum());
            for (_, raw) in raw_sources {
                all.extend_from_slice(raw);
            }
            std::borrow::Cow::Owned(all)
        }
    };
    let parsed = seq_values_with_indices(&combined, ranges);

    if !slurp {
        debug_assert_eq!(
            value_locations.len(),
            ranges.len(),
            "one location per value"
        );
        for &(_, index) in &parsed {
            let (source, line) = value_locations[index];
            locations.push(raw_sources[source].0.unwrap_or(0), line);
        }
    }

    parsed.into_iter().map(|(v, _)| v).collect()
}

/// Build the values and one `(source, line)` location per value for
/// raw-input (`-R`) mode across the whole file list at once (#1809).
///
/// Concatenates the files like [`build_seq_values`] and maps each line's
/// end offset back to a file and line with [`remap_ends_to_locations`]
/// (`--seq` no longer does: its locations come from where jq's reader
/// yielded each value, #3250): real jq's `-R` reader treats multiple files as one
/// continuous byte stream for line-splitting too -- confirmed live against
/// jq 1.7.1 that a file's own unterminated trailing line joins with the
/// next file's first line, the same way `--seq` joins a boundary-split
/// record. This can't reuse the `--seq` reader itself
/// (RS-delimited/JSON-shaped, not newline-shaped), so it does its own `\n`
/// scan to find each line's byte range instead.
///
/// Splits on `\n` matching [`str::lines`]'s own rule that a trailing `\n`
/// does not open an extra empty final line, but -- unlike `str::lines()`
/// -- keeps a trailing `\r` as part of each line's content rather than
/// stripping it: real jq's `-R` reader never strips `\r` either (confirmed
/// live: `printf 'abc\r\n' | jq -R -c '.'` => `"abc\r"`, not `"abc"`). This
/// needs its own scan rather than calling `str::lines()` regardless, since
/// that discards the byte offsets this needs for the remap.
///
/// Attributing each line's *end* offset to a file via
/// `remap_ends_to_locations` also fixes a real jq 1.7.1 line-number quirk
/// single-file `-R` already had wrong: a final line with no trailing `\n`
/// reports the *previous* completed line's number, not one past it
/// (confirmed live: `printf 'abc\ndef' | jq -R -c '., input_line_number'`
/// reports `1` for both lines, not `1` then `2`) -- no test previously
/// covered this, since every existing `-R` fixture's lines were all
/// `\n`-terminated.
///
/// Never called under `--slurp`: `-R -s` without `--input-dsv` returns
/// early above this point, and `-R -s --input-dsv` takes the per-file DSV
/// branch instead (DSV is checked first in that loop's `if`/`else`), so
/// `args.raw_input && args.input_dsv.is_none()` -- this function's own
/// call-site guard -- can only be reached with `slurp == false`.
fn build_raw_input_values(
    raw_inputs: &[(Option<usize>, String)],
    locations: &mut InputLocations,
) -> Vec<OwnedValue> {
    let (combined, file_ends) = concat_with_file_ends(raw_inputs);

    // ((content start, content end), line's own attribution-end offset --
    // the `\n`'s own position, or `combined.len()` for a final unterminated
    // line).
    let mut lines: Vec<((usize, usize), usize)> = Vec::new();
    let bytes = combined.as_bytes();
    let mut start = 0usize;
    for (idx, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            lines.push(((start, idx), idx));
            start = idx + 1;
        }
    }
    if start < bytes.len() {
        lines.push(((start, bytes.len()), bytes.len()));
    }

    remap_ends_to_locations(
        lines.iter().map(|&(_, end)| end),
        raw_inputs,
        &file_ends,
        locations,
    );

    lines
        .into_iter()
        .map(|((content_start, content_end), _)| {
            OwnedValue::String(combined[content_start..content_end].to_string())
        })
        .collect()
}

/// Concatenate every file's decoded content, in order, into one string,
/// with `jq_seq_reader::source_ends` for it -- [`build_raw_input_values`]'s
/// view of the whole multi-file input as one continuous stream of lines.
fn concat_with_file_ends(raw_inputs: &[(Option<usize>, String)]) -> (String, Vec<usize>) {
    let combined: String = raw_inputs.iter().map(|(_, raw)| raw.as_str()).collect();
    (combined, crate::jq_seq_reader::source_ends(raw_inputs))
}

/// Map each `end` offset in `ends` (non-decreasing, an exclusive position
/// within the `combined` stream `file_ends` was built from -- see
/// [`concat_with_file_ends`]) to its owning file and file-local line
/// number, pushing one `(source, line)` location per `end` onto
/// `locations` in the same order. [`build_raw_input_values`]'s (`-R`) only:
/// `--seq` names a value from where jq's reader yielded it instead
/// (`jq_seq_reader::value_locations`, #3250), which this end-offset rule
/// gets wrong for a value ending at a file's end.
///
/// A value/line ending *exactly* at a file boundary is attributed to the
/// file *starting* there, not the file ending there: `partition_point`'s
/// `fe <= end` predicate (not `fe < end`) is what makes that call, since
/// `file_ends[i]` is both file `i`'s own exclusive end and file `i+1`'s
/// start offset -- an `end` equal to that offset means the byte the
/// line's own trailing delimiter occupies is the *first* byte of
/// file `i+1`, not the last byte of file `i`. Getting this wrong (an
/// earlier version of both callers used `fe < end`) misattributes the
/// line/record to the wrong file entirely whenever a file's sole content is
/// the delimiter that terminates the *previous* file's unterminated
/// trailing content -- confirmed live against jq 1.7.1 in exactly that
/// degenerate case (three files `"abc"`, `"\n"`, `"def\n"`): both the
/// reported line number and the `error(...)` location moved to the correct
/// (second) file once fixed, matching jq exactly; before the fix both
/// pointed at the first file instead.
///
/// `current` caches the one [`LineCounter`] in use, replaced only when
/// `partition_point` reports a new file index -- since `end` values are
/// non-decreasing, that index is too, so this never backtracks to a file
/// already passed.
fn remap_ends_to_locations<S: AsRef<[u8]>>(
    ends: impl Iterator<Item = usize>,
    raw_inputs: &[(Option<usize>, S)],
    file_ends: &[usize],
    locations: &mut InputLocations,
) {
    let mut current: Option<(usize, LineCounter<'_>)> = None;
    for end in ends {
        let file_idx = file_ends
            .partition_point(|&fe| fe <= end)
            .min(file_ends.len().saturating_sub(1));
        if current.as_ref().map(|(idx, _)| *idx) != Some(file_idx) {
            current = Some((file_idx, LineCounter::new(raw_inputs[file_idx].1.as_ref())));
        }
        let file_start = if file_idx == 0 {
            0
        } else {
            file_ends[file_idx - 1]
        };
        let src = raw_inputs[file_idx].0.unwrap_or(0);
        let line = current
            .as_mut()
            .expect("just set above")
            .1
            .advance_to(end.saturating_sub(file_start));
        locations.push(src, line);
    }
}

/// Materialize the `--seq` values [`jq_seq_reader::walk_stream`] found:
/// each `(start, end)` range is raw offsets into `combined`, the
/// concatenated raw sources, and comes back paired with its index in
/// `ranges`, which is also its index in the walk's locations.
///
/// The reader already checked every value's grammar, so this only
/// decodes; `json_bytes_to_owned_value_checked` stays as defense in depth
/// (#2295). It keeps number-literal source fidelity (#1058/#1093), jq's
/// leading-zero tolerance (`007e5`, #1243), and an `f64`-overflowing
/// literal (`1e400` -> `1E+400`, #1267), all pinned by this module's
/// tests.
///
/// UTF-8 substitution happens here, per value, and only when the stream
/// has invalid UTF-8 at all (#3247). A value the reader accepted has
/// invalid bytes only inside its strings, and the substitution is scoped
/// per string (#1743), so this is what substituting the whole document
/// would have given -- without rewriting the bytes between values that the
/// reader decides on.
///
/// [`jq_seq_reader::walk_stream`]: crate::jq_seq_reader::walk_stream
fn seq_values_with_indices(combined: &[u8], ranges: &[(usize, usize)]) -> Vec<(OwnedValue, usize)> {
    // One pass over the whole stream. Almost every stream is valid, and then
    // no value needs a check of its own; otherwise only a value reaching
    // past the first invalid byte can hold one.
    let first_invalid = succinctly::text::utf8::validate_utf8(combined)
        .err()
        .map_or(usize::MAX, |e| e.offset);
    ranges
        .iter()
        .enumerate()
        .filter_map(|(index, &(start, end))| {
            let raw = &combined[start..end];
            let substituted;
            let bytes =
                if end <= first_invalid || succinctly::text::utf8::validate_utf8(raw).is_ok() {
                    raw
                } else {
                    substituted =
                        succinctly::jq::utf8_document::substitute_invalid_utf8_jq_document(raw);
                    substituted.as_bytes()
                };
            // #2295: the sequence reader already checks the grammar, but
            // retain the checked materializer as defense in depth.
            json_bytes_to_owned_value_checked(bytes)
                .ok()
                .map(|value| (value, index))
        })
        .collect()
}

/// Real jq's own stderr warning ("`jq: ignoring parse error: ...`") for a
/// `--seq` (RFC 7464) input with no RS byte anywhere at all -- one of
/// jq's several message templates for a dropped malformed record; see
/// `get_inputs`'s own call site for which combinations must never reach
/// this at all (`-R`, DSV, `-n` + a forced real read). `None` if `raw_bytes`
/// contains an RS byte anywhere (a different, unimplemented set of
/// templates applies then -- #1723) or is fully empty of any source.
///
/// A single pass over every source's bytes, bailing out the moment an RS
/// byte is seen, rather than one pass to check for an RS byte and a
/// second to count line/column -- the two were separate passes over the
/// identical bytes in an earlier version of this function, found by
/// review. Operates on raw bytes, not `--seq`'s later UTF-8-decoded
/// `String`s: an invalid byte becomes a 3-byte U+FFFD once
/// `substitute_invalid_utf8_jq_style` (#1617) runs, which would overcount
/// the column real jq counts against the *original* stream.
///
/// A leading UTF-8 BOM is stripped before counting, matching real jq
/// (oracle-verified: `printf '\xef\xbb\xbf1 2' | jq --seq '.'` reports
/// column 3, not 6) -- tracked via `bytes_seen_before_this_source == 0`,
/// not "is this the first element of `raw_bytes`": an earlier version
/// used the latter and silently failed to strip a BOM that lived in the
/// first *non-empty* source when preceded by an empty one (found by
/// review, oracle-verified: `jq --seq -c '.' empty.txt bom.txt` strips
/// the BOM in the second file, since real jq treats every source as one
/// continuous stream).
///
/// Two narrower gaps deliberately not chased, both apparent artifacts of
/// jq's own C byte-reader rather than a documented rule: an embedded NUL
/// byte stops jq's own column count from advancing any further on that
/// line (oracle-verified: `ab\0cd` reports column 2, not 5), and jq's BOM
/// detection consumes a *partial*, never-completed `EF`/`EF BB` prefix
/// too, not just a full 3-byte match. Both are folded into #1723's
/// existing "matching jq's own incremental-reader internals precisely"
/// scope rather than given a separate issue.
fn seq_no_rs_byte_warning(raw_bytes: &[(Option<usize>, Vec<u8>)]) -> Option<String> {
    let mut line = 1usize;
    let mut column = 0usize;
    // Shares `stream_bytes` with `jq_seq_reader`, which walks this same
    // stream to classify the templates that apply once an RS *is* present
    // (#1723) -- the two must agree on BOM handling and on where a column
    // starts, so they read the stream through one iterator.
    for b in crate::jq_seq_reader::stream_bytes(raw_bytes) {
        if b == ASCII_RS {
            return None;
        }
        if b == b'\n' {
            line += 1;
            column = 0;
        } else {
            column += 1;
        }
    }
    if raw_bytes.is_empty() {
        return None;
    }
    Some(crate::jq_seq_reader::ignored_parse_error(&format!(
        "Unfinished abandoned text at EOF at line {line}, column {column}"
    )))
}

/// Validate that the DSV delimiter is acceptable.
/// Returns an error if the delimiter is a special CSV character.
fn validate_dsv_delimiter(delimiter: char) -> Result<()> {
    // Disallow characters with special meaning in CSV parsing
    match delimiter {
        '"' => Err(anyhow::anyhow!(
            "Invalid delimiter '\"': quote character cannot be used as delimiter"
        )),
        '\n' | '\r' => Err(anyhow::anyhow!(
            "Invalid delimiter: newline characters cannot be used as delimiter"
        )),
        c if !c.is_ascii() => Err(anyhow::anyhow!(
            "Invalid delimiter '{c}': only ASCII characters are supported"
        )),
        _ => Ok(()),
    }
}

/// Parse DSV (delimiter-separated values) input into JSON arrays.
/// Each row becomes a JSON array of strings.
fn parse_dsv_input(s: &str, delimiter: char) -> Vec<OwnedValue> {
    use succinctly::dsv::{Dsv, DsvConfig};

    let config = DsvConfig::default().with_delimiter(delimiter as u8);

    let dsv = Dsv::parse_with_config(s.as_bytes(), &config);
    let mut values = Vec::with_capacity(dsv.row_count());

    for row in dsv.rows() {
        let fields: Vec<OwnedValue> = row
            .fields()
            .map(|field| {
                // Strip quotes from quoted fields and decode the content
                let field_str = strip_quotes_and_decode(field);
                OwnedValue::String(field_str)
            })
            .collect();
        values.push(OwnedValue::array_from(fields));
    }

    values
}

/// Strip surrounding quotes from a field and handle escaped quotes.
fn strip_quotes_and_decode(field: &[u8]) -> String {
    let s = String::from_utf8_lossy(field);

    // Check if field is quoted
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        // Remove surrounding quotes
        let inner = &s[1..s.len() - 1];
        // Unescape doubled quotes ("" -> ")
        inner.replace("\"\"", "\"")
    } else {
        s.into_owned()
    }
}

/// Evaluate the expression against an input value, handing each output to
/// `on_value` the moment the evaluator produces it (#1653).
///
/// Replaced the eager `evaluate_input`, which collected a whole input's
/// results into a `Vec` and returned them for the caller to write
/// afterwards; once every call site streamed, that function had no callers
/// left and was removed rather than kept as a second, divergent copy of the
/// same per-variant materialization.
///
/// Real jq is a lazy generator, so a filter that writes to stdout *and*
/// triggers a stderr side effect (`debug`, `stderr`, `halt_error`) or raises
/// mid-stream interleaves the two in real time. Collecting first cannot
/// reproduce that ordering however the writes are buffered -- every stderr
/// write has already happened before the first stdout write runs -- which is
/// why `--unbuffered`'s per-write `flush()` alone never fixed it.
///
/// `on_value` is handed the same `&mut ErrorSink` this function holds, rather
/// than capturing it: the writer needs it for `route_write_error`, and the
/// borrow checker cannot see that the two uses never overlap in time.
///
/// `on_value` returns `false` to stop the generator (a write error the caller
/// is already reporting). An uncaught error is reported to `sink` and yields
/// no values, so evaluation continues with the next input the way jq does and
/// `sink` drives the exit code (#355) -- but it is reported *after* the
/// outputs that preceded it have been written, which is what makes
/// `1, error("x"), 3` print `1` before its diagnostic like jq.
///
/// A per-item materialization failure (`materialize_stream_item`'s
/// `sink.materialize` calls) is defense-in-depth rather than reachable in
/// practice: `cursor` is always rooted in `input.to_json_input_bridge()`/
/// `input.to_json_jq_preserve()` (#2852), a fresh serialization of an
/// already-decoded `OwnedValue` -- a Rust `String`, which by construction
/// cannot hold an undecodable byte sequence, and whose escapes this crate's
/// own serializer writes. Same argument `eval_generic.rs`'s
/// textually-similar bridge relies on (search "defense-in-depth" there).
/// Kept as a reported diagnostic rather than an `unwrap()` so a real
/// failure, if that invariant is ever violated, surfaces as an ordinary
/// `EvalError` instead of a panic.
enum InputDoc {
    /// `--preserve-input`: text carries no bridge token, indexed like any
    /// ordinary document.
    Preserve(String, JsonIndex),
    /// The reindex bridge's own text, paired with its flagged index
    /// (`OwnedValue::input_bridge_doc`).
    Bridge(succinctly::jq::ReindexedDoc),
}

impl InputDoc {
    fn root(&self) -> JsonCursor<'_> {
        match self {
            Self::Preserve(text, index) => index.root(text.as_bytes()),
            Self::Bridge(doc) => doc.root(),
        }
    }
}

fn evaluate_input_streaming(
    input: &OwnedValue,
    expr: &jq::Expr,
    _context: &EvalContext,
    at: &ErrorAt<'_>,
    sink: &mut ErrorSink,
    output_config: &OutputConfig,
    on_value: &mut dyn FnMut(&mut ErrorSink, OwnedValue) -> Result<bool>,
) -> Result<()> {
    // #2852: `to_json()` always reformats a `NumberLiteral` to jq's own
    // spelling -- before this, every call site here reindexed through it
    // unconditionally, silently reformatting this one input value's own
    // numbers regardless of `--preserve-input` (a document read via the
    // `input`/`inputs` builtin themselves was unaffected, since those
    // resolve from an already-materialized queue that never reaches this
    // function or either `to_json` variant).
    //
    // #2874: the *choice* now reads off the one convention value rather than
    // an independent bool. The `to_json*` family itself deliberately stays
    // outside a `JsonConvention`-keyed mapping: `to_json_at_depth` hardcodes
    // jq's escape table for all three of its callers, so `to_json_yq` is
    // "`Preserve` numbers + *jq* escaping", and keying the family on the
    // enum would assert an equivalence that does not hold. See
    // `OwnedValue::to_json_yq`'s own doc comment.
    //
    // #2877: `to_json_input_bridge`, not `to_json` -- a document `nan`/
    // `Infinity` is a bare non-finite `Float` here, and the printer's
    // `null`/`DBL_MAX` substitutions would re-read as a real `null` and a
    // finite literal. The bridge variant writes the reindex tokens the
    // reparse already decodes. The `--preserve-input` arm keeps
    // `to_json_jq_preserve` (see that variant's doc comment for why).
    //
    // #3034: only the bridge variant's text is indexed as bridge text
    // (`JsonIndex::build_reindex`), the one index its tokens decode under.
    // `to_json_jq_preserve` writes no token, so its text is indexed like any
    // document. `input_bridge_doc` pairs the bridge text with its flagged
    // index (`OwnedValue::input_bridge_doc`'s own doc comment) so this site
    // can't forget the flag the way a hand-rolled `to_json_input_bridge` +
    // `JsonIndex::build_reindex` pair could.
    let preserve = output_config.convention.preserves_source_values();
    let doc = if preserve {
        let text = input.to_json_jq_preserve();
        let index = JsonIndex::build(text.as_bytes());
        InputDoc::Preserve(text, index)
    } else {
        InputDoc::Bridge(input.input_bridge_doc())
    };
    let cursor = doc.root();

    let mut write_err: Option<anyhow::Error> = None;
    let control = jq::eval_generic::eval_each_with_cursor(expr, cursor, &mut |result| {
        match materialize_stream_item(result, sink, at) {
            // Nothing to write: either genuinely no value, or a failure this
            // call already reported to `sink` (same "report and keep going"
            // contract the eager path's own arms followed, #355).
            None => true,
            // The eager route's copy of `to_jq_values`' depth gate (#3009
            // review): every caller writes through `write_output_owned_value`,
            // whose printer only refuses an over-deep value after 384 levels
            // of `[` are already on stdout. Checked here, before `on_value`,
            // so such a value prints nothing, is reported at exit 5 and the
            // generator carries on -- the lazy route's behaviour, and not the
            // `format_json` panic this route had before the writers merged.
            Some(v) => match v.check_tree_depth() {
                Err(e) => {
                    sink.report(DiagStyle::Jq, &e, &at.resolve());
                    true
                }
                Ok(()) => match on_value(sink, v) {
                    Ok(keep_going) => keep_going,
                    Err(e) => {
                        write_err = Some(e);
                        false
                    }
                },
            },
        }
    });
    // The control is reported *before* any write error is surfaced. The eager
    // path reported it during evaluation, i.e. always before the caller's
    // write loop could fail; returning `Err` first here would let an I/O
    // failure swallow the evaluator's own diagnostic (review finding).
    match control {
        None => {}
        Some(jq::Control::Error(e)) => sink.report(DiagStyle::Jq, &e, &at.resolve()),
        Some(jq::Control::Break(label)) => sink.report_break(DiagStyle::Jq, &label, &at.resolve()),
        // `halt`/`halt_error` (#791): not a diagnostic, so no `sink.report*`
        // call -- matching the eager path's own `Halt` arm.
        Some(jq::Control::Halt(code)) => sink.request_halt(code),
    }
    if let Some(e) = write_err {
        return Err(e);
    }
    Ok(())
}

/// One streamed output as an `OwnedValue`, or `None` when there is nothing to
/// write (a decode failure already reported to `sink`, or an empty result).
///
/// Carries the per-variant materialization the eager `evaluate_input` used to
/// do inline (removed in #1653, once every call site streamed), for exactly
/// the variants a *single* sink item can be.
///
/// That is a strictly smaller set than `GenericResult`'s: a sink item is
/// always a `GenericItem`, and `generic_item_to_result` maps its six variants
/// onto `One`/`OneCursor`/`Owned`/`LazyKeys`/`LazyIndexRange`/`LazySeq` and
/// nothing else. The remaining eight -- `None`, `Error`, `Break`, `Halt`, and
/// the four multi-value shapes -- are therefore unreachable *by
/// construction*, not merely unlikely, so they share one arm rather than
/// eight speculative ones that could never run (an earlier draft spelled all
/// eight out, which read as real handling and left a dozen permanently
/// uncovered lines behind). A control never arrives as an item either: it is
/// returned as the `Flow`'s own outcome and reported by the caller.
fn materialize_stream_item<V: succinctly::jq::document::DocumentValue>(
    result: GenericResult<V>,
    sink: &mut ErrorSink,
    at: &ErrorAt<'_>,
) -> Option<OwnedValue> {
    match result {
        // #2299: `generic_to_owned_checked` (`eval_generic::to_owned_checked`),
        // not the plain `generic_to_owned` this file's other call sites use
        // -- that one panics past `MAX_NESTING_DEPTH` deliberately (it's the
        // evaluator's own hot-path recursion guard, not meant to be
        // `try`/`catch`-able), but this is a CLI-output boundary with no
        // filter evaluation left to protect, the same reasoning
        // `json_bytes_to_owned_value_checked` already applies on the input
        // side. Confirmed live: `--slurp`'s own extra array-wrapping level
        // pushed an otherwise-safe 255-deep document one level past the
        // ceiling here specifically, panicking (exit 101) instead of
        // reporting a clean, catchable error.
        GenericResult::One(v) => sink.materialize(
            DiagStyle::Jq,
            generic_to_owned_checked::<JqSemantics, _>(&v),
            &at.resolve(),
        ),
        GenericResult::OneCursor(c) => sink.materialize(
            DiagStyle::Jq,
            generic_to_owned_checked::<JqSemantics, _>(
                &succinctly::jq::document::DocumentCursor::value(&c),
            ),
            &at.resolve(),
        ),
        GenericResult::Owned(v) => Some(v),
        // Same fallback reasoning the eager path's `LazyKeys` arm carried:
        // a fast-pathed `keys | length` never reaches this boundary, so this
        // only fires for bare `keys`/`keys_unsorted`. Sort iff `sorted`
        // (#683), matching eager `Keys`.
        GenericResult::LazyKeys {
            fields,
            sorted,
            collapse,
        } => {
            let mut keys = sink.materialize(
                DiagStyle::Jq,
                effective_keys(&fields, collapse),
                &at.resolve(),
            )?;
            if sorted {
                keys.sort();
            }
            Some(OwnedValue::Array(
                keys.into_iter().map(OwnedValue::String).collect(),
            ))
        }
        GenericResult::LazyIndexRange(len) => Some(OwnedValue::Array(
            (0..len).map(|i| OwnedValue::Int(i as i64)).collect(),
        )),
        GenericResult::LazySeq(seq) => match seq.materialize_atomic::<JqSemantics>() {
            Ok(v) => Some(v),
            Err(jq::Control::Error(e)) => {
                sink.report(DiagStyle::Jq, &e, &at.resolve());
                None
            }
            Err(jq::Control::Break(label)) => {
                sink.report_break(DiagStyle::Jq, &label, &at.resolve());
                None
            }
            Err(jq::Control::Halt(code)) => {
                sink.request_halt(code);
                None
            }
        },
        // The eight shapes a sink item provably never takes (see this
        // function's doc comment). `None` rather than `unreachable!()` so a
        // future regression cannot take the process down -- but it would drop
        // values rather than print them, so the trade is stated here instead
        // of being dressed up as a graceful fallback it is not.
        GenericResult::None
        | GenericResult::Error(_)
        | GenericResult::Break(_)
        | GenericResult::Halt(_)
        | GenericResult::Many(_)
        | GenericResult::ManyCursor(_)
        | GenericResult::ManyOwned(_)
        | GenericResult::Partial(..) => None,
    }
}

/// If `payload` (a caught panic's payload) is exactly
/// `to_owned_cursor_at_depth`'s `MAX_NESTING_DEPTH` guard (#1793), returns
/// its message; `None` for any other panic, so a caller can `resume_unwind`
/// anything unrelated rather than silently treating an unexpected panic as
/// this specific, known one.
///
/// An *exact* match against `assert_depth`'s own message template
/// (`src/jq/value.rs`), not a substring check -- `assert_value_tree_depth`
/// (`MAX_VALUE_TREE_DEPTH`, 384) shares that same template via the same
/// underlying `assert_depth` call and produces byte-identical text apart
/// from the number, so a substring match here would also silently catch
/// *that* guard's panic (a different failure class, from filter-driven
/// value growth rather than document nesting) and report it as if it were
/// this one. Confirmed live by review: `reduce range(400) as $i (null;
/// [.])` panics via the 384 guard and was being caught here before this
/// fix narrowed the match.
///
/// `assert!`'s formatted message (`"nesting depth exceeds limit of
/// {MAX_NESTING_DEPTH}"`) panics with a `String` payload, not `&'static
/// str` -- checked first since it's the only shape this specific guard
/// actually produces; the `&str` check is defense-in-depth for a future
/// caller of this same helper against an unformatted `panic!("literal")`.
fn nesting_depth_panic_message(payload: &(dyn core::any::Any + Send)) -> Option<String> {
    let text = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())?;
    (text == format!("nesting depth exceeds limit of {MAX_NESTING_DEPTH}"))
        .then(|| text.to_string())
}

/// One printable result, in whichever representation the evaluator already
/// had it in (#3009).
///
/// `GenericResult` arrives in two shapes: some arms carry cursors into the
/// source document, which `JqValue` exists to print lazily, and others
/// (`Owned`, `ManyOwned`, `Partial`, the sorted-`keys` arm) carry a finished
/// `OwnedValue`. Those used to be converted into `JqValue` via
/// `JqValue::try_from_owned` before printing -- a full rebuild of a tree that
/// was already owned, freeing each source map and allocating the destination
/// one, which since #3000's layout change lands in a different allocator bin
/// and shows up as peak RSS. This enum lets each shape reach its own writer
/// instead.
///
/// Deliberately *not* a new `JqValue::Owned` variant, which would have been
/// the smaller diff: `JqValue` is `pub`, matched across nine files, and
/// `src/jq/lazy.rs` alone answers ~19 questions about it through `_ =>`
/// wildcards (`is_truthy`, `as_str`, `as_i64`, `length`, ...). Every one of
/// those would compile clean and answer *wrongly* for an owned arm --
/// `is_truthy`'s `_ => true` is a wrong `-e` exit code, `as_str`'s
/// `_ => None` silently drops `-r`. A print-path optimisation must not be
/// able to move `-r`/`-e` semantics at all. Here the compiler checks every
/// destructure, and there are only two of them plus the sink closure.
#[derive(Debug)]
enum OutputItem<'a, W = Vec<u64>> {
    /// A result that is (or contains) a cursor into the source document.
    Lazy(JqValue<'a, W>),
    /// A result the evaluator already materialised.
    Owned(OwnedValue),
}

/// One M2 output, handed to the caller's writer. Named so the `&mut dyn`
/// spelling below stays inside clippy's `type_complexity` budget.
type JqValueSink<'a, 'w> = dyn FnMut(&mut ErrorSink, OutputItem<'a>) -> Result<bool> + 'w;

/// Writes the newline jq puts after every top-level output value. Generic
/// over `W: core::fmt::Write` so the same function works as
/// `GenericResult::stream_json`'s `on_value` callback whether the concrete
/// writer is a `String` buffer or an `FmtWriter` (#1576) -- a closure value
/// has one fixed concrete signature, which the two call sites below don't
/// share.
fn write_result_newline<W: core::fmt::Write>(w: &mut W) -> core::fmt::Result {
    w.write_char('\n')
}

/// Whether `expr` is guaranteed to produce *at most one* top-level result
/// (#1576) -- the extra condition `can_json_fast_path` requires on top of
/// `can_use_m2_streaming`'s own AST whitelist, jq-only. See
/// `evaluate_m2_fast_path`'s own doc comment for why this matters: that
/// function falls back to the general path on any detected malformation,
/// which is only safe when a *later* result's failure can't follow
/// already-written earlier ones.
///
/// `Expr::Iterate` (`.[]`) and `Builtin::Select` are the two exclusions --
/// the base whitelist's own only multi-result-capable shapes (a fanning
/// `cond` can republish `select`'s input more than once). Everything else
/// admitted by `can_use_m2_streaming` produces exactly one result by
/// construction: `map`/`sort`/`sort_by`/`unique`/`unique_by`/`reverse`/
/// `keys_unsorted` each build one array; `min`/`min_by`/`max`/`max_by`
/// pick one value; `first`/`last`/`nth` bound themselves to one result
/// (or none) regardless of what their own body could otherwise yield --
/// including a body containing `.[]`, which is why those three don't
/// recurse into their inner expression here at all, unlike
/// `can_use_m2_streaming`'s own recursion for the same shapes (that
/// recursion answers a different question: whether the inner shape can
/// stream a cursor at all, not how many results the outer wrapper emits).
fn m2_json_fallback_safe(expr: &Expr) -> bool {
    match expr {
        Expr::Iterate => false,
        Expr::Builtin(Builtin::Select(_)) => false,
        Expr::Pipe(exprs) => exprs.iter().all(m2_json_fallback_safe),
        Expr::Optional(inner) | Expr::Paren(inner) => m2_json_fallback_safe(inner),
        Expr::FirstExpr(_)
        | Expr::LastExpr(_)
        | Expr::NthExpr { .. }
        | Expr::Builtin(
            Builtin::FirstStream(_) | Builtin::LastStream(_) | Builtin::NthStream(_, _),
        ) => true,
        // `limit(n; ...)` can yield up to `n` results, unlike `first`/
        // `last`/`nth` above -- `Expr::Limit` isn't safe here even though
        // `can_use_m2_streaming` allows it.
        Expr::Limit { .. } => false,
        // `IndexExpr` (a computed index, `.[expr]`) is excluded, unlike
        // `Index` (a fixed literal index) -- confirmed live (#1576 review)
        // that a computed index target/key can itself raise mid-stream
        // after already producing output (`test_computed_index_*_streams_
        // prefix_*` in `tests/jq_cli_tests.rs`), which falling back on
        // would duplicate the already-written prefix rather than replace
        // it. `can_use_m2_streaming` still allows it for yq, whose own
        // gate doesn't fall back on error at all.
        Expr::Identity | Expr::Field(_) | Expr::Index { .. } => true,
        Expr::Builtin(
            Builtin::Sort
            | Builtin::SortBy(_)
            | Builtin::Unique
            | Builtin::UniqueBy(_)
            | Builtin::Min
            | Builtin::MinBy(_)
            | Builtin::Max
            | Builtin::MaxBy(_)
            | Builtin::Reverse,
        ) => true,
        Expr::Builtin(Builtin::Map(_)) => true,
        // `keys_unsorted` (`GenericResult::LazyKeys`) writes through
        // `stream_lazy_keys_json` (`src/jq/stream.rs`), a separate writer
        // from `JsonCursor::stream_json`'s buffering (#1576 review) --
        // confirmed live that it doesn't yet detect a malformed key the
        // way `stream_json_pretty`'s object arm does (`keys_unsorted` on
        // `{invalid}` silently answers `[]` instead of erroring), so its
        // own fallback-safety hasn't been established. Excluding it here
        // routes it to the general path unconditionally, not just on a
        // detected error -- a well-scoped follow-up, not something this
        // change needs to fix.
        Expr::Builtin(Builtin::KeysUnsorted) => false,
        _ => false,
    }
}

/// Evaluate one JSON document through the M2 fast path (#1576): stream
/// straight from `GenericResult`'s cursors instead of the materializing
/// `generic_result_to_jq_values`/`print_json` stack that feeds
/// [`evaluate_bytes_streaming`]. Only reached when the caller's own
/// `can_json_fast_path` (AST shape + flags) already holds -- which,
/// crucially, only admits expressions [`m2_json_fallback_safe`] confirms
/// produce *at most one* top-level result (`map`/`sort`/etc., plain
/// navigation, `keys_unsorted`, `first`/`last`/`nth` -- never `.[]`/
/// `Iterate` or `select`, which can yield several).
///
/// Returns `Ok(true)` when the result was written (or a genuine `halt`/
/// `halt_error` was requested) and the caller should move to the next
/// document; `Ok(false)` when this path detected a malformed/undecodable
/// document and deliberately wrote *nothing*, so the caller should fall
/// back to the general [`evaluate_bytes_streaming`] path for this one
/// document instead of reporting from here.
///
/// **Why fall back rather than report directly** (#1576 review): jq's own
/// `print_json`/`to_owned_cursor`/`DistinctKeyCursors` stack has
/// accumulated years of issue-specific, sometimes deliberately
/// *inconsistent* malformed-input handling (#1641 wants a truncated
/// partial prefix for a bad *value*; #1676 wants zero output for a bad
/// *delimiter*; #1642's `DisplayKeyGuard` catches a narrower colliding-
/// decode-failure-key case none of this writer's checks replicate) --
/// re-deriving every one of those exactly in this new writer is its own,
/// separately-scoped effort. Falling back to the already-correct general
/// path for the rare malformed case keeps this fast path's own contract
/// simple (validate then write, or write nothing) while still getting
/// every one of those existing behaviors for free, at the cost of
/// re-evaluating the one malformed document twice.
///
/// **Atomicity for the happy path** (#2066): real jq's array construction
/// is all-or-nothing -- `[1,2,"x"]|map(.+1)` prints nothing to stdout,
/// only the stderr diagnostic, verified against the regression that fix
/// closed (`map(.)` on `[1, {"bad": xyz123}]` used to print a garbled
/// `[1,{"bad":` prefix before erroring). `GenericResult::stream_json`'s
/// `LazySeq` arm streams from cursors as it goes, which is fine for yq --
/// its own cursor-streaming path already accepts a partial prefix on a
/// later element's decode failure as a settled trade
/// (`stream_maybe_colored`'s own doc comment, #1641/#1679) -- so this
/// buffers a `LazySeq` result locally and only copies it to `out` (or
/// falls back) once fully resolved. The non-`LazySeq` branch is safe
/// *without* its own buffer here for the same reason: `m2_json_fallback_
/// safe` guarantees at most one top-level result, and `JsonCursor::
/// stream_json` (`src/json/light.rs`) already buffers internally per
/// value for exactly this reason, so nothing reaches `out` (the real
/// writer, not the `FmtWriter` wrapping it) until that one result is
/// known good.
#[allow(clippy::too_many_arguments)] // one caller, one gate (`can_json_fast_path`); a struct would hide the 1:1 relationship each param has to a specific CLI flag.
fn evaluate_m2_fast_path<W: Write>(
    json_bytes: &[u8],
    expr: &jq::Expr,
    index: &JsonIndex,
    sink: &mut ErrorSink,
    out: &mut W,
    indent: IndentSpec,
    sort_keys: bool,
    numbers: JsonConvention,
    exit_status: bool,
    had_output: &mut bool,
    last_output: &mut Option<OwnedValue>,
) -> Result<bool> {
    let cursor = index.root(json_bytes);
    let result = eval_with_cursor(expr, cursor);

    // `stats.last_was_falsy` (not the value itself) is all `-e`'s own check
    // downstream ever reads out of `last_output` (`matches!(last,
    // OwnedValue::Null | OwnedValue::Bool(false))`) -- `Bool(false)` is a
    // faithful stand-in for "the real last value was falsy" regardless of
    // whether that value was actually `null` or `false`, and `Bool(true)`
    // likewise for "was truthy", regardless of the real value's own type.
    let record_exit_status = |last_output: &mut Option<OwnedValue>, stats: &StreamStats| {
        if exit_status && stats.count > 0 {
            *last_output = Some(OwnedValue::Bool(!stats.last_was_falsy));
        }
    };

    if matches!(result, GenericResult::LazySeq(_)) {
        let mut buf = String::new();
        let stats = result
            .stream_json::<_, JqSemantics>(
                &mut buf,
                indent,
                sort_keys,
                numbers,
                write_result_newline,
            )
            .map_err(|_| anyhow::anyhow!("write error"))?;
        if let Some(code) = stats.halt {
            sink.request_halt(code);
            return Ok(true);
        }
        if stats.error.is_some() {
            return Ok(false);
        }
        *had_output = *had_output || stats.count > 0;
        record_exit_status(last_output, &stats);
        out.write_all(buf.as_bytes())?;
        Ok(true)
    } else {
        let mut writer = FmtWriter(out);
        let stats = result
            .stream_json::<_, JqSemantics>(
                &mut writer,
                indent,
                sort_keys,
                numbers,
                write_result_newline,
            )
            .map_err(|_| anyhow::anyhow!("write error"))?;
        if let Some(code) = stats.halt {
            sink.request_halt(code);
            return Ok(true);
        }
        // `m2_json_fallback_safe` guarantees at most one top-level result
        // for every expression that reaches this branch, and
        // `JsonCursor::stream_json` buffers that one result internally
        // before ever touching `writer`/`out` -- so an error here means
        // nothing has reached `out` yet, same as the `LazySeq` branch
        // above, and falling back is equally safe.
        if stats.error.is_some() {
            return Ok(false);
        }
        *had_output = *had_output || stats.count > 0;
        record_exit_status(last_output, &stats);
        Ok(true)
    }
}

/// Evaluate against raw JSON bytes, handing each output to `on_value` the
/// moment the evaluator produces it (#1653).
///
/// The M2 counterpart of [`evaluate_input_streaming`], and deliberately not a
/// call into it: that one takes an already-materialized `OwnedValue` and
/// re-indexes it through `to_json()`/`to_json_jq_preserve()` (#2852), where
/// this streams from the `JsonIndex` the caller already built and keeps the
/// lazy [`write_output_jq_value`] writer, so a transparent filter still
/// writes raw source spans rather than materializing every output.
///
/// `on_value` returns `false` to stop the generator. `sink` is passed *into*
/// it rather than captured, so this function can keep its own `&mut` for the
/// control arms below -- the same shape `evaluate_input_streaming` uses.
fn evaluate_bytes_streaming<'a>(
    json_bytes: &'a [u8],
    expr: &jq::Expr,
    index: &'a JsonIndex,
    at: &InputLocation,
    sink: &mut ErrorSink,
    on_value: &mut JqValueSink<'a, '_>,
) -> Result<()> {
    let cursor = index.root(json_bytes);
    let mut write_err: Option<anyhow::Error> = None;
    let control = jq::eval_generic::eval_each_with_cursor(expr, cursor, &mut |result| {
        // `generic_result_to_jq_values` reports a per-result failure to
        // `sink` and yields nothing for it, the same "report and keep going"
        // contract the batched loop relied on (#355).
        for value in generic_result_to_jq_values(result, cursor, at, sink) {
            match on_value(sink, value) {
                Ok(true) => {}
                Ok(false) => return false,
                Err(e) => {
                    write_err = Some(e);
                    return false;
                }
            }
        }
        true
    });
    // Reported *before* any write error is surfaced, for the reason
    // `evaluate_input_streaming` gives at its own copy of this match: the
    // eager path reported the control during evaluation, always before the
    // caller's write loop could fail, so returning `Err` first here would let
    // an I/O failure swallow the evaluator's own diagnostic.
    match control {
        None => {}
        Some(jq::Control::Error(e)) => sink.report(DiagStyle::Jq, &e, at),
        Some(jq::Control::Break(label)) => sink.report_break(DiagStyle::Jq, &label, at),
        Some(jq::Control::Halt(code)) => sink.request_halt(code),
    }
    if let Some(e) = write_err {
        return Err(e);
    }
    Ok(())
}

/// Convert GenericResult to JqValue, preserving lazy cursor references.
///
/// This is similar to query_result_to_jq_values but works with the
/// cursor-aware GenericResult type from eval_generic.
/// Admit one already-materialized value for output, reporting an over-deep
/// one as an ordinary error rather than letting it panic (#1371).
///
/// A depth *check*, not a conversion (#3009). It used to be
/// `JqValue::try_from_owned`, and the rejection was a by-product of the
/// rebuild; now the value is handed to the writer as it stands and only the
/// depth question remains. The check has to stay at this position rather
/// than move into the writer, because the behaviour it holds up is about
/// output that has *not* been written yet:
/// `test_partial_result_over_depth_value_reports_cleanly_not_panic_1371`
/// requires an over-deep value to print nothing at all, and a writer-side
/// check only fires after 384 levels of `[` are already on stdout -- the
/// #1819 shape `print_json`'s own doc comment warns about.
///
/// Why an over-deep value gets here in the first place: `JqValue::from_owned`
/// asserts past `MAX_VALUE_TREE_DEPTH` (384), which is a reasonable contract
/// for a library caller that owns its input and a very unreasonable one here
/// -- with `def` recursing by evaluation rather than by pre-substituted body,
/// an ordinary recursive filter can build a value deeper than that ceiling,
/// and a filter a user typed must not be able to abort the process (#1098).
/// Returns no values on failure, having reported the error, exactly as every
/// other erroring arm around it does.
fn to_jq_values<'a, W: Clone + AsRef<[u64]>>(
    value: OwnedValue,
    at: &InputLocation,
    sink: &mut ErrorSink,
) -> Vec<OutputItem<'a, W>> {
    match value.check_tree_depth() {
        Ok(()) => vec![OutputItem::Owned(value)],
        Err(e) => {
            sink.report(DiagStyle::Jq, &e, at);
            Vec::new()
        }
    }
}

fn generic_result_to_jq_values<'a, W: Clone + AsRef<[u64]>>(
    result: GenericResult<StandardJson<'a, W>>,
    cursor: JsonCursor<'a, W>,
    at: &InputLocation,
    sink: &mut ErrorSink,
) -> Vec<OutputItem<'a, W>> {
    match result {
        GenericResult::One(v) => match standard_json_to_jq_value(v, &cursor) {
            Ok(jq_value) => vec![OutputItem::Lazy(jq_value)],
            Err(e) => {
                sink.report(DiagStyle::Jq, &e, at);
                vec![]
            }
        },
        // OneCursor: directly use the cursor - most memory efficient for unchanged values
        GenericResult::OneCursor(c) => vec![OutputItem::Lazy(JqValue::Cursor(c))],
        // Stops at the first element that fails to decode, keeping the
        // already-converted prefix -- matching how an ordinary `error`/
        // `break` mid-generator stops the rest of a stream elsewhere in this
        // evaluator (#1164), not a "skip the bad one and keep going"
        // semantic (no precedent for that at this granularity).
        GenericResult::Many(vs) => {
            let mut out = Vec::new();
            for v in vs {
                match standard_json_to_jq_value(v, &cursor) {
                    Ok(jq_value) => out.push(OutputItem::Lazy(jq_value)),
                    Err(e) => {
                        sink.report(DiagStyle::Jq, &e, at);
                        break;
                    }
                }
            }
            out
        }
        // ManyCursor: same lazy-cursor efficiency as OneCursor, per element.
        GenericResult::ManyCursor(cs) => cs
            .into_iter()
            .map(|c| OutputItem::Lazy(JqValue::Cursor(c)))
            .collect(),
        // Stays lazy all the way to output: a bare `keys_unsorted` never
        // materializes a `Vec<String>` — `write_json`/`print_json` stream
        // each key's raw bytes straight from `fields`. `JqValue::LazyKeysArray`
        // is document-order-only (no sort concept in its writer), so a
        // sorted `keys` result must never be routed there (#683) — it
        // materializes and sorts here instead, same as eager `Keys` always
        // did.
        // #1385: a duplicate key would be emitted twice by a writer that
        // streams raw key bytes with no collapse step. #1514: the rule
        // travels with the value instead of being settled here. Probing
        // first cost a whole extra cons-list walk, `key_str()`-decoding
        // every field, ahead of the walk that writes the output -- 96 ns
        // per key on `wide/10mb`, against an 87 ns/key baseline for the
        // entire query. `LazyKeysArray`'s consumers apply the rule as they
        // walk, which is sound because "first occurrence wins" needs no
        // lookahead.
        GenericResult::LazyKeys {
            fields,
            sorted: false,
            collapse,
        } => vec![OutputItem::Lazy(JqValue::LazyKeysArray {
            fields,
            collapse,
        })],
        GenericResult::LazyKeys {
            fields,
            sorted: true,
            collapse,
        } => match effective_keys(&fields, collapse) {
            Ok(mut keys) => {
                keys.sort();
                to_jq_values(
                    OwnedValue::Array(keys.into_iter().map(OwnedValue::String).collect()),
                    at,
                    sink,
                )
            }
            Err(e) => {
                sink.report(DiagStyle::Jq, &e, at);
                vec![]
            }
        },
        // Same laziness as `LazyKeys` above, for array `keys`/
        // `keys_unsorted` (#684): `write_json`/`print_json` write the
        // `[0,1,...,len-1]` digits directly, no `Vec<OwnedValue::Int>`.
        GenericResult::LazyIndexRange(len) => vec![OutputItem::Lazy(JqValue::LazyIndexRange(len))],
        // `JqValue` needs no new variant for a composed `map` chain (#724,
        // #725): `JqValue::Array` already stores per-element cursors (its
        // own "Phase 1 Lazy Optimization"). `drain_atomic`, not
        // `materialize_atomic` (#2066): the latter round-trips every element
        // through an `IndexMap`-backed `OwnedValue`, collapsing a duplicate
        // object key inside a moved element (`sort`/`unique`/`reverse`'s own
        // `LazySource::Cursors` #1687 case) even though `--preserve-input`'s
        // whole point is not doing that. `drain_atomic` keeps each element's
        // own cursor identity (or already-owned value, for a `map`-computed
        // one), matching `ManyCursor`'s own per-element `JqValue::Cursor`
        // mapping just above.
        //
        // `validate_cursor::<JqSemantics, _>(&c)` per `Cursor` element (#3156):
        // `to_owned_cursor`'s own walk, instantiated to build nothing, so it
        // answers exactly as `to_owned_cursor` would without the discarded
        // whole-element copy this arm used to make. Until #3156 it called
        // `to_owned_cursor` itself (#2066 review, #1793 regression): the
        // exact function `lazy_elem_to_owned`'s own `Cursor` arm calls, so it re-validates
        // everything `materialize_atomic`'s per-element walk used to --
        // `MAX_NESTING_DEPTH` (a panic, still caught by the `catch_unwind`
        // wrapper a few hundred lines up) *and* the malformed-member/
        // -delimiter checks `to_owned_cursor_at_depth` also performs (an
        // `EvalError`, not just depth). An earlier revision of this fix
        // wrote its own depth-only walk, satisfying the depth-panic-timing
        // invariant (`catch_unwind`'s own comment: "out's writer is never
        // mid-record when the stack unwinds") but not the delimiter one --
        // review found live: `map(.)` on `[1, {"bad": xyz123}]` printed a
        // garbled `[1,{"bad":` prefix to stdout before erroring, instead of
        // nothing, because delimiter validation had moved to `print_json`'s
        // own `Cursor` arm, which runs *while* writing. Reusing
        // `to_owned_cursor` wholesale closes that gap by construction rather
        // than by re-deriving its checks a second time -- which
        // `validate_cursor` keeps, since it is that same walk.
        GenericResult::LazySeq(seq) => match seq.drain_atomic() {
            // All-or-nothing, matching `materialize_atomic`'s own atomicity
            // contract ("real jq's array construction is all-or-nothing:
            // `[1,2,"x"]|map(.+1)` prints nothing to stdout, only the stderr
            // diagnostic") -- one `Result`, not a partial `JqValue::Array` of
            // whatever converted before the first bad element (#2066 review:
            // an earlier revision's `break`-and-fall-through shape printed an
            // empty `[]` for a single-element failing array instead of
            // suppressing it entirely).
            Ok(elems) => {
                let converted: Result<Vec<JqValue<'_, W>>, EvalError> = elems
                    .into_iter()
                    .map(|elem| match elem {
                        LazyElem::Cursor(c) => {
                            validate_cursor::<JqSemantics, _>(&c).map(|()| JqValue::Cursor(c))
                        }
                        LazyElem::Owned(v) => JqValue::try_from_owned(v),
                    })
                    .collect();
                match converted {
                    Ok(out) => vec![OutputItem::Lazy(JqValue::Array(out))],
                    Err(e) => {
                        sink.report(DiagStyle::Jq, &e, at);
                        vec![]
                    }
                }
            }
            Err(jq::Control::Error(e)) => {
                sink.report(DiagStyle::Jq, &e, at);
                vec![]
            }
            Err(jq::Control::Break(label)) => {
                sink.report_break(DiagStyle::Jq, &label, at);
                vec![]
            }
            Err(jq::Control::Halt(code)) => {
                sink.request_halt(code);
                vec![]
            }
        },
        GenericResult::None => vec![],
        GenericResult::Error(e) => {
            sink.report(DiagStyle::Jq, &e, at);
            vec![]
        }
        GenericResult::Owned(v) => to_jq_values(v, at, sink),
        GenericResult::ManyOwned(vs) => vs
            .into_iter()
            .flat_map(|v| to_jq_values(v, at, sink))
            .collect(),
        GenericResult::Break(label) => {
            sink.report_break(DiagStyle::Jq, &label, at);
            vec![]
        }
        // `halt`/`halt_error` (#791): not a diagnostic, so no `sink.report*`
        // call — `request_halt` records the exit code for the loop above to
        // short-circuit on, without touching `hit`/`report_count`.
        GenericResult::Halt(code) => {
            sink.request_halt(code);
            vec![]
        }
        // The outputs already produced no longer vanish behind the failure
        // (#400, #494). Each one still goes through the checked
        // `to_jq_values`, not `JqValue::from_owned` directly -- these can be
        // built by an ordinary recursive `def` now (#1371), so an
        // over-deep one must report cleanly rather than panic, same as
        // every other arm in this function (see `to_jq_values`'s own doc).
        GenericResult::Partial(vs, jq::Control::Error(e)) => {
            sink.report(DiagStyle::Jq, &e, at);
            vs.into_iter()
                .flat_map(|v| to_jq_values(v, at, sink))
                .collect()
        }
        GenericResult::Partial(vs, jq::Control::Break(label)) => {
            sink.report_break(DiagStyle::Jq, &label, at);
            vs.into_iter()
                .flat_map(|v| to_jq_values(v, at, sink))
                .collect()
        }
        GenericResult::Partial(vs, jq::Control::Halt(code)) => {
            sink.request_halt(code);
            vs.into_iter()
                .flat_map(|v| to_jq_values(v, at, sink))
                .collect()
        }
    }
}

/// Convert StandardJson to JqValue, preserving lazy cursor references.
///
/// **Phase 1 Lazy Optimization**: Arrays and objects store `JqValue::Cursor` for
/// each child instead of recursively materializing. This defers allocation until
/// the value is actually needed (e.g., for computation or output formatting).
///
/// Errors (#1192) rather than silently degrading when a top-level result
/// string (or an immediate key of a top-level result object) passes
/// structural validation but fails to *decode* -- this used to substitute an
/// empty string for such a value, and an empty-string *key* (colliding
/// multiple decode-failing keys together) for such a key, instead of
/// surfacing a real error. Only the immediate level is checked here because
/// array/object children stay lazy (`JqValue::Cursor`) rather than
/// recursively converting -- a decode failure nested deeper is caught later,
/// if and when that child cursor is itself materialized.
///
/// Also errors (#1194) on a *structurally* malformed member -- a key that is
/// not a string, or a child with no sibling to pair as its value. That is a
/// different failure from a decode failure: the text was never `key: value`,
/// and the semi-index accepted it only because bracket matching did.
/// `parent_cursor` supplies the document text the strict validator re-reads
/// to name the error; it was unused before that.
fn standard_json_to_jq_value<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    parent_cursor: &JsonCursor<'a, W>,
) -> Result<JqValue<'a, W>, EvalError> {
    // STYLE-0013-TAIL: `child_tail_gap_ok` is callable (both arms hold a
    // child cursor) and would close the trailing-comma half. Only the
    // zero-child half is genuinely blocked: that needs a *container* cursor,
    // and this is the true top level -- the one entry point with none to
    // give, the same documented gap `owned_from_standard_json_at_depth`'s
    // own comment describes. Wiring in the callable half is a behaviour
    // change, still untaken. #2594 did not reach this site: it closed the
    // zero-child gap in the evaluator arms that *do* hold a container
    // cursor, and by the time a value arrives here the walk that produced
    // it has already been checked there.
    // STYLE-0013: `preceding_gap_ok` directly, not `key_delimiter_ok`/
    // `value_delimiter_ok` -- this is the CLI-crate lazy materializer, not
    // `DocumentFields`-generic, and its array/object arms below already
    // hold the child/key/value cursors' own resolved `text_position()`
    // rather than an unresolved `DocumentValue`, so routing through the
    // library-side wrappers would re-derive a position this walk already
    // has for free.
    Ok(match value {
        StandardJson::Null => JqValue::Null,
        StandardJson::Bool(b) => JqValue::Bool(b),
        // A reindex-bridge token (#3034) is a computed value, not a
        // spelling: decode it here, so a `RawNumber` only ever holds
        // document text and its readers (`OwnedValue::from_number_bytes`,
        // `format_raw_number`) never see a token.
        StandardJson::Number(n) => match n.bridge_value() {
            Some(f) => JqValue::Float(f),
            // Use RawNumber to preserve original formatting like "4e4"
            None => JqValue::RawNumber(n.raw_bytes()),
        },
        StandardJson::String(s) => {
            // Keep string lazy - use raw bytes reference instead of decoding
            JqValue::String(
                s.as_str()
                    .map_err(|e| EvalError::decode_failure(format!("{e}")))?
                    .to_string(),
            )
        }
        StandardJson::Array(elements) => {
            // LAZY: Store cursor references instead of materializing children
            //
            // #2211 code review: this walk used to validate *nothing* about
            // the delimiter preceding each element -- not even the older
            // #1677 missing/doubled-comma-between-two-real-elements check
            // `print_json`'s own array arm (`check_preceding_delimiter`) and
            // `eval_generic::to_owned_cursor_at_depth`'s array loop already
            // perform. Only the top level (`GenericResult::One`/`Many`)
            // reaches this function at all, and its own container position
            // is not available here by construction (`GenericResult::One`
            // is only ever produced when no cursor is being tracked for
            // *this* node -- see `Expr::Identity`'s own doc comment in
            // `eval_generic.rs`), so unlike its `to_owned_cursor_at_depth`/
            // `cursor_to_owned_at_depth` siblings this cannot also check an
            // apparently-*empty* container's own opening-to-closing gap
            // (`[,]`) -- there is no cursor for `[,]` itself to check that
            // against, only for its (zero) children. Each child's *own*
            // cursor is still valid and positioned regardless, so the
            // between-real-elements check below is fully safe.
            let mut items: Vec<JqValue<'a, W>> = Vec::new();
            for (i, child_cursor) in elements.cursor_iter().enumerate() {
                if let Some(pos) = child_cursor.text_position() {
                    let expected = if i == 0 { None } else { Some(b',') };
                    if !preceding_gap_ok(child_cursor.text(), pos, expected) {
                        return Err(EvalError::malformed_json_text(child_cursor.text()));
                    }
                }
                items.push(JqValue::Cursor(child_cursor));
            }
            JqValue::Array(items)
        }
        StandardJson::Object(fields) => {
            // LAZY: Store cursor references instead of materializing values
            let mut map: IndexMap<String, JqValue<'a, W>> = IndexMap::new();
            let mut remaining = fields;
            let mut is_first = true;
            while let Some((f, rest)) = remaining.uncons() {
                // A key that isn't `StandardJson::String` at all is a
                // structurally malformed key, not a decode failure. This used
                // to `continue`, dropping the field and everything that
                // depended on it while `length` went on counting it (#1194).
                let key = match f.key().decoded_key_str_checked()? {
                    Some(key) => key.into_owned(),
                    None => return Err(EvalError::malformed_json_text(parent_cursor.text())),
                };
                // #2211 code review: same missing #1677 check as the array
                // arm above -- neither the key's own preceding `,` nor the
                // value's own preceding `:` was ever validated here. Same
                // "no container-level cursor available" limitation as the
                // array arm for the apparently-*empty* case (`{,}`); each
                // real field's own key/value cursor is unaffected by that.
                let key_cursor = f.key_cursor();
                let value_cursor = f.value_cursor();
                if let Some(key_pos) = key_cursor.text_position() {
                    let expected = if is_first { None } else { Some(b',') };
                    if !preceding_gap_ok(key_cursor.text(), key_pos, expected) {
                        return Err(EvalError::malformed_json_text(key_cursor.text()));
                    }
                }
                if let Some(value_pos) = value_cursor.text_position() {
                    if !preceding_gap_ok(value_cursor.text(), value_pos, Some(b':')) {
                        return Err(EvalError::malformed_json_text(value_cursor.text()));
                    }
                }
                // Use cursor for value instead of materializing
                map.insert(key, JqValue::Cursor(value_cursor));
                remaining = rest;
                is_first = false;
            }
            // A child with no sibling to pair as a value: the object is
            // malformed and `uncons` would drop it silently (#1194).
            if remaining.ends_unpaired() {
                return Err(EvalError::malformed_json_text(parent_cursor.text()));
            }
            JqValue::Object(map.into())
        }
        // See `eval_generic::to_owned_at_depth`'s own `is_error` arm
        // (#1194/#1247): a structurally malformed value -- one the
        // semi-index accepted as a span but could not classify as any JSON
        // token -- raises rather than becoming `null`. #2286: decode_failure,
        // not new -- same class as the malformed member/delimiter errors,
        // confirmed live that real jq treats this uncatchably too.
        StandardJson::Error(msg) => return Err(EvalError::decode_failure(msg)),
    })
}

/// `--raw-output0` (#1830) uses NUL as its own record terminator, so a NUL
/// byte embedded in a raw-output string's own content is genuinely
/// ambiguous to any NUL-delimited consumer downstream (`xargs -0`,
/// `read -d ''`) -- it would look identical to a record boundary. Real jq
/// refuses rather than emit it: confirmed live against jq 1.7.1,
/// `jq -r --raw-output0 '.'` on a string with an embedded NUL raises `jq:
/// error (at <stdin>:0): Cannot dump a string containing NUL with
/// --raw-output0 option`, exit 5 -- verified this fires only under
/// `--raw-output0` specifically, not `-r`/`-j` alone (those use newline/no
/// separator, where an embedded NUL creates no comparable ambiguity), and
/// only when the *rendered* bytes would actually contain a raw NUL (JSON's
/// own quoted output always escapes it to \u0000, six ASCII bytes, never
/// a raw byte -- this check is unreachable from that path and is not
/// wired there).
///
/// Checked here, immediately before the raw bytes reach the writer -- not
/// via a buffer-then-scan pass over previously-written output -- so an
/// already-good prior record (written by an earlier call to this same
/// function inside the caller's own per-record loop) is never buffered or
/// retroactively undone by a later record's violation. This mirrors real
/// jq's own confirmed flush-then-error ordering: `.[]` over
/// `[1, "bad value", 2]` under `--raw-output0` writes `1` (with its
/// own NUL terminator) to stdout, then errors on the second value without
/// writing any of its content, never reaching the third. This issue's
/// (#1830) yq-mode sibling, #1709, found the opposite design ("materialize
/// the whole rendered
/// result, scan it, then write") on the yq side caused three separate
/// regressions -- silently discarding already-good earlier results,
/// leaving a dangling partial write on an error mid-multi-document-stream,
/// and forcing full in-memory materialization of output this crate's own
/// M2/P9 streaming architecture exists specifically to avoid (+65% peak
/// RSS, measured). This function avoids all three by never buffering
/// anything beyond the one string already in hand.
fn reject_raw_output0_nul(s: &str, config: &OutputConfig) -> Result<()> {
    if config.raw_output0 && s.as_bytes().contains(&0) {
        return Err(MalformedJsonError::new(EvalError::new(
            "Cannot dump a string containing NUL with --raw-output0 option",
        ))
        .into());
    }
    Ok(())
}

/// Write a single output JqValue (preserves number formatting when possible).
fn write_output_jq_value<Out: Write, Wrd: Clone + AsRef<[u64]>>(
    out: &mut Out,
    value: &JqValue<'_, Wrd>,
    config: &OutputConfig,
) -> Result<()> {
    // Raw-output string, if any -- resolved and NUL-checked before
    // writing *any* byte of this record, including the `--seq` RS
    // separator below (code review, #1830: checking only after that
    // write left a dangling, unterminated RS byte on stdout for a
    // rejected record under `--seq --raw-output0`). A single RS write
    // below then covers both the raw and non-raw cases, rather than one
    // copy per branch.
    //
    // #2662: `--ascii-output` wins over `-r`/`-j` for a *string* value --
    // confirmed live against jq 1.7.1: `-acr '"café"'` prints `"café"`,
    // quoted and escaped, not the unquoted raw string `-r` alone would give.
    // `-a` on a non-string value (`-ar '42'` -> `42`) is unaffected, since
    // `as_str()` is `None` there regardless. Skipping the raw branch here
    // (rather than escaping its content in place) reproduces that exactly:
    // falls through to the quoted/escaped write below, the same as if `-r`
    // had never been passed.
    //
    // `as_str` is resolved once, gated only on `raw_output` (not also
    // `ascii_output`), and reused for both the `--seq` decision below and
    // the raw-content decision just after -- computing it twice would
    // decode the same string twice for no reason. The two *uses* of it
    // still differ deliberately: `--seq`'s RS-suppression must key on
    // whether this value *would have been* raw-shaped by `-r` alone (a
    // string, full stop), not on whether `-a` went on to override that
    // shaping -- confirmed live against jq 1.7.1, which suppresses the RS
    // byte for `--seq -acr` on a string exactly as it does for plain
    // `--seq -r`, even though the printed bytes are quoted+escaped either
    // way. Keying it off the post-override `raw_str` instead (this
    // function's first cut at the `-a`-wins fix) emitted a spurious RS byte
    // under `--seq -a -r`, contradicting the reference.
    let as_str = if config.raw_output {
        value.as_str()
    } else {
        None
    };
    if write_output_raw_prologue(out, as_str, config)? {
        return Ok(());
    }

    // `JqCompat` uses the jq-compatible formatter (reformats numbers); a
    // source-preserving convention uses the preserve formatter (keeps the
    // original number format).
    //
    // #2874: stays a static `if` over two monomorphised `print_json`
    // instantiations rather than a `&dyn LiteralFormatter`. This is the
    // default `-c` route -- the hottest path in the binary -- and a vtable
    // call in its per-scalar inner loop is not something to buy for tidiness
    // (#2603/#595: `[profile.release]` is pinned to cgu=1 + fat LTO because
    // this path is sensitive to codegen alone).
    //
    // #2662: `ascii_output` joins `sort_keys`/`color_output` on the
    // materialize side here -- `print_json` streams structural + scalar
    // bytes directly with no escaping hook of its own, where `format_json`
    // (used by the `else` arm below) already threads `ascii: config.
    // ascii_output` through `JsonFormatOpts`. Materializing *this one
    // value* to reuse that existing, already-correct escaping is far
    // cheaper than teaching `print_json`'s recursive walk a second escaping
    // mode, and still validates only the value being printed, not the rest
    // of the document -- the same "materializes what it reads" rule this
    // issue is generalizing to `-S`/`-C` above.
    if !config.sort_keys && !config.color_output && !config.ascii_output {
        if config.convention.preserves_source_values() {
            print_json(
                out,
                value,
                &PreserveFormatter,
                config,
                0,
                &mut Vec::new(),
                &mut Vec::new(),
                None,
            )?;
        } else {
            print_json(
                out,
                value,
                &JqCompatFormatter,
                config,
                0,
                &mut Vec::new(),
                &mut Vec::new(),
                None,
            )?;
        }
    } else {
        // For complex output (pretty-print, sort_keys, colors, ascii),
        // materialize first -- and surface a decode failure rather than
        // printing the empty string it used to become (#1247).
        //
        // #2662: wrapped in `MalformedJsonError`, not a bare `anyhow::anyhow!`
        // -- this function's only caller (`route_write_error`) downcasts for
        // that exact type to route a malformed-document failure into jq's
        // own diagnostic channel (exit 5) rather than `anyhow`'s (exit 1).
        // Before this issue, `sort_keys`/`color_output` were the only two
        // flags that could reach this branch at all, and both were already
        // excluded from the lazy path entirely (`can_use_lazy_path`), so
        // this branch -- and the bare-anyhow bug in it -- was unreachable
        // in practice. Moving `-S`/`-C`/`-a` onto the lazy path here is what
        // first exercises it: `printf '{invalid}' | succinctly jq -cS .`
        // used to exit 1 with a bare `Error: ...` where real jq (and this
        // crate's own default lazy route) exits 5 with `jq: error (at ...)`.
        //
        // `try_materialize`, not `materialize` (#2850): this is a
        // CLI-output-boundary call site, not the evaluator's own hot
        // recursion, so a nesting-depth violation belongs in this same
        // `Result`/`MalformedJsonError` channel rather than as an
        // uncaught-by-design panic. #2662's move of `-S`/`-C`/`-a` onto this
        // branch is what makes this the confirmed, live path #2850 is filed
        // over: `succinctly jq -cS .` on deeply-nested input used to leak
        // Rust's raw panic backtrace to stderr the same way bare `-e` did.
        let owned = value
            .try_materialize()
            .map_err(|e| anyhow::Error::from(MalformedJsonError::new(e)))?;
        out.write_all(format_json(&owned, config).as_bytes())?;
    }

    write_terminator(out, config)?;
    Ok(())
}

/// Write a single output value.
/// ASCII RS (Record Separator) character for JSON sequence format (RFC 7464)
const ASCII_RS: u8 = 0x1E;

/// Whether `--seq` should prepend the RS separator to this record (#1913).
///
/// Real jq writes it before each *JSON* output, but not before a
/// genuinely raw string produced by `-r`/`-j` -- confirmed live against jq
/// 1.7.1 (`--seq -r` produces no leading `\x1e` at all). `is_raw_output`
/// is the caller's own `raw_str.is_some()`, which already distinguishes
/// "not in raw-output mode" and "`-r`'s value isn't a string" from a
/// genuine raw write -- reusing it here (rather than checking `config.seq`
/// alone) gets a real-jq subtlety right for free: when `-r` falls back to
/// JSON output for a non-string value, `raw_str` is `None` there too, so
/// the separator is still written, matching jq.
///
/// Shared by both `write_output_jq_value` and `write_output` so the
/// condition can't drift between them -- see `write_output_jq_value`'s own
/// call site for why one of the two is currently unreachable under
/// `--seq` regardless.
///
/// Caveat this function doesn't attempt to fix: `raw_str`'s `None` can
/// also mean "the string failed to decode" (e.g. an unpaired UTF-16
/// surrogate `JqValue::as_str()` gives up on) rather than "genuinely not a
/// string" -- a separate, pre-existing gap in how undecodable strings are
/// classified, not a `--seq`-specific one. In practice this particular
/// codebase's parser already accepts input real jq rejects as a parse
/// error before either write path is ever reached, so no live divergence
/// was found for this combination; not investigated further here.
fn should_write_seq_separator(config: &OutputConfig, is_raw_output: bool) -> bool {
    config.seq && !is_raw_output
}

/// The record prologue every jq-mode writer shares: NUL rejection, the
/// `--seq` record separator, and the `-r`/`-j`/`--raw-output0` raw-string
/// write that short-circuits the JSON body entirely.
///
/// Returns `true` when the record was fully written as a raw string (the
/// caller must not write a body or a terminator), `false` when the caller
/// still owes a JSON body.
///
/// Extracted (#3009) for the reason `should_write_seq_separator` was already
/// extracted out of the same two prologues -- "so the condition can't drift
/// between them". Adding an owned-value writer would otherwise have made a
/// *third* copy of an ordering that took two issues to get right, both
/// confirmed live against jq 1.7.1 and neither obvious from reading:
///
/// - #1830: the NUL check runs before *any* byte of the record, including
///   the `--seq` RS. Checking after it left a dangling, unterminated RS on
///   stdout for a record `--raw-output0` then rejected.
/// - #2662: `-a` beats `-r` for a string (`-acr '"café"'` prints `"café"`,
///   quoted and escaped), so `raw_str` is `as_str` filtered by
///   `ascii_output` -- but `--seq`'s RS suppression keys on the *pre*-override
///   shape, because jq suppresses the RS for `--seq -acr` on a string exactly
///   as for plain `--seq -r`. Keying it off the post-override value emits a
///   spurious RS.
///
/// The `as_str` resolution itself stays at each call site: it is
/// `JqValue::as_str()` on one and a `String` match on the others, and the
/// lazy one wants it computed once for both uses here.
fn write_output_raw_prologue<Out: Write>(
    out: &mut Out,
    as_str: Option<Cow<'_, str>>,
    config: &OutputConfig,
) -> Result<bool> {
    let was_raw_shaped = as_str.is_some();
    let raw_str = if config.ascii_output { None } else { as_str };
    if let Some(s) = &raw_str {
        reject_raw_output0_nul(s, config)?;
    }
    if should_write_seq_separator(config, was_raw_shaped) {
        out.write_all(&[ASCII_RS])?;
    }

    if let Some(s) = raw_str {
        out.write_all(s.as_bytes())?;
        write_terminator(out, config)?;
        return Ok(true);
    }
    Ok(false)
}

/// The `-r`/`-j`/`--raw-output0` string of an owned value, or `None` when
/// this record is not raw-shaped. `OwnedValue`'s counterpart to
/// `JqValue::as_str()`, which the lazy writer uses for the same purpose --
/// but trivially total, since an owned string is already decoded and so
/// cannot fail the way a cursor-backed one can.
fn owned_raw_str<'v>(value: &'v OwnedValue, config: &OutputConfig) -> Option<Cow<'v, str>> {
    if !config.raw_output {
        return None;
    }
    match value {
        OwnedValue::String(s) => Some(Cow::Borrowed(s.as_str())),
        _ => None,
    }
}

/// [`write_output_jq_value`]'s twin for an already-owned result (#3009).
///
/// Same record shape as its sibling -- shared prologue, then a JSON body,
/// then the terminator -- and the same fast/slow split on the flags that
/// `print_json` has no hook for. What it does *not* do is rebuild the value
/// as a `JqValue` first:
///
/// - the fast branch streams through [`print_owned_json`], whose whole
///   contract is byte-equality with `print_json` over a rebuilt value;
/// - the slow branch (`-S`/`-C`/`-a`) hands `format_json` the `OwnedValue`
///   it already has. That is where the old route was worst: it converted an
///   owned tree *into* a `JqValue` only for `try_materialize` to convert it
///   straight back, three traversals to print one value.
///
/// Also serves the eager `-n`/`--slurp`/DSV-input route (formerly a separate
/// `write_output`, which formatted through `format_json` unconditionally for
/// every config, including plain `-c`): `write_output_owned_value_matches_old_write_output_3155`
/// proves the fast branch is byte-identical to that route's old
/// always-`format_json` output before this function took over its call
/// sites, the same discipline as the differential test against
/// `write_output_jq_value` below.
fn write_output_owned_value<Out: Write>(
    out: &mut Out,
    value: &OwnedValue,
    config: &OutputConfig,
) -> Result<()> {
    if write_output_raw_prologue(out, owned_raw_str(value, config), config)? {
        return Ok(());
    }

    // Same static `if` over two monomorphisations as the lazy sibling, and
    // for the same reason (#2874/#2603): this is the default `-c` route and
    // a vtable call in its per-scalar inner loop is not worth buying.
    if !config.sort_keys && !config.color_output && !config.ascii_output {
        if config.convention.preserves_source_values() {
            print_owned_json(out, value, &PreserveFormatter, config, 0)?;
        } else {
            print_owned_json(out, value, &JqCompatFormatter, config, 0)?;
        }
    } else {
        out.write_all(format_json(value, config).as_bytes())?;
    }

    write_terminator(out, config)?;
    Ok(())
}

/// Write the appropriate line terminator based on config. The NUL/newline/
/// join three-way choice itself is shared with yq_runner.rs's own
/// `terminator_from_config` via `output::Terminator` (#1711) -- jq mode's
/// own `--unbuffered` flush has no yq-mode equivalent at this call site,
/// so it stays local rather than folding into the shared type.
fn write_terminator<W: Write>(out: &mut W, config: &OutputConfig) -> Result<()> {
    Terminator::from_flags(config.raw_output0, config.join_output).write_io(out)?;
    if config.unbuffered {
        out.flush()?;
    }
    Ok(())
}

// =============================================================================
// LiteralFormatter trait and implementations
// =============================================================================

use std::borrow::Cow;

/// Trait for formatting JSON literals (scalars).
///
/// This separates the concern of how to format individual values from the
/// structural concerns of printing arrays, objects, and handling indentation.
trait LiteralFormatter {
    /// Format a raw number from source JSON bytes.
    fn format_raw_number<'a>(&self, raw: &'a [u8]) -> Cow<'a, str>;

    /// Format a computed floating-point number.
    fn format_float(&self, f: f64) -> String;

    /// Format a computed integer.
    fn format_int(&self, i: i64) -> String;
}

/// jq-compatible formatter: reformats numbers according to jq's rules.
///
/// - Scientific notation normalized: `4e4` → `4E+4`
/// - Small negative exponents expanded: `1e-3` → `0.001`
/// - Uppercase E with explicit + sign
struct JqCompatFormatter;

impl LiteralFormatter for JqCompatFormatter {
    fn format_raw_number<'a>(&self, raw: &'a [u8]) -> Cow<'a, str> {
        // The semi-index scanner accepts number *spans* more leniently than
        // RFC 8259 (leading zeros, multiple decimal points -- #966).
        // Sanitize via the same fallback every other "raw bytes -> number"
        // conversion in this crate uses, instead of echoing invalid text
        // verbatim and producing invalid JSON output.
        if !validate::is_valid_number(raw) {
            return Cow::Owned(match OwnedValue::from_number_bytes::<JqSemantics>(raw) {
                OwnedValue::Int(i) => self.format_int(i),
                OwnedValue::Float(f) => self.format_float(f),
                // A leading-dot span (`.5`, `-.5`) is jq-lenient-but-not-
                // RFC-8259 (#1171): `from_number_bytes` preserves its
                // spelling as a `NumberLiteral` here instead of degrading
                // to a plain `Float` (needed so trailing zeros survive,
                // `.500` -> `0.500` not `0.5`), so route it through the
                // same jq-compat reformatting a strictly-valid span gets
                // below via the literal's own text (identical to `raw`
                // for this shape) rather than falling into the `_` catch
                // -all and printing `null`.
                OwnedValue::NumberLiteral(_, literal) => {
                    format_number_jq_compat(literal.as_bytes())
                }
                _ => "null".to_string(),
            });
        }
        // A source literal that overflows to +/-Infinity (e.g. `1e400`) goes
        // through `format_number_jq_compat` like any other literal (#1087):
        // it already special-cases a non-finite input via
        // `format_overflow_literal_mantissa`, giving jq's own mantissa-
        // preserving renormalized text (`1e400` -> `1E+400`) -- confirmed
        // live against jq 1.7.1, where `1e400 | .` (identity, no
        // computation) echoes `1E+400`, not `null` or `DBL_MAX` text; only
        // an actual *computed* Infinity (`format_float` below) gets the
        // `DBL_MAX` substitution. RFC 8259's grammar has no NaN spelling,
        // so no *valid* span reaching this line is a NaN; jq's own `nan`/
        // `sNaN12` words (#2877) are not `is_valid_number` and take the
        // sanitizing arm above, where `from_number_bytes` reads them as a
        // bare `Float` and `format_float` prints jq's `null`.
        Cow::Owned(format_number_jq_compat(raw))
    }

    fn format_float(&self, f: f64) -> String {
        // A computed Infinity (`infinite`, an arithmetic overflow) has no
        // source literal to echo, so it renders jq's own `DBL_MAX` text
        // instead of `null` (#1087, confirmed live against jq 1.7.1: `null |
        // infinite` is `1.7976931348623157e+308`, not `null`). NaN has no
        // such fallback text in real jq either and stays `null`. Reuses
        // #1075's `nonfinite_display_string` (the same NaN-vs-Infinity split
        // already pinned for jq's *text*-format path) rather than a fourth
        // hand-rolled copy of the same two branches. #2456: the finite arm
        // used to be a bare `format!("{f}")`, which never switches to
        // scientific notation past jq's own threshold -- `jq_bare_float_display`
        // is the same formatter `OwnedValue::to_json` uses.
        if f.is_finite() {
            jq_bare_float_display(f)
        } else {
            nonfinite_display_string::<JqSemantics>(f).to_string()
        }
    }

    fn format_int(&self, i: i64) -> String {
        format!("{i}")
    }
}

/// Preservation formatter: outputs raw bytes unchanged.
///
/// Useful for maintaining original number formatting from the source JSON.
struct PreserveFormatter;

impl LiteralFormatter for PreserveFormatter {
    fn format_raw_number<'a>(&self, raw: &'a [u8]) -> Cow<'a, str> {
        // #2874: this used to be a hand-rolled `from_utf8`/lossy match,
        // which is what `String::from_utf8_lossy` already is -- same
        // `Cow::Borrowed`-when-valid behaviour, so no allocation is added on
        // this (hot, default `-c`) path. That one call is now the single
        // spelling of "echo the source number span verbatim" shared with
        // `jq::stream::real_output_finite_literal`, which is the same call
        // plus `.into_owned()` for its `fn(&[u8]) -> String` signature.
        //
        // Two siblings deliberately stay distinct rather than being churned
        // onto it: `output::format_json_impl`'s `JqPreserveInput` arm
        // already holds a `Box<str>`, so its echo is the infallible
        // `literal.to_string()`; and `json::light::write_json_number`'s
        // source-preserving arm writes into a `core::fmt::Write` and so
        // *errors* on invalid UTF-8 where these allocate a lossy
        // replacement -- it cannot build a `Cow` without changing that
        // signature.
        String::from_utf8_lossy(raw)
    }

    fn format_float(&self, f: f64) -> String {
        // Same split as `JqCompatFormatter::format_float` above (#1087, and
        // #2456's finite-arm fix): a computed Infinity has no source literal
        // for preserve mode to keep either, so both formatters need the
        // identical jq-real-output rule here, not just the jq_compat one.
        if f.is_finite() {
            jq_bare_float_display(f)
        } else {
            nonfinite_display_string::<JqSemantics>(f).to_string()
        }
    }

    fn format_int(&self, i: i64) -> String {
        format!("{i}")
    }
}

// =============================================================================
// Generic JSON Printer
// =============================================================================

// `preceding_gap_ok` (#1643) used to live here as a CLI-only check. #1677
// relocated it to `succinctly::json::light` so this printer and the
// evaluator's own object/array walk (`eval_generic.rs`) share one
// definition instead of drifting into two; see that function's own doc
// comment for the semi-index background (`json::standard::is_delim`
// treating `,`/`:` as invisible) and the backward-scan rationale.

// `following_gap_ok`/`trailing_gap_ok`/`scalar_end_pos` (#1676) used to live
// here as CLI-only checks, structurally identical to (but not calling)
// `succinctly::json::light`'s own private copies of the same predicates.
// #2263 migrated every call site here onto the `DocumentCursor`/
// `DocumentValue` trait methods #2243 added
// (`trailing_element_gap_ok`/`scalar_text_end`), which delegate to
// `json::light`'s copies -- so this printer and the evaluator's own
// object/array walk (`eval_generic.rs`) share one definition instead of
// three drifting copies, the same consolidation #1677 already did for
// `preceding_gap_ok`.

/// Array-element wrapper around [`preceding_gap_ok`]: element `index` (0-
/// based) must be preceded by `,`, except the first, which must be
/// preceded by nothing (#1643).
///
/// Called from the array arm's own validation pass, before any of `[`'s
/// contents are written -- same discipline as the object arm just below,
/// and for the same reason (its own comment there explains it): a
/// malformed array must not leave a partial `[` on `out` when the error
/// surfaces.
///
/// Returns the element's own `text_position()` on success, so the caller
/// can cache it and hand it to `print_json` as `known_text_pos` once the
/// write loop reaches that same element, instead of that recursive call
/// re-deriving the same rank/select lookup a second time -- a version
/// that didn't cache this measured 19-30% slower on `sjq -c .` (#1643,
/// see the PR): a document with many short elements pays for this lookup
/// once per element regardless, so paying for it *twice* per element,
/// once here and again moments later, was the entire regression, not the
/// gap check itself.
fn check_preceding_delimiter<W: AsRef<[u64]>>(
    child_cursor: &JsonCursor<'_, W>,
    index: usize,
) -> Result<Option<usize>> {
    // STYLE-0013: `preceding_gap_ok` directly -- this is `print_json`'s
    // own array-arm helper, returning the resolved position for its
    // caller to cache and reuse (see this function's own doc comment on
    // why: a version that re-derived the position instead measured 19-30%
    // slower, #1643). `key_delimiter_ok`/`value_delimiter_ok` don't return
    // a position, so adopting either would give that back.
    let Some(start) = child_cursor.text_position() else {
        return Ok(None);
    };
    let expected = if index == 0 { None } else { Some(b',') };
    if !preceding_gap_ok(child_cursor.text(), start, expected) {
        return Err(
            MalformedJsonError::new(EvalError::malformed_json_text(child_cursor.text())).into(),
        );
    }
    Ok(Some(start))
}

/// Recursively validate every object/array in `cursor`'s subtree for a
/// missing or doubled `,`/`:` (#1643), via [`preceding_gap_ok`] -- the same
/// check `print_json`'s object/array arms perform, needed again here for a
/// caller that doesn't go through `print_json` at all.
///
/// The evaluator's own materializer performs no validation of its own; it
/// expects the caller to have done so first -- [`parse_json_stream`]'s
/// fallback below (which backs `-S`, `-C`, and `--slurp` -- none of which
/// route through `print_json`) used to call it directly on unvalidated
/// bytes, so `{"a" 1}` would silently materialize as `{"a":1}` under those
/// flags even though the default `sjq -c .` path already rejects it since
/// #1643.
///
/// `depth` guards against a stack overflow on adversarially deep input the
/// same way [`generic_to_owned`]'s own recursion does, but as a catchable
/// error (`check_nesting_depth`, #1818) rather than a panic
/// (`assert_nesting_depth`, #998): this walk runs *before* any user filter
/// evaluation even begins (`parse_json_stream`'s fallback, backing `-S`,
/// `-C`, `--slurp`, `--ascii-output`, `--slurpfile`, and any filter using
/// `input`/`inputs`), so there's no `try`/`catch`-reachability concern the
/// way there is for `to_owned`'s own hot-path guard -- and a clean,
/// reported error beats a bare panic exiting 101 with an un-jq-shaped
/// message, matching `print_json`'s own `anyhow::ensure!`-based guard for
/// the analogous case on the lazy identity-path (confirmed live: an
/// adversarially deep document between 256 and 384 levels used to panic
/// here uncaught -- `succinctly jq -c --sort-keys '.' deep.json` exited 101
/// with `thread 'main' panicked at ...: nesting depth exceeds limit of
/// 256` instead of a clean jq-channel error).
///
/// Not a hot path: `parse_json_stream_strict`'s `serde_json` validation
/// already runs first and only fails (routing here) for a real jq leniency
/// like a leading-zero number (#1094) or a genuinely malformed document, so
/// this walk is cold by construction and doesn't need `print_json`'s
/// `known_text_pos` reuse trick.
fn validate_json_delimiters<W: Clone + AsRef<[u64]>>(
    cursor: &JsonCursor<'_, W>,
    depth: usize,
) -> core::result::Result<(), EvalError> {
    // STYLE-0013-TAIL: this walk does check both halves -- `gap_start` is
    // `open_pos + 1` when no child was seen (`container_gap_ok`'s own body)
    // and the last child's gap end otherwise -- but through
    // `DocumentCursor::trailing_element_gap_ok(gap_start, ..)` with a
    // pre-resolved position, which no `Option<last>`-dispatching helper in
    // `TAIL_ROUTES` can express. Not a #2594 site: the empty case is
    // covered here.
    // STYLE-0013: `preceding_gap_ok` directly, in both the array and
    // object arms below -- this is the CLI's own cold-path validator
    // (this function's own doc comment above explains why it exists
    // instead of routing through `print_json`), reusing each child's
    // already-resolved `text_position()` via `value_at` (see `scalar_end_pos`'s
    // own doc comment) the same way `check_preceding_delimiter` does.
    check_nesting_depth(depth)?;
    match cursor.value() {
        StandardJson::Array(elements) => {
            let mut saw_any = false;
            let mut last_gap_end = None;
            for (i, child) in elements.cursor_iter().enumerate() {
                saw_any = true;
                let start = child.text_position();
                if let Some(start) = start {
                    let expected = if i == 0 { None } else { Some(b',') };
                    if !preceding_gap_ok(child.text(), start, expected) {
                        return Err(EvalError::malformed_json_text(child.text()));
                    }
                }
                validate_json_delimiters(&child, depth + 1)?;
                // #1676: reuses `start` (already resolved above) via
                // `value_at` rather than `child.value()`, which would
                // re-derive it -- see `scalar_text_end`'s own doc comment.
                last_gap_end = start.and_then(|s| child.value_at(s).scalar_text_end(s));
            }
            // #1676: trailing `,` (`[1,2,]`) or a stray `,` in an
            // apparently-empty array (`[,]`) -- `last_gap_end` is `None`
            // both when there's nothing to check yet and when the last
            // element is itself a container (`scalar_text_end`'s own doc
            // comment explains why that case is skipped, not just unknown).
            if let Some(open_pos) = cursor.text_position() {
                let gap_start = if saw_any {
                    last_gap_end
                } else {
                    Some(open_pos + 1)
                };
                if let Some(gap_start) = gap_start {
                    if !cursor.trailing_element_gap_ok(gap_start, b']') {
                        return Err(EvalError::malformed_json_text(cursor.text()));
                    }
                }
            }
        }
        StandardJson::Object(fields) => {
            let mut remaining = fields;
            let mut field_index = 0usize;
            let mut last_gap_end = None;
            while let Some((field, rest)) = remaining.uncons() {
                let StandardJson::String(k) = field.key() else {
                    return Err(EvalError::malformed_json_text(cursor.text()));
                };
                let value_cursor = field.value_cursor();
                let value_start = value_cursor.text_position();
                if let Some(value_start) = value_start {
                    let comma_expected = if field_index == 0 { None } else { Some(b',') };
                    if !preceding_gap_ok(cursor.text(), k.start(), comma_expected)
                        || !preceding_gap_ok(cursor.text(), value_start, Some(b':'))
                    {
                        return Err(EvalError::malformed_json_text(cursor.text()));
                    }
                }
                validate_json_delimiters(&value_cursor, depth + 1)?;
                // #1676: same `value_at`-reuse discipline as the array arm.
                last_gap_end =
                    value_start.and_then(|s| value_cursor.value_at(s).scalar_text_end(s));
                remaining = rest;
                field_index += 1;
            }
            if remaining.ends_unpaired() {
                return Err(EvalError::malformed_json_text(cursor.text()));
            }
            // #1676: same trailing/empty-gap check as the array arm above,
            // applied to the last field's *value*.
            if let Some(open_pos) = cursor.text_position() {
                let gap_start = if field_index > 0 {
                    last_gap_end
                } else {
                    Some(open_pos + 1)
                };
                if let Some(gap_start) = gap_start {
                    if !cursor.trailing_element_gap_ok(gap_start, b'}') {
                        return Err(EvalError::malformed_json_text(cursor.text()));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Materializes a complete JSON document into an `OwnedValue` via the
/// evaluator's own `generic_to_owned`, preceded by the #1643 delimiter
/// check `bytes` must already satisfy -- `validate_json_delimiters`'s own
/// `check_nesting_depth` call also converts `generic_to_owned`'s panicking
/// `MAX_NESTING_DEPTH` guard into a catchable `EvalError` (#2295), so every
/// caller here is protected against adversarially deep input by a real,
/// designed guard rather than an unrelated library's incidental default.
///
/// #2295: used by every call site in this file that parses genuinely
/// external text -- `parse_json_value` (`--argjson`/`--jsonargs`, which
/// since #2052 validates through `validate::validate_jq_lenient` and calls
/// this once, with no retry),
/// `parse_json_stream_strict` (the primary `--slurp`/`--slurpfile`
/// document-input path), and the `--seq` per-record materializer. Before
/// #2295 only `parse_json_stream`'s `find_json_values` fallback used this;
/// every other caller reached a since-removed bare, panicking version
/// directly (`crate::output::json_bytes_to_owned_value`, now dead code with
/// no remaining callers -- deleted rather than left unused),
/// protected only incidentally by whichever upstream validator happened to
/// run first (`serde_json`'s own default 128-deep recursion limit, or
/// `validate::validate`'s own designed guard for `--seq`) -- fragile if
/// that upstream step were ever skipped, relaxed, or its own limit changed
/// independently of this one.
fn json_bytes_to_owned_value_checked(bytes: &[u8]) -> core::result::Result<OwnedValue, EvalError> {
    let index = JsonIndex::build(bytes);
    let cursor = index.root(bytes);
    validate_json_delimiters(&cursor, 0)?;
    generic_to_owned::<JqSemantics, _>(&cursor.value())
}

/// The bracket/brace-delimited, comma-and-indent-joined skeleton shared by
/// `print_json`'s and `print_owned_json`'s `Array`/`Object` arms (#3009
/// review): both walked a `len == 0` special case, a `compact` branch and a
/// pretty branch with identical comma/separator/indent placement, differing
/// only in what one element's own bytes are. Factored out so that skeleton
/// has one definition instead of four near-identical copies (two containers
/// x two printers) to keep in sync.
///
/// `write_item` gets `out` and one `T` (an array element, or an object's
/// `(key, value)` pair) and owns writing that element's bytes -- including,
/// for an object, the `"key":` prefix, since key escaping differs per
/// printer's `ascii_output` handling only in which string it escapes, not in
/// this skeleton.
///
/// `layout` bundles the four values `print_json`/`print_owned_json` already
/// compute together from `config`/`level` at the top of each function
/// (`compact` plus the three strings it gates) -- one group, not
/// `#[allow(clippy::too_many_arguments)]`'s usual `scratch`/`array_scratch`
/// situation of independent buffers that only look related.
struct JsonContainerLayout<'a> {
    compact: bool,
    separator: &'a str,
    next_indent: &'a str,
    current_indent: &'a str,
}

fn write_json_container<Out, I, T>(
    out: &mut Out,
    items: I,
    len: usize,
    brackets: (u8, u8),
    layout: &JsonContainerLayout<'_>,
    mut write_item: impl FnMut(&mut Out, T) -> Result<()>,
) -> Result<()>
where
    Out: Write,
    I: IntoIterator<Item = T>,
{
    let (open, close) = brackets;
    let JsonContainerLayout {
        compact,
        separator,
        next_indent,
        current_indent,
    } = *layout;
    if len == 0 {
        out.write_all(&[open, close])?;
        return Ok(());
    }
    out.write_all(&[open])?;
    if !compact {
        out.write_all(separator.as_bytes())?;
    }
    for (i, item) in items.into_iter().enumerate() {
        if i > 0 {
            out.write_all(b",")?;
            if !compact {
                out.write_all(separator.as_bytes())?;
            }
        }
        if !compact {
            out.write_all(next_indent.as_bytes())?;
        }
        write_item(out, item)?;
    }
    if !compact {
        out.write_all(separator.as_bytes())?;
        out.write_all(current_indent.as_bytes())?;
    }
    out.write_all(&[close])?;
    Ok(())
}

/// Print a JqValue as JSON using the provided literal formatter.
///
/// This is the unified printer that handles JSON structure (arrays, objects,
/// indentation) while delegating literal formatting to the formatter.
///
/// Guarded against adversarially deep JSON input (thousands of nested
/// arrays/objects, which would otherwise recurse this writer once per
/// nesting level and overflow the stack) by the
/// [`succinctly::jq::MAX_VALUE_TREE_DEPTH`] ceiling (384) -- **not**
/// [`succinctly::jq::eval_generic::MAX_NESTING_DEPTH`] (256), despite this
/// function importing that constant too for an unrelated panic-message
/// check elsewhere in this file. Before #1819, this writer used
/// `MAX_NESTING_DEPTH` for its own guard, one level lower than
/// `MAX_VALUE_TREE_DEPTH` -- the ceiling every *other* value-tree consumer
/// in the binary already honors (`OwnedValue::to_json`, `compare_values`,
/// `eval.rs`'s own `to_owned`, `reconcile_presentation`,
/// `format_json_impl`). A value strictly between the two limits passed
/// every construction-time guard, started printing, and only failed
/// **mid-write** -- after this function had already flushed up to 256
/// levels of syntactically-incomplete JSON to `out` (`anyhow::ensure!`
/// fires per-level, so every shallower stack frame's own opening
/// delimiter is already written by the time a deeper frame raises).
/// Matching `MAX_VALUE_TREE_DEPTH` closes that gap: nothing that passes
/// this crate's own construction-time depth ceiling can trip a *stricter*
/// one at print time. This function's own measured debug-build crash
/// boundary is 600-700 (see `MAX_NESTING_DEPTH`'s doc comment), so 384
/// leaves the same order of headroom `MAX_VALUE_TREE_DEPTH` itself was
/// tuned against for its own tightest consumer (580) -- comfortably safe.
/// A pure-cursor document deeper than 384 (no `OwnedValue` construction
/// involved at all) still hits this same corrupted-partial-output pattern,
/// just at the higher threshold -- eliminating that residual would need a
/// depth pre-check ahead of any writing, which isn't cheap to do generically
/// for an arbitrary cursor; #1819 scopes this fix to closing the
/// construction/print ceiling mismatch, not that broader limitation.
/// Unlike `eval_generic::to_owned`'s own guard (a panic, since that
/// function sits too deep in the evaluator's hot path for a `Result`-based
/// fix), this one is a clean, catchable `anyhow` error: a query like the
/// bare identity `.` never materializes an `OwnedValue` tree at all (it
/// stays lazy, streaming straight from the `JsonCursor`), so `to_owned`'s
/// own guard never gets a chance to fire for it -- this is the one
/// recursive step that shape always goes through, and it already returns
/// `Result` and already threads a `level` parameter (previously used only
/// for indentation, reused here as the depth counter), so there's no reason
/// to give up the clean failure mode the way `to_owned` had to.
#[allow(clippy::too_many_arguments)] // STYLE-0004: `array_scratch` (#1643) joins `scratch`
                                     // (#1385) as a second recursion-threaded buffer with its own distinct element type -- each
                                     // exists solely to give one specific container arm (object, array) a shared allocation instead
                                     // of a fresh `Vec` per container, so bundling them into one struct would hide that they're
                                     // independent, not a related group the way `DirectEvalOptions`'s bools are.
fn print_json<'a, F, Out, Wrd>(
    out: &mut Out,
    value: &JqValue<'a, Wrd>,
    formatter: &F,
    config: &OutputConfig,
    level: usize,
    scratch: &mut Vec<PreparedField<'a>>,
    // #1643: shared stack for the array arm's own single-walk validation,
    // same `base`/`truncate` discipline as `scratch` above and for the same
    // reason -- a fresh `Vec` per array measured as a real cost here, not
    // just for objects: the `arrays` perf-guard shape (#1523) is a top-level
    // array of ~100K 5-element arrays, so a fresh heap allocation per *inner*
    // array (100K of them) was still enough on its own to push
    // `arrays_identity` over the 5% threshold even after the double-walk fix
    // removed the dominant cost. Holds `(bp_pos, value_start)` pairs -- see
    // `PreparedField::value_start`'s doc comment for why `usize::MAX` is the
    // sentinel rather than `Option<usize>`.
    array_scratch: &mut Vec<(usize, usize)>,
    // #1643: when `value` is a `JqValue::Cursor` and the caller has
    // *already* called `text_position()` on it (to check the delimiter
    // preceding it against a sibling), it's passed through here so the
    // `Cursor` arm below can call `value_at` instead of `value` --
    // `text_position()` is a rank/select lookup, not free, and every
    // array/object element already pays for it once at the call site.
    // `None` for a value with no such caller-side lookup (the top-level
    // call, or any non-`Cursor` variant, which ignores this either way).
    known_text_pos: Option<usize>,
) -> Result<()>
where
    F: LiteralFormatter,
    Out: Write,
    Wrd: Clone + AsRef<[u64]>,
{
    // STYLE-0013-TAIL: same pre-resolved-position shape as
    // `validate_json_delimiters` above -- an explicit `is_empty()` arm
    // checking `trailing_element_gap_ok(open_pos + 1, ..)`, which is
    // `container_gap_ok`'s body, and a `last_gap_end` arm for the rest. Both
    // halves are covered, so this is not a #2594 site; it simply cannot
    // route through a helper that dispatches on `Option<last>` instead of
    // taking the gap position it already holds.
    // STYLE-0013: the object arm below calls `preceding_gap_ok` directly,
    // not `key_delimiter_ok`/`value_delimiter_ok` -- this is the streaming
    // writer's own single-walk validate-then-write pass (#1643), reusing
    // `k.start()`/`field.value_cursor().text_position()` this same walk
    // already resolved to build the output rather than a second decode.
    // Routing through the library-side wrappers only accepts an
    // already-resolved `DocumentValue`/cursor pair, which is exactly what
    // this walk is resolving for the first time as it goes.
    anyhow::ensure!(
        level < MAX_VALUE_TREE_DEPTH,
        "{}",
        succinctly::jq::nesting_depth_exceeded_message(MAX_VALUE_TREE_DEPTH)
    );
    // `|| indent_string.is_empty()`: `--indent 0` sets an empty indent with
    // `compact` false, and jq 1.7.1 prints that compactly -- confirmed live,
    // `jq --indent 0 '{a:[1,2]}'` is `{"a":[1,2]}`. `format_json`
    // (`output.rs`) has always keyed compactness off `indent.is_empty()` and
    // so already agreed with the reference; these two streaming printers
    // keyed it off `config.compact` alone and emitted newlines with
    // zero-width indents instead (#3155).
    //
    // That split used to be invisible because the two routes never met: the
    // eager `-n`/`--slurp`/DSV route formatted through `format_json` and the
    // lazy route streamed through here. Merging the owned writers put the
    // reference-correct route onto this code, so the divergence had to be
    // settled rather than documented -- and it is settled in the reference's
    // favour, per ADR-0018.
    let compact = config.compact || config.indent_string.is_empty();
    let indent = &config.indent_string;
    let current_indent = if compact {
        String::new()
    } else {
        indent.repeat(level)
    };
    let next_indent = if compact {
        String::new()
    } else {
        indent.repeat(level + 1)
    };
    let separator = if compact { "" } else { "\n" };
    let space_after_colon = if compact { "" } else { " " };
    let container_layout = JsonContainerLayout {
        compact,
        separator,
        next_indent: &next_indent,
        current_indent: &current_indent,
    };

    match value {
        JqValue::Null => out.write_all(b"null")?,
        JqValue::Bool(true) => out.write_all(b"true")?,
        JqValue::Bool(false) => out.write_all(b"false")?,
        JqValue::Int(n) => out.write_all(formatter.format_int(*n).as_bytes())?,
        JqValue::Float(f) => out.write_all(formatter.format_float(*f).as_bytes())?,
        JqValue::RawNumber(bytes) => {
            out.write_all(formatter.format_raw_number(bytes).as_bytes())?;
        }
        JqValue::NumberLiteral(literal) => {
            out.write_all(formatter.format_raw_number(literal.as_bytes()).as_bytes())?;
        }
        JqValue::Cursor(c) => {
            use succinctly::json::light::StandardJson;
            // #1643: reuse the caller's `text_position()` lookup when it
            // handed one in, instead of `value()` redoing it -- see
            // `known_text_pos`'s own doc comment above. Kept as its own
            // binding (rather than only inside the `Some` arm below) so the
            // container arms further down can reuse it for #1676's
            // trailing/empty-gap check without a second rank/select call.
            let container_pos = known_text_pos.or_else(|| c.text_position());
            let resolved = match container_pos {
                Some(text_pos) => c.value_at(text_pos),
                None => StandardJson::Error("invalid cursor position"),
            };
            match resolved {
                StandardJson::Null => out.write_all(b"null")?,
                StandardJson::Bool(true) => out.write_all(b"true")?,
                StandardJson::Bool(false) => out.write_all(b"false")?,
                // A cursor can point into reindex-bridge text: its tokens
                // print as the value they stand for, never as a spelling
                // (#3034) -- the `JqValue::Float` arm above.
                StandardJson::Number(n) => match n.bridge_value() {
                    Some(f) => out.write_all(formatter.format_float(f).as_bytes())?,
                    None => out.write_all(formatter.format_raw_number(n.raw_bytes()).as_bytes())?,
                },
                StandardJson::String(s) => {
                    // Zero-copy optimization when the source span needs no
                    // re-encoding under jq's own escape convention -- see
                    // `write_json_string_zero_copy`'s own doc comment
                    // (#2591/#2592) for the raw-DEL-byte exception.
                    let (raw, escaped, has_del) = s.raw_and_escaped();
                    write_json_string_zero_copy(out, raw, escaped, has_del, s, config)?;
                }
                StandardJson::Array(elements) => {
                    if elements.is_empty() {
                        // #1676: a stray `,`/`:` inside an apparently-empty
                        // array (`[,]`) -- cheap since there's no subtree
                        // to scan, just the gap between `[` and `]`.
                        if let Some(open_pos) = container_pos {
                            if !c.trailing_element_gap_ok(open_pos + 1, b']') {
                                return Err(MalformedJsonError::new(
                                    EvalError::malformed_json_text(c.text()),
                                )
                                .into());
                            }
                        }
                        out.write_all(b"[]")?;
                    } else {
                        // #1643: validate every element's preceding
                        // delimiter *before* writing anything, same
                        // discipline as the object arm below and for the
                        // same reason (its own comment explains why: a
                        // malformed document must not leave a partial `{`
                        // -- or here, `[` -- already on `out` when the
                        // error surfaces).
                        //
                        // Walked exactly once: a first cut called
                        // `elements.cursor_iter()` a second time for the
                        // write loop, re-navigating the same BP siblings
                        // it had just walked for the check -- the same
                        // shape of re-walk the object arm's own comment
                        // below measured at 8-9% of `sjq '.'` over a 10 MB
                        // document, and here it cost 12-15% of instruction
                        // count on a large flat array (#1643 perf-guard
                        // failure, arrays_identity). Recording each
                        // element's BP position -- not a whole
                        // `JsonCursor`, for the same reason `PreparedField`
                        // hoists `text`/`index` into `Frame` rather than
                        // storing them per element -- plus its
                        // already-resolved `text_position()` lets the
                        // write loop reconstruct the cursor via
                        // `Frame::cursor` instead of re-deriving it.
                        //
                        // `array_scratch` is the shared buffer threaded
                        // through the whole recursion (see its own doc
                        // comment on `print_json`'s signature): this
                        // array's elements occupy `base..`, a nested array
                        // appends past that and truncates back on the way
                        // out, same stack discipline `scratch` already uses
                        // for object fields.
                        let frame = Frame {
                            text: c.text(),
                            index: c.index(),
                        };
                        let base = array_scratch.len();
                        // #1676: the last element's own end position (cheap
                        // for a scalar; `None` for a container -- see
                        // `scalar_text_end`'s own doc comment) lets a
                        // trailing `,` before `]` (`[1,2,]`) be caught
                        // below, before anything is written. Reuses `pos`
                        // (already resolved by `check_preceding_delimiter`)
                        // via `value_at` rather than `child_cursor.value()`,
                        // which would re-derive the same position a second
                        // time -- see `scalar_text_end`'s own doc comment on
                        // why that matters on this hot path specifically.
                        // Resolved for the last element only, after the
                        // loop: every element's value used to be built here
                        // and discarded, and since #3222 building a number
                        // validates its span.
                        let mut last_child = None;
                        for (i, child_cursor) in elements.cursor_iter().enumerate() {
                            let pos = check_preceding_delimiter(&child_cursor, i)?;
                            array_scratch
                                .push((child_cursor.bp_position(), pos.unwrap_or(usize::MAX)));
                            last_child = Some((child_cursor, pos));
                        }
                        let last_gap_end = last_child.and_then(|(child_cursor, pos)| {
                            pos.and_then(|s| child_cursor.value_at(s).scalar_text_end(s))
                        });
                        if let Some(gap_start) = last_gap_end {
                            if !c.trailing_element_gap_ok(gap_start, b']') {
                                array_scratch.truncate(base);
                                return Err(MalformedJsonError::new(
                                    EvalError::malformed_json_text(c.text()),
                                )
                                .into());
                            }
                        }
                        if compact {
                            out.write_all(b"[")?;
                            for i in base..array_scratch.len() {
                                let (bp, value_start) = array_scratch[i];
                                if i > base {
                                    out.write_all(b",")?;
                                }
                                let child_value = JqValue::Cursor(frame.cursor(bp));
                                let known_text_pos =
                                    (value_start != usize::MAX).then_some(value_start);
                                print_json(
                                    out,
                                    &child_value,
                                    formatter,
                                    config,
                                    level + 1,
                                    scratch,
                                    array_scratch,
                                    known_text_pos,
                                )?;
                            }
                            out.write_all(b"]")?;
                        } else {
                            out.write_all(b"[")?;
                            out.write_all(separator.as_bytes())?;
                            for i in base..array_scratch.len() {
                                let (bp, value_start) = array_scratch[i];
                                if i > base {
                                    out.write_all(b",")?;
                                    out.write_all(separator.as_bytes())?;
                                }
                                out.write_all(next_indent.as_bytes())?;
                                let child_value = JqValue::Cursor(frame.cursor(bp));
                                let known_text_pos =
                                    (value_start != usize::MAX).then_some(value_start);
                                print_json(
                                    out,
                                    &child_value,
                                    formatter,
                                    config,
                                    level + 1,
                                    scratch,
                                    array_scratch,
                                    known_text_pos,
                                )?;
                            }
                            out.write_all(separator.as_bytes())?;
                            out.write_all(current_indent.as_bytes())?;
                            out.write_all(b"]")?;
                        }
                        array_scratch.truncate(base);
                    }
                }
                StandardJson::Object(fields) => {
                    if fields.is_empty() {
                        // #1676: a stray `,`/`:` inside an apparently-empty
                        // object (`{,}`) -- same reasoning as the array
                        // arm's own empty-case check above.
                        if let Some(open_pos) = container_pos {
                            if !c.trailing_element_gap_ok(open_pos + 1, b'}') {
                                return Err(MalformedJsonError::new(
                                    EvalError::malformed_json_text(c.text()),
                                )
                                .into());
                            }
                        }
                        out.write_all(b"{}")?;
                    } else {
                        // #1385: jq collapses a repeated key to its first
                        // position holding its last value, so the printer --
                        // the only place a cursor-backed object reaches
                        // output -- must too. That one change settles `.`,
                        // `.[0]`, `first(.[])` and `last(.[])` together: they
                        // diverged only because the value arriving here was
                        // still a cursor.
                        //
                        // A source-preserving convention keeps every
                        // occurrence *on output*. Reproducing the input
                        // verbatim is that extension's purpose, and ADR-0018
                        // rule 5 allows it because no reference-defined
                        // filter is perturbed. The evaluator is not exempt --
                        // see `docs/compliance/jq/limitations.md`.
                        //
                        // The field list is walked exactly once. `uncons` is
                        // BP navigation -- two sibling hops per field -- so
                        // re-walking it purely to probe for duplicates cost
                        // 8-9% of `sjq '.'` over a 10 MB document, an order
                        // more than the key text scan did. Recording each
                        // key's span on the way past costs nothing extra and
                        // hands the write loops below a span they no longer
                        // have to find for themselves.
                        //
                        // `scratch` is one buffer shared by the whole
                        // recursion, used as a stack: this object's fields
                        // occupy `base..end`, a nested object appends past
                        // `end` and truncates back on the way out. A fresh
                        // `Vec` per object instead measured ~3% on the same
                        // document -- small objects are the common case, so
                        // the allocator traffic showed up rather than the
                        // copying.
                        let frame = Frame {
                            text: c.text(),
                            index: c.index(),
                        };
                        let base = scratch.len();
                        // Driven by `uncons` rather than `for field in fields`
                        // so the walk's final position survives the loop: the
                        // `Iterator` adaptor discards it, and recovering it
                        // afterwards would mean a second `uncons` per field --
                        // the very re-walk the comment above measured at 8-9%
                        // of `sjq '.'` over 10 MB. This is the same single
                        // walk, just keeping its own tail.
                        let mut remaining = fields;
                        let mut field_index = 0usize;
                        // #1676: mirrors the array arm's own tracking, for
                        // the last field's *value* rather than an element --
                        // resolved after the loop, for the last field only.
                        let mut last_value = None;
                        while let Some((field, rest)) = remaining.uncons() {
                            // The `_` arm is *not* unreachable, whatever this
                            // comment used to claim: nothing enforces the JSON
                            // grammar on the default path, so `{invalid: 1}`
                            // reaches here with a bareword key (#1194). It
                            // used to record an empty span, which
                            // `write_object_key` then wrote as nothing at all
                            // -- including the colon -- producing `{1}`.
                            //
                            // Raised here rather than left to
                            // `write_object_key` so the whole object is
                            // rejected before its opening `{` goes out: that
                            // one still raises, but by then the brace has been
                            // written and stdout carries a stray `{`.
                            let StandardJson::String(k) = field.key() else {
                                scratch.truncate(base);
                                return Err(MalformedJsonError::new(
                                    EvalError::malformed_json_text(frame.text),
                                )
                                .into());
                            };
                            // #1643: this key must be preceded by a `,` (or
                            // nothing, if it's the object's first field), and
                            // its value must be preceded by a `:` -- neither
                            // is enforced anywhere else on this path. See
                            // `preceding_gap_ok` for why a missing/doubled
                            // trailing comma is what this catches, not a
                            // trailing one. `k.start()` reuses the position
                            // `field.key()` just resolved above rather than
                            // asking `field.key_cursor()` to re-derive it.
                            let key_start = k.start();
                            let value_start = field.value_cursor().text_position();
                            if let Some(value_start) = value_start {
                                let comma_expected =
                                    if field_index == 0 { None } else { Some(b',') };
                                if !preceding_gap_ok(frame.text, key_start, comma_expected)
                                    || !preceding_gap_ok(frame.text, value_start, Some(b':'))
                                {
                                    scratch.truncate(base);
                                    return Err(MalformedJsonError::new(
                                        EvalError::malformed_json_text(frame.text),
                                    )
                                    .into());
                                }
                            }
                            // #1676: same last-child tracking as the array
                            // arm, reusing `value_start` via `value_at`
                            // rather than `field.value_cursor().value()`,
                            // which would re-derive it.
                            last_value = Some((field.value_cursor(), value_start));
                            let (raw, escaped, has_del) = k.raw_and_escaped();
                            scratch.push(PreparedField {
                                key_bp: field.key_cursor().bp_position(),
                                value_bp: field.value_cursor().bp_position(),
                                value_start: value_start.unwrap_or(usize::MAX),
                                raw,
                                escaped,
                                has_del,
                            });
                            remaining = rest;
                            field_index += 1;
                        }
                        // An odd number of BP children means the text was
                        // never `key: value` -- `{invalid}`, `{"a"}`, or the
                        // trailing `2` of `{invalid, "b":2}`. `uncons` drops
                        // that child silently, so without this the object
                        // printed as `{}` (or one field short) at exit 0
                        // (#1194). Checked before the opening `{` is written
                        // so the raise leaves no partial record behind.
                        //
                        // Free for a well-formed object: the walk ends with a
                        // `None` key cursor, which `ends_unpaired` answers
                        // without touching the BP tree at all.
                        if remaining.ends_unpaired() {
                            scratch.truncate(base);
                            return Err(MalformedJsonError::new(EvalError::malformed_json_text(
                                frame.text,
                            ))
                            .into());
                        }
                        let last_gap_end = last_value.and_then(|(value_cursor, value_start)| {
                            value_start.and_then(|s| value_cursor.value_at(s).scalar_text_end(s))
                        });
                        if let Some(gap_start) = last_gap_end {
                            if !c.trailing_element_gap_ok(gap_start, b'}') {
                                scratch.truncate(base);
                                return Err(MalformedJsonError::new(
                                    EvalError::malformed_json_text(frame.text),
                                )
                                .into());
                            }
                        }
                        let collapsed = if !config.convention.preserves_source_values() {
                            collapse_duplicate_fields(&scratch[base..], &frame)
                        } else {
                            None
                        };
                        let count = collapsed.as_ref().map_or(scratch.len() - base, Vec::len);

                        out.write_all(b"{")?;
                        out.write_all(separator.as_bytes())?;
                        for i in 0..count {
                            let field = match &collapsed {
                                Some(list) => list[i],
                                None => scratch[base + i],
                            };
                            if i > 0 {
                                out.write_all(b",")?;
                                out.write_all(separator.as_bytes())?;
                            }
                            out.write_all(next_indent.as_bytes())?;
                            write_object_key(out, &frame, &field, config, space_after_colon)?;
                            let child_value = JqValue::Cursor(frame.cursor(field.value_bp));
                            // #1643: `value_start` was already resolved by
                            // the check above (or is the sentinel, if that
                            // check's own lookup came back `None`) -- see
                            // `PreparedField::value_start`'s doc comment.
                            let known_text_pos =
                                (field.value_start != usize::MAX).then_some(field.value_start);
                            print_json(
                                out,
                                &child_value,
                                formatter,
                                config,
                                level + 1,
                                scratch,
                                array_scratch,
                                known_text_pos,
                            )?;
                        }
                        out.write_all(separator.as_bytes())?;
                        out.write_all(current_indent.as_bytes())?;
                        out.write_all(b"}")?;
                        // Hand this object's slots back to the shared stack.
                        // Without it the buffer would keep every field of
                        // every object printed so far, growing with the whole
                        // document rather than with its nesting depth.
                        scratch.truncate(base);
                    }
                }
                // A structurally malformed value the semi-index accepted as a
                // span but could not classify (`[xyz123]`, `[tru]`). An
                // earlier attempt to raise here predated `MalformedJsonError`
                // (added for the object-member check just above): without
                // it, bailing produced a truncated document at a *generic*
                // exit 1 -- worse on every axis than the silent `null` it
                // replaced, so it was reverted back to `null` (#1194). That
                // convention exists now, so reuse it: same truncated-prefix
                // trade the object arm above and the `keys_unsorted` writer
                // already make (`docs/compliance/jq/limitations.md`), but a
                // clean diagnostic and jq's own exit 5 instead of a silent
                // wrong answer (#1641).
                StandardJson::Error(_) => {
                    return Err(
                        MalformedJsonError::new(EvalError::malformed_json_text(c.text())).into(),
                    );
                }
            }
        }
        JqValue::String(s) => {
            out.write_all(b"\"")?;
            let escaped = if config.ascii_output {
                escape_json_string_ascii(s)
            } else {
                escape_json_string(s)
            };
            out.write_all(escaped.as_bytes())?;
            out.write_all(b"\"")?;
        }
        JqValue::Array(arr) => {
            write_json_container(
                out,
                arr.iter(),
                arr.len(),
                (b'[', b']'),
                &container_layout,
                |out, v| {
                    print_json(
                        out,
                        v,
                        formatter,
                        config,
                        level + 1,
                        scratch,
                        array_scratch,
                        None,
                    )
                },
            )?;
        }
        JqValue::Object(obj) => {
            write_json_container(
                out,
                obj.iter(),
                obj.len(),
                (b'{', b'}'),
                &container_layout,
                |out, (k, v)| {
                    out.write_all(b"\"")?;
                    let escaped = if config.ascii_output {
                        escape_json_string_ascii(k)
                    } else {
                        escape_json_string(k)
                    };
                    out.write_all(escaped.as_bytes())?;
                    out.write_all(b"\":")?;
                    out.write_all(space_after_colon.as_bytes())?;
                    print_json(
                        out,
                        v,
                        formatter,
                        config,
                        level + 1,
                        scratch,
                        array_scratch,
                        None,
                    )
                },
            )?;
        }
        // Genuinely lazy: stream each key's raw bytes straight from its
        // cursor, same zero-copy convention as `JqValue::Cursor`'s
        // `StandardJson::Object` arm above — never collects a `Vec<String>`
        // first. Compact/pretty duplicated as two full loops, matching the
        // `StandardJson::Array`/`StandardJson::Object` arms above rather
        // than branching mid-loop.
        JqValue::LazyKeysArray { fields, collapse } => {
            use succinctly::json::light::StandardJson as SJ;
            // The unpaired-child half of #1194, checked before the `[` --
            // it is O(1) (`ends_unpaired` answers from a `None` cursor
            // without touching the BP tree), so it is free on well-formed
            // input.
            //
            // The non-string-key half is *not* checked up front, and that is
            // deliberate. This writer streams straight to `out` and cannot
            // rewind, so catching it early would mean a second walk over
            // every key -- and `keys_unsorted` over a 2 MB `wide` document is
            // one of the workloads `scripts/perf-guard.py` pins precisely
            // because it is sensitive to exactly that. The per-key arms below
            // raise instead, which can leave a truncated array on stdout
            // alongside the exit 5. That divergence is recorded in
            // `docs/compliance/jq/limitations.md`; it is the same trade the
            // YAML streaming path already makes, and for the same reason.
            if let Some(tail) = fields.unpaired_tail() {
                return Err(
                    MalformedJsonError::new(EvalError::malformed_json_text(tail.text())).into(),
                );
            }
            if fields.is_empty() {
                out.write_all(b"[]")?;
            } else if compact {
                out.write_all(b"[")?;
                // Iterated by `by_ref` so the walk's own
                // answer to "did this object end on an orphan?" survives it
                // (#1194). `unpaired_tail` above cannot see that: asked of
                // the list's *head* it reports only on the first child, so
                // `{"a":1, invalid}` reads as well formed there and used to
                // print a complete, wrong `["a"]` at exit 0.
                let mut keys = DistinctKeyCursors::new(fields, *collapse);
                let mut doc_text: Option<&[u8]> = None;
                for (i, (key, key_cursor)) in keys.by_ref().enumerate() {
                    if i > 0 {
                        out.write_all(b",")?;
                    }
                    doc_text = Some(key_cursor.text());
                    let SJ::String(k) = key else {
                        // Reachable, and the reason this writer can leave
                        // a truncated `[` behind: the check above it is the
                        // O(1) `unpaired_tail` one, which catches `{invalid}`
                        // but says nothing about a key's *type*. Catching
                        // that before the bracket would need a second walk
                        // over every key, on a path `scripts/perf-guard.py`
                        // pins -- so it is caught here instead, after the
                        // bracket is already out. Raising still beats the
                        // `[,"b"]` this used to print at exit 0 (#1194); see
                        // `docs/compliance/jq/limitations.md`.
                        return Err(MalformedJsonError::new(EvalError::malformed_json_text(
                            key_cursor.text(),
                        ))
                        .into());
                    };
                    // Zero-copy optimization when the source span needs no
                    // re-encoding under jq's own escape convention -- see
                    // `write_json_string_zero_copy`'s own doc comment
                    // (#2591/#2592) for the raw-DEL-byte exception.
                    let (raw, escaped, has_del) = k.raw_and_escaped();
                    write_json_string_zero_copy(out, raw, escaped, has_del, k, config)?;
                }
                bail_if_keys_malformed(&keys, doc_text)?;
                out.write_all(b"]")?;
            } else {
                out.write_all(b"[")?;
                out.write_all(separator.as_bytes())?;
                // See the compact branch above for why the iterator is kept
                // alive past the loop (#1194).
                let mut keys = DistinctKeyCursors::new(fields, *collapse);
                let mut doc_text: Option<&[u8]> = None;
                for (i, (key, key_cursor)) in keys.by_ref().enumerate() {
                    if i > 0 {
                        out.write_all(b",")?;
                        out.write_all(separator.as_bytes())?;
                    }
                    doc_text = Some(key_cursor.text());
                    out.write_all(next_indent.as_bytes())?;
                    let SJ::String(k) = key else {
                        // Reachable, and the reason this writer can leave
                        // a truncated `[` behind: the check above it is the
                        // O(1) `unpaired_tail` one, which catches `{invalid}`
                        // but says nothing about a key's *type*. Catching
                        // that before the bracket would need a second walk
                        // over every key, on a path `scripts/perf-guard.py`
                        // pins -- so it is caught here instead, after the
                        // bracket is already out. Raising still beats the
                        // `[,"b"]` this used to print at exit 0 (#1194); see
                        // `docs/compliance/jq/limitations.md`.
                        return Err(MalformedJsonError::new(EvalError::malformed_json_text(
                            key_cursor.text(),
                        ))
                        .into());
                    };
                    // Zero-copy optimization when the source span needs no
                    // re-encoding under jq's own escape convention -- see
                    // `write_json_string_zero_copy`'s own doc comment
                    // (#2591/#2592) for the raw-DEL-byte exception.
                    let (raw, escaped, has_del) = k.raw_and_escaped();
                    write_json_string_zero_copy(out, raw, escaped, has_del, k, config)?;
                }
                bail_if_keys_malformed(&keys, doc_text)?;
                out.write_all(separator.as_bytes())?;
                out.write_all(current_indent.as_bytes())?;
                out.write_all(b"]")?;
            }
        }
        // Genuinely lazy, same convention as `LazyKeysArray` above: no
        // `Vec<OwnedValue::Int>`/child `JqValue`s ever built, just ASCII
        // digits written straight to `out` (#684).
        JqValue::LazyIndexRange(len) => {
            if *len == 0 {
                out.write_all(b"[]")?;
            } else if compact {
                out.write_all(b"[")?;
                for i in 0..*len {
                    if i > 0 {
                        out.write_all(b",")?;
                    }
                    write!(out, "{i}")?;
                }
                out.write_all(b"]")?;
            } else {
                out.write_all(b"[")?;
                out.write_all(separator.as_bytes())?;
                for i in 0..*len {
                    if i > 0 {
                        out.write_all(b",")?;
                        out.write_all(separator.as_bytes())?;
                    }
                    out.write_all(next_indent.as_bytes())?;
                    write!(out, "{i}")?;
                }
                out.write_all(separator.as_bytes())?;
                out.write_all(current_indent.as_bytes())?;
                out.write_all(b"]")?;
            }
        }
    }
    Ok(())
}

/// [`print_json`]'s twin for a result that is **already owned** (#3009).
///
/// `GenericResult::Owned`/`ManyOwned`/`Partial` and the sorted-`keys` arm all
/// hand `jq_runner` a finished `OwnedValue`. Printing it used to mean
/// rebuilding it as a `JqValue` first (`JqValue::try_from_owned`), which
/// walks the tree object by object, freeing each source map and immediately
/// allocating the destination one. Since #3000 the two enums are 32 and 40
/// bytes, so those two chunk sizes land in different allocator bins and
/// nothing freed fits what is asked for next -- DHAT on
/// `wide_10mb | to_entries` measured *less* live heap at peak than the
/// pre-#3000 base (454 MB vs 533 MB) with more RSS, the difference being
/// holes. See ADR-0024 option G.
///
/// **This function's contract is byte-for-byte equality with
/// `print_json(&JqValue::try_from_owned(v))`, not "correct JSON".** That is
/// achievable by construction rather than by care, because
/// `JqValue::from_owned_at_depth` (`src/jq/lazy.rs`) is a total, structure-
/// preserving map from `OwnedValue`'s 7 variants onto 7 of `JqValue`'s: each
/// arm below is therefore the composition of that map with `print_json`'s
/// corresponding arm, and can be read off against it side by side.
/// `print_owned_json_matches_print_json_3009` sweeps the pair over every
/// output configuration rather than trusting the reading.
///
/// Two arms carry the whole risk, both number-shaped:
///
/// - `NumberLiteral`'s `NumberRepr` is **deliberately discarded** (`_`), the
///   way `from_owned_at_depth` discards it. It is not spare information to
///   make use of here: `stream.rs`'s own owned writer does dispatch on it,
///   and that is exactly why it prints `null` for `1e400` where this path
///   must print `1E+400` (pinned in `tests/jq_cli_tests.rs`). The `_` is
///   load-bearing.
/// - Every scalar goes through the caller's `F: LiteralFormatter` rather
///   than any local formatting. That is what keeps a computed `infinite`
///   printing as `1.7976931348623157e+308`: the non-finite rule lives in
///   `format_float`'s `nonfinite_display_string` call, and re-deriving it
///   here would be a second copy to drift.
///
/// No `scratch`/`array_scratch`/`known_text_pos`: those exist for
/// `print_json`'s `Cursor` arm, and `Cursor`/`RawNumber`/`LazyKeysArray`/
/// `LazyIndexRange` are all unreachable from an owned tree.
///
/// The depth `ensure!` mirrors `print_json`'s but is unreachable from the
/// CLI: `to_jq_values` rejects an over-deep owned value through
/// `OwnedValue::check_tree_depth` before any byte is written, which is what
/// keeps `test_partial_result_over_depth_value_reports_cleanly_not_panic_1371`
/// true (an in-writer check fires 384 levels of `[` too late). It stays as a
/// backstop for a future caller that skips that gate, and is covered
/// directly by `print_owned_json_depth_guard_3009`.
fn print_owned_json<F, Out>(
    out: &mut Out,
    value: &OwnedValue,
    formatter: &F,
    config: &OutputConfig,
    level: usize,
) -> Result<()>
where
    F: LiteralFormatter,
    Out: Write,
{
    anyhow::ensure!(
        level < MAX_VALUE_TREE_DEPTH,
        "{}",
        succinctly::jq::nesting_depth_exceeded_message(MAX_VALUE_TREE_DEPTH)
    );
    // `|| indent_string.is_empty()`: `--indent 0` sets an empty indent with
    // `compact` false, and jq 1.7.1 prints that compactly -- confirmed live,
    // `jq --indent 0 '{a:[1,2]}'` is `{"a":[1,2]}`. `format_json`
    // (`output.rs`) has always keyed compactness off `indent.is_empty()` and
    // so already agreed with the reference; these two streaming printers
    // keyed it off `config.compact` alone and emitted newlines with
    // zero-width indents instead (#3155).
    //
    // That split used to be invisible because the two routes never met: the
    // eager `-n`/`--slurp`/DSV route formatted through `format_json` and the
    // lazy route streamed through here. Merging the owned writers put the
    // reference-correct route onto this code, so the divergence had to be
    // settled rather than documented -- and it is settled in the reference's
    // favour, per ADR-0018.
    let compact = config.compact || config.indent_string.is_empty();
    let indent = &config.indent_string;
    let current_indent = if compact {
        String::new()
    } else {
        indent.repeat(level)
    };
    let next_indent = if compact {
        String::new()
    } else {
        indent.repeat(level + 1)
    };
    let separator = if compact { "" } else { "\n" };
    let space_after_colon = if compact { "" } else { " " };
    let container_layout = JsonContainerLayout {
        compact,
        separator,
        next_indent: &next_indent,
        current_indent: &current_indent,
    };

    match value {
        OwnedValue::Null => out.write_all(b"null")?,
        OwnedValue::Bool(true) => out.write_all(b"true")?,
        OwnedValue::Bool(false) => out.write_all(b"false")?,
        OwnedValue::Int(n) => out.write_all(formatter.format_int(*n).as_bytes())?,
        OwnedValue::Float(f) => out.write_all(formatter.format_float(*f).as_bytes())?,
        // The `_` on the repr is load-bearing -- see this function's doc.
        OwnedValue::NumberLiteral(_, literal) => {
            out.write_all(formatter.format_raw_number(literal.as_bytes()).as_bytes())?;
        }
        OwnedValue::String(s) => {
            out.write_all(b"\"")?;
            let escaped = if config.ascii_output {
                escape_json_string_ascii(s)
            } else {
                escape_json_string(s)
            };
            out.write_all(escaped.as_bytes())?;
            out.write_all(b"\"")?;
        }
        OwnedValue::Array(arr) => {
            write_json_container(
                out,
                arr.iter(),
                arr.len(),
                (b'[', b']'),
                &container_layout,
                |out, v| print_owned_json(out, v, formatter, config, level + 1),
            )?;
        }
        OwnedValue::Object(obj) => {
            write_json_container(
                out,
                obj.iter(),
                obj.len(),
                (b'{', b'}'),
                &container_layout,
                |out, (k, v)| {
                    out.write_all(b"\"")?;
                    let escaped = if config.ascii_output {
                        escape_json_string_ascii(k)
                    } else {
                        escape_json_string(k)
                    };
                    out.write_all(escaped.as_bytes())?;
                    out.write_all(b"\":")?;
                    out.write_all(space_after_colon.as_bytes())?;
                    print_owned_json(out, v, formatter, config, level + 1)
                },
            )?;
        }
    }
    Ok(())
}

/// Format a value as JSON.
fn format_json(value: &OwnedValue, config: &OutputConfig) -> String {
    let opts = JsonFormatOpts {
        indent: if config.compact {
            ""
        } else {
            &config.indent_string
        },
        sort_keys: config.sort_keys,
        ascii: config.ascii_output,
        // #2852: `-S`/`-a`/`-C`/`-s` used to reformat a `NumberLiteral`
        // regardless of `--preserve-input`, disagreeing with `print_json`'s
        // `JqCompatFormatter`/`PreserveFormatter` split on the default `-c`
        // route; #2874 replaced the `(control_escape, jq_compat)` pair that
        // fix left behind with the one convention value both routes now read.
        convention: config.convention,
        // Only meaningful alongside `JsonConvention::Preserve` (see
        // `JsonFormatOpts::json_sourced`'s own doc comment) -- jq mode
        // never consults it.
        json_sourced: false,
    };
    let json = output::format_json(value, &opts);

    if config.color_output {
        output::colorize_json(&json, &config.color_scheme)
    } else {
        json
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #3261 review: `nesting_depth_panic_message`'s whole point is telling
    /// apart two panics that share the identical message template
    /// (`assert_depth`, `src/jq/value.rs`) except for the interpolated
    /// number -- exact `MAX_NESTING_DEPTH` (256) match only, `None` for
    /// everything else, including the different-but-textually-similar
    /// `MAX_VALUE_TREE_DEPTH` (384) guard (f2789080a's own narrowing, #1793
    /// review). Before #3261 this was reachable end-to-end via `reduce
    /// range(400) as $i (null; [.])`, a live 384 panic that proved the
    /// narrowing correctly declined to catch it -- #3261 fixed that guard's
    /// most common call site to no longer panic at all, orphaning that
    /// live coverage. This pins the discrimination directly against the
    /// payload shapes a real panic actually produces, independent of
    /// whether any call site still panics with either message.
    #[test]
    fn nesting_depth_panic_message_distinguishes_the_two_guards_3261() {
        let payload_256 = Box::new(format!(
            "nesting depth exceeds limit of {MAX_NESTING_DEPTH}"
        )) as Box<dyn core::any::Any + Send>;
        assert_eq!(
            nesting_depth_panic_message(&*payload_256).as_deref(),
            Some("nesting depth exceeds limit of 256")
        );

        // The exact text `assert_value_tree_depth` panics with today
        // (`MAX_VALUE_TREE_DEPTH`, src/jq/value.rs) -- must NOT match.
        let payload_384 = Box::new("nesting depth exceeds limit of 384".to_string())
            as Box<dyn core::any::Any + Send>;
        assert_eq!(nesting_depth_panic_message(&*payload_384), None);

        // An unrelated panic payload, and the `&'static str` shape a bare
        // `panic!("literal")` produces (defense-in-depth per this
        // function's own doc comment) -- neither matches either.
        let payload_other =
            Box::new("some other panic".to_string()) as Box<dyn core::any::Any + Send>;
        assert_eq!(nesting_depth_panic_message(&*payload_other), None);
        let payload_static: Box<dyn core::any::Any + Send> = Box::new("literal panic");
        assert_eq!(nesting_depth_panic_message(&*payload_static), None);
    }

    /// #2951: run ids are interned per resolved module, and a module that
    /// cannot be resolved still gets one.
    ///
    /// The unresolvable arm is not reachable through the CLI -- every caller
    /// runs after a successful `load_module` -- but it is not dead either:
    /// it is what keeps `run_id_for` total, and a run id is only ever a
    /// diagnostic key, never a correctness one (`scan_scope` pairs a begin
    /// marker with an end by nesting, not by id). Unit-tested rather than
    /// left as an uncovered defensive line.
    mod run_id_interning_2951 {
        use super::*;

        #[test]
        fn unresolvable_module_still_gets_a_stable_id() {
            let mut loader = ModuleLoader::new(&[]);
            let first = loader.run_id_for("no-such-module-anywhere");
            let again = loader.run_id_for("no-such-module-anywhere");
            assert_eq!(first, again, "the same path must intern to one id");

            let other = loader.run_id_for("a-different-missing-module");
            assert_ne!(first, other, "distinct paths get distinct ids");

            // `~/.jq`'s reserved id is never handed out to a module.
            assert_ne!(first, AUTO_LOAD_RUN_ID);
            assert_ne!(other, AUTO_LOAD_RUN_ID);

            // Every id maps back to something nameable, which is the only
            // thing the id is for.
            assert_eq!(
                loader.run_origin(first),
                Some("no-such-module-anywhere"),
                "an unresolvable module is keyed by the path as written"
            );
            assert_eq!(loader.run_origin(u32::MAX), None, "unknown id");
        }
    }

    /// #2955: the processed program grows *linearly* with the depth of a
    /// module chain whose defs each call more than one def from the level
    /// below.
    ///
    /// The issue's own generator: `L` levels of `D` defs, each calling `F`
    /// defs of the level below, resolved at the top. Binding by copying the
    /// dependency ASTs into every caller held `F^L` copies of the bottom
    /// level; linking each dependency module once and forwarding to it holds
    /// each body once. A wall-clock or RSS bound would flake on a loaded CI
    /// box, so this counts `Expr` nodes instead and asserts the per-level
    /// delta is constant -- a `~/.jq` on the developer's machine adds the
    /// same constant to all three counts and cancels out.
    ///
    /// Against the copying loader this read 1253 / 5093 / 20453 nodes at
    /// L = 6 / 8 / 10 (deltas 3840 and 15360: x4 per two levels, as `F^L`
    /// predicts, and it failed here); with linking the deltas are equal.
    mod link_size_guard_2955 {
        use super::*;

        fn generate(dir: &std::path::Path, levels: usize, defs: usize, fan_out: usize) {
            let mut m0 = String::new();
            for i in 0..defs {
                m0.push_str(&format!("def f0_{i}: {i};\n"));
            }
            std::fs::write(dir.join("m0.jq"), m0).expect("write m0");
            for lvl in 1..levels {
                let mut src = format!("include \"m{}\";\n", lvl - 1);
                for i in 0..defs {
                    let calls: Vec<String> = (0..fan_out)
                        .map(|k| format!("f{}_{}", lvl - 1, (i + k) % defs))
                        .collect();
                    src.push_str(&format!("def f{lvl}_{i}: {};\n", calls.join(" + ")));
                }
                std::fs::write(dir.join(format!("m{lvl}.jq")), src).expect("write module");
            }
        }

        fn node_count(levels: usize) -> usize {
            let dir = tempfile::tempdir().expect("tempdir");
            generate(dir.path(), levels, 4, 2);
            let mut loader = ModuleLoader::new(&[dir.path().to_path_buf()]);
            let filter = format!("include \"m{}\"; f{}_0", levels - 1, levels - 1);
            let program = jq::parse_program(&filter).expect("parse");
            // The CLI loads through `unqualified_def_names` first, then
            // `process_program`; mirror that so the memo is exercised the
            // same way.
            loader.unqualified_def_names(&program).expect("names");
            let expr = loader.process_program(&program).expect("process");
            let mut n = 0usize;
            succinctly::jq::walk::any_subexpr(&expr, &mut |_| {
                n += 1;
                false
            });
            n
        }

        #[test]
        fn fan_out_chain_grows_linearly_with_depth() {
            let (n6, n8, n10) = (node_count(6), node_count(8), node_count(10));
            eprintln!("nodes at L = 6 / 8 / 10: {n6} / {n8} / {n10}");
            assert_eq!(
                n8 - n6,
                n10 - n8,
                "per-level growth must be constant: {n6} / {n8} / {n10}"
            );
        }

        /// The shadow-candidate seed (#2395) reads a module's own defs from
        /// the cache, never a link run, so no hidden spelling can reach the
        /// parser.
        #[test]
        fn unqualified_def_names_never_carry_a_link_spelling() {
            let dir = tempfile::tempdir().expect("tempdir");
            generate(dir.path(), 4, 3, 2);
            let mut loader = ModuleLoader::new(&[dir.path().to_path_buf()]);
            let program = jq::parse_program("include \"m3\"; f3_0").expect("parse");
            let names = loader.unqualified_def_names(&program).expect("names");
            assert!(names.contains("f3_0"));
            assert!(!names.contains("f2_0"), "a dependency is not re-exported");
            assert!(
                names.iter().all(|n| !n.starts_with('\u{0}')),
                "hidden spelling leaked: {names:?}"
            );
        }
    }

    /// #2955 review: `loaded_modules` must be keyed by the canonical file
    /// (`ModuleLoader::run_key`), not the literal path a caller wrote, or two
    /// spellings of one module reaching it from different includers (`"dep"`
    /// from one, `"./dep"` from another) parse and bind it twice.
    ///
    /// Output was never wrong on this path -- a `dependency_signatures` call
    /// records both the `loaded_modules` entry and the `run_id_for`/
    /// `run_origins` entry for the same spelling in the same call, so the
    /// linking step downstream always found *a* copy of the module -- but
    /// before this test's fix, each distinct literal spelling paid its own
    /// full parse-and-bind pass, including recursively loading *that*
    /// module's own dependencies again. #2955 exists specifically to bound
    /// module-loading cost, so a wide fan-out where many consumers each
    /// spell a shared leaf module slightly differently would reintroduce
    /// part of the duplication the linking fix eliminates.
    mod duplicate_spelling_shares_one_load_2955 {
        use super::*;

        #[test]
        fn two_spellings_of_one_module_load_once() {
            let dir = tempfile::tempdir().expect("tempdir");
            std::fs::write(dir.path().join("common.jq"), "def cfn: 42;\n").expect("write common");
            std::fs::write(
                dir.path().join("s1.jq"),
                "include \"common\";\ndef s1fn: cfn + 1;\n",
            )
            .expect("write s1");
            std::fs::write(
                dir.path().join("s2.jq"),
                "include \"./common\";\ndef s2fn: cfn + 2;\n",
            )
            .expect("write s2");

            let mut loader = ModuleLoader::new(&[dir.path().to_path_buf()]);
            loader.load_module("s1").expect("load s1");
            loader.load_module("s2").expect("load s2");

            assert_eq!(
                loader.loaded_modules.len(),
                3,
                "common.jq must load once despite the two spellings (s1, s2 and one \
                 common entry): {:?}",
                loader.loaded_modules.keys().collect::<Vec<_>>()
            );
        }
    }

    /// #1525: `seq_no_rs_byte_warning` direct unit coverage. Every expected
    /// value here was live-verified against the pinned jq 1.7.1 binary
    /// (see `tests/jq_cli_tests.rs`'s CLI-level `_1525` tests for the
    /// full-invocation equivalents).
    mod seq_no_rs_byte_warning_tests {
        use super::*;

        fn warning(sources: &[&[u8]]) -> Option<String> {
            let raw_bytes: Vec<(Option<usize>, Vec<u8>)> = sources
                .iter()
                .enumerate()
                .map(|(i, s)| (Some(i), s.to_vec()))
                .collect();
            seq_no_rs_byte_warning(&raw_bytes)
        }

        #[test]
        fn rs_byte_anywhere_suppresses_the_warning() {
            assert_eq!(warning(&[b"\x1e1\n"]), None);
            assert_eq!(warning(&[b"not valid json"]), Some(
                "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 14"
                    .to_string()
            ));
            // RS in a *later* source still suppresses it for the whole stream.
            assert_eq!(warning(&[b"junk", b"\x1e1\n"]), None);
        }

        #[test]
        fn no_sources_or_all_empty_sources() {
            assert_eq!(warning(&[]), None);
            assert_eq!(
                warning(&[b""]),
                Some(
                    "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 0"
                        .to_string()
                )
            );
            assert_eq!(
                warning(&[b"", b""]),
                Some(
                    "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 0"
                        .to_string()
                )
            );
        }

        #[test]
        fn line_and_column_span_multiple_sources() {
            // "ab" + "cd" concatenated -> one line, column 4.
            assert_eq!(
                warning(&[b"ab", b"cd"]),
                Some(
                    "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 4"
                        .to_string()
                )
            );
            // A newline crossing a source boundary still counts correctly.
            assert_eq!(
                warning(&[b"ab\n", b"cd"]),
                Some(
                    "jq: ignoring parse error: Unfinished abandoned text at EOF at line 2, column 2"
                        .to_string()
                )
            );
        }

        #[test]
        fn leading_bom_is_stripped_even_after_an_empty_source() {
            const BOM: &[u8] = b"\xEF\xBB\xBF";
            // BOM as the very first source.
            assert_eq!(
                warning(&[&[BOM, b"1 2"].concat()]),
                Some(
                    "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 3"
                        .to_string()
                )
            );
            // Regression: an empty leading source must not stop the BOM in
            // the next source from being recognized as the stream's own
            // first bytes.
            assert_eq!(
                warning(&[b"", &[BOM, b"1 2"].concat()]),
                Some(
                    "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 3"
                        .to_string()
                )
            );
            // A BOM-*shaped* byte sequence appearing after real content is
            // just ordinary bytes, not stripped.
            assert_eq!(
                warning(&[&[b"x", BOM].concat()]),
                Some(
                    "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 4"
                        .to_string()
                )
            );
        }
    }

    /// #723: direct unit coverage for the TTY-safety narrowing, independent
    /// of a real terminal (which `cargo test` never has one of anyway).
    /// Only the exact combination -- `-n`, no files, an interactive stdin --
    /// should suppress the forced real read; every other combination (any
    /// one of the three false) must force it whenever `uses_input_builtins`
    /// says the filter might need it.
    #[test]
    fn test_should_force_read_under_null_input_723() {
        // Never forces anything when the filter doesn't reference these
        // builtins at all, regardless of the other three inputs.
        assert!(!should_force_read_under_null_input(false, true, true, true));
        assert!(!should_force_read_under_null_input(
            false, false, false, false
        ));

        // Not under -n at all: always forces (there's no hang risk --
        // non-null-input mode already reads stdin/files unconditionally).
        assert!(should_force_read_under_null_input(true, false, true, true));
        assert!(should_force_read_under_null_input(
            true, false, false, false
        ));

        // Under -n with files given: forces (files can't block like a bare
        // TTY read can).
        assert!(should_force_read_under_null_input(true, true, false, true));

        // Under -n with no files but stdin isn't a terminal (piped/redirected):
        // forces -- this is the canonical `jq -n 'reduce inputs as $x (...)'`
        // streaming-aggregation case.
        assert!(should_force_read_under_null_input(true, true, true, false));

        // The one suppressed combination: -n, no files, interactive stdin.
        assert!(!should_force_read_under_null_input(true, true, true, true));
    }

    /// #1371: `rewrite_namespaced_calls` runs once, on the freshly parsed
    /// program, strictly before evaluation ever builds an `Expr::Shared` or
    /// `Expr::DefCall` -- so this pass can never actually receive one from
    /// its real caller. Named rather than wildcarded (see the arm's own
    /// comment), so exercised directly here: each node must come back
    /// unchanged, not e.g. have its nested args silently dropped or
    /// rewritten as if it were some other variant.
    #[test]
    fn test_rewrite_namespaced_calls_passes_through_shared_and_defcall_1371() {
        use std::rc::Rc;

        let shared = Expr::shared(Expr::Identity);
        assert_eq!(rewrite_namespaced_calls(shared.clone()), shared);

        let def_call = Expr::DefCall {
            def: Rc::new(jq::FuncDefData {
                name: "f".to_string(),
                params: Vec::new(),
                body: Expr::Identity,
            }),
            args: vec![Expr::Literal(jq::Literal::Int(1))],
            frames: 3,
            bound: jq::BoundBody::default(),
        };
        assert_eq!(rewrite_namespaced_calls(def_call.clone()), def_call);
    }

    #[test]
    fn test_jq_compat_formatter_format_raw_number() {
        // Finite numbers fall through the NaN/Infinity guard unchanged.
        assert_eq!(JqCompatFormatter.format_raw_number(b"42").as_ref(), "42");
        // Overflowed literals echo `format_number_jq_compat`'s own
        // mantissa-preserving reformat, not "null" (#1087 -- #561's original
        // premise, that this function "assumes a finite value," stopped
        // holding once #930 gave it a non-finite special case;
        // confirmed live against jq 1.7.1, `1e400 | .` echoes `1E+400`).
        assert_eq!(
            JqCompatFormatter.format_raw_number(b"1e400").as_ref(),
            "1E+400"
        );
        assert_eq!(
            JqCompatFormatter.format_raw_number(b"-1e400").as_ref(),
            "-1E+400"
        );
    }

    #[test]
    fn test_line_at() {
        // Single value, no trailing newline: jq's counter never advances (#524).
        let bytes = br#"{"a":1}"#;
        assert_eq!(line_at(bytes, bytes.len()), 0);

        // Multi-value, no trailing newline after the last value: both values
        // report the same line jq does, not "line of value + 1" (#524).
        let bytes = b"1\n2";
        let ends = find_json_values(bytes).unwrap();
        assert_eq!(ends, vec![(0, 1), (2, 3)]);
        assert_eq!(line_at(bytes, 1), 1); // "1" ends before the '\n' lookahead
        assert_eq!(line_at(bytes, 3), 1); // "2" ends at EOF, no lookahead byte

        // Container value spanning multiple lines, no trailing newline: jq
        // names the line the value's closing brace is on, not one past it
        // (#524 -- the naive "1 + newlines-before-end" formula overcounts).
        let bytes = b"{\n\"a\":1\n}";
        assert_eq!(line_at(bytes, bytes.len()), 2);

        // With a trailing newline after every value, the lookahead-aware
        // formula agrees with plain "count of newlines up to and including
        // the delimiter" -- unaffected by this fix.
        let bytes = b"1\n2\n";
        assert_eq!(line_at(bytes, 1), 1);
        assert_eq!(line_at(bytes, 3), 2);
    }

    /// #1213: `LineCounter::advance_to` must return exactly what `line_at`
    /// would for the same offset, for every offset in a monotonic sequence
    /// -- the whole point of introducing it is replacing a hot loop's
    /// repeated `line_at` calls without changing what line number any value
    /// gets reported at.
    #[test]
    fn test_line_counter_matches_line_at_for_monotonic_sequence_1213() {
        let bytes = b"1\n2\n{\n\"a\":1\n}\n3";
        let ends: Vec<usize> = find_json_values(bytes)
            .unwrap()
            .into_iter()
            .map(|(_, end)| end)
            .collect();

        let mut counter = LineCounter::new(bytes);
        for &end in &ends {
            assert_eq!(counter.advance_to(end), line_at(bytes, end), "end={end}");
        }
    }

    /// A value with no `\n` bytes between it and the previous one (adjacent
    /// values with no separator between them, e.g. `12` split into `1` then
    /// `2` isn't valid, but two values on the same line with only a space
    /// between them is) still gets the correct, un-advanced line number --
    /// `advance_to`'s internal scan window can be empty.
    #[test]
    fn test_line_counter_same_line_consecutive_values_1213() {
        let bytes = b"1 2\n3";
        let mut counter = LineCounter::new(bytes);
        assert_eq!(counter.advance_to(1), 0); // "1" ends before any '\n'
        assert_eq!(counter.advance_to(3), 1); // "2" ends right before the '\n'
        assert_eq!(counter.advance_to(5), 1); // "3" ends at EOF, no lookahead
    }

    #[test]
    fn test_trim_ascii_ws() {
        assert_eq!(trim_ascii_ws(b"  x  "), b"x");
        assert_eq!(trim_ascii_ws(b"abc"), b"abc");
        assert_eq!(trim_ascii_ws(b"\t\n false \r\n"), b"false");
        assert_eq!(trim_ascii_ws(b"   "), b"");
        assert_eq!(trim_ascii_ws(b""), b"");
    }

    #[test]
    fn test_identity_exit_status_value() {
        // Only the bare `null`/`false` literals (ignoring surrounding
        // whitespace) are falsy on the identity fast path.
        assert_eq!(identity_exit_status_value(b"null"), OwnedValue::Null);
        assert_eq!(identity_exit_status_value(b"  null\n"), OwnedValue::Null);
        assert_eq!(
            identity_exit_status_value(b"false"),
            OwnedValue::Bool(false)
        );
        assert_eq!(identity_exit_status_value(b"true"), OwnedValue::Bool(true));
        assert_eq!(identity_exit_status_value(b"0"), OwnedValue::Bool(true));
        // Quoted strings are truthy, not the falsy literals.
        assert_eq!(
            identity_exit_status_value(b"\"false\""),
            OwnedValue::Bool(true)
        );
    }

    #[test]
    fn test_parse_json_value() {
        assert!(matches!(
            parse_json_value("null").unwrap(),
            OwnedValue::Null
        ));
        assert!(matches!(
            parse_json_value("true").unwrap(),
            OwnedValue::Bool(true)
        ));
        // A whole number still round-trips its value (#1058: no longer a
        // bare `OwnedValue::Int` -- `parse_json_value` now goes through the
        // same fidelity-preserving semi-indexer as document input, which
        // always reconstructs `NumberLiteral` for valid number text).
        assert_eq!(parse_json_value("42").unwrap().to_json(), "42");
        assert!(matches!(
            parse_json_value("\"hello\"").unwrap(),
            OwnedValue::String(_)
        ));
    }

    /// #1058: `--argjson`/`--jsonargs`' number literals preserve their exact
    /// source spelling, matching a filter-embedded literal (#1035) and a
    /// document-sourced one -- previously lost via a `serde_json::Value`
    /// round-trip through Rust's own `f64`/`i64` `Display`. Verified against
    /// the pinned real `jq` binary, which also preserves these exactly.
    #[test]
    fn test_parse_json_value_preserves_number_literal_fidelity_1058() {
        assert_eq!(parse_json_value("1.500").unwrap().to_json(), "1.500");
        assert_eq!(parse_json_value("1e100").unwrap().to_json(), "1E+100");
        assert_eq!(
            parse_json_value(r#"{"a": 1.500, "b": [1e100, 2.0]}"#)
                .unwrap()
                .to_json(),
            r#"{"a":1.500,"b":[1E+100,2.0]}"#
        );
    }

    /// #1058 regression guard: preserving fidelity must not weaken the
    /// existing strict-validation behavior (#284) -- trailing garbage after
    /// a complete JSON value is still rejected, not silently truncated to
    /// just the leading value the way `JsonIndex`'s own lenient semi-index
    /// would accept on its own.
    #[test]
    fn test_parse_json_value_still_rejects_trailing_garbage_1058() {
        assert!(parse_json_value("42 garbage").is_err());
    }

    /// #2012: `--argjson`/`--jsonargs`' JSON parser accepts a lone low
    /// surrogate escape and substitutes U+FFFD, matching real jq -- rather
    /// than rejecting outright the way `serde_json`'s own validation gate
    /// does on its own. Verified live against jq 1.7.1
    /// (`jq -n --argjson x '"\udc00"' '$x'` => `"�"`, exit 0).
    #[test]
    fn test_parse_json_value_accepts_lone_low_surrogate_2012() {
        assert_eq!(
            parse_json_value(r#""\udc00""#).unwrap().to_json(),
            "\"\u{FFFD}\""
        );
        // Multiple lone low surrogates in the same value.
        assert_eq!(
            parse_json_value(r#""a\udc00b\udc01""#).unwrap().to_json(),
            "\"a\u{FFFD}b\u{FFFD}\""
        );
        // A valid surrogate pair still decodes to the real supplementary
        // character, not U+FFFD -- this retry must not clobber it.
        assert_eq!(
            parse_json_value(r#""𐀀""#).unwrap().to_json(),
            "\"\u{10000}\""
        );
        // Reached through an object value too, not just a bare string.
        assert_eq!(
            parse_json_value(r#"{"a":"\udc00"}"#).unwrap().to_json(),
            "{\"a\":\"\u{FFFD}\"}"
        );
    }

    /// #2012 regression guard: a lone *high* surrogate must stay rejected
    /// -- that's #2013's own (different-decoder) scope, not this fix's.
    /// Verified live: `jq -n --argjson x '"\ud800"' '$x'` errors, exit 2.
    #[test]
    fn test_parse_json_value_still_rejects_lone_high_surrogate_2012() {
        assert!(parse_json_value(r#""\ud800""#).is_err());
    }

    /// #2012 code review: a value needing *both* the #1094 leading-zero
    /// leniency and the #2012 lone-low-surrogate leniency at once must
    /// still be accepted -- an earlier version of this fix tried each
    /// normalization independently against the untouched original, so a
    /// value needing both failed both retries and was wrongly rejected.
    /// Verified live: `jq -n --argjson x '[007,"\udc00"]' '$x'` =>
    /// `[7,"�"]`, exit 0.
    #[test]
    fn test_parse_json_value_composes_leading_zero_and_low_surrogate_fixes_2012() {
        assert_eq!(
            parse_json_value(r#"[007,"\udc00"]"#).unwrap().to_json(),
            "[7,\"\u{FFFD}\"]"
        );
        // Nested inside an object too, not just a top-level array.
        assert_eq!(
            parse_json_value(r#"{"a":007,"b":{"c":"\udc00"}}"#)
                .unwrap()
                .to_json(),
            "{\"a\":7,\"b\":{\"c\":\"\u{FFFD}\"}}"
        );
    }

    #[test]
    fn split_json_values_keeps_the_prefix_before_a_failure_2961() {
        let bytes = b"1 [2] {\"a\":3} [4,";
        let (spans, error) = split_json_values(bytes);
        assert_eq!(spans, vec![(0, 1), (2, 5), (6, 13)]);
        assert_eq!(error, Some(14));
        assert_eq!(find_json_values(bytes), Err(14));

        let (spans, error) = split_json_values(b" 1 2 ");
        assert_eq!(spans, vec![(1, 2), (3, 4)]);
        assert_eq!(error, None);
    }

    #[test]
    fn parse_json_stream_prefix_stops_at_the_first_malformed_value_2961() {
        let show = |(values, error): (Vec<OwnedValue>, Option<EvalError>)| {
            (
                values.iter().map(OwnedValue::to_json).collect::<Vec<_>>(),
                error.map(|e| e.message),
            )
        };
        // A clean stream, including a leniency only the fallback accepts.
        assert_eq!(
            show(parse_json_stream_prefix("1 007 \"x\"")),
            (vec!["1".into(), "7".into(), "\"x\"".into()], None)
        );
        assert_eq!(show(parse_json_stream_prefix("  ")), (vec![], None));
        // A splitter failure: truncated, unterminated, unmatched.
        for text in ["1 2 [3,", "1 2 \"abc", "1 2 }"] {
            let (values, error) = show(parse_json_stream_prefix(text));
            assert_eq!(values, vec!["1", "2"], "{text}");
            assert_eq!(error.as_deref(), Some("Invalid JSON text"), "{text}");
        }
        // A span the splitter accepts but the materializer rejects: the
        // prefix ends there, and nothing after it is kept.
        let (values, error) = show(parse_json_stream_prefix("1 2 {invalid} 3"));
        assert_eq!(values, vec!["1", "2"]);
        assert!(error.is_some());
    }

    #[test]
    fn test_parse_json_stream() {
        let stream = r#"{"a":1} {"b":2} {"c":3}"#;
        let values = parse_json_stream(stream).unwrap();
        assert_eq!(values.len(), 3);
    }

    /// #1243: a leading-zero number token in a `--slurpfile`/`--slurp`
    /// stream no longer errors outright -- `serde_json::Deserializer`
    /// rejects it, but the fallback through `find_json_values` (which
    /// already tolerates a leading zero, same as `#1094` established for
    /// the primary document-input path) accepts it and materializes with
    /// full source-spelling fidelity, matching real jq's own `7E+5`.
    #[test]
    fn test_parse_json_stream_tolerates_leading_zero_1243() {
        let values = parse_json_stream("007e5").unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].to_json(), "7E+5");
    }

    /// A leading zero anywhere in a multi-value stream is tolerated, not
    /// just when it's the only value -- and every other value keeps its
    /// own exact spelling untouched.
    #[test]
    fn test_parse_json_stream_tolerates_leading_zero_among_other_values_1243() {
        let values = parse_json_stream("42 007e5 \"hi\"").unwrap();
        assert_eq!(values.len(), 3);
        assert_eq!(values[0].to_json(), "42");
        assert_eq!(values[1].to_json(), "7E+5");
        assert_eq!(values[2].to_json(), "\"hi\"");
    }

    /// Control: genuinely malformed input (not just a leading zero) still
    /// errors -- the fallback doesn't silently widen into general leniency.
    #[test]
    fn test_parse_json_stream_still_rejects_genuine_malformed_input_1243() {
        assert!(parse_json_stream("{invalid").is_err());
    }

    /// `build_seq_values` over string sources, wired the way `get_inputs`
    /// wires it: the raw walk's value ranges (#3247), non-slurp.
    fn seq_build(
        sources: &[(Option<usize>, String)],
        locations: &mut InputLocations,
    ) -> Vec<OwnedValue> {
        let raw: Vec<(Option<usize>, Vec<u8>)> = sources
            .iter()
            .map(|(idx, s)| (*idx, s.as_bytes().to_vec()))
            .collect();
        let walk = crate::jq_seq_reader::walk_stream(&raw, false, &mut |_| {});
        build_seq_values(&raw, &walk.values, &walk.locations, locations, false)
    }

    /// One `--seq` stream's values, the way `get_inputs` builds them.
    fn parse_seq(stream: &str) -> Vec<OwnedValue> {
        let mut locations = InputLocations::new(vec![None]);
        seq_build(&[(None, stream.to_string())], &mut locations)
    }

    /// #1243: same leading-zero tolerance as `--slurpfile`/`--slurp` above,
    /// for `--seq` (RFC 7464) -- previously silently dropped the whole
    /// record (its own documented "ignore parse failures" fallback) instead
    /// of accepting it the way real jq does.
    #[test]
    fn test_parse_json_seq_tolerates_leading_zero_1243() {
        let values: Vec<OwnedValue> = parse_seq("\x1E007e5\n");
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].to_json(), "7E+5");
    }

    /// Control: a genuinely malformed `--seq` record is still silently
    /// dropped (RFC 7464's own documented behavior), not resurrected by the
    /// leading-zero retry.
    #[test]
    fn test_parse_json_seq_still_drops_genuine_malformed_record_1243() {
        let values: Vec<OwnedValue> = parse_seq("\x1E{invalid\n\x1E5\n");
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].to_json(), "5");
    }

    /// #1267: the crate's own zero-allocation grammar validator has no
    /// `f64`-range rejection, unlike `serde_json::Value` -- so swapping to
    /// it for `--seq`'s per-record validation also fixed a real divergence
    /// from real jq, not just a speed one. `1e400` no longer silently
    /// drops as "unparseable"; it materializes with the same spelling
    /// primary document input already produces for it.
    #[test]
    fn test_parse_json_seq_accepts_magnitude_overflowing_number_1267() {
        let values: Vec<OwnedValue> = parse_seq("\x1E1e400\n");
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].to_json(), "1E+400");
    }

    /// #1571: a record's opening RS byte and closing bytes can live in
    /// different files -- `parse_json_seq` running once per file in
    /// isolation drops both independently-malformed halves. `build_seq_values`
    /// treats the whole file list as one continuous byte stream instead,
    /// matching real jq's own `-s`/`--seq` reader.
    #[test]
    fn test_build_seq_values_reassembles_across_file_boundary_1571() {
        let raw_inputs = vec![
            (Some(0), "\x1E1\n\x1E{\"a\":\"unterminated ".to_string()),
            (Some(1), "str\"}\n".to_string()),
        ];
        let mut locations =
            InputLocations::new(vec![Some("f1".to_string()), Some("f2".to_string())]);
        let values = seq_build(&raw_inputs, &mut locations);
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].to_json(), "1");
        assert_eq!(values[1].to_json(), "{\"a\":\"unterminated str\"}");
        assert_eq!(locations.len(), 2, "one location per value, non-slurp");
        // `1` lives entirely in f1, on its own line 1 (matches real jq
        // 1.7.1's own `input_line_number` for this exact byte layout,
        // verified live); the reassembled record's own *end* falls in f2,
        // so it's attributed there -- matching #1568's own file-
        // attribution rule for a boundary-spanning record. Not just a
        // location count -- the actual file and line each value reports
        // (#1808 code review: a prior version only checked
        // `locations.len()`, missing that both values were silently
        // misattributed to the *last* file whenever a mismatch fired
        // anywhere in the whole stream).
        assert_eq!(locations.get(0).file.as_deref(), Some("f1"));
        assert_eq!(locations.get(0).line, Some(1));
        assert_eq!(locations.get(1).file.as_deref(), Some("f2"));
        assert_eq!(locations.get(1).line, Some(1));
    }

    /// #1808 code review: a malformed record anywhere in the multi-file
    /// stream must not degrade *other* files' own precise locations --
    /// only the earlier design (reconciling two separately-scanned
    /// end/value lists by comparing counts) had this failure mode, and it
    /// applied to the *whole* stream on any single drop, not just the
    /// offending file. `f1` has a real value followed by a malformed
    /// record; `f2`'s own value must still resolve to `f2`'s own line, not
    /// silently fall back to `f1`'s.
    #[test]
    fn test_build_seq_values_malformed_record_does_not_misattribute_other_files_1808() {
        let raw_inputs = vec![
            (Some(0), "\x1E1\n\x1E{bad\n".to_string()),
            (Some(1), "\x1E2\n".to_string()),
        ];
        let mut locations =
            InputLocations::new(vec![Some("f1".to_string()), Some("f2".to_string())]);
        let values = seq_build(&raw_inputs, &mut locations);
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].to_json(), "1");
        assert_eq!(values[1].to_json(), "2");
        assert_eq!(locations.get(0).file.as_deref(), Some("f1"));
        assert_eq!(locations.get(0).line, Some(1));
        assert_eq!(
            locations.get(1).file.as_deref(),
            Some("f2"),
            "f2's own value must not be misattributed to f1 just because f1 dropped a record"
        );
        assert_eq!(locations.get(1).line, Some(1));
    }

    /// A record spanning three files, not just two.
    #[test]
    fn test_build_seq_values_reassembles_across_three_files_1571() {
        let raw_inputs = vec![
            (Some(0), "\x1E1\n\x1E{\"a\":\"unter".to_string()),
            (Some(1), "minated ".to_string()),
            (Some(2), "str\"}\n\x1E9\n".to_string()),
        ];
        let mut locations = InputLocations::new(vec![
            Some("f1".to_string()),
            Some("f2".to_string()),
            Some("f3".to_string()),
        ]);
        let values = seq_build(&raw_inputs, &mut locations);
        assert_eq!(values.len(), 3);
        assert_eq!(values[0].to_json(), "1");
        assert_eq!(values[1].to_json(), "{\"a\":\"unterminated str\"}");
        assert_eq!(values[2].to_json(), "9");
    }

    /// Control: a record genuinely malformed on its own, not at a file
    /// boundary, must still be dropped -- reassembly must not paper over
    /// real malformation just because multiple files are involved.
    #[test]
    fn test_build_seq_values_still_drops_genuinely_malformed_record_1571() {
        let raw_inputs = vec![(Some(0), "\x1E1\n\x1E{invalid\n\x1E3\n".to_string())];
        let mut locations = InputLocations::new(vec![Some("f1".to_string())]);
        let values = seq_build(&raw_inputs, &mut locations);
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].to_json(), "1");
        assert_eq!(values[1].to_json(), "3");
    }

    /// #1192: `standard_json_to_jq_value` now surfaces a genuinely
    /// undecodable top-level string as an `EvalError` instead of silently
    /// substituting an empty string. Array/object children stay lazy
    /// (`JqValue::Cursor`) here, so only a decode failure at the immediate
    /// top level is caught by this function itself -- a nested one surfaces
    /// later, if and when that child cursor is materialized.
    ///
    /// No CLI-level regression test accompanies this: this function
    /// converts a query *result*, not the input document, and every
    /// ordinary top-level jq/yq expression tried against a document
    /// containing this malformed byte sequence resolves its result through
    /// `to_owned`/`cursor_to_owned` (`eval_generic.rs`/`lazy.rs`, unaffected
    /// by this fix) before ever reaching here -- see
    /// `test_owned_from_standard_json_errors_on_string_decode_failure_1192`
    /// in `eval_generic.rs` for the full account of what was tried.
    #[test]
    fn test_standard_json_to_jq_value_errors_on_string_decode_failure_1192() {
        let json: &[u8] = b"\"\xff\xfe\"";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let err = standard_json_to_jq_value(value, &cursor).unwrap_err();
        assert!(err.message.contains("invalid UTF-8"), "{err:?}");
    }

    /// #1192: an immediate object key that fails to decode now errors too,
    /// instead of silently substituting an empty-string key -- which used
    /// to collide multiple decode-failing keys together into one field.
    #[test]
    fn test_standard_json_to_jq_value_errors_on_object_key_decode_failure_1192() {
        let json: &[u8] = b"{\"\xff\xfe\": 1, \"b\": 2}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let err = standard_json_to_jq_value(value, &cursor).unwrap_err();
        assert_eq!(err.message, "invalid UTF-8 in string in object key");
        assert!(err.is_decode_failure(), "{err:?}");
    }

    /// #1192: the `Ok` side of `standard_json_to_jq_value`'s string arm and
    /// its object-key handling -- the decode-*failure* tests above only
    /// exercise the `Err` side of each.
    #[test]
    fn test_standard_json_to_jq_value_succeeds_on_valid_input_1192() {
        let json: &[u8] = b"\"hello\"";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let jq_value = standard_json_to_jq_value(value, &cursor).unwrap();
        assert!(matches!(jq_value, JqValue::String(s) if s == "hello"));

        let json: &[u8] = br#"{"a": 1, "b\u0063": 2}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let jq_value = standard_json_to_jq_value(value, &cursor).unwrap();
        let JqValue::Object(map) = jq_value else {
            panic!("expected an object");
        };
        assert_eq!(map.keys().collect::<Vec<_>>(), vec!["a", "bc"]);
    }

    /// #3034: `standard_json_to_jq_value`'s number arm decodes a reindex
    /// bridge token to the float it stands for, rather than handing back the
    /// raw span as `JqValue::RawNumber`.
    ///
    /// No production call site builds this function's `cursor` over
    /// `JsonIndex::build_reindex` today -- `evaluate_bytes_streaming`, the
    /// only caller reachable from ordinary CLI usage, always indexes an
    /// actual user document with the plain `JsonIndex::build` (see its own
    /// doc comment), so a bridge token there is deliberately read back as
    /// ordinary malformed text (`test_bridge_tokens_decode_only_under_the_
    /// bridge_index_3034` in `src/json/light.rs` covers that half). This
    /// pins the arm directly so it doesn't silently bit-rot if a future
    /// caller does route bridge text through here.
    #[test]
    fn test_standard_json_to_jq_value_decodes_bridge_token_3034() {
        let token = OwnedValue::Float(f64::NAN).to_json_input_bridge();
        let json = format!("[{token}]");
        let bytes = json.as_bytes();
        let index = JsonIndex::build_reindex(bytes);
        let cursor = index.root(bytes).first_child().expect("one element");
        let jq_value = standard_json_to_jq_value(cursor.value(), &cursor).unwrap();
        assert!(
            matches!(jq_value, JqValue::Float(f) if f.is_nan()),
            "{token}"
        );
    }

    /// #1194: a key that isn't `StandardJson::String` at all (structurally
    /// malformed, not a decode failure) raises instead of dropping the field.
    ///
    /// Inverted from the drop this asserted when #1192 wrote it, in step with
    /// `eval_generic.rs`'s `test_owned_from_standard_json_raises_on_malformed_key_1194`
    /// -- these two are textually-similar copies of the same conversion, and a
    /// fix that moved only one would leave the CLI and the evaluator
    /// disagreeing about whether the document is valid.
    #[test]
    fn test_standard_json_to_jq_value_raises_on_malformed_key_1194() {
        let json: &[u8] = b"{123: 1, \"b\": 2}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let err =
            standard_json_to_jq_value(value, &cursor).expect_err("a bare numeric key is not JSON");
        assert!(
            err.message.contains("expected string key"),
            "message: {}",
            err.message
        );
    }

    /// #1194: an object whose children don't pair raises rather than
    /// materializing as `{}`. The `unpaired_tail` half of the check above.
    #[test]
    fn test_standard_json_to_jq_value_raises_on_unpaired_field_1194() {
        let json: &[u8] = b"{invalid}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let err =
            standard_json_to_jq_value(value, &cursor).expect_err("an unpaired member is not JSON");
        assert!(
            err.message.contains("Invalid JSON text"),
            "message: {}",
            err.message
        );
    }

    /// #1194: a top-level query *result* that is itself a bareword garbage
    /// token (`StandardJson::Error`, not a decode failure) raises instead of
    /// silently printing `null` -- the same class of fix as the malformed-key
    /// and unpaired-field cases above, for the array/object match arm's own
    /// fallthrough rather than either of their more specific checks. No
    /// CLI-level test accompanies this for the same reason given on this
    /// function's own doc comment: an ordinary `[xyz123] | to_entries`-style
    /// filter resolves its result through `to_owned` (`eval_generic.rs`)
    /// before it ever reaches this lazy `JqValue` conversion, so reaching
    /// this exact arm needs a result cursor built directly, as below.
    #[test]
    fn test_standard_json_to_jq_value_raises_on_malformed_top_level_value_1194() {
        let json: &[u8] = b"xyz123";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let err =
            standard_json_to_jq_value(value, &cursor).expect_err("a bareword is not a JSON value");
        assert!(!err.message.is_empty(), "{err:?}");
    }

    /// #2211 code review: this function's `Array`/`Object` arms never
    /// validated *any* delimiter at all -- not even the older #1677
    /// missing/doubled-comma-between-two-real-children check `print_json`'s
    /// own array/object arms and `eval_generic::to_owned_cursor_at_depth`
    /// already perform. `[1 2, 3]` is missing the comma between its first
    /// two elements (three real elements: `1`, `2`, `3`); `{"a" 1, "b": 2}`
    /// is missing the colon after its first key.
    ///
    /// No CLI-level regression test accompanies either half of this, same
    /// reasoning as this function's other unit-tested-only fixes above
    /// (see e.g. `test_standard_json_to_jq_value_raises_on_malformed_top_
    /// level_value_1194`'s own doc comment): whenever this function's
    /// output is actually printed, `print_json`'s own independent,
    /// pre-existing #1643/#1676 checks re-validate the same document
    /// (confirmed live against the pre-fix binary via `git stash`) and mask
    /// this function's own gap end-to-end -- calling this function
    /// directly is the only way to exercise its own validation in
    /// isolation.
    #[test]
    fn test_standard_json_to_jq_value_raises_on_missing_delimiter_between_array_elements_2211() {
        let json: &[u8] = b"[1 2, 3]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let err = standard_json_to_jq_value(value, &cursor)
            .expect_err("a missing comma between two real elements is not JSON");
        assert!(
            err.message.contains("Invalid JSON text"),
            "message: {}",
            err.message
        );
    }

    #[test]
    fn test_standard_json_to_jq_value_raises_on_missing_delimiter_in_object_2211() {
        let json: &[u8] = b"{\"a\" 1, \"b\": 2}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let err = standard_json_to_jq_value(value, &cursor)
            .expect_err("a missing colon after a real key is not JSON");
        assert!(
            err.message.contains("Invalid JSON text"),
            "message: {}",
            err.message
        );
    }

    /// #2211: the sibling half of the object check above -- a missing comma
    /// between two real *fields* (rather than a missing colon after one
    /// key) is caught by the second field's own `key_cursor()` delimiter
    /// check, not the first field's `value_cursor()` one.
    #[test]
    fn test_standard_json_to_jq_value_raises_on_missing_comma_between_object_fields_2211() {
        let json: &[u8] = b"{\"a\":1 \"b\":2}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let err = standard_json_to_jq_value(value, &cursor)
            .expect_err("a missing comma between two real fields is not JSON");
        assert!(
            err.message.contains("Invalid JSON text"),
            "message: {}",
            err.message
        );
    }

    /// #2211: well-formed multi-element arrays/objects are unaffected by
    /// the new between-elements delimiter check above --
    /// `test_standard_json_to_jq_value_succeeds_on_valid_input_1192`'s own
    /// object case only has two well-formed fields; this exercises a
    /// three-element well-formed array too, so the `is_first`/subsequent-
    /// element branches of the new check both get a `true` answer pinned.
    #[test]
    fn test_standard_json_to_jq_value_wellformed_array_unaffected_2211() {
        let json: &[u8] = b"[1, 2, 3]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let jq_value =
            standard_json_to_jq_value(value, &cursor).expect("a well-formed array must not raise");
        let JqValue::Array(items) = jq_value else {
            panic!("expected an array");
        };
        assert_eq!(items.len(), 3);
    }

    /// #1194: `MalformedJsonError` exists to be `downcast_ref`'d out of an
    /// `anyhow::Error` in `run_jq`, which reads its inner `EvalError` and
    /// never formats the wrapper. `Display` is still required by
    /// `std::error::Error`, so pin that it renders the message rather than
    /// something like `MalformedJsonError(..)` -- a future `anyhow` context
    /// chain would print it.
    #[test]
    fn test_malformed_json_error_displays_its_message_1194() {
        let wrapped = MalformedJsonError::new(EvalError::new("Invalid JSON text: whatever"));
        assert_eq!(wrapped.to_string(), "Invalid JSON text: whatever");

        // And it survives the round trip `run_jq` actually performs.
        let boxed: anyhow::Error = wrapped.into();
        let recovered = boxed
            .downcast_ref::<MalformedJsonError>()
            .expect("run_jq recovers this by downcast");
        assert_eq!(
            recovered.to_eval_error().message,
            "Invalid JSON text: whatever"
        );
    }

    /// #1194: the defensive arm of `EvalError::malformed_json_text`.
    ///
    /// It re-reads the document with the strict validator to name the error.
    /// If the validator ever *disagrees* -- says the document is fine after a
    /// swallow point has already fired -- the two layers have drifted apart,
    /// and the right answer is still an error, not silence. Not reachable
    /// through the CLI today, which is exactly why it is pinned here.
    #[test]
    fn test_malformed_json_text_still_errors_when_validator_disagrees_1194() {
        let err = EvalError::malformed_json_text(br#"{"a":1}"#);
        assert_eq!(err.message, "Invalid JSON text");

        // The normal arm, for contrast: the validator's own reason is used.
        let err = EvalError::malformed_json_text(b"{invalid}");
        assert!(
            err.message.contains("expected string key"),
            "message: {}",
            err.message
        );
    }

    /// #3009 corpus: every owned value shape whose printing could plausibly
    /// differ between the rebuild route and the direct one.
    ///
    /// Number spellings go through `parse_json_value` -- the `--argjson`
    /// entry point -- rather than being hand-assembled as
    /// `NumberLiteral(repr, text)` pairs. A hand-built pair can encode a
    /// `(repr, text)` combination no evaluator would ever produce, which
    /// would make the differential test below prove something about values
    /// that cannot occur while missing ones that can. The spellings
    /// themselves are lifted from the two suites that already pin them:
    /// `test_argjson_accept_set_matches_jq_2052` and
    /// `test_jq_number_spellings_every_input_path_2877`.
    #[cfg(test)]
    fn owned_print_corpus_3009() -> Vec<OwnedValue> {
        let mut out = vec![
            OwnedValue::Null,
            OwnedValue::Bool(true),
            OwnedValue::Bool(false),
            OwnedValue::Int(0),
            OwnedValue::Int(-1),
            OwnedValue::Int(i64::MIN),
            OwnedValue::Int(i64::MAX),
            // Computed floats: no spelling of their own, which is exactly
            // how the evaluator hands a `Float` to the printer.
            OwnedValue::Float(0.0),
            OwnedValue::Float(-0.0),
            OwnedValue::Float(1.0),
            OwnedValue::Float(0.1),
            OwnedValue::Float(1e17),
            OwnedValue::Float(1e18),
            OwnedValue::Float(f64::MAX),
            OwnedValue::Float(5e-324),
            // The three the `_2877` pins care about -- and the three
            // `stream.rs`'s owned writer gets wrong, which is why this
            // change could not simply reuse it.
            OwnedValue::Float(f64::INFINITY),
            OwnedValue::Float(f64::NEG_INFINITY),
            OwnedValue::Float(f64::NAN),
            OwnedValue::String(String::new()),
            OwnedValue::String("a".to_string()),
            OwnedValue::String("caf\u{e9}".to_string()),
            OwnedValue::String("\u{65e5}\u{672c}".to_string()),
            // Astral plane: a surrogate pair once `-a` escapes it.
            OwnedValue::String("\u{1f600}".to_string()),
            OwnedValue::String("\u{7f}".to_string()),
            // Drives `reject_raw_output0_nul` -- both writers must refuse
            // it identically under `--raw-output0`, not just print it the
            // same way.
            OwnedValue::String("a\0b".to_string()),
            // Must stay a quoted string, never become a number.
            OwnedValue::String("1e400".to_string()),
            OwnedValue::String("\"\\/\u{8}\u{c}\n\r\t".to_string()),
            OwnedValue::array_from(vec![]),
            OwnedValue::object_from([]),
            OwnedValue::array_from(vec![OwnedValue::Null]),
            OwnedValue::array_from(vec![OwnedValue::array_from(vec![OwnedValue::array_from(
                vec![],
            )])]),
            OwnedValue::object_from([("a".to_string(), OwnedValue::object_from([]))]),
            // Insertion order deliberately not sorted order, so `-S` has
            // something to reorder.
            OwnedValue::object_from([
                ("b".to_string(), OwnedValue::Int(1)),
                ("a".to_string(), OwnedValue::Int(2)),
                ("C".to_string(), OwnedValue::Int(3)),
            ]),
            OwnedValue::object_from([
                ("caf\u{e9}".to_string(), OwnedValue::Int(1)),
                ("with\"quote".to_string(), OwnedValue::Int(2)),
                ("with\u{7f}del".to_string(), OwnedValue::Int(3)),
            ]),
            // Exercises the `i > 0` comma logic at both ends of a long run.
            OwnedValue::array_from((0..100).map(OwnedValue::Int).collect::<Vec<_>>()),
        ];

        // Every C0 control on its own: the escape tables differ per control
        // and a single wrong byte here is a silent output change.
        for c in 0u8..0x20 {
            out.push(OwnedValue::String((c as char).to_string()));
        }

        for spelling in [
            "1e400",
            "-1e400",
            "1e400000000000",
            "1e999",
            "-1e999",
            "1e-999",
            ".5",
            "-.5",
            "1.e5",
            "1.E5",
            "007",
            "007e5",
            "00",
            "0099999999999999999999999",
            "1.500",
            ".0",
            "-0.0",
            "4e4",
            "1E+009",
            "9223372036854775808",
            "18446744073709551616",
            "99999999999999999",
            "2.7293109604053567083",
            "1.0",
            "123",
        ] {
            out.push(
                parse_json_value(spelling)
                    .unwrap_or_else(|e| panic!("corpus spelling {spelling} must parse: {e}")),
            );
        }

        // One mixed tree, so the container arms are exercised over the
        // scalars above rather than only over `Int`s.
        let scalars = out.iter().take(24).cloned().collect::<Vec<_>>();
        out.push(OwnedValue::object_from([
            ("nested".to_string(), OwnedValue::array_from(scalars)),
            (
                "deep".to_string(),
                OwnedValue::array_from(vec![OwnedValue::object_from([(
                    "k".to_string(),
                    OwnedValue::array_from(vec![OwnedValue::Int(1)]),
                )])]),
            ),
        ]));
        out
    }

    /// #3009: every `OutputConfig` a printed record can actually be written
    /// under.
    ///
    /// `compact` and `indent_string` are varied *together* rather than as an
    /// independent product, because only five of their combinations are
    /// reachable from `OutputConfig::from_args`: `-c` gives `("", true)`,
    /// `--indent 0` gives `("", false)`, and each remaining pretty spelling
    /// gives its own indent with `compact` false.
    #[cfg(test)]
    fn owned_print_configs_3009() -> Vec<OutputConfig> {
        let mut configs = Vec::new();
        for (indent_string, compact) in [
            (String::new(), true),
            // `--indent 0`: an empty indent with `compact` false. Previously
            // excluded here because the two printers disagreed with
            // `format_json` about it; both now key compactness off the indent
            // string as well, matching jq 1.7.1, so this shape is covered
            // like any other (#3155).
            (String::new(), false),
            ("  ".to_string(), false),
            ("    ".to_string(), false),
            ("\t".to_string(), false),
        ] {
            for &(raw_flag, join_output, raw_output0) in &[
                (false, false, false),
                (true, false, false),
                (false, true, false),
                (false, false, true),
            ] {
                for &ascii_output in &[false, true] {
                    for &color_output in &[false, true] {
                        for &sort_keys in &[false, true] {
                            for &seq in &[false, true] {
                                for &convention in
                                    &[JsonConvention::JqCompat, JsonConvention::JqPreserveInput]
                                {
                                    configs.push(OutputConfig {
                                        compact,
                                        // `from_args`' own derivation: `-j`
                                        // and `--raw-output0` imply `-r`.
                                        raw_output: raw_flag || join_output || raw_output0,
                                        join_output,
                                        raw_output0,
                                        ascii_output,
                                        color_output,
                                        color_scheme: ColorScheme::default(),
                                        sort_keys,
                                        indent_string: indent_string.clone(),
                                        // Flush timing only; cannot change a byte.
                                        unbuffered: false,
                                        seq,
                                        convention,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        configs
    }

    /// #3009: the flag spelling of an `OutputConfig`, for a failure message.
    ///
    /// `OutputConfig` deliberately stays `Debug`-free in production (it holds
    /// a `ColorScheme` that would have to grow a derive to suit a test), and
    /// the flags as a user would have typed them localise a mismatch faster
    /// than a struct dump would anyway.
    #[cfg(test)]
    fn describe_config_3009(config: &OutputConfig) -> String {
        let mut flags = Vec::new();
        if config.compact {
            flags.push("-c".to_string());
        } else {
            flags.push(format!("--indent {:?}", config.indent_string));
        }
        for (on, name) in [
            (config.raw_output, "-r"),
            (config.join_output, "-j"),
            (config.raw_output0, "--raw-output0"),
            (config.ascii_output, "-a"),
            (config.color_output, "-C"),
            (config.sort_keys, "-S"),
            (config.seq, "--seq"),
        ] {
            if on {
                flags.push(name.to_string());
            }
        }
        if config.convention.preserves_source_values() {
            flags.push("--preserve-input".to_string());
        }
        flags.join(" ")
    }

    /// #3009: `write_output_owned_value` must be byte-for-byte
    /// indistinguishable from the route it replaces -- printing the same
    /// value after `JqValue::try_from_owned` has rebuilt it.
    ///
    /// This is the load-bearing test of the change. The whole justification
    /// for skipping the rebuild is that it was pure churn, so the obligation
    /// is *identity*, not "valid JSON": any difference at all is a
    /// regression, whichever direction it points. `try_from_owned` therefore
    /// stays in the crate as this test's oracle even after no CLI path calls
    /// it for an owned result.
    ///
    /// It goes in at the **writer** level rather than at `print_owned_json`
    /// vs `print_json`, because `-r`/`-j`/`--raw-output0`, `--seq`'s record
    /// separator, the `-a`-beats-`-r` override, the NUL rejection and the
    /// terminator all live *above* the printer -- a printer-level test would
    /// leave every one of them unproven, and those are exactly the paths
    /// where the two writers' inputs differ in type (`Cow` vs `&str`).
    ///
    /// Errors are compared too, not just bytes: `--raw-output0` on a
    /// NUL-bearing string must fail on both routes with the same message,
    /// and a route that "agrees" by erroring where the other succeeds is not
    /// agreement.
    ///
    /// **What this test cannot police**, recorded because it is not obvious
    /// from the assertion: the two writers now *share*
    /// `write_output_raw_prologue`, so a fault inside it changes both sides
    /// identically and passes here. Negative-tested -- inverting #2662's
    /// pre-override `--seq` keying leaves all 278 bin tests green. That
    /// ordering is guarded instead by
    /// `test_seq_separator_keys_on_pre_ascii_override_raw_shape_2662` in
    /// `tests/jq_cli_tests.rs`, which does catch it. This test's subject is
    /// the *body*: everything below the prologue, where the two routes
    /// genuinely differ.
    ///
    /// **`--indent 0` is covered, and settling it was not optional.** Both
    /// printers used to key compactness off `config.compact` alone, so with
    /// an empty `indent_string` they emitted newlines with zero-width
    /// indents, while `format_json` keyed it off `indent.is_empty()` and
    /// printed compact. jq 1.7.1 agrees with `format_json` (confirmed live).
    /// While the eager `-n`/`--slurp`/DSV route had its own writer this was
    /// a dormant divergence on the lazy route only; merging the two owned
    /// writers would have carried the *wrong* spelling onto the route that
    /// was right, so both printers now key off the indent string too.
    #[test]
    fn write_output_owned_value_matches_rebuilt_route_3009() {
        let corpus = owned_print_corpus_3009();
        let configs = owned_print_configs_3009();
        for value in &corpus {
            let rebuilt = JqValue::<'_, Vec<u64>>::try_from_owned(value.clone())
                .expect("corpus values are all within the depth ceiling");
            for config in &configs {
                let mut direct = Vec::new();
                let direct_err = write_output_owned_value(&mut direct, value, config)
                    .err()
                    .map(|e| e.to_string());

                let mut via_rebuild = Vec::new();
                let rebuild_err = write_output_jq_value(&mut via_rebuild, &rebuilt, config)
                    .err()
                    .map(|e| e.to_string());

                let flags = describe_config_3009(config);
                assert_eq!(
                    direct_err, rebuild_err,
                    "error mismatch for {value:?} under `{flags}`"
                );
                assert_eq!(
                    String::from_utf8_lossy(&direct),
                    String::from_utf8_lossy(&via_rebuild),
                    "byte mismatch for {value:?} under `{flags}`"
                );
            }
        }
    }

    /// #3155: `write_output_owned_value` now also serves the
    /// `-n`/`--slurp`/DSV-input route that used to be a separate
    /// `write_output`, which formatted through `format_json` unconditionally
    /// for every config and never took `print_owned_json`'s fast path. This
    /// proves the fast branch matches that old, always-`format_json` route
    /// byte-for-byte before trusting the merge of the two call sites -- the
    /// same corpus/config sweep as the rebuilt-route test above, replicating
    /// the old function's exact body (shared prologue, `format_json`, shared
    /// terminator) as the oracle since the function itself no longer exists
    /// to call directly.
    #[test]
    fn write_output_owned_value_matches_old_write_output_3155() {
        fn old_write_output<Out: Write>(
            out: &mut Out,
            value: &OwnedValue,
            config: &OutputConfig,
        ) -> Result<()> {
            if write_output_raw_prologue(out, owned_raw_str(value, config), config)? {
                return Ok(());
            }
            out.write_all(format_json(value, config).as_bytes())?;
            write_terminator(out, config)?;
            Ok(())
        }

        let corpus = owned_print_corpus_3009();
        let configs = owned_print_configs_3009();
        for value in &corpus {
            for config in &configs {
                let mut direct = Vec::new();
                let direct_err = write_output_owned_value(&mut direct, value, config)
                    .err()
                    .map(|e| e.to_string());

                let mut via_old = Vec::new();
                let old_err = old_write_output(&mut via_old, value, config)
                    .err()
                    .map(|e| e.to_string());

                let flags = describe_config_3009(config);
                assert_eq!(
                    direct_err, old_err,
                    "error mismatch for {value:?} under `{flags}`"
                );
                assert_eq!(
                    String::from_utf8_lossy(&direct),
                    String::from_utf8_lossy(&via_old),
                    "byte mismatch for {value:?} under `{flags}`"
                );
            }
        }
    }

    /// #3009: `print_owned_json`'s depth `ensure!` is unreachable from the
    /// CLI -- an over-deep owned value is rejected through
    /// `OwnedValue::check_tree_depth` before any byte is written, by
    /// `to_jq_values` on the lazy route (which is what keeps
    /// `test_partial_result_over_depth_value_reports_cleanly_not_panic_1371`
    /// true) and by `evaluate_input_streaming` on the eager
    /// `-n`/`--slurp`/DSV route (which keeps
    /// `test_eager_over_depth_value_reports_cleanly_3155` true -- the gate the
    /// writer merge first shipped without). It stays as a backstop against a future caller that skips that
    /// gate, so it is covered here directly rather than left as an
    /// unexercised line.
    #[test]
    fn print_owned_json_depth_guard_3009() {
        let config = OutputConfig {
            compact: true,
            raw_output: false,
            join_output: false,
            raw_output0: false,
            ascii_output: false,
            color_output: false,
            color_scheme: ColorScheme::default(),
            sort_keys: false,
            indent_string: String::new(),
            unbuffered: false,
            seq: false,
            convention: JsonConvention::JqCompat,
        };
        let mut out = Vec::new();
        let err = print_owned_json(
            &mut out,
            &OwnedValue::Int(1),
            &JqCompatFormatter,
            &config,
            MAX_VALUE_TREE_DEPTH,
        )
        .expect_err("at the ceiling the writer must refuse");
        assert!(
            err.to_string().contains("nesting depth exceeds limit of"),
            "message: {err}"
        );
        assert!(out.is_empty(), "nothing may be written before refusing");
    }

    /// #3034: `print_json`'s `JqValue::Cursor` arm decodes a reindex bridge
    /// token to the float it stands for, rather than printing the token's
    /// raw span through `format_raw_number` -- the streaming-writer
    /// counterpart of `test_standard_json_to_jq_value_decodes_bridge_token_
    /// 3034` above.
    ///
    /// Like that test, direct construction rather than a CLI-level
    /// regression test: `evaluate_bytes_streaming`'s `index` is always built
    /// with the plain `JsonIndex::build` over a real user document, never
    /// `build_reindex` (its own doc comment), so this arm has no reachable
    /// caller in the shipped binary today -- pinned here so it doesn't
    /// silently bit-rot if a future one routes bridge text through it.
    #[test]
    fn test_write_output_jq_value_decodes_bridge_token_cursor_3034() {
        let token = OwnedValue::Float(f64::INFINITY).to_json_input_bridge();
        let json = format!("[{token}]");
        let bytes = json.as_bytes();
        let index = JsonIndex::build_reindex(bytes);
        let cursor = index.root(bytes).first_child().expect("one element");
        let config = OutputConfig {
            compact: true,
            raw_output: false,
            join_output: false,
            raw_output0: false,
            ascii_output: false,
            color_output: false,
            color_scheme: ColorScheme::default(),
            sort_keys: false,
            indent_string: String::new(),
            unbuffered: false,
            seq: false,
            convention: JsonConvention::JqCompat,
        };
        let mut out = Vec::new();
        write_output_jq_value(&mut out, &JqValue::Cursor(cursor), &config)
            .expect("a bridge-text cursor prints cleanly");
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!("{}\n", OwnedValue::Float(f64::INFINITY).to_json()),
            "{token}"
        );
    }

    /// #1192: `generic_result_to_jq_values`'s own `One`/`Many` arms --
    /// direct construction, since no ordinary top-level jq/yq expression
    /// found during this fix's development routes a *document-sourced*
    /// decode failure through this wrapper (see the sibling note on
    /// `standard_json_to_jq_value`'s doc comment).
    #[test]
    fn test_generic_result_to_jq_values_one_ok_and_err_1192() {
        let json: &[u8] = b"\"hello\"";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let mut sink = ErrorSink::default();
        let out = generic_result_to_jq_values(
            GenericResult::One(value),
            cursor,
            &InputLocation::at(None, 1),
            &mut sink,
        );
        assert_eq!(out.len(), 1);
        assert!(!sink.hit());

        let json: &[u8] = b"\"\xff\xfe\"";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let mut sink = ErrorSink::default();
        let out = generic_result_to_jq_values(
            GenericResult::One(value),
            cursor,
            &InputLocation::at(None, 1),
            &mut sink,
        );
        assert!(out.is_empty());
        assert!(sink.hit());
    }

    /// #1192: `generic_result_to_jq_values`'s `Many` arm stops at the first
    /// decode failure, keeping the already-converted prefix and reporting
    /// exactly once -- matching how an ordinary `error`/`break` mid-
    /// generator stops the rest of a stream elsewhere in this evaluator
    /// (#1164), not a "skip the bad one and keep going" semantic.
    #[test]
    fn test_generic_result_to_jq_values_many_stops_at_first_failure_1192() {
        let json: &[u8] = b"[1, \"\xff\xfe\", 3]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let StandardJson::Array(elements) = cursor.value() else {
            panic!("expected an array");
        };
        let vs: Vec<StandardJson<_>> = elements.collect();
        let mut sink = ErrorSink::default();
        let out = generic_result_to_jq_values(
            GenericResult::Many(vs),
            cursor,
            &InputLocation::at(None, 1),
            &mut sink,
        );
        assert!(matches!(
            out.as_slice(),
            [OutputItem::Lazy(JqValue::RawNumber(_))]
        ));
        assert!(sink.hit());
    }

    /// #2103 review: `generic_result_to_jq_values`'s `None`/`Error`/
    /// `ManyOwned`/`Break`/`Halt`/`Partial` arms all lost their only route
    /// once the eager M2 evaluator was deleted (#2103) -- its sole remaining
    /// caller, `evaluate_bytes_streaming`, feeds it exclusively through
    /// `generic_item_to_result`, which only ever produces `One`/`OneCursor`/
    /// `Owned`/`LazyKeys`/`LazyIndexRange`/`LazySeq` per item, plus a bare
    /// `Owned` from the input-queue-bridge branch; the top-level `Control`
    /// a filter escapes with is reported by that caller directly (its own
    /// `match control` a few lines up), never funneled back through this
    /// function. Direct construction, same as the `One`/`Many` precedent
    /// above (#1192) -- these variants are exhaustiveness/symmetry with
    /// `GenericResult`'s other producers (`eval.rs`'s owned path), not code
    /// this binary's own CLI surface can reach today.
    #[test]
    fn test_generic_result_to_jq_values_terminal_arms_2103() {
        let json: &[u8] = b"null";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let at = InputLocation::at(None, 1);

        let mut sink = ErrorSink::default();
        let out = generic_result_to_jq_values(GenericResult::None, cursor, &at, &mut sink);
        assert!(out.is_empty());
        assert!(!sink.hit());

        let mut sink = ErrorSink::default();
        let out = generic_result_to_jq_values(
            GenericResult::Error(EvalError::new("boom")),
            cursor,
            &at,
            &mut sink,
        );
        assert!(out.is_empty());
        assert!(sink.hit());

        let mut sink = ErrorSink::default();
        let out = generic_result_to_jq_values(
            GenericResult::ManyOwned(vec![OwnedValue::Int(1), OwnedValue::Int(2)]),
            cursor,
            &at,
            &mut sink,
        );
        // #3009: `ManyOwned` now reaches the writer as owned values rather
        // than as rebuilt `JqValue`s -- this assertion is where that routing
        // is visible.
        assert!(matches!(
            out.as_slice(),
            [
                OutputItem::Owned(OwnedValue::Int(1)),
                OutputItem::Owned(OwnedValue::Int(2))
            ]
        ));
        assert!(!sink.hit());

        let mut sink = ErrorSink::default();
        let out = generic_result_to_jq_values(
            GenericResult::Break("out".to_string()),
            cursor,
            &at,
            &mut sink,
        );
        assert!(out.is_empty());
        assert!(sink.hit());

        let mut sink = ErrorSink::default();
        let out = generic_result_to_jq_values(GenericResult::Halt(3), cursor, &at, &mut sink);
        assert!(out.is_empty());
        assert_eq!(sink.halted(), Some(3));

        let mut sink = ErrorSink::default();
        let out = generic_result_to_jq_values(
            GenericResult::Partial(
                vec![OwnedValue::Int(1)],
                jq::Control::Error(EvalError::new("boom")),
            ),
            cursor,
            &at,
            &mut sink,
        );
        assert!(matches!(
            out.as_slice(),
            [OutputItem::Owned(OwnedValue::Int(1))]
        ));
        assert!(sink.hit());

        let mut sink = ErrorSink::default();
        let out = generic_result_to_jq_values(
            GenericResult::Partial(vec![OwnedValue::Int(1)], jq::Control::Break("out".into())),
            cursor,
            &at,
            &mut sink,
        );
        assert!(matches!(
            out.as_slice(),
            [OutputItem::Owned(OwnedValue::Int(1))]
        ));
        assert!(sink.hit());

        let mut sink = ErrorSink::default();
        let out = generic_result_to_jq_values(
            GenericResult::Partial(vec![OwnedValue::Int(1)], jq::Control::Halt(7)),
            cursor,
            &at,
            &mut sink,
        );
        assert!(matches!(
            out.as_slice(),
            [OutputItem::Owned(OwnedValue::Int(1))]
        ));
        assert_eq!(sink.halted(), Some(7));
    }

    /// #2103 review (coverage-diff bot, PR #2652): `generic_result_to_jq_values`'s
    /// `ManyCursor` arm has the identical unreachable-via-CLI story as the
    /// batch `test_generic_result_to_jq_values_terminal_arms_2103` above
    /// already covers -- `evaluate_bytes_streaming`'s sole conversion route,
    /// `generic_item_to_result`, never produces a `Many`/`ManyCursor`/
    /// `ManyOwned` `GenericResult` from a single `GenericItem` (there is no
    /// `GenericItem` counterpart for any of the three). That batch's own
    /// commit only pinned the six arms the earlier "eager M2 route deleted"
    /// coverage-diff round flagged; `ManyCursor` surfaced as its own,
    /// separate finding once the `keys_unsorted[]` cursor-preservation fix
    /// (#2103's own change) shifted which other, unrelated arms the test
    /// suite's existing `keys_unsorted`/`limit` shapes happened to reach.
    /// Direct construction, same precedent as `Many`'s own #1192 test.
    #[test]
    fn test_generic_result_to_jq_values_many_cursor_2103() {
        let json: &[u8] = b"[1, 2, 3]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let StandardJson::Array(elements) = cursor.value() else {
            panic!("expected an array"); // omni-dev: coverage tolerate-line reason="unreachable in a passing suite by design -- the fixed b\"[1, 2, 3]\" literal above always decodes to StandardJson::Array (#2103)"
        };
        let cursors: Vec<_> = elements.cursor_iter().collect();
        let at = InputLocation::at(None, 1);

        let mut sink = ErrorSink::default();
        let out =
            generic_result_to_jq_values(GenericResult::ManyCursor(cursors), cursor, &at, &mut sink);
        assert_eq!(out.len(), 3);
        assert!(
            out.iter()
                .all(|v| matches!(v, OutputItem::Lazy(JqValue::Cursor(_)))),
            "{out:?}"
        );
        assert!(!sink.hit());
    }
}
