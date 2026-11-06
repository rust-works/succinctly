//! #6419: a `%` directive that go-yaml sees is checked, and one met after a document's content ends that
//! document. yq's header slurp takes a `%YAML` line only in the head of the stream, before any other
//! directive; every other `%YAML` reaches go-yaml, which accepts `1.1` alone (`found incompatible YAML
//! document` for any other version), once per document, with nothing after it but blanks and a comment.
//! A directive at column 0 where a node would start closes the document as a `...` does, whatever it held
//! (a pending property or value, a block scalar, a block collection), and a `---` has to follow. Every row was
//! captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const JSON: &[&str] = &["-o=json", "-I=0"];

#[test]
fn a_yaml_directive_go_yaml_sees_is_checked_6419() -> Result<()> {
    // yq prints the documents before one it refuses; only the exit code is compared.
    for doc in [
        "a: 1\n...\n%YAML 1.2\n---\nb: 1\n",
        "a: 1\n...\n%YAML 1.0\n---\nb: 1\n",
        "a: 1\n...\n%YAML 2.0\n---\nb: 1\n",
        "a: 1\n...\n%YAML 1.10\n---\nb: 1\n",
        "a: 1\n...\n%YAML 1\n---\nb: 1\n",
        "a: 1\n...\n%YAML 1.\n---\nb: 1\n",
        "a: 1\n...\n%YAML x\n---\nb: 1\n",
        "a: 1\n...\n%YAML\n---\nb: 1\n",
        "a: 1\n...\n%YAML 1.1 x\n---\nb: 1\n",
        "a: 1\n...\n%YAML 1 .1\n---\nb: 1\n",
        "a: 1\n...\n%YAML 001.1\n---\nb: 1\n",
        "a: 1\n...\n%YAML 1.100\n---\nb: 1\n",
        "a: 1\n...\n%YAML 100.1\n---\nb: 1\n",
        "a: 1\n...\n%YAML 1.1\n%YAML 1.1\n---\nb: 1\n",
        "a: 1\n...\n# c\n%YAML 1.2\n---\nb: 1\n",
        "a: 1\n...\n%YAMLL\n---\nb: 1\n",
        "%TAG ! x:\n%YAML 1.2\n---\na: 1\n",
        "%TAG ! x:\n%YAML 1.1\n%YAML 1.1\n---\na: 1\n",
        "%TAG ! x:\n%YAML 1.1 x\n---\na: 1\n",
        "%TAG ! x:\n%YAML x\n---\na: 1\n",
        "%TAG ! x:\n# c\n%YAML 1.2\n---\na: 1\n",
        "# c\n%YAML 1.2\n%TAG ! x:\n%YAML 1.2\n---\na: 1\n",
        "a: 1\n%YAML 1.2\n---\nb: 2\n",
        "a: 1\n%FOO\n---\nb: 2\n",
        "a: 1\n%\n---\nb: 2\n",
        "a: 1\n%TAG\n---\nb: 2\n",
        "a: 1\n%TAG !e! w\n",
        "a: 1\n%TAG !e! w\nb: 2\n",
        "a: 1\n%TAG !e! w\n...\nb: 2\n",
        "a: 1\n...\n%TAG !e! w\n",
        "a: 1\n...\n%YAML 1.1\n",
    ] {
        let (_out, _err, code) = run_yq_stdin_with_stderr(".", doc, JSON)?;
        assert_ne!(code, 0, "{doc:?}");
    }
    Ok(())
}

