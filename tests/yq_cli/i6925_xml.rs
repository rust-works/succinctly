//! #6925: `succinctly yq -p xml`, yq's `decoder_xml.go` over Go's `encoding/xml`, read as the YAML text
//! yq prints for the same nodes. Every answer was captured from yq v4.53.3 (its `-p` deprecation
//! warning on stderr left out), except the two rows that name yq's Go panic: `</a><b/>` makes yq
//! dereference a nil element and die, and this reads it as an error (`docs/compliance/yq/limitations.md`,
//! `-p xml`). The rest of that section lists what still differs: the blank line yq's node printer
//! writes around a comment, the indent of a comment after a nested block, and the multi-byte charsets.
//!
//! Sweep: `scripts/yq-xml-input-sweep.py` (random documents and their mutations, three outputs).

use std::io::Write;

use anyhow::Result;
use tempfile::TempDir;

use crate::{run_yq_files, run_yq_stdin_with_stderr};

/// (stdin, extra args, filter, yq's stdout, yq's first stderr line, yq's exit status).
type Row = (
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
    &'static str,
    i32,
);

/// The error line of `stderr`: a run with `RUST_BACKTRACE` set (the coverage job) adds anyhow's
/// backtrace after it.
fn error_line(stderr: &str) -> &str {
    stderr.lines().next().unwrap_or("")
}

fn write_file(dir: &TempDir, name: &str, bytes: &[u8]) -> Result<std::path::PathBuf> {
    let path = dir.path().join(name);
    std::fs::File::create(&path)?.write_all(bytes)?;
    Ok(path)
}

