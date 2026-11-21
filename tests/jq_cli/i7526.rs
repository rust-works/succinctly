//! #7526, #7527 and #7500: jq 1.8's hard limits are NOT adopted (owner decision, 2026-11-21).
//!
//! jq 1.8.x added fixed limits to protect its C implementation, each a security fix upstream:
//! a 10,000-level cap on `==`, ordering, `contains`, object merge `*` and the `setpath`/
//! `getpath`/`delpaths` family (`Equality check too deep`, `Comparison too deep`,
//! `Containment check too deep`, `Object merge too deep`, `Path too deep`), a 4,095-parameter cap
//! on a `def` (`too many function parameters or local function definitions (max 4095)`), and a
//! `Repeat string result too long` refusal from 2^31 bytes. `succinctly jq` is written in Rust with
//! 64-bit sizes and answers these inputs correctly, so each is a recorded divergence (see
//! `docs/compliance/jq/limitations.md`, "jq 1.8's hard limits are not adopted"), not a bug.
//!
//! These tests pin the answers so that a later "fidelity fix" toward jq 1.8.2 cannot add a cap
//! without failing here. Every expected value below was captured from jq 1.7.1 (`/usr/bin/jq`),
//! which agrees with `succinctly jq` at these depths (jq 1.7.1 crashes with SIGSEGV from about
//! 100,000 levels, where `succinctly jq` still answers). jq 1.8.2 (`/opt/homebrew/bin/jq`) answers
//! the error named in the comment on each row instead.
//!
//! The multi-gigabyte rows (#7540 strings of 2^31-9 bytes or more, #7541 a 1 GiB token) are not
//! run: they need several GB of memory. They are recorded in the limitations page from the issues'
//! measurements.
//!
//! The values are built by `reduce`, whose fold shares the accumulator, so 20,000 levels cost
//! milliseconds and a debug build's native stack is nowhere near the floor.

use anyhow::Result;

use crate::run_jq_full;

/// Depths either side of jq 1.8.2's 10,000-level caps: just past it, and double it.
const DEPTHS: [u32; 2] = [10_001, 20_000];

fn answer(program: &str) -> Result<String> {
    let (stdout, stderr, code) = run_jq_full(&["-nc", program], None)?;
    assert_eq!(
        code,
        0,
        "`{}`: {stderr}",
        &program[..program.len().min(200)]
    );
    Ok(stdout)
}

/// `[[[...[]...]]]` `n` levels deep, and the same with `1` in the innermost array.
fn nested(n: u32, innermost: &str) -> String {
    format!("(reduce range({n}) as $_ ({innermost}; [.]))")
}

#[test]
fn test_equality_and_ordering_of_values_past_10000_levels_7526() -> Result<()> {
    for n in DEPTHS {
        let a = nested(n, "[]");
        let b = nested(n, "[1]");
        // jq 1.8.2: `Equality check too deep`.
        assert_eq!(
            answer(&format!("{a} as $x | {a} as $y | $x == $y"))?,
            "true\n",
            "n={n}"
        );
        // jq 1.8.2: `Comparison too deep` (`<`, `sort`, `unique`).
        assert_eq!(
            answer(&format!(
                "{a} as $x | {b} as $y | [($x < $y), ($y < $x), ($x != $y)]"
            ))?,
            "[true,false,true]\n",
            "n={n}"
        );
        assert_eq!(
            answer(&format!(
                "{a} as $x | {b} as $y | [$x, $y] | sort | map(length)"
            ))?,
            "[1,1]\n",
            "n={n}"
        );
        assert_eq!(
            answer(&format!(
                "{a} as $x | {b} as $y | [$x, $y, $x] | unique | length"
            ))?,
            "2\n",
            "n={n}"
        );
    }
    Ok(())
}

