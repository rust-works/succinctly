//! FFI to the vendored Oniguruma (`vendor/oniguruma`, the commit jq 1.7.1 bundles), the engine
//! jq-mode regex moves to (#7965, ADR-0031).
//!
//! This is step 2 of the migration: the library is built and reachable, and nothing in the
//! evaluator calls it yet. The adapter (step 3) ports jq's `f_match` on top of [`OnigRegex`].
//!
//! The declarations follow `oniguruma.h` at the pin. jq calls `onig_new` with
//! `ONIG_SYNTAX_PERL_NG`, `ONIG_OPTION_CAPTURE_GROUP` plus the flag letters, and UTF-8; that is
//! the only encoding and syntax used here, so they are the only two of the library's statics
//! declared.
//!
//! Initialisation: `onig_initialize` runs once, under a [`Once`], before the first compile.
//! After it, `onig_new` and `onig_search_with_param` write no global state, so compiles and
//! searches on different threads need no lock (the `onig` crate's process-wide compile mutex
//! covered the lazy initialisation this does up front). The parse depth limit, a global, is set
//! in the same `Once`.

#![allow(unsafe_code)] // STYLE-0004: FFI to the vendored C Oniguruma; each block states what it relies on.
#![allow(dead_code)] // STYLE-0004: the adapter that calls this lands in step 3 of #7965; until then only the tests do.

use core::ffi::{c_int, c_uchar, c_uint, c_ulong, c_void};
use std::sync::Once;

/// `OnigOptionType` (`unsigned int`) and the option bits of `oniguruma.h`.
pub(crate) type OnigOptions = c_uint;
pub(crate) const ONIG_OPTION_NONE: OnigOptions = 0;
pub(crate) const ONIG_OPTION_IGNORECASE: OnigOptions = 1;
pub(crate) const ONIG_OPTION_EXTEND: OnigOptions = 1 << 1;
pub(crate) const ONIG_OPTION_MULTILINE: OnigOptions = 1 << 2;
pub(crate) const ONIG_OPTION_SINGLELINE: OnigOptions = 1 << 3;
pub(crate) const ONIG_OPTION_FIND_LONGEST: OnigOptions = 1 << 4;
pub(crate) const ONIG_OPTION_FIND_NOT_EMPTY: OnigOptions = 1 << 5;
pub(crate) const ONIG_OPTION_CAPTURE_GROUP: OnigOptions = 1 << 8;

const ONIG_NORMAL: c_int = 0;
const ONIG_MISMATCH: c_int = -1;
/// The size of the buffer `onig_error_code_to_str` writes into, as jq declares it.
const ONIG_MAX_ERROR_MESSAGE_LEN: usize = 90;

/// The parse depth limit jq 1.8.1 sets in `main.c` (GHSA-f946-j5j2-4w5m, #7528). Oniguruma's
/// default is 4096, which a deeply nested pattern can turn into a native stack overflow.
pub(crate) const PARSE_DEPTH_LIMIT: c_uint = 1024;

/// An opaque C struct, only ever handled by pointer.
#[repr(C)]
struct Opaque {
    _private: [u8; 0],
}

type OnigRegexPtr = *mut Opaque;

/// `OnigErrorInfo`: the name `onig_error_code_to_str` prints in a message such as
/// `undefined name <n> reference`.
#[repr(C)]
struct OnigErrorInfo {
    enc: *const Opaque,
    par: *mut c_uchar,
    par_end: *mut c_uchar,
}

/// `OnigRegion` (`struct re_registers`).
#[repr(C)]
struct OnigRegion {
    allocated: c_int,
    num_regs: c_int,
    beg: *mut c_int,
    end: *mut c_int,
    history_root: *mut c_void,
}

type NameCallback = extern "C" fn(
    *const c_uchar,
    *const c_uchar,
    c_int,
    *mut c_int,
    OnigRegexPtr,
    *mut c_void,
) -> c_int;

