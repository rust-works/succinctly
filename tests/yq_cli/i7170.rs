//! #7170: a TOML file read with `-p toml` and written back as TOML (`-o toml`, `-i` on a `*.toml` file)
//! keeps the comments yq's decoder gives its value nodes and its encoder writes (#7160): the root mapping's
//! head, a table's head above its `[header]`, an attribute's head above it and its line comment after it, an
//! array's the same and an array whose elements have heads in the multi-line form. The read writes the YAML
//! text a TOML-writing run needs: every head comment above the key it belongs to (a TOML file has no comment
//! of a key), a non-empty array's line comment after its key, and the root's head ahead of an explicit `---`
//! that keeps it apart from the first entry's. Every row was captured from yq v4.53.3 (stdout, the first
//! stderr line, the exit status) and kept only when both tools agree: 4,600 random documents over six filters
//! and 2,005 of the 2,026 real `*.toml` files on a developer machine under the identity filter (the other 21
//! hold inline tables, which this run writes as tables, see `limitations.md`).

use anyhow::Result;
use tempfile::TempDir;

use crate::{run_yq_files, run_yq_stdin_with_stderr};

/// (filter, stdin, yq's stdout, yq's first stderr line, yq's exit status).
type Row = (&'static str, &'static str, &'static str, &'static str, i32);

#[rustfmt::skip]
const ROWS: &[Row] = &[
    (".", "# top\na = 1 # l\n\n# tbl\n[t]\nb = [1, 2] # arr\nc = \"x\"\n", "# top\na = 1  # l\n\n# tbl\n[t]\nb = [1, 2]  # arr\nc = \"x\"\n", "", 0),
    (".", "# root head\n# second line\n\ntitle = \"x\" # tl\n\n# a table\n# more\n[owner]\n# name head\nname = \"Tom\" # nm\nlist = [\n  # first\n  1, # one\n  2,\n] # arr line\n", "# root head\n# second line\ntitle = \"x\"  # tl\n\n# a table\n# more\n[owner]\n# name head\nname = \"Tom\"  # nm\nlist = [\n  # first\n  1,\n\n  # one\n  2,\n]\n", "", 0),
    (".", "[[servers]]\n# srv head\nip = \"1.1.1.1\"\n# dc\n[servers.meta] # meta line\nx = 1\n\n# second server\n[[servers]]\nip = \"2.2.2.2\"\narr = [1,2] # al\n", "[[servers]]\n# srv head\nip = \"1.1.1.1\"\n\n# dc\n[servers.meta]\nx = 1\n[[servers]]\nip = \"2.2.2.2\"\narr = [1, 2]  # al\n", "", 0),
    (".", "#a\n[k]  #c\n#note\n\n[[n]]\n", "#a\n#note\n[k]\n[[n]]\n", "", 0),
    (".zz = \"v\"", "#a\n[k]  #c\n#note\n\n[[n]]\n", "#a\nzz = \"v\"\n\n#note\n[k]\n[[n]]\n", "", 0),
    ("del(.a)", "# top\na = 1 # l\nb = 2\n# cb\nc = 3 # lc\n", "# top\nb = 2\n# cb\nc = 3  # lc\n", "", 0),
    ("del(.c)", "# top\na = 1 # l\nb = 2\n# cb\nc = 3 # lc\n", "# top\na = 1  # l\nb = 2\n", "", 0),
    ("sort_keys(.)", "# top\nb = 1 # lb\na = 2 # la\n# ha\nc = [1] # lc\n", "# top\na = 2  # la\nb = 1  # lb\n# ha\nc = [1]  # lc\n", "", 0),
    (".t.b = 5", "# top\n\n# h\na = 1 # l\n[t]\nb = 2 # lb\n", "# top\n# h\na = 1  # l\n\n[t]\nb = 5  # lb\n", "", 0),
    (".t.new = 5", "# top\n\n# h\na = 1 # l\n[t]\nb = 2 # lb\n", "# top\n# h\na = 1  # l\n\n[t]\nb = 2  # lb\nnew = 5\n", "", 0),
    (".package.version = \"1.2.3\"", "[package]\nname = \"x\"\n# the version\nversion = \"0.1.0\" # bump me\n\n# deps\n[dependencies]\n# a dep\nserde = \"1\"\n", "[package]\nname = \"x\"\n# the version\nversion = \"1.2.3\"  # bump me\n\n# deps\n[dependencies]\n# a dep\nserde = \"1\"\n", "", 0),
    (".", "a = []  # empty\nb = 1 #nospace\nc = 3 #\n", "a = []  # empty\nb = 1  #nospace\nc = 3  #\n", "", 0),
    (".", "arr = [\n  # h1\n  1,\n  2, # l2\n  # h3\n  3,\n]\n", "arr = [\n  # h1\n  1,\n\n  2,\n\n  # l2\n  3,\n]\n", "", 0),
    (".", "[a]\n# between\n[a.b]\nx = 1\n", "[a.b]\nx = 1\n", "", 0),
    (".c = 1", "[a]\n# between\n[a.b]\nx = 1\n", "c = 1\n\n[a.b]\nx = 1\n", "", 0),
    (".", "# h\nk-2 = 1\n\"quoted key\" = 2 # q\n", "# h\nk-2 = 1\n\"quoted key\" = 2  # q\n", "", 0),
    (".", "# one\n\n# two\nx = 1\n", "# one\n# two\nx = 1\n", "", 0),
    (".", "a = [\n  # c1\n  [1, 2],\n  # c2\n  [3],\n]\n", "a = [\n  # c1\n  [1, 2],\n\n  # c2\n  [3],\n]\n", "", 0),
    (".", "# h\n\"a.b\" = 1 # l\n# h2\n\"x y\" = [1] # al\n\"? q\" = 2\n\"k: v\" = 3 # kv\n", "# h\n\"a.b\" = 1  # l\n# h2\n\"x y\" = [1]  # al\n\"? q\" = 2\n\"k: v\" = 3  # kv\n", "", 0),
    (".", "# h\n[a.b.c]\n# h2\nx = 1\n[a.d]\n# h3\ny = 2\n", "# h\n[a.b.c]\n# h2\nx = 1\n\n[a.d]\n# h3\ny = 2\n", "", 0),
    (".", "[[a]]\n# e1\nx = 1\n[[a.b]]\n# e2\ny = 2\n", "[[a]]\n# e1\nx = 1\n[[a.b]]\n# e2\ny = 2\n", "", 0),
    (".", "a.b.c = 1 # l\n# h\na.b.d = 2\n", "[a.b]\nc = 1  # l\n# h\nd = 2\n", "", 0),
    (".", "k = [ # open\n  1,\n]\n", "k = [\n  # open\n  1,\n]\n", "", 0),
    (".", "# h\nk = \"multi\\nline\" # l\ne = \"\"\"\nblock\n\"\"\" # bl\n", "# h\nk = \"multi\\nline\"  # l\ne = \"block\\n\"  # bl\n", "", 0),
    (".", "[a]\n# only\n", "# only\n[a]\n", "", 0),
    (".b = 5", "a = 1 # l\nb = [1] # al\n", "a = 1  # l\nb = 5  # al\n", "", 0),
    ("sort_keys(.)", "#a\n# h\n[t]\n# th\nx = 1\n", "#a\n# h\n[t]\n# th\nx = 1\n", "", 0),
];

