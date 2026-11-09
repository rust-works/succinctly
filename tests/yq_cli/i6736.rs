//! #6736: a sequence the cursor streamers write (`map(.)`, `sort`, `reverse`, `select`, and an array
//! of nodes, #3475) keeps the comments between its items, as yq does. `stream_yaml_sequence` wrote
//! only each item's own line comment, so `- a` / `# c` / `- b` lost its `# c` and the owned route
//! was the only one that kept it. Every answer was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_streamed_sequences_keep_the_comments_between_items_6736() -> Result<()> {
    // (document, filter, yq's output)
    let rows: &[(&str, &str, &str)] = &[
        ("- a\n# c\n- b\n", "map(.)", "- a\n# c\n- b\n"),
        ("- a\n# c\n- b\n", "[.[]]", "- a\n# c\n- b\n"),
        ("- a\n# c\n- b\n", "reverse", "# c\n- b\n- a\n"),
        ("- a\n# c\n- b\n", "sort", "- a\n# c\n- b\n"),
        ("- a\n# c\n- b\n", "map(select(. != null))", "- a\n# c\n- b\n"),
        ("- a: 1\n  # foot a\n\n# head b\n- b: 2  # l\n- c\n# end\n", "map(.)", "- a: 1\n  # foot a\n# head b\n- b: 2 # l\n- c\n# end\n"),
        ("- a: 1\n  # foot a\n\n# head b\n- b: 2  # l\n- c\n# end\n", "[.[]]", "- a: 1\n  # foot a\n# head b\n- b: 2 # l\n- c\n# end\n"),
        ("-\n  # inner head\n  a: 1\n# between\n- # on dash\n  b: 2\n", "map(.)", "- # inner head\n  a: 1\n# between\n- # on dash\n  b: 2\n"),
        ("- a\n- # sc\n  b\n", "map(.)", "- a\n# sc\n- b\n"),
        ("- - a\n  # in\n  - b\n# out\n- c\n", "map(.)", "- - a\n  # in\n  - b\n# out\n- c\n"),
        ("- - a\n  # in\n  - b\n# out\n- c\n", "[.[]]", "- - a\n  # in\n  - b\n# out\n- c\n"),
        ("users:\n  - id: 1\n    name: Al # c\n    age: 31\n    # foot\n  # head2\n  - id: 2\n    name: Bo\n    age: 20\n  # last\n", "[.users[] | select(.age > 1)]", "- id: 1\n  name: Al # c\n  age: 31\n  # foot\n# head2\n- id: 2\n  name: Bo\n  age: 20\n"),
        ("users:\n  - id: 1\n    name: Al # c\n    age: 31\n    # foot\n  # head2\n  - id: 2\n    name: Bo\n    age: 20\n  # last\n", ".users | map(.)", "- id: 1\n  name: Al # c\n  age: 31\n  # foot\n# head2\n- id: 2\n  name: Bo\n  age: 20\n"),
        ("users:\n  - id: 1\n    name: Al # c\n    age: 31\n    # foot\n  # head2\n  - id: 2\n    name: Bo\n    age: 20\n  # last\n", ".users | reverse", "# head2\n- id: 2\n  name: Bo\n  age: 20\n- id: 1\n  name: Al # c\n  age: 31\n  # foot\n"),
        ("# head\n- a\n# c\n- b\n# end\n", "map(.)", "- a\n# c\n- b\n# end\n"),
        ("# head\n- a\n# c\n- b\n# end\n", "[.[]]", "- a\n# c\n- b\n# end\n"),
        ("# head\n- a\n# c\n- b\n# end\n", "[.]", "- - a\n  # c\n  - b\n  # end\n"),
    ];
    for &(doc, filter, want) in rows {
        let (out, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!((out.as_str(), code), (want, 0), "`{filter}` over {doc:?}");
    }
    Ok(())
}

#[test]
fn test_a_commented_array_of_nodes_streams_and_keeps_the_dom_route_for_the_document_6736(
) -> Result<()> {
    // `[.[]]` streams over a commented document now (#3475 kept the owned route for it); `[.]`
    // is the document itself, whose head and foot comments only the owned route writes.
    let doc = "# head\n- a\n# c\n- b\n# end\n";
    let (streamed, code) = run_yq_stdin("[.[]]", doc, &[])?;
    assert_eq!((streamed.as_str(), code), ("- a\n# c\n- b\n# end\n", 0));
    let (mapped, _) = run_yq_stdin("map(.)", doc, &[])?;
    assert_eq!(streamed, mapped);
    let whole_doc = "# head\n- a: 1  # line a\n  b: 2\n# between\n- a: 3\n\n- a: 4 # l4\n# foot\n";
    let (whole, code) = run_yq_stdin("[.]", whole_doc, &[])?;
    assert_eq!(
        (whole.as_str(), code),
        (
            "- - a: 1 # line a\n    b: 2\n  # between\n  - a: 3\n  - a: 4 # l4\n\n# foot\n",
            0
        )
    );
    Ok(())
}

