//! #7260: `tostream` consumes the document it materialized and `tostream`/`paths` share the key
//! above a container among every path beneath it, as jq's paths hold the object's own key `jv`s.
//! Neither changes what they print; every row was captured from jq 1.7.1. The last two rows read
//! a shared key's identity in path position (`path($k)` on another path's copy of the key), which
//! jq answers `[]` because both are one key of one object.

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
