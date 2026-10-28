//! #5424: a body rebuilt for each value (`. as $x | (...)`) hands on the tails of the pipe it was
//! rebuilt from when the pipe reads no variable. The copies are pinned by a unit test in both modes;
//! these rows pin that `succinctly yq` still answers as yq does over a YAML sequence. Every output is
//! captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

const SEQUENCE: &str = "- 1\n- 2\n- 3\n- 4\n";

/// Ten computing stages, so each tail the pipe hands on is long enough to be kept.
const T: &str = ". + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1 | . + 1";

#[test]
fn test_body_rebuilt_per_value_matches_yq_5424() -> Result<()> {
    for (filter, want) in [
        (format!("[.[] | . as $x | ({T})]"), "[11,12,13,14]\n"),
        (
            format!("[.[] | . as $x | ({T} | . + $x)]"),
            "[12,14,16,18]\n",
        ),
        (
            format!("[.[] | . as $x | ({T}) | select(. > 12)]"),
            "[13,14]\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(&filter, SEQUENCE, &["-o=json", "-I0"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "`{filter}`");
    }
    Ok(())
}
