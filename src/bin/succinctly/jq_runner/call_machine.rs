//! The call machine's switch (#7977): `SUCCINCTLY_CALL_MACHINE` turns it on for the run, and
//! `SUCCINCTLY_DEBUG_CALL_MACHINE_LOG` names a file each routing decision and each compilation is
//! appended to.
//!
//! The machine is experimental and behind this runtime switch for measurement: one binary is
//! timed both ways with the variable set in both arms (`=1` against `=0`, #7961). The library reads
//! no environment for it; this is the one place that does, and it hands the answer over with
//! `path_mode::set_call_machine`.

use std::ffi::OsStr;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use succinctly::jq::path_mode::{self, CallMachineEvent};

/// Turns the machine on for any non-empty value but `0`, as the `SUCCINCTLY_DEBUG_JQ_*` switches
/// read theirs ([`super::window::WHOLE_FILE_ENV`]).
const SWITCH_ENV: &str = "SUCCINCTLY_CALL_MACHINE";

/// A file each decision appends `taken` or `declined` to, and each compilation `compiled`, one line
/// each, while [`SWITCH_ENV`] is on. `scripts/call_machine/check.py` and `tests/jq_cli/i7977.rs`
/// read it to tell a program the machine answered from one it declined, which the outputs alone
/// cannot, and to count how often a run decides and compiles (once each, #7977 step 5).
const DECISION_LOG_ENV: &str = "SUCCINCTLY_DEBUG_CALL_MACHINE_LOG";

/// The file [`DECISION_LOG_ENV`] named.
static DECISION_LOG: OnceLock<PathBuf> = OnceLock::new();

/// Whether `value`, the content of [`SWITCH_ENV`], turns the machine on.
fn switch_value_on(value: Option<&OsStr>) -> bool {
    value.is_some_and(|value| !value.is_empty() && value != "0")
}

/// Read the switch and the log for this run, before its route is decided.
pub(super) fn configure() {
    let on = switch_value_on(std::env::var_os(SWITCH_ENV).as_deref());
    path_mode::set_call_machine(on);
    if let Some(path) = std::env::var_os(DECISION_LOG_ENV).filter(|_| on) {
        let _ = DECISION_LOG.set(PathBuf::from(path));
        path_mode::observe_call_machine(Some(note_decision));
    }
}

/// The line [`DECISION_LOG`] records `event` as.
fn event_line(event: CallMachineEvent) -> &'static str {
    match event {
        CallMachineEvent::Taken => "taken",
        CallMachineEvent::Declined => "declined",
        CallMachineEvent::Compiled => "compiled",
    }
}

/// Append one decision or compilation to [`DECISION_LOG`].
fn note_decision(event: CallMachineEvent) {
    let Some(path) = DECISION_LOG.get() else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
    {
        let _ = writeln!(file, "{}", event_line(event));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_is_on_only_for_a_value_but_0_7977() {
        assert!(!switch_value_on(None));
        assert!(!switch_value_on(Some(OsStr::new(""))));
        assert!(!switch_value_on(Some(OsStr::new("0"))));
        assert!(switch_value_on(Some(OsStr::new("1"))));
        assert!(switch_value_on(Some(OsStr::new("yes"))));
    }

    #[test]
    fn each_event_has_its_own_line_7977() {
        assert_eq!(event_line(CallMachineEvent::Taken), "taken");
        assert_eq!(event_line(CallMachineEvent::Declined), "declined");
        assert_eq!(event_line(CallMachineEvent::Compiled), "compiled");
    }
}
