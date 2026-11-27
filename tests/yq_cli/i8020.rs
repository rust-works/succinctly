//! #8020: yq's node metadata readers (`line`, `column`, `kind`, `style`, `tag`, `anchor`, `alias`,
//! the three comment readers, `path`, `document_index`/`di`, `file_index`/`fi`, `filename`) each
//! answer a fresh scalar built with `CreateReplacement`, which keeps its input's parent and key. As
//! a `del()` target such an answer is therefore its input's slot after a step down (`del(.b | line)`
//! deletes `.b`), and at the root a node of its own with no parent, which stops yq's back-to-front
//! delete loop (`deleteChildOperator`) without deleting anything: `del(.a, line)` keeps the
//! document, `del(line, .a)` deletes `.a`. `succinctly yq` resolved such a target on the cursor to
//! the path of the node it read, so `del(.a, document_index)` became `del(.a, .)` and printed
//! nothing, `-i` truncated the file to zero bytes, and `(.a, line) = 7` replaced the document with
//! `7`. Every expected output below was captured from yq v4.53.3; every reader answers every row
//! the same way.

use std::process::{Command, Stdio};

use anyhow::Result;

/// The document of the plain rows: comments, a flow sequence, an anchor and an alias.
const DOC: &str = "# head\na: 1 # la\nb: hi # lb\n# foot b\nc: [1, 2]\nd: &x {p: 1}\ne: *x\n";

/// Two documents, for the per-document and `--eval-all` rows.
const TWO: &str = "a: 1\nb: hi\n---\na: 2\nb: ho\n";

/// Every reader yq builds with `CreateReplacement`, spelled as a filter.
const READERS: &[&str] = &[
    "line",
    "column",
    "kind",
    "style",
    "tag",
    "anchor",
    "alias",
    "head_comment",
    "line_comment",
    "foot_comment",
    "path",
    "document_index",
    "di",
    "file_index",
    "fi",
    "filename",
];

/// (filter with `R` for the reader, yq's stdout over [`DOC`]).
const DOC_ROWS: &[(&str, &str)] = &[
    ("del(R)", DOC),
    ("del(. | R)", DOC),
    ("del(.a, R)", DOC),
    (
        "del(R, .a)",
        "# head\nb: hi # lb\n# foot b\nc: [1, 2]\nd: &x {p: 1}\ne: *x\n",
    ),
    (
        "del(.a, R, .c)",
        "# head\na: 1 # la\nb: hi # lb\nd: &x {p: 1}\ne: *x\n",
    ),
    (
        "del(.b | R)",
        "# head\na: 1 # la\n# foot b\nc: [1, 2]\nd: &x {p: 1}\ne: *x\n",
    ),
    (
        "del(.a, (.b | R))",
        "# head\n# foot b\nc: [1, 2]\nd: &x {p: 1}\ne: *x\n",
    ),
    (
        "del((.b | R), .a)",
        "# head\n# foot b\nc: [1, 2]\nd: &x {p: 1}\ne: *x\n",
    ),
    (
        "del(.c[] | R)",
        "# head\na: 1 # la\nb: hi # lb\n# foot b\nc: []\nd: &x {p: 1}\ne: *x\n",
    ),
    (
        "del(.c[0] | R)",
        "# head\na: 1 # la\nb: hi # lb\n# foot b\nc: [2]\nd: &x {p: 1}\ne: *x\n",
    ),
    (
        "del(.d.p | R)",
        "# head\na: 1 # la\nb: hi # lb\n# foot b\nc: [1, 2]\nd: &x {}\ne: *x\n",
    ),
    (
        "del(.e | R)",
        "# head\na: 1 # la\nb: hi # lb\n# foot b\nc: [1, 2]\nd: &x {p: 1}\n",
    ),
    ("del(.[] | R)", "# head\n{}\n"),
    // `..` emits the root first: every node below it goes, then the root's answer stops the loop.
    ("del(.. | R)", "# head\n{}\n"),
    (
        "del((.b, .c) | R)",
        "# head\na: 1 # la\nd: &x {p: 1}\ne: *x\n",
    ),
    // The root's answer is the last target, so it stops the loop before `.b` is reached.
    ("del((.b, .) | R)", DOC),
    (
        "del((., .b) | R)",
        "# head\na: 1 # la\n# foot b\nc: [1, 2]\nd: &x {p: 1}\ne: *x\n",
    ),
    (
        "del(.a | select(true) | R)",
        "# head\nb: hi # lb\n# foot b\nc: [1, 2]\nd: &x {p: 1}\ne: *x\n",
    ),
    // A write through a reader's answer writes that fresh node, not the document.
    (
        "(.a, R) = 7",
        "# head\na: 7 # la\nb: hi # lb\n# foot b\nc: [1, 2]\nd: &x {p: 1}\ne: *x\n",
    ),
    ("(.b | R) = 5", DOC),
    ("(.b | R) |= 5", DOC),
];