unsafe extern "C" {
    static OnigEncodingUTF8: Opaque;
    static OnigSyntaxPerl_NG: Opaque;

    fn onig_initialize(encodings: *const *const Opaque, n: c_int) -> c_int;
    fn onig_set_parse_depth_limit(depth: c_uint) -> c_int;
    fn onig_version() -> *const core::ffi::c_char;
    fn onig_new(
        reg: *mut OnigRegexPtr,
        pattern: *const c_uchar,
        pattern_end: *const c_uchar,
        option: OnigOptions,
        enc: *const Opaque,
        syntax: *const Opaque,
        einfo: *mut OnigErrorInfo,
    ) -> c_int;
    fn onig_free(reg: OnigRegexPtr);
    fn onig_number_of_captures(reg: OnigRegexPtr) -> c_int;
    fn onig_foreach_name(reg: OnigRegexPtr, func: NameCallback, arg: *mut c_void) -> c_int;
    fn onig_region_new() -> *mut OnigRegion;
    fn onig_region_free(region: *mut OnigRegion, free_self: c_int);
    fn onig_new_match_param() -> *mut Opaque;
    fn onig_free_match_param(mp: *mut Opaque);
    fn onig_initialize_match_param(mp: *mut Opaque) -> c_int;
    fn onig_set_match_stack_limit_size_of_match_param(mp: *mut Opaque, limit: c_uint) -> c_int;
    fn onig_set_retry_limit_in_match_of_match_param(mp: *mut Opaque, limit: c_ulong) -> c_int;
    #[allow(clippy::too_many_arguments)] // STYLE-0004: the C signature.
    fn onig_search_with_param(
        reg: OnigRegexPtr,
        str: *const c_uchar,
        end: *const c_uchar,
        start: *const c_uchar,
        range: *const c_uchar,
        region: *mut OnigRegion,
        option: OnigOptions,
        mp: *mut Opaque,
    ) -> c_int;
    fn onig_error_code_to_str(s: *mut c_uchar, err_code: c_int, ...) -> c_int;
}

static INIT: Once = Once::new();

fn init() {
    INIT.call_once(|| {
        // `OnigEncodingUTF8` is a static the C library defines; only its address is taken.
        let encodings = [core::ptr::addr_of!(OnigEncodingUTF8)];
        // SAFETY: one valid encoding pointer, as the count says. Runs once, before any compile,
        // which is the call order the library requires.
        let r = unsafe { onig_initialize(encodings.as_ptr(), 1) };
        debug_assert_eq!(r, ONIG_NORMAL);
        // SAFETY: sets a global the parser reads; done inside the same `Once`, before any compile.
        let r = unsafe { onig_set_parse_depth_limit(PARSE_DEPTH_LIMIT) };
        debug_assert_eq!(r, ONIG_NORMAL);
    });
}

/// The version the vendored library reports. The pin predates the 6.9.9 release and still
/// says `6.9.8`.
pub(crate) fn version() -> String {
    // SAFETY: returns a pointer to a static, NUL-terminated string.
    let p = unsafe { onig_version() };
    // SAFETY: `p` is non-null and NUL-terminated, and lives as long as the process.
    unsafe { core::ffi::CStr::from_ptr(p) }
        .to_string_lossy()
        .into_owned()
}

/// An Oniguruma error: its code and the message `onig_error_code_to_str` wrote, as bytes (it can
/// cut a pattern's name inside a multibyte character, and jq prints those bytes as they are).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OnigError {
    pub(crate) code: i32,
    pub(crate) message: Vec<u8>,
}

impl OnigError {
    fn from_code(code: c_int, einfo: Option<&mut OnigErrorInfo>) -> Self {
        let mut buf = [0u8; ONIG_MAX_ERROR_MESSAGE_LEN];
        let len = match einfo {
            // SAFETY: `buf` has the size the C function writes at most; `einfo` is the info
            // `onig_new` filled for this code.
            Some(info) => unsafe {
                onig_error_code_to_str(buf.as_mut_ptr(), code, core::ptr::addr_of_mut!(*info))
            },
            // SAFETY: as above; a search error takes no info argument.
            None => unsafe { onig_error_code_to_str(buf.as_mut_ptr(), code) },
        };
        let len = usize::try_from(len).unwrap_or(0).min(buf.len());
        Self {
            code,
            message: buf[..len].to_vec(),
        }
    }

    /// The message, lossily decoded.
    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.message).into_owned()
    }
}

