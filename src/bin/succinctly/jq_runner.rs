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
use succinctly::jq::document::DocumentFields;
use succinctly::jq::eval_generic::{
    eval_with_cursor, to_owned as generic_to_owned, GenericResult, MAX_NESTING_DEPTH,
};
use succinctly::jq::{
    self, format_number_jq_compat, nonfinite_display_string, Expr, JqSemantics, JqValue,
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
            var,
            init,
            update,
        } => Expr::Reduce {
            input: Box::new(rewrite_namespaced_calls(*input)),
            var,
            init: Box::new(rewrite_namespaced_calls(*init)),
            update: Box::new(rewrite_namespaced_calls(*update)),
        },
        Expr::Foreach {
            input,
            var,
            init,
            update,
            extract,
        } => Expr::Foreach {
            input: Box::new(rewrite_namespaced_calls(*input)),
            var,
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
        | Expr::Slice { .. }
        | Expr::Iterate
        | Expr::RecursiveDescent
        | Expr::Literal(_)
        | Expr::Var(_)
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
        if !args.slurp && !args.null_input {
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
                    let at = InputLocation::at(file.as_deref(), row_idx + 1);
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
    // Both jq_compat (reformatting numbers) and preserve mode (keeping original formatting)
    // use the lazy path for correctness.
    let can_use_lazy_path = !args.slurp
        && !args.raw_input
        && args.input_dsv.is_none()
        && !args.seq // seq input mode parses differently
        && !output_config.sort_keys
        && !output_config.color_output
        && !output_config.ascii_output; // ASCII output requires escaping

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
            let values = find_json_values(raw);
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
                let at = InputLocation::at(filename.as_deref(), line_at(raw, end));
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
        // Original path: parse through serde_json (loses number formatting)
        let (inputs, locations) = match get_inputs(&args) {
            Ok(Ok(inputs)) => inputs,
            Ok(Err(e)) => return Err(e),
            Err(exit_code) => return Ok(exit_code), // Validation error
        };

        for (idx, input) in inputs.iter().enumerate() {
            let at = locations.get(idx);
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

/// Get input values based on arguments.
/// Returns Err(i32) for validation failures (exit code), Ok(Err) for other errors.
fn get_inputs(
    args: &JqCommand,
) -> std::result::Result<Result<(Vec<OwnedValue>, InputLocations)>, i32> {
    // Null input mode: use null as the single input
    if args.null_input {
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
            let ends: Vec<usize> = find_json_values(raw.as_bytes())
                .into_iter()
                .map(|(_, end)| end)
                .collect();
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
    fn single(at: InputLocation) -> Self {
        let mut locations = Self::new(vec![at.file.clone()]);
        if let Some(line) = at.line {
            locations.push(0, line);
        }
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
    fn extend_from_ends(&mut self, src: usize, raw: &str, ends: &[usize], values: usize) {
        if ends.len() == values {
            for &end in ends {
                self.push(src, line_at(raw.as_bytes(), end));
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
            Some(&(src, line)) => InputLocation::at(
                self.files.get(src as usize).and_then(Option::as_deref),
                line as usize,
            ),
            None => InputLocation::unknown(),
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

/// jq's line number for the value whose exclusive end offset is `end` within
/// `bytes`.
///
/// jq's `(at <file>:<line>)` marker names the line on which the input value
/// *ends*, so callers pass the exclusive end offset from [`find_json_values`].
/// jq's counter is the number of `\n` bytes its lexer has consumed by the
/// time the value's boundary is confirmed: every newline strictly before
/// `end`, plus exactly one byte of trailing lookahead if it exists and is a
/// newline. It is zero-based, not one-based — a value ending before any `\n`
/// reports line 0. Only reached on the error path, so a plain scan is cheap
/// enough.
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
fn find_json_values(bytes: &[u8]) -> Vec<(usize, usize)> {
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
            b'-' | b'0'..=b'9' => {
                // Number
                find_number_end(bytes, pos)
            }
            _ => None,
        };

        if let Some(end) = end {
            values.push((start, end));
            pos = end;
        } else {
            // Invalid JSON - skip to next whitespace
            while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() {
                pos += 1;
            }
        }
    }

    values
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

/// Find the end of a number starting at `pos`.
fn find_number_end(bytes: &[u8], pos: usize) -> Option<usize> {
    let mut i = pos;
    // Allow leading minus
    if i < bytes.len() && bytes[i] == b'-' {
        i += 1;
    }
    // Digits, dots, exponents
    while i < bytes.len() {
        match bytes[i] {
            b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-' => i += 1,
            _ => break,
        }
    }
    if i > pos {
        Some(i)
    } else {
        None
    }
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
/// Validates strictly via `serde_json::from_str` first -- not for its
/// resulting `Value` (discarded), but for its error message and its
/// rejection of trailing garbage (`42 garbage`) that `JsonIndex`'s own
/// semi-indexing wouldn't catch on its own. Deliberately parses to
/// `serde_json::Value` here rather than the cheaper `serde::de::IgnoredAny`
/// (which drives the identical grammar/trailing-content check without
/// allocating a parse tree this function then discards): `IgnoredAny`
/// doesn't range-check numbers at all, so a magnitude-overflowing literal
/// (`1e400`) that `Value` correctly rejects ("number out of range") would
/// instead silently reach `json_bytes_to_owned_value` and materialize as
/// `null` -- trading a clear error for silent data loss, a worse outcome
/// than the allocation this would save (#1095 review).
///
/// Then reparses the same, now-known-valid text through this crate's own
/// fidelity-preserving JSON semi-indexer (the same one the primary input
/// path already uses, see `evaluate_input` above) via the shared
/// `json_bytes_to_owned_value`.
fn parse_json_value(s: &str) -> Result<OwnedValue> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(OwnedValue::Null);
    }

    if let Err(e) = serde_json::from_str::<serde_json::Value>(s) {
        // Real jq's own number parser tolerates a leading zero (`007`,
        // `00`, `007.5`) that strict JSON doesn't (#1094) -- retry once
        // with every number token's leading zero stripped (string
        // contents/keys left untouched) before surfacing the original
        // error. Only paid on this (rare) failure path, so the common
        // case's validation cost is unaffected. `json_bytes_to_owned_value`
        // below still runs on the *original* text, not the normalized one:
        // this crate's own semi-indexer already tolerates a leading zero
        // on its own (confirmed live), so there's nothing left to repair
        // for it.
        let normalized = normalize_leading_zero_numbers(s);
        if normalized == s || serde_json::from_str::<serde_json::Value>(&normalized).is_err() {
            return Err(e).with_context(|| format!("Invalid JSON: {s}"));
        }
    }

    Ok(crate::output::json_bytes_to_owned_value(s.as_bytes()))
}

/// Strip a leading zero from every JSON number token in `s` (`007` ->
/// `7`, `-00` -> `-0`, `007.5` -> `7.5`), leaving string contents/keys and
/// all other structure untouched. Real jq's own number parser tolerates a
/// leading zero (unlike strict RFC 8259 JSON); this repairs just that one
/// divergence before re-validating with `serde_json` (#1094) -- the
/// trailing-garbage/number-range checks that validation exists for still
/// apply to the normalized text exactly as before.
fn normalize_leading_zero_numbers(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(s.len());
    let mut i = 0;
    let mut in_string = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            out.push(b);
            if b == b'\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1]);
                i += 2;
                continue;
            }
            if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if b == b'"' {
            in_string = true;
            out.push(b);
            i += 1;
            continue;
        }
        if b == b'-' || b.is_ascii_digit() {
            let start = i;
            if b == b'-' {
                i += 1;
            }
            let int_start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if int_start == i {
                // A bare `-` with no digit following it at all (`-`,
                // `[-,1]`, `{"a":-}`) isn't a number token in the first
                // place -- only reachable when `b == '-'`, since a digit
                // `b` always advances the scan above by at least one.
                // Leaving it untouched (not fabricating a `0` digit) is
                // required, not just tidier: filling one in turns a
                // genuinely malformed token into a syntactically valid
                // one (`-0`), which would make the retried `serde_json`
                // validation below wrongly pass and this genuinely
                // invalid input silently reach `json_bytes_to_owned_value`
                // as `null` instead of erroring (found by review before
                // merge).
                out.push(b'-');
                continue;
            }
            let stripped = s[int_start..i].trim_start_matches('0');
            out.extend_from_slice(&bytes[start..int_start]);
            out.extend_from_slice(if stripped.is_empty() {
                b"0"
            } else {
                stripped.as_bytes()
            });
            if i < bytes.len() && bytes[i] == b'.' {
                let frac_start = i;
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                out.extend_from_slice(&bytes[frac_start..i]);
            }
            if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                let exp_start = i;
                i += 1;
                if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                }
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                out.extend_from_slice(&bytes[exp_start..i]);
            }
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

/// Parse a JSON stream (multiple JSON values) from a string (`--slurpfile`).
///
/// Still loses number-literal source fidelity the way `parse_json_value`
/// did before #1058 -- `find_json_values` (below) already splits a
/// multi-value buffer into byte spans for the main lazy-input path and
/// `serde_json::Deserializer::byte_offset()` could delimit each value here
/// the same way, but wiring that up is deliberately deferred to #1093 to
/// keep #1058/#1095 scoped to `--argjson`/`--jsonargs`.
fn parse_json_stream(s: &str) -> Result<Vec<OwnedValue>> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(vec![]);
    }

    // Validate the whole stream via `serde_json::Deserializer` as before
    // (for its own error message and its rejection of anything that isn't
    // valid, whitespace-or-self-delineated-separated JSON) -- but discard
    // each parsed `Value` rather than converting it, and materialize the
    // real result from the same byte span instead, via this crate's own
    // fidelity-preserving semi-indexer (#1058's fix for `--argjson`,
    // extended here to `--slurpfile`, #1093). `byte_offset()` gives each
    // value's own end position after a successful `.next()`; its start is
    // the first non-whitespace byte after the previous value's end.
    let mut values = Vec::new();
    let mut deserializer = serde_json::Deserializer::from_str(s).into_iter::<serde_json::Value>();
    let mut prev_end = 0;

    while let Some(result) = deserializer.next() {
        result.context("Invalid JSON in stream")?;
        let end = deserializer.byte_offset();
        let start = s[prev_end..end]
            .find(|c: char| !c.is_whitespace())
            .map_or(prev_end, |offset| prev_end + offset);
        values.push(crate::output::json_bytes_to_owned_value(
            &s.as_bytes()[start..end],
        ));
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
/// segment is already isolated by the RS split, so `serde_json::from_str`
/// only needs to validate it (discarding the parsed `Value`) before
/// `json_bytes_to_owned_value` materializes the real result from the same
/// segment text -- no boundary-tracking needed, unlike `parse_json_stream`
/// above.
fn parse_json_seq(s: &str) -> Vec<OwnedValue> {
    let mut values = Vec::new();

    // Split on RS character (0x1E)
    for segment in s.split('\x1E') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }

        // Try to parse as JSON, silently ignore failures
        if serde_json::from_str::<serde_json::Value>(segment).is_ok() {
            values.push(crate::output::json_bytes_to_owned_value(segment.as_bytes()));
        }
        // Parse failures are silently ignored per RFC 7464
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
    at: &InputLocation,
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
        GenericResult::LazyKeys { fields, sorted } => {
            let mut keys = fields.keys();
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
                sink.report(DiagStyle::Jq, &e, at);
                Ok(vec![])
            }
            Err(jq::Control::Break(label)) => {
                sink.report_break(DiagStyle::Jq, &label, at);
                Ok(vec![])
            }
            Err(jq::Control::Halt(code)) => {
                sink.request_halt(code);
                Ok(vec![])
            }
        },
        GenericResult::None => Ok(vec![]),
        GenericResult::Error(e) => {
            sink.report(DiagStyle::Jq, &e, at);
            Ok(vec![])
        }
        GenericResult::Owned(v) => Ok(vec![v]),
        GenericResult::ManyOwned(vs) => Ok(vs),
        GenericResult::Break(label) => {
            sink.report_break(DiagStyle::Jq, &label, at);
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
            sink.report(DiagStyle::Jq, &e, at);
            Ok(vs)
        }
        GenericResult::Partial(vs, jq::Control::Break(label)) => {
            sink.report_break(DiagStyle::Jq, &label, at);
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
        GenericResult::One(v) => vec![standard_json_to_jq_value(v, &cursor)],
        // OneCursor: directly use the cursor - most memory efficient for unchanged values
        GenericResult::OneCursor(c) => vec![JqValue::Cursor(c)],
        GenericResult::Many(vs) => vs
            .into_iter()
            .map(|v| standard_json_to_jq_value(v, &cursor))
            .collect(),
        // ManyCursor: same lazy-cursor efficiency as OneCursor, per element.
        GenericResult::ManyCursor(cs) => cs.into_iter().map(JqValue::Cursor).collect(),
        // Stays lazy all the way to output: a bare `keys_unsorted` never
        // materializes a `Vec<String>` — `write_json`/`print_json` stream
        // each key's raw bytes straight from `fields`. `JqValue::LazyKeysArray`
        // is document-order-only (no sort concept in its writer), so a
        // sorted `keys` result must never be routed there (#683) — it
        // materializes and sorts here instead, same as eager `Keys` always
        // did.
        GenericResult::LazyKeys {
            fields,
            sorted: false,
        } => vec![JqValue::LazyKeysArray(fields)],
        GenericResult::LazyKeys {
            fields,
            sorted: true,
        } => {
            let mut keys = fields.keys();
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
fn standard_json_to_jq_value<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    _parent_cursor: &JsonCursor<'a, W>,
) -> JqValue<'a, W> {
    match value {
        StandardJson::Null => JqValue::Null,
        StandardJson::Bool(b) => JqValue::Bool(b),
        StandardJson::Number(n) => {
            // Use RawNumber to preserve original formatting like "4e4"
            JqValue::RawNumber(n.raw_bytes())
        }
        StandardJson::String(s) => {
            // Keep string lazy - use raw bytes reference instead of decoding
            JqValue::String(s.as_str().map(|c| c.to_string()).unwrap_or_default())
        }
        StandardJson::Array(elements) => {
            // LAZY: Store cursor references instead of materializing children
            let items: Vec<JqValue<'a, W>> = elements.cursor_iter().map(JqValue::Cursor).collect();
            JqValue::Array(items)
        }
        StandardJson::Object(fields) => {
            // LAZY: Store cursor references instead of materializing values
            let map: IndexMap<String, JqValue<'a, W>> = fields
                .map(|f| {
                    let key = match f.key() {
                        StandardJson::String(s) => s.as_str().unwrap_or_default().to_string(),
                        _ => String::new(),
                    };
                    // Use cursor for value instead of materializing
                    (key, JqValue::Cursor(f.value_cursor()))
                })
                .collect();
            JqValue::Object(map)
        }
        StandardJson::Error(_) => JqValue::Null,
    }
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
            print_json(out, value, &JqCompatFormatter, config, 0)?;
        } else {
            print_json(out, value, &PreserveFormatter, config, 0)?;
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
fn print_json<F, Out, Wrd>(
    out: &mut Out,
    value: &JqValue<'_, Wrd>,
    formatter: &F,
    config: &OutputConfig,
    level: usize,
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
                            print_json(out, &child_value, formatter, config, level + 1)?;
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
                            print_json(out, &child_value, formatter, config, level + 1)?;
                        }
                        out.write_all(separator.as_bytes())?;
                        out.write_all(current_indent.as_bytes())?;
                        out.write_all(b"]")?;
                    }
                }
                StandardJson::Object(fields) => {
                    use succinctly::json::light::StandardJson as SJ;
                    let mut is_first = true;
                    if fields.is_empty() {
                        out.write_all(b"{}")?;
                    } else if compact {
                        out.write_all(b"{")?;
                        for field in fields {
                            if !is_first {
                                out.write_all(b",")?;
                            }
                            is_first = false;
                            if let SJ::String(k) = field.key() {
                                // Zero-copy for keys without escapes
                                let raw = k.raw_bytes();
                                let content = &raw[1..raw.len().saturating_sub(1)];
                                if !config.ascii_output && !content.contains(&b'\\') {
                                    out.write_all(raw)?;
                                    out.write_all(b":")?;
                                } else if let Ok(decoded) = k.as_str() {
                                    out.write_all(b"\"")?;
                                    let escaped = if config.ascii_output {
                                        escape_json_string_ascii(&decoded)
                                    } else {
                                        escape_json_string(&decoded)
                                    };
                                    out.write_all(escaped.as_bytes())?;
                                    out.write_all(b"\":")?;
                                } else {
                                    out.write_all(raw)?;
                                    out.write_all(b":")?;
                                }
                            }
                            let child_value = JqValue::Cursor(field.value_cursor());
                            print_json(out, &child_value, formatter, config, level + 1)?;
                        }
                        out.write_all(b"}")?;
                    } else {
                        out.write_all(b"{")?;
                        out.write_all(separator.as_bytes())?;
                        for field in fields {
                            if !is_first {
                                out.write_all(b",")?;
                                out.write_all(separator.as_bytes())?;
                            }
                            is_first = false;
                            out.write_all(next_indent.as_bytes())?;
                            if let SJ::String(k) = field.key() {
                                // Zero-copy for keys without escapes
                                let raw = k.raw_bytes();
                                let content = &raw[1..raw.len().saturating_sub(1)];
                                if !config.ascii_output && !content.contains(&b'\\') {
                                    out.write_all(raw)?;
                                    out.write_all(b":")?;
                                    out.write_all(space_after_colon.as_bytes())?;
                                } else if let Ok(decoded) = k.as_str() {
                                    out.write_all(b"\"")?;
                                    let escaped = if config.ascii_output {
                                        escape_json_string_ascii(&decoded)
                                    } else {
                                        escape_json_string(&decoded)
                                    };
                                    out.write_all(escaped.as_bytes())?;
                                    out.write_all(b"\":")?;
                                    out.write_all(space_after_colon.as_bytes())?;
                                } else {
                                    out.write_all(raw)?;
                                    out.write_all(b":")?;
                                    out.write_all(space_after_colon.as_bytes())?;
                                }
                            }
                            let child_value = JqValue::Cursor(field.value_cursor());
                            print_json(out, &child_value, formatter, config, level + 1)?;
                        }
                        out.write_all(separator.as_bytes())?;
                        out.write_all(current_indent.as_bytes())?;
                        out.write_all(b"}")?;
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
                    print_json(out, v, formatter, config, level + 1)?;
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
                    print_json(out, v, formatter, config, level + 1)?;
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
                    print_json(out, v, formatter, config, level + 1)?;
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
                    print_json(out, v, formatter, config, level + 1)?;
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
        let ends = find_json_values(bytes);
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
}
