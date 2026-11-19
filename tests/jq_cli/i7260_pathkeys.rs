//! #7260: `tostream` consumes the document it materialized and `tostream`/`paths` share the key
//! above a container among every path beneath it, as jq's paths hold the object's own key `jv`s.
//! Neither changes what they print; every row was captured from jq 1.7.1. The last two rows read
//! a shared key's identity in path position (`path($k)` on another path's copy of the key), which
//! jq answers `[]` because both are one key of one object.
//!
//! In jq mode `tostream` consumes the document it materializes, collapsing a repeated key to its
//! first position and last value; with `. as $x` in scope the materialized value can be the
//! binding's own, shared, and is then cloned out of instead, which must print the same.

use anyhow::Result;

use crate::run_jq_stdin;

const DOC: &str =
    r#"{"a":{"b":1,"c":[2,{"d":"x"}],"e":{}},"f":[],"g":"s","h":{"a":1,"a":{"z":2}}}"#;

/// `(filter, stdout)` for `-c` over [`DOC`].
const ROWS: &[(&str, &str)] = &[
    (
        "[tostream]",
        r#"[[["a","b"],1],[["a","c",0],2],[["a","c",1,"d"],"x"],[["a","c",1,"d"]],[["a","c",1]],[["a","e"],{}],[["a","e"]],[["f"],[]],[["g"],"s"],[["h","a","z"],2],[["h","a","z"]],[["h","a"]],[["h"]]]"#,
    ),
    (
        "[paths]",
        r#"[["a"],["a","b"],["a","c"],["a","c",0],["a","c",1],["a","c",1,"d"],["a","e"],["f"],["g"],["h"],["h","a"],["h","a","z"]]"#,
    ),
    (
        "fromstream(tostream)",
        r#"{"a":{"b":1,"c":[2,{"d":"x"}],"e":{}},"f":[],"g":"s","h":{"a":{"z":2}}}"#,
    ),
    (
        "[tostream] | .[0][0][0] as $k | .[1][0][0] | path($k)",
        "[]",
    ),
    ("[paths] | .[0][0] as $k | .[1][0] | path($k)", "[]"),
];

#[test]
fn tostream_and_paths_print_as_jq_7260() -> Result<()> {
    for (filter, want) in ROWS {
        let (out, code) = run_jq_stdin(filter, DOC, &["-c"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(out.trim_end(), *want, "{filter}");
    }
    Ok(())
}

const DUPLICATE_KEYS: &str = r#"{"a":1,"b":{"c":"x"},"a":{"d":[2]}}"#;

#[test]
fn tostream_collapses_a_repeated_key_on_both_routes_7260() -> Result<()> {
    let want = r#"[[["a","d",0],2],[["a","d",0]],[["a","d"]],[["b","c"],"x"],[["b","c"]],[["b"]]]"#;
    for filter in ["[tostream]", ". as $x | [tostream]"] {
        let (out, code) = run_jq_stdin(filter, DUPLICATE_KEYS, &["-c"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(out.trim_end(), want, "{filter}");
    }
    let (out, code) = run_jq_stdin(
        "[tostream] | .[0][1] as $v | path(..|select(. == $v))",
        DUPLICATE_KEYS,
        &["-c"],
    )?;
    assert_eq!((out.trim_end(), code), ("[0,1]", 0));
    Ok(())
}

#[test]
fn tostream_over_a_malformed_document_fails_as_jq_7260() -> Result<()> {
    // jq 1.7.1: `parse error: Expected value before ',' at line 1, column 9`, exit 5.
    let (out, code) = run_jq_stdin("[tostream]", r#"{"a":[1,,2]}"#, &["-c"])?;
    assert_eq!((out.as_str(), code), ("", 5));
    Ok(())
}