/// A pattern compiled by Oniguruma with UTF-8 and the Perl_NG syntax.
pub(crate) struct OnigRegex {
    raw: OnigRegexPtr,
}

impl Drop for OnigRegex {
    fn drop(&mut self) {
        // SAFETY: `raw` came from a successful `onig_new` and is freed once, here.
        unsafe { onig_free(self.raw) }
    }
}

/// One search's capture spans as byte offsets into the subject: entry 0 is the whole match, and
/// a group that took no part is `None`.
pub(crate) type Spans = Vec<Option<(usize, usize)>>;

impl OnigRegex {
    /// `onig_new` on `pattern` with `options`, UTF-8 and Perl_NG, as jq's `f_match` calls it.
    pub(crate) fn new(pattern: &str, options: OnigOptions) -> Result<Self, OnigError> {
        init();
        let bytes = pattern.as_bytes();
        let mut raw: OnigRegexPtr = core::ptr::null_mut();
        let mut einfo = OnigErrorInfo {
            enc: core::ptr::null(),
            par: core::ptr::null_mut(),
            par_end: core::ptr::null_mut(),
        };
        // SAFETY: the pattern pointers delimit `bytes`; the encoding and syntax are the library's
        // own statics; `raw` and `einfo` are valid for writes; `init` has run.
        let r = unsafe {
            onig_new(
                &raw mut raw,
                bytes.as_ptr(),
                bytes.as_ptr().add(bytes.len()),
                options,
                core::ptr::addr_of!(OnigEncodingUTF8),
                core::ptr::addr_of!(OnigSyntaxPerl_NG),
                &raw mut einfo,
            )
        };
        if r != ONIG_NORMAL {
            return Err(OnigError::from_code(r, Some(&mut einfo)));
        }
        Ok(Self { raw })
    }

    /// The number of capture groups, not counting the whole match.
    pub(crate) fn captures_len(&self) -> usize {
        // SAFETY: `raw` is a live compiled pattern.
        usize::try_from(unsafe { onig_number_of_captures(self.raw) }).unwrap_or(0)
    }

    /// Each group name with the group numbers it names, in the library's order.
    pub(crate) fn names(&self) -> Vec<(String, Vec<usize>)> {
        extern "C" fn each(
            name: *const c_uchar,
            name_end: *const c_uchar,
            ngroups: c_int,
            groups: *mut c_int,
            _reg: OnigRegexPtr,
            arg: *mut c_void,
        ) -> c_int {
            // SAFETY: `arg` is the `Vec` `names` passed below, alive for the whole iteration.
            let out = unsafe { &mut *arg.cast::<Vec<(String, Vec<usize>)>>() };
            // SAFETY: the library passes the name as a byte range of the compiled pattern.
            let len = usize::try_from(unsafe { name_end.offset_from(name) }).unwrap_or(0);
            // SAFETY: `name..name + len` is that range.
            let bytes = unsafe { core::slice::from_raw_parts(name, len) };
            let n = usize::try_from(ngroups).unwrap_or(0);
            // SAFETY: `groups` points at `ngroups` group numbers.
            let nums = unsafe { core::slice::from_raw_parts(groups, n) };
            out.push((
                String::from_utf8_lossy(bytes).into_owned(),
                nums.iter()
                    .filter_map(|&g| usize::try_from(g).ok())
                    .collect(),
            ));
            0
        }
        let mut out: Vec<(String, Vec<usize>)> = Vec::new();
        // SAFETY: `raw` is live; the callback only reads what the library passes and writes `out`,
        // which outlives the call.
        unsafe { onig_foreach_name(self.raw, each, (&raw mut out).cast()) };
        out
    }

    /// `onig_search_with_param` forward from byte `start` to the end of `subject`, with the
    /// library's default limits (jq sets none). `Ok(None)` is no match.
    pub(crate) fn search(&self, subject: &[u8], start: usize) -> Result<Option<Spans>, OnigError> {
        let mut scratch = Scratch::new();
        if !self.search_in(&mut scratch, subject, start)? {
            return Ok(None);
        }
        Ok(Some((0..scratch.len()).map(|i| scratch.span(i)).collect()))
    }

