//! #6659: an owned array is a window on shared storage (ADR-0029). In `succinctly yq` that changes
//! no output; these rows pin the yq routes the change rewrote to read the window: `parent` through
//! a slice re-derives its ancestor by slicing again, and a `del()` batch of plain indices pads the
//! array through the window's `Vec`. Every row is captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "a: [1, 2, 3]\nb: {x: 1}\n";

#[test]
fn a_slice_and_a_padding_delete_answer_as_yq() -> Result<()> {
    for (filter, want) in [
        (".a[1:][0] | parent", "[2,3]"),
        (".a[1:] | .[0] | parent", "[2,3]"),
        (
            ".a[1:][0] | parent | parent",
            r#"{"a":[1,2,3],"b":{"x":1}}"#,
        ),
        (
            "del(.a[5], .a[7])",
            r#"{"a":[1,2,3,null,null,null],"b":{"x":1}}"#,
        ),
        ("del(.a[0], .a[5])", r#"{"a":[2,3,null,null],"b":{"x":1}}"#),
        (".a[1:]", "[2,3]"),
        (".a[1:][1:]", "[3]"),
        (".a[1:] as $t | $t | length", "2"),
    ] {
        let (out, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter}: {out}");
        assert_eq!(out.trim_end(), want, "{filter}");
    }
    Ok(())
}