#[test]
fn toml_written_back_as_toml_keeps_its_comments_7170() -> Result<()> {
    for &(filter, input, stdout, stderr, code) in ROWS {
        let (out, err, status) =
            run_yq_stdin_with_stderr(filter, input, &["-p", "toml", "-o", "toml"])?;
        let row = format!("-p toml -o toml {filter:?} on {input:?}");
        assert_eq!(out, stdout, "stdout of {row}");
        assert_eq!(err.lines().next().unwrap_or(""), stderr, "stderr of {row}");
        assert_eq!(status, code, "exit status of {row}");
    }
    Ok(())
}

/// `-i` on a `*.toml` file, which strips every comment before #7170.
#[test]
fn in_place_write_to_a_toml_file_keeps_its_comments_7170() -> Result<()> {
    let dir = TempDir::new()?;
    let path = dir.path().join("Cargo.toml");
    std::fs::write(
        &path,
        "# Cargo manifest\n\n[package]\nname = \"x\" # the name\n# the version\nversion = \"0.1.0\" # bump me\n\n# deps\n[dependencies]\n# a dep\nserde = \"1\"\n",
    )?;
    run_yq_files(".package.version = \"1.2.3\"", &[&path], &["-i"])?;
    assert_eq!(
        std::fs::read_to_string(&path)?,
        "# Cargo manifest\n[package]\nname = \"x\"  # the name\n# the version\nversion = \"1.2.3\"  # bump me\n\n# deps\n[dependencies]\n# a dep\nserde = \"1\"\n"
    );
    Ok(())
}
