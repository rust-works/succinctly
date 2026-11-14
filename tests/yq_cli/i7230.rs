//! #7230: a scalar's own line comment, when a write replaces the scalar with a block collection, is
//! held by yq's encoder and written on a later line (`- 5 # own` / `- 6` written `.[0] = {"a": 1}` is
//! `- a: 1` / `- 6 # own`). It was dropped. go-yaml gives a collection's line comment to its end event
//! and holds what it cannot write yet in two slots: the pending comment, which the next sequence item
//! writes (and one with a comment of its own replaces), and the one a mapping key moves aside, which
//! waits beside the mapping's entries for a value that can take it. The same two slots decide where
//! `-P` writes the comment of a flow collection it turns into block style (#4377), whose one-slot model
//! kept the comment waiting past a commented item of a block sequence where yq loses it. Every row is
//! yq v4.53.3's output, captured by script.

use anyhow::Result;

use crate::run_yq_stdin;

fn assert_rows(rows: &[(&str, &str, &[&str], &str)]) -> Result<()> {
    for &(input, filter, flags, expected) in rows {
        let (out, status) = run_yq_stdin(filter, input, flags)?;
        assert_eq!(
            (out.as_str(), status),
            (expected, 0),
            "{filter} {flags:?} on {input:?}"
        );
    }
    Ok(())
}

