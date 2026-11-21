//! Diagnostics that a standard error which cannot be written does not turn into a panic (#7476), and
//! output that a standard output which cannot be written does not turn into one either (#7538).
//!
//! `std::eprintln!` panics with `failed printing to stderr` when the write fails: a standard error
//! on a full disk or `/dev/full` (`ENOSPC`), on a file past `ulimit -f` (`EFBIG`), or on a pipe whose
//! reader has gone while SIGPIPE is ignored (`EPIPE`). Every error report then became exit 101,
//! where jq ignores its failed `fprintf(stderr, ...)` and exits 5, 2 or 3 as it would have, and yq
//! exits 1.
//!
//! `main.rs` declares this module first, with `#[macro_use]`, so the two macros below shadow the
//! standard library's `eprintln!` and `eprint!` in every module of the CLI: a call site keeps its
//! ordinary spelling and cannot reach the panicking one. Only a path-qualified `std::eprintln!`
//! would, and `cli_source_has_no_panicking_stderr_write_7476` below fails on one, on `dbg!`
//! (which prints through `std::eprintln!`), and on this module losing its place at the top of
//! `main.rs`. A write to standard error elsewhere in the CLI ignores its result the same way
//! (`let _ = writeln!(std::io::stderr(), ...)`).
//!
//! The standard output side is the same shape, with a different outcome (#7538). `std::println!` and
//! `std::print!` panic with `failed printing to stdout` when the write fails (`ENOSPC` on a full disk
//! or `/dev/full`, `EFBIG` past `ulimit -f`, `EPIPE` on a pipe whose reader has gone), so `json
//! generate`, `dsv generate`, `yaml generate`, `jq-locate`, `yq-locate`, the build-configuration
//! reports and the `bench` tables ended in exit 101 and a Rust panic message. The `println!`/`print!`
//! below shadow them the same way, and a failed write is what `succinctly jq` reports for one
//! (#7214): `succinctly: error: writing output failed: <strerror>` on standard error, exit 2, as
//! jq's `jq: error: writing output failed: <strerror>`, exit 2. A closed descriptor is a failed
//! write too (std would treat `EBADF` as success). A closed pipe (`| head`) is not an error report:
//! where there is a SIGPIPE the run dies of it, as `succinctly jq` and jq do (#5547); elsewhere it
//! exits quietly.
//!
//! The panic hook is not changed: the standard library's own hook already ignores a failed write
//! of the panic message, so a panic on a dead standard error is still exit 101, not an abort. Nor is
//! `src/main.rs`, which the shadow does not reach (it is the binary's own crate): the `Error: ...`
//! report of an error it returns is printed by the standard `Termination` impl, which ignores a
//! failed write since Rust 1.85 (below the crate's MSRV), and the scan below fails on any `eprint`
//! there.

/// `eprint!` that ignores a failed write, as jq ignores `fprintf(stderr, ...)`.
macro_rules! eprint {
    ($($arg:tt)*) => {
        $crate::cli::diag::write_stderr(format_args!($($arg)*))
    };
}

/// `eprintln!` that ignores a failed write, as jq ignores `fprintf(stderr, ...)`.
macro_rules! eprintln {
    () => {
        $crate::cli::diag::write_stderr(format_args!("\n"))
    };
    ($($arg:tt)*) => {
        $crate::cli::diag::write_stderr(format_args!("{}\n", format_args!($($arg)*)))
    };
}

/// `print!` that reports a failed write on standard error and exits 2 instead of panicking (#7538).
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::cli::diag::write_stdout(format_args!($($arg)*))
    };
}

/// `println!` that reports a failed write on standard error and exits 2 instead of panicking (#7538).
macro_rules! println {
    () => {
        $crate::cli::diag::write_stdout(format_args!("\n"))
    };
    ($($arg:tt)*) => {
        $crate::cli::diag::write_stdout(format_args!("{}\n", format_args!($($arg)*)))
    };
}

/// Write `args` to standard error, ignoring a failed write (#7476).
///
/// Under `cfg(test)` it prints through the standard macro instead, which the test harness captures
/// per test; a panic there costs nothing, and the binary is what the CLI tests run.
pub(crate) fn write_stderr(args: std::fmt::Arguments<'_>) {
    #[cfg(test)]
    {
        std::eprint!("{args}");
    }
    #[cfg(not(test))]
    {
        use std::io::Write;
        // One `write_fmt` under the lock, as `std::eprint!` does, so a line is not split by
        // another thread's.
        let _ = std::io::stderr().lock().write_fmt(args);
    }
}

/// Write `args` to standard output. A failed write ends the process (#7538): a closed pipe by
/// SIGPIPE (quietly, status 1 where there is none), anything else as `succinctly: error: writing
/// output failed: <strerror>` on standard error and status 2, which is how `succinctly jq` ends one.
///
/// Under `cfg(test)` it prints through the standard macro instead, which the test harness captures
/// per test; a panic there costs nothing, and the binary is what the CLI tests run.
pub(crate) fn write_stdout(args: std::fmt::Arguments<'_>) {
    #[cfg(test)]
    {
        std::print!("{args}");
    }
    #[cfg(not(test))]
    {
        if let Err(failure) = try_write_stdout(args) {
            stdout_write_failed(failure);
        }
    }
}

