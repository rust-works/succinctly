//! #6402: go-yaml's scanner starts no plain scalar, key or value, block or flow, on `%`, `@` or a
//! backtick (`found character that cannot start any token`), and reads a `%` at column 0 as a
//! directive before that, wherever the line stands. A line after a document has started that opens
//! with one is an error in yq, where this read it as content (`a: 1` / `%TAG ! x:` was a second key).
//!
//! A `%` that continues a plain scalar, or stands in a quoted or block scalar, is content in both
//! tools. yq prints the documents before the failing one and then the error, where this builds the
//! whole index first and prints nothing (`docs/compliance/yq/limitations.md`), so the refused rows
//! check the exit status and an empty stdout. Every row was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

const JSON: &[&str] = &["-o=json", "-I=0"];

#[test]
fn a_percent_at_or_after_a_node_start_is_refused_as_yq_refuses_it_6402() -> Result<()> {
    for doc in [
        // A column-0 directive line after the document has started.
        "---\n# c\n%TAG ! x:\n",
        "a: 1\n%TAG ! x:\n",
        "a: 1\n%YAML 1.2\n",
        "a: 1\n%FOO\n",
        "a: 1\n%\n",
        "a: 1\n\n%YAML 1.2\n",
        "a: 1\n# c\n%YAML 1.2\n",
        "- a\n%YAML 1.2\n",
        "a:\n  - b\n%x\n",
        "a:\n%YAML 1.2\n",
        "a: x\n%YAML 1.2\n",
        "a: !!str\n%YAML 1.2\n",
        "a: |\n  x\n%YAML 1.2\n",
        "? a\n%YAML 1.2\n",
        "- &a\n%YAML 1.2\n",
        "- a\n- \n%YAML 1.2\n",
        "a:\n  b:\n%YAML 1.2\n",
        // After the properties of a node with no content yet.
        "!!str\n%YAML 1.2\n",
        "&a\n%YAML 1.2\n",
        "!!map\n%YAML 1.2\n",
        "&a\n# c\n%YAML 1.2\n",
        // In a flow collection: the directive is read first.
        "[a,\n%YAML 1.2\n]\n",
        "{a: 1,\n%x\n}\n",
        // Indented, or after a key or an item: the scanner cannot start a token on it.
        "a:\n  %x: 1\n",
        "a: %x\n",
        "- %YAML\n",
        "a: [x,\n  %y]\n",
        "a: @x\n",
        "a: `x\n",
        "- @x\n",
        "- `x\n",
        "[a, %x]\n",
        "[a, [b, %c]]\n",
        "{a: [%b]}\n",
        "[a: %b]\n",
        "? [a,\n  %b]\n: 1\n",
        // After properties on the same line, and as a key after a dash or properties.
        "a: !!str %x\n",
        "- &a %x\n",
        "a: !!str @x\n",
        "a: &x `y\n",
        "- %k: 1\n",
        "!!str %k: 1\n",
        "{a: `x}\n",
        "{%k: 1}\n",
        "? %k\n",
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, JSON)?;
        assert_ne!(code, 0, "{doc:?} is refused");
        assert_eq!(stdout, "", "{doc:?} prints nothing");
    }
    Ok(())
}

#[test]
fn a_percent_that_continues_a_scalar_or_stands_in_one_is_content_6402() -> Result<()> {
    for (doc, expected) in [
        ("---\nx\n%YAML 1.2\n", "\"x %YAML 1.2\"\n"),
        ("a: \"x\n%YAML 1.2\"\n", "{\"a\":\"x %YAML 1.2\"}\n"),
        ("'x\n%YAML 1.2'\n", "\"x %YAML 1.2\"\n"),
        ("a: 1\n %YAML 1.2\n", "{\"a\":\"1 %YAML 1.2\"}\n"),
        ("a: 1\n @x\n", "{\"a\":\"1 @x\"}\n"),
        ("a: x\n  %y\n", "{\"a\":\"x %y\"}\n"),
        ("- a\n- x\n  %y\n", "[\"a\",\"x %y\"]\n"),
        ("? x\n  %y\n: 1\n", "{\"x %y\":1}\n"),
        ("a: -%x\n", "{\"a\":\"-%x\"}\n"),
        ("a: x%y\n", "{\"a\":\"x%y\"}\n"),
        ("a: x @y\n", "{\"a\":\"x @y\"}\n"),
        ("a: x `y\n", "{\"a\":\"x `y\"}\n"),
        ("[a\n%x]\n", "[\"a %x\"]\n"),
        ("- a%b\n", "[\"a%b\"]\n"),
        ("a: 'x @y'\n", "{\"a\":\"x @y\"}\n"),
        ("a: \"%x\"\n", "{\"a\":\"%x\"}\n"),
        ("\"%x\": 1\n", "{\"%x\":1}\n"),
        ("a: |\n  %x\n", "{\"a\":\"%x\\n\"}\n"),
        ("[a, \"%x\", 'y']\n", "[\"a\",\"%x\",\"y\"]\n"),
        ("{a: b%c}\n", "{\"a\":\"b%c\"}\n"),
        // The head's directive is yq's header, and a `---` after content is go-yaml's.
        ("%YAML 1.2\n---\na: 1\n", "{\"a\":1}\n"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, JSON)?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{doc:?}");
    }
    Ok(())
}
