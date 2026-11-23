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
//! A parent can instead leave the disposition at its default and block SIGPIPE in the mask it hands
//! on (#7857), which `exec` keeps as it keeps an ignored disposition. jq's write to a closed pipe
//! then fails `EPIPE` too, the signal staying pending until the process ends, and jq reports the
//! write the same way. [`NOTE_INHERITED`] reads the mask as well, and
//! [`closed_pipe_is_a_failed_write`] says whether either was inherited. Go unblocks SIGPIPE before
//! it raises it for descriptors 1 and 2, so yq v4.53.3 dies of it under a blocked mask as well, and
//! [`restore_default_on_this_thread`] lifts an inherited block on the thread that runs `succinctly yq`.
//!
//! Restored for the two query subcommands only. The benchmark orchestrator and the generators
//! keep the ignored disposition they handle `EPIPE` under.
//!
//! The signal has to reach the thread that wrote (#7847). macOS raises a write's SIGPIPE on the
//! process, not on the writing thread, and the kernel hands a process-wide signal to the first
//! thread that does not block it: the main thread, which waits in `join` while the evaluation
//! thread runs the query. The writer got `EPIPE` back and carried on, and when it ended the process
//! itself (`process::exit` with the run's status) before the main thread was scheduled to act on
//! the signal, the run exited normally: `debug` to a closed standard error now and then exited 0
//! under load, where jq, which has one thread, dies of SIGPIPE before its write returns.
//! [`block_on_main_thread`] blocks SIGPIPE on the main thread before it starts the evaluation
//! thread and [`unblock_on_this_thread`] lifts it there, first thing, so the evaluation thread is
//! the one the kernel picks and the write itself ends the process. Linux raises SIGPIPE on the
//! writing thread already. A mask that already blocked SIGPIPE when the process started is left
//! alone, so that run's writes fail `EPIPE` as jq's do.

#![allow(unsafe_code)] // `signal(2)`, `sigaction(2)`, `pthread_sigmask(3)`, and one initialiser the loader runs before `main`

use core::sync::atomic::{AtomicBool, Ordering};

/// Whether SIGPIPE was ignored when the process started, before Rust's runtime set it to ignored.
static INHERITED_IGNORED: AtomicBool = AtomicBool::new(false);

/// Whether SIGPIPE was blocked in the signal mask the process started with (#7857).
static INHERITED_BLOCKED: AtomicBool = AtomicBool::new(false);

/// Read the disposition and the mask the process inherited (#7732, #7857). Runs before `main`, on
/// the only thread there is, so it does no more than a `sigaction` and a `pthread_sigmask` query and
/// two atomic stores.
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
    INHERITED_BLOCKED.store(blocked_on_this_thread(), Ordering::Relaxed);
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
fn inherited_ignored() -> bool {
    INHERITED_IGNORED.load(Ordering::Relaxed)
}

/// Whether the process was started with SIGPIPE blocked (#7857). `false` where there is no SIGPIPE.
fn inherited_blocked() -> bool {
    INHERITED_BLOCKED.load(Ordering::Relaxed)
}

/// Whether a `succinctly jq` write to a closed pipe fails `EPIPE`, as jq's does, rather than ending
/// the process with SIGPIPE: the process was started with SIGPIPE ignored (#7732) or blocked
/// (#7857), which jq keeps. `false` where there is no SIGPIPE.
pub fn closed_pipe_is_a_failed_write() -> bool {
    inherited_ignored() || inherited_blocked()
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
/// An inherited block is kept too (#7857): nothing here lifts it.
pub fn restore_inherited() {
    if !inherited_ignored() {
        restore_default();
    }
}

/// Whether [`restore_default_on_this_thread`] has run, so [`unblock_after_the_run`] lifts an
/// inherited block on the main thread too.
#[cfg(unix)]
static FORCED_DEFAULT: AtomicBool = AtomicBool::new(false);

/// Put SIGPIPE back to its default action and lift any block of it on the calling thread, so that a
/// write to a closed pipe from this thread ends the process whatever it inherited, as Go does for
/// yq on descriptors 1 and 2 (#7732, #7857). The main thread lifts an inherited block of its own in
/// [`unblock_after_the_run`], before it reports the run's error. A no-op where there is no SIGPIPE.
pub fn restore_default_on_this_thread() {
    restore_default();
    #[cfg(unix)]
    {
        FORCED_DEFAULT.store(true, Ordering::Relaxed);
        unblock_sigpipe_on_this_thread();
    }
}

/// Lift any block of SIGPIPE on the calling thread.
#[cfg(unix)]
fn unblock_sigpipe_on_this_thread() {
    let set = sigpipe_set();
    // SAFETY: `pthread_sigmask` changes only the calling thread's mask; no old mask is asked for.
    unsafe {
        libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, core::ptr::null_mut());
    }
}