/// A write that replaces a commented scalar with a block collection: the comment lands on the next
/// line that can take it, as in yq, and is lost when nothing follows.
#[test]
fn a_replaced_scalars_comment_is_held_for_a_later_line_7230() -> Result<()> {
    assert_rows(&[
        // The next item writes it at its end, or on its dash when it is a collection.
        (
            "- 5 # own\n- 6\n",
            ".[0] = {\"a\": 1}",
            &[],
            "- a: 1\n- 6 # own\n",
        ),
        ("- 5 # own\n- 6\n", ".[0] = [7]", &[], "- - 7\n- 6 # own\n"),
        ("- 5 # own\n", ".[0] = {\"a\": 1}", &[], "- a: 1\n"),
        (
            "- 5 # own\n- - 1\n  - 2\n",
            ".[0] = {\"a\": 1}",
            &[],
            "- a: 1\n- # own\n  - 1\n  - 2\n",
        ),
        (
            "- 5 # own\n- a: 1\n  b: 2\n",
            ".[0] = [1]",
            &[],
            "- - 1\n- # own\n  a: 1\n  b: 2\n",
        ),
        // An item with a comment of its own replaces it.
        (
            "- 5 # own\n- 6 # six\n- 7\n",
            ".[0] = {\"a\": 1}",
            &[],
            "- a: 1\n- 6 # six\n- 7\n",
        ),
        // The next item a write also replaced takes it on its dash.
        (
            "- 5 # own\n- 6\n- 7\n",
            ".[0] = {\"a\": 1} | .[1] = {\"b\": 2}",
            &[],
            "- a: 1\n- # own\n  b: 2\n- 7\n",
        ),
        // Beside a mapping's entries it waits for a value without a comment, a block scalar's header
        // or a block collection's key line, past a flow collection and a standalone comment.
        (
            "a: 5 # own\nb: 6\n",
            ".a = {\"x\": 1}",
            &[],
            "a:\n  x: 1\nb: 6 # own\n",
        ),
        (
            "a: 5 # own\nb: 6 # six\nc: 7\n",
            ".a = {\"x\": 1}",
            &[],
            "a:\n  x: 1\nb: 6 # six\nc: 7 # own\n",
        ),
        (
            "a: 5 # own\nb: {c: 1}\nz: 1\n",
            ".a = {\"x\": 1}",
            &[],
            "a:\n  x: 1\nb: {c: 1}\nz: 1 # own\n",
        ),
        (
            "a: 5 # own\nb:\n  c: 1\n",
            ".a = {\"x\": 1}",
            &[],
            "a:\n  x: 1\nb: # own\n  c: 1\n",
        ),
        (
            "a: 5 # own\nb: |\n  text\n",
            ".a = {\"x\": 1}",
            &[],
            "a:\n  x: 1\nb: | # own\n  text\n",
        ),
        (
            "a: 5 # own\n# head\nb: 6\n",
            ".a = {\"x\": 1}",
            &[],
            "a:\n  x: 1\n# head\nb: 6 # own\n",
        ),
        // A key the write added takes it too.
        (
            "a: 5 # own\n",
            ".a = {\"x\": 1} | .b = 2",
            &[],
            "a:\n  x: 1\nb: 2 # own\n",
        ),
        // A key's own comment replaces it.
        (
            "a: 5 # own\nk: # note\n  x: 1\nz: 1\n",
            ".a = {\"x\": 1}",
            &[],
            "a:\n  x: 1\nk: # note\n  x: 1\nz: 1\n",
        ),
        // It leaves the collection it was in.
        (
            "a:\n  - 5 # own\nz: 1\n",
            ".a[0] = {\"x\": 1}",
            &[],
            "a:\n  - x: 1\nz: 1 # own\n",
        ),
        (
            "k:\n  - 5 # own\n  - 6\n",
            ".k[0] = {\"a\": {\"b\": 1}}",
            &[],
            "k:\n  - a:\n      b: 1\n  - 6 # own\n",
        ),
        // An item the write added takes it too, and one a key moved aside waits past the end of its
        // mapping for a value in the next item, a write's new mapping or a nested sequence's included.
        (
            "- 5 # own\n",
            ".[0] = {\"a\": 1} | .[1] = 3",
            &[],
            "- a: 1\n- 3 # own\n",
        ),
        (
            "- a: 5 # own\n  b: 6 # six\n- c: 1\n",
            ".[0].a = {\"x\": 1}",
            &[],
            "- a:\n    x: 1\n  b: 6 # six\n- c: 1 # own\n",
        ),
        (
            "- a: 5 # own\n  b: 6 # six\n- c: 1\n- d: 2\n",
            ".[0].a = {\"x\": 1} | .[1] = {\"e\": {\"f\": 1}}",
            &[],
            "- a:\n    x: 1\n  b: 6 # six\n- e: # own\n    f: 1\n- d: 2\n",
        ),
        (
            "- a: 5 # own\n  b: 6 # six\n- - c: 1\n",
            ".[0].a = {\"x\": 1}",
            &[],
            "- a:\n    x: 1\n  b: 6 # six\n- - c: 1 # own\n",
        ),
        // A flow collection that a block mapping replaces, an update, and `-P`.
        (
            "a: [1, 2] # ec\nz: 1\n",
            ".a = {\"x\": 1}",
            &[],
            "a:\n  x: 1\nz: 1 # ec\n",
        ),
        (
            "a: 5 # own\nz: 1\n",
            ".a |= {\"x\": 1}",
            &[],
            "a:\n  x: 1\nz: 1 # own\n",
        ),
        (
            "a: 5 # own\nz: 1\n",
            ".a = {\"x\": 1}",
            &["-P"],
            "a:\n  x: 1\nz: 1 # own\n",
        ),
        // Nothing follows a document root; inside a flow collection the comment stays on its line.
        ("5 # own\n", ". = {\"a\": 1}", &[], "a: 1\n"),
        (
            "a: [5, # own\n  6]\nz: 1\n",
            ".a[0] = {\"x\": 1}",
            &[],
            "a: [{x: 1} # own\n, 6]\nz: 1\n",
        ),
    ])
}

