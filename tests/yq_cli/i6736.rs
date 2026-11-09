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