/// Whether [`block_on_main_thread`] blocked SIGPIPE, so [`unblock_on_this_thread`] lifts it again.
#[cfg(unix)]
static BLOCKED_ON_MAIN_THREAD: AtomicBool = AtomicBool::new(false);

/// The set holding SIGPIPE alone.
#[cfg(unix)]
fn sigpipe_set() -> libc::sigset_t {
    // SAFETY: `sigemptyset` initialises the set and `sigaddset` adds a valid signal number to it;
    // all-zero bytes are a valid `sigset_t` to start from.
    unsafe {
        let mut set: libc::sigset_t = core::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGPIPE);
        set
    }
}

/// Block SIGPIPE on the calling thread, the main thread before it starts the evaluation thread, so
/// that a write's SIGPIPE is handed to the thread that wrote (#7847, see the module docs). A no-op
/// when the inherited mask already blocks it, and where there is no SIGPIPE (Windows). The thread it
/// starts inherits the mask and calls [`unblock_on_this_thread`] first thing.
pub fn block_on_main_thread() {
    #[cfg(unix)]
    {
        let set = sigpipe_set();
        // SAFETY: `pthread_sigmask` changes only the calling thread's mask and writes the old one
        // into `old`, a valid `sigset_t`; `sigismember` reads it.
        let newly_blocked = unsafe {
            let mut old: libc::sigset_t = core::mem::zeroed();
            libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut old) == 0
                && libc::sigismember(&old, libc::SIGPIPE) == 0
        };
        BLOCKED_ON_MAIN_THREAD.store(newly_blocked, Ordering::Relaxed);
    }
}

/// Lift [`block_on_main_thread`]'s block on the calling thread: the evaluation thread before it runs
/// anything, and the main thread once that thread has ended or could not be started, before it
/// writes. A no-op when nothing was blocked.
pub fn unblock_on_this_thread() {
    #[cfg(unix)]
    if BLOCKED_ON_MAIN_THREAD.load(Ordering::Relaxed) {
        unblock_sigpipe_on_this_thread();
    }
}

/// [`unblock_on_this_thread`] on the main thread once the evaluation thread has ended, and an
/// inherited block lifted as well when the run was yq's ([`restore_default_on_this_thread`]), so a
/// closed standard error ends the process when the main thread reports the run's error (#7857).
pub fn unblock_after_the_run() {
    #[cfg(unix)]
    if FORCED_DEFAULT.load(Ordering::Relaxed) {
        unblock_sigpipe_on_this_thread();
    }
    unblock_on_this_thread();
}

/// Whether SIGPIPE is blocked on the calling thread.
#[cfg(unix)]
fn blocked_on_this_thread() -> bool {
    // SAFETY: a null new set only reads the calling thread's mask into `mask`.
    unsafe {
        let mut mask: libc::sigset_t = core::mem::zeroed();
        libc::pthread_sigmask(libc::SIG_BLOCK, core::ptr::null(), &mut mask);
        libc::sigismember(&mask, libc::SIGPIPE) == 1
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// #7847: the main thread blocks SIGPIPE, the thread it starts inherits the block and lifts it,
    /// and the main thread lifts its own afterwards. A mask that already blocked SIGPIPE stays as it
    /// was on both threads. Each half runs on a thread of its own, which stands in for the main
    /// thread, so the test harness's threads keep their masks; one test, as both halves set the flag.
    #[test]
    fn the_main_thread_blocks_sigpipe_and_the_evaluation_thread_does_not_7847() {
        /// The two threads' masks: the started thread's before and after it lifts the block, then
        /// the starting thread's while it runs and once it has lifted its own.
        fn masks() -> (bool, bool, bool, bool) {
            block_on_main_thread();
            let evaluation = std::thread::spawn(|| {
                let inherited = blocked_on_this_thread();
                unblock_on_this_thread();
                (inherited, blocked_on_this_thread())
            });
            let (inherited, lifted) = evaluation.join().expect("joined");
            let main_during = blocked_on_this_thread();
            unblock_on_this_thread();
            (inherited, lifted, main_during, blocked_on_this_thread())
        }
        let unblocked = std::thread::spawn(|| {
            assert!(!blocked_on_this_thread());
            masks()
        });
        assert_eq!(
            unblocked.join().expect("joined"),
            (true, false, true, false)
        );
        let already_blocked = std::thread::spawn(|| {
            let set = sigpipe_set();
            // SAFETY: as in `block_on_main_thread`, on this test's own thread.
            unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, core::ptr::null_mut()) };
            masks()
        });
        assert_eq!(
            already_blocked.join().expect("joined"),
            (true, true, true, true)
        );
    }
}
