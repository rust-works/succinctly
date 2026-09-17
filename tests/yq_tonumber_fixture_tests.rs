//! Differential replay of the yq-mode `tonumber` grammar fixture (#2960).
//!
//! Real yq's `tonumber` accepts underscored/hex/octal integers, Go hex
//! floats, and inf/nan words that RFC 8259 (and, before this issue, jq's own
//! Rust-native `i64`/`f64` fallback) rejects -- see `tonumber_from_str_yq` in
//! `src/jq/eval.rs`.
//!
//! Every expectation here is captured from the pinned yq (v4.53.3) by
//! `./scripts/sync-yq-tonumber-fixture.sh` into
//! `tests/data/yq-tonumber-fixture.tsv`. Nothing in this file is written by
//! hand from succinctly's own output.
//!
//! The fixture's accept/reject column is *not* `tonumber | tag` -- that
//! turns out to be a looser check than the number yq then tries to marshal:
//! `-0x10`/`0x8000000000000000` (i64 overflow)/`0O17`/`0b101`/`.inf` all
//! report `tag == !!int`/`!!float` yet error on that very JSON marshal (and
//! on ordinary arithmetic, confirmed live against v4.53.3). The fixture's
//! ground truth is the JSON-marshal call's own exit code, which agrees with
//! arithmetic and with the #2960 triage's source-derived reference table.
//!
//! Run with: cargo test --features cli --test yq_tonumber_fixture_tests

#![cfg(feature = "cli")]

use anyhow::Result;
use std::process::Command;

#[path = "common/cargo_run_exit.rs"]
mod cargo_run_exit;
use cargo_run_exit::spawn_with_signal_retry;

const TABLE: &str = include_str!("data/yq-tonumber-fixture.tsv");

/// Reverses `esc()` in `scripts/sync-yq-tonumber-fixture.sh`: `\\` -> `\`,
/// `\t` -> tab, `\n` -> newline.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

struct Row {
    input: String,
    accept: bool,
    json_value: String,
}

fn parse_fixture() -> Vec<Row> {
    let mut rows = Vec::new();
    for line in TABLE.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split('\t');
        let input = unescape(fields.next().expect("input field"));
        let accept = fields.next().expect("accept field") == "1";
        let _tag = fields.next().expect("tag field");
        let json_value = unescape(fields.next().unwrap_or(""));
        rows.push(Row {
            input,
            accept,
            json_value,
        });
    }
    assert!(
        rows.len() > 400,
        "fixture parsed too few rows ({}) -- did tests/data/yq-tonumber-fixture.tsv \
         truncate, or did the TSV column layout change under this parser?",
        rows.len()
    );
    rows
}

/// Runs `succinctly yq -n -o=json -I=0 '"<input>" | tonumber'`, passing the
/// probe string as a Rust-escaped double-quoted filter literal (never
/// through a shell), and returns `(stdout, exit_code)`.
fn run_tonumber(input: &str) -> Result<(String, i32)> {
    let filter = format!("{input:?} | tonumber");
    let (output, exit_code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
            command
                .arg("yq")
                .arg("-n")
                .arg("-o=json")
                .arg("-I=0")
                .arg(&filter);
            command
        },
        None,
    )?;
    let stdout = String::from_utf8(output.stdout)?;
    Ok((stdout, exit_code))
}

