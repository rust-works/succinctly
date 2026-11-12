//! #7025: `-c` / `--yaml-compact-seq-indent` writes a block sequence under a mapping key two columns left of
//! where the default puts it: at the default column less two (`-I 2` leaves the items at the key's column).
//! The default column is the next multiple of the indent width past the key's column, so a key that sits
//! at a column that is not a multiple of the width (a mapping inside a sequence item, `-I 3`) is not
//! simply moved by the width. Every row's expected output was written by yq v4.53.3: ten nestings of
//! mappings and sequences at `-I 2`, 3, 4 and 6, and comments, block scalars, anchors, empty collections,
//! `-P`, a second document, a write and the forms of the flag.
//!
//! JSON output has no sequence indent, so `-c` leaves it as it was.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;

/// `(input, flags and filter, expected output)`; the filter is `.` unless the last flag is not a flag.
type Row = (&'static str, &'static [&'static str], &'static str);

const ROWS: &[Row] = &[
    ("k:\n  - 1\n  - 2\n", &["-c", "-I", "2"], "k:\n- 1\n- 2\n"),
    ("k:\n  - 1\n  - 2\n", &["-c", "-I", "3"], "k:\n - 1\n - 2\n"),
    (
        "k:\n  - 1\n  - 2\n",
        &["-c", "-I", "4"],
        "k:\n  - 1\n  - 2\n",
    ),
    (
        "k:\n  - 1\n  - 2\n",
        &["-c", "-I", "6"],
        "k:\n    - 1\n    - 2\n",
    ),
    (
        "a:\n  k:\n    - 1\n    - 2\n",
        &["-c", "-I", "2"],
        "a:\n  k:\n  - 1\n  - 2\n",
    ),
    (
        "a:\n  k:\n    - 1\n    - 2\n",
        &["-c", "-I", "3"],
        "a:\n   k:\n    - 1\n    - 2\n",
    ),
    (
        "a:\n  k:\n    - 1\n    - 2\n",
        &["-c", "-I", "4"],
        "a:\n    k:\n      - 1\n      - 2\n",
    ),
    (
        "a:\n  k:\n    - 1\n    - 2\n",
        &["-c", "-I", "6"],
        "a:\n      k:\n          - 1\n          - 2\n",
    ),
    (
        "- k:\n    - 1\n    - 2\n  z: 1\n",
        &["-c", "-I", "2"],
        "- k:\n  - 1\n  - 2\n  z: 1\n",
    ),
    (
        "- k:\n    - 1\n    - 2\n  z: 1\n",
        &["-c", "-I", "3"],
        "- k:\n - 1\n - 2\n  z: 1\n",
    ),
    (
        "- k:\n    - 1\n    - 2\n  z: 1\n",
        &["-c", "-I", "4"],
        "- k:\n  - 1\n  - 2\n  z: 1\n",
    ),
    (
        "- k:\n    - 1\n    - 2\n  z: 1\n",
        &["-c", "-I", "6"],
        "- k:\n    - 1\n    - 2\n  z: 1\n",
    ),
    (
        "a:\n  - k:\n      - 1\n      - 2\n    z: 1\n",
        &["-c", "-I", "2"],
        "a:\n- k:\n  - 1\n  - 2\n  z: 1\n",
    ),
    (
        "a:\n  - k:\n      - 1\n      - 2\n    z: 1\n",
        &["-c", "-I", "3"],
        "a:\n - k:\n    - 1\n    - 2\n   z: 1\n",
    ),
    (
        "a:\n  - k:\n      - 1\n      - 2\n    z: 1\n",
        &["-c", "-I", "4"],
        "a:\n  - k:\n      - 1\n      - 2\n    z: 1\n",
    ),
    (
        "a:\n  - k:\n      - 1\n      - 2\n    z: 1\n",
        &["-c", "-I", "6"],
        "a:\n    - k:\n          - 1\n          - 2\n      z: 1\n",
    ),
    (
        "- a:\n    k:\n      - 1\n      - 2\n",
        &["-c", "-I", "2"],
        "- a:\n    k:\n    - 1\n    - 2\n",
    ),
    (
        "- a:\n    k:\n      - 1\n      - 2\n",
        &["-c", "-I", "3"],
        "- a:\n   k:\n    - 1\n    - 2\n",
    ),
    (
        "- a:\n    k:\n      - 1\n      - 2\n",
        &["-c", "-I", "4"],
        "- a:\n    k:\n      - 1\n      - 2\n",
    ),
    (
        "- a:\n    k:\n      - 1\n      - 2\n",
        &["-c", "-I", "6"],
        "- a:\n      k:\n          - 1\n          - 2\n",
    ),
    (
        "- - k:\n      - 1\n      - 2\n",
        &["-c", "-I", "2"],
        "- - k:\n    - 1\n    - 2\n",
    ),
    (
        "- - k:\n      - 1\n      - 2\n",
        &["-c", "-I", "3"],
        "- - k:\n    - 1\n    - 2\n",
    ),
    (
        "- - k:\n      - 1\n      - 2\n",
        &["-c", "-I", "4"],
        "- - k:\n      - 1\n      - 2\n",
    ),
    (
        "- - k:\n      - 1\n      - 2\n",
        &["-c", "-I", "6"],
        "- - k:\n    - 1\n    - 2\n",
    ),
    (
        "a:\n  - - k:\n        - 1\n        - 2\n",
        &["-c", "-I", "2"],
        "a:\n- - k:\n    - 1\n    - 2\n",
    ),
    (
        "a:\n  - - k:\n        - 1\n        - 2\n",
        &["-c", "-I", "3"],
        "a:\n - - k:\n    - 1\n    - 2\n",
    ),
    (
        "a:\n  - - k:\n        - 1\n        - 2\n",
        &["-c", "-I", "4"],
        "a:\n  - - k:\n      - 1\n      - 2\n",
    ),
    (
        "a:\n  - - k:\n        - 1\n        - 2\n",
        &["-c", "-I", "6"],
        "a:\n    - - k:\n          - 1\n          - 2\n",
    ),
    (
        "a:\n  - b:\n      k:\n        - 1\n        - 2\n",
        &["-c", "-I", "2"],
        "a:\n- b:\n    k:\n    - 1\n    - 2\n",
    ),
    (
        "a:\n  - b:\n      k:\n        - 1\n        - 2\n",
        &["-c", "-I", "3"],
        "a:\n - b:\n      k:\n       - 1\n       - 2\n",
    ),
    (
        "a:\n  - b:\n      k:\n        - 1\n        - 2\n",
        &["-c", "-I", "4"],
        "a:\n  - b:\n        k:\n          - 1\n          - 2\n",
    ),
    (
        "a:\n  - b:\n      k:\n        - 1\n        - 2\n",
        &["-c", "-I", "6"],
        "a:\n    - b:\n            k:\n                - 1\n                - 2\n",
    ),
    (
        "a:\n  b:\n    - k:\n        - 1\n        - 2\n",
        &["-c", "-I", "2"],
        "a:\n  b:\n  - k:\n    - 1\n    - 2\n",
    ),
    (
        "a:\n  b:\n    - k:\n        - 1\n        - 2\n",
        &["-c", "-I", "3"],
        "a:\n   b:\n    - k:\n       - 1\n       - 2\n",
    ),
    (
        "a:\n  b:\n    - k:\n        - 1\n        - 2\n",
        &["-c", "-I", "4"],
        "a:\n    b:\n      - k:\n          - 1\n          - 2\n",
    ),
    (
        "a:\n  b:\n    - k:\n        - 1\n        - 2\n",
        &["-c", "-I", "6"],
        "a:\n      b:\n          - k:\n                - 1\n                - 2\n",
    ),
    (
        "- a:\n    - k:\n        - 1\n        - 2\n",
        &["-c", "-I", "2"],
        "- a:\n  - k:\n    - 1\n    - 2\n",
    ),
    (
        "- a:\n    - k:\n        - 1\n        - 2\n",
        &["-c", "-I", "3"],
        "- a:\n - k:\n    - 1\n    - 2\n",
    ),
    (
        "- a:\n    - k:\n        - 1\n        - 2\n",
        &["-c", "-I", "4"],
        "- a:\n  - k:\n      - 1\n      - 2\n",
    ),
    (
        "- a:\n    - k:\n        - 1\n        - 2\n",
        &["-c", "-I", "6"],
        "- a:\n    - k:\n          - 1\n          - 2\n",
    ),
    (
        "a:\n  - d\n  - |\n    text\n    more\n  - \"q\"\n  - [h, i]\nb: {c: [1, 2]}\n# tail\n",
        &["-c"],
        "a:\n- d\n- |\n  text\n  more\n- \"q\"\n- [h, i]\nb: {c: [1, 2]}\n# tail\n",
    ),
    (
        "a:\n  - d\n  - |\n    text\n    more\n  - \"q\"\n  - [h, i]\nb: {c: [1, 2]}\n# tail\n",
        &["-c", "-I", "4"],
        "a:\n  - d\n  - |\n    text\n    more\n  - \"q\"\n  - [h, i]\nb: {c: [1, 2]}\n# tail\n",
    ),
    (
        "a:\n  - d\n  - |\n    text\n    more\n  - \"q\"\n  - [h, i]\nb: {c: [1, 2]}\n# tail\n",
        &["-c", "-P"],
        "a:\n- d\n- |\n  text\n  more\n- q\n- - h\n  - i\nb:\n  c:\n  - 1\n  - 2\n# tail\n",
    ),
    (
        "a:\n  - d\n  - |\n    text\n    more\n  - \"q\"\n  - [h, i]\nb: {c: [1, 2]}\n# tail\n",
        &["--yaml-compact-seq-indent"],
        "a:\n- d\n- |\n  text\n  more\n- \"q\"\n- [h, i]\nb: {c: [1, 2]}\n# tail\n",
    ),
    (
        "a:\n  - d\n  - |\n    text\n    more\n  - \"q\"\n  - [h, i]\nb: {c: [1, 2]}\n# tail\n",
        &["-c=false"],
        "a:\n  - d\n  - |\n    text\n    more\n  - \"q\"\n  - [h, i]\nb: {c: [1, 2]}\n# tail\n",
    ),
    (
        "# head\na:\n  # lead\n  - x # one\n  # between\n  - y\n  # foot\nb: 1\n",
        &["-c"],
        "# head\na:\n# lead\n- x # one\n# between\n- y\n# foot\n\nb: 1\n",
    ),
    (
        "# head\na:\n  # lead\n  - x # one\n  # between\n  - y\n  # foot\nb: 1\n",
        &["-c", "-I", "4"],
        "# head\na:\n  # lead\n  - x # one\n  # between\n  - y\n  # foot\nb: 1\n",
    ),
    (
        "a: &s\n  - 1\n  - 2\nb: *s\nc:\n  - &e 3\n  - *e\n",
        &["-c"],
        "a: &s\n- 1\n- 2\nb: *s\nc:\n- &e 3\n- *e\n",
    ),
    (
        "a:\n  - e: 1\n    f:\n      - g\n      - h\n  - - x\n    - y\nz: []\ny: {}\n",
        &["-c"],
        "a:\n- e: 1\n  f:\n  - g\n  - h\n- - x\n  - y\nz: []\ny: {}\n",
    ),
    (
        "a:\n  - e: 1\n    f:\n      - g\n      - h\n  - - x\n    - y\nz: []\ny: {}\n",
        &["-c", "-I", "4"],
        "a:\n  - e: 1\n    f:\n      - g\n      - h\n  - - x\n    - y\nz: []\ny: {}\n",
    ),
    (
        "a:\n  - e: 1\n    f:\n      - g\n      - h\n  - - x\n    - y\nz: []\ny: {}\n",
        &["-c", "-I", "1"],
        "a:\n- e: 1\n  f:\n  - g\n  - h\n- - x\n  - y\nz: []\ny: {}\n",
    ),
    (
        "a:\n  - e: 1\n    f:\n      - g\n      - h\n  - - x\n    - y\nz: []\ny: {}\n",
        &["-c", "-I", "0"],
        "a:\n  - e: 1\n    f:\n      - g\n      - h\n  - - x\n    - y\nz: []\ny: {}\n",
    ),
    (
        "a:\n  - 1\n  - 2\n---\nb:\n  - 3\n",
        &["-c"],
        "a:\n- 1\n- 2\n---\nb:\n- 3\n",
    ),
    (
        "a:\n  - 1\n  - 2\nb: x\n",
        &["-c", ".c = [1, {\"d\": [2]}]"],
        "a:\n- 1\n- 2\nb: x\nc:\n- 1\n- d:\n  - 2\n",
    ),
    (
        "a:\n  - x\n  # foot\nb: 1\n",
        &["-c"],
        "a:\n- x\n# foot\n\nb: 1\n",
    ),
    (
        "a:\n  - x\n  # foot\nb: 1\n",
        &["-c", "-I", "4"],
        "a:\n  - x\n  # foot\nb: 1\n",
    ),
    (
        "a:\n  - x\n  - y\n  # foot\nb:\n  - z\n  # foot2\nc: 2\n",
        &["-c"],
        "a:\n- x\n- y\n# foot\n\nb:\n- z\n# foot2\n\nc: 2\n",
    ),
    (
        "a:\n  b:\n    - x\n    # foot\n  c: 1\nd: 1\n",
        &["-c"],
        "a:\n  b:\n  - x\n  # foot\n\n  c: 1\nd: 1\n",
    ),
    (
        "a:\n  - k: 1\n    j: 2\n  # foot\nb: 1\n",
        &["-c"],
        "a:\n- k: 1\n  j: 2\n  # foot\nb: 1\n",
    ),
    (
        "- a:\n    - x\n    # foot\n  b: 1\n",
        &["-c"],
        "- a:\n  - x\n  # foot\n\n  b: 1\n",
    ),
    (
        "a:\n  - x\n  # foot\n# next head\nb: 1\n",
        &["-c"],
        "a:\n- x\n# foot\n\n# next head\nb: 1\n",
    ),
];

fn run(input: &str, flags: &[&str]) -> Result<(String, i32)> {
    let mut args: Vec<&str> = flags.to_vec();
    // A filter is the last argument that does not start with `-` (and is not `-I`'s width).
    let has_filter = flags
        .iter()
        .enumerate()
        .any(|(i, f)| !f.starts_with('-') && (i == 0 || flags[i - 1] != "-I"));
    if !has_filter {
        args.push(".");
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().unwrap().write_all(input.as_bytes())?;
    let output = child.wait_with_output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code().unwrap_or(-1),
    ))
}

