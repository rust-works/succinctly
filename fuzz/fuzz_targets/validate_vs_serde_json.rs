//! Validity must agree with `serde_json`, modulo classified divergences.
//!
//! `tests/json_test_suite.rs` checks conformance against a fixed 318-case
//! corpus; this explores the space between those cases. It is the only check
//! that can catch the validator being *self-consistently* wrong, since every
//! other layer compares it against itself.
#![no_main]

use libfuzzer_sys::fuzz_target;

// Shared with tests/json_validate_properties.rs so the divergence classifier
// cannot drift between them — a stale copy here would silently excuse a real
// bug. See the module docs for why this is a path include.
#[path = "../../tests/common/json_oracle.rs"]
mod oracle;

fuzz_target!(|data: &[u8]| {
    oracle::assert_serde_agreement("fuzz", data);
});
