//! #5876: `succinctly yq` answers a value the program built past 384 levels as yq does, bounded by
//! the native-stack floor (ADR-0025) instead of `MAX_VALUE_TREE_DEPTH`, as `succinctly jq` has been
//! since #5832. Every expected answer was captured from yq v4.53.3: the YAML and JSON text a
//! 385- and a 2000-level fold prints byte for byte, and what the pipes after it answer.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

/// An `items`-element sequence, which `.[] as $i ireduce (0; [.])` folds `items` levels deep.
fn sequence(items: usize) -> String {
    (0..items).map(|i| format!("- {i}\n")).collect()
}

const LEVELS: [usize; 3] = [384, 385, 2000];

#[test]
fn a_fold_past_384_levels_prints_as_yq_does() -> Result<()> {
    for n in LEVELS {
        let input = sequence(n);
        let yaml = format!("{}0\n", "- ".repeat(n));
        let json = format!("{}0{}\n", "[".repeat(n), "]".repeat(n));
        assert_eq!(
            run_yq_stdin(".[] as $i ireduce (0; [.])", &input, &[])?,
            (yaml, 0),
            "{n} levels, YAML"
        );
        assert_eq!(
            run_yq_stdin(".[] as $i ireduce (0; [.])", &input, &["-o", "json", "-I0"])?,
            (json, 0),
            "{n} levels, JSON"
        );
    }
    Ok(())
}

#[test]
fn what_follows_a_deep_fold_answers_as_yq_does() -> Result<()> {
    for n in LEVELS {
        let input = sequence(n);
        for (filter, want) in [
            (".[] as $i ireduce (0; [.]) | length", "1\n".to_string()),
            (
                ".[] as $i ireduce (0; [.]) | .[0] | length",
                "1\n".to_string(),
            ),
            (
                ".[] as $i ireduce ([]; [.]) | flatten | length",
                "0\n".to_string(),
            ),
            (
                ".[] as $i ireduce (0; [.]) | to_yaml | length",
                format!("{}\n", 2 * n + 2),
            ),
            // The bridge hands a built object back to the evaluator whole: it is a value the
            // program built, not a document, so the 256-level document ceiling is not its bound.
            (
                r#".[] as $i ireduce ({}; {"a": .}) | .a | length"#,
                "1\n".to_string(),
            ),
            (
                r#".[] as $i ireduce ({}; {"a": .}) | .a | type"#,
                "!!map\n".to_string(),
            ),
            // yq's `==` is `false` for two containers at any depth (`[0] | . == .` too); the row
            // pins that the walk reaches the answer instead of refusing.
            (".[] as $i ireduce (0; [.]) | . == .", "false\n".to_string()),
        ] {
            assert_eq!(
                run_yq_stdin(filter, &input, &[])?,
                (want, 0),
                "{n} levels: {filter}"
            );
        }
    }
    Ok(())
}

/// The ceiling that held before is gone at the same depths in jq mode, so the two modes agree on a
/// value built between 256 and 384 levels, too.
#[test]
fn a_built_value_between_256_and_384_levels_passes_through_the_bridge() -> Result<()> {
    let input = sequence(300);
    let (out, err, code) = run_yq_stdin_with_stderr(
        r#".[] as $i ireduce ({}; {"a": .}) | .a | length"#,
        &input,
        &[],
    )?;
    assert_eq!((out.as_str(), err.as_str(), code), ("1\n", "", 0));
    Ok(())
}
