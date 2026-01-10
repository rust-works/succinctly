//! jq-compatible command runner for succinctly.
//!
//! This module implements a jq-compatible CLI interface using the succinctly
//! JSON semi-indexing and jq expression evaluator.

use anyhow::{Context, Result};
use indexmap::IndexMap;
use std::collections::BTreeMap;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use succinctly::jq::{self, Expr, JqValue, OwnedValue, Program, QueryResult};
use succinctly::json::light::{JsonCursor, StandardJson};
use succinctly::json::JsonIndex;

use super::JqCommand;

/// Exit codes matching jq behavior
pub mod exit_codes {
    pub const SUCCESS: i32 = 0;
    pub const FALSE_OR_NULL: i32 = 1; // With -e, last output was false or null
    #[allow(dead_code)]
    pub const USAGE_ERROR: i32 = 2; // Usage problem or system error
    #[allow(dead_code)]
    pub const COMPILE_ERROR: i32 = 3; // jq program compile error
    pub const NO_OUTPUT: i32 = 4; // With -e, no valid result produced
    #[allow(dead_code)]
    pub const HALT_ERROR: i32 = 5; // halt_error without explicit code
}

/// Print build configuration information (similar to jq --build-configuration)
fn print_build_configuration() {
    println!("succinctly jq build configuration:");
    println!();
    println!("Version: {}", env!("CARGO_PKG_VERSION"));
    println!(
        "Target: {}-{}-{}",
        std::env::consts::ARCH,
        std::env::consts::FAMILY,
        std::env::consts::OS
    );
    println!(
        "Profile: {}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    println!();
    println!("Features:");
    println!("  std: {}", cfg!(feature = "std"));
    println!("  simd: {}", cfg!(feature = "simd"));
    println!("  regex: {}", cfg!(feature = "regex"));
    println!();
    println!("Platform:");
    println!("  OS: {}", std::env::consts::OS);
    println!("  Arch: {}", std::env::consts::ARCH);
    println!("  Family: {}", std::env::consts::FAMILY);
    #[cfg(target_arch = "x86_64")]
    {
        println!();
        println!("x86_64 CPU features (runtime detected):");
        println!("  SSE2: true (baseline)");
        println!("  SSE4.2: {}", is_x86_feature_detected!("sse4.2"));
        println!("  AVX2: {}", is_x86_feature_detected!("avx2"));
        println!("  POPCNT: {}", is_x86_feature_detected!("popcnt"));
        println!("  BMI1: {}", is_x86_feature_detected!("bmi1"));
        println!("  BMI2: {}", is_x86_feature_detected!("bmi2"));
    }
    #[cfg(target_arch = "aarch64")]
    {
        println!();
        println!("aarch64 CPU features:");
        println!("  NEON: true (mandatory on aarch64)");
    }
}

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
    /// Loaded modules (path -> function definitions)
    loaded_modules: BTreeMap<String, Vec<(String, Expr)>>,
    /// Auto-loaded ~/.jq file definitions (if file exists)
    auto_loaded_defs: Vec<(String, Expr)>,
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

        ModuleLoader {
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
            format!("{}.jq", module_path)
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

    /// Load a module and return its function definitions.
    pub fn load_module(&mut self, module_path: &str) -> Result<Vec<(String, Expr)>> {
        // Check if already loaded
        if let Some(defs) = self.loaded_modules.get(module_path) {
            return Ok(defs.clone());
        }

        // Resolve the module path
        let file_path = self
            .resolve_module(module_path)
            .ok_or_else(|| anyhow::anyhow!("module '{}' not found in search path", module_path))?;

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
        for (name, body) in self.auto_loaded_defs.clone().into_iter().rev() {
            expr = Expr::FuncDef {
                name,
                params: Vec::new(),
                body: Box::new(body),
                then: Box::new(expr),
            };
        }

        // Process includes (definitions merged into current scope)
        for include in &program.includes {
            let defs = self.load_module(&include.path)?;
            // Wrap expression with function definitions from the included module
            for (name, body) in defs.into_iter().rev() {
                expr = Expr::FuncDef {
                    name,
                    params: Vec::new(), // We'll need to extract params too
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
            for (name, body) in defs.into_iter().rev() {
                let namespaced_name = format!("{}::{}", namespace, name);
                expr = Expr::FuncDef {
                    name: namespaced_name,
                    params: Vec::new(),
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
            let full_name = format!("{}::{}", namespace, name);
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
            pattern,
            body,
        } => Expr::AsPattern {
            expr: Box::new(rewrite_namespaced_calls(*expr)),
            pattern,
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
        // Expressions that don't contain sub-expressions - return as-is
        Expr::Identity
        | Expr::Field(_)
        | Expr::Index(_)
        | Expr::Slice { .. }
        | Expr::Iterate
        | Expr::RecursiveDescent
        | Expr::Literal(_)
        | Expr::Var(_)
        | Expr::Not
        | Expr::Format(_)
        | Expr::Builtin(_) => expr,
    }
}

/// Extract function definitions from an expression.
fn extract_func_defs(expr: &Expr) -> Vec<(String, Expr)> {
    let mut defs = Vec::new();

    fn extract_inner(expr: &Expr, defs: &mut Vec<(String, Expr)>) {
        if let Expr::FuncDef {
            name, body, then, ..
        } = expr
        {
            defs.push((name.clone(), (**body).clone()));
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

        // Determine color output with priority:
        // 1. --monochrome-output (-M) forces off
        // 2. --color-output (-C) forces on
        // 3. NO_COLOR env var disables color (https://no-color.org/)
        // 4. Default: color if stdout is a terminal
        let color_output = if args.monochrome_output {
            false
        } else if args.color_output {
            true
        } else if std::env::var("NO_COLOR").is_ok() {
            // NO_COLOR is set (any value means disable colors)
            false
        } else {
            // Default: color if stdout is a terminal
            atty::is(atty::Stream::Stdout)
        };

        // Get color scheme from JQ_COLORS env var (or defaults)
        let color_scheme = ColorScheme::from_env();

        // Determine jq_compat mode with priority:
        // 1. --preserve-input flag forces off (preserve original formatting)
        // 2. SUCCINCTLY_PRESERVE_INPUT=1 env var disables jq_compat
        // 3. Default: on (jq-compatible formatting)
        let jq_compat = !args.preserve_input
            && !std::env::var("SUCCINCTLY_PRESERVE_INPUT")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);

        OutputConfig {
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
        print_build_configuration();
        return Ok(exit_codes::SUCCESS);
    }

    // Build evaluation context from arguments
    let context = build_context(&args)?;

    // Get the filter expression
    let filter_str = get_filter(&args)?;

    // Parse the filter as a full program (with module directives)
    let program = jq::parse_program(&filter_str).map_err(|e| {
        eprintln!("jq: compile error: {}", e);
        anyhow::anyhow!("compile error")
    })?;

    // Create module loader and process imports/includes
    let mut module_loader = ModuleLoader::new(&args.library_path);
    let expr = module_loader.process_program(&program).map_err(|e| {
        eprintln!("jq: module error: {}", e);
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

    // Check if we can use the lazy path (preserves/reformats number formatting)
    // Conditions: not slurp, not raw_input, and either:
    // - compact output without sort_keys/colors/ascii (preserves formatting), OR
    // - jq_compat mode (needs raw bytes for proper number reformatting)
    let can_use_lazy_path = !args.slurp
        && !args.raw_input
        && !args.seq // seq input mode parses differently
        && !output_config.sort_keys
        && !output_config.color_output
        && !output_config.ascii_output // ASCII output requires escaping
        && (output_config.compact || output_config.jq_compat);

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

        for raw in raw_inputs {
            // Process as JSON stream (handle multiple JSON values in one input)
            let values = find_json_values(&raw);
            for (start, end) in values {
                let json_bytes = &raw[start..end];
                let index = JsonIndex::build(json_bytes);
                let results = evaluate_bytes_lazy(json_bytes, &expr, &index);

                for result in &results {
                    had_output = true;
                    // For exit_status tracking, we need to check the last value
                    if args.exit_status {
                        last_output = Some(result.materialize());
                    }
                    write_output_jq_value(&mut out, result, &output_config)?;
                }
            }
        }
    } else {
        // Original path: parse through serde_json (loses number formatting)
        let inputs = get_inputs(&args)?;

        for input in inputs {
            let results = evaluate_input(&input, &expr, &context)?;

            for result in results {
                had_output = true;
                last_output = Some(result.clone());
                write_output(&mut out, &result, &output_config)?;
            }
        }
    }

    out.flush()?;

    // Determine exit code
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
                .with_context(|| format!("Invalid JSON for --argjson {}", name))?;
            context.named.insert(name.clone(), json_value);
        }
    }

    // Process --slurpfile name file pairs
    for chunk in args.slurpfile.chunks(2) {
        if let [name, file] = chunk {
            let contents = std::fs::read_to_string(file)
                .with_context(|| format!("Failed to read file for --slurpfile {}", name))?;
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
                .with_context(|| format!("Failed to read file for --rawfile {}", name))?;
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
        let json_value = parse_json_value(arg)
            .with_context(|| format!("Invalid JSON for --jsonargs: {}", arg))?;
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
fn get_inputs(args: &JqCommand) -> Result<Vec<OwnedValue>> {
    // Null input mode: use null as the single input
    if args.null_input {
        return Ok(vec![OwnedValue::Null]);
    }

    // Get input files
    let files = get_input_files(args);

    // Collect raw input from files or stdin
    let raw_inputs = if files.is_empty() {
        vec![read_stdin()?]
    } else {
        files
            .iter()
            .map(|path| read_file(path))
            .collect::<Result<Vec<_>>>()?
    };

    // Process based on input mode
    let mut values = Vec::new();

    for raw in raw_inputs {
        if args.raw_input {
            // Raw input: each line becomes a string
            for line in raw.lines() {
                values.push(OwnedValue::String(line.to_string()));
            }
        } else if args.seq {
            // JSON sequence input (RFC 7464): split on RS, ignore parse failures
            let parsed = parse_json_seq(&raw);
            values.extend(parsed);
        } else {
            // JSON input: parse as JSON stream
            let parsed = parse_json_stream(&raw)?;
            values.extend(parsed);
        }
    }

    // Slurp mode: wrap all inputs in an array
    if args.slurp {
        Ok(vec![OwnedValue::Array(values)])
    } else {
        Ok(values)
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
                if let Some(end) = find_string_end(bytes, i) {
                    i = end;
                    continue;
                } else {
                    return None;
                }
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

/// Parse a JSON value from a string.
fn parse_json_value(s: &str) -> Result<OwnedValue> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(OwnedValue::Null);
    }

    // Use serde_json for parsing, then convert to OwnedValue
    let value: serde_json::Value =
        serde_json::from_str(s).with_context(|| format!("Invalid JSON: {}", s))?;

    Ok(serde_to_owned(&value))
}

/// Parse a JSON stream (multiple JSON values) from a string.
fn parse_json_stream(s: &str) -> Result<Vec<OwnedValue>> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(vec![]);
    }

    // Try to parse as a stream of JSON values
    let mut values = Vec::new();
    let deserializer = serde_json::Deserializer::from_str(s).into_iter::<serde_json::Value>();

    for result in deserializer {
        let value = result.context("Invalid JSON in stream")?;
        values.push(serde_to_owned(&value));
    }

    Ok(values)
}

/// Parse JSON sequence format (RFC 7464).
/// Input is split on RS (0x1E) characters, each segment parsed as JSON.
/// Parse failures are silently ignored (per RFC 7464 recommendation).
fn parse_json_seq(s: &str) -> Vec<OwnedValue> {
    let mut values = Vec::new();

    // Split on RS character (0x1E)
    for segment in s.split('\x1E') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }

        // Try to parse as JSON, silently ignore failures
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(segment) {
            values.push(serde_to_owned(&value));
        }
        // Parse failures are silently ignored per RFC 7464
    }

    values
}

/// Convert serde_json::Value to OwnedValue.
fn serde_to_owned(value: &serde_json::Value) -> OwnedValue {
    match value {
        serde_json::Value::Null => OwnedValue::Null,
        serde_json::Value::Bool(b) => OwnedValue::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                OwnedValue::Int(i)
            } else if let Some(f) = n.as_f64() {
                OwnedValue::Float(f)
            } else {
                OwnedValue::Null
            }
        }
        serde_json::Value::String(s) => OwnedValue::String(s.clone()),
        serde_json::Value::Array(arr) => {
            OwnedValue::Array(arr.iter().map(serde_to_owned).collect())
        }
        serde_json::Value::Object(obj) => OwnedValue::Object(
            obj.iter()
                .map(|(k, v)| (k.clone(), serde_to_owned(v)))
                .collect(),
        ),
    }
}

/// Evaluate the expression against an input value.
fn evaluate_input(
    input: &OwnedValue,
    expr: &jq::Expr,
    _context: &EvalContext,
) -> Result<Vec<OwnedValue>> {
    // Convert OwnedValue to JSON bytes for indexing
    let json_str = input.to_json();
    let json_bytes = json_str.as_bytes();

    // Build index and evaluate
    let index = JsonIndex::build(json_bytes);
    let cursor = index.root(json_bytes);

    // TODO: Pass context to evaluator for variable resolution
    // For now, we use the basic eval which doesn't support external variables
    let result = jq::eval(expr, cursor);

    // Convert result to Vec<OwnedValue>
    match result {
        QueryResult::One(v) => Ok(vec![standard_json_to_owned(&v)]),
        QueryResult::Many(vs) => Ok(vs.iter().map(standard_json_to_owned).collect()),
        QueryResult::None => Ok(vec![]),
        QueryResult::Error(e) => {
            eprintln!("jq: error: {}", e);
            Ok(vec![])
        }
        QueryResult::Owned(v) => Ok(vec![v]),
        QueryResult::ManyOwned(vs) => Ok(vs),
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
) -> Vec<JqValue<'a, Vec<u64>>> {
    let cursor = index.root(json_bytes);
    let result = jq::eval(expr, cursor);
    query_result_to_jq_values(result, cursor)
}

/// Convert StandardJson to OwnedValue.
fn standard_json_to_owned<W: Clone + AsRef<[u64]>>(value: &StandardJson<'_, W>) -> OwnedValue {
    match value {
        StandardJson::Null => OwnedValue::Null,
        StandardJson::Bool(b) => OwnedValue::Bool(*b),
        StandardJson::Number(n) => {
            if let Ok(i) = n.as_i64() {
                OwnedValue::Int(i)
            } else if let Ok(f) = n.as_f64() {
                OwnedValue::Float(f)
            } else {
                OwnedValue::Null
            }
        }
        StandardJson::String(s) => {
            OwnedValue::String(s.as_str().map(|c| c.to_string()).unwrap_or_default())
        }
        StandardJson::Array(elements) => {
            OwnedValue::Array((*elements).map(|e| standard_json_to_owned(&e)).collect())
        }
        StandardJson::Object(fields) => OwnedValue::Object(
            (*fields)
                .map(|f| {
                    let key = match f.key() {
                        StandardJson::String(s) => s.as_str().unwrap_or_default().to_string(),
                        _ => String::new(),
                    };
                    (key, standard_json_to_owned(&f.value()))
                })
                .collect(),
        ),
        StandardJson::Error(_) => OwnedValue::Null,
    }
}

/// Convert QueryResult to Vec<JqValue> for lazy output.
///
/// This preserves cursor references where possible, allowing lazy output
/// that preserves original number formatting (e.g., `4e4` stays as `4e4`).
fn query_result_to_jq_values<'a, W: Clone + AsRef<[u64]>>(
    result: QueryResult<'a, W>,
    cursor: JsonCursor<'a, W>,
) -> Vec<JqValue<'a, W>> {
    match result {
        QueryResult::One(v) => vec![standard_json_to_jq_value(v, &cursor)],
        QueryResult::Many(vs) => vs
            .into_iter()
            .map(|v| standard_json_to_jq_value(v, &cursor))
            .collect(),
        QueryResult::None => vec![],
        QueryResult::Error(e) => {
            eprintln!("jq: error: {}", e);
            vec![]
        }
        QueryResult::Owned(v) => vec![JqValue::from_owned(v)],
        QueryResult::ManyOwned(vs) => vs.into_iter().map(JqValue::from_owned).collect(),
    }
}

/// Convert StandardJson to JqValue, preserving raw number formatting.
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
            JqValue::String(s.as_str().map(|c| c.to_string()).unwrap_or_default())
        }
        StandardJson::Array(elements) => {
            let items: Vec<JqValue<'a, W>> = elements
                .map(|e| standard_json_to_jq_value(e, _parent_cursor))
                .collect();
            JqValue::Array(items)
        }
        StandardJson::Object(fields) => {
            let map: IndexMap<String, JqValue<'a, W>> = fields
                .map(|f| {
                    let key = match f.key() {
                        StandardJson::String(s) => s.as_str().unwrap_or_default().to_string(),
                        _ => String::new(),
                    };
                    (key, standard_json_to_jq_value(f.value(), _parent_cursor))
                })
                .collect();
            JqValue::Object(map)
        }
        StandardJson::Error(_) => JqValue::Null,
    }
}

