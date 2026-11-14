//! #7202: a document read with `-p xml` and written as XML (`-o xml`, `-i` on a `*.xml` file) has the
//! comments before its first element as that first key's head comment, as yq's decoder gave them. The
//! YAML text the read hands the run holds them as the document's leading block, which the XML encoder
//! wrote through a comment's marker rules and so trimmed a first line's trailing blank (`<!-- ` became
//! `<!--`), the commonest difference of a real-file sweep. Every row was captured from yq (stdout, the
//! first stderr line, the exit status).

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

/// (filter, stdin, yq's stdout, yq's first stderr line, yq's exit status).
type Row = (&'static str, &'static str, &'static str, &'static str, i32);

#[rustfmt::skip]
const ROWS: &[Row] = &[
    (".", "<?xml version=\"1.0\"?>\n<!-- \nfoo\nbar\n-->\n<a>1</a>\n", "<?xml version=\"1.0\"?>\n<!-- \nfoo\nbar\n-->\n<a>1</a>\n", "", 0),
    (".", "<!-- \nfoo\nbar\n-->\n<a>1</a>\n", "<!-- \nfoo\nbar\n-->\n<a>1</a>\n", "", 0),
    (".", "<!--x  -->\n<a>1</a>\n", "<!-- x -->\n<a>1</a>\n", "", 0),
    (".", "<!--x-->\n<!--y -->\n<a>1</a>\n", "<!-- x y -->\n<a>1</a>\n", "", 0),
    (".", "<?xml version=\"1.0\"?>\n<!-- a --><!-- b  \n c -->\n<a><b>1</b></a>\n", "<?xml version=\"1.0\"?>\n<!-- a   b  \nc -->\n<a>\n  <b>1</b>\n</a>\n", "", 0),
    (".a.b = 2", "<?xml version=\"1.0\"?>\n<!-- \nfoo\n-->\n<a><b>1</b></a>\n", "<?xml version=\"1.0\"?>\n<!-- \nfoo\n-->\n<a>\n  <b>2</b>\n</a>\n", "", 0),
    ("del(.a)", "<?xml version=\"1.0\"?>\n<!-- \nfoo\n-->\n<a><b>1</b></a>\n", "<?xml version=\"1.0\"?>\n<!-- \nfoo\n-->\n\n", "", 0),
    (".", "<!--a  \nb-->\n<?pi x?>\n<r/>\n", "<!-- a  \nb -->\n<?pi x?>\n<r></r>\n", "", 0),
];

#[test]
fn xml_written_back_as_xml_keeps_the_first_comment_as_the_first_keys_7202() -> Result<()> {
    for &(filter, input, stdout, stderr, code) in ROWS {
        let (out, err, status) =
            run_yq_stdin_with_stderr(filter, input, &["-p", "xml", "-o", "xml"])?;
        let row = format!("-p xml -o xml {filter:?} on {input:?}");
        assert_eq!(out, stdout, "stdout of {row}");
        assert_eq!(err.lines().next().unwrap_or(""), stderr, "stderr of {row}");
        assert_eq!(status, code, "exit status of {row}");
    }
    Ok(())
}
