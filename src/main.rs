//! The `succinctly` binary: the CLI lives in the library as `succinctly::cli` (#7384).

use std::process::ExitCode;

/// An error `main` returns is reported as `fn main() -> anyhow::Result<()>` reports it (`Error: {e:?}`
/// and status 1), by a write whose failure is ignored (#7476). Recent toolchains' `Termination` report
/// ignores it too, but older ones (the 1.73 MSRV among them) print with `eprintln!`, which panics
/// (exit 101) when standard error cannot be written.
fn main() -> ExitCode {
    match succinctly::cli::main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            use std::io::Write;
            let _ = writeln!(std::io::stderr(), "Error: {e:?}");
            ExitCode::FAILURE
        }
    }
}
