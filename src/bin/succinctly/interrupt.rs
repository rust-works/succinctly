//! Ctrl+C handling for the benchmark suites (`dev bench`, `utf8 bench`, ...): on Ctrl+C they
//! write the results gathered so far instead of dying with nothing.
//!
//! The handler needs `ctrlc`, which on Apple targets links `libobjc`, `Foundation` and
//! `CoreFoundation` and costs every start-up of the binary about 0.7 ms (#5551). Only the
//! benchmark runners want it, so it is a `bench-runner` dependency; without that feature the
//! suites run as before and Ctrl+C ends them the default way, with no partial results.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;

/// Make Ctrl+C set `interrupted` (and say so) instead of ending the process.
#[cfg(feature = "bench-runner")]
pub fn install(interrupted: &Arc<AtomicBool>) -> Result<()> {
    use anyhow::Context;
    use std::sync::atomic::Ordering;

    let flag = Arc::clone(interrupted);
    ctrlc::set_handler(move || {
        flag.store(true, Ordering::SeqCst);
        eprintln!("\nInterrupted! Writing partial results...");
    })
    .context("Failed to set Ctrl+C handler")
}

/// Without `bench-runner` there is no handler: Ctrl+C keeps its default action.
#[cfg(not(feature = "bench-runner"))]
#[allow(clippy::unnecessary_wraps)] // the same signature as the `bench-runner` build's
pub fn install(_interrupted: &Arc<AtomicBool>) -> Result<()> {
    Ok(())
}
