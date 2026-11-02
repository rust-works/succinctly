//! #5915: the deferred non-string computed-key check is jq's order; yq stringifies a scalar key
//! instead (#2508), so a scalar key never reaches the check. Every row is a live yq v4.53.3 capture.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn a_scalar_computed_key_is_still_stringified_in_yq_5915() -> Result<()> {
    for (filter, want) in [
        (r#"{"n":1} | {(.n): 2}"#, "{\"1\":2}\n"),
        (r#"{"n":1} | {(.n): .[0]?}"#, "{\"1\":null}\n"),
        (r#"{"n":null} | {(.n): 2}"#, "{\"null\":2}\n"),
        (r#"{"n":true} | {(.n): 2}"#, "{\"true\":2}\n"),
        (r#"{"n":1} | {(.n): (1,2)}"#, "{\"1\":1}\n{\"1\":2}\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, "", &["-n", "-o=json", "-I0"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}
