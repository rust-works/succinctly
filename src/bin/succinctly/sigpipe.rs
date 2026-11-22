//! Die of SIGPIPE like jq and yq do (#5547), unless the process was started with it ignored (#7732).
//!
//! Rust starts every process with SIGPIPE ignored, so a write to a closed pipe comes back as
//! `EPIPE` and the program has to report it: `succinctly jq -n 'range(10000000)' | head -n1`
//! printed `failed to flush output: Broken pipe` and exited 1. jq (C) and yq (Go) leave the
//! default disposition, so the same pipeline ends their process with SIGPIPE: no message,
//! status 141 under a shell, which is what a reader that stops early expects.
//!
//! jq does not set the disposition, it keeps the one it inherited. A parent that ignores SIGPIPE
//! (a systemd unit, Python's `subprocess` with `restore_signals=False`, `trap '' PIPE`) hands that
//! on through `exec`, and jq's write to a closed pipe then fails `EPIPE`: it reports `jq: error:
//! writing output failed: Broken pipe` and exits 2, and a failed write to standard error is ignored
//! as any other is (#7732). Rust's runtime has replaced the inherited disposition by the time `main`
//! runs, so [`NOTE_INHERITED`] reads it before that, from the loader's list of initialisers, and
//! [`restore_inherited`] puts back what it found. yq is not the same: Go's runtime raises SIGPIPE for
//! `EPIPE` on descriptors 1 and 2 whatever was inherited (yq v4.53.3 dies of it under `trap '' PIPE`
//! too), so `succinctly yq` keeps [`restore_default`].
//!
//! Restored for the two query subcommands only. The benchmark orchestrator and the generators
//! keep the ignored disposition they handle `EPIPE` under.

#![allow(unsafe_code)] // `signal(2)`, `sigaction(2)`, and one initialiser the loader runs before `main`

use core::sync::atomic::{AtomicBool, Ordering};

/// Whether SIGPIPE was ignored when the process started, before Rust's runtime set it to ignored.
static INHERITED_IGNORED: AtomicBool = AtomicBool::new(false);

/// Read the disposition the process inherited (#7732). Runs before `main`, so it does no more than
/// one `sigaction` query and an atomic store.
#[cfg(unix)]
extern "C" fn note_inherited() {
    // SAFETY: `sigaction` with a null new action only reads the current one into `old`, which is a
    // plain C struct that all-zero bytes are a valid value of.
    let ignored = unsafe {
        let mut old: libc::sigaction = core::mem::zeroed();
        libc::sigaction(libc::SIGPIPE, core::ptr::null(), &mut old) == 0
            && old.sa_sigaction == libc::SIG_IGN
    };
    INHERITED_IGNORED.store(ignored, Ordering::Relaxed);
}

/// [`note_inherited`] in the executable's list of initialisers, which the loader runs before `main`
/// and so before Rust's runtime sets SIGPIPE to ignored: `.init_array` on ELF platforms, the
/// `__mod_init_func` section on Apple's. `#[used]` keeps it in the link, from the library crate the
/// CLI lives in (#7384). On a Unix that has neither section the static is inert and SIGPIPE is taken
/// to have been at its default, which is what every run assumed before #7732.
#[cfg(unix)]
#[used]
#[cfg_attr(
    any(
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly",
        target_os = "illumos",
        target_os = "solaris"
    ),
    link_section = ".init_array"
)]
#[cfg_attr(target_vendor = "apple", link_section = "__DATA,__mod_init_func")]
static NOTE_INHERITED: extern "C" fn() = note_inherited;

/// Whether the process was started with SIGPIPE ignored (#7732). `false` where there is no SIGPIPE.
pub fn inherited_ignored() -> bool {
    INHERITED_IGNORED.load(Ordering::Relaxed)
}

/// Put SIGPIPE back to its default action. A no-op where there is no SIGPIPE (Windows).
pub fn restore_default() {
    #[cfg(unix)]
    {
        // SAFETY: `signal` with `SIG_DFL` installs no handler and reads or writes no Rust data.
        // It is async-signal-safe and process-wide, so it does not matter that `main` has
        // already started the evaluation thread this runs on; nothing in this program observes
        // the disposition except the kernel, when a write finds a pipe closed.
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        }
    }
}

/// Put SIGPIPE back to the disposition the process inherited, as jq keeps it (#7732): the default
/// action unless it was started with SIGPIPE ignored, which Rust's runtime has already set again.
pub fn restore_inherited() {
    if !inherited_ignored() {
        restore_default();
    }
}