/// (flags, filter with `R`, yq's stdout over [`TWO`]).
const TWO_ROWS: &[(&[&str], &str, &str)] = &[
    (&[], "del(R)", TWO),
    (&[], "del(.a, R)", TWO),
    (&[], "del(.b | R)", "a: 1\n---\na: 2\n"),
    (&[], "del(.a, (.b | R))", "{}\n---\n{}\n"),
    (&["ea"], "del(R)", TWO),
    (&["ea"], "del(.a, R)", TWO),
    (&["ea"], "del(.b | R)", "a: 1\n---\na: 2\n"),
    (&["ea"], "del(.a, (.b | R))", "{}\n---\n{}\n"),
];

/// `key` answers its input's key node, which also stands in its input's slot, and nothing at the
/// root: (filter, yq's stdout over [`DOC`]).
const KEY_ROWS: &[(&str, &str)] = &[
    ("del(key)", DOC),
    (
        "del(.a, key)",
        "# head\nb: hi # lb\n# foot b\nc: [1, 2]\nd: &x {p: 1}\ne: *x\n",
    ),
    (
        "del(.b | key)",
        "# head\na: 1 # la\n# foot b\nc: [1, 2]\nd: &x {p: 1}\ne: *x\n",
    ),
    (
        "del(.a, (.b | key))",
        "# head\n# foot b\nc: [1, 2]\nd: &x {p: 1}\ne: *x\n",
    ),
    (
        "del(.c[0] | key)",
        "# head\na: 1 # la\nb: hi # lb\n# foot b\nc: [2]\nd: &x {p: 1}\ne: *x\n",
    ),
    (
        "del(.e | key)",
        "# head\na: 1 # la\nb: hi # lb\n# foot b\nc: [1, 2]\nd: &x {p: 1}\n",
    ),
    ("del(.. | key)", "# head\n{}\n"),
    ("del(.[] | key)", "# head\n{}\n"),
];

/// A double-quoted scalar, for the rows whose other target selects by style on the cursor.
const QUOTED: &str = "a: \"x\"\nb: hi\n";

/// A target that selects by node metadata is resolved on the cursor to the paths it selects, while
/// a reader or a literal beside it in a comma is left as written: its `path` is `[]`, the root's,
/// which as a target is the document. A root path from anything but navigation keeps the target as
/// written too. (filter, yq's stdout over [`QUOTED`].)
const CURSOR_ROWS: &[(&str, &str)] = &[
    ("del(1, (.. | select(style == \"x\")))", QUOTED),
    ("del(1, (.. | select(style == \"double\")))", "b: hi\n"),
    ("del((.. | select(style == \"double\")), 1)", QUOTED),
    ("del(line, (.. | select(style == \"double\")))", "b: hi\n"),
    ("del((.. | select(style == \"double\")), line)", QUOTED),
    (
        "(1, (.. | select(style == \"double\"))) = 9",
        "a: 9\nb: hi\n",
    ),
    (
        "((.. | select(style == \"double\")), line) = 9",
        "a: 9\nb: hi\n",
    ),
    ("(.. | select(style == \"double\")) = 9", "a: 9\nb: hi\n"),
    ("del(1 | select(kind == \"scalar\"))", QUOTED),
    ("del([.b] | select(kind == \"seq\"))", QUOTED),
    // The root node itself still goes, as in yq, through any navigation that reaches it.
    ("del(.. | select(kind == \"map\"))", ""),
    ("del((.b, .) | select(kind == \"map\"))", ""),
    ("del(.[]? | select(style == \"double\"))", "b: hi\n"),
    ("del(.a | select(style == \"double\"))", "b: hi\n"),
];

