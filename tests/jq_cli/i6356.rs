//! #6356: `path()` of a static chain past 384 components answers as jq does. #5554 moved the owned
//! walk's bound to ADR-0025's native-stack floor and left the cursor walker on the constant, so
//! `path(.a.a...)` refused where the same chain with a trailing slice answered. Every expected
//! value was captured from jq 1.7.1 (`/usr/bin/jq`).

use std::io::Write;

use anyhow::Result;
use tempfile::NamedTempFile;

use crate::run_jq_full;

const FLOOR_REFUSAL: &str =
    "jq: error (at <stdin>:0): nesting depth exceeds limit of the native stack\n";

#[test]
fn a_static_chain_past_384_components_answers_as_jq_6356() -> Result<()> {
    for components in [385, 1000, 10_000] {
        let fields = ".a".repeat(components);
        let indexes = ".[0]".repeat(components);
        let n = components.to_string();
        let next = (components + 1).to_string();
        for (filter, input, want) in [
            (format!("path({fields}) | length"), "{}", n.as_str()),
            (format!("path({fields}[0]) | length"), "{}", next.as_str()),
            (format!("path({indexes}) | length"), "null", n.as_str()),
            (format!("first(path({fields})) | length"), "{}", n.as_str()),
            (format!("[path({fields})?] | length"), "{}", "1"),
            (
                format!(r#"try (path({fields}) | length) catch "x""#),
                "{}",
                n.as_str(),
            ),
        ] {
            let (stdout, stderr, code) = run_jq_full(&["-c", &filter], Some(input))?;
            assert_eq!(
                (stdout.trim_end(), stderr.as_str(), code),
                (want, "", 0),
                "{components} components"
            );
        }
    }
    Ok(())
}

/// `filter` run from a program file (a chain this long does not fit an argument), on `{}`.
fn run_program_file(filter: &str) -> Result<(String, String, i32)> {
    let mut program = NamedTempFile::new()?;
    program.write_all(filter.as_bytes())?;
    program.flush()?;
    let path = program.path().to_str().unwrap();
    run_jq_full(&["-c", "-f", path], Some("{}"))
}

/// The chain the search for the floor starts at, and the longest it tries.
const FIRST_CHAIN: usize = 2_000_000;
const LONGEST_CHAIN: usize = 32_000_000;

/// Past the floor the walker refuses at exit 5, never a stack overflow (134) or a raw panic (101),
/// and `?` and `try` do not absorb it: the guard keeps its `decode_failure` tag. jq has no row here,
/// since it refuses to compile a program this long.
///
/// How long a chain is past the floor follows the build (#7377): the stack's size keys on
/// `debug_assertions` and a frame's on the optimisation level. Two million components are refused by
/// the unoptimised build and by the release build; an `opt-level=1` build, which two `cli-gated` CI
/// legs are (#7373), answers `2000000` and refuses 2,500,000, and `opt-level=2` and `3` with debug
/// assertions answer 2,500,000 and refuse 3,000,000 (M5 Max). So the chain is found, not fixed: it
/// doubles from two million until the plain program is refused, and every chain short of that has
/// to answer its own length. Below the floor an answer, past it this refusal, and nothing else
/// on any build.
#[test]
fn a_chain_past_the_native_stack_floor_refuses_at_exit_5_6356() -> Result<()> {
    let refused = (String::new(), FLOOR_REFUSAL.to_string(), 5);
    let mut components = FIRST_CHAIN;
    loop {
        let chain = ".a".repeat(components);
        let got = run_program_file(&format!("path({chain}) | length"))?;
        if got == refused {
            break;
        }
        assert_eq!(
            got,
            (format!("{components}\n"), String::new(), 0),
            "{components} components: neither the answer nor the floor's refusal"
        );
        assert!(
            components < LONGEST_CHAIN,
            "no refusal up to {LONGEST_CHAIN} components"
        );
        components *= 2;
    }
    let chain = ".a".repeat(components);
    for filter in [
        format!("[path({chain})?] | length"),
        format!(r#"try (path({chain}) | length) catch "x""#),
    ] {
        assert_eq!(
            run_program_file(&filter)?,
            refused,
            "{components} components: {}",
            &filter[..20]
        );
    }
    Ok(())
}
