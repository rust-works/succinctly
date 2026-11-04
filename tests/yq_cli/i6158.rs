//! #6158: yq spells its JSON-encoding operator `to_json` and `tojson` alike, with an optional
//! literal indent, and its text is the indented form with a newline of its own. Every expected
//! output is what yq v4.53.3 prints (`-o=json -I=0`).

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, expected stdout).
const ROWS: &[(&str, &str, &str)] = &[
    ("{a: 1}\n", "tojson", "\"{\\n  \\\"a\\\": 1\\n}\\n\"\n"),
    ("\"x\"\n", "tojson", "\"\\\"x\\\"\\n\"\n"),
    ("null\n", "tojson", "\"null\\n\"\n"),
    (
        "[1, {b: c}]\n",
        "tojson",
        "\"[\\n  1,\\n  {\\n    \\\"b\\\": \\\"c\\\"\\n  }\\n]\\n\"\n",
    ),
    ("{a: 1}\n", "tojson(0)", "\"{\\\"a\\\":1}\"\n"),
    (
        "{a: 1, b: [1, 2]}\n",
        "tojson(0)",
        "\"{\\\"a\\\":1,\\\"b\\\":[1,2]}\"\n",
    ),
    ("{a: 1}\n", "tojson(4)", "\"{\\n    \\\"a\\\": 1\\n}\\n\"\n"),
    ("{a: 1}\n", ".a | tojson", "\"1\\n\"\n"),
    ("{a: 1}\n", "[tojson]", "[\"{\\n  \\\"a\\\": 1\\n}\\n\"]\n"),
    ("{a: 1}\n", "tojson | length", "13\n"),
];

#[test]
fn tojson_is_to_json_6158() -> Result<()> {
    for &(document, filter, expected) in ROWS {
        let (output, code) = run_yq_stdin(filter, document, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(output, expected, "`{filter}` over {document:?}");
    }
    Ok(())
}

/// jq's `tojson` is unchanged: compact, with no newline of its own.
#[test]
fn jq_mode_tojson_stays_compact_6158() -> Result<()> {
    let (output, code) = crate::run_jq_stdin("tojson", "{\"a\":1}", &[])?;
    assert_eq!((output.as_str(), code), ("\"{\\\"a\\\":1}\"\n", 0));
    Ok(())
}

/// A `del` through `to_json` or `tojson(N)` deletes the node it derives from, as through `tojson`.
#[test]
fn del_through_a_json_text_stage_keeps_the_slot_6158() -> Result<()> {
    for filter in [
        "del(.s | tojson)",
        "del(.s | tojson(0))",
        "del(.s | to_json)",
    ] {
        let (output, code) = run_yq_stdin(filter, "s: x\nt: 1\n", &["-o=json", "-I=0"])?;
        assert_eq!((output.as_str(), code), ("{\"t\":1}\n", 0), "{filter}");
    }
    Ok(())
}
