//! Ctrl+C handling for the `dev bench` commands.
//!
//! `ctrlc` is only a dependency of `bench-runner`: on Apple targets it brings in `objc2` and links
//! Foundation, which costs every `cli` start ~0.75 ms and ~4 MB (#5551). Without the feature
//! [`on_interrupt`] does nothing, so Ctrl+C takes the default action and the process ends at once.

use anyhow::Result;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Set `flag` and print `message` when the process is interrupted (a no-op without `bench-runner`).
#[cfg(feature = "bench-runner")]
pub fn on_interrupt(flag: &Arc<AtomicBool>, message: &'static str) -> Result<()> {
    use anyhow::Context;
    use std::sync::atomic::Ordering;

    let flag = Arc::clone(flag);
    ctrlc::set_handler(move || {
        flag.store(true, Ordering::SeqCst);
        eprintln!("\n{message}");
    })
    .context("Failed to set Ctrl+C handler")
}

#[cfg(not(feature = "bench-runner"))]
pub fn on_interrupt(_flag: &Arc<AtomicBool>, _message: &'static str) -> Result<()> {
    Ok(())
}
