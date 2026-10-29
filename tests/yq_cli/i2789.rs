//! #2789: `key | tostring` and `key | @format` over a string-spelled key hand the key's text to the
//! owned `tostring`/`@format` bypass (#2543) instead of re-materializing the key node through its
//! cursor. The bypass is a speed-only route, so every row here is the output the cursor route gave
//! (and, for the rows marked, real yq v4.53.3 gives).

use anyhow::Result;

use crate::run_yq_stdin;

const KEYS: &str =
    "plain: 1\n\"dq\": 2\nYes: 3\nname with space: 4\n\"\\u00e9\\u20ac\": 5\n\"\": 6\n";

fn lines(filter: &str, input: &str) -> Result<Vec<String>> {
    let (out, code) = run_yq_stdin(filter, input, &["-r"])?;
    assert_eq!(code, 0, "{filter}");
    Ok(out.lines().map(str::to_owned).collect())
}

/// Live yq v4.53.3 capture for every row.
#[test]
fn test_key_tostring_and_formats_over_string_keys_2789() -> Result<()> {
    let rows: &[(&str, &[&str])] = &[
        (
            ".[] | key | tostring",
            &[
                "plain",
                "dq",
                "Yes",
                "name with space",
                "\u{e9}\u{20ac}",
                "",
            ],
        ),
        (
            ".[] | key | @json",
            &[
                "\"plain\"",
                "\"dq\"",
                "\"Yes\"",
                "\"name with space\"",
                "\"\u{e9}\u{20ac}\"",
                "\"\"",
            ],
        ),
        (
            ".[] | key | @base64",
            &[
                "cGxhaW4=",
                "ZHE=",
                "WWVz",
                "bmFtZSB3aXRoIHNwYWNl",
                "w6nigqw=",
                "",
            ],
        ),
        (
            ".[] | key | @sh",
            &[
                "'plain'",
                "'dq'",
                "'Yes'",
                "'name with space'",
                "'\u{e9}\u{20ac}'",
                "''",
            ],
        ),
    ];
    for (filter, want) in rows {
        assert_eq!(&lines(filter, KEYS)?, want, "{filter}");
    }
    Ok(())
}

/// A key spelled like a null, bool or number keeps the cursor route, because `as_str` is the
/// scalar's text and not its type: `~` is a null and `01` an integer. This pins the *route
/// guard* (`key_spelling_may_retype`), not real yq's answer for these keys, which differs for
/// `~` (it comes back verbatim there; `01` has done since #2802 kept an integer's spelling).
#[test]
fn test_key_tostring_keeps_the_typed_key_route_2789() -> Result<()> {
    let input = "~: 1\n01: 2\ntrue: 3\nnull: 4\n";
    assert_eq!(
        lines(".[] | key | tostring", input)?,
        ["null", "01", "true", "null"]
    );
    Ok(())
}

/// `keys[]` reaches the same arm through the lazy-keys producer, which does not check a key's
/// explicit tag: a `!!null`/`!!bool` key must still come back typed (this is the cursor route's
/// answer, unchanged; real yq prints the raw text for `tostring`, a gap that is not this issue's),
/// while a plain string key and a quoted number-spelled one stay strings.
#[test]
fn test_keys_iteration_tostring_keeps_tagged_keys_typed_2789() -> Result<()> {
    let input = "plain: 1\n\"12\": 2\n!!null abc: 3\n!!bool xyz: 4\n";
    assert_eq!(
        lines("keys[] | tostring", input)?,
        ["plain", "12", "null", "false"]
    );
    assert_eq!(
        lines("keys[] | @json", input)?,
        ["\"plain\"", "\"12\"", "null", "false"]
    );
    Ok(())
}

