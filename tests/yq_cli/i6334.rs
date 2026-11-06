//! #6334: a comment written after an element of a flow collection (`[a, # c` / `  b]`) is kept, as yq keeps
//! it: the identity route, a write and a navigation all print the collection across lines with the comment
//! after the element's comma, and the closing bracket on a line of its own when the last element is
//! commented. Every row was captured from yq v4.53.3 (`yq`, YAML output).

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_a_comment_after_a_flow_collection_element_is_kept_6334() -> Result<()> {
    for (doc, filter, expected) in [
        (
            "v: [\"a\", # c1\n  \"b\", # c2\n  \"c\"]\n",
            ".",
            "v: [\"a\", # c1\n  \"b\", # c2\n  \"c\"]\n",
        ),
        (
            "v: [\"a\", # c1\n  \"b\", # c2\n  \"c\"]\n",
            ".zzkey = 1",
            "v: [\"a\", # c1\n  \"b\", # c2\n  \"c\"]\nzzkey: 1\n",
        ),
        ("v: [\n  x, # c\n  y\n]\n", ".", "v: [x, # c\n  y]\n"),
        (
            "v: [\n  x, # c\n  y\n]\n",
            ".zzkey = 1",
            "v: [x, # c\n  y]\nzzkey: 1\n",
        ),
        ("v: [a, b # last\n  ]\n", ".", "v: [a, b, # last\n]\n"),
        (
            "v: [a, b # last\n  ]\n",
            ".zzkey = 1",
            "v: [a, b, # last\n]\nzzkey: 1\n",
        ),
        (
            "v: [a, # c\n  b, # d\n  ]\n",
            ".",
            "v: [a, # c\n  b, # d\n]\n",
        ),
        (
            "v: [a, # c\n  b, # d\n  ]\n",
            ".zzkey = 1",
            "v: [a, # c\n  b, # d\n]\nzzkey: 1\n",
        ),
        ("v: [[1], # c\n  2]\n", ".", "v: [[1] # c\n, 2]\n"),
        (
            "v: [[1], # c\n  2]\n",
            ".zzkey = 1",
            "v: [[1] # c\n, 2]\nzzkey: 1\n",
        ),
        ("v: [a # c\n  , b]\n", ".", "v: [a, # c\n  b]\n"),
        (
            "v: [a # c\n  , b]\n",
            ".zzkey = 1",
            "v: [a, # c\n  b]\nzzkey: 1\n",
        ),
        (
            "v: [a, {k: 1}, # c\n  b]\n",
            ".",
            "v: [a, {k: 1} # c\n, b]\n",
        ),
        (
            "v: [a, {k: 1}, # c\n  b]\n",
            ".zzkey = 1",
            "v: [a, {k: 1} # c\n, b]\nzzkey: 1\n",
        ),
        ("v: {a: 1, # c\n  b: 2}\n", ".", "v: {a: 1, # c\n  b: 2}\n"),
        (
            "v: {a: 1, # c\n  b: 2}\n",
            ".zzkey = 1",
            "v: {a: 1, # c\n  b: 2}\nzzkey: 1\n",
        ),
        ("v: {a: 1, b: 2 # c\n  }\n", ".", "v: {a: 1, b: 2, # c\n}\n"),
        (
            "v: {a: 1, b: 2 # c\n  }\n",
            ".zzkey = 1",
            "v: {a: 1, b: 2, # c\n}\nzzkey: 1\n",
        ),
        (
            "a:\n  t: [x, # c\n    y]\nb: 1\n",
            ".",
            "a:\n  t: [x, # c\n    y]\nb: 1\n",
        ),
        (
            "a:\n  t: [x, # c\n    y]\nb: 1\n",
            ".zzkey = 1",
            "a:\n  t: [x, # c\n    y]\nb: 1\nzzkey: 1\n",
        ),
        (
            "a:\n  b: [[1, # c0\n    a,\n    x y, # c2\n    ]]\n  c: 2\n",
            ".",
            "a:\n  b: [[1, # c0\n      a, x y, # c2\n    ]]\n  c: 2\n",
        ),
        (
            "a:\n  b: [[1, # c0\n    a,\n    x y, # c2\n    ]]\n  c: 2\n",
            ".zzkey = 1",
            "a:\n  b: [[1, # c0\n      a, x y, # c2\n    ]]\n  c: 2\nzzkey: 1\n",
        ),
        ("- [x, # c\n  y]\n- 2\n", ".", "- [x, # c\n  y]\n- 2\n"),
        (
            "a: {k: [1, # c\n    2], j: 3}\n",
            ".",
            "a: {k: [1, # c\n    2], j: 3}\n",
        ),
        (
            "a: {k: [1, # c\n    2], j: 3}\n",
            ".zzkey = 1",
            "a: {k: [1, # c\n    2], j: 3}\nzzkey: 1\n",
        ),
        (
            "a: [{k: 1, # c\n    j: 2}, 3]\n",
            ".",
            "a: [{k: 1, # c\n    j: 2}, 3]\n",
        ),
        (
            "a: [{k: 1, # c\n    j: 2}, 3]\n",
            ".zzkey = 1",
            "a: [{k: 1, # c\n    j: 2}, 3]\nzzkey: 1\n",
        ),
        ("v: [ x, y ] # end\n", ".", "v: [x, y] # end\n"),
        (
            "v: [ x, y ] # end\n",
            ".zzkey = 1",
            "v: [x, y] # end\nzzkey: 1\n",
        ),
        (
            "v: [x, y, # c\n]\nw: [p, q]\n",
            ".",
            "v: [x, y, # c\n]\nw: [p, q]\n",
        ),
        (
            "v: [x, y, # c\n]\nw: [p, q]\n",
            ".zzkey = 1",
            "v: [x, y, # c\n]\nw: [p, q]\nzzkey: 1\n",
        ),
        ("v: [a, b] # whole\n", ".", "v: [a, b] # whole\n"),
        (
            "v: [a, b] # whole\n",
            ".zzkey = 1",
            "v: [a, b] # whole\nzzkey: 1\n",
        ),
        (
            "v: [a, # c\n  b] # whole\n",
            ".",
            "v: [a, # c\n  b] # whole\n",
        ),
        (
            "v: [a, # c\n  b] # whole\n",
            ".zzkey = 1",
            "v: [a, # c\n  b] # whole\nzzkey: 1\n",
        ),
        (
            "a:\n  t: [x, # c\n    y]\nb: 1\n",
            ".a",
            "t: [x, # c\n  y]\n",
        ),
        ("- [x, # c\n  y]\n- 2\n", ".[0]", "[x, # c\n  y]\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!(
            (stdout.as_str(), code),
            (expected, 0),
            "{filter} on {doc:?}"
        );
    }
    Ok(())
}
