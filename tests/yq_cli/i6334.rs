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

/// A comment on a line of its own inside a flow collection is the head of the element after it, or the foot
/// of the last element when the closing bracket follows it, and yq lays the collection out around it. Every
/// row was captured from yq v4.53.3.
#[test]
fn test_a_comment_on_its_own_line_inside_a_flow_collection_is_kept_6334() -> Result<()> {
    let rows: &[(&str, &str, &[&str], &str)] = &[
        (
            "v: [\n  # head\n  x,\n  y, # tail\n  ]\n",
            ".",
            &[],
            "v: [\n  # head\n  x, y, # tail\n]\n",
        ),
        ("v: [a,\n# mid\nb]\n", ".", &[], "v: [a,\n  # mid\n  b]\n"),
        (
            "v: [a, b,\n# last\n]\n",
            ".",
            &[],
            "v: [a, b,\n  # last\n]\n",
        ),
        (
            "v: [\n  # h1\n  # h2\n  x,\n  y]\n",
            ".",
            &[],
            "v: [\n  # h1\n  # h2\n  x, y]\n",
        ),
        (
            "v: {\n  # h\n  a: 1,\n  b: 2}\n",
            ".",
            &[],
            "v: {\n  # h\n  a: 1, b: 2}\n",
        ),
        (
            "v: {a: 1,\n  # mid\n  b: 2}\n",
            ".",
            &[],
            "v: {a: 1,\n  # mid\n  b: 2}\n",
        ),
        (
            "v: {a: 1,\n  b: 2,\n  # last\n  }\n",
            ".",
            &[],
            "v: {a: 1, b: 2,\n  # last\n}\n",
        ),
        (
            "v: [[1,\n  # in\n  2], 3]\n",
            ".",
            &[],
            "v: [[1,\n    # in\n    2], 3]\n",
        ),
        (
            "v: [x, # same\n  # own\n  y]\n",
            ".",
            &[],
            "v: [x, # same\n  # own\n  y]\n",
        ),
        (
            "v: [x,\n  y\n  # foot\n  ]\n",
            ".",
            &[],
            "v: [x, y,\n  # foot\n]\n",
        ),
        (
            "- [\n  # h\n  x,\n  y]\n",
            ".",
            &[],
            "- [\n  # h\n  x, y]\n",
        ),
        ("v: [\n  # only\n  ]\n", ".", &[], "v: []\n"),
        ("v: [ # c\n  x, y]\n", ".", &[], "v: [x, y]\n"),
        (
            "v: [x, [y, z]\n  # f\n  ]\n",
            ".",
            &[],
            "v: [x, [y, z]\n  # f\n]\n",
        ),
        (
            "v: [x, {a: 1}\n  # f\n  ]\n",
            ".",
            &[],
            "v: [x, {a: 1,}\n  # f\n]\n",
        ),
        (
            "v: {a: [1]\n  # f\n  }\n",
            ".",
            &[],
            "v: {a: [1],\n  # f\n}\n",
        ),
        (
            "v: {a: 1,\n  b: {c: 1}\n  # f\n  }\n",
            ".",
            &[],
            "v: {a: 1, b: {c: 1},\n  # f\n}\n",
        ),
        (
            "v: [x # a\n  , # b\n  y]\n",
            ".",
            &[],
            "v: [x, # a\n  # b\n  y]\n",
        ),
        ("v: [x\n  , # b\n  y]\n", ".", &[], "v: [x, # b\n  y]\n"),
        (
            "v: [x # a\n  , # b\n  ]\n",
            ".",
            &[],
            "v: [x, # a\n  # b\n]\n",
        ),
        ("v: [a, b\n# f\n,\n]\n", ".v", &[], "[a, b]\n"),
        (
            "v: [\n  # h\n  x,\n  y,\n  # f\n  ]\n",
            ".z = 1",
            &[],
            "v: [\n  # h\n  x, y,\n  # f\n]\nz: 1\n",
        ),
        (
            "v: [\n  # h\n  x,\n  y,\n  # f\n  ]\n",
            ".v",
            &[],
            "[\n  # h\n  x, y,\n  # f\n]\n",
        ),
        (
            "v: [\n  # h\n  x,\n  y,\n  # f\n  ]\n",
            ".",
            &["-P"],
            "v:\n  # h\n  - x\n  - y\n  # f\n",
        ),
        (
            "v: [\n  # h\n  x,\n  y,\n  # f\n  ]\n",
            ".",
            &["-I4"],
            "v: [\n    # h\n    x, y,\n    # f\n]\n",
        ),
        (
            "v: [\n  # h\n  x,\n  y,\n  # f\n  ]\n",
            ".v[0] | head_comment",
            &[],
            "h\n",
        ),
        (
            "v: [\n  # h\n  x,\n  y,\n  # f\n  ]\n",
            ".v[1] | foot_comment",
            &[],
            "f\n",
        ),
        (
            "v: [\n  # h\n  x,\n  y,\n  # f\n  ]\n",
            ".v[0] | foot_comment",
            &[],
            "\n",
        ),
        (
            "v: {\n  # h\n  a: 1,\n  # m\n  b: 2,\n  # f\n  }\n",
            ".v.b | key | head_comment",
            &[],
            "m\n",
        ),
        (
            "v: {\n  # h\n  a: 1,\n  # m\n  b: 2,\n  # f\n  }\n",
            ".v.b | key | foot_comment",
            &[],
            "f\n",
        ),
        (
            "v: [ [ b ,\n  # c4\n  b ]\n  # c6\n  ]\n",
            ".",
            &["-P"],
            "v:\n  - - b\n    # c4\n    - b\n\n# c6\n",
        ),
        (
            "a:\n  v: [\n  # h\n  x,\n  y]\n  w: 1\n",
            ".",
            &[],
            "a:\n  v: [\n    # h\n    x, y]\n  w: 1\n",
        ),
        ("v: [\n  x,\n  # h\n  y]\n", "del(.v[1])", &[], "v: [x]\n"),
        (
            "v: [\n  # h\n  x,\n  y,\n  # f\n  ]\n",
            "del(.v[1])",
            &[],
            "v: [\n  # h\n  x]\n",
        ),
    ];
    for &(doc, filter, flags, expected) in rows {
        let (stdout, code) = run_yq_stdin(filter, doc, flags)?;
        assert_eq!(
            (stdout.as_str(), code),
            (expected, 0),
            "{flags:?} {filter} on {doc:?}"
        );
    }
    Ok(())
}
