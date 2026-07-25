//! The validator must never panic or exhibit UB, on any input.
//!
//! This is the target that justifies the harness. The validator is reachable
//! from `succinctly json validate` and `sjq --validate` on attacker-controlled
//! bytes, so a panic is a denial of service (#151 was exactly that, via
//! unbounded recursion). Run under ASAN, this is also the only check in the
//! repo that would catch memory unsafety if the validator ever grows an
//! `unsafe` fast path.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The result is deliberately ignored: rejecting is fine, panicking is not.
    let _ = succinctly::json::validate::validate(data);
});
