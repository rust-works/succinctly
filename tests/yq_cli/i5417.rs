//! #5417: a parenthesized pipe over computed values that is driven one output at a time keeps the
//! owned copy of each tail on the pipe instead of copying the rest of it per value at every stage.
//! The copy count is pinned by a unit test in both modes; these rows pin that `succinctly yq`
//! still answers as yq does over a YAML sequence. Every output is captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

const SEQUENCE: &str = "- 1\n- 2\n- 3\n- 4\n";

/// Four computing stages, so each tail the pipe hands on is a pipe of its own.
const T: &str = ". + 1 | . + 1 | . + 1 | . + 1";

#[test]
fn test_parenthesized_pipe_driven_one_output_at_a_time_matches_yq_5417() -> Result<()> {
    for (filter, want) in [
        (format!("[.[] | ({T}) | select(. > 6)]"), "[7,8]\n"),
        (format!(".[] | ({T}) | select(. > 6)"), "7\n8\n"),
        (format!(".[] | ({T}) | select(. > 99)"), ""),
    ] {
        let (stdout, code) = run_yq_stdin(&filter, SEQUENCE, &["-o=json", "-I0"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "`{filter}`");
    }
    Ok(())
}
