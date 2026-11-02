//! #5915: a computed object key that is no string is rejected once a value exists to pair it with,
//! not when the key is produced. On an owned value the streaming route raised first, so
//! `{"n":1} | {(.n): .[0]?}` failed where jq prints nothing, and `{(.n): .[0]}` reported the key
//! where jq reports the value's own error. Every expected string was captured from jq 1.7.1
//! (`/usr/bin/jq`); stdout and the exit code are compared, since the text of an error is the
//! evaluator's own.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_bad_computed_key_waits_for_a_value_as_in_jq_5915() -> Result<()> {
    for (filter, want, want_code) in [
        (r#"{"n":1} | {(.n): .[0]?}"#, "", 0),
        (r#"{"n":1} | {(.n): empty}"#, "", 0),
        (r#"{"n":1} | {(.n): .[0]}"#, "", 5),
        (r#"{"n":1} | {(.n): 5}"#, "", 5),
        (r#"{"n":1} | {(.n, "a"): .[0]?}"#, "", 0),
        (r#"{"n":1} | {(.n, "a"): 2}"#, "", 5),
        (r#"{"n":null} | {(.n): .x}"#, "", 5),
        (r#"{"n":[1]} | {(.n): 3}"#, "", 5),
        (r#"{"n":1} | {"z": 0, (.n): .[0]?}"#, "", 0),
        (
            r#"{"n":1} | try {(.n): 2} catch ."#,
            "\"Cannot use number (1) as object key\"\n",
            0,
        ),
        (r#"{"n":1} | [{(.n): 2}?]"#, "[]\n", 0),
        (r#"{"n":1} | {"a": (1,2), (.n): .[0]?}"#, "", 0),
        (r#"{"n":1} | first({(.n): 2}, 7)"#, "", 5),
        (
            r#"reduce range(1) as $i ({"n":1}; {"a": (.|tojson), (.n): .[0]?}) | tojson"#,
            "\"null\"\n",
            0,
        ),
        (
            r#"{"n":1} | try {(.n): .[0]} catch ."#,
            "\"Cannot index object with number\"\n",
            0,
        ),
        (
            r#"{"n":1} | try {(.n): 2} catch ."#,
            "\"Cannot use number (1) as object key\"\n",
            0,
        ),
        (r#"{"n":1} | try {(.n): .[0]?} catch ."#, "", 0),
        (r#"{"n":1} | {(.n): (1,2)}"#, "", 5),
        (r#"{"n":1} | [limit(3; {(.n): (1,2)})]"#, "", 5),
        (
            r#"{"n":1} | try {(.n, "a"): error("v")} catch ."#,
            "\"v\"\n",
            0,
        ),
        (r#"{"n":1} | try {(.n): error("v")} catch ."#, "\"v\"\n", 0),
    ] {
        let (stdout, _, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!((stdout.as_str(), code), (want, want_code), "{filter}");
    }
    Ok(())
}