/// #2789 cost 2: `.[] | key` emits the key node the iteration holds, not one reached by a sibling
/// hop. Every row is what yq v4.53.3 prints, and what the sibling-hop route printed before.
///
/// (name, document).
const KEY_NODE_DOCS: &[(&str, &str)] = &[
    (
        "plain",
        "# top\na: 1 # one\n# before b\nb: two\n  \nc:\n  x: 1\n",
    ),
    ("dup", "a: 1\na: 2\nb: 3\n"),
    (
        "merge",
        "base: &b\n  p: 1\n  q: 2\nuse:\n  <<: *b\n  r: 3\n",
    ),
    ("typed", "1: x\ntrue: y\nnull: z\n\"2\": w\n!!str 3: v\n"),
    ("anchor", "&k a: 1\n? b\n: 2\n'c': 3\n\"d\": 4\n"),
    ("multi", "? |\n  block\n: 1\nplain key: 2\n[a, b]: 3\n"),
    ("seq", "- a: 1\n  b: 2\n- c: 3\n"),
    ("flow", "{a: 1, b: 2, c: {d: 4}}\n"),
    ("ints", "-3: x # neg\n0x1f: y\n1: z\n007: q\n1.5: r\n"),
    ("intnest", "m:\n  -1: a\n  2: b\nlist: [x, y]\n"),
];

