//! The path-context sweep's generator alphabet is part of the claim (#2416).
//!
//! `scripts/jq-path-context-oracle-sweep.sh` is #2416 phase 0's oracle-backed
//! net: it sweeps path-context shapes against pinned `jq` 1.7.1 and pinned
//! `yq` v4.53.3 so a migrated arm of `eval_stage_with_path_context` that
//! answers differently from the *reference* fails, rather than merely
//! differently from the in-tree bridge (which #2388 showed is itself not a
//! trustworthy reference).
//!
//! A sweep is only worth what its generator can emit. This repo has already
//! been burned once by a fuzzer whose pool could not produce the shape the
//! bug lived in (#2041), so the alphabet gets an assertion of its own — held
//! **here**, independently of the script's own `--self-test`, and stated as a
//! literal list rather than re-derived from the script. Shrinking the script's
//! alphabet then fails this test even if the same edit also shrinks the
//! script's self-check.
//!
//! Two claims are asserted:
//!
//! 1. **Coverage.** Every `<leaf>/<wrapper>/<outer>` combination class below
//!    appears in the generated corpus, per mode — `key`/`parent`/`path`/
//!    `file_index` in yq mode and the path-expression family in jq mode, each
//!    under `limit`/`first`/`last`/`foreach`/`reduce`/`getpath`/object
//!    construction/`select`/comma/`if`/`try`/`label`, and each of those again
//!    under `path(...)` and `?`.
//! 2. **yq comma/pipe precedence (#2420).** Real yq v4.53.3 groups `a, b | c`
//!    as `a, (b | c)` where jq groups it as `(a, b) | c`. succinctly used to
//!    apply jq's grouping in both modes; since #2420 `succinctly yq` follows
//!    yq's own precedence table and the two agree. The rule is kept all the
//!    same: a yq-mode case holding a `,` and a `|` at the same bracket depth
//!    would make its meaning depend on which grouping rule is in force, which
//!    says nothing about path context, so no such case may be generated.
//!
//! `--list-cases` needs neither the succinctly binary nor an oracle, so this
//! test is hermetic and runs on every CI leg.
//!
//! Deliberately **not** feature-gated: it needs nothing from the crate, and
//! the `cli` legs in `.github/workflows/ci.yml` run an explicit `--test` list
//! that this file is not on. A `#![cfg(feature = "cli")]` here would compile
//! it away on every leg that does run it, leaving a guard that never guards
//! (`tests/jq_path_context_arm_guard.rs` is ungated for the same reason).
//!
//! Run with: cargo test --test jq_path_context_alphabet_tests

use std::collections::BTreeSet;
use std::process::Command;

const SCRIPT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/scripts/jq-path-context-oracle-sweep.sh"
);

/// Path-context leaves per mode. jq 1.7.1 has no `key`/`parent`/bare `path`/
/// `file_index` at all — verified live — so the two modes necessarily carry
/// different leaf sets: `src/jq/eval.rs`'s `needs_path_context` family in yq
/// mode, and the path-*expression* family jq does have in jq mode.
const JQ_LEAVES: &[&str] = &["path_f", "paths", "getpath", "path_dd"];
const YQ_LEAVES: &[&str] = &["key", "parent", "path", "file_index"];

/// Every consumer construct #2416's phase-0 checklist names.
const WRAPPERS: &[&str] = &[
    "bare", "limit", "first", "last", "foreach", "reduce", "getpath", "object", "select", "comma",
    "if", "try", "label",
];

/// `path(...)` over each of the above, plus `?`.
const OUTERS: &[&str] = &["plain", "path", "opt"];

struct GeneratedCase {
    mode: String,
    class: String,
    filter: String,
}

/// Ask the sweep script for the corpus it would run. Deliberately the same
/// code path the sweep itself generates from, so this cannot pass against a
/// list nothing runs.
fn generated_cases() -> Vec<GeneratedCase> {
    let output = Command::new("bash")
        .arg(SCRIPT)
        .arg("--list-cases")
        .output()
        .unwrap_or_else(|e| panic!("run {SCRIPT} --list-cases: {e}"));
    assert!(
        output.status.success(),
        "{SCRIPT} --list-cases exited {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("--list-cases output is UTF-8");
    let cases: Vec<GeneratedCase> = text
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut fields = line.split('\t');
            let mode = fields.next().unwrap_or_default().to_string();
            let class = fields.next().unwrap_or_default().to_string();
            let filter = fields.next().unwrap_or_default().to_string();
            assert!(
                !mode.is_empty() && !class.is_empty() && !filter.is_empty(),
                "malformed --list-cases line (want `mode<TAB>class<TAB>filter`): {line}"
            );
            GeneratedCase {
                mode,
                class,
                filter,
            }
        })
        .collect();
    assert!(
        cases.len() >= 500,
        "generated corpus looks truncated ({} cases) — the alphabet is part of \
         the claim (#2041), so a shrink has to be argued, not slipped in",
        cases.len()
    );
    cases
}