/// A `select` after a parentless target hands that node on, so the target still stops the loop:
/// (filter, yq's stdout over [`QUOTED`]).
const SELECT_ROWS: &[(&str, &str)] = &[
    ("del(.a, (1 | select(true)))", QUOTED),
    ("del((1 | select(true)), .a)", "b: hi\n"),
    ("del(.a, (1 | select(false)))", "b: hi\n"),
    ("del(.a, (line | select(true)))", QUOTED),
    ("del((line | select(true)), .a)", "b: hi\n"),
    ("del((kind | select(. == \"map\")), .a)", "b: hi\n"),
];

/// Scalars of three styles, one with a line comment: the documents of the review rows.
const STYLED: &str = "a: \"x\"\nb: 1\nc: 2 # hi\n";
/// A key whose value is a mapping.
const NESTED: &str = "a: 1\nb: {c: 2}\n";
/// An anchor, an alias to it, and a scalar.
const ALIASED: &str = "d: &x {p: 1, q: 2}\ne: *x\nf: 3\n";
/// A mapping, a sequence and the keys a fresh container would name.
const CONTAINERS: &str = "a: 0\nb: {c: 1}\nc: 5\narr: [1, 2]\n";

/// A metadata `select` before a reader is resolved on the cursor: the owned value answers every
/// reader's default, so its `select` would keep every node (#8020 review). `key` straight after
/// `key` answers nothing. (document, filter, yq's stdout.)
const REVIEW_ROWS: &[(&str, &str, &str)] = &[
    (
        STYLED,
        "del(.[] | select(style == \"\") | line)",
        "a: \"x\"\n",
    ),
    (
        STYLED,
        "del(.[] | select(line_comment == \"\") | line)",
        "c: 2 # hi\n",
    ),
    (
        STYLED,
        "del(.. | select(style == \"\") | line)",
        "a: \"x\"\n",
    ),
    (
        STYLED,
        "del(.[] | select(style == \"\") | kind)",
        "a: \"x\"\n",
    ),
    (
        STYLED,
        "del(.[] | select(style == \"\") | key | line)",
        "a: \"x\"\n",
    ),
    (
        STYLED,
        "del(.[] | select(style == \"double\") | key)",
        "b: 1\nc: 2 # hi\n",
    ),
    (
        STYLED,
        "del(.[] | select(style == \"double\") | line)",
        "b: 1\nc: 2 # hi\n",
    ),
    (
        STYLED,
        "del(.[] | select(line_comment == \"# hi\") | line)",
        STYLED,
    ),
    (
        STYLED,
        "del(.a, (.[] | select(style == \"\") | line))",
        "{}\n",
    ),
    (
        STYLED,
        "del((.[] | select(style == \"\") | line), .a)",
        "{}\n",
    ),
    (STYLED, "(.[] | select(style == \"\") | line) = 9", STYLED),
    // `key` renames the selected node's key; the root has none.
    (
        STYLED,
        "(.[] | select(style == \"double\") | key) = 9",
        "9: \"x\"\nb: 1\nc: 2 # hi\n",
    ),
    (
        STYLED,
        "(.[] | select(line_comment == \"# hi\") | key) = 9",
        STYLED,
    ),
    (
        STYLED,
        "((.a, .) | select(style == \"\") | key) = 9",
        STYLED,
    ),
    (STYLED, "del((.a, .) | select(style == \"\") | key)", STYLED),
    (
        STYLED,
        "del(.. | select(style == \"\") | key)",
        "a: \"x\"\n",
    ),
    // yq's `first(f)` is the first child for which `f` holds: a condition, resolved on the cursor.
    (STYLED, "del(first(.a, line))", "b: 1\nc: 2 # hi\n"),
    (
        STYLED,
        "del(first(style == \"double\"))",
        "b: 1\nc: 2 # hi\n",
    ),
    (
        STYLED,
        "(first(style == \"double\")) = 9",
        "a: 9\nb: 1\nc: 2 # hi\n",
    ),
    // A reader that only answers a node, beside navigation: a fresh node and the node itself.
    (STYLED, "del(.b | (style, .))", "a: \"x\"\nc: 2 # hi\n"),
    (NESTED, "del(.b | key | key)", NESTED),
    (NESTED, "del(.[] | key | key)", NESTED),
    (NESTED, "del(.. | key | key)", NESTED),
    (NESTED, "del(.b.c | key | key)", NESTED),
    (NESTED, "del(.b | key | key | line)", NESTED),
    (NESTED, "del(.b | key | line)", "a: 1\n"),
    (NESTED, "del(.b | line | key)", "a: 1\n"),
    (NESTED, "del(.b | key | line | key)", "a: 1\n"),
    (NESTED, "del(.b | key | kind)", "a: 1\n"),
    (NESTED, "del(.b | line | line)", "a: 1\n"),
    (ALIASED, "del(.. | line)", "{}\n"),
    (ALIASED, "del(.e | line)", "d: &x {p: 1, q: 2}\nf: 3\n"),
    (ALIASED, "del(.e.p | line)", "d: &x {q: 2}\ne: *x\nf: 3\n"),
    (ALIASED, "del(.e | .[] | line)", "d: &x {}\ne: *x\nf: 3\n"),
    (
        ALIASED,
        "del(.. | select(style == \"\") | line)",
        "d: &x {}\n",
    ),
    (
        ALIASED,
        "del(.e | .[] | select(style == \"\") | line)",
        "d: &x {}\ne: *x\nf: 3\n",
    ),
];

