//! #6843: a comment line at or left of the parent's indent, straight after a block scalar's header,
//! ends the scalar before it has any content, as in go-yaml: the scalar is empty and the lines after
//! the comment belong to the document, so a deeper one is stray (`a: |` / `#x` / `  y` is refused).
//! The index skipped the comment to look for the content line, found `  y`, and let the block-end
//! kernel (which never looks at a scalar's first line) take the comment as the scalar's first line:
//! the scalar ran over `  y` and the decoder, which reads the comment as the first content line,
//! answered empty with no error. Only the document's root node reads past such a comment
//! (`--- |` / `#x` / `  y` is `"y\n"`). Every answer was captured from yq v4.53.3 (`-o=json -I=0`);
//! `None` is a document yq refuses.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, yq's answer or `None` for a refusal).
const ROWS: &[(&str, Option<&str>)] = &[
    ("a: |\n#x\n  y\n", None),
    ("a: >\n#x\n  y\n", None),
    ("a: |-\n#x\n  y\n", None),
    ("a: |+\n#x\n  y\n", None),
    ("- |\n#x\n  y\n", None),
    ("a:\n  b: |\n  #x\n    y\n", None),
    ("a:\n  - |\n  #x\n    y\n", None),
    ("- a: |\n  #x\n    y\n", None),
    ("a: |\n#x\n\n  y\n", None),
    ("a: |\n\n#x\n  y\n", None),
    ("a: |\n#x\n#z\n  y\n", None),
    ("a: |\n#x\n    y\n", None),
    ("a: |\t\n#x\n  y\n", None),
    ("a: | # c\n#x\n  y\n", None),
    ("a: &x |\n#x\n  y\n", None),
    ("a: !!str |\n#x\n  y\n", None),
    ("? |\n#x\n  y\n: v\n", None),
    ("a: |\n#x\nb: 1\n", Some("{\"a\":\"\",\"b\":1}")),
    ("a: |\n# x\n", Some("{\"a\":\"\"}")),
    ("a: |\n  #x\n", Some("{\"a\":\"#x\\n\"}")),
    ("a: |\n # x\n  y\n", Some("{\"a\":\"# x\\n y\\n\"}")),
    ("a: |\n  y\n#x\n", Some("{\"a\":\"y\\n\"}")),
    ("a: |\n  y\n#x\nb: 1\n", Some("{\"a\":\"y\\n\",\"b\":1}")),
    (
        "a:\n  b: |\n    y\n  #x\nc: 1\n",
        Some("{\"a\":{\"b\":\"y\\n\"},\"c\":1}"),
    ),
    ("--- |\n#x\n  y\n", Some("\"y\\n\"")),
    ("a: |\n#x\n#z\nb: 1\n", Some("{\"a\":\"\",\"b\":1}")),
    ("- |\n#x\n- q\n", Some("[\"\",\"q\"]")),
    ("a: |\n  #x\n  y\n", Some("{\"a\":\"#x\\ny\\n\"}")),
    ("a: >-\n    #x\n    y\n", Some("{\"a\":\"#x y\"}")),
];

#[test]
fn a_comment_left_of_the_parent_ends_an_empty_block_scalar_as_in_go_yaml_6843() -> Result<()> {
    for &(input, expected) in ROWS {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        match expected {
            Some(want) => {
                assert_eq!(code, 0, "{input:?}: {out}");
                assert_eq!(out.trim_end(), want, "{input:?}");
            }
            None => {
                assert_ne!(code, 0, "{input:?}: {out}");
                assert_eq!(out.trim_end(), "", "{input:?}");
            }
        }
    }
    Ok(())
}
