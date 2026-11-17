//! #7089: behind a byte-order mark, the block under the first `---` that a blank line detaches from
//! the body is the foot of the first key, item or scalar (the document's foot for a scalar), and is
//! dropped before a flow collection, where the block above the marker stays the head. It was placed
//! in the head with the block above (`# c1` / `---` / `# c2` / blank / `a: 1` was `c1\nc2\n` on `.a`'s key's
//! head). Blocks after the first under the marker stay the head, and a `%TAG` directive above it counts as
//! something above. A marker at the very start, a block with no blank line under it, and a file with no
//! mark are pinned unchanged.
//!
//! Every row is yq v4.53.3's output for `F` (the head and foot comment of the document and of every
//! key and item, `-o=json -I0`), `.` and `-o=props .`.

use anyhow::Result;

use crate::run_yq_stdin;

const F: &str = r#"[head_comment, foot_comment] + [.. | select(tag=="!!map") | keys[] | [head_comment, foot_comment]] + [.. | select(tag=="!!seq") | .[] | [head_comment, foot_comment]]"#;

#[test]
fn the_block_under_a_first_marker_is_the_first_nodes_foot_after_a_byte_order_mark() -> Result<()> {
    let rows: [(bool, &str, [&str; 3]); 54] = [
        (
            true,
            "# c1\n---\n# c2\n\na: 1\n",
            [
                "[\"\",\"\",[\"c1\",\"c2\"],[]]\n",
                "# c1\na: 1\n# c2\n",
                "# c1\na = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\n\na: 1 # one\n",
            [
                "[\"\",\"\",[\"c1\",\"c2\"],[]]\n",
                "# c1\na: 1 # one\n# c2\n",
                "# c1 one\na = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\n\na:\n  b: 1\n",
            [
                "[\"\",\"\",[\"c1\",\"c2\"],[\"\",\"\"],[]]\n",
                "# c1\na:\n  b: 1\n# c2\n",
                "a.b = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\n\n- 1\n",
            [
                "[\"\",\"\",[],[\"c1\",\"c2\"]]\n",
                "# c1\n- 1\n# c2\n",
                "# c1\n0 = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\n\n5\n",
            ["[\"c1\",\"c2\",[],[]]\n", "5\n", "5\n"],
        ),
        (
            true,
            "# c1\n---\n# c2\n\n{a: 1}\n",
            [
                "[\"c1\",\"\",[\"\",\"\"],[]]\n",
                "# c1\n{a: 1}\n",
                "a = 1\n",
            ],
        ),
        (
            true,
            "\n---\n# c1\n\na: 1\n",
            ["[\"\",\"\",[\"\",\"c1\"],[]]\n", "a: 1\n# c1\n", "a = 1\n"],
        ),
        (
            true,
            "\n---\n# c1\n\na: 1 # one\n",
            [
                "[\"\",\"\",[\"\",\"c1\"],[]]\n",
                "a: 1 # one\n# c1\n",
                "# one\na = 1\n",
            ],
        ),
        (
            true,
            "\n---\n# c1\n\na:\n  b: 1\n",
            [
                "[\"\",\"\",[\"\",\"c1\"],[\"\",\"\"],[]]\n",
                "a:\n  b: 1\n# c1\n",
                "a.b = 1\n",
            ],
        ),
        (
            true,
            "\n---\n# c1\n\n- 1\n",
            ["[\"\",\"\",[],[\"\",\"c1\"]]\n", "- 1\n# c1\n", "0 = 1\n"],
        ),
        (
            true,
            "\n---\n# c1\n\n5\n",
            ["[\"\",\"c1\",[],[]]\n", "5\n", "5\n"],
        ),
        (
            true,
            "\n---\n# c1\n\n{a: 1}\n",
            ["[\"\",\"\",[\"\",\"\"],[]]\n", "{a: 1}\n", "a = 1\n"],
        ),
        (
            false,
            "# c1\n---\n# c2\n\na: 1\n",
            [
                "[\"c1\\nc2\\n\",\"\",[\"\",\"\"],[]]\n",
                "# c1\n---\n# c2\n\na: 1\n",
                "# c1\n# c2\n\na = 1\n",
            ],
        ),
        (
            false,
            "# c1\n---\n# c2\n\na: 1 # one\n",
            [
                "[\"c1\\nc2\\n\",\"\",[\"\",\"\"],[]]\n",
                "# c1\n---\n# c2\n\na: 1 # one\n",
                "# c1\n# c2\n\n# one\na = 1\n",
            ],
        ),
        (
            false,
            "# c1\n---\n# c2\n\na:\n  b: 1\n",
            [
                "[\"c1\\nc2\\n\",\"\",[\"\",\"\"],[\"\",\"\"],[]]\n",
                "# c1\n---\n# c2\n\na:\n  b: 1\n",
                "# c1\n# c2\n\na.b = 1\n",
            ],
        ),
        (
            false,
            "# c1\n---\n# c2\n\n- 1\n",
            [
                "[\"c1\\nc2\\n\",\"\",[],[\"\",\"\"]]\n",
                "# c1\n---\n# c2\n\n- 1\n",
                "# c1\n# c2\n\n0 = 1\n",
            ],
        ),
        (
            false,
            "# c1\n---\n# c2\n\n5\n",
            [
                "[\"c1\\nc2\\n\",\"\",[],[]]\n",
                "# c1\n---\n# c2\n\n5\n",
                "# c1\n# c2\n\n5\n",
            ],
        ),
        (
            false,
            "# c1\n---\n# c2\n\n{a: 1}\n",
            [
                "[\"c1\\nc2\\n\",\"\",[\"\",\"\"],[]]\n",
                "# c1\n---\n# c2\n\n{a: 1}\n",
                "# c1\n# c2\n\na = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\na: 1\n",
            [
                "[\"\",\"\",[\"c1\\nc2\",\"\"],[]]\n",
                "# c1\n# c2\na: 1\n",
                "# c1\n# # c2\na = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\na: 1 # one\n",
            [
                "[\"\",\"\",[\"c1\\nc2\",\"\"],[]]\n",
                "# c1\n# c2\na: 1 # one\n",
                "# c1\n# # c2 one\na = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\na:\n  b: 1\n",
            [
                "[\"\",\"\",[\"c1\\nc2\",\"\"],[\"\",\"\"],[]]\n",
                "# c1\n# c2\na:\n  b: 1\n",
                "a.b = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\n- 1\n",
            [
                "[\"\",\"\",[],[\"c1\\nc2\",\"\"]]\n",
                "# c1\n# c2\n- 1\n",
                "# c1\n# # c2\n0 = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\n5\n",
            ["[\"c1\\nc2\",\"\",[],[]]\n", "5\n", "5\n"],
        ),
        (
            true,
            "# c1\n---\n# c2\n{a: 1}\n",
            [
                "[\"c1\\nc2\",\"\",[\"\",\"\"],[]]\n",
                "# c1\n# c2\n{a: 1}\n",
                "a = 1\n",
            ],
        ),
        (
            true,
            "---\n# c1\n\na: 1\n",
            [
                "[\"\",\"\",[\"c1\\n\",\"\"],[]]\n",
                "# c1\na: 1\n",
                "# c1\n# \na = 1\n",
            ],
        ),
        (
            true,
            "---\n# c1\n\na: 1 # one\n",
            [
                "[\"\",\"\",[\"c1\\n\",\"\"],[]]\n",
                "# c1\na: 1 # one\n",
                "# c1\n#  one\na = 1\n",
            ],
        ),
        (
            true,
            "---\n# c1\n\na:\n  b: 1\n",
            [
                "[\"\",\"\",[\"c1\\n\",\"\"],[\"\",\"\"],[]]\n",
                "# c1\na:\n  b: 1\n",
                "a.b = 1\n",
            ],
        ),
        (
            true,
            "---\n# c1\n\n- 1\n",
            [
                "[\"\",\"\",[],[\"c1\\n\",\"\"]]\n",
                "# c1\n- 1\n",
                "# c1\n# \n0 = 1\n",
            ],
        ),
        (
            true,
            "---\n# c1\n\n5\n",
            ["[\"c1\\n\",\"\",[],[]]\n", "5\n", "5\n"],
        ),
        (
            true,
            "---\n# c1\n\n{a: 1}\n",
            [
                "[\"c1\\n\",\"\",[\"\",\"\"],[]]\n",
                "# c1\n{a: 1}\n",
                "a = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\n\n# c3\n\na: 1\n",
            [
                "[\"\",\"\",[\"c1\\nc3\\n\",\"c2\"],[]]\n",
                "# c1\n# c3\na: 1\n# c2\n",
                "# c1\n# # c3\n# \na = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\n\n# c3\n\na: 1 # one\n",
            [
                "[\"\",\"\",[\"c1\\nc3\\n\",\"c2\"],[]]\n",
                "# c1\n# c3\na: 1 # one\n# c2\n",
                "# c1\n# # c3\n#  one\na = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\n\n# c3\n\na:\n  b: 1\n",
            [
                "[\"\",\"\",[\"c1\\nc3\\n\",\"c2\"],[\"\",\"\"],[]]\n",
                "# c1\n# c3\na:\n  b: 1\n# c2\n",
                "a.b = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\n\n# c3\n\n- 1\n",
            [
                "[\"\",\"\",[],[\"c1\\nc3\\n\",\"c2\"]]\n",
                "# c1\n# c3\n- 1\n# c2\n",
                "# c1\n# # c3\n# \n0 = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\n\n# c3\n\n5\n",
            ["[\"c1\\nc3\\n\",\"c2\",[],[]]\n", "5\n", "5\n"],
        ),
        (
            true,
            "# c1\n---\n# c2\n\n# c3\n\n{a: 1}\n",
            [
                "[\"c1\\nc3\\n\",\"\",[\"\",\"\"],[]]\n",
                "# c1\n# c3\n{a: 1}\n",
                "a = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\n\n# c3\n\n# c4\n\na: 1\n",
            [
                "[\"\",\"\",[\"c1\\nc3\\n\\nc4\\n\",\"c2\"],[]]\n",
                "# c1\n# c3\n\n# c4\na: 1\n# c2\n",
                "# c1\n# # c3\n# \n# # c4\n# \na = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\n\n# c3\n\n# c4\n\na: 1 # one\n",
            [
                "[\"\",\"\",[\"c1\\nc3\\n\\nc4\\n\",\"c2\"],[]]\n",
                "# c1\n# c3\n\n# c4\na: 1 # one\n# c2\n",
                "# c1\n# # c3\n# \n# # c4\n#  one\na = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\n\n# c3\n\n# c4\n\na:\n  b: 1\n",
            [
                "[\"\",\"\",[\"c1\\nc3\\n\\nc4\\n\",\"c2\"],[\"\",\"\"],[]]\n",
                "# c1\n# c3\n\n# c4\na:\n  b: 1\n# c2\n",
                "a.b = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\n\n# c3\n\n# c4\n\n- 1\n",
            [
                "[\"\",\"\",[],[\"c1\\nc3\\n\\nc4\\n\",\"c2\"]]\n",
                "# c1\n# c3\n\n# c4\n- 1\n# c2\n",
                "# c1\n# # c3\n# \n# # c4\n# \n0 = 1\n",
            ],
        ),
        (
            true,
            "# c1\n---\n# c2\n\n# c3\n\n# c4\n\n5\n",
            ["[\"c1\\nc3\\n\\nc4\\n\",\"c2\",[],[]]\n", "5\n", "5\n"],
        ),
        (
            true,
            "# c1\n---\n# c2\n\n# c3\n\n# c4\n\n{a: 1}\n",
            [
                "[\"c1\\nc3\\n\\nc4\\n\",\"\",[\"\",\"\"],[]]\n",
                "# c1\n# c3\n\n# c4\n{a: 1}\n",
                "a = 1\n",
            ],
        ),
        (
            true,
            "\n---\n# c1\n\n# c2\n\na: 1\n",
            [
                "[\"\",\"\",[\"c2\\n\",\"c1\"],[]]\n",
                "# c2\na: 1\n# c1\n",
                "# c2\n# \na = 1\n",
            ],
        ),
        (
            true,
            "\n---\n# c1\n\n# c2\n\na: 1 # one\n",
            [
                "[\"\",\"\",[\"c2\\n\",\"c1\"],[]]\n",
                "# c2\na: 1 # one\n# c1\n",
                "# c2\n#  one\na = 1\n",
            ],
        ),
        (
            true,
            "\n---\n# c1\n\n# c2\n\na:\n  b: 1\n",
            [
                "[\"\",\"\",[\"c2\\n\",\"c1\"],[\"\",\"\"],[]]\n",
                "# c2\na:\n  b: 1\n# c1\n",
                "a.b = 1\n",
            ],
        ),
        (
            true,
            "\n---\n# c1\n\n# c2\n\n- 1\n",
            [
                "[\"\",\"\",[],[\"c2\\n\",\"c1\"]]\n",
                "# c2\n- 1\n# c1\n",
                "# c2\n# \n0 = 1\n",
            ],
        ),
        (
            true,
            "\n---\n# c1\n\n# c2\n\n5\n",
            ["[\"c2\\n\",\"c1\",[],[]]\n", "5\n", "5\n"],
        ),
        (
            true,
            "\n---\n# c1\n\n# c2\n\n{a: 1}\n",
            [
                "[\"c2\\n\",\"\",[\"\",\"\"],[]]\n",
                "# c2\n{a: 1}\n",
                "a = 1\n",
            ],
        ),
        (
            true,
            "%TAG ! tag:example.com,2000:\n---\n# c\n\na: 1\n",
            ["[\"\",\"\",[\"\",\"c\"],[]]\n", "a: 1\n# c\n", "a = 1\n"],
        ),
        (
            true,
            "%TAG ! tag:example.com,2000:\n---\n# c\n\na: 1 # one\n",
            [
                "[\"\",\"\",[\"\",\"c\"],[]]\n",
                "a: 1 # one\n# c\n",
                "# one\na = 1\n",
            ],
        ),
        (
            true,
            "%TAG ! tag:example.com,2000:\n---\n# c\n\na:\n  b: 1\n",
            [
                "[\"\",\"\",[\"\",\"c\"],[\"\",\"\"],[]]\n",
                "a:\n  b: 1\n# c\n",
                "a.b = 1\n",
            ],
        ),
        (
            true,
            "%TAG ! tag:example.com,2000:\n---\n# c\n\n- 1\n",
            ["[\"\",\"\",[],[\"\",\"c\"]]\n", "- 1\n# c\n", "0 = 1\n"],
        ),
        (
            true,
            "%TAG ! tag:example.com,2000:\n---\n# c\n\n5\n",
            ["[\"\",\"c\",[],[]]\n", "5\n", "5\n"],
        ),
        (
            true,
            "%TAG ! tag:example.com,2000:\n---\n# c\n\n{a: 1}\n",
            ["[\"\",\"\",[\"\",\"\"],[]]\n", "{a: 1}\n", "a = 1\n"],
        ),
    ];
    for (bom, text, want) in rows {
        let input = if bom {
            format!("\u{feff}{text}")
        } else {
            text.to_string()
        };
        let runs: [(&str, &[&str]); 3] =
            [(F, &["-o=json", "-I0"]), (".", &[]), (".", &["-o=props"])];
        for ((filter, args), want) in runs.into_iter().zip(want) {
            let (stdout, code) = run_yq_stdin(filter, &input, args)?;
            assert_eq!(code, 0, "{text:?}: {stdout}");
            assert_eq!(stdout, want, "{filter} {args:?} of {text:?} (bom: {bom})");
        }
    }
    Ok(())
}

// Family 4 (slice 3): the head comment of a scalar root after a byte-order mark.
//
// After a mark yq sets no header aside, so the comment ahead of a document's content is the head of
// its first node, and yq's encoder writes a head comment for a collection and not for a scalar root.
// `# lead` / `5` is `5` under `-P .`, `. = 6` is `6` whatever the root was (a mapping or sequence that
// had the comment included), and `.a = 1` over the scalar leaves `5`; the comment stayed as the head
// here. A plain `.` prints none either, `head_comment` still reads it, and a result that is a
// collection keeps it. Rows are yq v4.53.3's output on the input behind a mark.

fn assert_rows_after_bom(rows: &[(&str, &[&str], &str, &str)]) -> Result<()> {
    for &(input, flags, filter, expected) in rows {
        let (out, status) = run_yq_stdin(filter, &format!("\u{feff}{input}"), flags)?;
        assert_eq!(
            (out.as_str(), status),
            (expected, 0),
            "{filter} {flags:?} on {input:?}"
        );
    }
    Ok(())
}

/// A scalar result written as the first document's root drops the comment the mark left on the node.
#[test]
fn a_scalar_root_after_a_bom_writes_no_head_comment_7089() -> Result<()> {
    assert_rows_after_bom(&[
        ("# lead\n5\n", &["-P"], ".", "5\n"),
        ("# lead\n5\n", &[], ". = 6", "6\n"),
        ("# lead\n5\n", &[], ". |= 6", "6\n"),
        ("# lead\n5\n", &[], ".a = 1", "5\n"),
        ("# lead\n5\n", &[], ". style=\"double\"", "5\n"),
        ("# lead\n\"s\"\n", &["-P"], ".", "s\n"),
        ("# lead\n\"s\"\n", &[], ". = 6", "6\n"),
        ("# lead\nnull\n", &["-P"], ".", "null\n"),
        ("# lead\ntrue\n", &["-P"], ".", "true\n"),
        ("# l1\n\n# l2\n5\n", &["-P"], ".", "5\n"),
        ("# a\n# b\n5\n", &["-P"], ".", "5\n"),
        ("\n# lead\n5\n", &["-P"], ".", "5\n"),
        ("# lead\n---\n5\n", &["-P"], ".", "5\n"),
        ("# lead\n5\n# foot\n", &["-P"], ".", "5\n"),
        ("# lead\n5 # line\n", &["-P"], ".", "5\n"),
        ("# lead\n[1]\n", &[], ". = 6", "6\n"),
        ("# lead\n{a: 1}\n", &[], ". = 6", "6\n"),
        ("# lead\n- 1\n", &[], ". |= 6", "6\n"),
        ("# lead\na: 1\n", &["-P"], ". = 6", "6\n"),
    ])
}

/// What agrees: a plain read prints no comment for a scalar, a collection result keeps it, and the
/// metadata of a read reads it.
#[test]
fn a_collection_result_keeps_the_head_comment_and_a_read_prints_none_7089() -> Result<()> {
    assert_rows_after_bom(&[
        ("# lead\n5\n", &[], ".", "5\n"),
        ("# lead\n\"s\"\n", &[], ".", "s\n"),
        (
            "# lead\n5\n",
            &["-o=json", "-I0"],
            "[head_comment, foot_comment]",
            "[\"lead\",\"\"]\n",
        ),
        ("# lead\n{a: 1}\n", &[], ".a = 2", "# lead\n{a: 2}\n"),
        ("# lead\n[1]\n", &[], ". = [7]", "# lead\n[7]\n"),
        ("# lead\n[1]\n", &[], ". = {\"b\": 2}", "# lead\nb: 2\n"),
        ("# lead\na: 1\n", &["-P"], ".", "# lead\na: 1\n"),
        ("# lead\n- 1\n", &[], ".[0] = 9", "# lead\n- 9\n"),
        ("# lead\nnull\n", &[], ".a = 1", "# lead\na: 1\n"),
    ])
}
