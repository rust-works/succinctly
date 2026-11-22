//! #7462: a run with no program is jq's usage error only when standard input and output are both
//! terminals (`main.c`: `if (!isatty(STDOUT_FILENO) || !isatty(STDIN_FILENO))` the program is
//! `.`). A test cannot give the binary a terminal, so the terminal side is the unit test of
//! `no_program_is_a_usage_error` and the manual check in `docs/compliance/jq/limitations.md`;
//! what a test can pin is that a pipe on either side still defaults to `.`, as jq 1.7.1 and 1.8.2
//! do (`echo '[1]' | jq -c` prints `[1]`, exit 0; `jq -n` with a pipe on stdout prints `null`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_no_program_with_a_pipe_on_each_side_is_the_identity_7462() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c"], Some("[1]\n"))?;
    assert_eq!((stdout.as_str(), stderr.as_str(), code), ("[1]\n", "", 0));
    Ok(())
}

#[test]
fn test_no_program_and_null_input_with_pipes_prints_null_7462() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n"], None)?;
    assert_eq!((stdout.as_str(), stderr.as_str(), code), ("null\n", "", 0));
    Ok(())
}
