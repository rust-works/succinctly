//! #5791: the jq-mode fix that makes `map_values(f)` read only the first output of `f` leaves yq
//! alone. Real yq (v4.53.3) raises the error `f` produces after its first output, for a sequence
//! and a mapping alike, and so does `succinctly yq`.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

#[test]
fn map_values_still_raises_an_error_after_the_first_output_in_yq_5791() -> Result<()> {
    for input in ["- 1\n", "a: 1\n"] {
        let (out, err, code) =
            run_yq_stdin_with_stderr("map_values((1, error(\"x\")))", input, &[])?;
        assert_eq!(
            (out.as_str(), code, err.contains('x')),
            ("", 1, true),
            "{input}: {err}"
        );
    }
    Ok(())
}

/// With `--jq-extensions` (`label` and `halt_error` are jq-only, so real yq has nothing to compare
/// with) yq mode keeps running `f` past its first output: a `break` or `halt_error(3)` after it
/// still surfaces, for a mapping and a sequence. This is `succinctly`'s own contract, pinned so the
/// arms the jq-mode cut no longer reaches stay covered.
#[test]
fn map_values_runs_a_break_or_halt_after_the_first_output_in_yq_5791() -> Result<()> {
    for input in ["- 1\n", "a: 1\n"] {
        let (out, _, code) = run_yq_stdin_with_stderr(
            "label $x | map_values((1, break $x))",
            input,
            &["--jq-extensions"],
        )?;
        assert_eq!((out.as_str(), code), ("", 0), "break over {input}");
        let (out, err, code) = run_yq_stdin_with_stderr(
            "map_values((1, halt_error(3)))",
            input,
            &["--jq-extensions"],
        )?;
        assert_eq!(
            (out.as_str(), code, err.as_str()),
            ("", 3, "1\n"),
            "halt over {input}"
        );
    }
    Ok(())
}
