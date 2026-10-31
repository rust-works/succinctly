//! The address space this process may map (#5634).
//!
//! `ulimit -v` (`RLIMIT_AS`) counts a thread's reserved stack, so the evaluation thread's size
//! has to be chosen against it: see `spawn_eval_thread` in `main.rs`.

#![allow(unsafe_code)] // one `getrlimit(2)` call, writing only into a local

/// The cap on the process's address space in bytes, or `None` when it is unlimited, unreadable
/// or (Windows) not a thing the platform has.
pub fn cap() -> Option<usize> {
    #[cfg(unix)]
    {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `getrlimit` writes only into the `rlimit` it is given.
        let read = unsafe { libc::getrlimit(libc::RLIMIT_AS, &mut limit) };
        if read != 0 || limit.rlim_cur == libc::RLIM_INFINITY {
            return None;
        }
        usize::try_from(limit.rlim_cur).ok()
    }
    #[cfg(not(unix))]
    {
        None
    }
}
