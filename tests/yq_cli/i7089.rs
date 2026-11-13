//! #7089: behind a byte-order mark, the block under the first `---` that a blank line detaches from
//! the body is the foot of the first key, item or scalar (the document's foot for a scalar), and is
//! dropped before a flow collection, where the block above the marker stays the head. It was placed
//! in the head with the block above (`# c1` / `---` / `# c2` / blank / `a: 1` was `c1\nc2\n` on `.a`'s key's
//! head). A marker at the very start, a block with no blank line under it, and a file with no mark
//! are pinned unchanged.
//!
//! Every row is yq v4.53.3's output for `F` (the head and foot comment of the document and of every
//! key and item, `-o=json -I0`), `.` and `-o=props .`.

use anyhow::Result;

use crate::run_yq_stdin;

const F: &str = r#"[head_comment, foot_comment] + [.. | select(tag=="!!map") | keys[] | [head_comment, foot_comment]] + [.. | select(tag=="!!seq") | .[] | [head_comment, foot_comment]]"#;

#[test]
fn the_block_under_a_first_marker_is_the_first_nodes_foot_after_a_byte_order_mark() -> Result<()> {
    let rows: [(bool, &str, [&str; 3]); 30] = [
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
