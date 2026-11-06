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

/// Past the floor the walker refuses at exit 5, never a stack overflow (134) or a raw panic (101),
/// and `?` and `try` do not absorb it: the guard keeps its `decode_failure` tag. jq has no row here,
/// since it refuses to compile a program this long. Two million components is past the floor of
/// both the debug and the release thread, and the program has to come from a file to fit.
#[test]
fn a_chain_past_the_native_stack_floor_refuses_at_exit_5_6356() -> Result<()> {
    let chain = ".a".repeat(2_000_000);
    for filter in [
        format!("path({chain}) | length"),
        format!("[path({chain})?] | length"),
        format!(r#"try (path({chain}) | length) catch "x""#),
    ] {
        let mut program = NamedTempFile::new()?;
        program.write_all(filter.as_bytes())?;
        program.flush()?;
        let path = program.path().to_str().unwrap();
        let (stdout, stderr, code) = run_jq_full(&["-c", "-f", path], Some("{}"))?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            ("", FLOOR_REFUSAL, 5),
            "{}",
            &filter[..20]
        );
    }
    Ok(())
}
