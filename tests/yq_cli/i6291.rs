//! #6291: a node with an explicit tag and no value is the empty string in yq whatever the tag (`!!seq`,
//! `!!map`, `!!binary`, `!!timestamp`, `!custom`), `false` for `!!bool` and `null` for `!!null`: the tag
//! decides the kind, so the empty text is no null. It read as `null` in JSON output and in a comparison with
//! `""`; `!!str` already answered. Every row is a live yq v4.53.3 capture (`-o=json -I0`), over a mapping
//! value, one with a comment, a sequence item, and a whole document with and without `---`.
//! The last rows end the text on the tag, and hold the explicit keys. Left out: `!!bool` compared with `""` (yq compares a scalar's text, so `!!bool` alone is `true`), and
//! `!!int`/`!!float`, where yq's JSON encoder fails (`docs/compliance/yq/limitations.md`).

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, expected stdout).
const ROWS: &[(&str, &str, &str)] = &[
    ("k: !!str\nj: 1\n", ".", "{\"k\":\"\",\"j\":1}\n"),
    ("k: !!str\nj: 1\n", ".k == \"\"", "true\n"),
    ("k: !!str # c\nj: 1\n", ".", "{\"k\":\"\",\"j\":1}\n"),
    ("k: !!str # c\nj: 1\n", ".k | length", "0\n"),
    ("- !!str\n- 1\n", ".", "[\"\",1]\n"),
    ("- !!str\n- 1\n", ".[0] == \"\"", "true\n"),
    ("!!str\n", ".", "\"\"\n"),
    ("!!str\n", ". == \"\"", "true\n"),
    ("--- !!str\n", ".", "\"\"\n"),
    ("--- !!str\n", ". == \"\"", "true\n"),
    ("k: !!seq\nj: 1\n", ".", "{\"k\":\"\",\"j\":1}\n"),
    ("k: !!seq\nj: 1\n", ".k == \"\"", "true\n"),
    ("k: !!seq # c\nj: 1\n", ".", "{\"k\":\"\",\"j\":1}\n"),
    ("k: !!seq # c\nj: 1\n", ".k | length", "0\n"),
    ("- !!seq\n- 1\n", ".", "[\"\",1]\n"),
    ("- !!seq\n- 1\n", ".[0] == \"\"", "true\n"),
    ("!!seq\n", ".", "\"\"\n"),
    ("!!seq\n", ". == \"\"", "true\n"),
    ("--- !!seq\n", ".", "\"\"\n"),
    ("--- !!seq\n", ". == \"\"", "true\n"),
    ("k: !!map\nj: 1\n", ".", "{\"k\":\"\",\"j\":1}\n"),
    ("k: !!map\nj: 1\n", ".k == \"\"", "true\n"),
    ("k: !!map # c\nj: 1\n", ".", "{\"k\":\"\",\"j\":1}\n"),
    ("k: !!map # c\nj: 1\n", ".k | length", "0\n"),
    ("- !!map\n- 1\n", ".", "[\"\",1]\n"),
    ("- !!map\n- 1\n", ".[0] == \"\"", "true\n"),
    ("!!map\n", ".", "\"\"\n"),
    ("!!map\n", ". == \"\"", "true\n"),
    ("--- !!map\n", ".", "\"\"\n"),
    ("--- !!map\n", ". == \"\"", "true\n"),
    ("k: !!binary\nj: 1\n", ".", "{\"k\":\"\",\"j\":1}\n"),
    ("k: !!binary\nj: 1\n", ".k == \"\"", "true\n"),
    ("k: !!binary # c\nj: 1\n", ".", "{\"k\":\"\",\"j\":1}\n"),
    ("k: !!binary # c\nj: 1\n", ".k | length", "0\n"),
    ("- !!binary\n- 1\n", ".", "[\"\",1]\n"),
    ("- !!binary\n- 1\n", ".[0] == \"\"", "true\n"),
    ("!!binary\n", ".", "\"\"\n"),
    ("!!binary\n", ". == \"\"", "true\n"),
    ("--- !!binary\n", ".", "\"\"\n"),
    ("--- !!binary\n", ". == \"\"", "true\n"),
    ("k: !!timestamp\nj: 1\n", ".", "{\"k\":\"\",\"j\":1}\n"),
    ("k: !!timestamp\nj: 1\n", ".k == \"\"", "true\n"),
    ("k: !!timestamp # c\nj: 1\n", ".", "{\"k\":\"\",\"j\":1}\n"),
    ("k: !!timestamp # c\nj: 1\n", ".k | length", "0\n"),
    ("- !!timestamp\n- 1\n", ".", "[\"\",1]\n"),
    ("- !!timestamp\n- 1\n", ".[0] == \"\"", "true\n"),
    ("!!timestamp\n", ".", "\"\"\n"),
    ("!!timestamp\n", ". == \"\"", "true\n"),
    ("--- !!timestamp\n", ".", "\"\"\n"),
    ("--- !!timestamp\n", ". == \"\"", "true\n"),
    ("k: !custom\nj: 1\n", ".", "{\"k\":\"\",\"j\":1}\n"),
    ("k: !custom\nj: 1\n", ".k == \"\"", "true\n"),
    ("k: !custom # c\nj: 1\n", ".", "{\"k\":\"\",\"j\":1}\n"),
    ("k: !custom # c\nj: 1\n", ".k | length", "0\n"),
    ("- !custom\n- 1\n", ".", "[\"\",1]\n"),
    ("- !custom\n- 1\n", ".[0] == \"\"", "true\n"),
    ("!custom\n", ".", "\"\"\n"),
    ("!custom\n", ". == \"\"", "true\n"),
    ("--- !custom\n", ".", "\"\"\n"),
    ("--- !custom\n", ". == \"\"", "true\n"),
    ("k: !!bool\nj: 1\n", ".", "{\"k\":false,\"j\":1}\n"),
    ("k: !!bool # c\nj: 1\n", ".", "{\"k\":false,\"j\":1}\n"),
    ("k: !!bool # c\nj: 1\n", ".k | length", "0\n"),
    ("- !!bool\n- 1\n", ".", "[false,1]\n"),
    ("!!bool\n", ".", "false\n"),
    ("--- !!bool\n", ".", "false\n"),
    ("k: !!null\nj: 1\n", ".", "{\"k\":null,\"j\":1}\n"),
    ("k: !!null\nj: 1\n", ".k == \"\"", "false\n"),
    ("k: !!null # c\nj: 1\n", ".", "{\"k\":null,\"j\":1}\n"),
    ("k: !!null # c\nj: 1\n", ".k | length", "0\n"),
    ("- !!null\n- 1\n", ".", "[null,1]\n"),
    ("- !!null\n- 1\n", ".[0] == \"\"", "false\n"),
    ("!!null\n", ".", "null\n"),
    ("!!null\n", ". == \"\"", "false\n"),
    ("--- !!null\n", ".", "null\n"),
    ("--- !!null\n", ". == \"\"", "false\n"),
    // The tag is the last text of the document (no trailing newline), a tagged explicit key stays
    // `null` as an untagged one does, and `!!null` and `!!bool` keep their own readings.
    ("k: !custom", ".", "{\"k\":\"\"}\n"),
    ("- !!seq", ".", "[\"\"]\n"),
    ("k: !!seq # c", ".", "{\"k\":\"\"}\n"),
    ("!!str", ".", "\"\"\n"),
    ("!!seq", ".", "\"\"\n"),
    ("--- !!seq", ".", "\"\"\n"),
    ("k: !!seq\nj:", ".", "{\"k\":\"\",\"j\":null}\n"),
    ("? !!seq", ".", "{\"\":null}\n"),
    ("? !!seq\n: 1", ".", "{\"\":1}\n"),
    ("? a", ".", "{\"a\":null}\n"),
    ("? a\n: !!seq", ".", "{\"a\":\"\"}\n"),
    ("- !!map\n- !!str", ".", "[\"\",\"\"]\n"),
    ("k: &a !!seq", ".", "{\"k\":\"\"}\n"),
    ("&a !custom", ".", "\"\"\n"),
    ("!!seq # c", ".", "\"\"\n"),
    ("k: !!bool", ".", "{\"k\":false}\n"),
    ("!!bool", ".", "false\n"),
    ("- !!bool", ".", "[false]\n"),
    ("k: !!null", ".", "{\"k\":null}\n"),
    ("!!null", ".", "null\n"),
];

#[test]
fn a_tag_with_no_value_is_the_empty_string_whatever_the_tag_6291() -> Result<()> {
    for (document, filter, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, document, &["-o", "json", "-I0"])?;
        assert_eq!(stdout, *expected, "{document:?} | {filter}");
        assert_eq!(code, 0, "{document:?} | {filter}");
    }
    Ok(())
}

/// `!!int` and `!!float` with no value: yq's JSON encoder fails (`strconv.ParseInt: parsing ""`), and this
/// reads the empty string, as it does `!!int abc` (the loader absorbs what it cannot read).
#[test]
fn a_valueless_int_or_float_tag_reads_the_empty_string_6291() -> Result<()> {
    for tag in ["!!int", "!!float"] {
        let document = format!("k: {tag}\nj: 1\n");
        let (stdout, code) = run_yq_stdin(".", &document, &["-o", "json", "-I0"])?;
        assert_eq!(stdout, "{\"k\":\"\",\"j\":1}\n", "{tag}");
        assert_eq!(code, 0, "{tag}");
    }
    Ok(())
}
