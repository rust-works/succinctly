//! The shim's exports (`src/shim.c`), once per vendored release.
//!
//! build.rs links two builds of the shim, `sjq171_*` over jq 1.7.1 and `sjq182_*` over jq 1.8.2.
//! [`Api`] holds one build's functions, so the readers in [`crate::reader`] take a release, not a
//! symbol name.

use std::ffi::{c_char, c_int};

/// `struct sjq_input` (shim.c): jq's `jq_util_input_state` and the captured open-failure text.
#[repr(C)]
pub struct SjqInput {
    _opaque: [u8; 0],
}

/// jq's `struct jv_parser`.
#[repr(C)]
pub struct JvParser {
    _opaque: [u8; 0],
}

/// One release's shim functions. Every pointer is an `extern "C"` function of that build; the
/// contracts are those of shim.c.
pub struct Api {
    pub free_buf: unsafe extern "C" fn(*mut c_char),
    pub input_new: unsafe extern "C" fn(c_int, c_int, c_int) -> *mut SjqInput,
    pub input_add: unsafe extern "C" fn(*mut SjqInput, *const c_char),
    pub input_next: unsafe extern "C" fn(*mut SjqInput, *mut *mut c_char, *mut usize) -> c_int,
    pub input_errors: unsafe extern "C" fn(*mut SjqInput) -> c_int,
    pub input_take_errs: unsafe extern "C" fn(*mut SjqInput, *mut usize) -> *mut c_char,
    pub input_filename: unsafe extern "C" fn(*mut SjqInput, *mut *mut c_char, *mut usize) -> c_int,
    pub input_line: unsafe extern "C" fn(*mut SjqInput) -> f64,
    pub input_free: unsafe extern "C" fn(*mut SjqInput),
    pub parser_new: unsafe extern "C" fn(c_int) -> *mut JvParser,
    pub parser_set_buf: unsafe extern "C" fn(*mut JvParser, *const c_char, c_int, c_int),
    pub parser_remaining: unsafe extern "C" fn(*mut JvParser) -> c_int,
    pub parser_next: unsafe extern "C" fn(*mut JvParser, *mut *mut c_char, *mut usize) -> c_int,
    pub parser_free: unsafe extern "C" fn(*mut JvParser),
    pub json_string:
        unsafe extern "C" fn(*const c_char, usize, *mut *mut c_char, *mut usize) -> c_int,
    pub utf8_missing: unsafe extern "C" fn(*const c_char, *const c_char) -> c_int,
    pub parser_sync_point: unsafe extern "C" fn(*mut JvParser, *mut c_int) -> c_int,
    pub parser_seed: unsafe extern "C" fn(*mut JvParser, *const c_int),
}

macro_rules! release_api {
    ($module:ident, $api:ident, $prefix:literal) => {
        mod $module {
            use super::{JvParser, SjqInput};
            use std::ffi::{c_char, c_int};
            extern "C" {
                #[link_name = concat!($prefix, "free_buf")]
                pub fn free_buf(p: *mut c_char);
                #[link_name = concat!($prefix, "input_new")]
                pub fn input_new(flags: c_int, raw: c_int, slurp: c_int) -> *mut SjqInput;
                #[link_name = concat!($prefix, "input_add")]
                pub fn input_add(i: *mut SjqInput, fname: *const c_char);
                #[link_name = concat!($prefix, "input_next")]
                pub fn input_next(
                    i: *mut SjqInput,
                    out: *mut *mut c_char,
                    len: *mut usize,
                ) -> c_int;
                #[link_name = concat!($prefix, "input_errors")]
                pub fn input_errors(i: *mut SjqInput) -> c_int;
                #[link_name = concat!($prefix, "input_take_errs")]
                pub fn input_take_errs(i: *mut SjqInput, len: *mut usize) -> *mut c_char;
                #[link_name = concat!($prefix, "input_filename")]
                pub fn input_filename(
                    i: *mut SjqInput,
                    out: *mut *mut c_char,
                    len: *mut usize,
                ) -> c_int;
                #[link_name = concat!($prefix, "input_line")]
                pub fn input_line(i: *mut SjqInput) -> f64;
                #[link_name = concat!($prefix, "input_free")]
                pub fn input_free(i: *mut SjqInput);
                #[link_name = concat!($prefix, "parser_new")]
                pub fn parser_new(flags: c_int) -> *mut JvParser;
                #[link_name = concat!($prefix, "parser_set_buf")]
                pub fn parser_set_buf(
                    p: *mut JvParser,
                    buf: *const c_char,
                    len: c_int,
                    partial: c_int,
                );
                #[link_name = concat!($prefix, "parser_remaining")]
                pub fn parser_remaining(p: *mut JvParser) -> c_int;
                #[link_name = concat!($prefix, "parser_next")]
                pub fn parser_next(
                    p: *mut JvParser,
                    out: *mut *mut c_char,
                    len: *mut usize,
                ) -> c_int;
                #[link_name = concat!($prefix, "parser_free")]
                pub fn parser_free(p: *mut JvParser);
                #[link_name = concat!($prefix, "json_string")]
                pub fn json_string(
                    s: *const c_char,
                    len: usize,
                    out: *mut *mut c_char,
                    out_len: *mut usize,
                ) -> c_int;
                #[link_name = concat!($prefix, "utf8_missing")]
                pub fn utf8_missing(last: *const c_char, min: *const c_char) -> c_int;
                #[link_name = concat!($prefix, "parser_sync_point")]
                pub fn parser_sync_point(p: *mut JvParser, out5: *mut c_int) -> c_int;
                #[link_name = concat!($prefix, "parser_seed")]
                pub fn parser_seed(p: *mut JvParser, in5: *const c_int);
            }
        }

        pub static $api: Api = Api {
            free_buf: $module::free_buf,
            input_new: $module::input_new,
            input_add: $module::input_add,
            input_next: $module::input_next,
            input_errors: $module::input_errors,
            input_take_errs: $module::input_take_errs,
            input_filename: $module::input_filename,
            input_line: $module::input_line,
            input_free: $module::input_free,
            parser_new: $module::parser_new,
            parser_set_buf: $module::parser_set_buf,
            parser_remaining: $module::parser_remaining,
            parser_next: $module::parser_next,
            parser_free: $module::parser_free,
            json_string: $module::json_string,
            utf8_missing: $module::utf8_missing,
            parser_sync_point: $module::parser_sync_point,
            parser_seed: $module::parser_seed,
        };
    };
}

release_api!(jq171, JQ_171, "sjq171_");
release_api!(jq182, JQ_182, "sjq182_");
