//! Give back the pages of the executable's relocation tables once the loader has applied them (#7412).
//!
//! The release binary is a position-independent executable, so before `main` runs the dynamic
//! loader applies every relocation in `.rela.dyn`: 18,132 entries, 429 KiB, on x86_64 Linux. It
//! reads the table once, front to back, and nothing reads it again. Every page it read stays mapped
//! for the rest of the run, though, and is counted in the process's RSS: 0.42 MB of the 4-5 MB a
//! `succinctly jq` run with a small input peaks at, where jq 1.7.1's whole binary is 0.4 MB.
//!
//! [`release`] unmaps those pages with `MADV_DONTNEED`. They are clean pages of a private file
//! mapping that is not writable, which neither the loader nor anything else wrote, so the page
//! cache still holds the same bytes and a later read (there is none in a normal run; a backtrace
//! does not read relocations) faults them back in unchanged. Only whole pages of the tables inside
//! a loadable segment without write permission, outside RELRO, are touched: the relocated data
//! itself (`.data.rel.ro`, `.got`) is in the writable segment and must never be, since dropping it
//! would bring back the file's unrelocated bytes. An executable with text relocations is left
//! alone, since the loader wrote its unwritable pages.
//!
//! It runs once, first thing in `main`, before the evaluation thread exists: unmapping in a
//! process with one thread needs no TLB shootdown on other cores.

#![allow(unsafe_code)] // reads the program headers the kernel passed in, and one `madvise(2)`

