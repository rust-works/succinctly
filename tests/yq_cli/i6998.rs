//! #6998: yq's codec operators over a string or a file: `from_props`, `from_csv`, `from_tsv`,
//! `from_xml`, `to_xml` and the loaders `load_props` and `load_xml` (and their `fromxml`-style
//! spellings), each the decoder or encoder of the matching `-p`/`-o` format. Every row is captured
//! from yq v4.53.3 with `-n -o=json -I=0`, run in a directory holding the files written below.

use std::process::Command;

use anyhow::Result;
use tempfile::TempDir;

/// (filter, yq's stdout, yq's first stderr line, yq's exit status).
#[rustfmt::skip]
const ROWS: &[(&str, &str, &str, i32)] = &[
    ("\"a,b\\n1,2\\n3,4\" | from_csv", "[{\"a\":1,\"b\":2},{\"a\":3,\"b\":4}]\n", "", 0),
    ("\"a;b\\n1;2\" | from_csv", "[{\"a;b\":\"1;2\"}]\n", "", 0),
    ("\"a\\tb\\n1\\t2\" | from_tsv", "[{\"a\":1,\"b\":2}]\n", "", 0),
    ("\"a,b\" | from_csv", "[]\n", "", 0),
    ("\"\" | from_csv", "", "Error: EOF", 1),
    ("\"\" | from_tsv", "", "Error: EOF", 1),
    ("\"a,b\\n\\\"x\\\",\\\"y,z\\\"\\n\" | from_csv", "[{\"a\":\"x\",\"b\":\"y,z\"}]\n", "", 0),
    ("\"a,b\\n1\" | from_csv", "", "Error: record on line 2: wrong number of fields", 1),
    ("\"k = v\\nn.x = 1\" | from_props", "{\"k\":\"v\",\"n\":{\"x\":\"1\"}}\n", "", 0),
    ("\"a.0 = x\\na.1 = y\" | from_props", "{\"a\":[\"x\",\"y\"]}\n", "", 0),
    ("\"\" | from_props", "", "Error: EOF", 1),
    ("\"a\" | from_props", "{\"a\":\"\"}\n", "", 0),
    ("\"<a x=\\\"1\\\"><b>t</b></a>\" | from_xml", "{\"a\":{\"+@x\":\"1\",\"b\":\"t\"}}\n", "", 0),
    ("\"<a><b>1</b><b>2</b></a>\" | from_xml | .a.b", "[\"1\",\"2\"]\n", "", 0),
    ("\"\" | from_xml", "null\n", "", 0),
    ("\"x\" | from_xml", "", "Error: invalid XML: Encountered chardata [x] outside of XML node", 1),
    ("\"<a/>\" | from_xml", "{\"a\":null}\n", "", 0),
    ("\"<a>\" | from_xml", "null\n", "", 0),
    ("\"<!--c--><a>1</a>\" | from_xml | head_comment", "\"\"\n", "", 0),
    ("\"<a/>\" | from_xml | .a | tag", "\"!!null\"\n", "", 0),
    ("{\"a\":{\"+@x\":\"1\",\"b\":\"t\"}} | to_xml", "\"<a x=\\\"1\\\">\\n  <b>t</b>\\n</a>\\n\"\n", "", 0),
    ("{\"a\":[1,2]} | to_xml", "\"<a>1</a>\\n<a>2</a>\\n\"\n", "", 0),
    ("{\"a\":{\"b\":null}} | to_xml", "\"<a>\\n  <b>null</b>\\n</a>\\n\"\n", "", 0),
    ("[1] | to_xml", "", "Error: cannot encode !!seq to XML - only maps can be encoded", 1),
    ("\"x\" | to_xml", "\"x\"\n", "", 0),
    ("{\"a\":\"é<\"} | to_xml", "\"<a>é&lt;</a>\\n\"\n", "", 0),
    ("{\"+p_xml\":\"version=\\\"1.0\\\"\",\"a\":1} | to_xml", "\"<?xml version=\\\"1.0\\\"?>\\n<a>1</a>\\n\"\n", "", 0),
    ("1 | from_csv", "[]\n", "", 0),
    ("null | from_xml", "", "Error: invalid XML: Encountered chardata [null] outside of XML node", 1),
    ("true | from_props", "{\"true\":\"\"}\n", "", 0),
    ("12 | from_props", "{\"12\":\"\"}\n", "", 0),
    ("[1] | from_csv", "", "Error: EOF", 1),
    ("{\"a\":1} | from_props", "", "Error: EOF", 1),
    ("{\"a\":1} | from_xml", "null\n", "", 0),
    ("1.5 | from_xml", "", "Error: invalid XML: Encountered chardata [1.5] outside of XML node", 1),
    ("\"k=v\" | from_props | .k |= \"z\"", "{\"k\":\"z\"}\n", "", 0),
    ("\"a,b\\n1,2\" | from_csv | .[0].a + 1", "2\n", "", 0),
    ("\"<a>1</a>\" | fromxml", "{\"a\":\"1\"}\n", "", 0),
    ("{\"a\":1} | toxml", "\"<a>1</a>\\n\"\n", "", 0),
    ("load_xml(\"x.xml\")", "{\"a\":{\"+@x\":\"1\",\"b\":[\"t\",\"u\"]}}\n", "", 0),
    ("load_props(\"p.properties\")", "{\"k\":\"v\",\"n\":{\"x\":\"1\"}}\n", "", 0),
    ("load_xml(\"e.xml\")", "null\n", "", 0),
    ("load_props(\"e.properties\")", "null\n", "", 0),
    ("load_xml(\"bad.xml\")", "", "Error: failed to load bad.xml: bad file 'bad.xml': invalid XML: Encountered chardata [text] outside of XML node", 1),
    ("load_xml(\"nope.xml\")", "", "Error: failed to load nope.xml: open nope.xml: no such file or directory", 1),
    ("load_props(\"nope.properties\")", "", "Error: failed to load nope.properties: open nope.properties: no such file or directory", 1),
    ("loadxml(\"x.xml\")", "{\"a\":{\"+@x\":\"1\",\"b\":[\"t\",\"u\"]}}\n", "", 0),
    ("load_xml(\"x.xml\") | .a.b[0]", "\"t\"\n", "", 0),
    ("\"<a/>\" | (from_xml | .a), \"x\"", "null\n\"x\"\n", "", 0),
];

#[test]
fn codec_operators_match_yq_6998() -> Result<()> {
    let dir = TempDir::new()?;
    for (name, text) in [
        ("x.xml", "<a x=\"1\"><b>t</b><b>u</b></a>"),
        ("p.properties", "k = v\nn.x = 1\n"),
        ("e.xml", ""),
        ("bad.xml", "text"),
        ("e.properties", ""),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for &(filter, stdout, stderr, code) in ROWS {
        let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .current_dir(dir.path())
            .args(["yq", "-n", "-o=json", "-I=0", filter])
            .output()?;
        let err = String::from_utf8(output.stderr)?;
        assert_eq!(
            String::from_utf8(output.stdout)?,
            stdout,
            "stdout of {filter}"
        );
        assert_eq!(
            err.lines().next().unwrap_or(""),
            stderr,
            "stderr of {filter}"
        );
        assert_eq!(output.status.code().unwrap_or(-1), code, "exit of {filter}");
    }
    Ok(())
}
