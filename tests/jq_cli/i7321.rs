//! The macOS binary is built for macOS 12.0, which is what gives it chained fixups (#7321).
//!
//! `.cargo/config.toml` sets `MACOSX_DEPLOYMENT_TARGET`, and nothing else would notice it
//! going missing: the binary would still run, 0.07 ms slower to start. This reads the load
//! commands of the binary this test run built. macOS only: no other platform has a deployment target.

#![cfg(target_os = "macos")]

use std::process::Command;

fn otool(flag: &str) -> String {
    let out = Command::new("otool")
        .args([flag, env!("CARGO_BIN_EXE_succinctly")])
        .output()
        .expect("otool is part of the Xcode command line tools");
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout).unwrap()
}

/// `LC_BUILD_VERSION` names the oldest macOS the binary runs on.
#[test]
fn the_binary_declares_macos_12_as_its_minimum() {
    // A caller that exports its own target (`force = false` lets it) is measuring something else.
    if std::env::var_os("MACOSX_DEPLOYMENT_TARGET").is_some_and(|v| v != "12.0") {
        return;
    }
    let loads = otool("-l");
    let minos = loads
        .lines()
        .find_map(|l| l.trim().strip_prefix("minos "))
        .expect("an LC_BUILD_VERSION with a minos line");
    assert_eq!(minos.trim(), "12.0", "{loads}");
}

/// A 12.0 target is what makes the linker emit chained fixups, which replace the classic
/// rebase and bind opcodes a 11.0 target gets.
#[test]
fn the_binary_uses_chained_fixups_and_no_classic_rebase_opcodes() {
    if std::env::var_os("MACOSX_DEPLOYMENT_TARGET").is_some_and(|v| v != "12.0") {
        return;
    }
    let loads = otool("-l");
    assert!(loads.contains("LC_DYLD_CHAINED_FIXUPS"), "{loads}");
    assert!(!loads.contains("LC_DYLD_INFO"), "{loads}");
}
