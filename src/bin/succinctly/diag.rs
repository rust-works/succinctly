//! Diagnostics that a standard error which cannot be written does not turn into a panic (#7476).
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

    /// #7476: no path in the CLI prints to standard error through a write that panics on failure.
    #[test]
    fn cli_source_has_no_panicking_stderr_write_7476() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let cli = root.join("src/bin/succinctly");
        let mut files = Vec::new();
        rust_files(&cli, &mut files);
        // The binary's own crate: the shadow does not reach it, so any `eprint` there is std's.
        let binary_root = root.join("src/main.rs");
        files.push(binary_root.clone());
        let this_file = cli.join("diag.rs");

        let mut offenders = Vec::new();
        for file in &files {
            if *file == this_file {
                continue;
            }
            let needles: &[&str] = if *file == binary_root {
                &["eprint", "dbg!("]
            } else {
                &["std::eprint", "dbg!("]
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
            "a write to standard error that panics when it fails (#7476): use `eprintln!`/`eprint!` \
             unqualified (diag.rs shadows them) or `let _ = writeln!(std::io::stderr(), ...)`:\n{}",
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
    }
}
