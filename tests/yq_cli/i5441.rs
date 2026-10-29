//! #5441: a body rebuilt for each value that reads its variable in its last stage, or before a long
//! run of stages, hands on the kept runs of its pipe around the stages that differ per value. The
//! copies are pinned by a unit test in both modes; these rows pin that `succinctly yq` still answers
//! as yq does over a YAML sequence. Every output is captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

const SEQUENCE: &str = "- 1\n- 2\n- 3\n- 4\n";

/// Ten computing stages, so each tail the pipe hands on is long enough to be split.
const T: &str = ". + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1";

#[test]
fn test_body_that_reads_its_variable_late_matches_yq_5441() -> Result<()> {
    for (filter, want) in [
        (
            format!("[.[] | . as $x | ({T} | . + $x)]"),
            "[12,14,16,18]\n",
        ),
        (
            format!("[.[] | . as $x | ({T} | . + $x | . + 1)]"),
            "[13,15,17,19]\n",
        ),
        (
            format!("[.[] | . as $x | ({T} | . + $x | {T})]"),
            "[22,24,26,28]\n",
        ),
        (
            format!("[.[] | . as $x | ({T} | . + $x) | select(. > 14)]"),
            "[16,18]\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(&filter, SEQUENCE, &["-o=json", "-I0"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "`{filter}`");
    }
    Ok(())
}