/// Write a single output JqValue (preserves number formatting when possible).
fn write_output_jq_value<'a, Out: Write, Wrd: Clone + AsRef<[u64]>>(
    out: &mut Out,
    value: &JqValue<'a, Wrd>,
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
    if config.jq_compat && !config.sort_keys && !config.color_output {
        write_jq_value_jq_compat(out, value, config, 0)?;
    } else if config.compact && !config.sort_keys && !config.color_output {
        // For simple compact output, use JqValue's direct output
        // This preserves number formatting like "4e4"
        let mut output = String::new();
        value
            .write_json(&mut output)
            .map_err(|_| anyhow::anyhow!("Failed to format JSON"))?;
        out.write_all(output.as_bytes())?;
    } else {
        // For complex output (pretty-print, sort_keys, colors), materialize first
        let owned = value.materialize();
        let output = if config.sort_keys {
            format_json_sorted(&owned, config)
        } else {
            format_json(&owned, config)
        };
        out.write_all(output.as_bytes())?;
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

    let output = if config.sort_keys {
        format_json_sorted(value, config)
    } else {
        format_json(value, config)
    };

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

/// Format a raw JSON number string according to jq's formatting rules.
///
/// jq's number formatting:
/// - Integers: output as-is
/// - Floats with trailing zeros: preserve them (0.10 → 0.10)
/// - Scientific notation: normalize mantissa (12e2 → 1.2E+3), uppercase E, explicit +
/// - e0 or e-0: eliminate exponent entirely (5.5e0 → 5.5)
/// - Negative exponents >= -5: convert to decimal (1e-3 → 0.001)
/// - Negative exponents < -5: keep scientific (1e-10 → 1E-10)
/// - Negative zero: preserve as -0
fn format_number_jq_compat(raw: &[u8]) -> String {
    let s = match core::str::from_utf8(raw) {
        Ok(s) => s,
        Err(_) => return String::from_utf8_lossy(raw).into_owned(),
    };

    // Check if it contains exponent notation
    let has_exp = s.contains('e') || s.contains('E');
    let has_dot = s.contains('.');

    if !has_exp && !has_dot {
        // Plain integer - output as-is
        return s.to_string();
    }

    if !has_exp {
        // Plain decimal without exponent - preserve as-is (keeps trailing zeros)
        return s.to_string();
    }

    // Has exponent - need to reformat according to jq rules
    // Parse the full number to get the actual value
    let value: f64 = match s.parse() {
        Ok(v) => v,
        Err(_) => return s.to_string(),
    };

    // Parse exponent to check for e0/e-0
    let exp: i32 = if let Some(pos) = s.find(['e', 'E']) {
        s[pos + 1..].parse().unwrap_or(0)
    } else {
        0
    };

    // For e0 or e-0, jq eliminates the exponent
    if exp == 0 {
        // Check if result is integer
        if value.fract() == 0.0 && value.abs() < 1e15 {
            return format!("{}", value as i64);
        } else {
            // Format as plain decimal
            let formatted = format!("{}", value);
            return formatted;
        }
    }

    // For negative exponents >= -5, jq converts to decimal
    if (-5..0).contains(&exp) {
        // Convert to decimal: 1e-3 → 0.001
        // Use smart rounding to avoid floating point noise
        return format_decimal_jq(value);
    }

    // For other cases, use normalized scientific notation
    // jq normalizes mantissa to have one digit before decimal point
    if value == 0.0 {
        return "0".to_string();
    }

    let abs_value = value.abs();
    let log10 = abs_value.log10().floor() as i32;
    let normalized_mantissa = abs_value / 10f64.powi(log10);
    let new_exp = log10;

    // Format mantissa with appropriate precision
    // Round to avoid floating point noise (e.g., 9.199999999999999 → 9.2)
    let mantissa_str = format_mantissa_jq(normalized_mantissa);

    let sign = if value < 0.0 { "-" } else { "" };
    let exp_sign = if new_exp >= 0 { "+" } else { "" };
    format!("{}{}E{}{}", sign, mantissa_str, exp_sign, new_exp)
}

/// Format a mantissa value for jq-compatible output.
/// Handles floating point precision issues by rounding appropriately.
fn format_mantissa_jq(value: f64) -> String {
    // Check if it's essentially an integer
    if (value.round() - value).abs() < 1e-10 {
        return format!("{}", value.round() as i64);
    }

    // Try different precisions and pick the shortest that rounds back correctly
    for precision in 1..=15 {
        let formatted = format!("{:.prec$}", value, prec = precision);
        if let Ok(parsed) = formatted.parse::<f64>() {
            if (parsed - value).abs() < 1e-14 {
                // Trim trailing zeros
                let trimmed = formatted.trim_end_matches('0');
                if trimmed.ends_with('.') {
                    return format!("{}0", trimmed);
                } else {
                    return trimmed.to_string();
                }
            }
        }
    }

    // Fallback: full precision
    let formatted = format!("{:.15}", value);
    let trimmed = formatted.trim_end_matches('0');
    if trimmed.ends_with('.') {
        format!("{}0", trimmed)
    } else {
        trimmed.to_string()
    }
}

/// Format a decimal value for jq-compatible output.
/// Uses smart rounding to avoid floating point noise.
fn format_decimal_jq(value: f64) -> String {
    let sign = if value < 0.0 { "-" } else { "" };
    let abs_value = value.abs();

    // Check if it's essentially an integer
    if (abs_value.round() - abs_value).abs() < 1e-10 {
        return format!("{}", value.round() as i64);
    }

    // Try different precisions and pick the shortest that rounds back correctly
    for precision in 1..=15 {
        let formatted = format!("{:.prec$}", abs_value, prec = precision);
        if let Ok(parsed) = formatted.parse::<f64>() {
            if (parsed - abs_value).abs() < 1e-14 {
                // Trim trailing zeros
                let trimmed = formatted.trim_end_matches('0');
                if trimmed.ends_with('.') {
                    return format!("{}{}0", sign, trimmed);
                } else {
                    return format!("{}{}", sign, trimmed);
                }
            }
        }
    }

    // Fallback: full precision
    let formatted = format!("{:.15}", abs_value);
    let trimmed = formatted.trim_end_matches('0');
    if trimmed.ends_with('.') {
        format!("{}{}0", sign, trimmed)
    } else {
        format!("{}{}", sign, trimmed)
    }
}

/// Write a JqValue as JSON with jq-compatible formatting.
///
/// This formats the value according to jq's output rules, including
/// proper number formatting (uppercase E, explicit +, etc.).
fn write_jq_value_jq_compat<'a, Out: Write, Wrd: Clone + AsRef<[u64]>>(
    out: &mut Out,
    value: &JqValue<'a, Wrd>,
    config: &OutputConfig,
    level: usize,
) -> Result<()> {
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
        JqValue::Int(n) => write!(out, "{}", n)?,
        JqValue::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                out.write_all(b"null")?;
            } else {
                // For computed floats, use jq's formatting
                write!(out, "{}", f)?;
            }
        }
        JqValue::RawNumber(bytes) => {
            // Reformat according to jq rules
            let formatted = format_number_jq_compat(bytes);
            out.write_all(formatted.as_bytes())?;
        }
        JqValue::Cursor(c) => {
            // For cursor values, get raw bytes and check what type it is
            if let Some(raw_bytes) = c.raw_bytes() {
                let first = raw_bytes.first().copied().unwrap_or(0);
                if first == b'-' || first.is_ascii_digit() {
                    // Number: reformat according to jq rules
                    let formatted = format_number_jq_compat(raw_bytes);
                    out.write_all(formatted.as_bytes())?;
                } else if first == b'"' {
                    // String: need to decode and re-encode with jq's escape preferences
                    // (e.g., \u0008 → \b, \u000c → \f)
                    let owned = value.materialize();
                    if let OwnedValue::String(s) = owned {
                        out.write_all(b"\"")?;
                        let escaped = if config.ascii_output {
                            escape_json_string_ascii(&s)
                        } else {
                            escape_json_string(&s)
                        };
                        out.write_all(escaped.as_bytes())?;
                        out.write_all(b"\"")?;
                    } else {
                        // Unexpected, just write raw
                        out.write_all(raw_bytes)?;
                    }
                } else {
                    // Other types (null, bool, array, object): write as-is
                    out.write_all(raw_bytes)?;
                }
            } else {
                // Fallback: materialize and format
                let owned = value.materialize();
                let json = format_json_impl(&owned, indent, level, config.ascii_output);
                out.write_all(json.as_bytes())?;
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
                    write_jq_value_jq_compat(out, v, config, level + 1)?;
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
                    write_jq_value_jq_compat(out, v, config, level + 1)?;
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
                    write_jq_value_jq_compat(out, v, config, level + 1)?;
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
                    write_jq_value_jq_compat(out, v, config, level + 1)?;
                }
                out.write_all(separator.as_bytes())?;
                out.write_all(current_indent.as_bytes())?;
                out.write_all(b"}")?;
            }
        }
    }
    Ok(())
}

