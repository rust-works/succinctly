//! #5363: `capture` keeps a named group that took no part in the match as `null` in yq mode too,
//! where it shares the implementation with jq mode. Every row is a live yq v4.53.3 capture.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_capture_keeps_a_group_that_did_not_match_as_null_5363() -> Result<()> {
    for (filter, want) in [
        (
            r#""b" | capture("(?<x>a)?(?<y>b)")"#,
            "{\"x\":null,\"y\":\"b\"}\n",
        ),
        (
            r#""a" | capture("(?<x>a)(?<y>b)?")"#,
            "{\"x\":\"a\",\"y\":null}\n",
        ),
        (
            r#""ab" | capture("(?<x>a)|(?<y>b)";"g")"#,
            "{\"x\":\"a\",\"y\":null}\n{\"x\":null,\"y\":\"b\"}\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(filter, "", &["-n", "-o=json", "-I=0"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}