#[test]
fn a_block_sequence_under_a_key_is_two_columns_left_of_the_default_7025() -> Result<()> {
    for &(input, flags, want) in ROWS {
        let (got, code) = run(input, flags)?;
        assert_eq!(code, 0, "{flags:?} {input:?}");
        assert_eq!(got, want, "{flags:?} {input:?}");
    }
    Ok(())
}

#[test]
fn json_output_and_the_default_are_not_changed_by_the_flag_7025() -> Result<()> {
    let input = "a:\n  - d\n  - e: 1\n    f: [1, 2]\n";
    let (plain_json, _) = run(input, &["-o", "json"])?;
    let (compact_json, _) = run(input, &["-c", "-o", "json"])?;
    assert_eq!(plain_json, compact_json);
    let (plain, _) = run(input, &[])?;
    let (off, _) = run(input, &["-c=false"])?;
    assert_eq!(plain, off);
    assert_ne!(plain, run(input, &["-c"])?.0);
    Ok(())
}

/// `--inplace` writes the compact layout to the file.
#[test]
fn inplace_writes_the_compact_layout_7025() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("f.yaml");
    std::fs::write(&path, "a:\n  - 1\n  - 2\nb: x\n")?;
    let status = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["yq", "-c", "-i", ".b = \"y\""])
        .arg(&path)
        .status()?;
    assert!(status.success());
    assert_eq!(std::fs::read_to_string(&path)?, "a:\n- 1\n- 2\nb: y\n");
    Ok(())
}