/// Unmap the whole pages of the executable's relocation tables. Returns the bytes unmapped: 0 off
/// Linux, and when there is nothing to unmap.
pub fn release() -> usize {
    #[cfg(target_os = "linux")]
    {
        imp::release()
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

#[cfg(target_os = "linux")]
pub(crate) mod imp {
    #[cfg(target_pointer_width = "32")]
    use libc::Elf32_Phdr as Phdr;
    #[cfg(target_pointer_width = "64")]
    use libc::Elf64_Phdr as Phdr;

    /// `ElfN_Dyn`: a tag and a value, each of the pointer's width.
    #[repr(C)]
    struct Dyn {
        tag: isize,
        val: usize,
    }

    const DT_NULL: isize = 0;
    const DT_PLTRELSZ: isize = 2;
    const DT_RELA: isize = 7;
    const DT_RELASZ: isize = 8;
    const DT_REL: isize = 17;
    const DT_RELSZ: isize = 18;
    const DT_TEXTREL: isize = 22;
    const DT_JMPREL: isize = 23;
    const DT_FLAGS: isize = 30;
    const DT_RELRSZ: isize = 35;
    const DT_RELR: isize = 36;
    const DF_TEXTREL: usize = 4;

    /// The page-aligned `(start, end)` ranges [`release`] unmaps: the relocation tables the dynamic
    /// section names, merged where they touch, cut to the whole pages each lies on, each inside a
    /// loadable segment that is not writable (lld gives the tables a read-only segment of their own;
    /// GNU ld on aarch64 puts them in the one that also holds `.text`) and outside RELRO. Empty when the executable has no program
    /// headers to read, no dynamic section, or text relocations.
    pub(crate) fn ranges() -> Vec<(usize, usize)> {
        // SAFETY: `getauxval` only reads the auxiliary vector the kernel built for the process.
        let (at, count, entry, page) = unsafe {
            (
                libc::getauxval(libc::AT_PHDR) as usize,
                libc::getauxval(libc::AT_PHNUM) as usize,
                libc::getauxval(libc::AT_PHENT) as usize,
                libc::getauxval(libc::AT_PAGESZ) as usize,
            )
        };
        if at == 0 || count == 0 || entry != std::mem::size_of::<Phdr>() || !page.is_power_of_two()
        {
            return Vec::new();
        }
        // SAFETY: the kernel maps the executable's program headers at `AT_PHDR`, `AT_PHNUM` entries
        // of `AT_PHENT` bytes (checked above), and they stay mapped for the life of the process.
        let phdrs = unsafe { std::slice::from_raw_parts(at as *const Phdr, count) };
        let Some(own) = phdrs.iter().find(|p| p.p_type == libc::PT_PHDR) else {
            return Vec::new();
        };
        let bias = at.wrapping_sub(own.p_vaddr as usize);
        let Some(dynamic) = phdrs.iter().find(|p| p.p_type == libc::PT_DYNAMIC) else {
            return Vec::new();
        };
        // SAFETY: `PT_DYNAMIC` is inside a loaded segment; it holds `p_memsz` bytes of entries and
        // ends with `DT_NULL`, and the loader has finished writing it before `main`.
        let entries = unsafe {
            std::slice::from_raw_parts(
                bias.wrapping_add(dynamic.p_vaddr as usize) as *const Dyn,
                dynamic.p_memsz as usize / std::mem::size_of::<Dyn>(),
            )
        };
        let mut tables = [(0usize, 0usize); 4]; // RELA, REL, JMPREL, RELR: (address, size)
        for d in entries.iter().take_while(|d| d.tag != DT_NULL) {
            match d.tag {
                DT_TEXTREL => return Vec::new(),
                DT_FLAGS if d.val & DF_TEXTREL != 0 => return Vec::new(),
                DT_RELA => tables[0].0 = d.val,
                DT_RELASZ => tables[0].1 = d.val,
                DT_REL => tables[1].0 = d.val,
                DT_RELSZ => tables[1].1 = d.val,
                DT_JMPREL => tables[2].0 = d.val,
                DT_PLTRELSZ => tables[2].1 = d.val,
                DT_RELR => tables[3].0 = d.val,
                DT_RELRSZ => tables[3].1 = d.val,
                _ => {}
            }
        }
        // glibc rewrites these entries to run-time addresses as it loads a PIE; musl leaves the
        // link-time ones. A PIE is loaded far above its own size, so the two cannot be confused,
        // and a guess that came out wrong would fail the segment check below.
        let mut spans: Vec<(usize, usize)> = tables
            .iter()
            .filter(|t| t.0 != 0 && t.1 != 0)
            .map(|&(a, n)| {
                let a = if a >= bias { a } else { a.wrapping_add(bias) };
                (a, a.saturating_add(n))
            })
            .collect();
        // At most four: an insertion sort written out, since `sort_unstable` would instantiate a
        // generic sort outside the linked start-up block and keep a page of it resident.
        for i in 1..spans.len() {
            let mut j = i;
            while j > 0 && spans[j - 1] > spans[j] {
                spans.swap(j - 1, j);
                j -= 1;
            }
        }
        let mut merged: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
        for (a, b) in spans {
            match merged.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        let segment = |p: &Phdr| {
            let lo = bias.wrapping_add(p.p_vaddr as usize);
            (lo, lo.saturating_add(p.p_filesz as usize))
        };
        let unwritable: Vec<(usize, usize)> = phdrs
            .iter()
            .filter(|p| p.p_type == libc::PT_LOAD && p.p_flags & libc::PF_W == 0)
            .map(segment)
            .collect();
        let relro: Vec<(usize, usize)> = phdrs
            .iter()
            .filter(|p| p.p_type == libc::PT_GNU_RELRO)
            .map(|p| {
                let (lo, _) = segment(p);
                (lo, lo.saturating_add(p.p_memsz as usize))
            })
            .collect();
        merged
            .into_iter()
            .filter(|&(a, b)| unwritable.iter().any(|&(lo, hi)| lo <= a && b <= hi))
            .filter(|&(a, b)| relro.iter().all(|&(lo, hi)| b <= lo || hi <= a))
            .map(|(a, b)| (a.next_multiple_of(page), b & !(page - 1)))
            .filter(|&(a, b)| a < b)
            .collect()
    }

    pub(super) fn release() -> usize {
        let mut released = 0;
        for (a, b) in ranges() {
            // SAFETY: `[a, b)` is whole pages of an unwritable, file-backed private mapping of the
            // executable that nothing wrote (no text relocations, outside RELRO), and no Rust
            // reference points into a relocation table. `MADV_DONTNEED` drops the mappings; a read
            // faults the file's own bytes back in.
            if unsafe { libc::madvise(a as *mut libc::c_void, b - a, libc::MADV_DONTNEED) } == 0 {
                released += b - a;
            }
        }
        released
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::io::{Read, Seek, SeekFrom};

    /// Whether the page at `addr` is mapped in this process (`/proc/self/pagemap` bit 63), or
    /// `None` where pagemap cannot be read.
    fn present(addr: usize, page: usize) -> Option<bool> {
        let mut f = std::fs::File::open("/proc/self/pagemap").ok()?;
        f.seek(SeekFrom::Start((addr / page * 8) as u64)).ok()?;
        let mut e = [0u8; 8];
        f.read_exact(&mut e).ok()?;
        Some(u64::from_le_bytes(e) >> 63 == 1)
    }

    #[test]
    fn the_relocation_table_is_unmapped_and_reads_back_unchanged_7412() {
        // The test binary is a PIE with a relocation table of its own (Rust's default on Linux).
        let ranges = super::imp::ranges();
        assert!(
            !ranges.is_empty(),
            "no relocation table found:\n{}",
            std::fs::read_to_string("/proc/self/maps").unwrap_or_default()
        );
        // SAFETY: `getauxval` only reads the auxiliary vector.
        let page = unsafe { libc::getauxval(libc::AT_PAGESZ) } as usize;
        let (a, b) = ranges[0];
        assert!(a % page == 0 && b % page == 0 && a < b);
        // SAFETY: `[a, b)` is mapped and readable: an unwritable segment of this executable.
        let before: Vec<u8> = unsafe { std::slice::from_raw_parts(a as *const u8, b - a) }.to_vec();
        assert!(present(a, page).unwrap_or(true));
        assert!(super::release() >= b - a);
        if let Some(mapped) = present(a, page) {
            assert!(!mapped, "the first page of the table is still mapped");
        }
        // SAFETY: as above; the read faults the pages back in from the file.
        let after = unsafe { std::slice::from_raw_parts(a as *const u8, b - a) };
        assert_eq!(before, after);
    }

    #[test]
    fn nothing_writable_is_ever_in_a_range_7412() {
        let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
        for (a, b) in super::imp::ranges() {
            let line = maps
                .lines()
                .find(|l| {
                    let (lo, hi) = l.split_once(' ').unwrap().0.split_once('-').unwrap();
                    let (lo, hi) = (
                        usize::from_str_radix(lo, 16).unwrap(),
                        usize::from_str_radix(hi, 16).unwrap(),
                    );
                    lo <= a && b <= hi
                })
                .expect("a range outside any one mapping");
            let perms = line.split_whitespace().nth(1).unwrap();
            assert!(
                perms.starts_with('r') && &perms[1..2] == "-" && perms.ends_with('p'),
                "{line}"
            );
        }
    }
}
