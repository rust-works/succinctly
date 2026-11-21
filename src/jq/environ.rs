//! The process environment as the C library holds it, entry by entry (#7487).
//!
//! `std::env::vars_os` drops an entry whose name is empty (`=v`, which only a raw `execve` or
//! `Command::env("", "v")` makes) and splits `=X=1` after the `X`, because it looks for the `=`
//! from the second byte on. jq's `env` splits each `environ` string at its first `=`, so `=v` is
//! `"": "v"` and `=X=1` is `"": "X=1"`. Reading `environ` itself gives the strings as they are.
//!
//! Unix only, and the same read `std` does, without its lock: a thread that calls `set_var` while
//! another builds `$ENV` is as unsound here as it is for C's `getenv`. The CLI changes no variable,
//! and a library caller that does has to keep it away from an evaluation, as for `getenv`.

#![allow(unsafe_code)] // reads the `environ` array of NUL-terminated strings

#[cfg(unix)]
use alloc::vec::Vec;

/// Every `NAME=value` string of the environment, in `environ`'s order, as bytes.
#[cfg(unix)]
pub(super) fn entries() -> Vec<Vec<u8>> {
    use core::ffi::{c_char, CStr};

    #[cfg(not(target_vendor = "apple"))]
    extern "C" {
        static environ: *const *const c_char;
    }
    #[cfg(target_vendor = "apple")]
    extern "C" {
        fn _NSGetEnviron() -> *const *const *const c_char;
    }

    // SAFETY: `environ` is null or a null-terminated array of pointers to NUL-terminated strings,
    // which stay valid while no thread changes the environment (see the module comment). On Apple,
    // `_NSGetEnviron` gives the address of that array's pointer, as `environ` is not exported
    // to a library there.
    unsafe {
        #[cfg(not(target_vendor = "apple"))]
        let mut at = environ;
        #[cfg(target_vendor = "apple")]
        let mut at = *_NSGetEnviron();
        let mut entries = Vec::new();
        if at.is_null() {
            return entries;
        }
        while !(*at).is_null() {
            entries.push(CStr::from_ptr(*at).to_bytes().to_vec());
            at = at.add(1);
        }
        entries
    }
}
