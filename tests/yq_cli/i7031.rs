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
