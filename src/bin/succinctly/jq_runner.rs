//! jq-compatible command runner for succinctly.
//!
//! This module implements a jq-compatible CLI interface using the succinctly
//! JSON semi-indexing and jq expression evaluator.

use anyhow::{Context, Result};
use indexmap::IndexMap;
use std::collections::BTreeMap;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use succinctly::dsv::{build_index as build_dsv_index, DsvConfig, DsvRows};
use succinctly::jq::document::{effective_keys, key_hash, DistinctKeyCursors};
use succinctly::jq::eval_generic::{
    check_nesting_depth, eval_with_cursor, to_owned as generic_to_owned, GenericResult,
    MAX_NESTING_DEPTH,
};
use succinctly::jq::walk::map_builtin_subexprs;
use succinctly::jq::{
    self, format_number_jq_compat, nonfinite_display_string, EvalError, Expr, JqSemantics, JqValue,
    OwnedValue, Program, UnresolvedCall, MAX_VALUE_TREE_DEPTH,
};
use succinctly::json::light::{preceding_gap_ok, JsonCursor, StandardJson};
use succinctly::json::validate::{self, ValidationError};
use succinctly::json::JsonIndex;

use super::JqCommand;
use crate::output::{
    self, escape_json_string, escape_json_string_ascii, exit_codes, flush_then_err, ColorScheme,
    ControlEscape, DiagStyle, ErrorSink, FloatStyle, InputLocation, JsonFormatOpts,
    LoudFlushWriter, Terminator,
};

/// Evaluation context for passing variables to the jq evaluator.
#[derive(Debug, Default)]
pub struct EvalContext {
    /// Named arguments from --arg, --argjson, --slurpfile, --rawfile
    pub named: IndexMap<String, OwnedValue>,
    /// Positional arguments from --args or --jsonargs
    pub positional: Vec<OwnedValue>,
}

