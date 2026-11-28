//! The memory the jq call machine's bound reads (#8064): what the process holds now, and the most
//! it may hold, which `jq_runner::call_machine` installs as the machine's probe
//! (`path_mode::set_call_machine_memory`).
//!
//! **What is read.** On Linux the resident set, the second field of `/proc/self/statm` times the
//! page size, read with `pread` on a descriptor opened once. On macOS the physical footprint
//! (`proc_pid_rusage`'s `ri_phys_footprint`), the figure the kernel's memory-pressure limits act
//! on: unlike the resident size it keeps pages the system has compressed, so a run that has
//! pushed its own memory into the compressor is not read as having shrunk. Elsewhere nothing is
//! read, so no probe is installed and the machine keeps the shipped output bound.
//!
//! **The limit**, computed once per process ([`limit`]): the least of half the physical memory,
//! half the cgroup's `memory.max` (v2, or v1's `memory.limit_in_bytes`), a quarter of the address
//! space a `ulimit -v` cap leaves when the process starts, and [`CEILING`].
//! - Half, because the bound is sampled: a run can pass the limit between two samples, and a leaf
//!   can grow inside one evaluation, which no check point sees. What is left also holds the
//!   refusal and whatever else runs on the machine or in the cgroup.
//! - A quarter under an address-space cap, because the cap counts address space, not memory: a
//!   vector's reserved but untouched capacity, the evaluation thread's stack and the mapped input
//!   count against it and not against the resident set.
//! - [`CEILING`], so that a runaway collector on a large machine is refused at a size jq itself
//!   handles comfortably, rather than after taking half of the machine.

#![allow(unsafe_code)] // `sysconf`, `sysctlbyname` and `proc_pid_rusage`, each writing only into a local

/// The most the call machine lets the process hold however much memory is available: 1.5 GiB.
/// It answers `[range(1e7), range(1e7), range(1e7), range(1e7)] | length`, which jq 1.7.1
/// answers in 1.84 GB, and refuses #8064's runaway collectors within the 1.5 GiB its acceptance
/// asks for.
pub const CEILING: usize = 3 << 29;

/// The bytes the process holds now, or `None` where they cannot be read.
pub fn resident() -> Option<usize> {
    #[cfg(target_os = "linux")]
    {
        linux::resident()
    }
    #[cfg(target_vendor = "apple")]
    {
        apple::footprint()
    }
    #[cfg(not(any(target_os = "linux", target_vendor = "apple")))]
    {
        None
    }
}

/// The most the process may hold while the call machine runs, computed once per process, or
/// `None` where [`resident`] cannot be read.
pub fn limit() -> Option<usize> {
    static LIMIT: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| {
        resident()?;
        Some(limit_from(
            physical(),
            cgroup(),
            crate::cli::address_space::cap().map(|cap| cap.saturating_sub(mapped().unwrap_or(0))),
        ))
    })
}

/// [`limit`] from the physical memory, the cgroup's limit and the address space a cap leaves.
fn limit_from(physical: Option<usize>, cgroup: Option<usize>, room: Option<usize>) -> usize {
    [
        Some(CEILING),
        physical.map(|b| b / 2),
        cgroup.map(|b| b / 2),
        room.map(|b| b / 4),
    ]
    .into_iter()
    .flatten()
    .min()
    .unwrap_or(CEILING)
}

/// The physical memory of the machine.
fn physical() -> Option<usize> {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: `sysconf` takes no pointers.
        let pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) };
        usize::try_from(pages)
            .ok()?
            .checked_mul(linux::page_size()?)
    }
    #[cfg(target_vendor = "apple")]
    {
        let mut bytes: u64 = 0;
        let mut len = core::mem::size_of::<u64>();
        // SAFETY: `hw.memsize` is a `u64`, written into `bytes`, whose size `len` gives.
        let read = unsafe {
            libc::sysctlbyname(
                b"hw.memsize\0".as_ptr().cast(),
                (&mut bytes as *mut u64).cast(),
                &mut len,
                core::ptr::null_mut(),
                0,
            )
        };
        if read != 0 {
            return None;
        }
        usize::try_from(bytes).ok()
    }
    #[cfg(not(any(target_os = "linux", target_vendor = "apple")))]
    {
        None
    }
}

