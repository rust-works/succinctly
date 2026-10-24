//! `succinctly dev jq-path-mode` (hidden, #5223): run a filter
//! through the register-carrying path-mode interpreter
//! (`succinctly::jq::path_mode`) instead of the shipped evaluator.
//!
//! Only the surface the differential sweeps use: the JSON values on stdin (or
//! a file), `-c`, `-r` and `-n`. Output is always compact. An uncaught
//! error exits 5, as `jq` does; a construct the interpreter does not implement
//! exits [`UNSUPPORTED_EXIT`] with a `path-mode: unsupported` diagnostic, so
//! `scripts/jq-path-mode-3way.py` can count those rows apart from verdicts.
//! `scripts/jq-path-mode.sh` maps a sweep's `BIN jq -c PROG` argv onto it.

use std::io::{Read, Write};
use std::path::PathBuf;

use clap::Parser;
use succinctly::jq::path_mode::{self, Outcome};
use succinctly::jq::{Expr, OwnedValue};

/// Exit status for a row the interpreter cannot run.
const UNSUPPORTED_EXIT: i32 = 64;

/// Arguments for `dev jq-path-mode`.
#[derive(Debug, Parser)]
pub struct JqPathModeArgs {
    /// Compact output (always on; accepted for argv compatibility with `jq`)
    #[arg(short = 'c', long = "compact-output")]
    compact: bool,
    /// Print strings without quotes
    #[arg(short = 'r', long = "raw-output")]
    raw: bool,
    /// Use `null` as the single input
    #[arg(short = 'n', long = "null-input")]
    null_input: bool,
    /// The jq filter
    filter: String,
    /// Input file (default: stdin)
    file: Option<PathBuf>,
}

/// Run the interpreter and return the process exit status.
pub fn run(args: JqPathModeArgs) -> i32 {
    let _ = args.compact;
    let program = match path_mode::compile(&args.filter) {
        Ok(program) => program,
        Err(e) => {
            eprintln!("jq: error: {e}\njq: 1 compile error");
            return 3;
        }
    };
    // `-n` reads stdin only for the input builtins, as jq does.
    let bytes = if args.null_input && !succinctly::jq::walk::uses_input_builtins(&program) {
        None
    } else {
        let mut bytes = Vec::new();
        let read = match &args.file {
            Some(path) => std::fs::File::open(path).and_then(|mut f| f.read_to_end(&mut bytes)),
            None => std::io::stdin().read_to_end(&mut bytes),
        };
        if let Err(e) = read {
            eprintln!("jq: error: could not read input: {e}");
            return 2;
        }
        Some(bytes)
    };
    // The interpreter runs on a thread like the CLI's own evaluation thread,
    // with the same stack and the same registered native-stack floor
    // (ADR-0025), so a recursion that would overflow is refused instead
    // (values are `Rc`, so the input is parsed there too).
    let (filter, raw, null_input) = (args.filter, args.raw, args.null_input);
    let worker = std::thread::Builder::new()
        .stack_size(crate::EVAL_STACK_SIZE)
        .spawn(move || {
            succinctly::jq::with_stack_budget(crate::EVAL_STACK_SIZE, || {
                // Compiled again on this thread: the program holds `Rc`s.
                match path_mode::compile(&filter) {
                    Ok(program) => run_inputs(&program, bytes.as_deref(), raw, null_input),
                    Err(_) => 3, // patchcov: coverage tolerate-line reason="unreachable: the same filter compiled above"
                }
            })
        })
        .expect("spawn the interpreter's thread");
    worker.join().unwrap_or_else(|_| {
        // A panic is an interpreter bug: count the row as unsupported, never as a
        // verdict on the register model.
        eprintln!("path-mode: unsupported: the interpreter panicked");
        UNSUPPORTED_EXIT
    })
}

/// Run the filter on every input as `jq` does: each top-level value in turn,
/// drawn from the same queue `input`/`inputs` read (so `[., input]` pairs
/// them), the exit status the last input's, and a malformed value ending the
/// read (exit 5, as jq's parse error does) after the values before it ran.
/// With `-n` the filter runs once on `null`, and the queue holds every value.
fn run_inputs(program: &Expr, bytes: Option<&[u8]>, raw: bool, null_input: bool) -> i32 {
    let (spans, malformed) = match bytes {
        Some(bytes) => crate::jq_runner::split_json_values_strict(bytes),
        None => (Vec::new(), None),
    };
    let mut documents = Vec::new();
    for (start, end) in spans {
        let bytes = bytes.unwrap_or_default();
        match crate::jq_runner::json_bytes_to_owned_value_checked(&bytes[start..end]) {
            Ok(value) => documents.push((value, 0, 0)),
            Err(e) => {
                eprintln!("jq: error (at <stdin>:0): {}", e.message);
                return 5;
            }
        }
    }
    succinctly::jq::seed_remaining_inputs(documents, None);
    if null_input {
        return report(program, OwnedValue::Null, raw);
    }
    let mut status = 0;
    while let Some(input) = succinctly::jq::pop_remaining_input() {
        status = report(program, input, raw);
        if status != 0 && status != 5 {
            return status;
        }
    }
    if malformed.is_some() {
        eprintln!("jq: error (at <stdin>:0): Cannot parse the input as JSON");
        return 5;
    }
    status
}

fn report(program: &Expr, input: OwnedValue, raw: bool) -> i32 {
    let run = path_mode::run_compiled(program, input);
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for value in &run.outputs {
        let text = match value {
            OwnedValue::String(s) if raw => s.as_str().to_string(),
            other => other.to_json_jq_preserve(),
        };
        let _ = writeln!(out, "{text}");
    }
    let _ = out.flush();
    match run.outcome {
        Outcome::Done => 0,
        Outcome::Error(error) => {
            match &error.payload() {
                OwnedValue::String(s) => eprintln!("jq: error (at <unknown>): {}", s.as_str()),
                other => eprintln!(
                    "jq: error (at <unknown>) (not a string): {}",
                    other.to_json_jq_preserve()
                ),
            }
            5
        }
        // `halt_error` has already written its message.
        Outcome::Halt(code) => code,
        Outcome::Unsupported(what) => {
            eprintln!("path-mode: unsupported: {what}");
            UNSUPPORTED_EXIT
        }
    }
}