#[test]
fn every_combination_class_is_generated() {
    let cases = generated_cases();
    // A class key in the corpus is `<leaf>/<wrapper>/<outer>/<prefix>` for the
    // curated cross product (and `gen/...` for the mechanically composed
    // rows); the first three components are what this test claims.
    let present: BTreeSet<(String, String)> = cases
        .iter()
        .filter_map(|case| {
            let parts: Vec<&str> = case.class.split('/').collect();
            (parts.len() >= 4 && parts[0] != "gen")
                .then(|| (case.mode.clone(), parts[..3].join("/")))
        })
        .collect();

    let mut missing = Vec::new();
    for (mode, leaves) in [("jq", JQ_LEAVES), ("yq", YQ_LEAVES)] {
        for leaf in leaves {
            for wrapper in WRAPPERS {
                for outer in OUTERS {
                    let want = (mode.to_string(), format!("{leaf}/{wrapper}/{outer}"));
                    if !present.contains(&want) {
                        missing.push(format!("{mode}: {leaf}/{wrapper}/{outer}"));
                    }
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "scripts/jq-path-context-oracle-sweep.sh no longer generates {} combination \
         class(es) this test claims it covers:\n  {}\n\nThe alphabet is part of the \
         claim (#2416 phase 0, #2041's lesson): restore the atom, or change this \
         list deliberately and say why.",
        missing.len(),
        missing.join("\n  ")
    );
}

/// Whether `filter` holds a `,` and a `|` at the same bracket depth — the
/// #2420 precedence hazard.
fn comma_and_pipe_share_a_depth(filter: &str) -> bool {
    let mut depth: usize = 0;
    let mut seen_comma = vec![false];
    let mut seen_pipe = vec![false];
    for ch in filter.chars() {
        match ch {
            '(' | '[' | '{' => {
                depth += 1;
                if seen_comma.len() <= depth {
                    seen_comma.push(false);
                    seen_pipe.push(false);
                } else {
                    seen_comma[depth] = false;
                    seen_pipe[depth] = false;
                }
            }
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' => {
                seen_comma[depth] = true;
                if seen_pipe[depth] {
                    return true;
                }
            }
            '|' => {
                seen_pipe[depth] = true;
                if seen_comma[depth] {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

#[test]
fn no_yq_case_mixes_comma_and_pipe_at_one_depth() {
    let offenders: Vec<String> = generated_cases()
        .iter()
        .filter(|case| case.mode == "yq" && comma_and_pipe_share_a_depth(&case.filter))
        .map(|case| format!("[{}] {}", case.class, case.filter))
        .collect();
    assert!(
        offenders.is_empty(),
        "{} yq-mode case(s) mix `,` and `|` at one bracket depth. Real yq v4.53.3 \
         groups `a, b | c` as `a, (b | c)` where jq groups `(a, b) | c` (#2420, \
         now matched in yq mode), so these would make a case's meaning depend on \
         the grouping rule rather than on path context — parenthesise each comma \
         branch:\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}

/// The hazard detector itself needs a negative test: a check that cannot fail
/// is not a check. Both directions, so neither a false accept nor a false
/// reject can hide (the #2416 brief's own "negative-test every way a gate can
/// pass" rule).
#[test]
fn comma_pipe_detector_is_sensitive_and_specific() {
    // Hazards: the two orderings that actually reparse under yq.
    assert!(comma_and_pipe_share_a_depth(".a, .b | key"));
    assert!(comma_and_pipe_share_a_depth(".a | key, path"));
    assert!(comma_and_pipe_share_a_depth("((.a, .b | key))"));
    assert!(comma_and_pipe_share_a_depth("[.a, .b | key]"));
    // Safe: the comma is parenthesised away from the pipe, or there is no
    // comma/pipe pair at any one depth at all.
    assert!(!comma_and_pipe_share_a_depth("(.a | key), (.b | key)"));
    assert!(!comma_and_pipe_share_a_depth(".a[] | ((key), 1)"));
    assert!(!comma_and_pipe_share_a_depth(".a | key"));
    assert!(!comma_and_pipe_share_a_depth("[1, 2]"));
    assert!(!comma_and_pipe_share_a_depth(
        r".a[] | label $o | ((key), break $o)"
    ));
}
