//! #5487's fix is jq mode only: a full-range slice and the reindex bridge's storage sharing are
//! gated off for `succinctly yq`, whose answers do not change (yq has no node identity to observe,
//! so the rows pin that the values are the same ones yq prints). Captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_yq_variable_headed_pipe_and_full_slice_answers_are_unchanged_5487() -> Result<()> {
    for (filter, want) in [
        (".[0:] | .", "[1,2,3]\n"),
        (". as $a | $a | . | .", "[1,2,3]\n"),
        (".[0:2] | .", "[1,2]\n"),
        (". as $a | ($a | .[0:] | .) == $a", "false\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, "[1,2,3]", &["-p=json", "-o=json", "-I=0"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}