    /// [`search`](Self::search) into `scratch`, which a loop of searches reuses as jq's `f_match`
    /// reuses its region: `true` when it matched, and the spans are then `scratch`'s.
    pub(crate) fn search_in(
        &self,
        scratch: &mut Scratch,
        subject: &[u8],
        start: usize,
    ) -> Result<bool, OnigError> {
        let region = scratch.region.0;
        self.search_raw(scratch, region, subject, start)
    }

    /// Whether a search from byte `start` finds a match, with no region: jq's test mode, which
    /// asks the library for no spans (its `f_match` stops at the first match).
    pub(crate) fn matches_in(
        &self,
        scratch: &mut Scratch,
        subject: &[u8],
        start: usize,
    ) -> Result<bool, OnigError> {
        self.search_raw(scratch, core::ptr::null_mut(), subject, start)
    }

    fn search_raw(
        &self,
        scratch: &mut Scratch,
        region: *mut OnigRegion,
        subject: &[u8],
        start: usize,
    ) -> Result<bool, OnigError> {
        if start > subject.len() {
            return Ok(false);
        }
        let base = subject.as_ptr();
        // SAFETY: every pointer is within `subject` (`start <= len`); the region (when not null)
        // and the match parameter are live allocations of the library's own types, and the
        // library resizes the region to this pattern's groups.
        let r = unsafe {
            let end = base.add(subject.len());
            onig_search_with_param(
                self.raw,
                base,
                end,
                base.add(start),
                end,
                region,
                ONIG_OPTION_NONE,
                scratch.param.0,
            )
        };
        if r == ONIG_MISMATCH {
            return Ok(false);
        }
        if r < 0 {
            return Err(OnigError::from_code(r, None));
        }
        Ok(true)
    }
}

/// The region and match parameter of a search, kept for the next one ([`OnigRegex::search_in`]).
pub(crate) struct Scratch {
    region: Region,
    param: MatchParam,
}

impl Scratch {
    pub(crate) fn new() -> Self {
        Self {
            region: Region::new(),
            param: MatchParam::new(),
        }
    }

    /// The number of spans the last match holds: the whole match and each group.
    pub(crate) fn len(&self) -> usize {
        // SAFETY: the region is live; `num_regs` is its own count.
        usize::try_from(unsafe { (*self.region.0).num_regs }).unwrap_or(0)
    }

    /// Span `i` of the last match as byte offsets: entry 0 is the whole match, and a group that
    /// took no part (or a number past the last) is `None`.
    pub(crate) fn span(&self, i: usize) -> Option<(usize, usize)> {
        if i >= self.len() {
            return None;
        }
        // SAFETY: after a match the region holds `num_regs` begin and end offsets, and `i` is
        // below that count.
        let (b, e) = unsafe {
            let reg = &*self.region.0;
            (*reg.beg.add(i), *reg.end.add(i))
        };
        Some((usize::try_from(b).ok()?, usize::try_from(e).ok()?))
    }
}

/// An `OnigRegion`, freed on drop.
struct Region(*mut OnigRegion);

impl Region {
    fn new() -> Self {
        // SAFETY: allocates an empty region; checked for null below.
        let p = unsafe { onig_region_new() };
        assert!(!p.is_null(), "onig_region_new: out of memory");
        Self(p)
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from `onig_region_new` and is freed once, with the struct itself.
        unsafe { onig_region_free(self.0, 1) }
    }
}

/// An `OnigMatchParam` holding the library's default limits, freed on drop.
struct MatchParam(*mut Opaque);

impl MatchParam {
    fn new() -> Self {
        // SAFETY: allocates a match parameter; checked for null below.
        let p = unsafe { onig_new_match_param() };
        assert!(!p.is_null(), "onig_new_match_param: out of memory");
        // SAFETY: `p` is a fresh match parameter; this copies the global defaults into it.
        unsafe { onig_initialize_match_param(p) };
        Self(p)
    }

    /// The match-stack limit of this search, in stack entries (0 is unlimited).
    fn match_stack_limit(&self, entries: c_uint) {
        // SAFETY: `self.0` is a live match parameter.
        unsafe { onig_set_match_stack_limit_size_of_match_param(self.0, entries) };
    }

