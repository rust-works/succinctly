//! jq-compatible command runner for succinctly.
//!
//! This module implements a jq-compatible CLI interface using the succinctly
//! JSON semi-indexing and jq expression evaluator.

use anyhow::{Context, Result};
use indexmap::IndexMap;
use std::collections::BTreeMap;
use std::io::{BufWriter, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use succinctly::dsv::{build_index as build_dsv_index, DsvConfig, DsvRows};
use succinctly::jq::document::{collapsed_fields, effective_keys};
use succinctly::jq::eval_generic::{
    eval_with_cursor, to_owned as generic_to_owned, GenericResult, MAX_NESTING_DEPTH,
};
use succinctly::jq::{
    self, format_number_jq_compat, nonfinite_display_string, EvalError, Expr, JqSemantics, JqValue,
    OwnedValue, Program,
};
use succinctly::json::light::{JsonCursor, StandardJson};
use succinctly::json::validate::{self, ValidationError};
use succinctly::json::JsonIndex;

use super::JqCommand;
use crate::output::{
    self, escape_json_string, escape_json_string_ascii, exit_codes, ColorScheme, ControlEscape,
    DiagStyle, ErrorSink, FloatStyle, InputLocation, JsonFormatOpts,
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
        | Expr::Format(_)
        | Expr::Builtin(_) => expr,
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

/// Whether any two key spans are byte-identical.
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
    // Sorting keeps the wide case out of the quadratic loop: on a 10 MB
    // document whose root object is wide, an unbounded pairwise scan
    // measured 1240% slower than not collapsing at all.
    let mut spans: Vec<&[u8]> = prepared.iter().map(|field| field.raw).collect();
    spans.sort_unstable();
    spans.windows(2).any(|w| w[0] == w[1])
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
    // JSON grammar makes every key a string; anything else means the cursor
    // did not resolve, and the value is written on its own as before.
    let StandardJson::String(key) = frame.cursor(field.key_bp).value() else {
        return Ok(());
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

    // Parse the filter as a full program (with module directives)
    let program = jq::parse_program(&filter_str).map_err(|e| {
        eprintln!("jq: compile error: {e}");
        anyhow::anyhow!("compile error")
    })?;

    // Create module loader and process imports/includes
    let mut module_loader = ModuleLoader::new(&args.library_path);
    let expr = module_loader.process_program(&program).map_err(|e| {
        eprintln!("jq: module error: {e}");
        anyhow::anyhow!("module error")
    })?;

    // Build the $ARGS special variable
    let args_value = build_args_var(&context);

    // Substitute variables from context into the expression
    // First substitute regular named variables, then add $ARGS
    let mut all_vars: Vec<(&str, &OwnedValue)> =
        context.named.iter().map(|(k, v)| (k.as_str(), v)).collect();
    all_vars.push(("ARGS", &args_value));

    let expr = jq::substitute_vars(&expr, all_vars);

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
    let mut out = BufWriter::new(stdout.lock());

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

                    // Evaluate expression on this row
                    let results = evaluate_input(&row_value, &expr, &context, &at, &mut sink)?;

                    for result in results {
                        had_output = true;
                        if args.exit_status {
                            last_output = Some(result.clone());
                        }
                        write_output(&mut out, &result, &output_config)?;
                    }
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

        // Check if we can use the identity fast path (raw bytes output, no materialization)
        let use_identity_fast_path = expr.is_identity() && output_config.can_use_raw_identity();

        for (idx, raw) in raw_inputs.iter().enumerate() {
            let filename: Option<String> = files.get(idx).map(|p| p.to_string_lossy().to_string());
            // Validate JSON if --validate flag is set
            if args.validate {
                if let Err(exit_code) = validate_json_input(raw, filename.as_deref()) {
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
                let results = evaluate_bytes_lazy(json_bytes, &expr, &index, &at, &mut sink);

                // Consume results to free memory after each value is written
                for result in results {
                    had_output = true;
                    // For exit_status tracking, we need to check the last value
                    if args.exit_status {
                        last_output = Some(result.materialize());
                    }
                    write_output_jq_value(&mut out, &result, &output_config)?;
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
            Ok(Err(e)) => return Err(e),
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
                let results = evaluate_input(
                    &OwnedValue::Null,
                    &expr,
                    &context,
                    &ErrorAt::Live(&locations),
                    &mut sink,
                )?;
                for result in results {
                    had_output = true;
                    last_output = Some(result.clone());
                    write_output(&mut out, &result, &output_config)?;
                }
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
                    let results = evaluate_input(
                        &input,
                        &expr,
                        &context,
                        &ErrorAt::Live(&locations),
                        &mut sink,
                    )?;
                    for result in results {
                        had_output = true;
                        last_output = Some(result.clone());
                        write_output(&mut out, &result, &output_config)?;
                    }
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
                let results = evaluate_input(input, &expr, &context, &at, &mut sink)?;

                for result in results {
                    had_output = true;
                    last_output = Some(result.clone());
                    write_output(&mut out, &result, &output_config)?;
                }
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

    // Collect raw input from files or stdin
    let raw_inputs = if files.is_empty() {
        match read_stdin() {
            Ok(s) => vec![(None, s)],
            Err(e) => return Ok(Err(e)),
        }
    } else {
        let mut inputs = Vec::new();
        for (idx, path) in files.iter().enumerate() {
            match read_file(path) {
                Ok(s) => inputs.push((Some(idx), s)),
                Err(e) => return Ok(Err(e)),
            }
        }
        inputs
    };

    let mut locations = InputLocations::new(
        files
            .iter()
            .map(|p| Some(p.to_string_lossy().to_string()))
            .collect(),
    );

    // jq -R -s: the entire input (all files concatenated) becomes a single
    // string; no line splitting and no array wrap.
    if args.raw_input && args.slurp && args.input_dsv.is_none() {
        let mut combined = String::new();
        for (_, raw) in &raw_inputs {
            combined.push_str(raw);
        }
        // One value spanning everything: jq names its last content line.
        locations.push(0, content_lines(&combined));
        return Ok(Ok((vec![OwnedValue::String(combined)], locations)));
    }

    // Process based on input mode
    let mut values = Vec::new();

    for (file_idx, raw) in raw_inputs {
        let src = file_idx.unwrap_or(0);

        if let Some(delimiter) = args.input_dsv {
            // DSV input: each row becomes a JSON array of strings
            let parsed = parse_dsv_input(&raw, delimiter);
            // One row per line (approximate for embedded newlines; jq has no
            // DSV input mode, so there is no oracle to match here).
            for line in 1..=parsed.len() {
                locations.push(src, line);
            }
            values.extend(parsed);
        } else if args.raw_input {
            // Raw input: each line becomes a string
            for (line_idx, line) in raw.lines().enumerate() {
                values.push(OwnedValue::String(line.to_string()));
                locations.push(src, line_idx + 1);
            }
        } else if args.seq {
            // JSON sequence input (RFC 7464): split on RS, ignore parse failures
            let parsed = parse_json_seq(&raw);
            locations.extend_from_ends(src, &raw, &json_seq_ends(raw.as_bytes()), parsed.len());
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
            values.extend(parsed);
        }

        debug_assert_eq!(locations.len(), values.len(), "one location per value");
    }

    // Slurp mode: wrap all inputs in an array
    if args.slurp {
        // One value covering every input: jq names the line the last of them
        // ended on.
        let last = locations.last();
        Ok(Ok((
            vec![OwnedValue::Array(values)],
            InputLocations::single(last),
        )))
    } else {
        Ok(Ok((values, locations)))
    }
}

/// 1-based number of the last line carrying content.
///
/// What jq's marker reports for modes that collapse the whole input into one
/// value (`-R -s`): a trailing newline does not open a new line.
fn content_lines(raw: &str) -> usize {
    raw.lines().count().max(1)
}

/// Exclusive end offsets of the RS-delimited records in a `--seq` stream.
///
/// Trailing whitespace is trimmed so the line lands on the record itself rather
/// than on the separator that follows it, matching jq.
fn json_seq_ends(raw: &[u8]) -> Vec<usize> {
    const RS: u8 = 0x1e;
    let starts: Vec<usize> = raw
        .iter()
        .enumerate()
        .filter(|(_, &b)| b == RS)
        .map(|(i, _)| i)
        .collect();

    (0..starts.len())
        .map(|i| {
            // A record runs up to the next separator, or to end of input.
            let mut end = starts.get(i + 1).copied().unwrap_or(raw.len());
            while end > 0 && raw[end - 1].is_ascii_whitespace() {
                end -= 1;
            }
            end
        })
        .collect()
}

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
    /// to point at (`-n`), which jq renders as `<unknown>`.
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

    /// Locations for a single value at an already-resolved location.
    ///
    /// Always pushes exactly one `per_value` entry, even when `at.line` is
    /// `None` (an empty/whitespace-only `--slurp`, whose single wrapped-array
    /// value still has no content line to name). Slurp mode always produces
    /// exactly one value, so `get_inputs`'s `values`/`locations` invariant
    /// ("one location per value") must hold here unconditionally -- the
    /// seeding `.zip()` in `run_jq` silently truncates to the shorter side on
    /// a mismatch instead of erroring, so a skipped push here previously lost
    /// the whole slurped document to `input`/`inputs` (debug-build panic on
    /// the `debug_assert_eq!` guarding that zip, release-build silent empty
    /// output instead of jq's own `[]`) -- confirmed live against jq 1.7.1:
    /// `printf '' | jq -c -s '., inputs'` prints `[]`.
    fn single(at: InputLocation) -> Self {
        let mut locations = Self::new(vec![at.file.clone()]);
        locations.push(0, at.line.unwrap_or(0));
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
    /// `ends` is already non-decreasing (both of this crate's own callers --
    /// `find_json_values`/`json_seq_ends` -- are single left-to-right scans),
    /// so one shared `LineCounter` keeps this whole loop O(n) rather than the
    /// O(n^2) a per-value `line_at` rescan from byte 0 produced (#1213).
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

    /// Location of the last value, or `<unknown>` if there were none.
    fn last(&self) -> InputLocation {
        self.get(self.per_value.len().saturating_sub(1))
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
    /// name; [`resolve`](Self::resolve) turns it back.
    fn per_value(&self) -> &[(u32, u32)] {
        &self.per_value
    }

    /// Turn a raw `(source, line)` -- as handed back by
    /// `jq::current_input_location` -- into a printable location.
    fn resolve(&self, src: u32, line: u32) -> InputLocation {
        InputLocation::at(
            self.files.get(src as usize).and_then(Option::as_deref),
            line as usize,
        )
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
        let last_src = self.files.len().saturating_sub(1) as u32;
        Some(match self.per_value.last() {
            Some(&(src, line)) if src == last_src => (last_src, line),
            _ => (last_src, 0),
        })
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
/// `evaluate_input` reports at most once per call: its `Error`, `Break`,
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

/// Read stdin to string.
fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("Failed to read from stdin")?;
    Ok(buf)
}

/// Read a file to string.
fn read_file(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path.display()))
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
/// order [`find_json_values`]/`json_seq_ends` already produce their offsets
/// in, since both are themselves single left-to-right scans.
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
        let first_byte = bytes[pos];

        // Determine end of this JSON value
        let end = match first_byte {
            b'{' | b'[' => {
                // Object or array - find matching close
                find_matching_close(bytes, pos)
            }
            b'"' => {
                // String - find end quote
                find_string_end(bytes, pos)
            }
            b't' | b'f' | b'n' => {
                // true, false, null
                find_literal_end(bytes, pos)
            }
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
        };

        match end {
            Some(end) => {
                values.push((start, end));
                pos = end;
            }
            None => return Err(start),
        }
    }

    Ok(values)
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
fn validate_and_materialize_json(s: &str) -> serde_json::Result<OwnedValue> {
    validate_json_str(s)?;
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
/// path already uses, see `evaluate_input` above).
fn parse_json_value(s: &str) -> Result<OwnedValue> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(OwnedValue::Null);
    }

    match validate_and_materialize_json(s) {
        Ok(v) => Ok(v),
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
            Ok(crate::output::json_bytes_to_owned_value(s.as_bytes()))
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
/// document-input path (see the `evaluate_input` call site above, reached
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
/// call site (below `evaluate_input`) depends on this function rejecting
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
                Ok(spans) => Ok(spans
                    .into_iter()
                    .map(|(start, end)| {
                        crate::output::json_bytes_to_owned_value(&bytes[start..end])
                    })
                    .collect()),
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
        values.push(crate::output::json_bytes_to_owned_value(&bytes[start..end]));
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
fn parse_json_seq(s: &str) -> Vec<OwnedValue> {
    let mut values = Vec::new();

    // Split on RS character (0x1E)
    for segment in s.split('\x1E') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }

        // Try to parse as JSON, silently ignore failures
        if validate::validate(segment.as_bytes()).is_ok() {
            values.push(crate::output::json_bytes_to_owned_value(segment.as_bytes()));
        } else {
            let normalized = normalize_leading_zero_numbers(segment);
            if normalized != segment && validate_json_str(&normalized).is_ok() {
                values.push(crate::output::json_bytes_to_owned_value(segment.as_bytes()));
            }
            // Genuine parse failures are silently ignored per RFC 7464
        }
    }

    values
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

/// Evaluate the expression against an input value.
///
/// An uncaught error is reported to `sink` and yields no values, so evaluation
/// continues with the next input the way jq does; `sink` then drives the exit
/// code (#355).
fn evaluate_input(
    input: &OwnedValue,
    expr: &jq::Expr,
    _context: &EvalContext,
    at: &ErrorAt<'_>,
    sink: &mut ErrorSink,
) -> Result<Vec<OwnedValue>> {
    // Convert OwnedValue to JSON bytes for indexing
    let json_str = input.to_json();
    let json_bytes = json_str.as_bytes();

    // Build index and evaluate
    let index = JsonIndex::build(json_bytes);
    let cursor = index.root(json_bytes);

    // Use eval_with_cursor to preserve cursor context for position-based navigation
    // (at_offset, at_position builtins)
    let result = eval_with_cursor(expr, cursor);

    // Convert result to Vec<OwnedValue>
    match result {
        GenericResult::One(v) => Ok(vec![generic_to_owned(&v)]),
        GenericResult::OneCursor(c) => Ok(vec![generic_to_owned(&c.value())]),
        GenericResult::Many(vs) => Ok(vs.iter().map(generic_to_owned).collect()),
        GenericResult::ManyCursor(cs) => {
            Ok(cs.iter().map(|c| generic_to_owned(&c.value())).collect())
        }
        // Fallback: materialize. This runner boundary never sees a
        // fast-pathed `keys`/`keys_unsorted | length`/`.[]`/`.[n]`/`first`/
        // `last` — those are fully resolved inside the evaluator's `Pipe`
        // dispatch before it gets here — so this only fires for `keys`/
        // `keys_unsorted` alone, or piped into something else (`map`,
        // `select`, ...). Sort iff `sorted` (#683), matching eager `Keys`.
        GenericResult::LazyKeys {
            fields,
            sorted,
            collapse,
        } => {
            let mut keys = effective_keys(&fields, collapse);
            if sorted {
                keys.sort();
            }
            Ok(vec![OwnedValue::Array(
                keys.into_iter().map(OwnedValue::String).collect(),
            )])
        }
        // Same reasoning as `LazyKeys` above, for array `keys`/
        // `keys_unsorted` (#684).
        GenericResult::LazyIndexRange(len) => Ok(vec![OwnedValue::Array(
            (0..len).map(|i| OwnedValue::Int(i as i64)).collect(),
        )]),
        // Same reasoning as `LazyKeys`/`LazyIndexRange` above, for a
        // composed `map` chain (#724, #725) that never resolved into a
        // narrower shape before reaching this materializing boundary.
        GenericResult::LazySeq(seq) => match seq.materialize_atomic() {
            Ok(v) => Ok(vec![v]),
            Err(jq::Control::Error(e)) => {
                sink.report(DiagStyle::Jq, &e, &at.resolve());
                Ok(vec![])
            }
            Err(jq::Control::Break(label)) => {
                sink.report_break(DiagStyle::Jq, &label, &at.resolve());
                Ok(vec![])
            }
            Err(jq::Control::Halt(code)) => {
                sink.request_halt(code);
                Ok(vec![])
            }
        },
        GenericResult::None => Ok(vec![]),
        GenericResult::Error(e) => {
            sink.report(DiagStyle::Jq, &e, &at.resolve());
            Ok(vec![])
        }
        GenericResult::Owned(v) => Ok(vec![v]),
        GenericResult::ManyOwned(vs) => Ok(vs),
        GenericResult::Break(label) => {
            sink.report_break(DiagStyle::Jq, &label, &at.resolve());
            Ok(vec![])
        }
        // `halt`/`halt_error` (#791): not a diagnostic, so no `sink.report*`
        // call — `request_halt` records the exit code for the loop above to
        // short-circuit on, without touching `hit`/`report_count`.
        GenericResult::Halt(code) => {
            sink.request_halt(code);
            Ok(vec![])
        }
        // The outputs already produced no longer vanish behind the failure
        // (#400, #494): report the diagnostic (which drives the exit code
        // via `sink`), but still return the prefix for the caller to print.
        GenericResult::Partial(vs, jq::Control::Error(e)) => {
            sink.report(DiagStyle::Jq, &e, &at.resolve());
            Ok(vs)
        }
        GenericResult::Partial(vs, jq::Control::Break(label)) => {
            sink.report_break(DiagStyle::Jq, &label, &at.resolve());
            Ok(vs)
        }
        GenericResult::Partial(vs, jq::Control::Halt(code)) => {
            sink.request_halt(code);
            Ok(vs)
        }
    }
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
        // #1385: a duplicate key would be emitted twice by the lazy
        // writer, which streams raw key bytes without a collapse step. Probe
        // first and stay lazy only when the object is clean -- a repeated
        // key materializes the collapsed list instead, keeping the fast path
        // for every ordinary document.
        GenericResult::LazyKeys {
            fields,
            sorted: false,
            collapse,
        } if !collapse || collapsed_fields(&fields).is_none() => {
            vec![JqValue::LazyKeysArray(fields)]
        }
        GenericResult::LazyKeys {
            fields,
            sorted: false,
            collapse,
        } => vec![JqValue::from_owned(OwnedValue::Array(
            effective_keys(&fields, collapse)
                .into_iter()
                .map(OwnedValue::String)
                .collect(),
        ))],
        GenericResult::LazyKeys {
            fields,
            sorted: true,
            collapse,
        } => {
            let mut keys = effective_keys(&fields, collapse);
            keys.sort();
            vec![JqValue::from_owned(OwnedValue::Array(
                keys.into_iter().map(OwnedValue::String).collect(),
            ))]
        }
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
fn standard_json_to_jq_value<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    _parent_cursor: &JsonCursor<'a, W>,
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
                    .map_err(|e| EvalError::new(format!("{e}")))?
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
            for f in fields {
                // A key that isn't `StandardJson::String` at all is a
                // structurally malformed key, not a decode failure -- out of
                // scope here (#1194's territory), same as before this fix.
                let key = match f.key() {
                    StandardJson::String(s) => match s.as_str() {
                        Ok(cow) => cow.to_string(),
                        Err(e) => return Err(EvalError::new(format!("{e} in object key"))),
                    },
                    _ => continue,
                };
                // Use cursor for value instead of materializing
                map.insert(key, JqValue::Cursor(f.value_cursor()));
            }
            JqValue::Object(map)
        }
        StandardJson::Error(_) => JqValue::Null,
    })
}

/// Write a single output JqValue (preserves number formatting when possible).
fn write_output_jq_value<Out: Write, Wrd: Clone + AsRef<[u64]>>(
    out: &mut Out,
    value: &JqValue<'_, Wrd>,
    config: &OutputConfig,
) -> Result<()> {
    // In seq mode, prepend RS (Record Separator) before each value
    if config.seq {
        out.write_all(&[ASCII_RS])?;
    }

    // Handle raw output for strings
    if config.raw_output {
        if let Some(s) = value.as_str() {
            out.write_all(s.as_bytes())?;
            write_terminator(out, config)?;
            return Ok(());
        }
    }

    // For jq_compat mode, use the jq-compatible formatter (reformats numbers)
    // For preserve mode (!jq_compat), use the preserve formatter (keeps original number format)
    if !config.sort_keys && !config.color_output {
        if config.jq_compat {
            print_json(out, value, &JqCompatFormatter, config, 0, &mut Vec::new())?;
        } else {
            print_json(out, value, &PreserveFormatter, config, 0, &mut Vec::new())?;
        }
    } else {
        // For complex output (pretty-print, sort_keys, colors), materialize first
        let owned = value.materialize();
        out.write_all(format_json(&owned, config).as_bytes())?;
    }

    write_terminator(out, config)?;
    Ok(())
}

/// Write a single output value.
/// ASCII RS (Record Separator) character for JSON sequence format (RFC 7464)
const ASCII_RS: u8 = 0x1E;

fn write_output<W: Write>(out: &mut W, value: &OwnedValue, config: &OutputConfig) -> Result<()> {
    // In seq mode, prepend RS (Record Separator) before each value
    if config.seq {
        out.write_all(&[ASCII_RS])?;
    }

    let output = format_json(value, config);

    // Handle raw output for strings
    if config.raw_output {
        if let OwnedValue::String(s) = value {
            out.write_all(s.as_bytes())?;
            write_terminator(out, config)?;
            return Ok(());
        }
    }

    out.write_all(output.as_bytes())?;
    write_terminator(out, config)?;

    Ok(())
}

/// Write the appropriate line terminator based on config.
fn write_terminator<W: Write>(out: &mut W, config: &OutputConfig) -> Result<()> {
    if config.raw_output0 {
        out.write_all(&[0])?; // NUL byte
    } else if !config.join_output {
        out.write_all(b"\n")?;
    }
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

/// Print a JqValue as JSON using the provided literal formatter.
///
/// This is the unified printer that handles JSON structure (arrays, objects,
/// indentation) while delegating literal formatting to the formatter.
///
/// Guarded against adversarially deep JSON input (thousands of nested
/// arrays/objects, which would otherwise recurse this writer once per
/// nesting level and overflow the stack) by the shared
/// [`succinctly::jq::eval_generic::MAX_NESTING_DEPTH`] ceiling -- see that
/// constant's own doc comment for how 256 was chosen, including this
/// function's own measured debug-build crash boundary of depth 600-700.
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
fn print_json<'a, F, Out, Wrd>(
    out: &mut Out,
    value: &JqValue<'a, Wrd>,
    formatter: &F,
    config: &OutputConfig,
    level: usize,
    scratch: &mut Vec<PreparedField<'a>>,
) -> Result<()>
where
    F: LiteralFormatter,
    Out: Write,
    Wrd: Clone + AsRef<[u64]>,
{
    anyhow::ensure!(
        level < MAX_NESTING_DEPTH,
        "nesting depth exceeds limit of {MAX_NESTING_DEPTH}"
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
            match c.value() {
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
                        out.write_all(b"[]")?;
                    } else if compact {
                        out.write_all(b"[")?;
                        for (i, child_cursor) in elements.cursor_iter().enumerate() {
                            if i > 0 {
                                out.write_all(b",")?;
                            }
                            let child_value = JqValue::Cursor(child_cursor);
                            print_json(out, &child_value, formatter, config, level + 1, scratch)?;
                        }
                        out.write_all(b"]")?;
                    } else {
                        out.write_all(b"[")?;
                        out.write_all(separator.as_bytes())?;
                        for (i, child_cursor) in elements.cursor_iter().enumerate() {
                            if i > 0 {
                                out.write_all(b",")?;
                                out.write_all(separator.as_bytes())?;
                            }
                            out.write_all(next_indent.as_bytes())?;
                            let child_value = JqValue::Cursor(child_cursor);
                            print_json(out, &child_value, formatter, config, level + 1, scratch)?;
                        }
                        out.write_all(separator.as_bytes())?;
                        out.write_all(current_indent.as_bytes())?;
                        out.write_all(b"]")?;
                    }
                }
                StandardJson::Object(fields) => {
                    if fields.is_empty() {
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
                        for field in fields {
                            // The `_` arm is unreachable by JSON grammar --
                            // every key is a string -- and exists only
                            // because `key()` returns the open
                            // `StandardJson` enum. An empty span writes
                            // nothing, matching what this printer did with
                            // a non-string key before #1385.
                            let (raw, escaped) = match field.key() {
                                StandardJson::String(k) => k.raw_and_escaped(),
                                _ => (&b""[..], false),
                            };
                            scratch.push(PreparedField {
                                key_bp: field.key_cursor().bp_position(),
                                value_bp: field.value_cursor().bp_position(),
                                raw,
                                escaped,
                            });
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
                            print_json(out, &child_value, formatter, config, level + 1, scratch)?;
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
                StandardJson::Error(_) => out.write_all(b"null")?,
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
                    print_json(out, v, formatter, config, level + 1, scratch)?;
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
                    print_json(out, v, formatter, config, level + 1, scratch)?;
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
                    print_json(out, v, formatter, config, level + 1, scratch)?;
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
                    print_json(out, v, formatter, config, level + 1, scratch)?;
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
        JqValue::LazyKeysArray(fields) => {
            use succinctly::json::light::StandardJson as SJ;
            if fields.is_empty() {
                out.write_all(b"[]")?;
            } else if compact {
                out.write_all(b"[")?;
                for (i, field) in (*fields).enumerate() {
                    if i > 0 {
                        out.write_all(b",")?;
                    }
                    if let SJ::String(k) = field.key() {
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
                }
                out.write_all(b"]")?;
            } else {
                out.write_all(b"[")?;
                out.write_all(separator.as_bytes())?;
                for (i, field) in (*fields).enumerate() {
                    if i > 0 {
                        out.write_all(b",")?;
                        out.write_all(separator.as_bytes())?;
                    }
                    out.write_all(next_indent.as_bytes())?;
                    if let SJ::String(k) = field.key() {
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
                }
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
        let values = parse_json_seq("\x1E007e5\n");
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].to_json(), "7E+5");
    }

    /// Control: a genuinely malformed `--seq` record is still silently
    /// dropped (RFC 7464's own documented behavior), not resurrected by the
    /// leading-zero retry.
    #[test]
    fn test_parse_json_seq_still_drops_genuine_malformed_record_1243() {
        let values = parse_json_seq("\x1E{invalid\n\x1E5\n");
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
        let values = parse_json_seq("\x1E1e400\n");
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].to_json(), "1E+400");
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

    /// #1192: a key that isn't `StandardJson::String` at all (structurally
    /// malformed, not a decode failure) still silently drops the field --
    /// unchanged from before this fix, and deliberately so (#1194's
    /// territory). A bare numeric key with a valid sibling field reaches
    /// the per-field drop (`{invalid}` alone does not -- see the matching
    /// test in `eval_generic.rs` for why).
    #[test]
    fn test_standard_json_to_jq_value_drops_structurally_malformed_key_1194() {
        let json: &[u8] = b"{123: 1, \"b\": 2}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let jq_value = standard_json_to_jq_value(value, &cursor).unwrap();
        let JqValue::Object(map) = jq_value else {
            panic!("expected an object");
        };
        assert_eq!(map.keys().collect::<Vec<_>>(), vec!["b"]);
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
