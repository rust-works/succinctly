//! Environment-variable handling shared by the CLI runners.
//!
//! The user-facing reference for every variable read by succinctly lives in
//! `docs/reference/environment-variables.md`.
//!
//! Each variable is decoded by a pure function taking the raw value, with a thin
//! wrapper that reads the environment. Keeping the decision pure lets the rules be
//! unit-tested without mutating the process environment, which is `unsafe` under the
//! 2024 edition and racy across concurrently-running tests.

use std::ffi::OsStr;

/// Returns `true` when a `NO_COLOR` value should disable coloured output.
///
/// Per <https://no-color.org/>, colour is suppressed when the variable is present
/// **and** non-empty, regardless of its value; `NO_COLOR=` (empty) leaves colour
/// enabled. jq 1.7.1 implements the same rule, so this matches both the spec and
/// the tool being emulated.
///
/// This is only consulted after the explicit `-C` / `-M` flags, which take priority.
pub fn no_color_disables(value: Option<&OsStr>) -> bool {
    value.is_some_and(|v| !v.is_empty())
}

/// Reads `NO_COLOR` from the environment. See [`no_color_disables`] for the rule.
pub fn no_color_from_env() -> bool {
    no_color_disables(std::env::var_os("NO_COLOR").as_deref())
}

/// What the command-line flags asked for with respect to color.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorChoice {
    /// `--color-output` (`-C`): color regardless of destination or `NO_COLOR`.
    Always,
    /// `--monochrome-output` (`-M`): never color.
    Never,
    /// Neither flag given: let `NO_COLOR` and terminal detection decide.
    Auto,
}

impl ColorChoice {
    /// Map the two mutually-exclusive flags onto a choice. `-M` wins if both are
    /// somehow set.
    pub fn from_flags(monochrome: bool, force_color: bool) -> Self {
        if monochrome {
            Self::Never
        } else if force_color {
            Self::Always
        } else {
            Self::Auto
        }
    }
}

/// Resolve whether to colorize output, in jq's precedence order:
///
/// 1. `--monochrome-output` (`-M`) forces color off
/// 2. `--color-output` (`-C`) forces color on, overriding `NO_COLOR`
/// 3. `NO_COLOR` disables color
/// 4. Otherwise color only when stdout is a terminal
///
/// Taking `stdout_is_tty` as an argument keeps the rule testable: the terminal-
/// dependent arms are unreachable from an integration test, whose stdout is always
/// a pipe.
pub fn resolve_color(choice: ColorChoice, no_color: bool, stdout_is_tty: bool) -> bool {
    match choice {
        ColorChoice::Never => false,
        ColorChoice::Always => true,
        ColorChoice::Auto => !no_color && stdout_is_tty,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn os(value: &str) -> OsString {
        OsString::from(value)
    }

    #[test]
    fn no_color_unset_keeps_color() {
        assert!(!no_color_disables(None));
    }

    #[test]
    fn no_color_empty_keeps_color() {
        // https://no-color.org/ requires a *non-empty* value; jq 1.7.1 agrees.
        assert!(!no_color_disables(Some(&os(""))));
    }

    #[test]
    fn no_color_any_non_empty_value_disables_color() {
        // The spec is explicit that the value itself is not interpreted, so even
        // "0" and "false" disable colour.
        for value in ["1", "0", "false", "no", "anything"] {
            assert!(
                no_color_disables(Some(&os(value))),
                "NO_COLOR={value} should disable colour"
            );
        }
    }

    #[test]
    fn monochrome_flag_beats_everything() {
        for &no_color in &[false, true] {
            for &tty in &[false, true] {
                assert!(!resolve_color(ColorChoice::Never, no_color, tty));
            }
        }
    }

    #[test]
    fn color_flag_beats_no_color_and_pipe() {
        // -C forces colour on even when NO_COLOR is set and stdout is a pipe.
        assert!(resolve_color(ColorChoice::Always, true, false));
        assert!(resolve_color(ColorChoice::Always, false, false));
    }

    #[test]
    fn no_color_beats_tty_detection() {
        assert!(!resolve_color(ColorChoice::Auto, true, true));
    }

    #[test]
    fn without_flags_or_no_color_tty_decides() {
        assert!(resolve_color(ColorChoice::Auto, false, true));
        assert!(!resolve_color(ColorChoice::Auto, false, false));
    }

    #[test]
    fn from_flags_maps_each_flag_combination() {
        assert_eq!(ColorChoice::from_flags(false, false), ColorChoice::Auto);
        assert_eq!(ColorChoice::from_flags(false, true), ColorChoice::Always);
        assert_eq!(ColorChoice::from_flags(true, false), ColorChoice::Never);
        // -M wins over -C if both are somehow set.
        assert_eq!(ColorChoice::from_flags(true, true), ColorChoice::Never);
    }
}
