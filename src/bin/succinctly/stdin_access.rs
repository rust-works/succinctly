//! Whether standard input can be read at all (#7126).
//!
//! Rust's `std::io::stdin()` reads a descriptor that is closed or was not opened for reading
//! (`jq . 0<&-`, `jq . 0>>file`, the write end of a pipe) as an empty stream: it turns `EBADF` into
//! end of input. jq reads such a stream as empty too but counts the failure and exits 2 once the run
//! is over, so a caller has to be told before the read is made.

#![allow(unsafe_code)] // one `fcntl(2)` call that only reads the descriptor's flags

/// Whether descriptor 0 is closed or not open for reading, so that a read of it fails with `EBADF`.
/// Always `false` where there is no descriptor 0 to ask (Windows).
pub fn unreadable() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: `F_GETFL` reads the status flags of a descriptor and touches nothing else; a
        // descriptor that is not open answers `-1` with `EBADF`.
        let flags = unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_GETFL) };
        flags == -1 || flags & libc::O_ACCMODE == libc::O_WRONLY
    }
    #[cfg(not(unix))]
    {
        false
    }
}
