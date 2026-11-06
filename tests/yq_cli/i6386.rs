//! #6386: a key with no value, followed by a quoted or flow-collection key at the same indent (or a
//! dedent), took that key's text as its value: `a:` / `"b": 1` was `{"a": "b", "b": 1}`. The empty
//! value's node was written at the start of the next line, where the reader decodes a node by its
//! first byte; it is written at its key's colon now, which is where yq puts an empty value (`.a | line`
//! of `a:` is the key's line). Every row was captured from yq v4.53.3 (`yq -o=json -I0 .`).

use anyhow::Result;

use crate::run_yq_stdin;

const ARGS: &[&str] = &["-o=json", "-I=0"];

#[test]
fn test_an_empty_value_before_a_quoted_or_flow_key_stays_null_6386() -> Result<()> {
    for (doc, expected) in [
        ("a:\n\"b\": 1\n", "{\"a\":null,\"b\":1}"),
        ("a:\n'b': 1\n", "{\"a\":null,\"b\":1}"),
        ("a:\n\"b\":\n", "{\"a\":null,\"b\":null}"),
        ("a:\n\"b\"  : 1\n", "{\"a\":null,\"b\":1}"),
        ("a:\n\"b\": 1\nc: 2\n", "{\"a\":null,\"b\":1,\"c\":2}"),
        ("a:\n[x]: 1\n", "{\"a\":null,\"\":1}"),
        ("a:\n{x: 1}: 1\n", "{\"a\":null,\"\":1}"),
        ("a:  \n\"b\": 1\n", "{\"a\":null,\"b\":1}"),
        ("a: # c\n\"b\": 1\n", "{\"a\":null,\"b\":1}"),
        ("a:\n\n\"b\": 1\n", "{\"a\":null,\"b\":1}"),
        ("\"a\":\n\"b\": 1\n", "{\"a\":null,\"b\":1}"),
        ("'a':\n'b': 1\n", "{\"a\":null,\"b\":1}"),
        (
            "a:\n\"b\": 1\n\"c\":\n\"d\": 2\n",
            "{\"a\":null,\"b\":1,\"c\":null,\"d\":2}",
        ),
        (
            "a:\n\"b\": [1]\n\"c\": {d: 1}\n",
            "{\"a\":null,\"b\":[1],\"c\":{\"d\":1}}",
        ),
        ("---\na:\n\"b\": 1\n", "{\"a\":null,\"b\":1}"),
        (
            "a:\n\"b\": 1\n---\nc:\n\"d\": 2\n",
            "{\"a\":null,\"b\":1}\n{\"c\":null,\"d\":2}",
        ),
        // A dedent to the root, from a mapping and from a sequence item's compact mapping.
        ("x:\n  a:\n\"b\": 1\n", "{\"x\":{\"a\":null},\"b\":1}"),
        ("x:\n  a:\n[y]: 1\n", "{\"x\":{\"a\":null},\"\":1}"),
        ("x:\n  - a:\n\"b\": 1\n", "{\"x\":[{\"a\":null}],\"b\":1}"),
        (
            "x:\n  - a:\n  - b\n\"c\": 1\n",
            "{\"x\":[{\"a\":null},\"b\"],\"c\":1}",
        ),
        // A block scalar header at the key's own indent cannot be a key: it is the value.
        ("a:\n|\n  x\n", "{\"a\":\"x\\n\"}"),
        ("a:\n>\n  x\n", "{\"a\":\"x\\n\"}"),
        ("a:\n|-\n  x\nb: 1\n", "{\"a\":\"x\",\"b\":1}"),
        ("a:\n|\n  x\n\"b\": 1\n", "{\"a\":\"x\\n\",\"b\":1}"),
        ("a:\n|\n  x\n  y\nb: 2\n", "{\"a\":\"x\\ny\\n\",\"b\":2}"),
        ("a:\n>+\n  x\n\nb: 1\n", "{\"a\":\"x\\n\\n\",\"b\":1}"),
        ("a:\n|2\n    x\nb: 1\n", "{\"a\":\"  x\\n\",\"b\":1}"),
        (
            "x:\n  a:\n  |\n    t\n  b: 1\n",
            "{\"x\":{\"a\":\"t\\n\",\"b\":1}}",
        ),
        // The shapes that already read.
        ("a:\nb: 1\n", "{\"a\":null,\"b\":1}"),
        ("a:\n&n b: 1\n", "{\"a\":null,\"b\":1}"),
        ("a:\n  \"b\": 1\n", "{\"a\":{\"b\":1}}"),
        ("x:\n  a:\n  \"b\": 1\n", "{\"x\":{\"a\":null,\"b\":1}}"),
        ("- a:\n- \"b\": 1\n", "[{\"a\":null},{\"b\":1}]"),
        ("a:\n- x\n\"b\": 1\n", "{\"a\":[\"x\"],\"b\":1}"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, ARGS)?;
        assert_eq!((stdout.trim_end(), code), (expected, 0), "{doc:?}");
    }
    Ok(())
}

#[test]
fn test_an_empty_value_sits_at_its_keys_colon_6386() -> Result<()> {
    // `line` and `column` of an empty value: the key's line, right after its colon.
    for (doc, filter, expected) in [
        ("a:\nb: 1\n", ".a | line", "1"),
        ("a:\nb: 1\n", ".a | column", "3"),
        ("a:\n\n\nb: 1\n", ".a | line", "1"),
        ("a: # c\nb: 1\n", ".a | line", "1"),
        ("a: # c\nb: 1\n", ".a | column", "3"),
        ("x:\n  a:\n  b: 1\n", ".x.a | line", "2"),
        ("a:\n\"b\": 1\n", ".a | line", "1"),
        ("x:\n  - a:\n    b: 1\n", ".x[0].a | line", "2"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!((stdout.trim_end(), code), (expected, 0), "{doc:?} {filter}");
    }
    Ok(())
}
