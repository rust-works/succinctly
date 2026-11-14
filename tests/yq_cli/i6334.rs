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
        (
            "v: [a: 1,\n # h\n b: 2]\n",
            ".",
            &[],
            "v: [{a: 1}, {\n    # h\n    b: 2}]\n",
        ),
        (
            "v: [\n # h\n a: 1]\n",
            ".",
            &[],
            "v: [{\n    # h\n    a: 1}]\n",
        ),
        (
            "v: {a:\n # x\n 1, b: 2}\n",
            ".",
            &[],
            "v: {a: 1,\n  # x\n  b: 2}\n",
        ),
        ("v: {a:\n # x\n 1}\n", ".", &[], "v: {a: 1,\n  # x\n}\n"),
        (
            "v: {a:\n # x\n [1, 2], b: 2}\n",
            ".",
            &[],
            "v: {a: [\n    # x\n    1, 2], b: 2}\n",
        ),
        (
            "v: {a:\n # x\n {c: 1}, b: 2}\n",
            ".",
            &[],
            "v: {a: {\n    # x\n    c: 1}, b: 2}\n",
        ),
        (
            "v: {a: # x\n [1, 2], b: 2}\n",
            ".",
            &[],
            "v: {a: [1, 2], b: 2}\n",
        ),
        (
            "v: {a:\n # x\n # y\n 1,\n b: 2}\n",
            ".",
            &[],
            "v: {a: 1,\n  # x\n  # y\n  b: 2}\n",
        ),
        (
            "k:\n  - [1, 2]\n  # c\n",
            ".",
            &["-P"],
            "k:\n  - - 1\n    - 2\n\n# c\n",
        ),
        (
            "k:\n  - a: 1\n    b: 2\n  # c\n",
            ".",
            &["-P"],
            "k:\n  - a: 1\n    b: 2\n# c\n",
        ),
        (
            "k:\n  - { a: 1 }\n  # c\n",
            ".k[0].a = 2",
            &[],
            "k:\n  - {a: 2,}\n  # c\n",
        ),
        (
            "v: [[1] # c\n  # f\n  ]\n",
            ".",
            &[],
            "v: [[1] # c\n  # f\n]\n",
        ),
        (
            "v: [{a: 1} # c\n  # f\n  ]\n",
            ".",
            &[],
            "v: [{a: 1,} # c\n  # f\n]\n",
        ),
        (
            "v: {a: [1] # c\n  # f\n  }\n",
            ".",
            &[],
            "v: {a: [1] # c\n,\n  # f\n}\n",
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

/// Under `-P` a flow collection becomes a block, and the comment on an element that is a collection (`[[1], # a`
/// / `2]`) has no line of its own: yq holds it for the next line without a comment, as it holds one after the
/// whole collection (#4377), but an element with a comment of its own overwrites it instead of letting it wait.
/// Every row was captured from yq v4.53.3.
#[test]
fn test_p_holds_the_comment_of_a_flow_element_that_is_a_collection_6334() -> Result<()> {
    let rows: &[(&str, &str, &[&str], &str)] = &[
        (
            "v: [1, # a\n 2, # b\n 3]\n",
            ".",
            &["-P"],
            "v:\n  - 1 # a\n  - 2 # b\n  - 3\n",
        ),
        (
            "v: {a: 1, # a\n b: 2}\n",
            ".",
            &["-P"],
            "v:\n  a: 1 # a\n  b: 2\n",
        ),
        (
            "v: [[1], # a\n 2]\n",
            ".",
            &["-P"],
            "v:\n  - - 1\n  - 2 # a\n",
        ),
        (
            "v: [[1], # a\n [2]]\n",
            ".",
            &["-P"],
            "v:\n  - - 1\n  - # a\n    - 2\n",
        ),
        (
            "v: [{a: 1}, # a\n {b: 2}, # b\n 3]\n",
            ".",
            &["-P"],
            "v:\n  - a: 1\n  - # a\n    b: 2\n  - 3 # b\n",
        ),
        (
            "v: {a: [1], # a\n b: 2}\n",
            ".",
            &["-P"],
            "v:\n  a:\n    - 1\n  b: 2 # a\n",
        ),
        (
            "v: {a: {c: 1}, # a\n b: 2}\n",
            ".",
            &["-P"],
            "v:\n  a:\n    c: 1\n  b: 2 # a\n",
        ),
        (
            "v: [1, [2], # a\n 3]\n",
            ".",
            &["-P"],
            "v:\n  - 1\n  - - 2\n  - 3 # a\n",
        ),
        ("v: [1, [2] # a\n]\n", ".", &["-P"], "v:\n  - 1\n  - - 2\n"),
        (
            "v: [[~], # c7\n a , # c9\n []]\n",
            ".",
            &["-P"],
            "v:\n  - - ~\n  - a # c9\n  - []\n",
        ),
        (
            "v: [[1], # a\n b, # b\n c, # c\n d]\n",
            ".",
            &["-P"],
            "v:\n  - - 1\n  - b # b\n  - c # c\n  - d\n",
        ),
        (
            "a: [1] # a\nb: 2 # b\nc: [3]\nd: 4\n",
            ".",
            &["-P"],
            "a:\n  - 1\nb: 2 # b\nc: # a\n  - 3\nd: 4\n",
        ),
        (
            "a: {x: [1] # xi\n  }\nb: 3\n",
            ".",
            &["-P"],
            "a:\n  x:\n    - 1\nb: 3 # xi\n",
        ),
        (
            "a: [[1] # xi\n  ]\nb: 3\n",
            ".",
            &["-P"],
            "a:\n  - - 1\nb: 3 # xi\n",
        ),
        (
            "a: [[1] # xi\n  ] # ec\nb: 3\n",
            ".",
            &["-P"],
            "a:\n  - - 1\nb: 3 # ec\n",
        ),
        (
            "a: [[1], # xi\n  [2]] # ec\nb: 3\n",
            ".",
            &["-P"],
            "a:\n  - - 1\n  - # xi\n    - 2\nb: 3 # ec\n",
        ),
        (
            "v: {a: [1], # a\n b: 2, # b\n c: 3}\n",
            ".",
            &["-P"],
            "v:\n  a:\n    - 1\n  b: 2 # b\n  c: 3 # a\n",
        ),
        (
            "x: &x 5\nv: [[1], # a\n *x, 2]\n",
            ".",
            &["-P"],
            "x: &x 5\nv:\n  - - 1\n  - *x # a\n  - 2\n",
        ),
        (
            "x: &x 5\nv: {a: [1], # a\n b: *x, c: 2}\n",
            ".",
            &["-P"],
            "x: &x 5\nv:\n  a:\n    - 1\n  b: *x\n  c: 2 # a\n",
        ),
        (
            "x: &x 5\nv: [[1], # a\n *x # b\n, 2]\n",
            ".",
            &["-P"],
            "x: &x 5\nv:\n  - - 1\n  - *x # b\n  - 2\n",
        ),
        (
            "v: {a: {x: 1}, # a\n b: 2, # b\n c: 3}\n",
            ".",
            &["-P"],
            "v:\n  a:\n    x: 1\n  b: 2 # b\n  c: 3 # a\n",
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

/// What is left of #6334 for a comment on a line of its own, as `docs/compliance/yq/limitations.md` lists it:
/// each document below prints differently from yq v4.53.3 (the difference is named per row), so a change that
/// closes one fails here and should replace the row with yq's output.
#[test]
fn test_flow_comments_that_still_differ_from_yq_6334() -> Result<()> {
    for (doc, expected, yq_prints) in [
        // The comment on the line of a comma that starts a line after own-line comments is the next scalar's
        // line comment in yq.
        (
            "v: [\n  [ 's'\n  # c3\n  , # c4\n  ~, \"q\"\n  ]]\n",
            "v: [['s',\n    # c3\n    ~, \"q\"]]\n",
            "v: [['s',\n    # c3\n    ~, # c4\n    \"q\"]]\n",
        ),
        // A comment after a comma that follows own-line comments, before the closing bracket, is written by yq
        // after the collection.
        (
            "v: [a, b\n# f\n,\n# g\n]\nw: 1\n",
            "v: [a, b]\nw: 1\n",
            "v: [a, b]\n# g\n\nw: 1\n",
        ),
        // yq writes an empty line before the first comment of a collection after a sibling with a foot.
        (
            "v: [{k: 1\n  # c\n  }, [1,\n  # d\n  2]]\n",
            "v: [{k: 1,\n    # c\n}, [1,\n    # d\n    2]]\n",
            "v: [{k: 1,\n    # c\n}, [1,\n\n    # d\n    2]]\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, &[])?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{doc:?}");
        assert_ne!(stdout, yq_prints, "{doc:?} now matches yq");
    }
    // Under -P yq writes the foot of a last element that is a collection after the key line that follows it.
    let doc = "v: [{a: 1} # c\n  # f\n  ]\nw: [1] # z\ny: 2\n";
    let (stdout, code) = run_yq_stdin(".", doc, &["-P"])?;
    assert_eq!(
        (stdout.as_str(), code),
        ("v:\n  - a: 1\n\n# f\nw: # c\n  - 1\ny: 2 # z\n", 0),
        "-P on {doc:?}"
    );
    assert_ne!(stdout, "v:\n  - a: 1\nw: # c\n# f\n  - 1\ny: 2 # z\n");
    Ok(())
}