/// Format a value as JSON.
fn format_json(value: &OwnedValue, config: &OutputConfig) -> String {
    let json = if config.compact {
        format_json_impl(value, "", 0, config.ascii_output)
    } else {
        format_json_impl(value, &config.indent_string, 0, config.ascii_output)
    };

    if config.color_output {
        colorize_json(&json, &config.color_scheme)
    } else {
        json
    }
}

/// Format a value as JSON with sorted keys.
fn format_json_sorted(value: &OwnedValue, config: &OutputConfig) -> String {
    let sorted = sort_keys(value);
    format_json(&sorted, config)
}

/// Sort object keys recursively.
fn sort_keys(value: &OwnedValue) -> OwnedValue {
    match value {
        OwnedValue::Object(obj) => {
            // Collect into BTreeMap to sort, then convert to IndexMap
            let sorted_btree: BTreeMap<String, OwnedValue> =
                obj.iter().map(|(k, v)| (k.clone(), sort_keys(v))).collect();
            let sorted: IndexMap<String, OwnedValue> = sorted_btree.into_iter().collect();
            OwnedValue::Object(sorted)
        }
        OwnedValue::Array(arr) => OwnedValue::Array(arr.iter().map(sort_keys).collect()),
        _ => value.clone(),
    }
}

/// Format JSON implementation with optional pretty printing and ASCII mode.
fn format_json_impl(value: &OwnedValue, indent: &str, level: usize, ascii: bool) -> String {
    let compact = indent.is_empty();
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
        OwnedValue::Null => "null".to_string(),
        OwnedValue::Bool(b) => b.to_string(),
        OwnedValue::Int(i) => i.to_string(),
        OwnedValue::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                "null".to_string() // JSON doesn't support NaN or Infinity
            } else {
                f.to_string()
            }
        }
        OwnedValue::String(s) => {
            if ascii {
                format!("\"{}\"", escape_json_string_ascii(s))
            } else {
                format!("\"{}\"", escape_json_string(s))
            }
        }
        OwnedValue::Array(arr) => {
            if arr.is_empty() {
                "[]".to_string()
            } else if compact {
                let items: Vec<String> = arr
                    .iter()
                    .map(|v| format_json_impl(v, indent, level + 1, ascii))
                    .collect();
                format!("[{}]", items.join(","))
            } else {
                let items: Vec<String> = arr
                    .iter()
                    .map(|v| {
                        format!(
                            "{}{}",
                            next_indent,
                            format_json_impl(v, indent, level + 1, ascii)
                        )
                    })
                    .collect();
                format!(
                    "[{}{}{separator}{}]",
                    separator,
                    items.join(&format!(",{}", separator)),
                    current_indent
                )
            }
        }
        OwnedValue::Object(obj) => {
            if obj.is_empty() {
                "{}".to_string()
            } else if compact {
                let items: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        let key = if ascii {
                            escape_json_string_ascii(k)
                        } else {
                            escape_json_string(k)
                        };
                        format!(
                            "\"{}\":{}",
                            key,
                            format_json_impl(v, indent, level + 1, ascii)
                        )
                    })
                    .collect();
                format!("{{{}}}", items.join(","))
            } else {
                let items: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        let key = if ascii {
                            escape_json_string_ascii(k)
                        } else {
                            escape_json_string(k)
                        };
                        format!(
                            "\"{}\":{}{}",
                            key,
                            space_after_colon,
                            format_json_impl(v, indent, level + 1, ascii)
                        )
                    })
                    .collect();
                // Add indent before each key
                let indented_items: Vec<String> = items
                    .iter()
                    .map(|item| format!("{}{}", next_indent, item))
                    .collect();
                format!(
                    "{{{}{}{separator}{}}}",
                    separator,
                    indented_items.join(&format!(",{}", separator)),
                    current_indent
                )
            }
        }
    }
}