#[test]
fn test_a_comment_free_sequence_is_written_as_before_6736() -> Result<()> {
    let (out, code) = run_yq_stdin("map(.)", "- a\n- b: 1\n- [c]\n", &[])?;
    assert_eq!((out.as_str(), code), ("- a\n- b: 1\n- [c]\n", 0));
    Ok(())
}

#[test]
fn test_a_streamed_sequence_is_written_as_dot_writes_it_6736() -> Result<()> {
    // `stream_yaml_sequence` and the `Sequence` arm of `.` each write the comments between items,
    // from different places; over a sequence with no header or document foot they must agree.
    for doc in [
        "- a\n# c\n- b\n",
        "- a: 1\n  # foot a\n\n# head b\n- b: 2  # l\n- c\n",
        "- - a\n  # in\n  - b\n# out\n- c\n",
        "-\n  # inner head\n  a: 1\n# between\n- # on dash\n  b: 2\n",
        "- a\n- # sc\n  b\n",
        "- a\n\n# c\n\n- b\n\n\n- c\n",
    ] {
        let (dot, _) = run_yq_stdin(".", doc, &[])?;
        let (mapped, _) = run_yq_stdin("map(.)", doc, &[])?;
        assert_eq!(mapped, dot, "{doc:?}");
    }
    Ok(())
}

#[test]
fn test_an_array_of_nodes_over_a_commented_document_matches_the_dom_route_6736() -> Result<()> {
    // Which arrays stream over a comment is decided by `array_holds_the_document`; `-P` forces the
    // DOM route, so this pins the decision to what is written, whatever the predicates say.
    let docs = [
        "# head\n- a: 1\n# between\n- b: x\n# end\n",
        "users:\n  - id: 1\n    # foot\n  # head2\n  - id: 2\n  # last\nother: 1\n",
    ];
    for doc in docs {
        for filter in [
            "[.]",
            "[.[]]",
            "[.[0]]",
            "[.users]",
            "[.users[]]",
            ".users | [.[]]",
            "[.[] | select(.id)]",
            "[.users[] | select(.id)]",
            "[select(.users)]",
            "[. | select(.[0])]",
            "[select(.[0])]",
        ] {
            let (streamed, code) = run_yq_stdin(filter, doc, &[])?;
            let (dom, dom_code) = run_yq_stdin(filter, doc, &["-P"])?;
            // A filter that does not fit the document fails the same way on both routes.
            assert_eq!(code, dom_code, "`{filter}` over {doc:?}");
            assert_eq!(streamed, dom, "`{filter}` over {doc:?}");
        }
    }
    Ok(())
}

#[test]
fn test_the_comments_between_items_follow_the_indent_and_the_other_callers_6736() -> Result<()> {
    let nested = "- - a\n  # in\n  - b\n# out\n- c\n";
    for indent in ["-I=2", "-I=3", "-I=4"] {
        let (out, code) = run_yq_stdin("map(.)", nested, &[indent])?;
        assert_eq!((out.as_str(), code), (nested, 0), "{indent}");
    }
    // `--slurp` writes document contents, which are not sequence items: unchanged.
    let docs = "# h\n- a\n# c\n- b\n---\n- c\n# d\n- d\n";
    let (out, code) = run_yq_stdin(".", docs, &["--slurp"])?;
    assert_eq!(
        (out.as_str(), code),
        ("- - a\n  # c\n  - b\n- - c\n  # d\n  - d\n", 0)
    );
    // An alias takes the DOM route over a comment, which expands it and keeps the comment.
    let aliased = "# c\nbase: &b {x: 1}\nitems:\n  - *b\n  # between\n  - y: 2\n";
    let (out, code) = run_yq_stdin("[.items[]]", aliased, &[])?;
    assert_eq!((out.as_str(), code), ("- x: 1\n# between\n- y: 2\n", 0));
    Ok(())
}

#[test]
fn test_what_a_streamed_sequence_still_does_not_write_6736() -> Result<()> {
    // Recorded divergences (docs/compliance/yq/limitations.md), pinned so a change to them is seen.
    // yq prints the `# c0` on a bare `-` that holds the sequence, ahead of its first element.
    let (out, code) = run_yq_stdin("[.[0][]]", "- # c0\n  - 1\n  # z\n  - 2\n  # tail\n", &[])?;
    assert_eq!((out.as_str(), code), ("- 1\n# z\n- 2\n# tail\n", 0));
    // yq prints the trailing `# e` after a reordered sequence of mappings.
    let (out, code) = run_yq_stdin("reverse", "- k: 3\n# c\n- k: 1\n# e\n", &[])?;
    assert_eq!((out.as_str(), code), ("# c\n- k: 1\n- k: 3\n", 0));
    Ok(())
}
