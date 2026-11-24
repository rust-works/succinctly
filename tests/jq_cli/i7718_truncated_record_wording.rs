//! #7718's design, where it meets a wording that is not adopted: a type error raised before a
//! `--seq` record that is cut short. jq 1.7.1 (`/usr/bin/jq`) prints the type error alone, because
//! it ends the stream without the end-of-input warning; jq 1.8.2 (`/opt/homebrew/bin/jq`) prints
//! the warning, and words the type error its own way (`string ("a")`, jqlang/jq#3478).
//! `succinctly jq` follows 1.8.2 for the warning (#7718) and keeps 1.7.1's wording (#7450), so its
//! standard error is neither release's. Recorded in `docs/compliance/jq/limitations.md`; the rows
//! pin what `succinctly jq` prints, with both jq answers in the comments (captured on macOS, and
//! jq 1.7.1 on Linux too).

use anyhow::Result;

use crate::run_jq_full;

const STREAM: &str = "\u{1e}[1]\n\u{1e}[2";
const WARNING: &str = "jq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 3\n";
const TYPE_ERROR: &str = "jq: error (at <stdin>:1): Cannot index array with string \"a\"\n";

#[test]
fn a_type_error_before_a_truncated_seq_record_7718() -> Result<()> {
    // jq 1.7.1: the type error, no warning, exit 5.
    // jq 1.8.2: `jq: error (at <stdin>:1): Cannot index array with string ("a")`, then the
    // warning, exit 5.
    let (stdout, stderr, code) = run_jq_full(&["--seq", "-c", ".a"], Some(STREAM))?;
    assert_eq!(
        (stdout.as_str(), stderr, code),
        ("", format!("{TYPE_ERROR}{WARNING}"), 5)
    );
    Ok(())
}

#[test]
fn a_type_error_before_a_truncated_seq_stream_record_7718() -> Result<()> {
    // jq 1.7.1: the type error twice (once for each event of `[1]`), no warning, exit 5.
    // jq 1.8.2: its own wording twice, then the warning, exit 5.
    let (stdout, stderr, code) = run_jq_full(&["--seq", "--stream", "-c", ".a"], Some(STREAM))?;
    assert_eq!(
        (stdout.as_str(), stderr, code),
        ("", format!("{TYPE_ERROR}{TYPE_ERROR}{WARNING}"), 5)
    );
    Ok(())
}