/// Escape special characters in a JSON string.
fn escape_json_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\x08' => result.push_str("\\b"), // backspace
            '\x0C' => result.push_str("\\f"), // form feed
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_control() => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => result.push(c),
        }
    }
    result
}

/// Escape special characters in a JSON string, also escaping non-ASCII as \uXXXX.
fn escape_json_string_ascii(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\x08' => result.push_str("\\b"), // backspace
            '\x0C' => result.push_str("\\f"), // form feed
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_control() => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            c if !c.is_ascii() => {
                // Escape non-ASCII characters as \uXXXX
                // For characters outside BMP, use surrogate pairs
                let code = c as u32;
                if code <= 0xFFFF {
                    result.push_str(&format!("\\u{:04x}", code));
                } else {
                    // Surrogate pair for characters above U+FFFF
                    let adjusted = code - 0x10000;
                    let high = 0xD800 + (adjusted >> 10);
                    let low = 0xDC00 + (adjusted & 0x3FF);
                    result.push_str(&format!("\\u{:04x}\\u{:04x}", high, low));
                }
            }
            c => result.push(c),
        }
    }
    result
}

/// Default ANSI color codes for JSON syntax highlighting.
/// These match jq's default colors.
mod default_colors {
    pub const RESET: &str = "\x1b[0m";
    pub const NULL: &str = "\x1b[1;30m"; // Bold black (gray) - jq default
    pub const FALSE: &str = "\x1b[0;39m"; // Default - jq default
    pub const TRUE: &str = "\x1b[0;39m"; // Default - jq default
    pub const NUMBER: &str = "\x1b[0;39m"; // Default - jq default
    pub const STRING: &str = "\x1b[0;32m"; // Green - jq default
    pub const ARRAY: &str = "\x1b[1;39m"; // Bold default - jq default
    pub const OBJECT: &str = "\x1b[1;39m"; // Bold default - jq default
    pub const KEY: &str = "\x1b[1;34m"; // Bold blue - jq default (or 1;39)
}

