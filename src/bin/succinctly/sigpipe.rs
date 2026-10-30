//! Die of SIGPIPE like jq and yq do (#5547).
//!
//! Rust starts every process with SIGPIPE ignored, so a write to a closed pipe comes back as
//! `EPIPE` and the program has to report it: `succinctly jq -n 'range(10000000)' | head -n1`
//! printed `failed to flush output: Broken pipe` and exited 1. jq (C) and yq (Go) leave the
//! default disposition, so the same pipeline ends their process with SIGPIPE: no message,
//! status 141 under a shell, which is what a reader that stops early expects.
//!
//! Restored for the two query subcommands only. The benchmark orchestrator and the generators
//! keep the ignored disposition they handle `EPIPE` under.

#![allow(unsafe_code)] // one `signal(2)` call, no state shared with Rust

/// Put SIGPIPE back to its default action. A no-op where there is no SIGPIPE (Windows).
pub fn restore_default() {
    #[cfg(unix)]
    {
        // SAFETY: `signal` with `SIG_DFL` installs no handler and reads no Rust data; it is
        // called once, at the top of a subcommand, before any thread of ours is started.
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        }
    }
}
