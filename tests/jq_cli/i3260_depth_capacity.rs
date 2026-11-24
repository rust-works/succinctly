//! The depth-capacity gauge: how deep a recursion answers before ADR-0025's native-stack floor
//! (or `MAX_EVAL_FRAMES`) refuses it, measured, printed and held above a floor per build kind.
//!
//! **If this test fails, a recursive evaluator frame grew.** Every level of a `def` recursion
//! pays for the native frames of the evaluator functions on its path (`eval_each_generic`, the
//! `DefCall` and `as` arms, `eval_single`, ...), and the stack's floor converts their size into a
//! depth. A new `match` arm, a local or a temporary in one of them costs every level, and the
//! capacity printed here shrinks in proportion. ADR-0025 (`docs/adrs/adr-0025.md`) has the model
//! and the ways earlier changes kept their frames out of the recursive ones (an out-of-line
//! `#[inline(never)]` helper for the arm's locals, a shared arm in place of a new one).
//!
//! It exists because the capacity used to be watched by accident. The last row of
//! `test_literal_dollar_argument_matches_jq_3260` ran this module's first shape at depth 12,000,
//! which the unoptimised arm64 builds answer to about 12,075: a frame that grew by a hundred
//! bytes failed that test, on one CI leg, with a message about literal `$` arguments. That row
//! now runs at a depth every build has room for, and this test says what it measures.
//!
//! Each shape's capacity is found by bisection (the greatest depth that answers; every probe must
//! either answer or be refused, never end on a signal) and written to stderr past the harness's
//! capture, so a run that passes shows the numbers too: a capacity that drifts toward its floor
//! is visible before it fails.
//!
//! # Measured capacities (2026-11-24, `cargo test --features cli`)
//!
//! Levels answered, the same on every run of a build. `cap` marks a shape that `MAX_EVAL_FRAMES`
//! (40,000 counted frames) refuses before the stack's floor does: its capacity is 40,000 over the
//! frames a level counts, and it moves only once the floor has sunk under that.
//!
//! | Shape                                | unoptimised arm64 macOS | unoptimised x86_64 Linux | release arm64 macOS | release x86_64 Linux |
//! |--------------------------------------|-------------------------|--------------------------|---------------------|----------------------|
//! | two `$` parameters, one passed on    |                  12,075 |                   13,085 |              12,208 |               12,499 |
//! | two `$` parameters, both computed    |                   9,255 |             10,000 (cap) |               9,487 |                9,707 |
//! | one `$` parameter, under a `reduce`  |                   9,227 |                    9,984 |               8,111 |                8,304 |
//! | no parameter, under an operator      |             6,666 (cap) |              6,666 (cap) |         6,666 (cap) |          6,666 (cap) |
//!
//! Each floor is 5% under the smaller capacity of its build kind, rounded down. A build's capacity
//! does not vary from run to run, so the margin is not for noise: it is room for the platforms
//! not measured. The unoptimised Linux ARM64 build, which CI runs and which could not be
//! measured here, answered the first shape to within 1% of the arm64 macOS build's depth when
//! PR #7873 failed on it, so the arm64 macOS column stands in for it and that CI leg is the
//! confirmation. A frame growth of 1% on the tightest build shows as a smaller number in
//! this test's output, and one past the margin fails it.
//!
//! The build kind is read from `debug_assertions`, which is also what sizes the evaluation
//! thread's stack (2 GiB with them, 256 MiB without). A build with debug assertions and some
//! optimisation (`opt-level=1`, as two `cli-gated` CI legs are) has smaller frames on the larger
//! stack and clears the unoptimised floors with room to spare. This test is listed in
//! `scripts/deep-recursion-tests.sh`, so in CI it runs in the `deep-recursion` legs, which are
//! unoptimised on every platform; the release floors are checked by a `cargo test --release` run
//! and by no CI leg. A coverage build (`cfg(coverage)`) measures and prints without holding the
//! numbers to a floor: its frames are instrumented, and it is neither build kind.
//!
//! A raised floor is fine when a change shrinks a frame for good. A lowered one needs the
//! capacities re-measured on every build kind in the table, and the reason in the commit.

use std::io::Write;

use anyhow::Result;

use crate::run_jq_full;

/// One recursion shape: its program at a depth, what that answers, and its floors.
struct Shape {
    name: &'static str,
    /// The program's text with `DEPTH` standing for the depth.
    template: &'static str,
    /// What the program prints at `depth`.
    answer: fn(u32) -> String,
    /// The least capacity an unoptimised build (debug assertions on) may have.
    unoptimised_floor: u32,
    /// The least capacity a release build (debug assertions off) may have.
    release_floor: u32,
}

/// The greatest depth the search tries. `MAX_EVAL_FRAMES` (40,000) refuses every shape here well
/// short of it on any build.
const DEEPEST: u32 = 1 << 20;