/// `-P`'s held comment of a flow collection follows the same two slots: an item of a block sequence
/// with a comment of its own replaces it, an alias item takes it, and one a key moved aside waits
/// past the items of the sequence it ends in for the next mapping value.
#[test]
fn pretty_holds_a_flow_collections_comment_in_two_slots_7230() -> Result<()> {
    assert_rows(&[
        (
            "- [1] # ec\n- 6 # six\n- 7\n",
            ".",
            &["-P"],
            "- - 1\n- 6 # six\n- 7\n",
        ),
        (
            "- a: [1] # ec\n  b: 6 # six\n- 7\n- c: 8\n",
            ".",
            &["-P"],
            "- a:\n    - 1\n  b: 6 # six\n- 7\n- c: 8 # ec\n",
        ),
        (
            "- a: [1] # ec\n  b: 6 # six\n- [9] # nc\n- c: 8\n",
            ".",
            &["-P"],
            "- a:\n    - 1\n  b: 6 # six\n- - 9\n- # nc\n  c: 8 # ec\n",
        ),
        (
            "x: &x 1\nl:\n  - [1] # ec\n  - *x\n  - 3\n",
            ".",
            &["-P"],
            "x: &x 1\nl:\n  - - 1\n  - *x # ec\n  - 3\n",
        ),
        (
            "x: &x 1\na: [1] # ec\nb: *x\nc: 3\n",
            ".",
            &["-P"],
            "x: &x 1\na:\n  - 1\nb: *x\nc: 3 # ec\n",
        ),
        // A comment held beside a commented entry of a mapping that is a sequence item waits for a
        // mapping value, so the next item of the outer sequence does not take it (#6334's row,
        // which reached `- 2 # c5` before).
        (
            "- {k0: {k0: true}, # c5\n  m1: b # c6\n  }\n- 2\n",
            ".",
            &["-P"],
            "- k0:\n    k0: true\n  m1: b # c6\n- 2\n",
        ),
        (
            "a: [1] # ec\nb: [6] # bc\nc: 8\n",
            ".",
            &["-P"],
            "a:\n  - 1\nb: # ec\n  - 6\nc: 8 # bc\n",
        ),
    ])
}

/// Where yq writes the held comment on a dash and a flow collection on the next line at the
/// sequence's indentation (`- # own` / `{a: 1}`), output it cannot read back, the comment is written
/// beside the collection instead (docs/compliance/yq/limitations.md). An item with a comment of its
/// own keeps that one.
#[test]
fn a_flow_item_takes_the_held_comment_beside_it_7230() -> Result<()> {
    assert_rows(&[
        (
            "- 5 # own\n- {a: 1}\n",
            ".[0] = {\"b\": 1}",
            &[],
            "- b: 1\n- {a: 1} # own\n",
        ),
        (
            "- 5 # own\n- []\n- 1\n",
            ".[0] = {\"x\": 1}",
            &[],
            "- x: 1\n- [] # own\n- 1\n",
        ),
        (
            "- 5 # own\n- [1] # one\n",
            ".[0] = {\"x\": 1}",
            &[],
            "- x: 1\n- [1] # one\n",
        ),
        (
            "a: 5 # own\nb: 3\nz: 1\n",
            ".a = {\"x\": 1} | .b = {}",
            &[],
            "a:\n  x: 1\nb: {} # own\nz: 1\n",
        ),
    ])
}

/// The other encoders write the node's own comment where each writes a comment, as yq's do: only
/// the YAML encoder holds it for a later line. It was dropped.
#[test]
fn other_encoders_write_a_replaced_scalars_comment_on_the_node_7230() -> Result<()> {
    for (format, expected) in [
        ("xml", "<!-- own --><a>1</a>\n<a>2</a>\n<b>6</b>\n"),
        (
            "lua",
            "return {\n\t[\"a\"] = {\n\t\t1,\n\t\t2,\n\t}; -- own\n\t[\"b\"] = 6;\n};\n",
        ),
        (
            "kyaml",
            "{\n  a: [\n    1,\n    2,\n  ], # own\n  b: 6,\n}\n",
        ),
        ("toml", "a = [1, 2]  # own\nb = 6\n"),
    ] {
        let flag = format!("-o={format}");
        let (out, status) = run_yq_stdin(".a = [1, 2]", "a: 5 # own\nb: 6\n", &[flag.as_str()])?;
        assert_eq!((out.as_str(), status), (expected, 0), "{flag}");
    }
    Ok(())
}