#[test]
fn a_directive_that_ends_a_document_and_a_header_yaml_line_are_read_6419() -> Result<()> {
    for (doc, want) in [
        (
            "a: 1\n...\n%YAML 1.1\n---\nb: 1\n",
            "{\"a\":1}\n{\"b\":1}\n",
        ),
        (
            "a: 1\n...\n%YAML 1.1 # c\n---\nb: 1\n",
            "{\"a\":1}\n{\"b\":1}\n",
        ),
        (
            "a: 1\n...\n%YAML 1.1#c\n---\nb: 1\n",
            "{\"a\":1}\n{\"b\":1}\n",
        ),
        (
            "a: 1\n...\n%YAML\t1.1\n---\nb: 1\n",
            "{\"a\":1}\n{\"b\":1}\n",
        ),
        (
            "a: 1\n...\n%YAML 01.1\n---\nb: 1\n",
            "{\"a\":1}\n{\"b\":1}\n",
        ),
        (
            "a: 1\n...\n%YAML 1.1\n%TAG ! x:\n---\nb: 1\n",
            "{\"a\":1}\n{\"b\":1}\n",
        ),
        ("%TAG ! x:\n%YAML 1.1\n---\na: 1\n", "{\"a\":1}\n"),
        ("%YAML 1.2\n%YAML 1.2\n---\na: 1\n", "{\"a\":1}\n"),
        ("%YAML 1.2\n\n%YAML 9.9\n---\na: 1\n", "{\"a\":1}\n"),
        (
            "%YAML 1.1\n%TAG ! x:\n%YAML 1.1\n---\na: 1\n",
            "{\"a\":1}\n",
        ),
        ("%YAMLL 1.2\n%YAML 1.2\n---\na: 1\n", "{\"a\":1}\n"),
        ("a: 1\n%TAG !e! w\n--- x\n", "{\"a\":1}\n\"x\"\n"),
        ("a: 1\n%TAG !e! w\n---\nb: 2\n", "{\"a\":1}\n{\"b\":2}\n"),
        ("a: 1\n%YAML 1.1\n---\nb: 2\n", "{\"a\":1}\n{\"b\":2}\n"),
        ("!!str\n%TAG !e! w\n---\nb: 2\n", "\"\"\n{\"b\":2}\n"),
        ("&a\n%TAG !e! w\n---\nb: 2\n", "null\n{\"b\":2}\n"),
        ("a:\n%TAG !e! w\n---\nb: 2\n", "{\"a\":null}\n{\"b\":2}\n"),
        (
            "a:\n  b:\n%TAG !e! w\n---\nb: 2\n",
            "{\"a\":{\"b\":null}}\n{\"b\":2}\n",
        ),
        ("- \n%TAG !e! w\n---\nb: 2\n", "[null]\n{\"b\":2}\n"),
        (
            "- a\n- b\n%TAG !e! w\n---\nb: 2\n",
            "[\"a\",\"b\"]\n{\"b\":2}\n",
        ),
        ("? a\n%TAG !e! w\n---\nb: 2\n", "{\"a\":null}\n{\"b\":2}\n"),
        (
            "? a\n: b\n%TAG !e! w\n---\nb: 2\n",
            "{\"a\":\"b\"}\n{\"b\":2}\n",
        ),
        (
            "a: |\n  x\n%TAG !e! w\n---\nb: 2\n",
            "{\"a\":\"x\\n\"}\n{\"b\":2}\n",
        ),
        (
            "a: >\n  x\n%TAG !e! w\n---\nb: 2\n",
            "{\"a\":\"x\\n\"}\n{\"b\":2}\n",
        ),
        (
            "a: \"x\"\n%TAG !e! w\n---\nb: 2\n",
            "{\"a\":\"x\"}\n{\"b\":2}\n",
        ),
        (
            "a: [x]\n%TAG !e! w\n---\nb: 2\n",
            "{\"a\":[\"x\"]}\n{\"b\":2}\n",
        ),
        (
            "a: {x: 1}\n%TAG !e! w\n---\nb: 2\n",
            "{\"a\":{\"x\":1}}\n{\"b\":2}\n",
        ),
        (
            "a: !!str\n%TAG !e! w\n---\nb: 2\n",
            "{\"a\":\"\"}\n{\"b\":2}\n",
        ),
        (
            "a: &x\n%TAG !e! w\n---\nb: 2\n",
            "{\"a\":null}\n{\"b\":2}\n",
        ),
        (
            "a: x\n  y\n%TAG !e! w\n---\nb: 2\n",
            "{\"a\":\"x y\"}\n{\"b\":2}\n",
        ),
        ("\"x\"\n%TAG !e! w\n---\nb: 2\n", "\"x\"\n{\"b\":2}\n"),
        ("[a]\n%TAG !e! w\n---\nb: 2\n", "[\"a\"]\n{\"b\":2}\n"),
        ("---\n%TAG !e! w\n---\nb: 2\n", "{\"b\":2}\n"),
        (
            "---\na: 1\n%TAG !e! w\n---\nb: 2\n",
            "{\"a\":1}\n{\"b\":2}\n",
        ),
        (
            "a: 1\n---\n%TAG !e! w\n---\nb: 2\n",
            "{\"a\":1}\nnull\n{\"b\":2}\n",
        ),
        (
            "a: 1\n%TAG !e! w\n%TAG !f! w\n---\nb: 2\n",
            "{\"a\":1}\n{\"b\":2}\n",
        ),
        (
            "a: 1\n%TAG !e! w\n# c\n---\nb: 2\n",
            "{\"a\":1}\n{\"b\":2}\n",
        ),
        ("a: 1\n%TAG !e! w\n\n---\nb: 2\n", "{\"a\":1}\n{\"b\":2}\n"),
        ("a: 1\n%TAG !e! w\n---\n...\n", "{\"a\":1}\nnull\n"),
        (
            "a: 1\n%TAG !e! w\n---\n%TAG !f! w\n---\nc: 3\n",
            "{\"a\":1}\nnull\n{\"c\":3}\n",
        ),
        (
            "a: 1\n  %TAG !e! w\n---\nb: 2\n",
            "{\"a\":\"1 %TAG !e! w\"}\n{\"b\":2}\n",
        ),
        ("x\n%TAG !e! w\n---\ny\n", "\"x %TAG !e! w\"\n\"y\"\n"),
        ("x\n%YAML 1.1\n", "\"x %YAML 1.1\"\n"),
        ("x\n%TAG !e! w\n---\ny\n", "\"x %TAG !e! w\"\n\"y\"\n"),
        ("x\n  %TAG !e! w\n", "\"x %TAG !e! w\"\n"),
        ("--- x\n%TAG !e! w\n", "\"x%TAG !e! w\"\n"),
        ("\"x\n%TAG\"\n", "\"x %TAG\"\n"),
        ("'x\n%TAG'\n", "\"x %TAG\"\n"),
        ("---\n%YAML 1.2\n---\na: 1\n", "{\"a\":1}\n"),
        ("---\n%YAML 1.2\n%TAG ! x:\n---\na: 1\n", "{\"a\":1}\n"),
        (
            "a: 1\n...\n%YAML 1.01\n---\nb: 1\n",
            "{\"a\":1}\n{\"b\":1}\n",
        ),
        ("a: 1\n%TAG !e! w\n--- |\n  t\n", "{\"a\":1}\n\"t\\n\"\n"),
        ("a: 1\n%TAG !e! w\n--- &a\nb: 1\n", "{\"a\":1}\n{\"b\":1}\n"),
        (
            "a: 1\n%TAG !e! w\n--- # c\nb: 1\n",
            "{\"a\":1}\n{\"b\":1}\n",
        ),
        ("a: 1\n%TAG !e! w\n--- !!str\nb\n", "{\"a\":1}\n\"b\"\n"),
        (
            "a: 1\n...\n%YAML 1.1\n---\nb: |\n  x\n%YAML 1.1\n---\nc: 1\n",
            "{\"a\":1}\n{\"b\":\"x\\n\"}\n{\"c\":1}\n",
        ),
    ] {
        let (out, _err, code) = run_yq_stdin_with_stderr(".", doc, JSON)?;
        assert_eq!((out.as_str(), code), (want, 0), "{doc:?}");
    }
    Ok(())
}
