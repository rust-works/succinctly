//! #5305: the cursor path for a string interpolation is jq mode only. yq's slots take only the
//! first value of their generator (live yq v4.53.3: `"\(.a[])"` over `a: [1, 2]` is `1`), so yq
//! mode must keep its own route for the streamed and the collected form alike.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_yq_interpolation_slot_takes_only_its_first_value_5305() -> Result<()> {
    for (filter, want) in [
        (r#""\(.a[])""#, "\"1\"\n"),
        (r#"["\(.a[])"]"#, "[\"1\"]\n"),
        (r#"["\(.a[])\(.b)"]"#, "[\"1x\"]\n"),
        (r#"["\(.a[] | . + 1)"]"#, "[\"2\"]\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, "a: [1, 2]\nb: x\n", &["-o=json", "-I0"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}
