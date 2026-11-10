//! #6855: a plain scalar in a flow collection that is followed by a blank line before the closing
//! `]` or `}`, or before a `,` on a later line, ends at its last character: the run of line breaks is
//! what separates it from what follows, and no content of it. The scan stepped over a blank line's
//! two breaks at once and trimmed only spaces and tabs from the end, so `[42` / `` / `]` was the
//! string `"42\n"`, a `#` that opened the line after a blank line was scalar content, and an
//! explicit flow key kept the breaks before its `:`. Every answer was captured from yq v4.53.3
//! (`-o=json -I=0`); `None` is a document yq refuses. The last rows read right before and stay so.

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
