//! #5467: yq's `.` single-quotes a plain scalar that holds a `:` or `?` inside a flow collection,
//! because go-yaml's emitter will not write one plain there; the cursor-streaming route echoed
//! the source text. Every row was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_flow_plain_scalar_with_colon_or_question_is_single_quoted_5467() -> Result<()> {
    for (input, want) in [
        (
            "a: [http://x.y/z, b]\nc: {k: x:y, p?q: 1}\n",
            "a: ['http://x.y/z', b]\nc: {k: 'x:y', 'p?q': 1}\n",
        ),
        ("a: [x, y:z]\n", "a: [x, 'y:z']\n"),
        ("a: [a b:c d]\n", "a: ['a b:c d']\n"),
        ("a: [x:]\n", "a: ['x:']\n"),
        ("a: [?x]\n", "a: ['?x']\n"),
        ("a: [:x]\n", "a: [':x']\n"),
        ("a: [a:b:c]\n", "a: ['a:b:c']\n"),
        // Nested flow collections, a root flow sequence, a multi-line flow sequence.
        ("a: [[x:y]]\n", "a: [['x:y']]\n"),
        ("a: {k: [x:y]}\n", "a: {k: ['x:y']}\n"),
        ("[http://a, b]\n", "['http://a', b]\n"),
        ("a: [\n  x:y,\n  p:q\n]\n", "a: ['x:y', 'p:q']\n"),
        // A flow-mapping key, plain or explicit.
        ("a: {x:y: 1}\n", "a: {'x:y': 1}\n"),
        ("a: {? x:y : z}\n", "a: {'x:y': z}\n"),
        // Properties stay in front; the type does not matter, only the characters.
        ("a: [&n x:y]\n", "a: [&n 'x:y']\n"),
        (
            "a: [!!str x:y, !custom p:q]\n",
            "a: [!!str 'x:y', !custom 'p:q']\n",
        ),
        (
            "a: [2001-12-14T21:59:43Z]\n",
            "a: ['2001-12-14T21:59:43Z']\n",
        ),
        (
            "a: {k: 2001-12-14T21:59:43Z}\n",
            "a: {k: '2001-12-14T21:59:43Z'}\n",
        ),
    ] {
        let (out, code) = run_yq_stdin(".", input, &[])?;
        assert_eq!((out.as_str(), code), (want, 0), "input {input:?}");
    }
    Ok(())
}

#[test]
fn test_flow_scalars_without_a_flow_indicator_are_echoed_5467() -> Result<()> {
    for input in [
        // `#` after a non-blank and `-x` are not indicators.
        "a: [a#b, -x]\n",
        // Already quoted, and other types, stay as written.
        "a: [\"q:r\", 'p:q']\n",
        "a: [1, true, null, 1.5, abc]\n",
        // Block context keeps `:` and `?` bare.
        "a: x:y\nb: p?q\nc:\n  - u:v\n",
    ] {
        let (out, code) = run_yq_stdin(".", input, &[])?;
        assert_eq!((out.as_str(), code), (input, 0), "input {input:?}");
    }
    Ok(())
}