/// (document name, filter, expected `-o=json -I=0` output, exit code).
const KEY_NODE_ROWS: &[(&str, &str, &str, i32)] = &[
    ("plain", "[.[] | key]", "[\"a\",\"b\",\"c\"]\n", 0),
    ("plain", "[.[] | key | line]", "[1,3,5]\n", 0),
    ("plain", "[.[] | key | column]", "[1,1,1]\n", 0),
    (
        "plain",
        "[.[] | key | head_comment]",
        "[\"\",\"before b\",\"\"]\n",
        0,
    ),
    (
        "plain",
        "[.[] | key | line_comment]",
        "[\"\",\"\",\"\"]\n",
        0,
    ),
    ("plain", "[.[] | key | style]", "[\"\",\"\",\"\"]\n", 0),
    (
        "plain",
        "[.[] | key | tag]",
        "[\"!!str\",\"!!str\",\"!!str\"]\n",
        0,
    ),
    ("plain", "[.[] | key | anchor]", "[\"\",\"\",\"\"]\n", 0),
    (
        "plain",
        "[.[] | key | tostring]",
        "[\"a\",\"b\",\"c\"]\n",
        0,
    ),
    (
        "plain",
        "[.[] | key | path]",
        "[[\"a\"],[\"b\"],[\"c\"]]\n",
        0,
    ),
    ("plain", "[.[] | key | parent | length]", "[3,3,3]\n", 0),
    ("plain", "[.[] | key | key]", "[]\n", 0),
    ("plain", "[.[] | parent | length]", "[3,3,3]\n", 0),
    ("plain", "[.[] | key] | length", "3\n", 0),
    (
        "plain",
        "[.[] | select(. != 99) | key | line]",
        "[1,3,5]\n",
        0,
    ),
    (
        "plain",
        "[.[] | [key, line]]",
        "[[\"a\",1],[\"b\",3],[\"c\",6]]\n",
        0,
    ),
    ("plain", ".[] | key", "\"a\"\n\"b\"\n\"c\"\n", 0),
    ("plain", "[first(.[] | key)]", "[{\"x\":1}]\n", 0),
    ("plain", "[limit(2; .[] | key)]", "", 1),
    ("plain", "[.[][] | key]", "[\"x\"]\n", 0),
    ("plain", "[.a[]? | key]", "[]\n", 0),
    ("plain", "[.m[] | key]", "[]\n", 0),
    ("plain", "[.m[] | key | line]", "[]\n", 0),
    ("plain", "[.list[] | key]", "[]\n", 0),
    (
        "plain",
        "[.[] | (key | tostring)]",
        "[\"a\",\"b\",\"c\"]\n",
        0,
    ),
    ("plain", "[.[] | key] | length", "3\n", 0),
    ("dup", "[.[] | key]", "[\"a\",\"b\"]\n", 0),
    ("dup", "[.[] | key | line]", "[2,3]\n", 0),
    ("dup", "[.[] | key | column]", "[1,1]\n", 0),
    ("dup", "[.[] | key | head_comment]", "[\"\",\"\"]\n", 0),
    ("dup", "[.[] | key | line_comment]", "[\"\",\"\"]\n", 0),
    ("dup", "[.[] | key | style]", "[\"\",\"\"]\n", 0),
    ("dup", "[.[] | key | tag]", "[\"!!str\",\"!!str\"]\n", 0),
    ("dup", "[.[] | key | anchor]", "[\"\",\"\"]\n", 0),
    ("dup", "[.[] | key | tostring]", "[\"a\",\"b\"]\n", 0),
    ("dup", "[.[] | key | path]", "[[\"a\"],[\"b\"]]\n", 0),
    ("dup", "[.[] | key | parent | length]", "[3,3]\n", 0),
    ("dup", "[.[] | key | key]", "[]\n", 0),
    ("dup", "[.[] | parent | length]", "[3,3]\n", 0),
    ("dup", "[.[] | key] | length", "2\n", 0),
    ("dup", "[.[] | select(. != 99) | key | line]", "[2,3]\n", 0),
    ("dup", "[.[] | [key, line]]", "[[\"a\",2],[\"b\",3]]\n", 0),
    ("dup", ".[] | key", "\"a\"\n\"b\"\n", 0),
    ("dup", "[first(.[] | key)]", "[]\n", 0),
    ("dup", "[limit(2; .[] | key)]", "", 1),
    ("dup", "[.[][] | key]", "[]\n", 0),
    ("dup", "[.a[]? | key]", "[]\n", 0),
    ("dup", "[.m[] | key]", "[]\n", 0),
    ("dup", "[.m[] | key | line]", "[]\n", 0),
    ("dup", "[.list[] | key]", "[]\n", 0),
    ("dup", "[.[] | (key | tostring)]", "[\"a\",\"b\"]\n", 0),
    ("dup", "[.[] | key] | length", "2\n", 0),
    ("merge", "[.[] | key]", "[\"base\",\"use\"]\n", 0),
    ("merge", "[.[] | key | line]", "[1,4]\n", 0),
    ("merge", "[.[] | key | column]", "[1,1]\n", 0),
    ("merge", "[.[] | key | head_comment]", "[\"\",\"\"]\n", 0),
    ("merge", "[.[] | key | line_comment]", "[\"\",\"\"]\n", 0),
    ("merge", "[.[] | key | style]", "[\"\",\"\"]\n", 0),
    ("merge", "[.[] | key | tag]", "[\"!!str\",\"!!str\"]\n", 0),
    ("merge", "[.[] | key | anchor]", "[\"\",\"\"]\n", 0),
    ("merge", "[.[] | key | tostring]", "[\"base\",\"use\"]\n", 0),
    ("merge", "[.[] | key | path]", "[[\"base\"],[\"use\"]]\n", 0),
    ("merge", "[.[] | key | parent | length]", "[2,2]\n", 0),
    ("merge", "[.[] | key | key]", "[]\n", 0),
    ("merge", "[.[] | parent | length]", "[2,2]\n", 0),
    ("merge", "[.[] | key] | length", "2\n", 0),
    (
        "merge",
        "[.[] | select(. != 99) | key | line]",
        "[1,4]\n",
        0,
    ),
    (
        "merge",
        "[.[] | [key, line]]",
        "[[\"base\",1],[\"use\",5]]\n",
        0,
    ),
    ("merge", ".[] | key", "\"base\"\n\"use\"\n", 0),
    ("merge", "[first(.[] | key)]", "[{\"p\":1,\"q\":2}]\n", 0),
    ("merge", "[limit(2; .[] | key)]", "", 1),
    (
        "merge",
        "[.[][] | key]",
        "[\"p\",\"q\",\"p\",\"q\",\"r\"]\n",
        0,
    ),
    ("merge", "[.a[]? | key]", "[]\n", 0),
    ("merge", "[.m[] | key]", "[]\n", 0),
    ("merge", "[.m[] | key | line]", "[]\n", 0),
    ("merge", "[.list[] | key]", "[]\n", 0),
    (
        "merge",
        "[.[] | (key | tostring)]",
        "[\"base\",\"use\"]\n",
        0,
    ),
    ("merge", "[.[] | key] | length", "2\n", 0),
    ("typed", "[.[] | key]", "[1,true,null,\"2\",\"3\"]\n", 0),
    (
        "typed",
        "[.[] | key | head_comment]",
        "[\"\",\"\",\"\",\"\",\"\"]\n",
        0,
    ),
    (
        "typed",
        "[.[] | key | line_comment]",
        "[\"\",\"\",\"\",\"\",\"\"]\n",
        0,
    ),
    (
        "typed",
        "[.[] | key | tag]",
        "[\"!!int\",\"!!bool\",\"!!null\",\"!!str\",\"!!str\"]\n",
        0,
    ),
    (
        "typed",
        "[.[] | key | anchor]",
        "[\"\",\"\",\"\",\"\",\"\"]\n",
        0,
    ),
    (
        "typed",
        "[.[] | key | tostring]",
        "[\"1\",\"true\",\"null\",\"2\",\"3\"]\n",
        0,
    ),
    ("typed", "[.[] | key | parent | length]", "[5,5,5,5,5]\n", 0),
    ("typed", "[.[] | key | key]", "[]\n", 0),
    ("typed", "[.[] | parent | length]", "[5,5,5,5,5]\n", 0),
    ("typed", "[.[] | key] | length", "5\n", 0),
    (
        "typed",
        "[.[] | [key, line]]",
        "[[1,1],[true,2],[null,3],[\"2\",4],[\"3\",5]]\n",
        0,
    ),
    ("typed", ".[] | key", "1\ntrue\nnull\n\"2\"\n\"3\"\n", 0),
    ("typed", "[first(.[] | key)]", "[]\n", 0),
    ("typed", "[limit(2; .[] | key)]", "", 1),
    ("typed", "[.[][] | key]", "[]\n", 0),
    ("typed", "[.a[]? | key]", "[]\n", 0),
    ("typed", "[.m[] | key]", "[]\n", 0),
    ("typed", "[.m[] | key | line]", "[]\n", 0),
    ("typed", "[.list[] | key]", "[]\n", 0),
    (
        "typed",
        "[.[] | (key | tostring)]",
        "[\"1\",\"true\",\"null\",\"2\",\"3\"]\n",
        0,
    ),
    ("typed", "[.[] | key] | length", "5\n", 0),
    ("anchor", "[.[] | key]", "[\"a\",\"b\",\"c\",\"d\"]\n", 0),
    ("anchor", "[.[] | key | line]", "[1,2,4,5]\n", 0),
    ("anchor", "[.[] | key | column]", "[1,3,1,1]\n", 0),
    (
        "anchor",
        "[.[] | key | head_comment]",
        "[\"\",\"\",\"\",\"\"]\n",
        0,
    ),
    (
        "anchor",
        "[.[] | key | line_comment]",
        "[\"\",\"\",\"\",\"\"]\n",
        0,
    ),
    (
        "anchor",
        "[.[] | key | style]",
        "[\"\",\"\",\"single\",\"double\"]\n",
        0,
    ),
    (
        "anchor",
        "[.[] | key | tag]",
        "[\"!!str\",\"!!str\",\"!!str\",\"!!str\"]\n",
        0,
    ),
    (
        "anchor",
        "[.[] | key | anchor]",
        "[\"k\",\"\",\"\",\"\"]\n",
        0,
    ),
    (
        "anchor",
        "[.[] | key | tostring]",
        "[\"a\",\"b\",\"c\",\"d\"]\n",
        0,
    ),
    (
        "anchor",
        "[.[] | key | path]",
        "[[\"a\"],[\"b\"],[\"c\"],[\"d\"]]\n",
        0,
    ),
    ("anchor", "[.[] | key | parent | length]", "[4,4,4,4]\n", 0),
    ("anchor", "[.[] | key | key]", "[]\n", 0),
    ("anchor", "[.[] | parent | length]", "[4,4,4,4]\n", 0),
    ("anchor", "[.[] | key] | length", "4\n", 0),
    (
        "anchor",
        "[.[] | select(. != 99) | key | line]",
        "[1,2,4,5]\n",
        0,
    ),
    (
        "anchor",
        "[.[] | [key, line]]",
        "[[\"a\",1],[\"b\",3],[\"c\",4],[\"d\",5]]\n",
        0,
    ),
    ("anchor", ".[] | key", "\"a\"\n\"b\"\n\"c\"\n\"d\"\n", 0),
    ("anchor", "[first(.[] | key)]", "[]\n", 0),
    ("anchor", "[limit(2; .[] | key)]", "", 1),
    ("anchor", "[.[][] | key]", "[]\n", 0),
    ("anchor", "[.a[]? | key]", "[]\n", 0),
    ("anchor", "[.m[] | key]", "[]\n", 0),
    ("anchor", "[.m[] | key | line]", "[]\n", 0),
    ("anchor", "[.list[] | key]", "[]\n", 0),
    (
        "anchor",
        "[.[] | (key | tostring)]",
        "[\"a\",\"b\",\"c\",\"d\"]\n",
        0,
    ),
    ("anchor", "[.[] | key] | length", "4\n", 0),
    (
        "multi",
        "[.[] | key | head_comment]",
        "[\"\",\"\",\"\"]\n",
        0,
    ),
    (
        "multi",
        "[.[] | key | line_comment]",
        "[\"\",\"\",\"\"]\n",
        0,
    ),
    ("multi", "[.[] | key | anchor]", "[\"\",\"\",\"\"]\n", 0),
    (
        "multi",
        "[.[] | key | path]",
        "[[\"block\\n\"],[\"plain key\"],[\"\"]]\n",
        0,
    ),
    ("multi", "[.[] | key | parent | length]", "[3,3,3]\n", 0),
    ("multi", "[.[] | key | key]", "[]\n", 0),
    ("multi", "[.[] | parent | length]", "[3,3,3]\n", 0),
    ("multi", "[.[] | key] | length", "3\n", 0),
    ("multi", "[first(.[] | key)]", "[]\n", 0),
    ("multi", "[limit(2; .[] | key)]", "", 1),
    ("multi", "[.[][] | key]", "[]\n", 0),
    ("multi", "[.a[]? | key]", "[]\n", 0),
    ("multi", "[.m[] | key]", "[]\n", 0),
    ("multi", "[.m[] | key | line]", "[]\n", 0),
    ("multi", "[.list[] | key]", "[]\n", 0),
    ("multi", "[.[] | key] | length", "3\n", 0),
    ("seq", "[.[] | key]", "[0,1]\n", 0),
    ("seq", "[.[] | key | line]", "[0,0]\n", 0),
    ("seq", "[.[] | key | column]", "[0,0]\n", 0),
    ("seq", "[.[] | key | head_comment]", "[\"\",\"\"]\n", 0),
    ("seq", "[.[] | key | line_comment]", "[\"\",\"\"]\n", 0),
    ("seq", "[.[] | key | style]", "[\"\",\"\"]\n", 0),
    ("seq", "[.[] | key | tag]", "[\"!!int\",\"!!int\"]\n", 0),
    ("seq", "[.[] | key | anchor]", "[\"\",\"\"]\n", 0),
    ("seq", "[.[] | key | tostring]", "[\"0\",\"1\"]\n", 0),
    ("seq", "[.[] | key | parent | length]", "[2,2]\n", 0),
    ("seq", "[.[] | key | key]", "[]\n", 0),
    ("seq", "[.[] | parent | length]", "[2,2]\n", 0),
    ("seq", "[.[] | key] | length", "2\n", 0),
    ("seq", "[.[] | select(. != 99) | key | line]", "[0,0]\n", 0),
    ("seq", "[.[] | [key, line]]", "[[0,1],[1,3]]\n", 0),
    ("seq", ".[] | key", "0\n1\n", 0),
    ("seq", "[first(.[] | key)]", "[{\"a\":1,\"b\":2}]\n", 0),
    ("seq", "[limit(2; .[] | key)]", "", 1),
    ("seq", "[.[][] | key]", "[\"a\",\"b\",\"c\"]\n", 0),
    ("seq", "[.a[]? | key]", "", 1),
    ("seq", "[.m[] | key]", "", 1),
    ("seq", "[.m[] | key | line]", "", 1),
    ("seq", "[.list[] | key]", "", 1),
    ("seq", "[.[] | (key | tostring)]", "[\"0\",\"1\"]\n", 0),
    ("seq", "[.[] | key] | length", "2\n", 0),
    ("flow", "[.[] | key]", "[\"a\",\"b\",\"c\"]\n", 0),
    ("flow", "[.[] | key | line]", "[1,1,1]\n", 0),
    ("flow", "[.[] | key | column]", "[2,8,14]\n", 0),
    (
        "flow",
        "[.[] | key | head_comment]",
        "[\"\",\"\",\"\"]\n",
        0,
    ),
    (
        "flow",
        "[.[] | key | line_comment]",
        "[\"\",\"\",\"\"]\n",
        0,
    ),
    ("flow", "[.[] | key | style]", "[\"\",\"\",\"\"]\n", 0),
    (
        "flow",
        "[.[] | key | tag]",
        "[\"!!str\",\"!!str\",\"!!str\"]\n",
        0,
    ),
    ("flow", "[.[] | key | anchor]", "[\"\",\"\",\"\"]\n", 0),
    ("flow", "[.[] | key | tostring]", "[\"a\",\"b\",\"c\"]\n", 0),
    (
        "flow",
        "[.[] | key | path]",
        "[[\"a\"],[\"b\"],[\"c\"]]\n",
        0,
    ),
    ("flow", "[.[] | key | parent | length]", "[3,3,3]\n", 0),
    ("flow", "[.[] | key | key]", "[]\n", 0),
    ("flow", "[.[] | parent | length]", "[3,3,3]\n", 0),
    ("flow", "[.[] | key] | length", "3\n", 0),
    (
        "flow",
        "[.[] | select(. != 99) | key | line]",
        "[1,1,1]\n",
        0,
    ),
    (
        "flow",
        "[.[] | [key, line]]",
        "[[\"a\",1],[\"b\",1],[\"c\",1]]\n",
        0,
    ),
    ("flow", ".[] | key", "\"a\"\n\"b\"\n\"c\"\n", 0),
    ("flow", "[first(.[] | key)]", "[{\"d\":4}]\n", 0),
    ("flow", "[limit(2; .[] | key)]", "", 1),
    ("flow", "[.[][] | key]", "[\"d\"]\n", 0),
    ("flow", "[.a[]? | key]", "[]\n", 0),
    ("flow", "[.m[] | key]", "[]\n", 0),
    ("flow", "[.m[] | key | line]", "[]\n", 0),
    ("flow", "[.list[] | key]", "[]\n", 0),
    (
        "flow",
        "[.[] | (key | tostring)]",
        "[\"a\",\"b\",\"c\"]\n",
        0,
    ),
    ("flow", "[.[] | key] | length", "3\n", 0),
    ("ints", "[.[] | key]", "[-3,31,1,7,1.5]\n", 0),
    ("ints", "[.[] | key | line]", "[1,2,3,4,5]\n", 0),
    ("ints", "[.[] | key | column]", "[1,1,1,1,1]\n", 0),
    (
        "ints",
        "[.[] | key | head_comment]",
        "[\"\",\"\",\"\",\"\",\"\"]\n",
        0,
    ),
    (
        "ints",
        "[.[] | key | line_comment]",
        "[\"\",\"\",\"\",\"\",\"\"]\n",
        0,
    ),
    (
        "ints",
        "[.[] | key | style]",
        "[\"\",\"\",\"\",\"\",\"\"]\n",
        0,
    ),
    (
        "ints",
        "[.[] | key | tag]",
        "[\"!!int\",\"!!int\",\"!!int\",\"!!int\",\"!!float\"]\n",
        0,
    ),
    (
        "ints",
        "[.[] | key | anchor]",
        "[\"\",\"\",\"\",\"\",\"\"]\n",
        0,
    ),
    ("ints", "[.[] | key | parent | length]", "[5,5,5,5,5]\n", 0),
    ("ints", "[.[] | key | key]", "[]\n", 0),
    ("ints", "[.[] | parent | length]", "[5,5,5,5,5]\n", 0),
    ("ints", "[.[] | key] | length", "5\n", 0),
    ("ints", ".[] | key", "-3\n31\n1\n7\n1.5\n", 0),
    ("ints", "[first(.[] | key)]", "[]\n", 0),
    ("ints", "[limit(2; .[] | key)]", "", 1),
    ("ints", "[.[][] | key]", "[]\n", 0),
    ("ints", "[.a[]? | key]", "[]\n", 0),
    ("ints", "[.m[] | key]", "[]\n", 0),
    ("ints", "[.m[] | key | line]", "[]\n", 0),
    ("ints", "[.list[] | key]", "[]\n", 0),
    (
        "ints",
        "[.[] | (key | tostring)]",
        "[\"-3\",\"0x1f\",\"1\",\"007\",\"1.5\"]\n",
        0,
    ),
    ("ints", "[.[] | key] | length", "5\n", 0),
    ("intnest", "[.[] | key]", "[\"m\",\"list\"]\n", 0),
    ("intnest", "[.[] | key | line]", "[1,4]\n", 0),
    ("intnest", "[.[] | key | column]", "[1,1]\n", 0),
    ("intnest", "[.[] | key | head_comment]", "[\"\",\"\"]\n", 0),
    ("intnest", "[.[] | key | line_comment]", "[\"\",\"\"]\n", 0),
    ("intnest", "[.[] | key | style]", "[\"\",\"\"]\n", 0),
    ("intnest", "[.[] | key | tag]", "[\"!!str\",\"!!str\"]\n", 0),
    ("intnest", "[.[] | key | anchor]", "[\"\",\"\"]\n", 0),
    ("intnest", "[.[] | key | tostring]", "[\"m\",\"list\"]\n", 0),
    ("intnest", "[.[] | key | path]", "[[\"m\"],[\"list\"]]\n", 0),
    ("intnest", "[.[] | key | parent | length]", "[2,2]\n", 0),
    ("intnest", "[.[] | key | key]", "[]\n", 0),
    ("intnest", "[.[] | parent | length]", "[2,2]\n", 0),
    ("intnest", "[.[] | key] | length", "2\n", 0),
    (
        "intnest",
        "[.[] | select(. != 99) | key | line]",
        "[1,4]\n",
        0,
    ),
    (
        "intnest",
        "[.[] | [key, line]]",
        "[[\"m\",2],[\"list\",4]]\n",
        0,
    ),
    ("intnest", ".[] | key", "\"m\"\n\"list\"\n", 0),
    (
        "intnest",
        "[first(.[] | key)]",
        "[{\"-1\":\"a\",\"2\":\"b\"}]\n",
        0,
    ),
    ("intnest", "[limit(2; .[] | key)]", "", 1),
    ("intnest", "[.[][] | key]", "[-1,2,0,1]\n", 0),
    ("intnest", "[.a[]? | key]", "[]\n", 0),
    ("intnest", "[.m[] | key]", "[-1,2]\n", 0),
    ("intnest", "[.m[] | key | line]", "[2,3]\n", 0),
    ("intnest", "[.list[] | key]", "[0,1]\n", 0),
    (
        "intnest",
        "[.[] | (key | tostring)]",
        "[\"m\",\"list\"]\n",
        0,
    ),
    ("intnest", "[.[] | key] | length", "2\n", 0),
];

#[test]
fn test_key_after_iteration_is_the_members_key_node_2789() -> Result<()> {
    for (name, filter, want, want_code) in KEY_NODE_ROWS {
        let doc = KEY_NODE_DOCS
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, d)| *d)
            .expect("every row names a document");
        let (out, code) = run_yq_stdin(filter, doc, &["-o=json", "-I=0"])?;
        assert_eq!(
            (out.as_str(), code),
            (*want, *want_code),
            "{name}: {filter}"
        );
    }
    Ok(())
}
