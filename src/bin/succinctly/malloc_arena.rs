//! Every thread allocates from glibc's main arena (#6437).
//!
//! All the work runs on the evaluation thread (`spawn_eval_thread` in `main.rs`), and glibc gives
//! a thread other than the main one a secondary arena. That arena grows its heap by exactly what a
//! request needs, with no `M_TOP_PAD`, so a run that allocates a document's values makes one
//! `mprotect` per 4 KiB page: about 1,500 on a 667 KB file. The main arena grows through `brk` in
//! 128 KiB steps. The evaluation thread is the only one allocating while it runs, so there is
//! nothing for a second arena to keep apart.
//!
//! Only `M_ARENA_MAX`: setting `M_TOP_PAD` as well also turns off glibc's dynamic mmap threshold,
//! and `users_assign_update` at 10 MB was 14% slower with it than with this alone.

#![allow(unsafe_code)] // one `mallopt(3)` call, which takes no pointers

/// Limit the process to one arena. Has to run before the evaluation thread is spawned, since a
/// thread keeps the arena it first allocated from. Reports whether glibc took the setting; other
/// allocators have no such arena and nothing to set.
pub fn use_one() -> bool {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        // SAFETY: `mallopt` takes no pointers and takes the arena's lock itself.
        unsafe { libc::mallopt(libc::M_ARENA_MAX, 1) == 1 }
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_allocator_takes_the_setting_where_it_has_arenas_6437() {
        assert_eq!(
            super::use_one(),
            cfg!(all(target_os = "linux", target_env = "gnu"))
        );
    }
}
