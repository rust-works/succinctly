//! #6771: yq's `env(NAME)` decodes the variable's value as YAML and answers its first document, so
//! `12` is a number, `true` a boolean, `{b: fish}` a mapping and `~` a null; `strenv(NAME)` is the string,
//! and an unset variable is `""` for it (yq never checks) where `env` raises. An empty value raises for
//! `env`. `succinctly yq` answered the text from `env` and raised for an unset `strenv`. Every answer
//! was captured from yq v4.53.3 (`-o=json -I=0`). `succinctly jq`'s own `env(NAME)` and `strenv(NAME)`
//! extensions are unchanged.

use std::process::{Command, Stdio};

use anyhow::Result;

/// `succinctly MODE -n -o json -I 0 FILTER` with `myenv` set to `value` (or removed), as `(stdout, status)`.
fn run(mode: &str, filter: &str, value: Option<&str>) -> Result<(String, i32)> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command.arg(mode).env_remove("myenv");
    if mode == "yq" {
        command.args(["-n", "-o", "json", "-I", "0", filter]);
    } else {
        command.args(["-n", "-c", filter]);
    }
    if let Some(value) = value {
        command.env("myenv", value);
    }
    let output = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        output.status.code().unwrap_or(-1),
    ))
}

#[test]
fn test_yq_env_decodes_the_value_as_yaml_6771() -> Result<()> {
    for (value, filter, want) in [
        ("true", "env(myenv)", "true\n"),
        ("true", "env(myenv) | tag", "\"!!bool\"\n"),
        ("true", ".a = env(myenv)", "{\"a\":true}\n"),
        ("12", "env(myenv)", "12\n"),
        ("12", "env(myenv) | tag", "\"!!int\"\n"),
        ("12", ".a = env(myenv)", "{\"a\":12}\n"),
        ("0x1f", "env(myenv)", "31\n"),
        ("0x1f", "env(myenv) | tag", "\"!!int\"\n"),
        ("0x1f", ".a = env(myenv)", "{\"a\":31}\n"),
        ("1.50", "env(myenv)", "1.50\n"),
        ("1.50", "env(myenv) | tag", "\"!!float\"\n"),
        ("1.50", ".a = env(myenv)", "{\"a\":1.50}\n"),
        ("{b: fish}", "env(myenv)", "{\"b\":\"fish\"}\n"),
        ("{b: fish}", "env(myenv) | tag", "\"!!map\"\n"),
        ("{b: fish}", ".a = env(myenv)", "{\"a\":{\"b\":\"fish\"}}\n"),
        ("[1,2]", "env(myenv)", "[1,2]\n"),
        ("[1,2]", "env(myenv) | tag", "\"!!seq\"\n"),
        ("[1,2]", ".a = env(myenv)", "{\"a\":[1,2]}\n"),
        ("- a", "env(myenv)", "[\"a\"]\n"),
        ("- a", "env(myenv) | tag", "\"!!seq\"\n"),
        ("- a", ".a = env(myenv)", "{\"a\":[\"a\"]}\n"),
        ("~", "env(myenv)", "null\n"),
        ("~", "env(myenv) | tag", "\"!!null\"\n"),
        ("~", ".a = env(myenv)", "{\"a\":null}\n"),
        ("null", "env(myenv)", "null\n"),
        ("null", "env(myenv) | tag", "\"!!null\"\n"),
        ("null", ".a = env(myenv)", "{\"a\":null}\n"),
        ("\"quoted\"", "env(myenv)", "\"quoted\"\n"),
        ("\"quoted\"", "env(myenv) | tag", "\"!!str\"\n"),
        ("\"quoted\"", ".a = env(myenv)", "{\"a\":\"quoted\"}\n"),
        ("'sq'", "env(myenv)", "\"sq\"\n"),
        ("'sq'", "env(myenv) | tag", "\"!!str\"\n"),
        ("'sq'", ".a = env(myenv)", "{\"a\":\"sq\"}\n"),
        ("&a 1", "env(myenv)", "1\n"),
        ("&a 1", "env(myenv) | tag", "\"!!int\"\n"),
        ("&a 1", ".a = env(myenv)", "{\"a\":1}\n"),
        ("!!str 12", "env(myenv)", "\"12\"\n"),
        ("!!str 12", "env(myenv) | tag", "\"!!str\"\n"),
        ("!!str 12", ".a = env(myenv)", "{\"a\":\"12\"}\n"),
        (" ", "env(myenv)", "null\n"),
        (" ", "env(myenv) | tag", "\"!!null\"\n"),
        (" ", ".a = env(myenv)", "{\"a\":null}\n"),
        ("# only comment", "env(myenv)", "null\n"),
        ("# only comment", "env(myenv) | tag", "\"!!null\"\n"),
        ("# only comment", ".a = env(myenv)", "{\"a\":null}\n"),
        ("hello", "env(myenv)", "\"hello\"\n"),
        ("hello", "env(myenv) | tag", "\"!!str\"\n"),
        ("hello", ".a = env(myenv)", "{\"a\":\"hello\"}\n"),
        ("yes", "env(myenv)", "\"yes\"\n"),
        ("yes", "env(myenv) | tag", "\"!!str\"\n"),
        ("yes", ".a = env(myenv)", "{\"a\":\"yes\"}\n"),
        ("0o17", "env(myenv)", "15\n"),
        ("0o17", "env(myenv) | tag", "\"!!int\"\n"),
        ("0o17", ".a = env(myenv)", "{\"a\":15}\n"),
        ("a: 1", "env(myenv)", "{\"a\":1}\n"),
        ("a: 1", "env(myenv) | tag", "\"!!map\"\n"),
        ("a: 1", ".a = env(myenv)", "{\"a\":{\"a\":1}}\n"),
        ("1 2", "env(myenv)", "\"1 2\"\n"),
        ("1 2", "env(myenv) | tag", "\"!!str\"\n"),
        ("1 2", ".a = env(myenv)", "{\"a\":\"1 2\"}\n"),
        ("x: &a 1\ny: *a", "env(myenv)", "{\"x\":1,\"y\":1}\n"),
        ("x: &a 1\ny: *a", "env(myenv) | tag", "\"!!map\"\n"),
        (
            "x: &a 1\ny: *a",
            ".a = env(myenv)",
            "{\"a\":{\"x\":1,\"y\":1}}\n",
        ),
    ] {
        let (stdout, status) = run("yq", filter, Some(value))?;
        assert_eq!(
            (stdout.as_str(), status),
            (want, 0),
            "{filter} with {value:?}"
        );
    }
    Ok(())
}

