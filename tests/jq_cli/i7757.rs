//! #7757: `-e` changes the exit status and nothing else, in jq and here.
//!
//! The lazy route's `-e` used to materialize every result before writing it, to remember its
//! truthiness. When that failed it reported the fault and told the evaluator to go on with the
//! document, so `printf '[2,]\n' | succinctly jq -c -e '.,1'` printed the parse error and then `1`
//! (jq: the error only), `.,.` reported the fault twice, and a string the run without `-e`
//! forwards (#5308) raised. Every route now records an output's truthiness through one function
//! after the write (`observe_exit_status` in `jq_runner.rs`), so the two runs cannot part.
//!
//! The rows were captured from jq 1.7.1 (`/usr/bin/jq` on macOS and on Linux) and jq 1.8.2, which
//! agree on every one: with and without `-e` they print the same stdout and stderr, and `-e` maps
//! an exit of 0 to 4 when nothing was printed, 1 when the last output was `null` or `false`, and
//! leaves a fault's 5 (and `halt`'s 0) alone, whatever came before the fault.

use anyhow::Result;

use crate::{small_window, spawn_with_signal_retry, succinctly_bin};

/// The documents, each a fault jq's parser stops on (`nul` and `invalid_utf8` are raw bytes).
const FAULTS: &[(&str, &[u8])] = &[
    ("trailing_comma", b"[2,]"),
    ("stray_closer", b"[2}"),
    ("bad_literal", b"[tru]"),
    ("truncated", b"[1,2"),
    ("invalid_escape", b"[\"\\x\"]"),
    ("nul", b"[1\x002]"),
    ("invalid_utf8", b"[\"\xff\"]"),
];

/// What comes before and after the fault: the exit status of a falsy last output before it
/// (`after_null`, `after_false`) is the fault's.
const CONTEXTS: &[(&str, &[u8], &[u8])] = &[
    ("alone", b"", b""),
    ("after_null", b"null\n", b""),
    ("after_false", b"false\n", b""),
    ("then_3", b"", b"\n3"),
];

fn document(fault: &str, context: &str) -> Vec<u8> {
    let (_, bytes) = FAULTS.iter().find(|(name, _)| *name == fault).unwrap();
    let (_, before, after) = CONTEXTS.iter().find(|(name, ..)| *name == context).unwrap();
    [*before, *bytes, *after, b"\n"].concat()
}

/// How a run reads the document.
#[derive(Clone, Copy, Debug)]
enum Route {
    /// On stdin.
    Pipe,
    /// As a file argument.
    File,
    /// The file, then a second one holding `"g"`.
    TwoFiles,
    /// `-s` over the file.
    Slurp,
    /// `-n` over the file.
    NullInput,
    /// `--stream` over the file.
    Stream,
    /// `--seq` over the file.
    Seq,
    /// `-R` over the file.
    Raw,
    /// The file after four windows of white space, read a window at a time (#7376).
    Windowed,
}

struct Run {
    stdout: String,
    stderr: String,
    code: i32,
}

fn run(route: Route, exit_status: bool, program: &str, doc: &[u8]) -> Result<Run> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("f.json");
    let second = dir.path().join("g.json");
    let body = match route {
        Route::Windowed => [&vec![b' '; small_window::MIN_BYTES][..], doc].concat(),
        _ => doc.to_vec(),
    };
    std::fs::write(&file, &body)?;
    std::fs::write(&second, "\"g\"\n")?;
    let mut args: Vec<std::ffi::OsString> = vec!["jq".into(), "-c".into()];
    if exit_status {
        args.push("-e".into());
    }
    match route {
        Route::Slurp => args.push("-s".into()),
        Route::NullInput => args.push("-n".into()),
        Route::Stream => args.push("--stream".into()),
        Route::Seq => args.push("--seq".into()),
        Route::Raw => args.push("-R".into()),
        _ => {}
    }
    args.push(program.into());
    // Named from the directory the run starts in, so the two runs' messages name the same file.
    match route {
        Route::Pipe => {}
        Route::TwoFiles => {
            args.push("f.json".into());
            args.push("g.json".into());
        }
        _ => args.push("f.json".into()),
    }
    let stdin = matches!(route, Route::Pipe).then_some(doc);
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = std::process::Command::new(succinctly_bin());
            command
                .args(&args)
                .current_dir(dir.path())
                .env_remove(small_window::ENV);
            if matches!(route, Route::Windowed) {
                small_window::apply(&mut command);
            }
            command
        },
        stdin,
    )?;
    Ok(Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code,
    })
}