/// Why a write to standard output failed.
#[cfg(not(test))]
enum StdoutFailure {
    /// The reader of a pipe has gone.
    BrokenPipe,
    /// Any other failure; the `strerror` text.
    Other(String),
}

#[cfg(not(test))]
fn try_write_stdout(args: std::fmt::Arguments<'_>) -> Result<(), StdoutFailure> {
    use std::io::Write;
    // Whatever another part of the CLI left in the standard library's buffer goes first, so the two
    // writers cannot reorder. A failure here is the same condition the write below reports.
    let _ = std::io::stdout().flush();
    // The `jq` subcommand's writer: unbuffered on a duplicate of descriptor 1, and a closed
    // descriptor is an error (`std::io::Stdout` treats `EBADF` as success, #7214).
    let mut out = super::output::JqStdout::new();
    out.write_fmt(args).map_err(|e| {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            return StdoutFailure::BrokenPipe;
        }
        match e
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<super::output::OutputWriteFailed>())
        {
            Some(failed) => StdoutFailure::Other(failed.0.clone()),
            None => StdoutFailure::Other(e.to_string()),
        }
    })
}

#[cfg(not(test))]
fn stdout_write_failed(failure: StdoutFailure) -> ! {
    match failure {
        StdoutFailure::BrokenPipe => {
            // Die of SIGPIPE as jq does (#5547): restore the default action, then write again.
            super::sigpipe::restore_default();
            let _ = try_write_stdout(format_args!("\n"));
            std::process::exit(1)
        }
        StdoutFailure::Other(strerror) => {
            eprintln!("succinctly: error: writing output failed: {strerror}");
            std::process::exit(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    fn rust_files(dir: &Path, into: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("readable source directory") {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                rust_files(&path, into);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                into.push(path);
            }
        }
    }

    /// The code part of a line: what precedes a `//` comment (string literals holding `//` are rare
    /// enough in this tree that the scan reads past them).
    fn code_of(line: &str) -> &str {
        line.find("//").map_or(line, |at| &line[..at])
    }

    /// #7476, #7538: no path in the CLI prints to standard error or standard output through a write
    /// that panics on failure.
    #[test]
    fn cli_source_has_no_panicking_stderr_write_7476() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let cli = root.join("src/bin/succinctly");
        let mut files = Vec::new();
        rust_files(&cli, &mut files);
        // The binary's own crate: the shadow does not reach it, so any `print` or `eprint` there is std's.
        let binary_root = root.join("src/main.rs");
        files.push(binary_root.clone());
        let this_file = cli.join("diag.rs");

        let mut offenders = Vec::new();
        for file in &files {
            if *file == this_file {
                continue;
            }
            let needles: &[&str] = if *file == binary_root {
                &["print", "dbg!("]
            } else {
                &["std::eprint", "std::print", "dbg!("]
            };
            let text = std::fs::read_to_string(file).expect("readable source file");
            for (number, line) in text.lines().enumerate() {
                let code = code_of(line);
                let bypasses_the_shadow = needles.iter().any(|needle| code.contains(needle));
                if bypasses_the_shadow {
                    offenders.push(format!(
                        "{}:{}: {}",
                        file.display(),
                        number + 1,
                        line.trim()
                    ));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "a write to standard error or output that panics when it fails (#7476, #7538): use \
             `eprintln!`/`eprint!`/`println!`/`print!` unqualified (diag.rs shadows them), \
             `let _ = writeln!(std::io::stderr(), ...)`, or a `Write` whose error is returned:\n{}",
            offenders.join("\n")
        );

        // The shadow reaches a module only if it is declared before it, and `main.rs`'s own body
        // prints too: `#[macro_use] mod diag;` has to be its first item.
        let main = std::fs::read_to_string(cli.join("main.rs")).expect("readable main.rs");
        let first_item = main
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with("//"))
            .expect("main.rs has items");
        assert_eq!(
            first_item, "#[macro_use]",
            "`#[macro_use] mod diag;` must be the first item of main.rs (#7476)"
        );
        let after = main
            .split_once("#[macro_use]")
            .map(|(_, rest)| rest.trim_start())
            .expect("checked above");
        assert!(
            after.starts_with("mod diag;"),
            "`#[macro_use] mod diag;` must be the first item of main.rs (#7476)"
        );
    }

    /// The shadowing macros format as the standard ones do (inline captures, trailing commas, no
    /// arguments).
    #[test]
    fn the_shadowing_macros_accept_every_standard_form_7476() {
        let value = 7;
        eprintln!();
        eprintln!("plain");
        eprintln!("inline {value}");
        eprintln!("positional {} {}", value, "x",);
        eprint!("no newline {value}");
        eprint!("\n");
        println!();
        println!("plain");
        println!("inline {value}");
        println!("positional {} {}", value, "x",);
        print!("no newline {value}");
        print!("\n");
    }
}