#[test]
fn test_contains_of_values_past_10000_levels_7526() -> Result<()> {
    for n in DEPTHS {
        let a = nested(n, "[]");
        let b = nested(n, "[1]");
        // jq 1.8.2: `Containment check too deep`.
        assert_eq!(
            answer(&format!("{a} as $x | $x | contains($x)"))?,
            "true\n",
            "n={n}"
        );
        assert_eq!(
            answer(&format!(
                "{a} as $x | {b} as $y | [($x | contains($y)), ($y | contains($x)), ($x | inside($y))]"
            ))?,
            "[false,true,true]\n",
            "n={n}"
        );
    }
    Ok(())
}

#[test]
fn test_object_merge_of_values_past_10000_levels_7526() -> Result<()> {
    for n in DEPTHS {
        let o = format!("(reduce range({n}) as $_ ({{}}; {{a: .}}))");
        // jq 1.8.2: `Object merge too deep`.
        assert_eq!(
            answer(&format!(
                "{o} as $x | [($x * $x | length), ($x * $x == $x)]"
            ))?,
            "[1,true]\n",
            "n={n}"
        );
    }
    Ok(())
}

#[test]
fn test_paths_longer_than_10000_components_7526() -> Result<()> {
    for n in DEPTHS {
        let a = nested(n, "[]");
        let p = format!("[range({n})|0]");
        // jq 1.8.2: `Path too deep` for each of `setpath`, `getpath`, `delpaths` and `tostream`.
        assert_eq!(
            answer(&format!("{p} as $p | null | setpath($p; 0) | getpath($p)"))?,
            "0\n",
            "n={n}"
        );
        assert_eq!(answer(&format!("{a} | getpath({p})"))?, "[]\n", "n={n}");
        let shorter = format!("[range({})|0]", n - 1);
        assert_eq!(
            answer(&format!("{a} | delpaths([{p}]) | getpath({shorter})"))?,
            "[]\n",
            "n={n}"
        );
        assert_eq!(
            answer(&format!("{a} | [tostream] | length"))?,
            format!("{}\n", n + 1),
            "n={n}"
        );
    }
    Ok(())
}

#[test]
fn test_def_with_more_than_4095_parameters_7527() -> Result<()> {
    // jq 1.8.2: `jq: error: too many function parameters or local function definitions
    // (max 4095)` and exit 3 for 4,096 and more; 4,095 is accepted by all three.
    for params in [4095_u32, 4096, 4097, 20_000] {
        let names: Vec<String> = (0..params).map(|i| format!("a{i}")).collect();
        let args: Vec<String> = (0..params).map(|i| i.to_string()).collect();
        let program = format!("def f({}): .; f({})", names.join(";"), args.join(";"));
        // The 20,000-parameter program is about 230 KB, over Linux's 128 KiB limit on one argv
        // word (`MAX_ARG_STRLEN`), so every row goes through a program file.
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("params.jq");
        std::fs::write(&path, &program)?;
        let path = path.to_str().expect("a UTF-8 temp path");
        let (stdout, stderr, code) = run_jq_full(&["-nc", "-f", path], None)?;
        assert_eq!(code, 0, "{params} parameters: {stderr}");
        assert_eq!(stdout, "null\n", "{params} parameters");
    }
    Ok(())
}

#[test]
fn test_string_repeat_refusal_is_succinctlys_own_7500() -> Result<()> {
    // jq 1.8.2 refuses a result of 2,147,483,639 bytes or more with `Repeat string result too
    // long`; jq 1.7.1 on x86_64 Linux answers `""` through an out-of-range `int` cast. Neither is
    // reproduced: `succinctly jq` builds any string the allocator can reserve (a 2 GiB build is
    // not run here) and otherwise refuses with its own wording, at exit 5 and uncatchable by `?`.
    // An unrepresentable count is the cheap row.
    for count in ["1e19", "infinite"] {
        let (stdout, stderr, code) =
            run_jq_full(&["-nc", &format!(r#""x" * {count} | length"#)], None)?;
        assert_eq!(code, 5, "`\"x\" * {count}`: {stderr}");
        assert_eq!(stdout, "");
        assert!(
            stderr.contains("Cannot repeat string to 9223372036854775807 bytes"),
            "`\"x\" * {count}`: {stderr}"
        );
        assert!(
            !stderr.contains("Repeat string result too long"),
            "{stderr}"
        );
    }
    Ok(())
}