#[rustfmt::skip]
const ROWS: &[Row] = &[
    ("<a>1</a>", &["-o", "json", "-I0"], ".", "{\"a\":\"1\"}\n", "", 0),
    ("<a x=\"1\" y='2'>hi<b>2</b></a>", &["-o", "json", "-I0"], ".", "{\"a\":{\"+content\":\"hi\",\"+@x\":\"1\",\"+@y\":\"2\",\"b\":\"2\"}}\n", "", 0),
    ("<a><b>1</b><b>2</b><c/></a>", &["-o", "json", "-I0"], ".", "{\"a\":{\"b\":[\"1\",\"2\"],\"c\":null}}\n", "", 0),
    ("<a>x<b>1</b>y</a>", &["-o", "json", "-I0"], ".", "{\"a\":{\"+content\":[\"x\",\"y\"],\"b\":\"1\"}}\n", "", 0),
    ("<a>  hi  </a>", &["-o", "json", "-I0"], ".", "{\"a\":\"hi\"}\n", "", 0),
    ("<a> <b/> </a>", &["-o", "json", "-I0"], ".", "{\"a\":{\"b\":null}}\n", "", 0),
    ("<a>\u{200b}zw\u{200b}\u{a0}nbsp\u{a0}</a>", &["-o", "json", "-I0"], ".", "{\"a\":\"zw\u{200b}\u{a0}nbsp\"}\n", "", 0),
    ("<a>true</a><b>null</b><c>0x1f</c>", &["-o", "json", "-I0"], ".", "{\"a\":\"true\",\"b\":\"null\",\"c\":\"0x1f\"}\n", "", 0),
    ("<a>&lt;&amp;&gt;&apos;&quot;&#65;&#x42;</a>", &["-o", "json", "-I0"], ".", "{\"a\":\"<&>'\\\"AB\"}\n", "", 0),
    ("<a>&nbsp;&bogus;&amp</a>", &["-o", "json", "-I0"], ".", "{\"a\":\"&nbsp;&bogus;&amp\"}\n", "", 0),
    ("<a><![CDATA[x<y]]></a>", &["-o", "json", "-I0"], ".", "{\"a\":\"x<y\"}\n", "", 0),
    ("<a><![CDATA[a]b]]>t<![CDATA[c]]></a>", &["-o", "json", "-I0"], ".", "{\"a\":[\"a]b\",\"t\",\"c\"]}\n", "", 0),
    ("<a>a\r\nb\rc</a>", &["-o", "json", "-I0"], ".", "{\"a\":\"a\\nb\\nc\"}\n", "", 0),
    ("<?xml version=\"1.0\"?><?php echo 1 ?><a/>", &["-o", "json", "-I0"], ".", "{\"+p_xml\":\"version=\\\"1.0\\\"\",\"+p_php\":\"echo 1 \",\"a\":null}\n", "", 0),
    ("<!DOCTYPE a><a/>", &["-o", "json", "-I0"], ".", "{\"+directive\":\"DOCTYPE a\",\"a\":null}\n", "", 0),
    ("<!DOCTYPE a [<!ENTITY e \"v\">]><a/>", &["-o", "json", "-I0"], ".", "{\"+directive\":\"DOCTYPE a [<!ENTITY e \\\"v\\\">]\",\"a\":null}\n", "", 0),
    ("<a><?p x?><?p y?></a>", &["-o", "json", "-I0"], ".", "{\"a\":{\"+p_p\":[\"x\",\"y\"]}}\n", "", 0),
    ("<?xml version=\"1.0\"?><a/>", &["-o", "json", "-I0", "--xml-skip-proc-inst"], ".", "{\"a\":null}\n", "", 0),
    ("<!DOCTYPE a><a/>", &["-o", "json", "-I0", "--xml-skip-directives"], ".", "{\"a\":null}\n", "", 0),
    ("<a x=\"1\">t</a>", &["-o", "json", "-I0", "--xml-attribute-prefix=@", "--xml-content-name=#text"], ".", "{\"a\":{\"#text\":\"t\",\"@x\":\"1\"}}\n", "", 0),
    ("<?p x?><!DOCTYPE a><a/>", &["-o", "json", "-I0", "--xml-proc-inst-prefix=?", "--xml-directive-name=!"], ".", "{\"?p\":\"x\",\"!\":\"DOCTYPE a\",\"a\":null}\n", "", 0),
    ("<a xmlns:n=\"u\" n:c=\"1\"><n:b>v</n:b></a>", &["-o", "json", "-I0"], ".", "{\"a\":{\"+@xmlns:n\":\"u\",\"+@n:c\":\"1\",\"n:b\":\"v\"}}\n", "", 0),
    ("<a xmlns:n=\"u\" n:c=\"1\"><n:b>v</n:b></a>", &["-o", "json", "-I0", "--xml-keep-namespace=false"], ".", "{\"a\":{\"+@n\":\"u\",\"+@c\":\"1\",\"b\":\"v\"}}\n", "", 0),
    ("<a xmlns:n=\"u\" n:c=\"1\"><n:b>v</n:b></a>", &["-o", "json", "-I0", "--xml-raw-token=false"], ".", "{\"a\":{\"+@xmlns:n\":\"u\",\"+@u:c\":\"1\",\"u:b\":\"v\"}}\n", "", 0),
    ("<a xml:lang=\"en\"/>", &["-o", "json", "-I0", "--xml-raw-token=false"], ".", "{\"a\":{\"+@http://www.w3.org/XML/1998/namespace:lang\":\"en\"}}\n", "", 0),
    ("<a b=c d/>", &["-o", "json", "-I0"], ".", "{\"a\":{\"+@b\":\"c\",\"+@d\":\"d\"}}\n", "", 0),
    ("<a b=c/>", &["-o", "json", "-I0", "--xml-strict-mode"], ".", "", "Error: bad file '-': XML syntax error on line 1: unquoted or missing attribute value in element", 1),
    ("<a></b>", &["-o", "json", "-I0"], ".", "{\"a\":null}\n", "", 0),
    ("<a><b></a>", &["-o", "json", "-I0"], ".", "null\n", "", 0),
    ("<a></b>", &["-o", "json", "-I0", "--xml-raw-token=false"], ".", "", "Error: bad file '-': XML syntax error on line 1: unexpected end element </b>", 1),
    ("<a></b>", &["-o", "json", "-I0", "--xml-raw-token=false", "--xml-strict-mode"], ".", "", "Error: bad file '-': XML syntax error on line 1: element <a> closed by </b>", 1),
    ("<a>&bogus;</a>", &["-o", "json", "-I0", "--xml-strict-mode"], ".", "", "Error: bad file '-': XML syntax error on line 1: invalid character entity &bogus;", 1),
    ("<a>", &["-o", "json", "-I0"], ".", "null\n", "", 0),
    ("<a>", &["-o", "json", "-I0", "--xml-raw-token=false"], ".", "", "Error: bad file '-': XML syntax error on line 1: unexpected EOF", 1),
    ("<a/>trailing", &["-o", "json", "-I0"], ".", "{\"+content\":\"trailing\",\"a\":null}\n", "", 0),
    ("<a/><b/>", &["-o", "json", "-I0"], ".", "{\"a\":null,\"b\":null}\n", "", 0),
    ("", &["-o", "json", "-I0"], ".", "null\n", "", 0),
    ("   \n", &["-o", "json", "-I0"], ".", "null\n", "", 0),
    ("<!--c-->", &["-o", "json", "-I0"], ".", "null\n", "", 0),
    ("<!--c-->x", &["-o", "json", "-I0"], ".", "\"x\"\n", "", 0),
    ("text", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': invalid XML: Encountered chardata [text] outside of XML node", 1),
    ("<a/>", &["-o", "json", "-I0"], ".a | tag", "\"!!null\"\n", "", 0),
    ("<a>1</a>", &["-o", "json", "-I0"], ".a | tag", "\"!!str\"\n", "", 0),
    ("<a>1</a>", &["-o", "json", "-I0"], ".a | . == \"1\"", "true\n", "", 0),
    ("<a><b>1</b><b>2</b></a>", &["-o", "json", "-I0"], ".a.b | length", "2\n", "", 0),
    ("<a x=\"1\"/>", &["-o", "json", "-I0"], ".a.\"+@x\"", "\"1\"\n", "", 0),
    ("<a><b>1</b></a>", &["-o", "xml"], ".", "<a>\n  <b>1</b>\n</a>\n", "", 0),
    ("<a><b/><c x=\"1\"/><d>t</d></a>", &["-o", "xml"], ".", "<a>\n  <b></b>\n  <c x=\"1\"></c>\n  <d>t</d>\n</a>\n", "", 0),
    ("<a><b>1</b></a>", &["-o", "xml"], ".a.b = \"2\"", "<a>\n  <b>2</b>\n</a>\n", "", 0),
    ("<a><b>1</b></a>", &["-o", "props"], ".", "a.b = 1\n", "", 0),
    ("<a><b>1</b><b>2</b></a>", &["-o", "yaml"], ".", "a:\n  b:\n    - \"1\"\n    - \"2\"\n", "", 0),
    ("<a><b>1</b><b>2</b></a>", &["-o", "csv"], ".a.b", "1,2\n", "", 0),
    ("<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a>é</a>", &["-o", "json", "-I0"], ".", "{\"+p_xml\":\"version=\\\"1.0\\\" encoding=\\\"ISO-8859-1\\\"\",\"a\":\"Ã©\"}\n", "", 0),
    ("<?xml version=\"1.0\" encoding=\"windows-1251\"?><a>é</a>", &["-o", "json", "-I0"], ".", "{\"+p_xml\":\"version=\\\"1.0\\\" encoding=\\\"windows-1251\\\"\",\"a\":\"Г©\"}\n", "", 0),
    ("<?xml version=\"1.0\" encoding=\"utf8\"?><a>é</a>", &["-o", "json", "-I0"], ".", "{\"+p_xml\":\"version=\\\"1.0\\\" encoding=\\\"utf8\\\"\",\"a\":\"é\"}\n", "", 0),
    ("<?xml version=\"1.0\" encoding=\"UTF-8\"?><a>é</a>", &["-o", "json", "-I0"], ".", "{\"+p_xml\":\"version=\\\"1.0\\\" encoding=\\\"UTF-8\\\"\",\"a\":\"é\"}\n", "", 0),
    ("<?xml version=\"1.0\" encoding=\"x-nope\"?><a/>", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': xml: opening charset \"x-nope\": unsupported charset: \"x-nope\"", 1),
    ("<?xml version=\"1.1\"?><a/>", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': xml: unsupported version \"1.1\"; only version 1.0 is supported", 1),
    ("<?xml version=\"1.0\" encoding=\"\"?><a/>", &["-o", "json", "-I0"], ".", "{\"+p_xml\":\"version=\\\"1.0\\\" encoding=\\\"\\\"\",\"a\":null}\n", "", 0),
    ("<a b=\"1", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': XML syntax error on line 1: unexpected EOF", 1),
    ("<a b='1' b2=", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': XML syntax error on line 1: unexpected EOF", 1),
    ("<a><![CDATA[x", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': XML syntax error on line 1: unexpected EOF in CDATA section", 1),
    ("<!-- x", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': XML syntax error on line 1: unexpected EOF", 1),
    ("<a><!-- x -- y --></a>", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': XML syntax error on line 1: invalid sequence \"--\" not allowed in comments", 1),
    ("<a><!- x></a>", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': XML syntax error on line 1: invalid sequence <!- not part of <!--", 1),
    ("<a><![XDATA[x]]></a>", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': XML syntax error on line 1: invalid <![ sequence", 1),
    ("<a>x ]]> y</a>", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': XML syntax error on line 1: unescaped ]]> not in CDATA section", 1),
    ("<a>\u{1}</a>", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': XML syntax error on line 1: illegal character code U+0001", 1),
    ("< a/>", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': XML syntax error on line 1: expected element name after <", 1),
    ("<1a/>", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': XML syntax error on line 1: invalid XML name: 1a", 1),
    ("<a:b:c/>", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': XML syntax error on line 1: expected element name after <", 1),
    ("</>", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': XML syntax error on line 1: expected element name after </", 1),
    ("<a/ >", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': XML syntax error on line 1: expected /> in element", 1),
    ("<a>&#x110000;&#;&#xZ;&#65</a>", &["-o", "json", "-I0"], ".", "{\"a\":\"&#x110000;&#;&#xZ;&#65\"}\n", "", 0),
    ("<?xml?><a/>", &["-o", "json", "-I0"], ".", "{\"+p_xml\":\"\",\"a\":null}\n", "", 0),
    ("<? ?><a/>", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': XML syntax error on line 1: expected target name after <?", 1),
    ("<a/>\n</a>", &["-o", "json", "-I0"], ".", "{\"a\":null}\n", "", 0),
    ("</a><b/>", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': invalid XML: markup after the end of the document's root", 1),
    ("<a/></a>x<b/>", &["-o", "json", "-I0"], ".", "", "Error: bad file '-': invalid XML: markup after the end of the document's root", 1),
    ("<a><!--h--><b>1</b></a>", &["-o", "yaml"], ".", "a:\n  # h\n  b: \"1\"\n", "", 0),
    ("<a><b>1</b><!--f--></a>", &["-o", "yaml"], ".", "a:\n  b: \"1\"\n  # f\n", "", 0),
    ("<a><b>1<!--l--></b></a>", &["-o", "yaml"], ".", "a:\n  b: \"1\" # l\n", "", 0),
    ("<a><b>1</b><!--f--><c>2</c></a>", &["-o", "yaml"], ".", "a:\n  b: \"1\"\n  # f\n\n  c: \"2\"\n", "", 0),
    ("<a><b/><!--f--></a>", &["-o", "yaml"], ".", "a:\n  b:\n  # f\n", "", 0),
    ("<!--top--><a>1</a><!--end-->", &["-o", "yaml"], ".", "# top\na: \"1\"\n# end\n", "", 0),
    ("<a><b><c>1</c></b><!--f--></a>", &["-o", "yaml"], ".", "a:\n  b:\n    c: \"1\"\n  # f\n", "", 0),
    ("<a><!--h--><b>x</b><b>y</b></a>", &["-o", "yaml"], ".", "a:\n  # h\n  b:\n    - x\n    - y\n", "", 0),
    ("<a><b>x</b><b>y</b><!--f--></a>", &["-o", "yaml"], ".", "a:\n  b:\n    - x\n    - y\n    # f\n", "", 0),
    ("<a><b>x</b><!--f1--><!--f2--></a>", &["-o", "yaml"], ".", "a:\n  b: x\n  # f1 f2\n", "", 0),
    ("<a><b><!--h2-->x</b></a>", &["-o", "yaml"], ".", "a:\n  # h2\n  b: x\n", "", 0),
    ("<a><b><!--h--></b></a>", &["-o", "yaml"], ".", "a:\n  b: # h\n", "", 0),
    ("<a>x<!--l-->y<b/></a>", &["-o", "yaml"], ".", "a:\n  +content: # l\n    - x\n    - y\n  b:\n", "", 0),
    ("<a>x<!--l--><b/></a>", &["-o", "yaml"], ".", "a:\n  +content: x # l\n  b:\n", "", 0),
    ("<a><b>1</b><b>2</b><!--f--></a>", &["-o", "yaml"], ".", "a:\n  b:\n    - \"1\"\n    - \"2\"\n    # f\n", "", 0),
    ("<a><!-- spaced  --><b>1</b></a>", &["-o", "yaml"], ".", "a:\n  # spaced\n  b: \"1\"\n", "", 0),
    ("<a><!--l1\nl2--><b>1</b></a>", &["-o", "yaml"], ".", "a:\n  # l1\n  # l2\n  b: \"1\"\n", "", 0),
    ("<a><b><c/></b><b><c/></b></a>", &["-o", "yaml"], ".", "a:\n  b:\n    - c:\n    - c:\n", "", 0),
    ("<a><b><!--h--><c>1</c></b><b><c>2</c></b></a>", &["-o", "yaml"], ".", "a:\n  b:\n    - # h\n      c: \"1\"\n    - c: \"2\"\n", "", 0),
    ("<a><b>x<c/></b><b>y</b></a>", &["-o", "yaml"], ".", "a:\n  b:\n    - +content: x\n      c:\n    - y\n", "", 0),
    ("<a><b>x</b><b>y<c/></b></a>", &["-o", "yaml"], ".", "a:\n  b:\n    - x\n    - +content: y\n      c:\n", "", 0),
    ("<a><b><![CDATA[x]]>y</b><b>z</b></a>", &["-o", "yaml"], ".", "a:\n  b:\n    - - x\n      - y\n    - z\n", "", 0),
    ("<a>line1\nline2</a>", &["-o", "yaml"], ".", "a: |-\n  line1\n  line2\n", "", 0),
    ("<a x=\"a\nb\">1</a>", &["-o", "yaml"], ".", "a:\n  +content: \"1\"\n  +@x: |-\n    a\n    b\n", "", 0),
    ("<a><b>~</b><c>yes</c><d>1.5</d><e></e></a>", &["-o", "yaml"], ".", "a:\n  b: \"~\"\n  c: yes\n  d: \"1.5\"\n  e:\n", "", 0),
    ("<a/>", &["-o", "yaml"], ".", "a:\n", "", 0),
    ("", &["-o", "yaml"], ".", "\n", "", 0),
    ("<!--c-->", &["-o", "yaml"], ".", "\n", "", 0),
    ("<?xml version=\"1.0\"?><!-- c --><a x=\"1\">t<b/></a>", &["-o", "yaml"], ".", "# c\n+p_xml: version=\"1.0\"\na:\n  +content: t\n  +@x: \"1\"\n  b:\n", "", 0),
    ("<a><b>1</b><b>2</b></a>", &["-o", "yaml"], ".a.b[1]", "2\n", "", 0),
    ("<a><b>1</b><b>2</b></a>", &["-o", "json", "-I0"], "[.a.b[] | tonumber] | length", "2\n", "", 0),
    ("<a x=\"1\"><b>t</b></a>", &["-o", "json", "-I0"], ".a | keys", "[\"+@x\",\"b\"]\n", "", 0),
    ("<a x=\"1\"><b>t</b></a>", &["-o", "json", "-I0"], "[.. | select(tag == \"!!str\")]", "[\"1\",\"t\"]\n", "", 0),
];

#[test]
fn xml_input_matches_yq_on_stdin_6925() -> Result<()> {
    for &(input, extra, filter, stdout, stderr, code) in ROWS {
        let mut args = vec!["-p", "xml"];
        args.extend_from_slice(extra);
        let (out, err, status) = run_yq_stdin_with_stderr(filter, input, &args)?;
        let row = format!("-p xml {extra:?} {filter:?} on {input:?}");
        assert_eq!(out, stdout, "stdout of {row}");
        assert_eq!(error_line(&err), stderr, "stderr of {row}");
        assert_eq!(status, code, "exit status of {row}");
    }
    Ok(())
}

/// `*.xml` and `*.x` (any case) are XML both ways under `-p auto`: the output is XML too, and `-i`
/// writes XML back. An empty element stays empty there (`<true/>`), where a `null` is otherwise
/// written `null`.
#[test]
fn xml_by_extension_and_inplace_6925() -> Result<()> {
    let dir = TempDir::new()?;
    let a = write_file(&dir, "a.xml", b"<a><b x=\"1\">t</b><c/></a>")?;
    let b = write_file(&dir, "b.X", b"<d>1</d>")?;
    assert_eq!(
        run_yq_files(".a.b", &[&a], &["-o", "json", "-I0"])?,
        ("{\"+content\":\"t\",\"+@x\":\"1\"}\n".into(), String::new(), 0)
    );
    assert_eq!(
        run_yq_files(".", &[&a], &[])?,
        ("<a>\n  <b x=\"1\">t</b>\n  <c></c>\n</a>\n".into(), String::new(), 0)
    );
    assert_eq!(
        run_yq_files(".", &[&b, &a], &["-o", "json", "-I0"])?,
        (
            "{\"d\":\"1\"}\n{\"a\":{\"b\":{\"+content\":\"t\",\"+@x\":\"1\"},\"c\":null}}\n".into(),
            String::new(),
            0
        )
    );
    let (_, _, status) = run_yq_files(".a.b.+content = \"u\"", &[&a], &["-i"])?;
    assert_eq!(status, 0);
    assert_eq!(
        std::fs::read_to_string(&a)?,
        "<a>\n  <b x=\"1\">u</b>\n  <c></c>\n</a>\n"
    );
    Ok(())
}

/// A file read whole, with its charset declared in the prolog: the declaration is read as UTF-8,
/// the rest in the declared encoding; a label that is not an encoding, and one this reader does not
/// have (the multi-byte encodings), are errors.
#[test]
fn xml_declared_charset_6925() -> Result<()> {
    let dir = TempDir::new()?;
    let latin = write_file(&dir, "l.xml", b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a>caf\xe9 \xa4</a>")?;
    let cyrillic = write_file(&dir, "c.xml", b"<?xml version=\"1.0\" encoding=\"windows-1251\"?><a>\xcf\xf0\xe8\xe2\xe5\xf2</a>")?;
    let utf8 = write_file(&dir, "u.xml", b"<?xml version=\"1.0\" encoding=\"utf8\"?><a>x\xffy</a>")?;
    let sjis = write_file(&dir, "s.xml", b"<?xml version=\"1.0\" encoding=\"shift_jis\"?><a/>")?;
    let args = ["-o", "json", "-I0", ".a"];
    assert_eq!(
        run_yq_files(args[3], &[&latin], &args[..3])?,
        ("\"caf\u{e9} \u{a4}\"\n".into(), String::new(), 0)
    );
    assert_eq!(
        run_yq_files(args[3], &[&cyrillic], &args[..3])?,
        ("\"\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}\"\n".into(), String::new(), 0)
    );
    assert_eq!(
        run_yq_files(args[3], &[&utf8], &args[..3])?,
        ("\"x\u{fffd}y\"\n".into(), String::new(), 0)
    );
    let (out, err, status) = run_yq_files(args[3], &[&sjis], &args[..3])?;
    assert_eq!(out, "");
    assert_eq!(
        error_line(&err),
        format!(
            "Error: bad file '{}': xml: opening charset \"shift_jis\": unsupported charset: \"shift_jis\"",
            sjis.display()
        )
    );
    assert_eq!(status, 1);
    Ok(())
}
