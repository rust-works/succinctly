//! #6901: yq encodes a node with the presentation it was written with, so `to_yaml`, `@yaml` and
//! `tostring` over a container that is a document node print its flow style, the nested flow
//! collections of a block node, its trailing comment, and (for a sequence item) its own head and
//! foot comment lines; an alias node is the reference `*name`, a container or a scalar alike. They
//! printed the canonical block text of the node's value, which has no style or comment.
//!
//! Every row was captured from yq v4.53.3 (`-I2`) and kept only when both tools agree. A value the
//! filter builds (`[.o] | to_yaml`, a node behind a write) has no node to read the style from and
//! is left out: it still prints block.

use anyhow::Result;

use crate::run_yq_stdin;

/// (input, filter, yq's stdout).
const ROWS: &[(&str, &str, &str)] = &[
    (
        "o: {p: 1, q: {r: 2}}\nm: [a, b] # c\nl: [1, 2]\ne: {}\nz: []\n",
        ".o|to_yaml",
        "{p: 1, q: {r: 2}}\n\n",
    ),
    (
        "o: {p: 1, q: {r: 2}}\nm: [a, b] # c\nl: [1, 2]\ne: {}\nz: []\n",
        ".m|to_yaml",
        "[a, b] # c\n\n",
    ),
    (
        "o: {p: 1, q: {r: 2}}\nm: [a, b] # c\nl: [1, 2]\ne: {}\nz: []\n",
        ".l|to_yaml",
        "[1, 2]\n\n",
    ),
    (
        "o: {p: 1, q: {r: 2}}\nm: [a, b] # c\nl: [1, 2]\ne: {}\nz: []\n",
        ".e|to_yaml",
        "{}\n\n",
    ),
    (
        "o: {p: 1, q: {r: 2}}\nm: [a, b] # c\nl: [1, 2]\ne: {}\nz: []\n",
        ".z|to_yaml",
        "[]\n\n",
    ),
    (
        "o: {p: 1, q: {r: 2}}\nm: [a, b] # c\nl: [1, 2]\ne: {}\nz: []\n",
        ".o|@yaml",
        "{p: 1, q: {r: 2}}\n\n",
    ),
    (
        "o: {p: 1, q: {r: 2}}\nm: [a, b] # c\nl: [1, 2]\ne: {}\nz: []\n",
        ".m|@yaml",
        "[a, b] # c\n\n",
    ),
    (
        "o: {p: 1, q: {r: 2}}\nm: [a, b] # c\nl: [1, 2]\ne: {}\nz: []\n",
        ".o|tostring",
        "{p: 1, q: {r: 2}}\n",
    ),
    (
        "o: {p: 1, q: {r: 2}}\nm: [a, b] # c\nl: [1, 2]\ne: {}\nz: []\n",
        ".m|tostring",
        "[a, b] # c\n",
    ),
    (
        "o: {p: 1, q: {r: 2}}\nm: [a, b] # c\nl: [1, 2]\ne: {}\nz: []\n",
        ".l|tostring",
        "[1, 2]\n",
    ),
    (
        "o: {p: 1, q: {r: 2}}\nm: [a, b] # c\nl: [1, 2]\ne: {}\nz: []\n",
        ".o|to_yaml(0)",
        "{p: 1, q: {r: 2}}\n\n",
    ),
    (
        "o: {p: 1, q: {r: 2}}\nm: [a, b] # c\nl: [1, 2]\ne: {}\nz: []\n",
        ".o|to_yaml(4)",
        "{p: 1, q: {r: 2}}\n\n",
    ),
    (
        "o: {p: 1, q: {r: 2}}\nm: [a, b] # c\nl: [1, 2]\ne: {}\nz: []\n",
        ".l|to_yaml(1)",
        "[1, 2]\n\n",
    ),
    (
        "o: {p: 1, q: {r: 2}}\nm: [a, b] # c\nl: [1, 2]\ne: {}\nz: []\n",
        ".|to_yaml",
        "o: {p: 1, q: {r: 2}}\nm: [a, b] # c\nl: [1, 2]\ne: {}\nz: []\n\n",
    ),
    (
        "b:\n  x: 1\n  y: [1, 2] # in\n  z:\n    - k\n",
        ".b|to_yaml",
        "x: 1\ny: [1, 2] # in\nz:\n  - k\n\n",
    ),
    (
        "b:\n  x: 1\n  y: [1, 2] # in\n  z:\n    - k\n",
        ".b|tostring",
        "x: 1\ny: [1, 2] # in\nz:\n  - k\n",
    ),
    (
        "b:\n  x: 1\n  y: [1, 2] # in\n  z:\n    - k\n",
        ".b|@yaml",
        "x: 1\ny: [1, 2] # in\nz:\n  - k\n\n",
    ),
    (
        "b:\n  x: 1\n  y: [1, 2] # in\n  z:\n    - k\n",
        ".b.y|to_yaml",
        "[1, 2] # in\n\n",
    ),
    (
        "b:\n  x: 1\n  y: [1, 2] # in\n  z:\n    - k\n",
        ".b.z|tostring",
        "- k\n",
    ),
    (
        "n: # d\n  a: 1\n  b: [1]  # e\n",
        ".n|to_yaml",
        "a: 1\nb: [1] # e\n\n",
    ),
    (
        "n: # d\n  a: 1\n  b: [1]  # e\n",
        ".n|tostring",
        "a: 1\nb: [1] # e\n",
    ),
    (
        "n: # d\n  a: 1\n  b: [1]  # e\n",
        ".|to_yaml",
        "n: # d\n  a: 1\n  b: [1] # e\n\n",
    ),
    (
        "h:\n  # hd\n  - [1, 2] # i\n  - x # ix\n  # ft\n  - {k: v}\n",
        ".h|to_yaml",
        "# hd\n- [1, 2] # i\n- x # ix\n# ft\n- {k: v}\n\n",
    ),
    (
        "h:\n  # hd\n  - [1, 2] # i\n  - x # ix\n  # ft\n  - {k: v}\n",
        ".h|tostring",
        "# hd\n- [1, 2] # i\n- x # ix\n# ft\n- {k: v}\n",
    ),
    (
        "h:\n  # hd\n  - [1, 2] # i\n  - x # ix\n  # ft\n  - {k: v}\n",
        ".h[0]|to_yaml",
        "# hd\n[1, 2] # i\n\n",
    ),
    (
        "h:\n  # hd\n  - [1, 2] # i\n  - x # ix\n  # ft\n  - {k: v}\n",
        ".h[0]|tostring",
        "# hd\n[1, 2] # i\n",
    ),
    (
        "h:\n  # hd\n  - [1, 2] # i\n  - x # ix\n  # ft\n  - {k: v}\n",
        ".h[1]|to_yaml",
        "x\n\n",
    ),
    (
        "h:\n  # hd\n  - [1, 2] # i\n  - x # ix\n  # ft\n  - {k: v}\n",
        ".h[2]|to_yaml",
        "# ft\n{k: v}\n\n",
    ),
    (
        "h:\n  # hd\n  - [1, 2] # i\n  - x # ix\n  # ft\n  - {k: v}\n",
        ".h[2]|tostring",
        "# ft\n{k: v}\n",
    ),
    (
        "base: &x {k: 1, l: [2, 3]}\nuse: *x\nn: &y 1\nm: *y\ns: &z [4]\nt: *z\n",
        ".use|to_yaml",
        "*x\n\n",
    ),
    (
        "base: &x {k: 1, l: [2, 3]}\nuse: *x\nn: &y 1\nm: *y\ns: &z [4]\nt: *z\n",
        ".use|tostring",
        "*x\n",
    ),
    (
        "base: &x {k: 1, l: [2, 3]}\nuse: *x\nn: &y 1\nm: *y\ns: &z [4]\nt: *z\n",
        ".use|@yaml",
        "*x\n\n",
    ),
    (
        "base: &x {k: 1, l: [2, 3]}\nuse: *x\nn: &y 1\nm: *y\ns: &z [4]\nt: *z\n",
        ".m|to_yaml",
        "*y\n\n",
    ),
    (
        "base: &x {k: 1, l: [2, 3]}\nuse: *x\nn: &y 1\nm: *y\ns: &z [4]\nt: *z\n",
        ".m|tostring",
        "*y\n",
    ),
    (
        "base: &x {k: 1, l: [2, 3]}\nuse: *x\nn: &y 1\nm: *y\ns: &z [4]\nt: *z\n",
        ".t|to_yaml",
        "*z\n\n",
    ),
    (
        "base: &x {k: 1, l: [2, 3]}\nuse: *x\nn: &y 1\nm: *y\ns: &z [4]\nt: *z\n",
        ".base|to_yaml",
        "&x {k: 1, l: [2, 3]}\n\n",
    ),
    (
        "base: &x {k: 1, l: [2, 3]}\nuse: *x\nn: &y 1\nm: *y\ns: &z [4]\nt: *z\n",
        ".n|to_yaml",
        "1\n\n",
    ),
];

#[test]
fn a_document_node_is_encoded_as_it_was_written_6901() -> Result<()> {
    for (input, filter, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(code, 0, "{filter} over {input:?}");
        assert_eq!(&stdout, expected, "{filter} over {input:?}");
    }
    Ok(())
}

/// The encoded text is a string, so `-o=json` shows it whole, and a JSON-sourced container (which
/// has no flow style to keep) is block as before.
#[test]
fn the_encoded_node_is_the_string_the_json_output_shows_6901() -> Result<()> {
    let (stdout, code) = run_yq_stdin(".m | to_yaml", "m: [a, b] # c\n", &["-o=json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "\"[a, b] # c\\n\"\n");

    let (stdout, code) = run_yq_stdin(
        ".a | to_yaml",
        "{\"a\": {\"b\": [1, 2], \"e\": {}}}",
        &["-p=json", "-o=yaml"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "b:\n  - 1\n  - 2\ne: {}\n\n");
    Ok(())
}