/// Shapes `succinctly yq` refuses where yq answers, the safe direction recorded in
/// `docs/compliance/yq/limitations.md`: a deriving stage after a metadata `select`, a fresh
/// container after one (#8037), whose `path` is not the node yq writes, and a metadata `select`
/// the cursor cannot resolve soundly. (document, filter.)
const REFUSED_ROWS: &[(&str, &str)] = &[
    // `eval(..)` runs its program detached from the document, so even the cursor answers
    // `eval("style")` with `""`: yq keeps `a: "x"` in both.
    (STYLED, "del(.[] | select(eval(\"style\") == \"\") | line)"),
    (STYLED, "del(.[] | select(eval(\"style\") == \"\"))"),
    (
        CONTAINERS,
        "(.b | select(kind == \"map\") | {\"a\": 1} | .a) = 9",
    ),
    (
        CONTAINERS,
        "(.b | select(kind == \"map\") | {\"a\": 1} | .a) |= 9",
    ),
    (
        CONTAINERS,
        "del(.b | select(kind == \"map\") | {\"a\": 1} | .a)",
    ),
    (
        CONTAINERS,
        "del(.arr | select(kind == \"seq\") | [.[]] | .[0])",
    ),
    (
        CONTAINERS,
        "del(.b | select(kind == \"map\") | with_entries(.) | .c)",
    ),
];

/// A key rename through a reader's answer (`.a | line | key`): yq renames `.a`'s key (`9: "x"`),
/// and `succinctly yq` writes nothing, as it did before (the safe direction, recorded).
/// (filter, `succinctly yq`'s stdout over [`STYLED`].)
const WRITES_LESS_ROWS: &[(&str, &str)] = &[(
    "(.[] | select(style == \"double\") | line | key) = 9",
    STYLED,
)];

/// A custom tag on one scalar, for `type`, which is yq's tag.
const TAGGED: &str = "a: !Foo x\nb: y\nc: 1\n";
/// Two members of a nested mapping, one double-quoted.
const INNER: &str = "x: {a: \"q\", b: 2}\n";