/// Module loader for resolving and loading jq modules.
#[derive(Debug)]
pub struct ModuleLoader {
    /// Search path for modules (in order of priority)
    search_path: Vec<PathBuf>,
    /// Loaded modules (path -> function definitions: name, params, body)
    loaded_modules: BTreeMap<String, Vec<(String, Vec<String>, Expr)>>,
    /// Auto-loaded ~/.jq file definitions (if file exists): name, params, body
    auto_loaded_defs: Vec<(String, Vec<String>, Expr)>,
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
        if let Some(home) = std::env::var_os("HOME") {
            let jq_path = PathBuf::from(home).join(".jq");
            if jq_path.is_file() {
                // Auto-load ~/.jq file - functions defined here are always available
                if let Ok(contents) = std::fs::read_to_string(&jq_path) {
                    if let Ok(program) = jq::parse_program(&contents) {
                        auto_loaded_defs = extract_func_defs(&program.expr);
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
        }
    }

    /// Resolve a module path to a file path.
    fn resolve_module(&self, module_path: &str) -> Option<PathBuf> {
        // Add .jq extension if not present
        let module_file = if module_path.ends_with(".jq") {
            module_path.to_string()
        } else {
            format!("{module_path}.jq")
        };

        // Search in each path
        for base in &self.search_path {
            let full_path = base.join(&module_file);
            if full_path.is_file() {
                return Some(full_path);
            }
        }

        None
    }

    /// Load a module and return its function definitions (name, params, body).
    pub fn load_module(&mut self, module_path: &str) -> Result<Vec<(String, Vec<String>, Expr)>> {
        // Check if already loaded
        if let Some(defs) = self.loaded_modules.get(module_path) {
            return Ok(defs.clone());
        }

        // Resolve the module path
        let file_path = self
            .resolve_module(module_path)
            .ok_or_else(|| anyhow::anyhow!("module '{module_path}' not found in search path"))?;

        // Read and parse the module
        let contents = std::fs::read_to_string(&file_path)
            .with_context(|| format!("failed to read module: {}", file_path.display()))?;

        let program = jq::parse_program(&contents).map_err(|e| {
            anyhow::anyhow!("parse error in module '{}': {}", file_path.display(), e)
        })?;

        // Extract function definitions from the expression
        let defs = extract_func_defs(&program.expr);

        // Cache the loaded module
        self.loaded_modules
            .insert(module_path.to_string(), defs.clone());

        Ok(defs)
    }

    /// Process imports and includes, returning the modified expression with all functions defined.
    pub fn process_program(&mut self, program: &Program) -> Result<Expr> {
        let mut expr = program.expr.clone();

        // First, prepend auto-loaded ~/.jq definitions (lowest priority, can be overridden)
        for (name, params, body) in self.auto_loaded_defs.clone().into_iter().rev() {
            expr = Expr::FuncDef {
                name,
                params,
                body: Box::new(body),
                then: Box::new(expr),
            };
        }

        // Process includes (definitions merged into current scope)
        for include in &program.includes {
            let defs = self.load_module(&include.path)?;
            // Wrap expression with function definitions from the included module
            for (name, params, body) in defs.into_iter().rev() {
                expr = Expr::FuncDef {
                    name,
                    params,
                    body: Box::new(body),
                    then: Box::new(expr),
                };
            }
        }

        // Process imports (definitions available under namespace::)
        // Load modules and add their functions with namespace prefixes
        for import in &program.imports {
            let defs = self.load_module(&import.path)?;
            let namespace = &import.alias;

            // Add each function with a namespaced name (namespace::funcname)
            for (name, params, body) in defs.into_iter().rev() {
                let namespaced_name = format!("{namespace}::{name}");
                expr = Expr::FuncDef {
                    name: namespaced_name,
                    params,
                    body: Box::new(body),
                    then: Box::new(expr),
                };
            }
        }

        // Transform NamespacedCall expressions to regular FuncCall expressions
        expr = rewrite_namespaced_calls(expr);

        Ok(expr)
    }
}

/// Recursively rewrite NamespacedCall expressions to regular FuncCall expressions
/// by transforming `namespace::func(args)` to `namespace::func(args)` as a regular call
fn rewrite_namespaced_calls(expr: Expr) -> Expr {
    match expr {
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
        Expr::FuncCall { name, args } => {
            let new_args: Vec<Expr> = args.into_iter().map(rewrite_namespaced_calls).collect();
            Expr::FuncCall {
                name,
                args: new_args,
            }
        }
        Expr::FuncDef {
            name,
            params,
            body,
            then,
        } => Expr::FuncDef {
            name,
            params,
            body: Box::new(rewrite_namespaced_calls(*body)),
            then: Box::new(rewrite_namespaced_calls(*then)),
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
            patterns,
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
            patterns,
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
            patterns,
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
        | Expr::Index(_)
        | Expr::IndexNumber { .. }
        | Expr::Slice { .. }
        | Expr::SliceNumber { .. }
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
fn extract_func_defs(expr: &Expr) -> Vec<(String, Vec<String>, Expr)> {
    let mut defs = Vec::new();

    fn extract_inner(expr: &Expr, defs: &mut Vec<(String, Vec<String>, Expr)>) {
        if let Expr::FuncDef {
            name,
            params,
            body,
            then,
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
    /// Format numbers like jq (normalize 4e4 → 40000, 0.10 → 0.1)
    jq_compat: bool,
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

        // Determine jq_compat mode with priority:
        // 1. --preserve-input flag forces off (preserve original formatting)
        // 2. SUCCINCTLY_PRESERVE_INPUT=1 env var disables jq_compat
        // 3. Default: on (jq-compatible formatting)
        let jq_compat = !args.preserve_input
            && !std::env::var("SUCCINCTLY_PRESERVE_INPUT")
                .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));

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
            jq_compat,
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
        // - Not jq_compat (would need to reformat numbers like 4e4 → 4E+4)
        self.compact
            && !self.color_output
            && !self.sort_keys
            && !self.ascii_output
            && !self.raw_output
            && !self.seq
            && !self.jq_compat
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

/// Above this many fields, [`spans_repeat`] sorts instead of comparing every
/// pair.
///
/// Objects in real documents are small, and below the threshold the pairwise
/// loop needs no allocation. Above it the pairwise loop is quadratic, which
/// is not a tuning question but a correctness-of-scale one: on a 10 MB
/// document whose root object is wide, leaving it unbounded measured 1240%
/// slower than not collapsing at all.
///
/// This is the only such threshold in the tree; `src/jq/document.rs`'s
/// duplicate probe is fingerprint-based and has no pairwise branch to bound.
const PAIRWISE_SPAN_SCAN_LIMIT: usize = 16;

/// A cheap discriminator for a key's raw span: its length plus its first and
/// last content bytes, packed into one word.
///
/// Distinct fingerprints prove distinct keys, so the pairwise scan below
/// compares words and only falls back to comparing bytes when two collide.
/// Keys within one object almost always differ in length or in their first
/// byte, which makes the byte comparison rare enough to disappear from the
/// profile -- comparing the spans directly instead measured ~3% of
/// `sjq '.'` on a 10 MB document.
#[inline]
fn span_fingerprint(raw: &[u8]) -> u64 {
    let n = raw.len();
    let first = if n > 2 { raw[1] } else { 0 };
    let last = if n > 3 { raw[n - 2] } else { 0 };
    ((n as u64) << 16) | ((first as u64) << 8) | last as u64
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
            *slot = span_fingerprint(field.raw);
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
        return Err(MalformedJsonError(EvalError::malformed_json_text(frame.text)).into());
    };
    if !config.ascii_output && !field.escaped {
        out.write_all(field.raw)?;
    } else if let Ok(decoded) = key.as_str() {
        out.write_all(b"\"")?;
        let text = if config.ascii_output {
            escape_json_string_ascii(&decoded)
        } else {
            escape_json_string(&decoded)
        };
        out.write_all(text.as_bytes())?;
        out.write_all(b"\"")?;
    } else {
        out.write_all(field.raw)?;
    }
    out.write_all(b":")?;
    out.write_all(space_after_colon.as_bytes())?;
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
fn bail_if_keys_malformed<F: succinctly::jq::document::DocumentFields>(
    keys: &DistinctKeyCursors<F>,
    doc_text: Option<&[u8]>,
) -> Result<()> {
    match (keys.is_malformed(), doc_text) {
        (true, Some(text)) => Err(MalformedJsonError(EvalError::malformed_json_text(text)).into()),
        _ => Ok(()),
    }
}

#[derive(Debug)]
pub struct MalformedJsonError(pub EvalError);

impl std::fmt::Display for MalformedJsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.message)
    }
}

impl std::error::Error for MalformedJsonError {}

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
            Some(MalformedJsonError(err)) => {
                sink.report(DiagStyle::Jq, err, &at());
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

/// Report a call the compile-time resolution pass could not resolve, in jq's
/// own compile-error shape (#1473):
///
/// ```text
/// jq: error: f/3 is not defined at <top-level>, line 1:
/// def f(x): x; if false then f(1;2;3) else 1 end
/// jq: 1 compile error
/// ```
///
/// **The line is located by searching `filter` for the offending identifier,
/// not read off the AST** — `Expr::FuncCall` carries no source position, and
/// adding one would perturb `format!("{body:?}").len()`, which #1381's
/// `MAX_FUNC_EXPANSION_WEIGHTED_COST` is calibrated against. A filter that
/// mentions the same undefined name more than once therefore cites the first
/// occurrence's line, which may not be the one that failed to resolve; a
/// filter whose failing call came from an `include`d module or `~/.jq` has no
/// occurrence in `filter` at all, and drops the line marker and source echo
/// rather than inventing a position.
///
/// jq pads the echoed line with trailing spaces (a `%*s` in its own
/// `locfile_locate`); the padding width follows the failing node's start
/// column for a simple undefined name but points elsewhere for an
/// arity mismatch, so this reproduces the column rule rather than every case.
/// It is trailing whitespace either way.
///
/// Always "1 compile error": [`jq::resolve_func_calls`] stops at the first
/// unresolvable call, where jq reports all of them. Reporting the rest needs
/// the same per-call positions.
fn report_unresolved_call(unresolved: &UnresolvedCall, filter: &str) {
    let UnresolvedCall { name, arity } = unresolved;

    let Some((line_no, line_text, column)) = locate_identifier(filter, name) else {
        eprintln!("jq: error: {name}/{arity} is not defined at <top-level>");
        eprintln!("jq: 1 compile error");
        return;
    };

    eprintln!("jq: error: {name}/{arity} is not defined at <top-level>, line {line_no}:");
    eprintln!("{line_text}{}", " ".repeat(column));
    eprintln!("jq: 1 compile error");
}

/// Find the first occurrence of `name` in `filter` that stands alone as an
/// identifier, returning its 1-based line, that line's text, and its 0-based
/// column.
///
/// "Stands alone" means neither neighbour is an identifier character, so a
/// search for `f` does not match the `f` inside `first` — but `::` is allowed
/// on the left, since a namespaced call arrives here as `ns::f` while the
/// source spells the two halves either side of the separator.
fn locate_identifier(filter: &str, name: &str) -> Option<(usize, String, usize)> {
    fn is_ident_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }

    let bytes = filter.as_bytes();
    let mut search_from = 0;
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
    ))
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

    // Build evaluation context from arguments
    let context = build_context(&args)?;

    // Get the filter expression
    let filter_str = get_filter(&args)?;

    // Parse the filter as a full program (with module directives).
    //
    // Returns the exit code rather than an `anyhow` error (#1473): a failed
    // parse is jq's compile error, exit 3, and routing it through `anyhow`
    // gave exit 1 *and* printed a second, stray `Error: compile error` line
    // from `main`'s own reporting. Both were confirmed live against jq 1.7.1,
    // which exits 3 with no such line. Nothing else distinguished this arm
    // from a resolution failure below, so the two now answer identically.
    let program = match jq::parse_program(&filter_str) {
        Ok(program) => program,
        Err(e) => {
            eprintln!("jq: compile error: {e}");
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
    let expr = match module_loader.process_program(&program) {
        Ok(expr) => expr,
        Err(e) => {
            eprintln!("jq: module error: {e}");
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

    let expr = jq::substitute_vars(&expr, all_vars);

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
    if let Err(unresolved) = jq::resolve_func_calls(&expr) {
        report_unresolved_call(&unresolved, &filter_str);
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
            let raw_inputs: Vec<Vec<u8>> = if files.is_empty() {
                vec![read_stdin_bytes()?]
            } else {
                files
                    .iter()
                    .map(|path| read_file_bytes(path))
                    .collect::<Result<Vec<_>>>()?
            };

            for (file_idx, raw) in raw_inputs.into_iter().enumerate() {
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

                    let row_value = OwnedValue::Array(fields);

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
                        &mut |sink, result| {
                            had_output = true;
                            if args.exit_status {
                                last_output = Some(result.clone());
                            }
                            let stop = route_write_error(
                                sink,
                                &mut out,
                                || at.resolve(),
                                |o| write_output(o, &result, &output_config),
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
    // - Not using output transformations that need full access to values (sort_keys, color, ascii)
    // - Not using input/inputs/input_line_number (#723): those need the
    //   "original" path's own already-materialized Vec<OwnedValue> to share
    //   with the shared input queue below; the lazy path never builds one.
    // Both jq_compat (reformatting numbers) and preserve mode (keeping original formatting)
    // use the lazy path for correctness.
    let can_use_lazy_path = !args.slurp
        && !args.raw_input
        && args.input_dsv.is_none()
        && !args.seq // seq input mode parses differently
        && !output_config.sort_keys
        && !output_config.color_output
        && !output_config.ascii_output // ASCII output requires escaping
        && !uses_input_builtins;

    if can_use_lazy_path && !args.null_input {
        // Lazy path: read files as raw bytes and process directly
        // This preserves original number formatting like "4e4"
        let files = get_input_files(&args);
        let raw_inputs: Vec<Vec<u8>> = if files.is_empty() {
            vec![read_stdin_bytes()?]
        } else {
            files
                .iter()
                .map(|path| read_file_bytes(path))
                .collect::<Result<Vec<_>>>()?
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

        for (idx, raw) in raw_inputs.iter().enumerate() {
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
            // Process as JSON stream (handle multiple JSON values in one input)
            let values = match find_json_values(raw) {
                Ok(values) => values,
                Err(offset) => {
                    let at = InputLocation::at(filename.as_deref(), line_at(raw, offset));
                    sink.report(DiagStyle::Jq, &EvalError::new("Invalid JSON text"), &at);
                    continue;
                }
            };
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
                    out.write_all(json_bytes)?;
                    out.write_all(b"\n")?;
                    continue;
                }

                // Slow path: build index and evaluate expression
                let index = JsonIndex::build(json_bytes);
                // jq names the line the input value ends on, counted in the
                // whole file rather than in this value's slice.
                let at = InputLocation::at(filename.as_deref(), line_counter.advance_to(end));
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
                let results = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    evaluate_bytes_lazy(json_bytes, &expr, &index, &at, &mut sink)
                })) {
                    Ok(results) => results,
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
                        if let Some(code) = sink.halted() {
                            out.flush()?;
                            return Ok(code);
                        }
                        continue;
                    }
                };

                // Consume results to free memory after each value is written
                for result in results {
                    had_output = true;
                    // For exit_status tracking, we need to check the last value
                    if args.exit_status {
                        // `-e` is the flag that forces materialization at
                        // all, so it is where a decode failure first becomes
                        // observable here (#1247). Report and skip the value
                        // rather than letting an undecodable string count as
                        // a truthiness answer; `sink` drives the exit code.
                        match result.materialize() {
                            Ok(owned) => last_output = Some(owned),
                            Err(e) => {
                                sink.report(DiagStyle::Jq, &e, &at);
                                continue;
                            }
                        }
                    }
                    // A malformed document is a data error, not an I/O one: it
                    // belongs in jq's diagnostic channel (exit 5) rather than
                    // aborting the process through `anyhow` (#1194). Stop
                    // emitting results for *this* document, but fall through
                    // to the halt check below and carry on with the rest of
                    // the stream (#355) -- real jq stops at the first parse
                    // error instead, a divergence recorded in
                    // `docs/compliance/jq/limitations.md`.
                    if route_write_error(
                        &mut sink,
                        &mut out,
                        || at.clone(),
                        |o| write_output_jq_value(o, &result, &output_config),
                    )? {
                        break;
                    }
                    // result is dropped here, freeing its memory immediately
                }
                // halt/halt_error (#791) outranks everything else, including
                // remaining values/files still to process.
                if let Some(code) = sink.halted() {
                    out.flush()?;
                    return Ok(code);
                }
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
        let (inputs, locations) = match get_inputs(&args, force_read_under_null_input) {
            Ok(Ok(inputs)) => inputs,
            // A malformed or undecodable document is a data error, so it goes
            // out in jq's own diagnostic shape at exit 5 rather than through
            // `anyhow` at exit 1 with an `Error:` prefix jq never prints
            // (#1194). Everything else here really is I/O and keeps the
            // `anyhow` path.
            //
            // Line 0, the same placeholder the lazy path prints when it has
            // no better position: this route reads the whole stream up front
            // and fails before any per-document offset exists. Reusing the
            // existing shape beats introducing a second rendering.
            Ok(Err(e)) => match e.downcast::<MalformedJsonError>() {
                Ok(MalformedJsonError(err)) => {
                    let files = get_input_files(&args);
                    let file = files.first().map(|p| p.to_string_lossy().to_string());
                    sink.report(DiagStyle::Jq, &err, &InputLocation::at(file.as_deref(), 0));
                    return Ok(DiagStyle::Jq.error_exit_code());
                }
                Err(e) => return Err(e),
            },
            Err(exit_code) => return Ok(exit_code), // Validation error
        };

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
                jq::seed_remaining_inputs(queue, locations.exhausted(args.slurp));
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
                    &mut |sink, result| {
                        had_output = true;
                        last_output = Some(result.clone());
                        let stop = route_write_error(
                            sink,
                            &mut out,
                            || ErrorAt::Live(&locations).resolve(),
                            |o| write_output(o, &result, &output_config),
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
                while let Some(input) = jq::pop_remaining_input() {
                    // Streaming, not collect-then-write (#1653) -- see
                    // `evaluate_input_streaming`.
                    evaluate_input_streaming(
                        &input,
                        &expr,
                        &context,
                        &ErrorAt::Live(&locations),
                        &mut sink,
                        &mut |sink, result| {
                            had_output = true;
                            last_output = Some(result.clone());
                            let stop = route_write_error(
                                sink,
                                &mut out,
                                || ErrorAt::Live(&locations).resolve(),
                                |o| write_output(o, &result, &output_config),
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
                    &mut |sink, result| {
                        had_output = true;
                        last_output = Some(result.clone());
                        let stop = route_write_error(
                            sink,
                            &mut out,
                            || at.resolve(),
                            |o| write_output(o, &result, &output_config),
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
fn build_context(args: &JqCommand) -> Result<EvalContext> {
    let mut context = EvalContext::default();

    // Process --arg name value pairs
    for chunk in args.arg.chunks(2) {
        if let [name, value] = chunk {
            context
                .named
                .insert(name.clone(), OwnedValue::String(value.clone()));
        }
    }

    // Process --argjson name value pairs
    for chunk in args.argjson.chunks(2) {
        if let [name, value] = chunk {
            let json_value = parse_json_value(value)
                .with_context(|| format!("Invalid JSON for --argjson {name}"))?;
            context.named.insert(name.clone(), json_value);
        }
    }

    // Process --slurpfile name file pairs
    for chunk in args.slurpfile.chunks(2) {
        if let [name, file] = chunk {
            let contents = std::fs::read_to_string(file)
                .with_context(|| format!("Failed to read file for --slurpfile {name}"))?;
            let values = parse_json_stream(&contents)?;
            context
                .named
                .insert(name.clone(), OwnedValue::Array(values));
        }
    }

    // Process --rawfile name file pairs
    for chunk in args.rawfile.chunks(2) {
        if let [name, file] = chunk {
            let contents = std::fs::read_to_string(file)
                .with_context(|| format!("Failed to read file for --rawfile {name}"))?;
            context
                .named
                .insert(name.clone(), OwnedValue::String(contents));
        }
    }

    // Process --args: values become string positional args
    for arg in &args.args {
        context.positional.push(OwnedValue::String(arg.clone()));
    }

    // Process --jsonargs: values become JSON positional args
    for arg in &args.jsonargs {
        let json_value =
            parse_json_value(arg).with_context(|| format!("Invalid JSON for --jsonargs: {arg}"))?;
        context.positional.push(json_value);
    }

    Ok(context)
}

/// Build the $ARGS special variable containing named and positional args.
fn build_args_var(context: &EvalContext) -> OwnedValue {
    let mut args_obj = IndexMap::new();

    // Build named object from context.named
    let named_obj: IndexMap<String, OwnedValue> = context.named.clone();
    args_obj.insert("named".to_string(), OwnedValue::Object(named_obj));

    // Build positional array from context.positional
    args_obj.insert(
        "positional".to_string(),
        OwnedValue::Array(context.positional.clone()),
    );

    OwnedValue::Object(args_obj)
}

/// Get the filter expression from arguments.
fn get_filter(args: &JqCommand) -> Result<String> {
    if let Some(ref path) = args.from_file {
        // Filter comes from file
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read filter file: {}", path.display()))?;
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

/// Get input values based on arguments.
/// Returns Err(i32) for validation failures (exit code), Ok(Err) for other errors.
fn get_inputs(
    args: &JqCommand,
    force_read_under_null_input: bool,
) -> std::result::Result<Result<(Vec<OwnedValue>, InputLocations)>, i32> {
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
        return Ok(Ok((vec![OwnedValue::Null], InputLocations::unknown())));
    }

    // Get input files
    let files = get_input_files(args);

    // Collect raw input from files or stdin. Read as bytes and decode below
    // rather than through `read_to_string`, which refused the whole input on
    // a stray byte and reported it as a *read* failure when the read had in
    // fact succeeded (#1247).
    let raw_bytes: Vec<(Option<usize>, Vec<u8>)> = if files.is_empty() {
        match read_stdin_bytes() {
            Ok(b) => vec![(None, b)],
            Err(e) => return Ok(Err(e)),
        }
    } else {
        let mut inputs = Vec::new();
        for (idx, path) in files.iter().enumerate() {
            match read_file_bytes(path) {
                Ok(b) => inputs.push((Some(idx), b)),
                Err(e) => return Ok(Err(e)),
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
    if args.seq && !args.raw_input && args.input_dsv.is_none() && !args.null_input {
        if let Some(warning) = seq_no_rs_byte_warning(&raw_bytes) {
            eprintln!("{warning}");
        }
    }

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
    // EOF (#1520), computed here from `raw_inputs` while it's still whole,
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
    let slurp_eof_line: Option<usize> = if args.slurp {
        raw_inputs.last().and_then(|(_, raw)| {
            // #1550: the drop check has to read across every file as one
            // stream, not just `raw_inputs.last()` alone -- a truncated
            // record's own opening RS byte and its closing/disambiguating
            // bytes can live in different files.
            let dropped =
                args.seq && !args.raw_input && seq_stream_trailing_record_is_dropped(&raw_inputs);
            if dropped {
                None
            } else {
                Some(line_at(raw.as_bytes(), raw.len()))
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
        )));
    }

    // Process based on input mode
    let mut values = Vec::new();

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
    if args.seq && !args.raw_input && args.input_dsv.is_none() {
        values = build_seq_values(&raw_inputs, &mut locations, args.slurp);
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
                // Parse as JSON stream
                let parsed = match parse_json_stream(&raw) {
                    Ok(p) => p,
                    Err(e) => return Ok(Err(e)),
                };
                // Skipped under `--slurp` (#1541) -- see the DSV branch above.
                // `find_json_values` exists here only to feed
                // `locations.extend_from_ends`, so slurp mode skips that scan
                // entirely rather than running it and discarding the result.
                // Side effect: the divergence check below (`find_json_values`
                // disagreeing with `parse_json_stream`/`serde_json`) no longer
                // runs under `--slurp` either -- if the two validators were ever
                // to disagree on some input, non-slurp mode would still raise
                // the internal error below, but slurp mode would now silently
                // proceed. Accepted: the comment above already treats that
                // divergence as unreachable through this crate's public CLI
                // surface, so this only narrows *where* an already-believed-dead
                // safety net runs, not what output a real input can produce.
                if !args.slurp {
                    // `parse_json_stream` (above) already validated this exact
                    // input successfully via `serde_json`, which is strictly
                    // pickier than `find_json_values`'s own lenient heuristic
                    // scan (RFC 8259 plus #1094's leading-zero tolerance, vs.
                    // `find_json_values`'s RFC 8259 plus leading-zero *and*
                    // leading-dot tolerance, #1171) -- so `find_json_values`
                    // should never fail here in practice; unreachable through
                    // this crate's own public CLI surface, not exercised by a
                    // test for that reason (matching this codebase's established
                    // convention for exhaustive-but-dead defensive arms, e.g.
                    // #1064). Surfaced as an internal error rather than silently
                    // reusing a stale/wrong offset list if the two validators
                    // ever do diverge.
                    let ends: Vec<usize> = match find_json_values(raw.as_bytes()) {
                        Ok(values) => values.into_iter().map(|(_, end)| end).collect(),
                        Err(offset) => {
                            return Ok(Err(anyhow::anyhow!(
                                "internal error: find_json_values failed at byte {offset} \
                                 after parse_json_stream already validated this input"
                            )));
                        }
                    };
                    locations.extend_from_ends(src, &raw, &ends, parsed.len());
                }
                values.extend(parsed);
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
            vec![OwnedValue::Array(values)],
            InputLocations::single(at),
        )))
    } else {
        Ok(Ok((values, locations)))
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
    /// from an isolated `raw`, so `build_seq_values`/`parse_json_seq_with_ends`
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
    fn per_value(&self) -> &[(u32, u32)] {
        &self.per_value
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
fn read_file_bytes(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("Failed to read file: {}", path.display()))
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
/// order [`find_json_values`]/[`parse_json_seq_with_ends`] already produce
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
            None => return Err(start),
        }
    }

    Ok(values)
}

/// Where the JSON token starting at `pos` ends, or `None` if no token can
/// start there.
///
/// Extracted from [`find_json_values`] so `--seq`'s own lenient scanner
/// (#1723) can ask the identical per-token question without a second copy of
/// this dispatch drifting from it.
///
/// "Ends" is structural, not a validity verdict: `{"a":1 xyz}` scans as one
/// token because its braces match, and only validation rejects it. Keeping
/// the two separate is what lets the `--seq` caller reject a whole record
/// without ever reading a value out of the middle of a malformed one.
fn scan_one_json_token(bytes: &[u8], pos: usize) -> Option<usize> {
    match bytes[pos] {
        // Object or array - find matching close
        b'{' | b'[' => find_matching_close(bytes, pos),
        // String - find end quote
        b'"' => find_string_end(bytes, pos),
        // true, false, null
        b't' | b'f' | b'n' => find_literal_end(bytes, pos),
        // Number. `number_literal_end` (shared with `light.rs`'s own
        // materializer, #1171 review -- one validated implementation
        // instead of independently-maintained copies) both finds the
        // end of and validates the token: a `.` is accepted as a
        // leading byte when at least one digit follows (`.5` -> `0.5`,
        // matching real jq's own leniency beyond strict JSON), and a
        // byte sequence that only *looks* number-shaped (`-e5`, `1e`,
        // a bare `.`) is rejected outright rather than silently
        // accepted as a truncated or zero-length span.
        b'-' | b'.' | b'0'..=b'9' => succinctly::json::light::number_literal_end(bytes, pos),
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
                // Skip string
                let end = find_string_end(bytes, i)?;
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

/// Find the end of a string starting at `pos` (which points to opening quote).
fn find_string_end(bytes: &[u8], pos: usize) -> Option<usize> {
    let mut i = pos + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Some(i + 1),
            b'\\' => i += 2, // Skip escaped character
            _ => i += 1,
        }
    }
    None
}

/// Find the end of a literal (true, false, null) starting at `pos`.
fn find_literal_end(bytes: &[u8], pos: usize) -> Option<usize> {
    let mut i = pos;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        i += 1;
    }
    Some(i)
}

/// Validate `s` as a single, complete JSON value via `serde_json::from_str`
/// -- not for its resulting `Value` (discarded by every caller), but for
/// its error message and its rejection of trailing garbage (`42 garbage`)
/// that `JsonIndex`'s own semi-indexing wouldn't catch on its own. Shared
/// by `parse_json_value` (`--argjson`/`--jsonargs`, #1058) and
/// `parse_json_seq` (`--seq`, #1093) -- see #1163. Not the same function as
/// `validate_json_input` above (RFC 8259 strict-mode `--validate`/`json
/// validate`, a separate hand-rolled zero-allocation validator with its
/// own grammar) -- similar names, different jobs.
///
/// Deliberately parses to `serde_json::Value` here rather than the cheaper
/// `serde::de::IgnoredAny`: `IgnoredAny` doesn't range-check numbers at
/// all, so a magnitude-overflowing literal (`1e400`) that `Value`
/// correctly rejects ("number out of range") would instead silently reach
/// `json_bytes_to_owned_value` and materialize as `null` -- trading a
/// clear error for silent data loss (#1095 review). `parse_json_stream`
/// below doesn't call this: it validates a whole multi-value stream via
/// `serde_json::Deserializer` instead of one value via `from_str`, a
/// structurally different mechanism this helper can't drive.
fn validate_json_str(s: &str) -> serde_json::Result<()> {
    serde_json::from_str::<serde_json::Value>(s).map(|_| ())
}

/// Validate `s` via `validate_json_str`, then materialize the real
/// `OwnedValue` from those same bytes via `json_bytes_to_owned_value` --
/// the full "validate, then reparse the same span" pattern #1163 asked to
/// share, for the call sites where the validated string and the
/// materialized string are the same one. `parse_json_value`'s own
/// leading-zero retry can't use this for its retry branch (it validates a
/// *normalized* copy but must still materialize the *original* text, see
/// its own comment), so that one branch calls `validate_json_str`
/// directly instead.
fn validate_and_materialize_json(
    s: &str,
) -> serde_json::Result<core::result::Result<OwnedValue, EvalError>> {
    validate_json_str(s)?;
    // Two nested results on purpose: the *outer* `Err` is serde's, and only
    // it may trigger the caller's leading-zero retry. A decode failure
    // (#1247) is not a validation failure serde could disagree about, so it
    // must not be mistaken for one -- retrying would just fail again.
    Ok(crate::output::json_bytes_to_owned_value(s.as_bytes()))
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
/// Validates strictly via `validate_and_materialize_json` first (see its
/// own doc for why `serde_json::Value`, not the cheaper `IgnoredAny`),
/// which reparses the same, now-known-valid text through this crate's own
/// fidelity-preserving JSON semi-indexer (the same one the primary input
/// path already uses, see `evaluate_input_streaming` above).
fn parse_json_value(s: &str) -> Result<OwnedValue> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(OwnedValue::Null);
    }

    match validate_and_materialize_json(s) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(anyhow::anyhow!("Invalid JSON: {s}: {e}")),
        Err(e) => {
            // Real jq's own number parser tolerates a leading zero
            // (`007`, `00`, `007.5`) that strict JSON doesn't (#1094) --
            // retry once with every number token's leading zero stripped
            // (string contents/keys left untouched) before surfacing the
            // original error. Only paid on this (rare) failure path, so
            // the common case's validation cost is unaffected. Can't use
            // `validate_and_materialize_json` here: it must validate the
            // *normalized* copy but still materialize the *original*
            // text below -- this crate's own semi-indexer already
            // tolerates a leading zero on its own (confirmed live), so
            // there's nothing left to repair for it, and normalizing the
            // materialized text too would silently rewrite the value's
            // source spelling.
            let normalized = normalize_leading_zero_numbers(s);
            if normalized == s || validate_json_str(&normalized).is_err() {
                return Err(e).with_context(|| format!("Invalid JSON: {s}"));
            }
            crate::output::json_bytes_to_owned_value(s.as_bytes())
                .map_err(|e| anyhow::anyhow!("Invalid JSON: {s}: {e}"))
        }
    }
}

/// Where a lenient, JSON-*ish* number token's integer-digit run and its
/// overall span end, both exclusive, relative to the same `bytes` slice
/// [`find_number_end`] scanned. `int_end` marks the end of the digit run
/// that follows an optional leading `-` (i.e. where a leading-zero strip
/// would need to stop); `end` marks the end of the whole token, including
/// any fraction/exponent. `int_end..end`, if non-empty, is always exactly
/// the token's fraction-plus-exponent suffix, contiguous by construction
/// since the scan that produces both is strictly sequential.
#[derive(Debug, PartialEq, Eq)]
struct NumberTokenEnd {
    int_end: usize,
    end: usize,
}

/// Find a lenient, JSON-*ish* number token's boundaries, starting at
/// `pos`, which must point at `-` or an ASCII digit. Unlike
/// [`crate::json::validate::is_valid_number`] (a strict RFC 8259
/// whole-slice checker, not a token-boundary finder, and not reusable here
/// for that reason), this tolerates a leading zero in the integer part
/// (`007`) the way real jq's own number parser does (#1094) -- the whole
/// point of the caller this exists for. Returns `None` only for a bare
/// `-` with no digit following it at all (`-`, `[-,1]`, `{"a":-}`) --
/// not a number token in the first place.
///
/// Also deliberately **not** [`succinctly::json::light::number_literal_end`]
/// (#1154 review) despite that function already living in this same file's
/// import list two functions above, for a `find_json_values` caller with
/// a similar-sounding job: its own doc comment explicitly scopes it to
/// *top-level* document splitting and warns against reuse elsewhere, and
/// its grammar genuinely diverges from what this repair pass needs --
/// it rejects a dangling exponent marker outright (`"5e"` -> `None`,
/// whereas real jq's own parser, and this function, consume it leniently
/// and let the later `serde_json` re-validation reject the whole string
/// instead), and it accepts a leading-dot fraction with zero integer
/// digits (`".5"`/`"-.5"`) that this function's caller has no leading-zero
/// stripping to do for in the first place (a bare `-.5` isn't reachable
/// here: `normalize_leading_zero_numbers` only calls this when `b == '-'`
/// or `b` is itself a digit, never `.`).
///
/// This is the fourth of (at least) four independent number-token
/// scanners in the crate, none delegating to any other (#1218) -- besides
/// `number_literal_end` above, see `succinctly::json::light`'s private
/// `nested_number_span` (greedy, backs `StandardJson`'s nested-value
/// materialization) and `succinctly::json::simple_light`'s private
/// `find_number_end` (also greedy, backs the separate `SimpleJsonIndex`).
/// See #1218 for the full survey and why a blanket consolidation needs
/// its own design pass.
///
/// Mirrors [`find_string_end`]/[`find_matching_close`]'s "find the end of
/// this token" shape (#1154) rather than interleaving grammar-walking
/// with a transformation, the way an earlier version of
/// [`normalize_leading_zero_numbers`] did -- that shape is exactly what
/// let a real bug (fabricating a bare `-` into `-0`, turning genuinely
/// invalid input into something that would silently validate) hide until
/// review caught it before #1152 merged.
///
/// Returns [`NumberTokenEnd`] rather than just the overall end (#1154
/// review) so the caller's own leading-zero strip doesn't need a second,
/// independent scan to re-derive `int_end` -- an earlier version of this
/// function discarded that boundary internally, forcing
/// `normalize_leading_zero_numbers` to re-walk the same digit run with its
/// own `take_while(is_ascii_digit)`, provably redundant work and, worse, a
/// second definition of "where the integer part ends" that could silently
/// drift from this one if either were ever edited without the other (a
/// future digit-separator or different-digit-set change to just this
/// function's loop, for instance, wouldn't be caught by any test that
/// only exercises this function in isolation).
fn find_number_end(bytes: &[u8], pos: usize) -> Option<NumberTokenEnd> {
    let mut i = pos;
    if bytes[i] == b'-' {
        i += 1;
    }
    let int_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == int_start {
        return None;
    }
    let int_end = i;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    Some(NumberTokenEnd { int_end, end: i })
}

/// Strip a leading zero from every JSON number token in `s` (`007` ->
/// `7`, `-00` -> `-0`, `007.5` -> `7.5`), leaving string contents/keys and
/// all other structure untouched. Real jq's own number parser tolerates a
/// leading zero (unlike strict RFC 8259 JSON); this repairs just that one
/// divergence before re-validating with `serde_json` (#1094) -- the
/// trailing-garbage/number-range checks that validation exists for still
/// apply to the normalized text exactly as before.
///
/// Delegates string-skipping to [`find_string_end`] and number-token
/// boundaries to [`find_number_end`] (#1154) rather than hand-rolling
/// both scans again -- see the latter's own doc comment for why this
/// matters beyond tidiness.
fn normalize_leading_zero_numbers(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            // `s` has already failed `validate_and_materialize_json`'s
            // stricter check by the time this repair function runs (see
            // `parse_json_value`), so an unterminated string here is a
            // real, reachable case, not just hypothetical -- copy to
            // end-of-input verbatim rather than panicking; the retried
            // validation below will reject the result either way.
            let end = find_string_end(bytes, i).unwrap_or(bytes.len());
            out.extend_from_slice(&bytes[i..end]);
            i = end;
            continue;
        }
        if b == b'-' || b.is_ascii_digit() {
            let start = i;
            let Some(NumberTokenEnd { int_end, end }) = find_number_end(bytes, start) else {
                // Bare `-`, not a number token -- see `find_number_end`'s
                // own doc comment for why this must stay untouched rather
                // than fabricating a digit.
                out.push(b'-');
                i += 1;
                continue;
            };
            // `int_start` is one byte past `start` for a signed token,
            // `start` itself otherwise -- not re-derived from `int_end`
            // via a second scan (#1154 review): `find_number_end` already
            // computed and returned `int_end` directly, so this is the
            // only "where does the integer part start/end" logic in the
            // whole function.
            let int_start = if b == b'-' { start + 1 } else { start };
            let stripped = s[int_start..int_end].trim_start_matches('0');
            out.extend_from_slice(&bytes[start..int_start]);
            out.extend_from_slice(if stripped.is_empty() {
                b"0"
            } else {
                stripped.as_bytes()
            });
            // Fraction and exponent, if any, are contiguous from
            // `int_end` to `end` by construction of `find_number_end` --
            // copied verbatim in one slice, unlike the two separate
            // frac/exp copies an earlier version of this function did.
            out.extend_from_slice(&bytes[int_end..end]);
            i = end;
            continue;
        }
        // Every byte reaching here is copied verbatim, one at a time,
        // whether ASCII structural syntax or (only possible inside a
        // string, already handled above) a multi-byte UTF-8 continuation
        // byte -- `out` ends up byte-identical to `s` except for the
        // leading-zero digits actually stripped above, so `from_utf8`
        // below can never fail.
        out.push(b);
        i += 1;
    }
    String::from_utf8(out).expect("only ever copies s's own bytes verbatim or drops leading ASCII '0' digits, never splits a multi-byte sequence")
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
/// span within the stream, and `json_bytes_to_owned_value` materializes
/// the real result from that span.
///
/// Doesn't *primarily* share span-finding with `find_json_values` (above,
/// #1163 follow-up question), despite both walking a byte string to
/// delimit consecutive JSON values: `find_json_values` is a permissive,
/// jq-compatible scanner (accepts a leading `.` as a number start, doesn't
/// reject trailing garbage) with no `serde_json` dependency at all, while
/// this function's own validation strictness is load-bearing for more than
/// just `--slurpfile`'s CLI-arg error message -- the main input path's own
/// call site (below `evaluate_input_streaming`) depends on this function rejecting
/// everything `find_json_values` would reject, so its own internal
/// `find_json_values` cross-check never diverges.
///
/// It *does* fall back to `find_json_values` on a `serde_json` failure,
/// though (#1243): real jq's own number parser tolerates a leading zero
/// (`007`) that strict JSON doesn't (#1094), and unlike `parse_json_value`'s
/// own `--argjson` retry, there's no cheap single-string
/// `normalize_leading_zero_numbers` fix here -- this function also backs
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
                    .map_err(|de| anyhow::Error::from(MalformedJsonError(de))),
                Err(_) => Err(e),
            }
        }
    }
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
            crate::output::json_bytes_to_owned_value(&bytes[start..end])
                .map_err(|e| anyhow::Error::from(MalformedJsonError(e)))?,
        );
        prev_end = end;
    }

    Ok(values)
}

/// Parse JSON sequence format (RFC 7464) (`--seq`).
/// Input is split on RS (0x1E) characters, each segment parsed as JSON.
/// Parse failures are silently ignored (per RFC 7464 recommendation).
///
/// Preserves number-literal source fidelity the same way `parse_json_value`
/// does for `--argjson` (#1058, extended here to `--seq`, #1093): each
/// segment is already isolated by the RS split, so a validate-then-
/// materialize step can validate and materialize the same segment text
/// directly -- no boundary-tracking needed, unlike `parse_json_stream`
/// above.
///
/// Validates via the crate's own zero-allocation RFC 8259 grammar
/// validator (`json::validate::validate`, already used by `--validate`/
/// `json validate`) rather than `validate_and_materialize_json`'s
/// `serde_json::Value`-tree-based check (#1267) -- `--seq` is a streaming,
/// record-oriented format, so a discarded parse tree's allocation cost is
/// paid once per record rather than a bounded number of times per run the
/// way `--argjson`/`--jsonargs` pay it. Measured (500k-record stream,
/// interleaved A/B, 5 reps -- real jq's own baseline is ~0.47s either way):
/// a real but modest ~10-20% improvement (median ~1.30s -> ~1.15s), not the
/// dramatic win a first, non-interleaved measurement suggested (1.38s ->
/// 1.56s, i.e. apparently *worse* -- sequential-halves noise, the exact
/// trap this repo's own benchmarking guide warns about). The remaining gap
/// to real jq is dominated by something else entirely, out of this issue's
/// own scope.
///
/// This also fixes a real, jq-observable divergence, not just a speed one:
/// `json::validate::validate` is a pure grammar check with no `f64`-range
/// rejection, unlike `serde_json::Value` -- so a magnitude-overflowing
/// literal (`1e400`) that used to silently drop as an "unparseable" record
/// now materializes correctly (`1E+400`, matching real jq's own primary-
/// document-input behavior, live-verified) instead. `validate_json_str`
/// (used by `--argjson` and this function's own leading-zero retry below)
/// keeps its `serde_json::Value`-based magnitude rejection unchanged --
/// this fix is scoped to `parse_json_seq`'s own hot path only; see #1267's
/// own text for why extending it to the shared `--argjson` helper too
/// isn't attempted here (that path's error-message text is user-visible
/// and untouched by this issue).
///
/// Also retries a failed segment with its leading zeros stripped before
/// giving up on it, mirroring `parse_json_value`'s own `--argjson` retry
/// (#1094, extended here to `--seq`, #1243) -- real jq's own number parser
/// tolerates a leading zero (`007`) that strict JSON doesn't, so `007e5`
/// silently dropping as an unparseable record (RFC 7464's own recommended
/// failure mode -- but only for content that's actually malformed, not for
/// a shape jq itself accepts) was a real, jq-observable divergence, not a
/// spelling nit.
/// Build the values (and, when `!slurp`, one `(source, line)` location per
/// value) for `--seq` input across the whole file list at once (#1571).
///
/// Concatenates every file's raw content, in order, into one string and
/// runs [`parse_json_seq_with_ends`] over it exactly once -- matching real
/// jq's own `-s` reader, which treats the entire multi-file input as one
/// continuous RFC 7464 byte stream and doesn't stop scanning a record at a
/// file boundary. This is what makes `seq_trailing_record_is_dropped`'s own
/// ambiguous-bare-number-at-EOF check come out correct too: run against the
/// *true* full stream, it can only ever see genuine end-of-input, never a
/// false EOF at some earlier file's own end -- no separate per-file
/// special-casing needed for that interaction.
///
/// `!slurp` locations still need each value's own *file* and *file-local*
/// line, not the byte offset's raw position in the throwaway `combined`
/// string, so each value's own end offset (computed by
/// `parse_json_seq_with_ends` in the same pass as the value itself -- never
/// a separately-scanned list to fall out of sync with) is mapped back to
/// whichever file's own byte range contains it via `partition_point`, since
/// both `file_ends` and each value's own `end` are non-decreasing (a
/// boundary-spanning record is attributed to the file its *end* falls in --
/// matching #1568's own precedent for the stream's trailing record, which
/// uses the same rule). `current` caches the one `LineCounter` in use,
/// replaced only when `partition_point` reports a new file index -- since
/// `end` values are non-decreasing, that index is too, so this never
/// backtracks to a file already passed.
fn build_seq_values(
    raw_inputs: &[(Option<usize>, String)],
    locations: &mut InputLocations,
    slurp: bool,
) -> Vec<OwnedValue> {
    let (combined, file_ends) = concat_with_file_ends(raw_inputs);
    let parsed = parse_json_seq_with_ends(&combined);

    if !slurp {
        remap_ends_to_locations(
            parsed.iter().map(|&(_, end)| end),
            raw_inputs,
            &file_ends,
            locations,
        );
    }

    parsed.into_iter().map(|(v, _)| v).collect()
}

/// Build the values and one `(source, line)` location per value for
/// raw-input (`-R`) mode across the whole file list at once (#1809).
///
/// Mirrors [`build_seq_values`]'s concatenate-then-remap pattern (sharing
/// its [`concat_with_file_ends`]/[`remap_ends_to_locations`] helpers
/// directly): real jq's `-R` reader treats multiple files as one
/// continuous byte stream for line-splitting too -- confirmed live against
/// jq 1.7.1 that a file's own unterminated trailing line joins with the
/// next file's first line, the same way `--seq` joins a boundary-split
/// record. This can't reuse `parse_json_seq_with_ends` itself
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

/// Concatenate every file's raw content, in order, into one string,
/// recording each file's own cumulative end offset in the result -- shared
/// by [`build_seq_values`] and [`build_raw_input_values`], both of which
/// treat the whole multi-file input as one continuous byte stream for
/// their own purposes (RFC 7464 records / newline-delimited lines).
fn concat_with_file_ends(raw_inputs: &[(Option<usize>, String)]) -> (String, Vec<usize>) {
    let mut combined = String::new();
    let mut file_ends: Vec<usize> = Vec::with_capacity(raw_inputs.len());
    for (_, raw) in raw_inputs {
        combined.push_str(raw);
        file_ends.push(combined.len());
    }
    (combined, file_ends)
}

/// Map each `end` offset in `ends` (non-decreasing, an exclusive position
/// within the `combined` stream `file_ends` was built from -- see
/// [`concat_with_file_ends`]) to its owning file and file-local line
/// number, pushing one `(source, line)` location per `end` onto
/// `locations` in the same order. Shared by [`build_seq_values`] and
/// [`build_raw_input_values`].
///
/// A value/line ending *exactly* at a file boundary is attributed to the
/// file *starting* there, not the file ending there: `partition_point`'s
/// `fe <= end` predicate (not `fe < end`) is what makes that call, since
/// `file_ends[i]` is both file `i`'s own exclusive end and file `i+1`'s
/// start offset -- an `end` equal to that offset means the byte the
/// record/line's own trailing delimiter occupies is the *first* byte of
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
fn remap_ends_to_locations(
    ends: impl Iterator<Item = usize>,
    raw_inputs: &[(Option<usize>, String)],
    file_ends: &[usize],
    locations: &mut InputLocations,
) {
    let mut current: Option<(usize, LineCounter<'_>)> = None;
    for end in ends {
        let file_idx = file_ends
            .partition_point(|&fe| fe <= end)
            .min(file_ends.len().saturating_sub(1));
        if current.as_ref().map(|(idx, _)| *idx) != Some(file_idx) {
            current = Some((
                file_idx,
                LineCounter::new(raw_inputs[file_idx].1.as_bytes()),
            ));
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

/// Parse `--seq` content into values paired with each surviving segment's
/// own trimmed end byte offset, both computed in the same pass so they can
/// never fall out of sync (#1808 code review, following #1571's own
/// cross-file fix: an earlier version scanned end offsets separately via a
/// now-removed `json_seq_ends` helper and reconciled the two lists by comparing lengths
/// afterward -- when a record anywhere in a multi-file stream was
/// genuinely malformed, that reconciliation's "counts disagree" fallback
/// attributed *every* value across the *whole* stream to the last file's
/// last line, not just the dropped record's own, silently misreporting the
/// filename in an error message for records that had nothing to do with
/// the drop). [`build_seq_values`] is this function's only remaining
/// caller; a `parse_json_seq(s) -> Vec<OwnedValue>` values-only wrapper
/// existed here too until its own last caller (this function's own
/// predecessor) was replaced -- removed rather than kept unused, per its
/// own three unit tests now calling this function directly instead.
fn parse_json_seq_with_ends(s: &str) -> Vec<(OwnedValue, usize)> {
    let bytes = s.as_bytes();
    const RS: u8 = 0x1e;

    // Byte offset where each `s.split('\x1E')` segment begins: 0, then one
    // past each RS byte -- enumerates the identical segments, in the
    // identical order, `s.split('\x1E')` always has (segment 0, anything
    // before the very first RS byte and ordinarily empty, included).
    let mut segment_starts = vec![0usize];
    segment_starts.extend(
        bytes
            .iter()
            .enumerate()
            .filter(|(_, &b)| b == RS)
            .map(|(i, _)| i + 1),
    );
    let last_idx = segment_starts.len() - 1;
    // `segment_starts` always holds a leading 0, so more than one entry
    // means at least one RS byte was found.
    let has_rs = segment_starts.len() > 1;

    // The stream's own trailing record, when real jq's incremental reader
    // never resolves it (malformed, or an ambiguous bare number at true
    // EOF -- see `seq_trailing_record_is_dropped`), is silently dropped
    let mut results = Vec::new();
    for (i, &start) in segment_starts.iter().enumerate() {
        let raw_end = segment_starts
            .get(i + 1)
            .map_or(bytes.len(), |&next| next - 1);

        // Full (both-sides) trim to decide parse-eligibility -- matches
        // the original per-segment `segment.trim()` check exactly.
        let raw_segment = &s[start..raw_end];
        let segment = raw_segment.trim();
        if segment.is_empty() {
            continue;
        }
        // Anything before the *first* RS byte is not a record at all, and
        // real jq discards it however well-formed it is: `printf '"a"
        // \x1e3\n' | jq --seq -c .` prints only `3`. Segment 0 is that
        // prefix whenever the input has an RS byte -- and when it has none,
        // segment 0 *is* the whole input and #1525's abandonment rule below
        // owns it instead. (A review caught this: multi-value support made
        // a pre-existing single-value leak here emit every value in the
        // prefix, widening the divergence rather than introducing it.)
        if has_rs && i == 0 {
            continue;
        }
        // *No RS byte anywhere* is #1525's abandonment case: real jq drops
        // the entire input, however well-formed its content is (`printf
        // '1 2\n' | jq --seq -c .` prints nothing). Checked *before* the
        // scan below, which would otherwise tokenize and validate the whole
        // input only for it to be discarded. With an RS byte present there
        // is nothing extra to do here -- `seq_record_scan` decides per value
        // what a record yields, and returns nothing for one it drops.
        if !has_rs {
            continue;
        }
        // Scanned once and reused -- a review found this record being
        // re-scanned up to three times, each pass allocating and
        // re-tokenizing. The *untrimmed* text, so the pending-token rule can
        // see the whitespace that trimming would remove. `i < last_idx`:
        // this record is followed by another RS byte, the boundary at which
        // a bare literal is truncatable.
        let ranges = seq_record_scan(raw_segment, i < last_idx).values;

        // `ranges` is exactly the set of values `seq_record_scan` decided
        // this record yields -- empty for one it dropped, so there is no
        // separate "does this parse" gate here.
        for &(vs, ve) in &ranges {
            let Ok(v) = crate::output::json_bytes_to_owned_value(&raw_segment.as_bytes()[vs..ve])
            else {
                // Parses but will not decode (#1247): dropped, exactly as
                // the pre-#1723 path dropped it.
                continue;
            };
            // Every value reports its own end. Handing the record's trimmed
            // end to the *last* range was wrong once the pending-token rule
            // could pop the real last token: the survivor then inherited a
            // line past the value that was dropped (a review found
            // `\x1e"a"\n\n\n2\x1e"z"\n` reporting line 3 where jq reports
            // line 1). For a single-value record the two are identical
            // anyway, since a value's end *is* the record's trimmed end.
            let value_end = start + ve;
            results.push((v, value_end));
        }
        // Genuine parse failures are silently ignored per RFC 7464
    }

    results
}

/// Every JSON value a `--seq` record yields, as byte ranges into the record's
/// *untrimmed* text.
///
/// **Conservative by construction: this never emits a value real jq would
/// not.** A record whose tokens are not cleanly whitespace-separated, or any
/// of whose tokens is not legal JSON, yields *nothing* -- the pre-#1723
/// whole-record drop.
///
/// That conservatism is the point, and it was learned the hard way. An
/// earlier version of this function tried to reproduce jq's own resync
/// (emit the values before a malformed suffix, skip the bad token, carry
/// on). jq's `--seq` reader is a streaming lexer/parser with error recovery,
/// and a token scanner cannot imitate it: the attempt **fabricated output**,
/// emitting values real jq never emits. `\x1e1-2\n` is the clearest case --
/// jq lexes `1-2` as one malformed number and prints nothing, while a
/// scanner reads `1` then `-2` and prints both. `\x1e1null\n`, `\x1e12true\n`
/// and `\x1etrue1\n` all fail the same way, and `\x1e5-3 7\n` printed `-3`
/// out of the middle of a bad token.
///
/// Requiring whitespace after every token is what makes that impossible:
/// the adjacency a scanner mis-splits is exactly what this rejects. The cost
/// is that a record jq *can* partially read is dropped instead -- a
/// divergence in the safe direction, recorded in
/// `docs/compliance/jq/limitations.md` rather than papered over. Reaching
/// jq's own answer needs its incremental parser's failure classification,
/// which is what #1723 asks for and is tracked there.
fn seq_record_scan(raw_segment: &str, rs_terminated: bool) -> SeqRecordScan {
    let bytes = raw_segment.as_bytes();
    let mut values = Vec::new();
    let mut pos = 0;

    while pos < bytes.len() {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let Some(end) = scan_one_json_token(bytes, pos) else {
            return SeqRecordScan::dropped();
        };
        if !seq_value_is_valid(&raw_segment[pos..end]) {
            return SeqRecordScan::dropped();
        }
        // The delimiter check, and it applies only to *pending* tokens.
        //
        // A string, object or array carries its own closing delimiter, so
        // adjacency cannot mis-split it and jq reads `{"a":1}{"b":2}` and
        // `"a""b"` as two values apiece -- requiring whitespace after those
        // dropped records real jq reads fully (a review measured 332 such
        // regressions). A number or literal has no closing delimiter: it is
        // "pending" until something confirms it, which is exactly where a
        // scanner and jq's lexer can disagree.
        //
        // What confirms one, oracle-verified: whitespace, or the start of a
        // self-delimiting value (`1[1]` and `1"a"` are two values in jq).
        // Anything else -- another digit, a letter, `-`, or structural
        // punctuation -- means this record is not a clean sequence of
        // values, and guessing at the boundary is what fabricated output.
        if seq_token_is_pending(bytes, pos)
            && bytes
                .get(end)
                .is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b'"' | b'{' | b'['))
        {
            return SeqRecordScan::dropped();
        }
        values.push((pos, end));
        pos = end;
    }

    // The only value-level drop that survives: a *pending* token (number or
    // bare literal) ending the record with no whitespace after it is
    // ambiguous to jq's incremental scanner, which cannot rule out more
    // input arriving. `\x1e1 2` is `1`; `\x1e1 2 ` (trailing space) is `1`
    // and `2`; `\x1etrue\x1e3` is `3` alone.
    let trailing_unresolved = match values.last() {
        Some(&(vs, ve)) if !seq_pending_token_is_terminated(bytes, vs, ve, rs_terminated) => {
            values.pop();
            true
        }
        Some(_) => false,
        None => true,
    };
    SeqRecordScan {
        values,
        trailing_unresolved,
    }
}

/// What [`seq_record_scan`] found in one `--seq` record.
struct SeqRecordScan {
    /// Byte ranges, into the record's *untrimmed* text, of every value the
    /// record yields.
    values: Vec<(usize, usize)>,
    /// Whether the record ends unresolved -- a malformed record, or one
    /// whose final value is an ambiguous bare number. This is what leaves
    /// real jq's incremental parser with no EOF position to report
    /// (`<unknown>`, #1542), and it is *not* the same question as "yielded
    /// nothing": `\x1e"a" 2` yields `"a"` and still ends unresolved.
    trailing_unresolved: bool,
}

impl SeqRecordScan {
    /// A record that yields nothing and ends unresolved.
    fn dropped() -> Self {
        Self {
            values: Vec::new(),
            trailing_unresolved: true,
        }
    }
}

/// Whether the token at `start..end` is *pending* -- a number or a bare
/// `true`/`false`/`null`, neither of which carries a closing delimiter.
///
/// Strings, objects and arrays are self-delimiting and are never pending:
/// `"`, `}` and `]` end them unambiguously.
fn seq_token_is_pending(bytes: &[u8], start: usize) -> bool {
    matches!(bytes[start], b'-' | b'.' | b'0'..=b'9' | b't' | b'f' | b'n')
}

/// Whether a *pending* token ending a record was confirmed.
///
/// jq's incremental scanner cannot rule out more input arriving for a token
/// with no closing delimiter, so one butting against a record boundary may
/// be abandoned -- but **numbers and literals differ on which boundary**,
/// and conflating them cost data in both directions:
///
/// | token | at an RS byte | at real EOF |
/// |-------|---------------|-------------|
/// | number (`1`) | abandoned | abandoned |
/// | literal (`true`) | abandoned | **kept** |
/// | string/array/object | kept | kept |
///
/// Checking numbers alone left 92 shapes emitting a `true` jq drops;
/// checking both at every boundary then dropped `printf '\x1etrue'`, which
/// jq prints. Both were found by differential sweeps, not by reasoning.
fn seq_pending_token_is_terminated(
    bytes: &[u8],
    start: usize,
    end: usize,
    rs_terminated: bool,
) -> bool {
    if bytes.get(end).is_some_and(u8::is_ascii_whitespace) {
        return true;
    }
    match bytes[start] {
        // A number is truncatable at *either* boundary: more digits could
        // always follow, so jq abandons it at an RS byte and at real EOF
        // alike (`\x1e1` and `\x1e1\x1e3` both drop the `1`).
        b'-' | b'.' | b'0'..=b'9' => false,
        // A bare literal is truncatable only at an RS byte. At real EOF jq
        // has seen all the input there will ever be, so `true` is complete:
        // `printf '\x1etrue'` prints `true`, while `\x1etrue\x1e3` prints
        // only `3`. Treating the two boundaries alike dropped the EOF case
        // and lost data real jq (and `main`) keep.
        b't' | b'f' | b'n' => !rs_terminated,
        // Self-delimiting: nothing to confirm.
        _ => true,
    }
}

/// Whether one value out of a `--seq` record is legal JSON, allowing the
/// same leading-zero form (`007e5`) `--seq` has accepted since #1243.
///
/// Normalization is needed *here* and only here: `validate::validate` is
/// strict RFC 8259 and rejects a leading zero, while the semi-indexer behind
/// `json_bytes_to_owned_value` already reads `0007` as `7` on its own -- so
/// the value-building side needs no fallback.
fn seq_value_is_valid(value_text: &str) -> bool {
    if validate::validate(value_text.as_bytes()).is_ok() {
        return true;
    }
    let normalized = normalize_leading_zero_numbers(value_text);
    normalized != value_text && validate_json_str(&normalized).is_ok()
}

/// Whether `raw`'s trailing `--seq` record (RFC 7464, everything after the
/// last RS byte) leaves real jq's own incremental parser with no EOF
/// position to report -- distinct from a malformed record *elsewhere* in
/// the stream, which a later valid record still resyncs after (#1542,
/// oracle-verified: `\x1e1\n\x1e{"a":1\n\x1e3\n` still reports the trailing
/// `3`'s own line; only the stream's actual *last* record can trigger this).
///
/// Two shapes, both silently swallowed by real jq's own `--seq` reader:
///
/// - **Genuinely malformed/truncated** -- [`seq_record_scan`] yields no values
///   (an unterminated string/object, e.g.), matching
///   [`parse_json_seq_with_ends`]'s own silent-drop rule (RFC 7464's
///   recommended failure mode, #1243).
/// - **A bare number with nothing at all after it before EOF** -- valid
///   JSON on its own, but jq's streaming number scanner can't rule out more
///   digits still arriving without seeing a terminating byte (whitespace,
///   or the start of the next token) after the last one it read, so a
///   number that happens to butt right up against real EOF is
///   indistinguishable from one truncated mid-digit. Every other JSON type
///   has its own unambiguous closing delimiter (`"`, `}`, `]`, the last
///   letter of `true`/`false`/`null`) and reports normally even with
///   nothing trailing it -- oracle-verified: `\x1e1\n\x1e2` (bare `2`, no
///   trailing byte at all) reports `<unknown>`, but the same shape with
///   `true`/`"s"`/`{}`/`[]`/`null` in place of `2`, or `2` followed by even
///   one trailing space, all report their own line normally.
///
/// A third shape needs no record-level check at all: content with **no RS
/// byte anywhere**. RFC 7464 requires every record to start with one, so
/// real jq's `--seq` reader never even attempts to read unprefixed text --
/// it's "abandoned" the instant EOF arrives with nothing synced onto,
/// oracle-verified as `<unknown>` regardless of what the unprefixed text
/// actually contains (even fully well-formed JSON). *Empty* content is the
/// one exception: an empty last file has no text to abandon, so it keeps
/// #1520's own plain "empty source, EOF at line 0" rule instead (oracle-
/// verified: an empty last file after a valid one still reports
/// `emptyfile:0`, not `<unknown>`).
fn seq_trailing_record_is_dropped(raw: &str) -> bool {
    let Some((_, tail)) = raw.rsplit_once('\u{1e}') else {
        return !raw.trim().is_empty();
    };
    if tail.trim().is_empty() {
        return false;
    }
    // Dropped when the record ends *unresolved* -- not merely when it
    // yields nothing. `\x1e"a" 2` yields `"a"` and still leaves real jq
    // without an EOF position, because its trailing `2` never resolved
    // (#1542); asking "did it yield anything" got that backwards and broke
    // two location tests. `seq_record_scan`'s rules decide both, including
    // the ambiguous trailing bare number this function used to ask about
    // separately -- now rule 3, applied per value. Scanned on `tail`
    // (untrimmed) so that rule can see the terminating whitespace.
    // `false`: this is the stream's trailing record by construction, so its
    // end is real EOF, never an RS byte.
    seq_record_scan(tail, false).trailing_unresolved
}

/// Whether `--seq -s`'s trailing record, read across every file on the
/// command line as one continuous byte stream (matching real jq's own `-s`
/// reader), leaves real jq's incremental parser with no EOF position to
/// report -- extending [`seq_trailing_record_is_dropped`]'s single-source
/// check across a file boundary (#1550).
///
/// A record's own opening RS byte and its closing bytes can live in
/// different files, so this walks backward one file at a time: any file
/// with no RS byte of its own is exactly that record's own continuation (or
/// a disambiguating trailing byte after an otherwise-bare number, oracle-
/// verified: `\x1e5` in one file plus a lone trailing space in the next
/// still resolves normally, not `<unknown>`) and is folded, byte for byte,
/// onto whatever followed it -- never trimmed or skipped by emptiness --
/// until a file containing an RS byte is reached, at which point the
/// single-source check runs against the reassembled tail. `false` (never
/// dropped) if no file in the whole stream contains an RS byte at all:
/// per [`seq_trailing_record_is_dropped`]'s own third case, that's
/// "abandoned" only when there's non-whitespace content to abandon, so an
/// all-empty stream keeps #1520's own plain "empty source, EOF at line 0"
/// rule instead.
fn seq_stream_trailing_record_is_dropped(raw_inputs: &[(Option<usize>, String)]) -> bool {
    let mut suffix = String::new();
    for (_, raw) in raw_inputs.iter().rev() {
        if raw.contains('\u{1e}') {
            return seq_trailing_record_is_dropped(&format!("{raw}{suffix}"));
        }
        suffix = format!("{raw}{suffix}");
    }
    !suffix.trim().is_empty()
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
    let mut bytes_seen = 0usize;
    for (_, raw) in raw_bytes {
        let bytes = if bytes_seen == 0 {
            raw.strip_prefix(crate::front_matter::UTF8_BOM)
                .unwrap_or(raw)
        } else {
            raw.as_slice()
        };
        for &b in bytes {
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
        bytes_seen += raw.len();
    }
    if bytes_seen == 0 && raw_bytes.is_empty() {
        return None;
    }
    Some(format!(
        "jq: ignoring parse error: Unfinished abandoned text at EOF at line {line}, column {column}"
    ))
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
        values.push(OwnedValue::Array(fields));
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
/// practice: `cursor` is always rooted in `input.to_json()`, a fresh
/// serialization of an already-decoded `OwnedValue` -- a Rust `String`, which
/// by construction cannot hold an undecodable byte sequence, and whose
/// escapes this crate's own serializer writes. Same argument
/// `eval_generic.rs`'s textually-similar bridge relies on (search
/// "defense-in-depth" there). Kept as a reported diagnostic rather than an
/// `unwrap()` so a real failure, if that invariant is ever violated,
/// surfaces as an ordinary `EvalError` instead of a panic.
fn evaluate_input_streaming(
    input: &OwnedValue,
    expr: &jq::Expr,
    _context: &EvalContext,
    at: &ErrorAt<'_>,
    sink: &mut ErrorSink,
    on_value: &mut dyn FnMut(&mut ErrorSink, OwnedValue) -> Result<bool>,
) -> Result<()> {
    let json_str = input.to_json();
    let json_bytes = json_str.as_bytes();
    let index = JsonIndex::build(json_bytes);
    let cursor = index.root(json_bytes);

    let mut write_err: Option<anyhow::Error> = None;
    let control = jq::eval_generic::eval_each_with_cursor(expr, cursor, &mut |result| {
        match materialize_stream_item(result, sink, at) {
            // Nothing to write: either genuinely no value, or a failure this
            // call already reported to `sink` (same "report and keep going"
            // contract the eager path's own arms followed, #355).
            None => true,
            Some(v) => match on_value(sink, v) {
                Ok(keep_going) => keep_going,
                Err(e) => {
                    write_err = Some(e);
                    false
                }
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
        GenericResult::One(v) => {
            sink.materialize(DiagStyle::Jq, generic_to_owned(&v), &at.resolve())
        }
        GenericResult::OneCursor(c) => sink.materialize(
            DiagStyle::Jq,
            generic_to_owned(&succinctly::jq::document::DocumentCursor::value(&c)),
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
        GenericResult::LazySeq(seq) => match seq.materialize_atomic() {
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

/// Evaluate expression against raw JSON bytes, returning lazy JqValues.
///
/// This function preserves original number formatting by working directly
/// with the source bytes instead of parsing through serde_json.
fn evaluate_bytes_lazy<'a>(
    json_bytes: &'a [u8],
    expr: &jq::Expr,
    index: &'a JsonIndex,
    at: &InputLocation,
    sink: &mut ErrorSink,
) -> Vec<JqValue<'a, Vec<u64>>> {
    let cursor = index.root(json_bytes);
    // Use eval_with_cursor to preserve cursor context for position-based navigation
    let result = eval_with_cursor(expr, cursor);
    generic_result_to_jq_values(result, cursor, at, sink)
}

/// Convert GenericResult to JqValue, preserving lazy cursor references.
///
/// This is similar to query_result_to_jq_values but works with the
/// cursor-aware GenericResult type from eval_generic.
fn generic_result_to_jq_values<'a, W: Clone + AsRef<[u64]>>(
    result: GenericResult<StandardJson<'a, W>>,
    cursor: JsonCursor<'a, W>,
    at: &InputLocation,
    sink: &mut ErrorSink,
) -> Vec<JqValue<'a, W>> {
    match result {
        GenericResult::One(v) => match standard_json_to_jq_value(v, &cursor) {
            Ok(jq_value) => vec![jq_value],
            Err(e) => {
                sink.report(DiagStyle::Jq, &e, at);
                vec![]
            }
        },
        // OneCursor: directly use the cursor - most memory efficient for unchanged values
        GenericResult::OneCursor(c) => vec![JqValue::Cursor(c)],
        // Stops at the first element that fails to decode, keeping the
        // already-converted prefix -- matching how an ordinary `error`/
        // `break` mid-generator stops the rest of a stream elsewhere in this
        // evaluator (#1164), not a "skip the bad one and keep going"
        // semantic (no precedent for that at this granularity).
        GenericResult::Many(vs) => {
            let mut out = Vec::new();
            for v in vs {
                match standard_json_to_jq_value(v, &cursor) {
                    Ok(jq_value) => out.push(jq_value),
                    Err(e) => {
                        sink.report(DiagStyle::Jq, &e, at);
                        break;
                    }
                }
            }
            out
        }
        // ManyCursor: same lazy-cursor efficiency as OneCursor, per element.
        GenericResult::ManyCursor(cs) => cs.into_iter().map(JqValue::Cursor).collect(),
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
        } => vec![JqValue::LazyKeysArray { fields, collapse }],
        GenericResult::LazyKeys {
            fields,
            sorted: true,
            collapse,
        } => match effective_keys(&fields, collapse) {
            Ok(mut keys) => {
                keys.sort();
                vec![JqValue::from_owned(OwnedValue::Array(
                    keys.into_iter().map(OwnedValue::String).collect(),
                ))]
            }
            Err(e) => {
                sink.report(DiagStyle::Jq, &e, at);
                vec![]
            }
        },
        // Same laziness as `LazyKeys` above, for array `keys`/
        // `keys_unsorted` (#684): `write_json`/`print_json` write the
        // `[0,1,...,len-1]` digits directly, no `Vec<OwnedValue::Int>`.
        GenericResult::LazyIndexRange(len) => vec![JqValue::LazyIndexRange(len)],
        // `JqValue` needs no new variant for a composed `map` chain (#724,
        // #725): `JqValue::Array` already stores per-element cursors (its
        // own "Phase 1 Lazy Optimization"), so materializing once here and
        // wrapping via `from_owned` reuses the existing `write_json` path
        // entirely.
        GenericResult::LazySeq(seq) => match seq.materialize_atomic() {
            Ok(v) => vec![JqValue::from_owned(v)],
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
        GenericResult::Owned(v) => vec![JqValue::from_owned(v)],
        GenericResult::ManyOwned(vs) => vs.into_iter().map(JqValue::from_owned).collect(),
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
        // (#400, #494).
        GenericResult::Partial(vs, jq::Control::Error(e)) => {
            sink.report(DiagStyle::Jq, &e, at);
            vs.into_iter().map(JqValue::from_owned).collect()
        }
        GenericResult::Partial(vs, jq::Control::Break(label)) => {
            sink.report_break(DiagStyle::Jq, &label, at);
            vs.into_iter().map(JqValue::from_owned).collect()
        }
        GenericResult::Partial(vs, jq::Control::Halt(code)) => {
            sink.request_halt(code);
            vs.into_iter().map(JqValue::from_owned).collect()
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
    Ok(match value {
        StandardJson::Null => JqValue::Null,
        StandardJson::Bool(b) => JqValue::Bool(b),
        StandardJson::Number(n) => {
            // Use RawNumber to preserve original formatting like "4e4"
            JqValue::RawNumber(n.raw_bytes())
        }
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
            let items: Vec<JqValue<'a, W>> = elements.cursor_iter().map(JqValue::Cursor).collect();
            JqValue::Array(items)
        }
        StandardJson::Object(fields) => {
            // LAZY: Store cursor references instead of materializing values
            let mut map: IndexMap<String, JqValue<'a, W>> = IndexMap::new();
            let mut remaining = fields;
            while let Some((f, rest)) = remaining.uncons() {
                // A key that isn't `StandardJson::String` at all is a
                // structurally malformed key, not a decode failure. This used
                // to `continue`, dropping the field and everything that
                // depended on it while `length` went on counting it (#1194).
                let key = match f.key() {
                    StandardJson::String(s) => match s.as_str() {
                        Ok(cow) => cow.to_string(),
                        Err(e) => {
                            return Err(EvalError::decode_failure(format!("{e} in object key")))
                        }
                    },
                    _ => return Err(EvalError::malformed_json_text(parent_cursor.text())),
                };
                // Use cursor for value instead of materializing
                map.insert(key, JqValue::Cursor(f.value_cursor()));
                remaining = rest;
            }
            // A child with no sibling to pair as a value: the object is
            // malformed and `uncons` would drop it silently (#1194).
            if remaining.ends_unpaired() {
                return Err(EvalError::malformed_json_text(parent_cursor.text()));
            }
            JqValue::Object(map)
        }
        // See `eval_generic::to_owned_at_depth`'s own `is_error` arm
        // (#1194/#1247): a structurally malformed value -- one the
        // semi-index accepted as a span but could not classify as any JSON
        // token -- raises rather than becoming `null`.
        StandardJson::Error(msg) => return Err(EvalError::new(msg.to_string())),
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
        return Err(MalformedJsonError(EvalError::new(
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
    let raw_str = if config.raw_output {
        value.as_str()
    } else {
        None
    };
    if let Some(s) = &raw_str {
        reject_raw_output0_nul(s, config)?;
    }

    // #1913: see `should_write_seq_separator`'s own doc comment. This is
    // `write_output_jq_value`'s copy of the identical fix in `write_output`
    // below -- currently dead in practice, since this function's only call
    // site is gated by `can_use_lazy_path`, which already excludes
    // `args.seq` entirely (`--seq` always takes the materializing path
    // through `write_output` instead). Kept anyway as a correctness
    // guarantee that doesn't depend on that gate staying in place: if a
    // future change ever let `--seq` reach the lazy path, this would
    // already be right instead of silently reintroducing #1913.
    if should_write_seq_separator(config, raw_str.is_some()) {
        out.write_all(&[ASCII_RS])?;
    }

    if let Some(s) = raw_str {
        out.write_all(s.as_bytes())?;
        write_terminator(out, config)?;
        return Ok(());
    }

    // For jq_compat mode, use the jq-compatible formatter (reformats numbers)
    // For preserve mode (!jq_compat), use the preserve formatter (keeps original number format)
    if !config.sort_keys && !config.color_output {
        if config.jq_compat {
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
        } else {
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
        }
    } else {
        // For complex output (pretty-print, sort_keys, colors), materialize
        // first -- and surface a decode failure rather than printing the
        // empty string it used to become (#1247). `anyhow` is the only error
        // channel this writer has; the message is preserved verbatim.
        let owned = value
            .materialize()
            .map_err(|e| anyhow::anyhow!("{}", e.message))?;
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

fn write_output<W: Write>(out: &mut W, value: &OwnedValue, config: &OutputConfig) -> Result<()> {
    // Raw-output string, if any -- resolved and NUL-checked before
    // writing *any* byte of this record, including the `--seq` RS
    // separator below (same reasoning as `write_output_jq_value`'s
    // sibling fix). A single RS write below then covers both the raw and
    // non-raw cases, rather than one copy per branch.
    let raw_str = if config.raw_output {
        match value {
            OwnedValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    } else {
        None
    };
    if let Some(s) = raw_str {
        reject_raw_output0_nul(s, config)?;
    }

    // #1913: see `should_write_seq_separator`'s own doc comment.
    if should_write_seq_separator(config, raw_str.is_some()) {
        out.write_all(&[ASCII_RS])?;
    }

    if let Some(s) = raw_str {
        out.write_all(s.as_bytes())?;
        write_terminator(out, config)?;
        return Ok(());
    }

    // Not computed until this non-raw fallthrough path actually needs it
    // (code review, #1830) -- the raw-output early return above never
    // touches it, so computing it unconditionally wasted a full
    // JSON-formatting pass on every raw-output string reaching this
    // writer (the "materializing path", `-n`/`inputs`/DSV input).
    let output = format_json(value, config);
    out.write_all(output.as_bytes())?;
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
            return Cow::Owned(match OwnedValue::from_number_bytes(raw) {
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
        // `DBL_MAX` substitution. JSON's number grammar has no NaN spelling,
        // so a NaN literal can't reach this function at all -- only via
        // `format_float`.
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
        // hand-rolled copy of the same two branches.
        if f.is_finite() {
            format!("{f}")
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
        match core::str::from_utf8(raw) {
            Ok(s) => Cow::Borrowed(s),
            Err(_) => Cow::Owned(String::from_utf8_lossy(raw).into_owned()),
        }
    }

    fn format_float(&self, f: f64) -> String {
        // Same split as `JqCompatFormatter::format_float` above (#1087): a
        // computed Infinity has no source literal for preserve mode to keep
        // either, so both formatters need the identical jq-real-output rule
        // here, not just the jq_compat one.
        if f.is_finite() {
            format!("{f}")
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

/// Forward-scan mirror of [`preceding_gap_ok`], used to catch the trailing
/// half of #1643's own deferred gap (#1676): a `,`/`:` sitting between a
/// container's last real child and its closing bracket (`{"a":1,}`,
/// `[1,2,]`), or inside an apparently-empty container (`{,}`, `[,]`).
///
/// Scans *forward* from `start` -- a position every caller here obtains
/// cheaply (a scalar child's own `text_range().1`, or `open_pos + 1` for a
/// genuinely empty container) -- exactly as bounded as `preceding_gap_ok`'s
/// backward scan: it stops the instant it reaches real content, so its cost
/// is the size of the gap itself, never the container's remaining text.
///
/// Returns the position of the first non-whitespace, non-delimiter byte on
/// success (so the caller can confirm it's the expected closing bracket),
/// or `None` if the gap holds a doubled delimiter or one that doesn't match
/// `expected`.
fn following_gap_ok(text: &[u8], start: usize, expected: Option<u8>) -> Option<usize> {
    let mut i = start;
    let mut found = None;
    while i < text.len() {
        match text[i] {
            b @ (b',' | b':') => {
                if found.is_some() {
                    return None; // doubled delimiter
                }
                found = Some(b);
                i += 1;
            }
            b if b.is_ascii_whitespace() => i += 1,
            _ => break, // reached real content -- a value, or the closing bracket
        }
    }
    (found == expected).then_some(i)
}

/// Whether a container closes cleanly once its last real child's own text
/// has ended at `gap_start` (or `gap_start` is `open_pos + 1`, for a
/// genuinely empty container): no trailing `,`/`:` before the matching
/// closing bracket (#1676).
///
/// Deliberately narrower than a complete fix: `gap_start` must already be
/// known cheaply. Callers only have that when the last child is a scalar
/// (via `scalar_end_pos`, an O(1) length read, not a fresh scan) or when
/// the container is empty. When the last child is itself a container,
/// finding *its* closing bracket costs exactly the O(subtree) walk this
/// function's sibling `preceding_gap_ok` was written to avoid, so callers
/// skip this check in that case rather than pay for it -- the same
/// "tracked as a follow-up, not folded in here" trade #1643 already made,
/// narrowed by one more case rather than eliminated.
fn trailing_gap_ok(text: &[u8], gap_start: usize, close_char: u8) -> bool {
    matches!(following_gap_ok(text, gap_start, None), Some(pos) if text.get(pos) == Some(&close_char))
}

/// Cheap end position (one past the last byte) of an already-resolved
/// scalar `value` known to start at `start` -- `None` for a container or
/// unresolved value, which callers treat as "can't determine, skip"
/// (#1676's own deferred case for a container, or a defensively-skipped
/// unresolved cursor).
///
/// Deliberately doesn't call `JsonCursor::text_range()`: every caller here
/// already resolved `value` via `.value_at(start)` (reusing the *same*
/// `start` a caller-side `text_position()` already paid for -- see
/// `check_preceding_delimiter`'s own doc comment on why a second lookup
/// measured 19-30% slower for #1643), so re-deriving the position and
/// re-dispatching on the leading byte a second time would silently
/// reintroduce that exact regression. `raw_bytes()` on a string/number is
/// already the value's own exact text span; `true`/`false`/`null` are
/// fixed-width literals.
fn scalar_end_pos<W: AsRef<[u64]>>(start: usize, value: &StandardJson<'_, W>) -> Option<usize> {
    match value {
        StandardJson::String(s) => Some(start + s.raw_bytes().len()),
        StandardJson::Number(n) => Some(start + n.raw_bytes().len()),
        StandardJson::Bool(true) => Some(start + 4),
        StandardJson::Bool(false) => Some(start + 5),
        StandardJson::Null => Some(start + 4),
        StandardJson::Array(_) | StandardJson::Object(_) | StandardJson::Error(_) => None,
    }
}

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
    let Some(start) = child_cursor.text_position() else {
        return Ok(None);
    };
    let expected = if index == 0 { None } else { Some(b',') };
    if !preceding_gap_ok(child_cursor.text(), start, expected) {
        return Err(MalformedJsonError(EvalError::malformed_json_text(child_cursor.text())).into());
    }
    Ok(Some(start))
}

/// Recursively validate every object/array in `cursor`'s subtree for a
/// missing or doubled `,`/`:` (#1643), via [`preceding_gap_ok`] -- the same
/// check `print_json`'s object/array arms perform, needed again here for a
/// caller that doesn't go through `print_json` at all.
///
/// [`crate::output::json_bytes_to_owned_value`]'s own doc comment says
/// plainly that it "performs no validation of its own" and expects the
/// caller to have done so first; [`parse_json_stream`]'s fallback below
/// (which backs `-S`, `-C`, and `--slurp` -- none of which route through
/// `print_json`) was calling it directly on unvalidated bytes, so `{"a"
/// 1}` would silently materialize as `{"a":1}` under those flags even
/// though the default `sjq -c .` path already rejects it since #1643.
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
fn validate_json_delimiters<W: AsRef<[u64]>>(
    cursor: &JsonCursor<'_, W>,
    depth: usize,
) -> core::result::Result<(), EvalError> {
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
                // re-derive it -- see `scalar_end_pos`'s own doc comment.
                last_gap_end = start.and_then(|s| scalar_end_pos(s, &child.value_at(s)));
            }
            // #1676: trailing `,` (`[1,2,]`) or a stray `,` in an
            // apparently-empty array (`[,]`) -- `last_gap_end` is `None`
            // both when there's nothing to check yet and when the last
            // element is itself a container (`scalar_end_pos`'s own doc
            // comment explains why that case is skipped, not just unknown).
            if let Some(open_pos) = cursor.text_position() {
                let gap_start = if saw_any {
                    last_gap_end
                } else {
                    Some(open_pos + 1)
                };
                if let Some(gap_start) = gap_start {
                    if !trailing_gap_ok(cursor.text(), gap_start, b']') {
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
                    value_start.and_then(|s| scalar_end_pos(s, &value_cursor.value_at(s)));
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
                    if !trailing_gap_ok(cursor.text(), gap_start, b'}') {
                        return Err(EvalError::malformed_json_text(cursor.text()));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// [`crate::output::json_bytes_to_owned_value`], preceded by the #1643
/// delimiter check that function's own doc comment says its caller must
/// supply. Used only by [`parse_json_stream`]'s fallback -- every other
/// caller of the unchecked version re-serializes an already-validated span
/// (`--argjson`'s retry re-validates its normalized copy against
/// `serde_json` before ever reaching it; the primary lazy input path
/// validates via `print_json` instead).
fn json_bytes_to_owned_value_checked(bytes: &[u8]) -> core::result::Result<OwnedValue, EvalError> {
    let index = JsonIndex::build(bytes);
    let cursor = index.root(bytes);
    validate_json_delimiters(&cursor, 0)?;
    generic_to_owned(&cursor.value())
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
    anyhow::ensure!(
        level < MAX_VALUE_TREE_DEPTH,
        "{}",
        succinctly::jq::nesting_depth_exceeded_message(MAX_VALUE_TREE_DEPTH)
    );
    let compact = config.compact;
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
                StandardJson::Number(n) => {
                    out.write_all(formatter.format_raw_number(n.raw_bytes()).as_bytes())?;
                }
                StandardJson::String(s) => {
                    let raw = s.raw_bytes();
                    // Zero-copy optimization: if no escapes (backslash) and not ASCII mode,
                    // output raw bytes directly without decode/encode roundtrip.
                    // This is valid because JSON strings without backslashes need no normalization.
                    let content = &raw[1..raw.len().saturating_sub(1)]; // Content between quotes
                    if !config.ascii_output && !content.contains(&b'\\') {
                        // Zero-copy: output raw bytes directly (includes quotes)
                        out.write_all(raw)?;
                    } else if let Ok(decoded) = s.as_str() {
                        // Has escapes or ASCII mode - decode and re-encode for normalization
                        out.write_all(b"\"")?;
                        let escaped = if config.ascii_output {
                            escape_json_string_ascii(&decoded)
                        } else {
                            escape_json_string(&decoded)
                        };
                        out.write_all(escaped.as_bytes())?;
                        out.write_all(b"\"")?;
                    } else {
                        // Decode failed - output raw bytes as fallback
                        out.write_all(raw)?;
                    }
                }
                StandardJson::Array(elements) => {
                    if elements.is_empty() {
                        // #1676: a stray `,`/`:` inside an apparently-empty
                        // array (`[,]`) -- cheap since there's no subtree
                        // to scan, just the gap between `[` and `]`.
                        if let Some(open_pos) = container_pos {
                            if !trailing_gap_ok(c.text(), open_pos + 1, b']') {
                                return Err(MalformedJsonError(EvalError::malformed_json_text(
                                    c.text(),
                                ))
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
                        // #1676: tracks the last element's own end position
                        // (cheap for a scalar; `None` for a container --
                        // see `scalar_end_pos`'s own doc comment) so a
                        // trailing `,` before `]` (`[1,2,]`) can be caught
                        // below, before anything is written. Reuses `pos`
                        // (already resolved by `check_preceding_delimiter`)
                        // via `value_at` rather than `child_cursor.value()`,
                        // which would re-derive the same position a second
                        // time -- see `scalar_end_pos`'s own doc comment on
                        // why that matters on this hot path specifically.
                        let mut last_gap_end = None;
                        for (i, child_cursor) in elements.cursor_iter().enumerate() {
                            let pos = check_preceding_delimiter(&child_cursor, i)?;
                            array_scratch
                                .push((child_cursor.bp_position(), pos.unwrap_or(usize::MAX)));
                            last_gap_end =
                                pos.and_then(|s| scalar_end_pos(s, &child_cursor.value_at(s)));
                        }
                        if let Some(gap_start) = last_gap_end {
                            if !trailing_gap_ok(c.text(), gap_start, b']') {
                                array_scratch.truncate(base);
                                return Err(MalformedJsonError(EvalError::malformed_json_text(
                                    c.text(),
                                ))
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
                            if !trailing_gap_ok(c.text(), open_pos + 1, b'}') {
                                return Err(MalformedJsonError(EvalError::malformed_json_text(
                                    c.text(),
                                ))
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
                        // `--preserve-input` (`!jq_compat`) keeps every
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
                        // the last field's *value* rather than an element.
                        let mut last_gap_end = None;
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
                                return Err(MalformedJsonError(EvalError::malformed_json_text(
                                    frame.text,
                                ))
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
                                    return Err(MalformedJsonError(
                                        EvalError::malformed_json_text(frame.text),
                                    )
                                    .into());
                                }
                            }
                            // #1676: same last-child tracking as the array
                            // arm, reusing `value_start` via `value_at`
                            // rather than `field.value_cursor().value()`,
                            // which would re-derive it.
                            last_gap_end = value_start
                                .and_then(|s| scalar_end_pos(s, &field.value_cursor().value_at(s)));
                            let (raw, escaped) = k.raw_and_escaped();
                            scratch.push(PreparedField {
                                key_bp: field.key_cursor().bp_position(),
                                value_bp: field.value_cursor().bp_position(),
                                value_start: value_start.unwrap_or(usize::MAX),
                                raw,
                                escaped,
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
                            return Err(MalformedJsonError(EvalError::malformed_json_text(
                                frame.text,
                            ))
                            .into());
                        }
                        if let Some(gap_start) = last_gap_end {
                            if !trailing_gap_ok(frame.text, gap_start, b'}') {
                                scratch.truncate(base);
                                return Err(MalformedJsonError(EvalError::malformed_json_text(
                                    frame.text,
                                ))
                                .into());
                            }
                        }
                        let collapsed = if config.jq_compat {
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
                    return Err(MalformedJsonError(EvalError::malformed_json_text(c.text())).into());
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
            if arr.is_empty() {
                out.write_all(b"[]")?;
            } else if compact {
                out.write_all(b"[")?;
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 {
                        out.write_all(b",")?;
                    }
                    print_json(
                        out,
                        v,
                        formatter,
                        config,
                        level + 1,
                        scratch,
                        array_scratch,
                        None,
                    )?;
                }
                out.write_all(b"]")?;
            } else {
                out.write_all(b"[")?;
                out.write_all(separator.as_bytes())?;
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 {
                        out.write_all(b",")?;
                        out.write_all(separator.as_bytes())?;
                    }
                    out.write_all(next_indent.as_bytes())?;
                    print_json(
                        out,
                        v,
                        formatter,
                        config,
                        level + 1,
                        scratch,
                        array_scratch,
                        None,
                    )?;
                }
                out.write_all(separator.as_bytes())?;
                out.write_all(current_indent.as_bytes())?;
                out.write_all(b"]")?;
            }
        }
        JqValue::Object(obj) => {
            if obj.is_empty() {
                out.write_all(b"{}")?;
            } else if compact {
                out.write_all(b"{")?;
                for (i, (k, v)) in obj.iter().enumerate() {
                    if i > 0 {
                        out.write_all(b",")?;
                    }
                    out.write_all(b"\"")?;
                    let escaped = if config.ascii_output {
                        escape_json_string_ascii(k)
                    } else {
                        escape_json_string(k)
                    };
                    out.write_all(escaped.as_bytes())?;
                    out.write_all(b"\":")?;
                    print_json(
                        out,
                        v,
                        formatter,
                        config,
                        level + 1,
                        scratch,
                        array_scratch,
                        None,
                    )?;
                }
                out.write_all(b"}")?;
            } else {
                out.write_all(b"{")?;
                out.write_all(separator.as_bytes())?;
                for (i, (k, v)) in obj.iter().enumerate() {
                    if i > 0 {
                        out.write_all(b",")?;
                        out.write_all(separator.as_bytes())?;
                    }
                    out.write_all(next_indent.as_bytes())?;
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
                    )?;
                }
                out.write_all(separator.as_bytes())?;
                out.write_all(current_indent.as_bytes())?;
                out.write_all(b"}")?;
            }
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
                return Err(MalformedJsonError(EvalError::malformed_json_text(tail.text())).into());
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
                        return Err(MalformedJsonError(EvalError::malformed_json_text(
                            key_cursor.text(),
                        ))
                        .into());
                    };
                    let raw = k.raw_bytes();
                    let content = &raw[1..raw.len().saturating_sub(1)];
                    if !config.ascii_output && !content.contains(&b'\\') {
                        out.write_all(raw)?;
                    } else if let Ok(decoded) = k.as_str() {
                        out.write_all(b"\"")?;
                        let escaped = if config.ascii_output {
                            escape_json_string_ascii(&decoded)
                        } else {
                            escape_json_string(&decoded)
                        };
                        out.write_all(escaped.as_bytes())?;
                        out.write_all(b"\"")?;
                    } else {
                        out.write_all(raw)?;
                    }
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
                        return Err(MalformedJsonError(EvalError::malformed_json_text(
                            key_cursor.text(),
                        ))
                        .into());
                    };
                    let raw = k.raw_bytes();
                    let content = &raw[1..raw.len().saturating_sub(1)];
                    if !config.ascii_output && !content.contains(&b'\\') {
                        out.write_all(raw)?;
                    } else if let Ok(decoded) = k.as_str() {
                        out.write_all(b"\"")?;
                        let escaped = if config.ascii_output {
                            escape_json_string_ascii(&decoded)
                        } else {
                            escape_json_string(&decoded)
                        };
                        out.write_all(escaped.as_bytes())?;
                        out.write_all(b"\"")?;
                    } else {
                        out.write_all(raw)?;
                    }
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
        float_style: FloatStyle::Shortest,
        control_escape: ControlEscape::Jq,
        // Only meaningful alongside `ControlEscape::Yq` (see
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

    /// #1154: direct unit coverage for the extracted token-boundary
    /// finder, independent of the CLI-level `--argjson` tests (which
    /// already cover `normalize_leading_zero_numbers`'s end-to-end
    /// behavior via subprocess spawns invisible to `cargo llvm-cov`).
    /// Checks both `int_end` and `end` (#1154 review) -- `int_end` is
    /// what the caller actually needs to strip a leading zero without a
    /// second scan, so a regression there wouldn't be caught by only
    /// checking the overall token length.
    #[test]
    fn test_find_number_end_1154() {
        fn ends(bytes: &[u8], pos: usize) -> Option<(usize, usize)> {
            find_number_end(bytes, pos).map(|e| (e.int_end, e.end))
        }

        // Plain and negative integers: int_end == end, no frac/exp.
        assert_eq!(ends(b"42", 0), Some((2, 2)));
        assert_eq!(ends(b"-42", 0), Some((3, 3)));
        // Leading zeros tolerated (that's the whole point of this scan).
        assert_eq!(ends(b"007", 0), Some((3, 3)));
        // Fraction and exponent, each optionally signed: int_end stops at
        // the integer digit run, end includes the rest.
        assert_eq!(ends(b"1.5", 0), Some((1, 3)));
        assert_eq!(ends(b"1e10", 0), Some((1, 4)));
        assert_eq!(ends(b"1E+10", 0), Some((1, 5)));
        assert_eq!(ends(b"1.5e-10", 0), Some((1, 7)));
        // A dangling '.' or 'e' with no digits after it still consumes
        // the marker itself, matching the lenient original behavior --
        // the retried `serde_json` validation rejects the result either
        // way, so this scan doesn't need to. This is the concrete example
        // #1218 uses to illustrate the crate's 4-way number-scanner
        // divergence -- `succinctly::json::light::number_literal_end`
        // deliberately rejects the same `5e`/`1E` shape outright instead.
        assert_eq!(ends(b"5.", 0), Some((1, 2)));
        assert_eq!(ends(b"5e", 0), Some((1, 2)));
        // Stops at the token's own end, not the end of a larger buffer.
        assert_eq!(ends(b"42,43", 0), Some((2, 2)));
        // A bare '-' with nothing (or nothing digit-shaped) after it is
        // not a number token at all.
        assert_eq!(ends(b"-", 0), None);
        assert_eq!(ends(b"-,", 0), None);
        // Starting position other than 0, with a multi-digit int_end.
        assert_eq!(ends(b"[1,007]", 3), Some((6, 6)));
    }

    #[test]
    fn test_normalize_leading_zero_numbers_1154() {
        assert_eq!(normalize_leading_zero_numbers("007"), "7");
        assert_eq!(normalize_leading_zero_numbers("-00"), "-0");
        assert_eq!(normalize_leading_zero_numbers("007.5"), "7.5");
        assert_eq!(normalize_leading_zero_numbers("007e10"), "7e10");
        assert_eq!(normalize_leading_zero_numbers("-007.5e-10"), "-7.5e-10");
        // String contents and object keys are never touched, even when
        // they look like a leading-zero number themselves.
        assert_eq!(
            normalize_leading_zero_numbers(r#"{"007":"007"}"#),
            r#"{"007":"007"}"#
        );
        // An escaped quote inside a string doesn't end the string early.
        assert_eq!(
            normalize_leading_zero_numbers(r#"["a\"b",007]"#),
            r#"["a\"b",7]"#
        );
        // A bare '-' is left untouched, not fabricated into "-0" (#1094
        // review finding, the bug this whole delegation exists to avoid
        // reintroducing).
        assert_eq!(normalize_leading_zero_numbers("-"), "-");
        assert_eq!(normalize_leading_zero_numbers("[-,1]"), "[-,1]");
        // A normal, already-valid number is unaffected.
        assert_eq!(normalize_leading_zero_numbers("42"), "42");
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

    /// #1243: same leading-zero tolerance as `--slurpfile`/`--slurp` above,
    /// for `--seq` (RFC 7464) -- previously silently dropped the whole
    /// record (its own documented "ignore parse failures" fallback) instead
    /// of accepting it the way real jq does.
    #[test]
    fn test_parse_json_seq_tolerates_leading_zero_1243() {
        let values: Vec<OwnedValue> = parse_json_seq_with_ends("\x1E007e5\n")
            .into_iter()
            .map(|(v, _)| v)
            .collect();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].to_json(), "7E+5");
    }

    /// Control: a genuinely malformed `--seq` record is still silently
    /// dropped (RFC 7464's own documented behavior), not resurrected by the
    /// leading-zero retry.
    #[test]
    fn test_parse_json_seq_still_drops_genuine_malformed_record_1243() {
        let values: Vec<OwnedValue> = parse_json_seq_with_ends("\x1E{invalid\n\x1E5\n")
            .into_iter()
            .map(|(v, _)| v)
            .collect();
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
        let values: Vec<OwnedValue> = parse_json_seq_with_ends("\x1E1e400\n")
            .into_iter()
            .map(|(v, _)| v)
            .collect();
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
        let values = build_seq_values(&raw_inputs, &mut locations, false);
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
        let values = build_seq_values(&raw_inputs, &mut locations, false);
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
        let values = build_seq_values(&raw_inputs, &mut locations, false);
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
        let values = build_seq_values(&raw_inputs, &mut locations, false);
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
        assert!(
            err.message.contains("invalid UTF-8") && err.message.contains("object key"),
            "{err:?}"
        );
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

        let json: &[u8] = b"{\"a\": 1, \"b\": 2}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let jq_value = standard_json_to_jq_value(value, &cursor).unwrap();
        let JqValue::Object(map) = jq_value else {
            panic!("expected an object");
        };
        assert_eq!(map.keys().collect::<Vec<_>>(), vec!["a", "b"]);
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

    /// #1194: `MalformedJsonError` exists to be `downcast_ref`'d out of an
    /// `anyhow::Error` in `run_jq`, which reads its inner `EvalError` and
    /// never formats the wrapper. `Display` is still required by
    /// `std::error::Error`, so pin that it renders the message rather than
    /// something like `MalformedJsonError(..)` -- a future `anyhow` context
    /// chain would print it.
    #[test]
    fn test_malformed_json_error_displays_its_message_1194() {
        let wrapped = MalformedJsonError(EvalError::new("Invalid JSON text: whatever"));
        assert_eq!(wrapped.to_string(), "Invalid JSON text: whatever");

        // And it survives the round trip `run_jq` actually performs.
        let boxed: anyhow::Error = wrapped.into();
        let recovered = boxed
            .downcast_ref::<MalformedJsonError>()
            .expect("run_jq recovers this by downcast");
        assert_eq!(recovered.0.message, "Invalid JSON text: whatever");
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
        assert!(matches!(out.as_slice(), [JqValue::RawNumber(_)]));
        assert!(sink.hit());
    }
}