/// An empty value is not provided, as an unset one is; the first document of several is the answer.
#[test]
fn test_yq_env_refuses_an_empty_value_and_reads_the_first_document_6771() -> Result<()> {
    for filter in ["env(myenv)", ".a = env(myenv)"] {
        let (stdout, status) = run("yq", filter, Some(""))?;
        assert_eq!((stdout.as_str(), status), ("", 1), "{filter}");
        let (stdout, status) = run("yq", filter, None)?;
        assert_eq!((stdout.as_str(), status), ("", 1), "{filter}");
    }
    let (stdout, status) = run("yq", "env(myenv)", Some("a: b\n---\nc: d"))?;
    assert_eq!((stdout.as_str(), status), ("{\"a\":\"b\"}\n", 0));
    let (stdout, status) = run("yq", "env(myenv)", Some("a: ["))?;
    assert_eq!((stdout.as_str(), status), ("", 1));
    Ok(())
}

#[test]
fn test_yq_strenv_is_the_text_and_an_unset_variable_is_empty_6771() -> Result<()> {
    for (value, filter, want) in [
        (Some("12"), "strenv(myenv)", "\"12\"\n"),
        (Some("{b: fish}"), "strenv(myenv) | tag", "\"!!str\"\n"),
        (Some(""), "strenv(myenv)", "\"\"\n"),
        (None, "strenv(myenv)", "\"\"\n"),
        (None, ".a = strenv(myenv)", "{\"a\":\"\"}\n"),
        (None, "[strenv(myenv)]", "[\"\"]\n"),
        (None, "strenv(myenv) // \"d\"", "\"\"\n"),
    ] {
        let (stdout, status) = run("yq", filter, value)?;
        assert_eq!(
            (stdout.as_str(), status),
            (want, 0),
            "{filter} with {value:?}"
        );
    }
    Ok(())
}

/// jq's own `env(NAME)` and `strenv(NAME)` extensions keep the text and the unset error.
#[test]
fn test_jq_env_extensions_are_unchanged_6771() -> Result<()> {
    let (stdout, status) = run("jq", "env(myenv)", Some("12"))?;
    assert_eq!((stdout.as_str(), status), ("\"12\"\n", 0));
    let (stdout, status) = run("jq", "strenv(myenv)", None)?;
    assert_eq!((stdout.as_str(), status), ("", 5));
    Ok(())
}