/// Color scheme for JSON syntax highlighting.
/// Can be customized via JQ_COLORS environment variable.
#[derive(Clone)]
struct ColorScheme {
    reset: String,
    null: String,
    false_: String,
    true_: String,
    number: String,
    string: String,
    array: String,
    object: String,
    key: String,
}

impl Default for ColorScheme {
    fn default() -> Self {
        ColorScheme {
            reset: default_colors::RESET.to_string(),
            null: default_colors::NULL.to_string(),
            false_: default_colors::FALSE.to_string(),
            true_: default_colors::TRUE.to_string(),
            number: default_colors::NUMBER.to_string(),
            string: default_colors::STRING.to_string(),
            array: default_colors::ARRAY.to_string(),
            object: default_colors::OBJECT.to_string(),
            key: default_colors::KEY.to_string(),
        }
    }
}

impl ColorScheme {
    /// Parse JQ_COLORS environment variable.
    /// Format: "null:false:true:numbers:strings:arrays:objects:objectkeys"
    /// Each value is an SGR parameter like "1;30" for bold black.
    fn from_env() -> Self {
        let mut scheme = ColorScheme::default();

        if let Ok(colors) = std::env::var("JQ_COLORS") {
            let parts: Vec<&str> = colors.split(':').collect();

            // Parse each color in order
            if let Some(sgr) = parts.first() {
                if !sgr.is_empty() {
                    scheme.null = format!("\x1b[{}m", sgr);
                }
            }
            if let Some(sgr) = parts.get(1) {
                if !sgr.is_empty() {
                    scheme.false_ = format!("\x1b[{}m", sgr);
                }
            }
            if let Some(sgr) = parts.get(2) {
                if !sgr.is_empty() {
                    scheme.true_ = format!("\x1b[{}m", sgr);
                }
            }
            if let Some(sgr) = parts.get(3) {
                if !sgr.is_empty() {
                    scheme.number = format!("\x1b[{}m", sgr);
                }
            }
            if let Some(sgr) = parts.get(4) {
                if !sgr.is_empty() {
                    scheme.string = format!("\x1b[{}m", sgr);
                }
            }
            if let Some(sgr) = parts.get(5) {
                if !sgr.is_empty() {
                    scheme.array = format!("\x1b[{}m", sgr);
                }
            }
            if let Some(sgr) = parts.get(6) {
                if !sgr.is_empty() {
                    scheme.object = format!("\x1b[{}m", sgr);
                }
            }
            if let Some(sgr) = parts.get(7) {
                if !sgr.is_empty() {
                    scheme.key = format!("\x1b[{}m", sgr);
                }
            }
        }

        scheme
    }
}