/// Selections the owned value cannot run exactly though they name no metadata reader: `type` (yq's
/// tag), a `..` below an alias and yq's `first(f)`, all resolved on the cursor (#8020 second
/// review). (document, filter, yq's stdout.)
const ALLOWLIST_ROWS: &[(&str, &str, &str)] = &[
    (
        TAGGED,
        "del(.[] | select(type == \"!!str\") | line)",
        "a: !Foo x\nc: 1\n",
    ),
    (
        TAGGED,
        "del(.[] | select(tag == \"!!str\") | line)",
        "a: !Foo x\nc: 1\n",
    ),
    (
        TAGGED,
        "del(.[] | select(type == \"!Foo\"))",
        "b: y\nc: 1\n",
    ),
    (
        TAGGED,
        "(.[] | select(type == \"!!str\")) = 9",
        "a: !Foo x\nb: 9\nc: 1\n",
    ),
    (
        ALIASED,
        "del(.e | .. | select(style == \"\"))",
        "d: &x {p: 1, q: 2}\nf: 3\n",
    ),
    (
        ALIASED,
        "del(.e | .. | select(style == \"\") | line)",
        "d: &x {p: 1, q: 2}\nf: 3\n",
    ),
    (ALIASED, "del(.e | .. | line)", "d: &x {p: 1, q: 2}\nf: 3\n"),
    (
        ALIASED,
        "del(.e | .. | tostring)",
        "d: &x {p: 1, q: 2}\nf: 3\n",
    ),
    (
        STYLED,
        "del(.[] | select(style == \"\") | tostring)",
        "a: \"x\"\n",
    ),
    (INNER, "del(.x | first(style == \"\"))", "x: {a: \"q\"}\n"),
    (
        INNER,
        "del(.x | first(style == \"double\") | line)",
        "x: {b: 2}\n",
    ),
];

/// An anchor, an alias to it and a mapping like it.
const ALIAS_LIKE: &str = "d: &x {p: 1}\ne: *x\nf: {p: 2}\n";
/// A merge key and an alias.
const MERGED: &str = "base: &b {x: 1, y: 2}\nm: {<<: *b, z: 3}\nal: *b\n";
/// Scalars whose spelling yq compares (`True`, `~`).
const SPELLED: &str = "a: True\nb: true\nc: ~\nd: null\n";

/// A values-only `select` the owned value answers differently than yq's document (#8020 third
/// review): yq's `has`, `length`, `tostring` and the string tests see an alias node itself, and yq
/// compares a scalar's text (`True == true` is false). Neither the owned value, which expanded the
/// alias and lost the spelling, nor the cursor, which reads the same values, can pick yq's nodes,
/// so the reader stays parentless and nothing is deleted, where yq deletes some (the safe
/// direction, recorded). (document, filter, `succinctly yq`'s stdout.)
const LESS_ROWS: &[(&str, &str, &str)] = &[
    (
        ALIAS_LIKE,
        "del(.[] | select(has(\"p\")) | line)",
        ALIAS_LIKE,
    ),
    (
        ALIAS_LIKE,
        "del(.[] | select(length == 1) | style)",
        ALIAS_LIKE,
    ),
    (
        ALIAS_LIKE,
        "del(.[] | select(tostring | test(\"p\")) | key)",
        ALIAS_LIKE,
    ),
    (
        ALIAS_LIKE,
        "del(.. | select(has(\"p\")) | line)",
        ALIAS_LIKE,
    ),
    // The merge key is printed expanded, as it was before this change for a `del()` that deletes
    // nothing.
    (
        MERGED,
        "del(.[] | select(has(\"x\")) | key)",
        "base: &b {x: 1, y: 2}\nm: {x: 1, y: 2, z: 3}\nal: *b\n",
    ),
    (
        MERGED,
        "del(.. | select(has(\"x\")) | line)",
        "base: &b {x: 1, y: 2}\nm: {x: 1, y: 2, z: 3}\nal: *b\n",
    ),
    (SPELLED, "del(.[] | select(. == true) | line)", SPELLED),
    (
        SPELLED,
        "del(.[] | select(tostring == \"null\") | line)",
        SPELLED,
    ),
];

fn run(args: &[&str], stdin: &str) -> Result<(String, String, i32)> {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin.as_bytes())?;
    let output = child.wait_with_output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        output.status.code().unwrap_or(-1),
    ))
}

