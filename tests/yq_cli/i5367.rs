//! #5367: POSIX bracket classes stay ASCII-only in yq mode. yq compiles patterns with Go's
//! `regexp`, whose `[[:alpha:]]` is ASCII (a live yq v4.53.3 capture), so the Unicode rewrite jq
//! mode applies (Oniguruma's classes are Unicode-aware) must not reach it.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_posix_bracket_classes_stay_ascii_in_yq_mode_5367() -> Result<()> {
    for (filter, want) in [
        (r#""é" | test("[[:alpha:]]")"#, "false\n"),
        (r#""é" | test("[^[:alpha:]]")"#, "true\n"),
        (r#""a" | test("[[:alpha:]]")"#, "true\n"),
        // `\p{L}` is Unicode in Go and stays so.
        (r#""é" | test("\\p{L}")"#, "true\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, "", &["-n"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}