const SHAPES: [Shape; 4] = [
    // The program of #3260's test, whose depth row this gauge replaced: `$p2` is passed on
    // unchanged, and the call is the body's last step.
    Shape {
        name: "two `$` parameters, one passed on",
        template: "def r($p1; $p2): if $p1 == 0 then $p2 else r($p1 - 1; $p2) end; r(DEPTH; 7)",
        answer: |_| "7".to_string(),
        unoptimised_floor: 11_471,
        release_floor: 11_597,
    },
    Shape {
        name: "two `$` parameters, both computed",
        template: "def r($a; $b): if $a == 0 then $b else r($a - 1; $b + 1) end; r(DEPTH; 0)",
        answer: |depth| depth.to_string(),
        unoptimised_floor: 8_792,
        release_floor: 9_012,
    },
    // Not a tail call: each level's output climbs back through a `reduce` (ADR-0025's #7856
    // amendment checks that climb).
    Shape {
        name: "one `$` parameter, under a `reduce`",
        template: "def f($n): if $n == 0 then 0 else reduce f($n - 1) as $x (1; . + $x) end; \
                   f(DEPTH)",
        answer: |depth| depth.to_string(),
        unoptimised_floor: 8_765,
        release_floor: 7_705,
    },
    // Six counted frames a level, so `MAX_EVAL_FRAMES` refuses it at 40,000 / 6 on every build
    // measured, and it falls only once a level's stack has grown enough to bring the floor under
    // that. (`def f: if . > 0 then (. - 1 | f) else . end` would not do: a tail call with no
    // parameter loops in place, #5379, and answers at any depth.)
    Shape {
        name: "no parameter, under an operator",
        template: "def f: if . > 0 then (. - 1 | f) + 1 else 0 end; DEPTH | f",
        answer: |depth| depth.to_string(),
        unoptimised_floor: 6_332,
        release_floor: 6_332,
    },
];

/// Whether `shape` answers at `depth`: `false` when the floor or the frame count refused it
/// (exit 5, `... exceeded maximum recursion depth`). Anything else, a signal or a wrong answer
/// above all, fails the test where it stands.
fn answers(shape: &Shape, depth: u32) -> Result<bool> {
    let program = shape.template.replace("DEPTH", &depth.to_string());
    let (out, err, code) = run_jq_full(&["-nc", &program], None)?;
    if code == 5 && out.is_empty() && err.ends_with("exceeded maximum recursion depth\n") {
        return Ok(false);
    }
    assert_eq!(
        (out.trim_end(), err.as_str(), code),
        ((shape.answer)(depth).as_str(), "", 0),
        "depth {depth}: neither the answer nor a recursion-depth refusal: {program}"
    );
    Ok(true)
}

/// The greatest depth at which `shape` answers: doubled from `start` until a depth is refused,
/// then bisected to the level.
fn capacity(shape: &Shape, start: u32) -> Result<u32> {
    let (mut answered, mut refused) = (0, start.max(1));
    while answers(shape, refused)? {
        answered = refused;
        refused *= 2;
        assert!(
            refused <= DEEPEST,
            "{}: no refusal up to depth {answered}",
            shape.name
        );
    }
    while refused - answered > 1 {
        let middle = answered + (refused - answered) / 2;
        if answers(shape, middle)? {
            answered = middle;
        } else {
            refused = middle;
        }
    }
    Ok(answered)
}

#[test]
fn recursion_depth_capacity_stays_above_its_floor() -> Result<()> {
    let unoptimised = cfg!(debug_assertions);
    let held = !cfg!(coverage);
    let build = format!(
        "{} build, {} {}",
        match (held, unoptimised) {
            (false, _) => "coverage (floors not held)",
            (true, true) => "unoptimised (debug assertions on)",
            (true, false) => "release",
        },
        std::env::consts::ARCH,
        std::env::consts::OS
    );
    let mut report = format!("\nrecursion depth capacity, {build}:\n");
    let mut below = Vec::new();
    for shape in &SHAPES {
        let floor = if unoptimised {
            shape.unoptimised_floor
        } else {
            shape.release_floor
        };
        let measured = capacity(shape, floor)?;
        let verdict = if measured >= floor {
            "ok"
        } else if held {
            below.push(shape.name);
            "BELOW ITS FLOOR"
        } else {
            "below, not held"
        };
        report.push_str(&format!(
            "  {measured:>6} levels answered, floor {floor:>6} ({:+.1}%), {verdict}: {}\n         {}\n",
            (f64::from(measured) / f64::from(floor) - 1.0) * 100.0,
            shape.name,
            shape.template,
        ));
    }
    // Straight to stderr, which the harness does not capture: the capacities show on a pass too.
    std::io::stderr().write_all(report.as_bytes())?;
    assert!(
        below.is_empty(),
        "{report}\
         The recursion depth capacity fell below its floor for: {}.\n\
         Look for a recursive evaluator frame that grew: see ADR-0025 (docs/adrs/adr-0025.md) and \
         the header of tests/jq_cli/i3260_depth_capacity.rs. Every level of a `def` recursion pays \
         for the native frames of the evaluator functions on its path, so a new match arm, local or \
         temporary in one of them (`eval_each_generic`, the `DefCall` and `as` arms, `eval_single`) \
         lowers these numbers: move it into an `#[inline(never)]` helper. A shape that answers \
         exactly 40,000 / k levels is bound by `MAX_EVAL_FRAMES`, not by the stack. Lower a floor \
         only after re-measuring every build kind that file lists.\n",
        below.join("; ")
    );
    Ok(())
}
