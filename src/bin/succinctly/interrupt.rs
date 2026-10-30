//! Ctrl+C handling for the benchmark suites (`dev bench`, `utf8 bench`, ...): on Ctrl+C they
//! write the results gathered so far instead of dying with nothing.
//!
//! On Unix this is a plain `SIGINT` handler that stores into the suite's flag, through the
//! `libc` the CLI already depends on. It used to be `ctrlc`, which on Apple targets links
//! `libobjc`, `Foundation` and `CoreFoundation` and cost every start-up of the binary about
//! 0.7 ms (#5551). `ctrlc` stays for the orchestrator, and for Windows `bench-runner` builds;
//! a Windows build without `bench-runner` has no handler, and Ctrl+C ends a suite as usual.
//!
//! One suite runs per process: a second `install` points the handler at the new flag.

#![allow(unsafe_code)] // a signal handler and the one pointer it reads

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;

#[cfg(unix)]
mod imp {
    use std::sync::atomic::{AtomicPtr, Ordering};

    use super::{Arc, AtomicBool, Result};

    /// The flag the handler sets: a leaked `Arc` clone, so the pointer outlives every use.
    static TARGET: AtomicPtr<AtomicBool> = AtomicPtr::new(std::ptr::null_mut());

    const MESSAGE: &[u8] = b"\nInterrupted! Writing partial results...\n";

    extern "C" fn on_sigint(_signal: libc::c_int) {
        let flag = TARGET.load(Ordering::SeqCst);
        if !flag.is_null() {
            // SAFETY: `TARGET` only ever holds a pointer from `Arc::into_raw`, never freed.
            unsafe { (*flag).store(true, Ordering::SeqCst) };
        }
        // SAFETY: `write(2)` is async-signal-safe; a short or failed write is ignored.
        unsafe {
            libc::write(libc::STDERR_FILENO, MESSAGE.as_ptr().cast(), MESSAGE.len());
        }
    }

    pub fn install(interrupted: &Arc<AtomicBool>) -> Result<()> {
        TARGET.store(
            Arc::into_raw(Arc::clone(interrupted)).cast_mut(),
            Ordering::SeqCst,
        );
        // SAFETY: installs an `extern "C"` handler that only touches an atomic and calls `write`.
        let previous =
            unsafe { libc::signal(libc::SIGINT, on_sigint as *const () as libc::sighandler_t) };
        if previous == libc::SIG_ERR {
            anyhow::bail!(
                "Failed to set Ctrl+C handler: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn restore_default() {
        // SAFETY: puts the default action back after a test raised `SIGINT` at itself.
        unsafe { libc::signal(libc::SIGINT, libc::SIG_DFL) };
    }
}

#[cfg(all(not(unix), feature = "bench-runner"))]
mod imp {
    use std::sync::atomic::Ordering;

    use anyhow::Context;

    use super::{Arc, AtomicBool, Result};

    pub fn install(interrupted: &Arc<AtomicBool>) -> Result<()> {
        let flag = Arc::clone(interrupted);
        ctrlc::set_handler(move || {
            flag.store(true, Ordering::SeqCst);
            eprintln!("\nInterrupted! Writing partial results...");
        })
        .context("Failed to set Ctrl+C handler")
    }
}

#[cfg(all(not(unix), not(feature = "bench-runner")))]
mod imp {
    use super::{Arc, AtomicBool, Result};

    #[allow(clippy::unnecessary_wraps)] // the same signature as the other builds'
    pub fn install(_interrupted: &Arc<AtomicBool>) -> Result<()> {
        Ok(())
    }
}

/// Make Ctrl+C set `interrupted` (and say so) instead of ending the process.
pub fn install(interrupted: &Arc<AtomicBool>) -> Result<()> {
    imp::install(interrupted)
}

#[cfg(all(test, unix))]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;

    /// Raising `SIGINT` at the process sets the flag instead of ending it.
    #[test]
    fn sigint_sets_the_installed_flag_5551() {
        let flag = Arc::new(AtomicBool::new(false));
        install(&flag).expect("install");
        assert!(!flag.load(Ordering::SeqCst));
        // SAFETY: delivers the signal this test just installed a handler for.
        assert_eq!(unsafe { libc::raise(libc::SIGINT) }, 0);
        assert!(flag.load(Ordering::SeqCst));
        imp::restore_default();
    }
}
