//! #5436: a body rebuilt for each value that reads its variable keeps the tails that start after its
//! last reading stage. The copies are pinned by a unit test in both modes; these rows pin that
//! `succinctly yq` still answers as yq does over a YAML sequence. Every output is captured from yq
//! v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

const SEQUENCE: &str = "- 1\n- 2\n- 3\n- 4\n";

/// Ten computing stages, so each tail the pipe hands on is long enough to be kept.
const T: &str = ". + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1";

#[test]
fn test_body_that_reads_its_variable_matches_yq_5436() -> Result<()> {
    for (filter, want) in [
        (
            format!("[.[] | . as $x | (. + $x | {T})]"),
            "[12,14,16,18]\n",
        ),
        (
            format!("[.[] | . as $x | (. + $x | {T} | . + $x)]"),
            "[13,16,19,22]\n",
        ),
        (
            format!("[.[] | . as $x | ({T} | . + $x | {T})]"),
            "[22,24,26,28]\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(&filter, SEQUENCE, &["-o=json", "-I0"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "`{filter}`");
    }
    Ok(())
}
