//! Every reported error position must be reconstructible from its offset.
//!
//! `Position` is public API and drives the CLI's rendered diagnostics (the
//! caret column in `json validate` output). The property test samples this;
//! the fuzzer searches for the input that breaks it — most likely somewhere in
//! `validate_keyword`, the one place that builds a `Position` by hand rather
//! than calling `position()`.
#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../tests/common/json_oracle.rs"]
mod oracle;

fuzz_target!(|data: &[u8]| {
    oracle::assert_position_invariant("fuzz", data);
});