fn assert_row(args: &[&str], stdin: &str, want: &str) -> Result<()> {
    let got = run(args, stdin)?;
    assert_eq!(
        (got.0.as_str(), got.1.as_str(), got.2),
        (want, "", 0),
        "{args:?} over {stdin:?}"
    );
    Ok(())
}

#[test]
fn a_metadata_reader_del_target_is_its_input_slot_or_a_parentless_stop_8020() -> Result<()> {
    for reader in READERS {
        for (form, want) in DOC_ROWS {
            assert_row(&[&form.replace('R', reader)], DOC, want)?;
        }
    }
    Ok(())
}

#[test]
fn a_metadata_reader_del_target_per_document_and_under_eval_all_8020() -> Result<()> {
    for reader in READERS {
        for (flags, form, want) in TWO_ROWS {
            let filter = form.replace('R', reader);
            let args: Vec<&str> = flags.iter().copied().chain([filter.as_str()]).collect();
            assert_row(&args, TWO, want)?;
        }
    }
    Ok(())
}

#[test]
fn a_metadata_reader_del_target_on_the_owned_route_8020() -> Result<()> {
    // `-o=json` and a `del` inside a collect leave the document route; `-n` has no document.
    for reader in READERS {
        assert_row(
            &["-o=json", "-I0", &format!("del(.a, {reader})")],
            DOC,
            "{\"a\":1,\"b\":\"hi\",\"c\":[1,2],\"d\":{\"p\":1},\"e\":{\"p\":1}}\n",
        )?;
        assert_row(
            &["-o=json", "-I0", &format!("[del(.a, (.b | {reader}))]")],
            DOC,
            "[{\"c\":[1,2],\"d\":{\"p\":1},\"e\":{\"p\":1}}]\n",
        )?;
        assert_row(
            &["-n", &format!("{{\"a\": 1, \"b\": 2}} | del(.a, {reader})")],
            "",
            "a: 1\nb: 2\n",
        )?;
        assert_row(
            &["-n", &format!("{{\"a\": 1, \"b\": 2}} | del({reader}, .a)")],
            "",
            "b: 2\n",
        )?;
    }
    Ok(())
}

#[test]
fn a_reader_or_literal_beside_a_cursor_selection_is_not_the_document_8020() -> Result<()> {
    for (filter, want) in CURSOR_ROWS {
        assert_row(&[filter], QUOTED, want)?;
    }
    Ok(())
}

#[test]
fn a_select_after_a_parentless_target_still_stops_the_loop_8020() -> Result<()> {
    for (filter, want) in SELECT_ROWS {
        assert_row(&[filter], QUOTED, want)?;
    }
    Ok(())
}

#[test]
fn del_through_key_deletes_the_slot_8020() -> Result<()> {
    for (filter, want) in KEY_ROWS {
        assert_row(&[filter], DOC, want)?;
    }
    Ok(())
}

#[test]
fn a_metadata_select_before_a_reader_tail_matches_yq_8020() -> Result<()> {
    for (doc, filter, want) in REVIEW_ROWS {
        assert_row(&[filter], doc, want)?;
    }
    Ok(())
}

#[test]
fn a_target_whose_path_is_not_yqs_node_is_refused_8020() -> Result<()> {
    for (doc, filter) in REFUSED_ROWS {
        let got = run(&[filter], doc)?;
        assert_eq!((got.0.as_str(), got.2), ("", 1), "{filter} over {doc:?}");
        assert!(
            got.1.starts_with("Error: Invalid path expression"),
            "{filter} over {doc:?}: {}",
            got.1
        );
    }
    Ok(())
}

#[test]
fn a_select_the_owned_value_reads_differently_deletes_less_than_yq_8020() -> Result<()> {
    for (doc, filter, want) in LESS_ROWS {
        assert_row(&[filter], doc, want)?;
    }
    Ok(())
}

#[test]
fn a_key_rename_through_a_reader_writes_less_than_yq_8020() -> Result<()> {
    for (filter, want) in WRITES_LESS_ROWS {
        assert_row(&[filter], STYLED, want)?;
    }
    Ok(())
}

#[test]
fn a_selection_beyond_values_is_resolved_on_the_cursor_8020() -> Result<()> {
    for (doc, filter, want) in ALLOWLIST_ROWS {
        assert_row(&[filter], doc, want)?;
    }
    Ok(())
}