/// jq's exit status under `-e`, from the run without it: a fault (or any other error) keeps its
/// code, `halt` exits 0 whatever it printed, and otherwise the last output decides.
fn exit_status_of(plain: &Run, program: &str) -> i32 {
    if plain.code != 0 || program == "halt" {
        return plain.code;
    }
    match plain.stdout.lines().last() {
        None => 4,
        Some("null" | "false") => 1,
        Some(_) => 0,
    }
}

/// Every fault, before and after other documents, under each program, on one route: the run with
/// `-e` prints what the run without it prints, and exits as jq maps that run's status.
fn e_only_changes_the_exit_status(route: Route, programs: &[&str]) -> Result<()> {
    let mut failures = Vec::new();
    for &(fault, _) in FAULTS {
        for &(context, ..) in CONTEXTS {
            let doc = document(fault, context);
            for &program in programs {
                let plain = run(route, false, program, &doc)?;
                let e = run(route, true, program, &doc)?;
                let expected = exit_status_of(&plain, program);
                if (&e.stdout, &e.stderr, e.code) != (&plain.stdout, &plain.stderr, expected) {
                    failures.push(format!(
                        "{route:?} {fault} {context} {program:?}: plain ({:?}, {:?}, {}), \
                         -e ({:?}, {:?}, {}), -e should exit {expected}",
                        plain.stdout, plain.stderr, plain.code, e.stdout, e.stderr, e.code
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

/// The programs every route runs: forwarding, two outputs of the faulty document, a navigation,
/// a generator, a caught read, nothing at all, and the stream.
const PROGRAMS: &[&str] = &[
    ".",
    ".,1",
    ".,.",
    "first",
    ".[]",
    "try . catch 1",
    "empty",
    "[inputs]",
];

#[test]
fn e_only_changes_the_exit_status_on_a_pipe_7757() -> Result<()> {
    let mut programs = PROGRAMS.to_vec();
    programs.extend([
        ".,.,.", ".[0]", "length", "null", "false", "input", "halt", "error",
    ]);
    e_only_changes_the_exit_status(Route::Pipe, &programs)
}

#[test]
fn e_only_changes_the_exit_status_on_a_file_7757() -> Result<()> {
    e_only_changes_the_exit_status(Route::File, PROGRAMS)
}

#[test]
fn e_only_changes_the_exit_status_on_two_files_7757() -> Result<()> {
    e_only_changes_the_exit_status(Route::TwoFiles, PROGRAMS)
}

#[test]
fn e_only_changes_the_exit_status_under_slurp_7757() -> Result<()> {
    e_only_changes_the_exit_status(Route::Slurp, PROGRAMS)
}

#[test]
fn e_only_changes_the_exit_status_under_null_input_7757() -> Result<()> {
    e_only_changes_the_exit_status(
        Route::NullInput,
        &[
            ".",
            "input",
            "[inputs]",
            "try input catch 1",
            "first(inputs)",
        ],
    )
}

#[test]
fn e_only_changes_the_exit_status_under_stream_7757() -> Result<()> {
    e_only_changes_the_exit_status(Route::Stream, &[".", ".,1", "first", "empty", "[inputs]"])
}

#[test]
fn e_only_changes_the_exit_status_under_seq_7757() -> Result<()> {
    e_only_changes_the_exit_status(Route::Seq, &[".", ".,1", ".,.", "first", "empty"])
}

#[test]
fn e_only_changes_the_exit_status_under_raw_input_7757() -> Result<()> {
    e_only_changes_the_exit_status(Route::Raw, &[".", ".,1", "fromjson", "empty", "[inputs]"])
}

#[test]
fn e_only_changes_the_exit_status_on_the_windowed_route_7757() -> Result<()> {
    e_only_changes_the_exit_status(Route::Windowed, &[".", ".,1", ".,.", "first", ".[]"])
}

/// `(fault, context, program, route, stdout, stderr, exit, exit under -e)`.
type JqRow = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    i32,
    i32,
);

/// `(fault, context, program, route, stdout, stderr, exit, exit under -e)` as jq 1.7.1 and 1.8.2
/// answer, for every row of the matrix where `succinctly jq` without `-e` already answers as jq
/// (the rest differ by the recorded "a filter validates only what it reads" rule, #2103, #3035,
/// #5308, and `-e` follows the run without it there too, as the tests above hold).
const JQ_ROWS: &[JqRow] = &[
    ("trailing_comma", "alone", ".", "pipe", "", "jq: parse error: Expected another array element at line 1, column 4\n", 5, 5),
    ("trailing_comma", "alone", ".", "slurp", "", "jq: parse error: Expected another array element at line 1, column 4\n", 5, 5),
    ("trailing_comma", "alone", ".,1", "pipe", "", "jq: parse error: Expected another array element at line 1, column 4\n", 5, 5),
    ("trailing_comma", "alone", ".,1", "slurp", "", "jq: parse error: Expected another array element at line 1, column 4\n", 5, 5),
    ("trailing_comma", "alone", ".,.", "pipe", "", "jq: parse error: Expected another array element at line 1, column 4\n", 5, 5),
    ("trailing_comma", "alone", ".,.", "slurp", "", "jq: parse error: Expected another array element at line 1, column 4\n", 5, 5),
    ("trailing_comma", "alone", "first", "pipe", "", "jq: parse error: Expected another array element at line 1, column 4\n", 5, 5),
    ("trailing_comma", "alone", "first", "slurp", "", "jq: parse error: Expected another array element at line 1, column 4\n", 5, 5),
    ("trailing_comma", "alone", ".[]", "slurp", "", "jq: parse error: Expected another array element at line 1, column 4\n", 5, 5),
    ("trailing_comma", "after_null", ".", "pipe", "null\n", "jq: parse error: Expected another array element at line 2, column 4\n", 5, 5),
    ("trailing_comma", "after_null", ".", "slurp", "", "jq: parse error: Expected another array element at line 2, column 4\n", 5, 5),
    ("trailing_comma", "after_null", ".,1", "pipe", "null\n1\n", "jq: parse error: Expected another array element at line 2, column 4\n", 5, 5),
    ("trailing_comma", "after_null", ".,1", "slurp", "", "jq: parse error: Expected another array element at line 2, column 4\n", 5, 5),
    ("trailing_comma", "after_null", ".,.", "pipe", "null\nnull\n", "jq: parse error: Expected another array element at line 2, column 4\n", 5, 5),
    ("trailing_comma", "after_null", ".,.", "slurp", "", "jq: parse error: Expected another array element at line 2, column 4\n", 5, 5),
    ("trailing_comma", "after_null", "first", "pipe", "null\n", "jq: parse error: Expected another array element at line 2, column 4\n", 5, 5),
    ("trailing_comma", "after_false", ".", "pipe", "false\n", "jq: parse error: Expected another array element at line 2, column 4\n", 5, 5),
    ("trailing_comma", "after_false", ".", "slurp", "", "jq: parse error: Expected another array element at line 2, column 4\n", 5, 5),
    ("trailing_comma", "after_false", ".,1", "pipe", "false\n1\n", "jq: parse error: Expected another array element at line 2, column 4\n", 5, 5),
    ("trailing_comma", "after_false", ".,1", "slurp", "", "jq: parse error: Expected another array element at line 2, column 4\n", 5, 5),
    ("trailing_comma", "after_false", ".,.", "pipe", "false\nfalse\n", "jq: parse error: Expected another array element at line 2, column 4\n", 5, 5),
    ("trailing_comma", "after_false", ".,.", "slurp", "", "jq: parse error: Expected another array element at line 2, column 4\n", 5, 5),
    ("trailing_comma", "then_3", ".", "pipe", "", "jq: parse error: Expected another array element at line 1, column 4\n", 5, 5),
    ("trailing_comma", "then_3", ".", "slurp", "", "jq: parse error: Expected another array element at line 1, column 4\n", 5, 5),
    ("trailing_comma", "then_3", ".,1", "pipe", "", "jq: parse error: Expected another array element at line 1, column 4\n", 5, 5),
    ("trailing_comma", "then_3", ".,1", "slurp", "", "jq: parse error: Expected another array element at line 1, column 4\n", 5, 5),
    ("trailing_comma", "then_3", ".,.", "pipe", "", "jq: parse error: Expected another array element at line 1, column 4\n", 5, 5),
    ("trailing_comma", "then_3", ".,.", "slurp", "", "jq: parse error: Expected another array element at line 1, column 4\n", 5, 5),
    ("trailing_comma", "then_3", "first", "pipe", "", "jq: parse error: Expected another array element at line 1, column 4\n", 5, 5),
    ("trailing_comma", "then_3", "first", "slurp", "", "jq: parse error: Expected another array element at line 1, column 4\n", 5, 5),
    ("trailing_comma", "then_3", ".[]", "slurp", "", "jq: parse error: Expected another array element at line 1, column 4\n", 5, 5),
    ("stray_closer", "alone", ".", "pipe", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "alone", ".", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "alone", ".,1", "pipe", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "alone", ".,1", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "alone", ".,.", "pipe", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "alone", ".,.", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "alone", "first", "pipe", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "alone", "first", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "alone", ".[]", "pipe", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "alone", ".[]", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "after_null", ".", "pipe", "null\n", "jq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "after_null", ".", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "after_null", ".,1", "pipe", "null\n1\n", "jq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "after_null", ".,1", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "after_null", ".,.", "pipe", "null\nnull\n", "jq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "after_null", ".,.", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "after_null", "first", "pipe", "null\n", "jq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "after_null", "first", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "after_null", ".[]", "pipe", "", "jq: error (at <stdin>:1): Cannot iterate over null (null)\njq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "after_null", ".[]", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "after_false", ".", "pipe", "false\n", "jq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "after_false", ".", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "after_false", ".,1", "pipe", "false\n1\n", "jq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "after_false", ".,1", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "after_false", ".,.", "pipe", "false\nfalse\n", "jq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "after_false", ".,.", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "after_false", "first", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "after_false", ".[]", "pipe", "", "jq: error (at <stdin>:1): Cannot iterate over boolean (false)\njq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "after_false", ".[]", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 2, column 3\n", 5, 5),
    ("stray_closer", "then_3", ".", "pipe", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "then_3", ".", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "then_3", ".,1", "pipe", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "then_3", ".,1", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "then_3", ".,.", "pipe", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "then_3", ".,.", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "then_3", "first", "pipe", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "then_3", "first", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "then_3", ".[]", "pipe", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("stray_closer", "then_3", ".[]", "slurp", "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 3\n", 5, 5),
    ("bad_literal", "alone", ".", "pipe", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "alone", ".", "slurp", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "alone", ".,1", "pipe", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "alone", ".,1", "slurp", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "alone", ".,.", "pipe", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "alone", ".,.", "slurp", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "alone", "first", "pipe", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "alone", "first", "slurp", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "alone", ".[]", "pipe", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "alone", ".[]", "slurp", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "after_null", ".", "pipe", "null\n", "jq: parse error: Invalid literal at line 2, column 5\n", 5, 5),
    ("bad_literal", "after_null", ".", "slurp", "", "jq: parse error: Invalid literal at line 2, column 5\n", 5, 5),
    ("bad_literal", "after_null", ".,1", "pipe", "null\n1\n", "jq: parse error: Invalid literal at line 2, column 5\n", 5, 5),
    ("bad_literal", "after_null", ".,1", "slurp", "", "jq: parse error: Invalid literal at line 2, column 5\n", 5, 5),
    ("bad_literal", "after_null", ".,.", "pipe", "null\nnull\n", "jq: parse error: Invalid literal at line 2, column 5\n", 5, 5),
    ("bad_literal", "after_null", ".,.", "slurp", "", "jq: parse error: Invalid literal at line 2, column 5\n", 5, 5),
    ("bad_literal", "after_null", "first", "pipe", "null\n", "jq: parse error: Invalid literal at line 2, column 5\n", 5, 5),
    ("bad_literal", "after_null", ".[]", "pipe", "", "jq: error (at <stdin>:1): Cannot iterate over null (null)\njq: parse error: Invalid literal at line 2, column 5\n", 5, 5),
    ("bad_literal", "after_false", ".", "pipe", "false\n", "jq: parse error: Invalid literal at line 2, column 5\n", 5, 5),
    ("bad_literal", "after_false", ".", "slurp", "", "jq: parse error: Invalid literal at line 2, column 5\n", 5, 5),
    ("bad_literal", "after_false", ".,1", "pipe", "false\n1\n", "jq: parse error: Invalid literal at line 2, column 5\n", 5, 5),
    ("bad_literal", "after_false", ".,1", "slurp", "", "jq: parse error: Invalid literal at line 2, column 5\n", 5, 5),
    ("bad_literal", "after_false", ".,.", "pipe", "false\nfalse\n", "jq: parse error: Invalid literal at line 2, column 5\n", 5, 5),
    ("bad_literal", "after_false", ".,.", "slurp", "", "jq: parse error: Invalid literal at line 2, column 5\n", 5, 5),
    ("bad_literal", "after_false", ".[]", "pipe", "", "jq: error (at <stdin>:1): Cannot iterate over boolean (false)\njq: parse error: Invalid literal at line 2, column 5\n", 5, 5),
    ("bad_literal", "then_3", ".", "pipe", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "then_3", ".", "slurp", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "then_3", ".,1", "pipe", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "then_3", ".,1", "slurp", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "then_3", ".,.", "pipe", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "then_3", ".,.", "slurp", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "then_3", "first", "pipe", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "then_3", "first", "slurp", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "then_3", ".[]", "pipe", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("bad_literal", "then_3", ".[]", "slurp", "", "jq: parse error: Invalid literal at line 1, column 5\n", 5, 5),
    ("truncated", "alone", ".", "pipe", "", "jq: parse error: Unfinished JSON term at EOF at line 2, column 0\n", 5, 5),
    ("truncated", "alone", ".", "slurp", "", "jq: parse error: Unfinished JSON term at EOF at line 2, column 0\n", 5, 5),
    ("truncated", "alone", ".,1", "pipe", "", "jq: parse error: Unfinished JSON term at EOF at line 2, column 0\n", 5, 5),
    ("truncated", "alone", ".,1", "slurp", "", "jq: parse error: Unfinished JSON term at EOF at line 2, column 0\n", 5, 5),
    ("truncated", "alone", ".,.", "pipe", "", "jq: parse error: Unfinished JSON term at EOF at line 2, column 0\n", 5, 5),
    ("truncated", "alone", ".,.", "slurp", "", "jq: parse error: Unfinished JSON term at EOF at line 2, column 0\n", 5, 5),
    ("truncated", "alone", "first", "pipe", "", "jq: parse error: Unfinished JSON term at EOF at line 2, column 0\n", 5, 5),
    ("truncated", "alone", "first", "slurp", "", "jq: parse error: Unfinished JSON term at EOF at line 2, column 0\n", 5, 5),
    ("truncated", "alone", ".[]", "pipe", "", "jq: parse error: Unfinished JSON term at EOF at line 2, column 0\n", 5, 5),
    ("truncated", "alone", ".[]", "slurp", "", "jq: parse error: Unfinished JSON term at EOF at line 2, column 0\n", 5, 5),
    ("truncated", "after_null", ".", "pipe", "null\n", "jq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "after_null", ".", "slurp", "", "jq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "after_null", ".,1", "pipe", "null\n1\n", "jq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "after_null", ".,1", "slurp", "", "jq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "after_null", ".,.", "pipe", "null\nnull\n", "jq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "after_null", ".,.", "slurp", "", "jq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "after_null", "first", "pipe", "null\n", "jq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "after_null", "first", "slurp", "", "jq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "after_null", ".[]", "pipe", "", "jq: error (at <stdin>:1): Cannot iterate over null (null)\njq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "after_null", ".[]", "slurp", "", "jq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "after_false", ".", "pipe", "false\n", "jq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "after_false", ".", "slurp", "", "jq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "after_false", ".,1", "pipe", "false\n1\n", "jq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "after_false", ".,1", "slurp", "", "jq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "after_false", ".,.", "pipe", "false\nfalse\n", "jq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "after_false", ".,.", "slurp", "", "jq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "after_false", "first", "slurp", "", "jq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "after_false", ".[]", "pipe", "", "jq: error (at <stdin>:1): Cannot iterate over boolean (false)\njq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "after_false", ".[]", "slurp", "", "jq: parse error: Unfinished JSON term at EOF at line 3, column 0\n", 5, 5),
    ("truncated", "then_3", ".", "pipe", "", "jq: parse error: Expected separator between values at line 3, column 0\n", 5, 5),
    ("truncated", "then_3", ".", "slurp", "", "jq: parse error: Expected separator between values at line 3, column 0\n", 5, 5),
    ("truncated", "then_3", ".,1", "pipe", "", "jq: parse error: Expected separator between values at line 3, column 0\n", 5, 5),
    ("truncated", "then_3", ".,1", "slurp", "", "jq: parse error: Expected separator between values at line 3, column 0\n", 5, 5),
    ("truncated", "then_3", ".,.", "pipe", "", "jq: parse error: Expected separator between values at line 3, column 0\n", 5, 5),
    ("truncated", "then_3", ".,.", "slurp", "", "jq: parse error: Expected separator between values at line 3, column 0\n", 5, 5),
    ("truncated", "then_3", "first", "pipe", "", "jq: parse error: Expected separator between values at line 3, column 0\n", 5, 5),
    ("truncated", "then_3", "first", "slurp", "", "jq: parse error: Expected separator between values at line 3, column 0\n", 5, 5),
    ("truncated", "then_3", ".[]", "pipe", "", "jq: parse error: Expected separator between values at line 3, column 0\n", 5, 5),
    ("truncated", "then_3", ".[]", "slurp", "", "jq: parse error: Expected separator between values at line 3, column 0\n", 5, 5),
    ("nul", "alone", ".", "pipe", "[1]\n", "", 0, 0),
    ("nul", "alone", ".", "slurp", "[[1]]\n", "", 0, 0),
    ("nul", "alone", ".,1", "pipe", "[1]\n1\n", "", 0, 0),
    ("nul", "alone", ".,1", "slurp", "[[1]]\n1\n", "", 0, 0),
    ("nul", "alone", ".,.", "pipe", "[1]\n[1]\n", "", 0, 0),
    ("nul", "alone", ".,.", "slurp", "[[1]]\n[[1]]\n", "", 0, 0),
    ("nul", "alone", "first", "pipe", "1\n", "", 0, 0),
    ("nul", "alone", "first", "slurp", "[1]\n", "", 0, 0),
    ("nul", "alone", ".[]", "pipe", "1\n", "", 0, 0),
    ("nul", "alone", ".[]", "slurp", "[1]\n", "", 0, 0),
    ("nul", "after_null", ".", "pipe", "null\n[1]\n", "", 0, 0),
    ("nul", "after_null", ".", "slurp", "[null,[1]]\n", "", 0, 0),
    ("nul", "after_null", ".,1", "pipe", "null\n1\n[1]\n1\n", "", 0, 0),
    ("nul", "after_null", ".,1", "slurp", "[null,[1]]\n1\n", "", 0, 0),
    ("nul", "after_null", ".,.", "pipe", "null\nnull\n[1]\n[1]\n", "", 0, 0),
    ("nul", "after_null", ".,.", "slurp", "[null,[1]]\n[null,[1]]\n", "", 0, 0),
    ("nul", "after_null", "first", "pipe", "null\n1\n", "", 0, 0),
    ("nul", "after_null", "first", "slurp", "null\n", "", 0, 1),
    ("nul", "after_null", ".[]", "pipe", "1\n", "jq: error (at <stdin>:1): Cannot iterate over null (null)\n", 0, 0),
    ("nul", "after_null", ".[]", "slurp", "null\n[1]\n", "", 0, 0),
    ("nul", "after_false", ".", "pipe", "false\n[1]\n", "", 0, 0),
    ("nul", "after_false", ".", "slurp", "[false,[1]]\n", "", 0, 0),
    ("nul", "after_false", ".,1", "pipe", "false\n1\n[1]\n1\n", "", 0, 0),
    ("nul", "after_false", ".,1", "slurp", "[false,[1]]\n1\n", "", 0, 0),
    ("nul", "after_false", ".,.", "pipe", "false\nfalse\n[1]\n[1]\n", "", 0, 0),
    ("nul", "after_false", ".,.", "slurp", "[false,[1]]\n[false,[1]]\n", "", 0, 0),
    ("nul", "after_false", "first", "slurp", "false\n", "", 0, 1),
    ("nul", "after_false", ".[]", "pipe", "1\n", "jq: error (at <stdin>:1): Cannot iterate over boolean (false)\n", 0, 0),
    ("nul", "after_false", ".[]", "slurp", "false\n[1]\n", "", 0, 0),
    ("nul", "then_3", ".", "pipe", "[1]\n3\n", "", 0, 0),
    ("nul", "then_3", ".", "slurp", "[[1],3]\n", "", 0, 0),
    ("nul", "then_3", ".,1", "pipe", "[1]\n1\n3\n1\n", "", 0, 0),
    ("nul", "then_3", ".,1", "slurp", "[[1],3]\n1\n", "", 0, 0),
    ("nul", "then_3", ".,.", "pipe", "[1]\n[1]\n3\n3\n", "", 0, 0),
    ("nul", "then_3", ".,.", "slurp", "[[1],3]\n[[1],3]\n", "", 0, 0),
    ("nul", "then_3", "first", "slurp", "[1]\n", "", 0, 0),
    ("nul", "then_3", ".[]", "pipe", "1\n", "jq: error (at <stdin>:2): Cannot iterate over number (3)\n", 5, 5),
    ("nul", "then_3", ".[]", "slurp", "[1]\n3\n", "", 0, 0),
    ("invalid_utf8", "alone", ".", "pipe", "[\"\u{fffd}\"]\n", "", 0, 0),
    ("invalid_utf8", "alone", ".", "slurp", "[[\"\u{fffd}\"]]\n", "", 0, 0),
    ("invalid_utf8", "alone", ".,1", "pipe", "[\"\u{fffd}\"]\n1\n", "", 0, 0),
    ("invalid_utf8", "alone", ".,1", "slurp", "[[\"\u{fffd}\"]]\n1\n", "", 0, 0),
    ("invalid_utf8", "alone", ".,.", "pipe", "[\"\u{fffd}\"]\n[\"\u{fffd}\"]\n", "", 0, 0),
    ("invalid_utf8", "alone", ".,.", "slurp", "[[\"\u{fffd}\"]]\n[[\"\u{fffd}\"]]\n", "", 0, 0),
    ("invalid_utf8", "alone", "first", "pipe", "\"\u{fffd}\"\n", "", 0, 0),
    ("invalid_utf8", "alone", "first", "slurp", "[\"\u{fffd}\"]\n", "", 0, 0),
    ("invalid_utf8", "alone", ".[]", "pipe", "\"\u{fffd}\"\n", "", 0, 0),
    ("invalid_utf8", "alone", ".[]", "slurp", "[\"\u{fffd}\"]\n", "", 0, 0),
    ("invalid_utf8", "after_null", ".", "pipe", "null\n[\"\u{fffd}\"]\n", "", 0, 0),
    ("invalid_utf8", "after_null", ".", "slurp", "[null,[\"\u{fffd}\"]]\n", "", 0, 0),
    ("invalid_utf8", "after_null", ".,1", "pipe", "null\n1\n[\"\u{fffd}\"]\n1\n", "", 0, 0),
    ("invalid_utf8", "after_null", ".,1", "slurp", "[null,[\"\u{fffd}\"]]\n1\n", "", 0, 0),
    ("invalid_utf8", "after_null", ".,.", "pipe", "null\nnull\n[\"\u{fffd}\"]\n[\"\u{fffd}\"]\n", "", 0, 0),
    ("invalid_utf8", "after_null", ".,.", "slurp", "[null,[\"\u{fffd}\"]]\n[null,[\"\u{fffd}\"]]\n", "", 0, 0),
    ("invalid_utf8", "after_null", "first", "pipe", "null\n\"\u{fffd}\"\n", "", 0, 0),
    ("invalid_utf8", "after_null", "first", "slurp", "null\n", "", 0, 1),
    ("invalid_utf8", "after_null", ".[]", "pipe", "\"\u{fffd}\"\n", "jq: error (at <stdin>:1): Cannot iterate over null (null)\n", 0, 0),
    ("invalid_utf8", "after_null", ".[]", "slurp", "null\n[\"\u{fffd}\"]\n", "", 0, 0),
    ("invalid_utf8", "after_false", ".", "pipe", "false\n[\"\u{fffd}\"]\n", "", 0, 0),
    ("invalid_utf8", "after_false", ".", "slurp", "[false,[\"\u{fffd}\"]]\n", "", 0, 0),
    ("invalid_utf8", "after_false", ".,1", "pipe", "false\n1\n[\"\u{fffd}\"]\n1\n", "", 0, 0),
    ("invalid_utf8", "after_false", ".,1", "slurp", "[false,[\"\u{fffd}\"]]\n1\n", "", 0, 0),
    ("invalid_utf8", "after_false", ".,.", "pipe", "false\nfalse\n[\"\u{fffd}\"]\n[\"\u{fffd}\"]\n", "", 0, 0),
    ("invalid_utf8", "after_false", ".,.", "slurp", "[false,[\"\u{fffd}\"]]\n[false,[\"\u{fffd}\"]]\n", "", 0, 0),
    ("invalid_utf8", "after_false", "first", "slurp", "false\n", "", 0, 1),
    ("invalid_utf8", "after_false", ".[]", "pipe", "\"\u{fffd}\"\n", "jq: error (at <stdin>:1): Cannot iterate over boolean (false)\n", 0, 0),
    ("invalid_utf8", "after_false", ".[]", "slurp", "false\n[\"\u{fffd}\"]\n", "", 0, 0),
    ("invalid_utf8", "then_3", ".", "pipe", "[\"\u{fffd}\"]\n3\n", "", 0, 0),
    ("invalid_utf8", "then_3", ".", "slurp", "[[\"\u{fffd}\"],3]\n", "", 0, 0),
    ("invalid_utf8", "then_3", ".,1", "pipe", "[\"\u{fffd}\"]\n1\n3\n1\n", "", 0, 0),
    ("invalid_utf8", "then_3", ".,1", "slurp", "[[\"\u{fffd}\"],3]\n1\n", "", 0, 0),
    ("invalid_utf8", "then_3", ".,.", "pipe", "[\"\u{fffd}\"]\n[\"\u{fffd}\"]\n3\n3\n", "", 0, 0),
    ("invalid_utf8", "then_3", ".,.", "slurp", "[[\"\u{fffd}\"],3]\n[[\"\u{fffd}\"],3]\n", "", 0, 0),
    ("invalid_utf8", "then_3", "first", "slurp", "[\"\u{fffd}\"]\n", "", 0, 0),
    ("invalid_utf8", "then_3", ".[]", "pipe", "\"\u{fffd}\"\n", "jq: error (at <stdin>:2): Cannot iterate over number (3)\n", 5, 5),
    ("invalid_utf8", "then_3", ".[]", "slurp", "[\"\u{fffd}\"]\n3\n", "", 0, 0),
];

#[test]
fn e_and_plain_runs_answer_as_jq_7757() -> Result<()> {
    let mut failures = Vec::new();
    for &(fault, context, program, route, stdout, stderr, code, e_code) in JQ_ROWS {
        let route = match route {
            "pipe" => Route::Pipe,
            "slurp" => Route::Slurp,
            other => unreachable!("{other}"),
        };
        let doc = document(fault, context);
        for (exit_status, want) in [(false, code), (true, e_code)] {
            let got = run(route, exit_status, program, &doc)?;
            if (got.stdout.as_str(), got.stderr.as_str(), got.code) != (stdout, stderr, want) {
                failures.push(format!(
                    "{route:?} {fault} {context} {program:?} -e={exit_status}: \
                     ({:?}, {:?}, {}) where jq answers ({stdout:?}, {stderr:?}, {want})",
                    got.stdout, got.stderr, got.code
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

/// The issue's rows, spelled out: the fault ends the document under `-e`, as without it.
#[test]
fn a_fault_ends_the_document_under_e_7757() -> Result<()> {
    let error = "jq: parse error: Expected another array element at line 1, column 4\n";
    for program in [".,1", ".,.", ".,.,."] {
        for exit_status in [false, true] {
            let got = run(Route::Pipe, exit_status, program, b"[2,]\n")?;
            assert_eq!(
                (got.stdout.as_str(), got.stderr.as_str(), got.code),
                ("", error, 5),
                "{program:?} -e={exit_status}"
            );
        }
    }
    Ok(())
}

/// A string with an invalid escape is forwarded by `.` without `-e` (#5308, recorded as a
/// divergence: jq refuses the document at parse time), and `-e` reads only whether it is truthy,
/// so it forwards it too and exits 0 (it raised before #7757).
#[test]
fn e_forwards_what_the_run_without_it_forwards_7757() -> Result<()> {
    for exit_status in [false, true] {
        let got = run(Route::Pipe, exit_status, ".", b"[\"\\x\"]\n")?;
        assert_eq!(
            (got.stdout.as_str(), got.stderr.as_str(), got.code),
            ("[\"\\x\"]\n", "", 0),
            "-e={exit_status}"
        );
    }
    Ok(())
}
