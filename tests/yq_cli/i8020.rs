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
    // The root node itself still goes, as in yq.
    ("del(.. | select(kind == \"map\"))", ""),
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
fn del_through_key_deletes_the_slot_8020() -> Result<()> {
    for (filter, want) in KEY_ROWS {
        assert_row(&[filter], DOC, want)?;
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
