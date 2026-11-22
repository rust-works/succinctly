//! The stack of the process's main thread, for a run that cannot create its evaluation thread (#7733).
//!
//! `main` runs everything on a thread with a sized stack (ADR-0025), but at the user's process or
//! thread limit (`ulimit -u`, `prlimit --nproc`, a container's pids limit) `pthread_create` fails
//! `EAGAIN` whatever the size, and every run died with `failed to spawn the evaluation thread`,
//! where jq, which is single-threaded, runs. The run then goes on the main thread, and the
//! native-stack floor has to be registered at the stack that thread really has, not the size the
//! evaluation thread would have had, so that deep recursion is still refused instead of overflowing.
//!
//! On Linux the main thread's stack grows on demand up to the soft `RLIMIT_STACK` (8 MiB by default)
//! read at the time it grows, and the kernel leaves at least 128 MiB below the stack's top free of
//! other mappings whatever that limit was at `exec`. So a soft limit below [`MAIN_STACK_GOAL`] is
//! raised to it (within the hard limit) first, giving the run 64 MiB instead of 8. macOS fixes the
//! main thread's stack at `exec` (8 MiB by default), and it is used as it is.

#![allow(unsafe_code)] // `getrlimit`/`setrlimit` and the `pthread` stack queries, each into a local

/// The main-thread stack the fallback asks for: half of the 128 MiB gap Linux keeps free below the
/// stack's top (`mmap_base`'s `MIN_GAP`), four times the least the evaluation thread settles for.
#[cfg(any(target_os = "linux", target_os = "android"))]
const MAIN_STACK_GOAL: usize = 64 * 1024 * 1024;

/// How many bytes of stack lie below the caller on the main thread, or `None` where this platform
/// cannot tell. Call it on the main thread only.
pub fn available() -> Option<usize> {
    let here = {
        let marker = 0u8;
        core::ptr::addr_of!(marker) as usize
    };
    let lowest = lowest_address()?;
    here.checked_sub(lowest)
}

/// Raise the soft `RLIMIT_STACK` to [`MAIN_STACK_GOAL`] when it is lower and the hard limit allows.
/// An unlimited soft limit is left alone: the kernel then lays the address space out bottom-up and
/// the stack is not the constraint.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn raise_stack_limit() {
    let goal = MAIN_STACK_GOAL as libc::rlim_t;
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` writes only into the `rlimit` it is given.
    if unsafe { libc::getrlimit(libc::RLIMIT_STACK, &mut limit) } != 0 {
        return;
    }
    if limit.rlim_cur == libc::RLIM_INFINITY || limit.rlim_cur >= goal {
        return;
    }
    let raised = libc::rlimit {
        rlim_cur: if limit.rlim_max == libc::RLIM_INFINITY {
            goal
        } else {
            goal.min(limit.rlim_max)
        },
        rlim_max: limit.rlim_max,
    };
    // SAFETY: `setrlimit` reads only the `rlimit` it is given. A refusal leaves the limit as it was.
    let _ = unsafe { libc::setrlimit(libc::RLIMIT_STACK, &raised) };
}

/// The lowest address the main thread's stack may reach.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn lowest_address() -> Option<usize> {
    raise_stack_limit();
    // SAFETY: `pthread_getattr_np` fills the attribute object it is given (for the main thread from
    // the stack mapping and the current `RLIMIT_STACK`), `pthread_attr_getstack` reads it into two
    // locals, and the object is destroyed before returning. All-zero bytes are a valid value of the
    // C struct before it is filled.
    unsafe {
        let mut attr: libc::pthread_attr_t = core::mem::zeroed();
        if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
            return None;
        }
        let mut low: *mut libc::c_void = core::ptr::null_mut();
        let mut size: libc::size_t = 0;
        let read = libc::pthread_attr_getstack(&attr, &mut low, &mut size);
        libc::pthread_attr_destroy(&mut attr);
        (read == 0 && !low.is_null()).then_some(low as usize)
    }
}

/// The lowest address the main thread's stack may reach.
#[cfg(target_vendor = "apple")]
fn lowest_address() -> Option<usize> {
    // SAFETY: both calls only read the calling thread's stack bounds; `pthread_get_stackaddr_np`
    // answers the top of the stack, which grows down from it.
    let (top, size) = unsafe {
        let this = libc::pthread_self();
        (
            libc::pthread_get_stackaddr_np(this) as usize,
            libc::pthread_get_stacksize_np(this),
        )
    };
    top.checked_sub(size)
}

/// Elsewhere the bounds are not read: the run keeps the spawn failure.
#[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
fn lowest_address() -> Option<usize> {
    None
}