/// Colorize a JSON string using ANSI escape codes.
/// This is a simple parser that adds colors to JSON tokens.
fn colorize_json(json: &str, scheme: &ColorScheme) -> String {
    let mut result = String::with_capacity(json.len() * 2);
    let mut chars = json.chars().peekable();
    let mut in_string = false;
    let mut escape_next = false;
    let mut depth_stack: Vec<char> = Vec::new(); // Track context: '{' for object, '[' for array
    let mut expecting_key = false; // True when next string in object is a key

    while let Some(c) = chars.next() {
        if escape_next {
            result.push(c);
            escape_next = false;
            continue;
        }

        if in_string {
            if c == '\\' {
                result.push(c);
                escape_next = true;
            } else if c == '"' {
                result.push(c);
                result.push_str(&scheme.reset);
                in_string = false;
            } else {
                result.push(c);
            }
        } else {
            match c {
                '"' => {
                    // Use expecting_key to determine if this is a key
                    if expecting_key {
                        result.push_str(&scheme.key);
                        expecting_key = false; // After seeing key, next string is value
                    } else {
                        result.push_str(&scheme.string);
                    }
                    result.push(c);
                    in_string = true;
                }
                '{' => {
                    result.push_str(&scheme.object);
                    result.push(c);
                    result.push_str(&scheme.reset);
                    depth_stack.push('{');
                    expecting_key = true; // First thing in object is a key
                }
                '[' => {
                    result.push_str(&scheme.array);
                    result.push(c);
                    result.push_str(&scheme.reset);
                    depth_stack.push('[');
                    // Arrays don't have keys
                }
                '}' => {
                    result.push_str(&scheme.object);
                    result.push(c);
                    result.push_str(&scheme.reset);
                    depth_stack.pop();
                    expecting_key = false;
                }
                ']' => {
                    result.push_str(&scheme.array);
                    result.push(c);
                    result.push_str(&scheme.reset);
                    depth_stack.pop();
                    expecting_key = false;
                }
                ':' => {
                    result.push(c);
                    // After colon, we're expecting a value, not a key
                    expecting_key = false;
                }
                ',' => {
                    result.push(c);
                    // After comma in object context, next string is a key
                    if depth_stack.last() == Some(&'{') {
                        expecting_key = true;
                    }
                }
                't' => {
                    // true
                    result.push_str(&scheme.true_);
                    result.push(c);
                    // Consume rest of the keyword
                    while let Some(&next) = chars.peek() {
                        if next.is_alphabetic() {
                            result.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    result.push_str(&scheme.reset);
                }
                'f' => {
                    // false
                    result.push_str(&scheme.false_);
                    result.push(c);
                    // Consume rest of the keyword
                    while let Some(&next) = chars.peek() {
                        if next.is_alphabetic() {
                            result.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    result.push_str(&scheme.reset);
                }
                'n' => {
                    // null
                    result.push_str(&scheme.null);
                    result.push(c);
                    while let Some(&next) = chars.peek() {
                        if next.is_alphabetic() {
                            result.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    result.push_str(&scheme.reset);
                }
                '0'..='9' | '-' | '.' | 'e' | 'E' | '+' => {
                    result.push_str(&scheme.number);
                    result.push(c);
                    // Consume rest of number
                    while let Some(&next) = chars.peek() {
                        if next.is_ascii_digit()
                            || next == '.'
                            || next == 'e'
                            || next == 'E'
                            || next == '+'
                            || next == '-'
                        {
                            result.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    result.push_str(&scheme.reset);
                }
                _ => {
                    // Whitespace and other characters
                    result.push(c);
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(matches!(
            parse_json_value("42").unwrap(),
            OwnedValue::Int(42)
        ));
        assert!(matches!(
            parse_json_value("\"hello\"").unwrap(),
            OwnedValue::String(_)
        ));
    }

    #[test]
    fn test_parse_json_stream() {
        let stream = r#"{"a":1} {"b":2} {"c":3}"#;
        let values = parse_json_stream(stream).unwrap();
        assert_eq!(values.len(), 3);
    }

    #[test]
    fn test_escape_json_string() {
        assert_eq!(escape_json_string("hello"), "hello");
        assert_eq!(escape_json_string("hello\nworld"), "hello\\nworld");
        assert_eq!(escape_json_string("say \"hi\""), "say \\\"hi\\\"");
    }

    #[test]
    fn test_sort_keys() {
        let mut obj = IndexMap::new();
        obj.insert("z".to_string(), OwnedValue::Int(1));
        obj.insert("a".to_string(), OwnedValue::Int(2));
        let value = OwnedValue::Object(obj);

        let sorted = sort_keys(&value);
        if let OwnedValue::Object(obj) = sorted {
            let keys: Vec<_> = obj.keys().collect();
            assert_eq!(keys, vec!["a", "z"]);
        }
    }
}
