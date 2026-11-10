//! #6855: a plain scalar in a flow collection that is followed by a blank line before the closing
//! `]` or `}`, or before a `,` on a later line, ends at its last character: the run of line breaks is
//! what separates it from what follows, and no content of it. The scan stepped over a blank line's
//! two breaks at once and trimmed only spaces and tabs from the end, so `[42` / `` / `]` was the
//! string `"42\n"`, a `#` that opened the line after a blank line was scalar content, and an
//! explicit flow key kept the breaks before its `:`. Every answer was captured from yq v4.53.3
//! (`-o=json -I=0`); `None` is a document yq refuses. The rows after the first block read right before and stay so.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, yq's answer or `None` for a refusal).
const ROWS: &[(&str, Option<&str>)] = &[
    ("[42\n\n]", Some("[42]")),
    ("[a\n\n]", Some("[\"a\"]")),
    ("[true\n\n]", Some("[true]")),
    ("[null\n\n]", Some("[null]")),
    ("[-1\n\n]", Some("[-1]")),
    ("{a: 42\n\n}", Some("{\"a\":42}")),
    ("{a: true\n\n}", Some("{\"a\":true}")),
    ("{a: null\n\n}", Some("{\"a\":null}")),
    ("{a: 4.5\n\n}", Some("{\"a\":4.5}")),
    ("{a: x y\n\n}", Some("{\"a\":\"x y\"}")),
    ("{a: x\n  y\n\n}", Some("{\"a\":\"x y\"}")),
    ("{a: 42\n\n\n\n}", Some("{\"a\":42}")),
    ("{a: 42\n \n}", Some("{\"a\":42}")),
    ("{a: 42\r\n\r\n}", Some("{\"a\":42}")),
    ("[42\r\n\r\n]", Some("[42]")),
    ("[1,\n 42\n\n]", Some("[1,42]")),
    ("{a: 1,\nb: 42\n\n}", Some("{\"a\":1,\"b\":42}")),
    (
        "{ ?foo: bar,\nbar: 42\n\n}",
        Some("{\"?foo\":\"bar\",\"bar\":42}"),
    ),
    ("{a: 42\n\n,b: 1}", Some("{\"a\":42,\"b\":1}")),
    ("[ a\n\n , b ]", Some("[\"a\",\"b\"]")),
    ("[a b\n\n c\n\n]", Some("[\"a b\\nc\"]")),
    ("k: {a: 42\n\n  }\n", Some("{\"k\":{\"a\":42}}")),
    ("- {a: 42\n\n  }\n", Some("[{\"a\":42}]")),
    ("a:\n  b: [1\n\n  ]\n", Some("{\"a\":{\"b\":[1]}}")),
    (
        "{a: [1\n\n], b: {c: 2\n\n}}",
        Some("{\"a\":[1],\"b\":{\"c\":2}}"),
    ),
    ("[42\n\n# c\n]", Some("[42]")),
    ("[42\n\n  # c\n]", Some("[42]")),
    ("{a: 42\n\n# c\n\n}", Some("{\"a\":42}")),
    ("{ ? a\n\n : b}", Some("{\"a\":\"b\"}")),
    ("[ ? a\n\n : b ]", Some("[{\"a\":\"b\"}]")),
    // NEL, LS and PS are line breaks to go-yaml: a trailing one is trimmed like `\n`; a `#` glued to text stays content; refusals
    ("[42\u{2028}\n\n]", Some("[42]")),
    ("{a: 42\u{85}\n\n}", Some("{\"a\":42}")),
    ("[42\n\u{2028}\n]", Some("[42]")),
    ("[42\n\u{85}\n]", Some("[42]")),
    ("[a\n\n\u{2028}]", Some("[\"a\"]")),
    ("[42\u{2029}]", Some("[42]")),
    ("{a: 42\u{2028}}", Some("{\"a\":42}")),
    ("[a\u{2028}b]", Some("[\"a\\u2028b\"]")),
    ("[a\u{2028}\n b]", Some("[\"a\\u2028\\nb\"]")),
    ("[a\u{85}b\n\n]", Some("[\"a b\"]")),
    ("{a: x\u{2028}\n\n, b: 1}", Some("{\"a\":\"x\",\"b\":1}")),
    ("[a\n b#c\n\n]", Some("[\"a b#c\"]")),
    ("[a\n\n#c b]", None),
    ("[a\n#c\n b]", None),
    ("{a: 42\n\n  b: 1}", None),
    ("{a\n\n: 1}", None),
    ("{a: 42\n}", Some("{\"a\":42}")),
    ("{a: 42,\n\n}", Some("{\"a\":42}")),
    ("{a: 42 # c\n\n}", Some("{\"a\":42}")),
    ("{a: 42\n# c\n\n}", Some("{\"a\":42}")),
    ("{a: 'q'\n\n}", Some("{\"a\":\"q\"}")),
    ("[a,\n\n b]", Some("[\"a\",\"b\"]")),
    ("{a: x\n\n y}", Some("{\"a\":\"x\\ny\"}")),
    ("{a: x\n \n y}", Some("{\"a\":\"x\\ny\"}")),
    ("[a\n\n b]", Some("[\"a\\nb\"]")),
    ("[1,\n\n 2]", Some("[1,2]")),
];

#[test]
fn a_flow_plain_scalar_ends_at_its_last_character_before_a_blank_line_6855() -> Result<()> {
    for &(input, expected) in ROWS {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        match expected {
            Some(want) => {
                assert_eq!(code, 0, "{input:?}: {out}");
                assert_eq!(out.trim_end(), want, "{input:?}");
            }
            None => {
                assert_ne!(code, 0, "{input:?}: {out}");
                assert_eq!(out.trim_end(), "", "{input:?}");
            }
        }
    }
    Ok(())
}

/// (document, filter, extra flags, yq's YAML text): the same scalars on the YAML output and write routes.
const YAML_ROWS: &[(&str, &str, &[&str], &str)] = &[
    ("{a: 42\n\n}", ".", &[], "{a: 42}\n"),
    ("[42\n\n]", ".", &[], "[42]\n"),
    ("k: {a: 42\n\n  }\n", ".", &[], "k: {a: 42}\n"),
    ("{a: 42\n\n}", ".a = 9", &[], "{a: 9}\n"),
    ("k: {a: 42\n\n  }\n", ".a = 9", &[], "k: {a: 42}\na: 9\n"),
    ("[1,\n 42\n\n]", ".", &["-P"], "- 1\n- 42\n"),
    ("{a: x\n  y\n\n}", ".", &[], "{a: x y}\n"),
];

#[test]
fn a_flow_plain_scalar_before_a_blank_line_is_the_same_scalar_in_yaml_output_6855() -> Result<()> {
    for &(input, filter, flags, expected) in YAML_ROWS {
        let (out, code) = run_yq_stdin(filter, input, flags)?;
        assert_eq!(code, 0, "{input:?} {filter}: {out}");
        assert_eq!(out, expected, "{input:?} {filter}");
    }
    Ok(())
}
