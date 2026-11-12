//! #7031: a `-p props` input's nodes have no style, as in yq, so a write prints its result with the
//! quoting yq's encoder picks for the new value (`a=1` with `.a |= . + "x"` is `a: 1x`), and `style`
//! reads `""`. The YAML text the input is read as records the quoting its transcode chose
//! (`a: "1"`), which a write used to keep. Every answer was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

const INPUT: &str = "a=1\nb=true\nc=hello\nd=two words\ne=x: y\ng=multi\\\n  line\nh=\\n\ni=007\n";

/// (filter, flags, yq's stdout).
const ROWS: &[(&str, &[&str], &str)] = &[
    (".a |= . + \"x\"", &["-o", "yaml"], "a: 1x\nb: \"true\"\nc: hello\nd: two words\ne: 'x: y'\ng: multiline\nh: |+\n\ni: \"007\"\n"),
    (".b |= . + \"x\"", &["-o", "yaml"], "a: \"1\"\nb: truex\nc: hello\nd: two words\ne: 'x: y'\ng: multiline\nh: |+\n\ni: \"007\"\n"),
    (".i |= . + \"x\"", &["-o", "yaml"], "a: \"1\"\nb: \"true\"\nc: hello\nd: two words\ne: 'x: y'\ng: multiline\nh: |+\n\ni: 007x\n"),
    (".e |= . + \"!\"", &["-o", "yaml"], "a: \"1\"\nb: \"true\"\nc: hello\nd: two words\ne: 'x: y!'\ng: multiline\nh: |+\n\ni: \"007\"\n"),
    (".[] |= . + \"\"", &["-o", "yaml"], "a: \"1\"\nb: \"true\"\nc: hello\nd: two words\ne: 'x: y'\ng: multiline\nh: |+\n\ni: \"007\"\n"),
    (".z = .a + \"y\"", &["-o", "yaml"], "a: \"1\"\nb: \"true\"\nc: hello\nd: two words\ne: 'x: y'\ng: multiline\nh: |+\n\ni: \"007\"\nz: 1y\n"),
    ("[.. | style]", &["-o", "json", "-I0"], "[\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\"]\n"),
    (".a | style", &["-o", "yaml"], "\n"),
    (".", &["-o", "yaml"], "a: \"1\"\nb: \"true\"\nc: hello\nd: two words\ne: 'x: y'\ng: multiline\nh: |+\n\ni: \"007\"\n"),
    (".", &["-P"], "a: \"1\"\nb: \"true\"\nc: hello\nd: two words\ne: 'x: y'\ng: multiline\nh: |+\n\ni: \"007\"\n"),
    ("with_entries(.value |= . + \"1\")", &["-o", "yaml"], "a: \"11\"\nb: true1\nc: hello1\nd: two words1\ne: 'x: y1'\ng: multiline1\nh: |-\n\n  1\ni: \"0071\"\n"),
];

#[test]
fn props_input_nodes_have_no_style_7031() -> Result<()> {
    for (filter, flags, expected) in ROWS {
        let mut args = vec!["-p", "props"];
        args.extend_from_slice(flags);
        let (stdout, status) = run_yq_stdin(filter, INPUT, &args)?;
        assert_eq!(
            (stdout.as_str(), status),
            (*expected, 0),
            "{flags:?} {filter}"
        );
    }
    Ok(())
}

/// Quoting decides the tag: the dates and `<<` stay strings, and a write, `-P` and `style=""` quote
/// them again as yq's encoder does (`a: "2001-12-14"`, `"<<": "1x"`), where the plain `1x` goes bare.
const QUOTED_INPUT: &str = "a=2001-12-14\nm=2001-12-14T21:59:43Z\nn=1\n<<=1\nc=hello\n";

const QUOTED_ROWS: &[(&str, &[&str], &str)] = &[
    (
        ".",
        &["-P"],
        "a: \"2001-12-14\"\nm: \"2001-12-14T21:59:43Z\"\nn: \"1\"\n\"<<\": \"1\"\nc: hello\n",
    ),
    ("keys", &["-P"], "- a\n- m\n- n\n- \"<<\"\n- c\n"),
    (
        ".z = 1",
        &["-P"],
        "a: \"2001-12-14\"\nm: \"2001-12-14T21:59:43Z\"\nn: \"1\"\n\"<<\": \"1\"\nc: hello\nz: 1\n",
    ),
    (
        ".a style=\"\"",
        &["-o", "yaml"],
        "a: \"2001-12-14\"\nm: \"2001-12-14T21:59:43Z\"\nn: \"1\"\n\"<<\": \"1\"\nc: hello\n",
    ),
    (
        ".[] style=\"\"",
        &["-o", "yaml"],
        "a: \"2001-12-14\"\nm: \"2001-12-14T21:59:43Z\"\nn: \"1\"\n\"<<\": \"1\"\nc: hello\n",
    ),
    (
        ".n tag = \"!!str\"",
        &["-o", "yaml"],
        "a: \"2001-12-14\"\nm: \"2001-12-14T21:59:43Z\"\nn: \"1\"\n\"<<\": \"1\"\nc: hello\n",
    ),
    (
        "[.a, .m, .n, .c] | map(tag)",
        &["-o", "json", "-I0"],
        "[\"!!str\",\"!!str\",\"!!str\",\"!!str\"]\n",
    ),
    (".a | downcase", &["-o", "yaml"], "2001-12-14\n"),
    (
        ".z = 1",
        &["-o", "yaml"],
        "a: \"2001-12-14\"\nm: \"2001-12-14T21:59:43Z\"\nn: \"1\"\n\"<<\": \"1\"\nc: hello\nz: 1\n",
    ),
    (
        ".n |= . + \"x\"",
        &["-o", "yaml"],
        "a: \"2001-12-14\"\nm: \"2001-12-14T21:59:43Z\"\nn: 1x\n\"<<\": \"1\"\nc: hello\n",
    ),
    ("keys", &["-o", "yaml"], "- a\n- m\n- n\n- \"<<\"\n- c\n"),
    (
        ".[\"<<\"] |= . + \"x\"",
        &["-o", "yaml"],
        "a: \"2001-12-14\"\nm: \"2001-12-14T21:59:43Z\"\nn: \"1\"\n\"<<\": 1x\nc: hello\n",
    ),
    (
        "sort_keys(.)",
        &["-o", "yaml"],
        "\"<<\": \"1\"\na: \"2001-12-14\"\nc: hello\nm: \"2001-12-14T21:59:43Z\"\nn: \"1\"\n",
    ),
];

#[test]
fn props_input_quoting_still_decides_the_tag_7031() -> Result<()> {
    for (filter, flags, expected) in QUOTED_ROWS {
        let mut args = vec!["-p", "props"];
        args.extend_from_slice(flags);
        let (stdout, status) = run_yq_stdin(filter, QUOTED_INPUT, &args)?;
        assert_eq!(
            (stdout.as_str(), status),
            (*expected, 0),
            "{flags:?} {filter}"
        );
    }
    Ok(())
}
