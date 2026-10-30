//! Ctrl+C handling for the `dev bench` commands: on Ctrl+C they write the results gathered so far
//! instead of dying with nothing.
//!
//! On Unix this is a plain `SIGINT` handler that stores into the suite's flag, through the `libc`
//! the CLI already depends on. `ctrlc` is only a dependency of `bench-runner`: on Apple targets it
//! brings in `objc2` and links Foundation, which costs every `cli` start ~0.75 ms and ~4 MB (#5551).
//! `ctrlc` serves Windows `bench-runner` builds; a Windows build without that feature has no
//! handler, and Ctrl+C takes the default action and ends the process at once.
//!
//! One suite runs per process: a second [`on_interrupt`] points the handler at the new flag.

#![allow(unsafe_code)] // a signal handler and the one pointer it reads

use anyhow::Result;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[cfg(unix)]
mod imp {
    use std::sync::atomic::{AtomicPtr, Ordering};

    use super::{Arc, AtomicBool, Result};

    /// What the handler does: set `flag`, then print `message`.
    struct Target {
        flag: Arc<AtomicBool>,
        message: &'static str,
    }

    /// A leaked `Target`, so the pointer outlives every use of it.
    static TARGET: AtomicPtr<Target> = AtomicPtr::new(std::ptr::null_mut());

    extern "C" fn on_sigint(_signal: libc::c_int) {
        let target = TARGET.load(Ordering::SeqCst);
        if target.is_null() {
            return;
        }
        // SAFETY: `TARGET` only ever holds a pointer from `Box::into_raw`, never freed.
        let target = unsafe { &*target };
        target.flag.store(true, Ordering::SeqCst);
        // SAFETY: `write(2)` is async-signal-safe; a short or failed write is ignored.
        unsafe {
            libc::write(libc::STDERR_FILENO, b"\n".as_ptr().cast(), 1);
            libc::write(
                libc::STDERR_FILENO,
                target.message.as_ptr().cast(),
                target.message.len(),
            );
            libc::write(libc::STDERR_FILENO, b"\n".as_ptr().cast(), 1);
        }
    }

    pub fn on_interrupt(flag: &Arc<AtomicBool>, message: &'static str) -> Result<()> {
        let target = Box::new(Target {
            flag: Arc::clone(flag),
            message,
        });
        TARGET.store(Box::into_raw(target), Ordering::SeqCst);
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

    pub fn on_interrupt(flag: &Arc<AtomicBool>, message: &'static str) -> Result<()> {
        let flag = Arc::clone(flag);
        ctrlc::set_handler(move || {
            flag.store(true, Ordering::SeqCst);
            eprintln!("\n{message}");
        })
        .context("Failed to set Ctrl+C handler")
    }
}

#[cfg(all(not(unix), not(feature = "bench-runner")))]
mod imp {
    use super::{Arc, AtomicBool, Result};

    #[allow(clippy::unnecessary_wraps)] // the same signature as the other builds'
    pub fn on_interrupt(_flag: &Arc<AtomicBool>, _message: &'static str) -> Result<()> {
        Ok(())
    }
}

/// Set `flag` and print `message` when the process is interrupted (Unix, or a Windows `bench-runner`
/// build; otherwise a no-op).
pub fn on_interrupt(flag: &Arc<AtomicBool>, message: &'static str) -> Result<()> {
    imp::on_interrupt(flag, message)
}

#[cfg(all(test, unix))]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;

    /// Raising `SIGINT` at the process sets the flag instead of ending it.
    #[test]
    fn sigint_sets_the_installed_flag_5551() {
        let flag = Arc::new(AtomicBool::new(false));
        on_interrupt(&flag, "Interrupted!").expect("install");
        assert!(!flag.load(Ordering::SeqCst));
        // SAFETY: delivers the signal this test just installed a handler for.
        assert_eq!(unsafe { libc::raise(libc::SIGINT) }, 0);
        assert!(flag.load(Ordering::SeqCst));
        imp::restore_default();
    }
}