    /// The retry limit of this search (0 is unlimited).
    fn retry_limit(&self, retries: c_ulong) {
        // SAFETY: `self.0` is a live match parameter.
        unsafe { onig_set_retry_limit_in_match_of_match_param(self.0, retries) };
    }
}

impl Drop for MatchParam {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from `onig_new_match_param` and is freed once, here.
        unsafe { onig_free_match_param(self.0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(p: &str) -> OnigRegex {
        OnigRegex::new(p, ONIG_OPTION_CAPTURE_GROUP).unwrap_or_else(|e| panic!("{p}: {}", e.text()))
    }

    fn refusal(p: &str) -> String {
        match OnigRegex::new(p, ONIG_OPTION_CAPTURE_GROUP) {
            Ok(_) => panic!("{p} compiled"),
            Err(e) => e.text(),
        }
    }

    #[test]
    fn the_vendored_library_is_the_pin_jq_1_7_1_bundles() {
        // The pin is 18 commits before the 6.9.9 release and has not bumped the version yet.
        assert_eq!(version(), "6.9.8");
        // 6.9.10 added `ONIG_SYN_ALLOW_CHAR_TYPE_FOLLOWED_BY_MINUS_IN_CC` to Perl_NG (upstream
        // #298); jq 1.7.1 and the pin refuse a class type followed by `-`.
        assert_eq!(
            refusal("[\\d-a]"),
            "unmatched range specifier in char-class"
        );
    }

    #[test]
    fn a_search_reports_byte_spans_and_groups_that_took_no_part() {
        let re = compile("a(b)?(c)");
        assert_eq!(re.captures_len(), 2);
        assert_eq!(
            re.search(b"xac", 0).unwrap(),
            Some(vec![Some((1, 3)), None, Some((2, 3))])
        );
        assert_eq!(
            re.search(b"xabc", 0).unwrap(),
            Some(vec![Some((1, 4)), Some((2, 3)), Some((3, 4))])
        );
        assert_eq!(re.search(b"xabc", 2).unwrap(), None);
        assert_eq!(re.search(b"xabc", 9).unwrap(), None);
        let multibyte = compile("é");
        assert_eq!(
            multibyte.search("aé".as_bytes(), 0).unwrap(),
            Some(vec![Some((1, 3))])
        );
    }

    #[test]
    fn a_refusal_carries_oniguruma_s_words_and_the_name_it_names() {
        assert_eq!(refusal("("), "end pattern with unmatched parenthesis");
        // Each as jq 1.7.1 (`/usr/bin/jq`) words it.
        assert_eq!(refusal("*"), "target of repeat operator is not specified");
        assert_eq!(
            refusal("a{2,1}"),
            "upper is smaller than lower in repeat range"
        );
        assert_eq!(refusal("[b-a]"), "empty range in char class");
        assert_eq!(refusal("\\k<nope>"), "undefined name <nope> reference");
    }

    #[test]
    fn group_names_map_to_their_numbers() {
        let re = compile("(?<x>a)(?<y>b)(?<x>c)");
        let mut names = re.names();
        names.sort();
        assert_eq!(
            names,
            vec![("x".to_string(), vec![1, 3]), ("y".to_string(), vec![2])]
        );
    }

    #[test]
    fn the_parse_depth_limit_is_jq_1_8_1_s() {
        let nested = |depth: usize| format!("{}a{}", "(".repeat(depth), ")".repeat(depth));
        assert!(OnigRegex::new(&nested(200), ONIG_OPTION_CAPTURE_GROUP).is_ok());
        assert_eq!(refusal(&nested(2000)), "parse depth limit over");
    }

    #[test]
    fn compiles_and_searches_run_on_several_threads_at_once() {
        let threads: Vec<_> = (0..8)
            .map(|i| {
                std::thread::spawn(move || {
                    for _ in 0..200 {
                        let re = compile(&format!("(?<n>a{{{i}}})b"));
                        let subject = format!("{}b", "a".repeat(i));
                        assert_eq!(
                            re.search(subject.as_bytes(), 0).unwrap().map(|s| s[0]),
                            Some(Some((0, i + 1)))
                        );
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
    }

    #[test]
    fn a_match_parameter_takes_per_search_limits() {
        let p = MatchParam::new();
        p.match_stack_limit(1000);
        p.retry_limit(1000);
    }
}
