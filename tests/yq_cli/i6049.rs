//! #6049 fixed a `-0` index's path in jq mode (`path(.[-0])` is `[-0]`). yq's `path` says `0` for the
//! same index (live yq v4.53.3: `[1,2,3]` with `.[-0] | path` prints `- 0`), so the jq fix must stay
//! out of yq mode.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn a_negative_zero_index_stays_an_integer_zero_in_yqs_path() -> Result<()> {
    for filter in [".[-0] | path", ".[0,-0] | path"] {
        let (stdout, code) = run_yq_stdin(filter, "[1, 2, 3]\n", &["-o=json", "-I0"])?;
        assert_eq!(code, 0, "`{filter}` failed");
        assert!(
            stdout.lines().all(|line| line == "[0]"),
            "`{filter}` printed {stdout:?}"
        );
    }
    Ok(())
}
