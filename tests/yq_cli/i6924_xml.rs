//! #6924: `succinctly yq -o xml`, a port of yq's XML encoder over Go's `encoding/xml` printer: a key is an
//! element, a sequence repeats it, `+@name` keys are attributes, `+content` the text, `+p_target` a
//! processing instruction, `+directive` a directive, `-I` spaces of indent. Every answer was captured from
//! yq v4.53.3; a row is kept only when both agree on stdout and the exit status. Left out because they
//! still differ (see `docs/compliance/yq/limitations.md`): a comment, a null written `~`, a boolean written
//! `True` and an empty document.

use anyhow::Result;

use crate::run_yq_stdin;

/// (flags, input, yq's stdout, yq's exit status).
const ROWS: &[(&[&str], &str, &str, i32)] = &[
    (&[], "a: 1\n", "<a>1</a>\n", 0),
    (
        &[],
        "a: {b: {c: 1}}\n",
        "<a>\n  <b>\n    <c>1</c>\n  </b>\n</a>\n",
        0,
    ),
    (&[], "a: [1, 2]\n", "<a>1</a>\n<a>2</a>\n", 0),
    (
        &[],
        "a: [[1, 2], [3]]\n",
        "<a>1</a>\n<a>2</a>\n<a>3</a>\n",
        0,
    ),
    (&[], "a: []\n", "\n", 0),
    (&[], "a: {}\n", "<a></a>\n", 0),
    (&[], "a: \"\"\nb: null\n", "<a></a>\n<b>null</b>\n", 0),
    (
        &[],
        "a: \"x<y&\\\"z'>\\t\\nq\\r\"\n",
        "<a>x&lt;y&amp;&#34;z&#39;&gt;&#x9;\nq&#xD;</a>\n",
        0,
    ),
    (
        &[],
        "\"+@id\": \"1\"\na: 2\n",
        "<+@id>1</+@id>\n<a>2</a>\n",
        0,
    ),
    (
        &[],
        "a: {\"+@id\": \"1\", \"+content\": \"txt\"}\n",
        "<a id=\"1\">txt</a>\n",
        0,
    ),
    (
        &[],
        "a: {\"+@id\": \"1\", b: 2}\n",
        "<a id=\"1\">\n  <b>2</b>\n</a>\n",
        0,
    ),
    (&[], "a: {\"+@id\": [1]}\n", "", 1),
    (
        &[],
        "\"+p_xml\": \"version=\\\"1.0\\\"\"\na: 1\n",
        "<?xml version=\"1.0\"?>\n<a>1</a>\n",
        0,
    ),
    (
        &[],
        "a: 1\n\"+p_xml\": \"version=\\\"1.0\\\" encoding=\\\"UTF-8\\\"\"\n",
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<a>1</a>\n",
        0,
    ),
    (&[], "a: {\"+p_pi\": \"x y\"}\n", "<a><?pi x y?></a>\n", 0),
    (
        &[],
        "\"+directive\": \"DOCTYPE a\"\na: 1\n",
        "<!DOCTYPE a>\n<a>1</a>\n",
        0,
    ),
    (&[], "a: {\"+directive\": \"x\"}\n", "<a><!x></a>\n", 0),
    (&[], "\"\": 1\n", "", 1),
    (&[], "a b: 1\n", "<a b>1</a b>\n", 0),
    (&[], "a: {\"\": 1}\n", "", 1),
    (
        &[],
        "a: {b: [{c: 1}, {d: 2}]}\n",
        "<a>\n  <b>\n    <c>1</c>\n  </b>\n  <b>\n    <d>2</d>\n  </b>\n</a>\n",
        0,
    ),
    (
        &[],
        "a: 1\nb: {c: x}\nd: [1, {e: 2}]\n",
        "<a>1</a>\n<b>\n  <c>x</c>\n</b>\n<d>1</d>\n<d>\n  <e>2</e>\n</d>\n",
        0,
    ),
    (&[], "a: \"\\u0001x\\ufffe\"\n", "<a>�x�</a>\n", 0),
    (&[], "[1]\n", "", 1),
    (&[], "hello\n", "hello", 0),
    (&[], "3\n", "3", 0),
    (&[], "{}\n", "\n", 0),
    (
        &[],
        "a: {\"+content\": x, b: 1}\n",
        "<a>x\n  <b>1</b>\n</a>\n",
        0,
    ),
    (
        &[],
        "a: {b: 1, \"+content\": x}\n",
        "<a>\n  <b>1</b>x\n</a>\n",
        0,
    ),
    (&[], "a: {\"+content\": [x, y]}\n", "<a></a>\n", 0),
    (&[], "x:y: 1\n", "<x:y>1</x:y>\n", 0),
    (&[], "a: {\"+@x:y\": \"1\"}\n", "<a x:y=\"1\"></a>\n", 0),
    (&[], "\"+@id\": \"1\"\n", "<+@id>1</+@id>\n", 0),
    (&[], "\"+content\": x\n", "<+content>x</+content>\n", 0),
    (&[], "a: 1\n---\nb: 2\n", "<a>1</a>\n<b>2</b>\n", 0),
    (
        &["-I4"],
        "a: {b: [1, {c: 2}]}\n",
        "<a>\n    <b>1</b>\n    <b>\n        <c>2</c>\n    </b>\n</a>\n",
        0,
    ),
    (
        &["-I0"],
        "a: {b: [1, {c: 2}]}\n",
        "<a><b>1</b><b><c>2</c></b></a>\n",
        0,
    ),
    (&["-I1"], "a: {b: 1}\n", "<a>\n <b>1</b>\n</a>\n", 0),
    (
        &["--xml-attribute-prefix=@"],
        "a: {\"@id\": \"1\", b: 2}\n",
        "<a id=\"1\">\n  <b>2</b>\n</a>\n",
        0,
    ),
    (
        &["--xml-content-name=#text"],
        "a: {\"#text\": x, \"+@id\": 1}\n",
        "<a id=\"1\">x</a>\n",
        0,
    ),
    (
        &["--xml-proc-inst-prefix=?"],
        "\"?xml\": \"version=\\\"1.0\\\"\"\na: 1\n",
        "<?xml version=\"1.0\"?>\n<a>1</a>\n",
        0,
    ),
    (
        &["--xml-directive-name=!"],
        "\"!\": \"DOCTYPE a\"\na: 1\n",
        "<!DOCTYPE a>\n<a>1</a>\n",
        0,
    ),
    (&[], "\"+p_a\": \"x?>y\"\nb: 1\n", "", 1),
    (&[], "\"+directive\": \"<x\"\nb: 1\n", "", 1),
    (&[], "\"+p_1x\": \"y\"\nb: 1\n", "", 1),
    (&[], "a: {\"+p_xml\": \"v\"}\n", "", 1),
];

#[test]
fn xml_rows_match_yq_6924() -> Result<()> {
    for (flags, input, expected, code) in ROWS {
        let mut args = vec!["-o=xml"];
        args.extend(flags.iter().copied());
        let (stdout, status) = run_yq_stdin(".", input, &args)?;
        assert_eq!(
            (stdout.as_str(), status),
            (*expected, *code),
            "-o=xml {flags:?} over {input:?}"
        );
    }
    Ok(())
}

#[test]
fn xml_short_name_6924() -> Result<()> {
    let one = run_yq_stdin(".", "a: 1\n", &["-o=x"])?;
    let other = run_yq_stdin(".", "a: 1\n", &["-o=xml"])?;
    assert_eq!(one, other);
    Ok(())
}