/// The memory limit of the process's cgroup: v2's `memory.max`, else v1's
/// `memory.limit_in_bytes`; `None` when it has none or is not in one.
fn cgroup() -> Option<usize> {
    #[cfg(target_os = "linux")]
    {
        let v2 = std::fs::read_to_string("/proc/self/cgroup")
            .ok()
            .and_then(|groups| cgroup_v2_path(&groups))
            .and_then(|path| {
                std::fs::read_to_string(format!("/sys/fs/cgroup{path}/memory.max")).ok()
            })
            .and_then(|max| cgroup_bytes(&max));
        v2.or_else(|| {
            std::fs::read_to_string("/sys/fs/cgroup/memory/memory.limit_in_bytes")
                .ok()
                .and_then(|max| cgroup_bytes(&max))
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// The v2 cgroup path in a `/proc/self/cgroup` text: its `0::` line.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn cgroup_v2_path(groups: &str) -> Option<&str> {
    groups
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .map(str::trim)
}

/// A cgroup memory limit file's bytes: `None` for `max` and for v1's "no limit" (a page-rounded
/// `i64::MAX`, past any memory).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn cgroup_bytes(text: &str) -> Option<usize> {
    let bytes: u64 = text.trim().parse().ok()?;
    if bytes >= 1 << 60 {
        return None;
    }
    usize::try_from(bytes).ok()
}

/// The address space the process maps, from `/proc/self/status`'s `VmSize`, read once.
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

/// The resident pages of a `/proc/<pid>/statm` text, its second field.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn statm_resident_pages(statm: &[u8]) -> Option<usize> {
    let text = core::str::from_utf8(statm).ok()?;
    text.split_ascii_whitespace().nth(1)?.parse().ok()
}

#[cfg(target_os = "linux")]
mod linux {
    use std::fs::File;
    use std::os::unix::fs::FileExt;
    use std::sync::OnceLock;

    /// The page size, read once.
    pub(super) fn page_size() -> Option<usize> {
        static PAGE: OnceLock<Option<usize>> = OnceLock::new();
        // SAFETY: `sysconf` takes no pointers.
        *PAGE.get_or_init(|| usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok())
    }

    /// The resident set: `/proc/self/statm`'s second field, read with `pread` on a descriptor
    /// opened once, so a sample is one system call.
    pub(super) fn resident() -> Option<usize> {
        static STATM: OnceLock<Option<File>> = OnceLock::new();
        let file = STATM
            .get_or_init(|| File::open("/proc/self/statm").ok())
            .as_ref()?;
        let mut buf = [0u8; 128];
        let n = file.read_at(&mut buf, 0).ok()?;
        super::statm_resident_pages(&buf[..n])?.checked_mul(page_size()?)
    }
}

#[cfg(target_vendor = "apple")]
mod apple {
    /// The physical footprint: `proc_pid_rusage`'s `ri_phys_footprint`, one system call.
    pub(super) fn footprint() -> Option<usize> {
        // SAFETY: `rusage_info_v2` is plain integers, for which all zeros is a value.
        let mut info: libc::rusage_info_v2 = unsafe { core::mem::zeroed() };
        // SAFETY: `RUSAGE_INFO_V2` writes a `rusage_info_v2` into the buffer it is given.
        let read = unsafe {
            libc::proc_pid_rusage(
                libc::getpid(),
                libc::RUSAGE_INFO_V2,
                (&mut info as *mut libc::rusage_info_v2).cast(),
            )
        };
        if read != 0 {
            return None;
        }
        usize::try_from(info.ri_phys_footprint).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limit_is_the_least_share_of_what_is_available_8064() {
        assert_eq!(limit_from(None, None, None), CEILING);
        assert_eq!(limit_from(Some(64 << 30), None, None), CEILING);
        assert_eq!(limit_from(Some(2 << 30), None, None), 1 << 30);
        assert_eq!(limit_from(Some(64 << 30), Some(1 << 30), None), 512 << 20);
        assert_eq!(limit_from(Some(64 << 30), None, Some(1 << 30)), 256 << 20);
    }

    #[test]
    fn the_files_the_limit_reads_parse_8064() {
        assert_eq!(statm_resident_pages(b"12345 678 90 1 0 2 0\n"), Some(678));
        assert_eq!(statm_resident_pages(b"12345"), None);
        assert_eq!(
            cgroup_v2_path("0::/user.slice/x.scope\n"),
            Some("/user.slice/x.scope")
        );
        assert_eq!(cgroup_v2_path("12:memory:/x\n"), None);
        assert_eq!(cgroup_bytes("max\n"), None);
        assert_eq!(cgroup_bytes("1073741824\n"), Some(1 << 30));
        assert_eq!(cgroup_bytes("9223372036854771712\n"), None);
        let status = "Name:\tsuccinctly\nVmSize:\t   12345 kB\nVmRSS:\t 100 kB\n";
        assert_eq!(vm_size(status), Some(12345 * 1024));
        assert_eq!(vm_size("Name:\tx\n"), None);
    }

    /// Where a probe exists, it reads the process: a block it touches is seen.
    #[test]
    fn the_probe_reads_what_the_process_holds_8064() {
        let Some(before) = resident() else {
            assert!(cfg!(not(any(target_os = "linux", target_vendor = "apple"))));
            return;
        };
        let block = vec![1u8; 64 << 20];
        let after = resident().expect("read once, read again");
        assert!(after >= before + (32 << 20), "{before} -> {after}");
        drop(block);
        let limit = limit().expect("a probe, so a limit");
        assert!(limit > 0 && limit <= CEILING, "{limit}");
    }
}
