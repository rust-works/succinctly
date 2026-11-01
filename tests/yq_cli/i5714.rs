//! #5714: the named-string door of #5691 (`$s|length`, `startswith`, `endswith`) answers in yq
//! mode as the bridge did (`startswith`/`endswith` are jq builtins, so they need `--jq-extensions`;
//! the outputs equal the pre-#5710 binary's, which is what this pins): `length` counts bytes there (`h\u{e9}llo` is 6, as in #5712), and the
//! prefix and suffix tests compare bytes, bare and inside an array construction.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_named_string_door_counts_bytes_as_yq_does_5714() -> Result<()> {
    for (filter, expected) in [
        ("$s|length", "6\n"),
        ("[$s|length]|tojson", "[6]\n"),
        ("$s|startswith(\"h\")", "true\n"),
        ("$s|endswith(\"llo\")", "true\n"),
        ("[$s|endswith(\"x\")]|tojson", "[false]\n"),
    ] {
        let (stdout, code) = run_yq_stdin(
            filter,
            "",
            &["-n", "--jq-extensions", "--arg", "s", "h\u{e9}llo"],
        )?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}");
    }
    Ok(())
}