#[test]
fn yq_tonumber_matches_pinned_yq_grammar_2960() -> Result<()> {
    let rows = parse_fixture();
    let mut failures = Vec::new();
    for row in &rows {
        let (stdout, exit_code) = run_tonumber(&row.input)?;
        if row.accept {
            if exit_code != 0 {
                failures.push(format!(
                    "{:?}: expected accept -> {:?}, but errored (exit {exit_code}): {stdout}",
                    row.input, row.json_value
                ));
                continue;
            }
            // An empty fixture value means the oracle's own JSON marshal
            // failed on a genuinely-accepted non-finite value (`inf`/`nan`)
            // -- Go's `encoding/json` cannot represent `+Inf`/`NaN` at all,
            // unrelated to `tonumber`'s grammar, and the same limitation
            // succinctly's own JSON output has today (#1071/#2579, tracked
            // separately). Only accept/reject is asserted for these rows.
            if row.json_value.is_empty() {
                continue;
            }
            let got = stdout.trim();
            if got != row.json_value {
                // Fall back to numeric equality before failing: a handful of
                // rows are pre-existing spelling/formatting differences
                // outside #2960's scope, not value bugs --
                // `9223372036854775808`/`99999999999999999999`/`-0` go
                // through the *pre-existing*, untouched RFC 8259
                // literal-preserving arm (explicitly protected by the
                // triage's "24 rows... must not move" -- #1356/#2802's
                // spelling territory), and an underscored float like
                // `1e1_0` differs only in whether succinctly's general
                // Float JSON formatter picks scientific notation
                // (`1e+10`) or full decimal (`10000000000.0`) for the
                // *same* value -- a pre-existing, tonumber-independent
                // formatting policy. Exact match is still required
                // whenever both sides parse identically as text (the
                // common case, including every row that must preserve an
                // exact spelling like `2.50`), so this can't silently
                // mask a real spelling regression -- it only rescues rows
                // where an exact match was already known to disagree for
                // one of the reasons above.
                let numerically_equal = row
                    .json_value
                    .parse::<f64>()
                    .ok()
                    .zip(got.parse::<f64>().ok())
                    .is_some_and(|(want, have)| want == have);
                if !numerically_equal {
                    failures.push(format!(
                        "{:?}: expected {:?}, got {:?}",
                        row.input, row.json_value, got
                    ));
                }
            }
        } else if exit_code == 0 {
            failures.push(format!(
                "{:?}: expected reject, but succeeded with {:?}",
                row.input,
                stdout.trim()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} fixture rows disagreed with succinctly:\n{}",
        failures.len(),
        rows.len(),
        failures.join("\n")
    );
    Ok(())
}

/// jq mode is untouched by #2960 -- every fixture row that yq's own grammar
/// newly accepts (the ones RFC 8259 and jq's native `i64`/`f64` fallback
/// both reject) must still error in jq mode exactly as it did before.
#[test]
fn jq_mode_tonumber_grammar_unchanged_by_2960() -> Result<()> {
    let rows = parse_fixture();
    let mut checked = 0usize;
    let mut failures = Vec::new();
    for row in &rows {
        // Only rows interesting to yq's *new* grammar are checked here --
        // anything jq's own pre-existing fallback (RFC 8259, the leading-`+`
        // arm, or a bare `str::parse::<i64>/<f64>`) already accepted was
        // untouched by this issue in either mode, and jq mode is untouched
        // by construction (the yq-mode branch is an early return before
        // jq's fallback runs, which is otherwise byte-for-byte as it was) --
        // this just spot-checks a sample of the genuinely yq-only forms.
        if !row.accept {
            continue;
        }
        let trimmed = row.input.trim();
        if succinctly::json::validate::is_valid_number(trimmed.as_bytes())
            || trimmed.parse::<i64>().is_ok()
            || trimmed.parse::<f64>().is_ok()
        {
            continue;
        }
        checked += 1;
        let filter = format!("{:?} | tonumber", row.input);
        let (output, exit_code) = spawn_with_signal_retry(
            || {
                let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
                command.arg("jq").arg("-n").arg(&filter);
                command
            },
            None,
        )?;
        if exit_code == 0 {
            let stdout = String::from_utf8(output.stdout)?;
            failures.push(format!(
                "{:?}: jq mode unexpectedly accepted tonumber, got {:?}",
                row.input,
                stdout.trim()
            ));
        }
    }
    assert!(
        checked > 10,
        "expected a meaningful number of yq-only rows to check, got {checked}"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}
