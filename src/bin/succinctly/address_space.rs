//! The address space this process may map (#5634).
//!
//! `ulimit -v` (`RLIMIT_AS`) counts a thread's reserved stack, so the evaluation thread's size
//! has to be chosen against it: see `spawn_eval_thread` in `main.rs`. Read where the kernel
//! enforces the cap; macOS does not enforce `RLIMIT_AS`.

#![allow(unsafe_code)] // one `getrlimit(2)` call, writing only into a local

/// The cap on the process's address space in bytes, or `None` when it is unlimited, unreadable
/// or (macOS, Windows) not enforced by the platform.
pub fn cap() -> Option<usize> {
    #[cfg(all(unix, not(target_vendor = "apple")))]
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
    #[cfg(not(all(unix, not(target_vendor = "apple"))))]
    {
        None
    }
}

/// The address space the cap leaves unmapped now, or `None` when there is no cap (#8064): the
/// jq call machine holds its heap budget to half of it, so that it refuses while there is room
/// to make the refusal. The cap is read once; what is mapped is read on each call, which happens
/// only under a cap. When the mapping cannot be read, half the cap is taken as left.
pub fn room() -> Option<usize> {
    static CAP: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    let cap = (*CAP.get_or_init(cap))?;
    Some(mapped().map_or(cap / 2, |mapped| cap.saturating_sub(mapped)))
}

/// The bytes of address space the process maps, from `/proc/self/status`'s `VmSize`.
fn mapped() -> Option<usize> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    vm_size(&status)
}

/// The `VmSize` of a `/proc/<pid>/status` text, in bytes.
fn vm_size(status: &str) -> Option<usize> {
    let line = status.lines().find(|line| line.starts_with("VmSize:"))?;
    let kb: usize = line
        .trim_start_matches("VmSize:")
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    kb.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mapped_size_is_read_from_vm_size_8064() {
        let status =
            "Name:\tsuccinctly\nVmPeak:\t  20000 kB\nVmSize:\t   12345 kB\nVmRSS:\t 100 kB\n";
        assert_eq!(vm_size(status), Some(12345 * 1024));
        assert_eq!(vm_size("Name:\tx\n"), None);
        assert_eq!(vm_size("VmSize:\tlots kB\n"), None);
    }

    #[test]
    fn there_is_room_to_report_only_under_a_cap_8064() {
        assert_eq!(room().is_some(), cap().is_some());
    }
}