/// A `def` (`--jq-extensions`; yq has none) whose body reads metadata: the owned value cannot run
/// it, so the reader stays parentless and nothing is deleted, never more than the inline
/// `select(style == "")` deletes (`a: "x"` stays either way).
#[test]
fn a_def_that_reads_metadata_deletes_nothing_through_a_reader_8020() -> Result<()> {
    let filter = "def f: style == \"\"; del(.[] | select(f) | line)";
    assert_row(&["--jq-extensions", filter], STYLED, STYLED)?;
    let file = tempfile::NamedTempFile::new()?;
    std::fs::write(file.path(), STYLED)?;
    let path = file.path().to_str().expect("utf-8 temp path");
    let got = run(&["--jq-extensions", "-i", filter, path], "")?;
    assert_eq!((got.0.as_str(), got.1.as_str(), got.2), ("", "", 0));
    assert_eq!(std::fs::read_to_string(file.path())?, STYLED);
    Ok(())
}

/// One `-i` row per review finding: the file holds yq's result, or is left as it was.
#[test]
fn inplace_review_rows_keep_the_file_8020() -> Result<()> {
    let rows: &[(&str, &str, &str, i32)] = &[
        (
            STYLED,
            "del(.[] | select(style == \"\") | line)",
            "a: \"x\"\n",
            0,
        ),
        (NESTED, "del(.b | key | key)", NESTED, 0),
        (
            ALIAS_LIKE,
            "del(.[] | select(has(\"p\")) | line)",
            ALIAS_LIKE,
            0,
        ),
        (SPELLED, "del(.[] | select(. == true) | line)", SPELLED, 0),
        (
            TAGGED,
            "del(.[] | select(type == \"!!str\") | line)",
            "a: !Foo x\nc: 1\n",
            0,
        ),
        (
            STYLED,
            "del(.[] | select(eval(\"style\") == \"\") | line)",
            STYLED,
            1,
        ),
        (
            STYLED,
            "(.[] | select(style == \"double\") | key) = 9",
            "9: \"x\"\nb: 1\nc: 2 # hi\n",
            0,
        ),
        (
            ALIASED,
            "del(.e | .. | line)",
            "d: &x {p: 1, q: 2}\nf: 3\n",
            0,
        ),
        (
            CONTAINERS,
            "del(.arr | select(kind == \"seq\") | [.[]] | .[0])",
            CONTAINERS,
            1,
        ),
        (
            CONTAINERS,
            "(.b | select(kind == \"map\") | {\"a\": 1} | .a) = 9",
            CONTAINERS,
            1,
        ),
    ];
    for (doc, filter, want, status) in rows {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), doc)?;
        let path = file.path().to_str().expect("utf-8 temp path");
        let got = run(&["-i", filter, path], "")?;
        assert_eq!(
            (got.0.as_str(), got.2),
            ("", *status),
            "{filter}: {}",
            got.1
        );
        assert_eq!(std::fs::read_to_string(file.path())?, *want, "{filter}");
    }
    Ok(())
}

/// The data loss itself: `-i` wrote the empty result back, leaving a zero-byte file.
#[test]
fn inplace_del_with_a_metadata_reader_keeps_the_file_8020() -> Result<()> {
    let rows: &[(&str, &str)] = &[
        ("del(.a, document_index)", DOC),
        ("del(.a, filename)", DOC),
        ("del(.a, line)", DOC),
        (
            "del(.a, (.b | kind))",
            "# head\n# foot b\nc: [1, 2]\nd: &x {p: 1}\ne: *x\n",
        ),
        ("del(.. | line)", "# head\n{}\n"),
    ];
    for (filter, want) in rows {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), DOC)?;
        let path = file.path().to_str().expect("utf-8 temp path");
        let got = run(&["-i", filter, path], "")?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            ("", "", 0),
            "{filter}"
        );
        let after = std::fs::read_to_string(file.path())?;
        assert!(!after.is_empty(), "{filter} emptied the file");
        assert_eq!(after, *want, "{filter}");
    }
    Ok(())
}