/// A node navigated to after the write is printed as a document of its own: the comment held in it
/// lands on its own later line, and a comment held before it does not reach it.
#[test]
fn a_node_navigated_to_after_the_write_places_its_own_held_comments_7230() -> Result<()> {
    assert_rows(&[
        (
            "a:\n  b: 5 # own\n  c: 6\nz: 1\n",
            ".a.b = {\"x\": 1} | .a",
            &[],
            "b:\n  x: 1\nc: 6 # own\n",
        ),
        (
            "a: 5 # own\nb: 6\nc:\n  d: 1\n",
            ".a = {\"x\": 1} | .b",
            &[],
            "6\n",
        ),
        (
            "a: 5 # own\nb: 6\nc:\n  d: 1\n",
            ".a = {\"x\": 1} | .a",
            &[],
            "x: 1\n",
        ),
    ])
}

/// An entries operator around the write, whose presentation is replayed through it, holds the
/// comment the same way (`main` dropped it).
#[test]
fn a_held_comment_survives_an_entries_replay_7230() -> Result<()> {
    assert_rows(&[
        (
            "a: 5 # own\nb: 6\nc: 7\n",
            ".a = {\"x\": 1} | to_entries | from_entries",
            &[],
            "a:\n  x: 1\nb: 6 # own\nc: 7\n",
        ),
        (
            "a: 5 # own\nb: 6\nc: 7\n",
            "with_entries(select(.key != \"c\")) | .a = {\"x\": 1}",
            &[],
            "a:\n  x: 1\nb: 6 # own\n",
        ),
    ])
}

/// An empty flow `[]` / `{}` that a write fills is written in block style (#4849), and its comment is
/// held for a later line like any block collection's. It was dropped.
#[test]
fn a_filled_empty_flow_collections_comment_is_held_7230() -> Result<()> {
    assert_rows(&[
        (
            "a: [] # c\nb: 6\n",
            ".a = [1]",
            &[],
            "a:\n  - 1\nb: 6 # c\n",
        ),
        (
            "a: {} # c\nb: 6\n",
            ".a.x = 1",
            &[],
            "a:\n  x: 1\nb: 6 # c\n",
        ),
        ("- [] # c\n- 6\n", ".[0] += [1]", &[], "- - 1\n- 6 # c\n"),
    ])
}

/// #6954: under `-P` the held comment of a flow collection lands on a later item a write gave a new
/// kind, on its dash, and a comment held from a nested collection reaches it only as yq's two slots
/// say (`.[4] = [7]`, an append, leaves `# c1` where the item before it puts it).
#[test]
fn pretty_places_a_held_comment_on_a_kind_changed_item_6954() -> Result<()> {
    let nested = "- [{y: ~}] # c8\n- - {c: ~} # c1\n- true # c5\n- {y: 2}\n";
    assert_rows(&[
        (
            "- [1, 2] # ec\n- 5\n",
            ".[1] = {\"a\": 1}",
            &["-P"],
            "- - 1\n  - 2\n- # ec\n  a: 1\n",
        ),
        (
            "- [1, 2] # ec\n- 5\n",
            ".[1] = [7]",
            &["-P"],
            "- - 1\n  - 2\n- # ec\n  - 7\n",
        ),
        (
            nested,
            ".[2] = [7]",
            &["-P"],
            "- - y: ~\n- # c8\n  - c: ~\n- # c1\n  - 7\n- # c5\n  y: 2\n",
        ),
        (
            nested,
            ".[4] = [7]",
            &["-P"],
            "- - y: ~\n- # c8\n  - c: ~\n- true # c5\n- y: 2\n- - 7\n",
        ),
    ])
}
