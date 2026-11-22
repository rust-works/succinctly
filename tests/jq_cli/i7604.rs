//! #7604: a data module (`import "d" as $d;`) holding a byte that is not valid UTF-8 is read as jq
//! 1.7.1 and 1.8.2 read it, with U+FFFD, where it was refused with `error loading data file ...:
//! stream did not contain valid UTF-8` (exit 3). jq's `jv_load_file` parses the file's bytes, so the
//! bytes of a string are decoded as the main input's and a `--slurpfile` file's are (#7436).
//!
//! Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 agrees on every row.

use anyhow::Result;

/// (sequence, the file with `%s` where the sequence goes, which program, jq's output).
const ROWS: &[(&[u8], &str, usize, &str)] = &[
    (b"\xff", "\"%s\"", 0, "[[\"�\"],[[65533]]]\n"),
    (b"\xff", "\"%s\"", 1, "1\n"),
    (b"\xff", "[\"x%s\", 1]", 0, "[[[\"x�\",1]],[[120,65533]]]\n"),
    (b"\xff", "[\"x%s\", 1]", 1, "1\n"),
    (b"\xff", "{\"k%s\": 2}", 0, "[[{\"k�\":2}],[[107,65533]]]\n"),
    (b"\xff", "{\"k%s\": 2}", 1, "1\n"),
    (
        b"\xff",
        "{\"a\": {\"k%s\": [\"%s\"]}}\n[1]",
        0,
        "[[{\"a\":{\"k�\":[\"�\"]}},[1]],[[97],[107,65533],[65533]]]\n",
    ),
    (b"\xff", "{\"a\": {\"k%s\": [\"%s\"]}}\n[1]", 1, "2\n"),
    (
        b"\xff",
        "\"%s\"\n\"b%s\"",
        0,
        "[[\"�\",\"b�\"],[[65533],[98,65533]]]\n",
    ),
    (b"\xff", "\"%s\"\n\"b%s\"", 1, "2\n"),
    (b"\xc3", "\"%s\"", 0, "[[\"�\"],[[65533]]]\n"),
    (b"\xc3", "\"%s\"", 1, "1\n"),
    (b"\xc3", "[\"x%s\", 1]", 0, "[[[\"x�\",1]],[[120,65533]]]\n"),
    (b"\xc3", "[\"x%s\", 1]", 1, "1\n"),
    (b"\xc3", "{\"k%s\": 2}", 0, "[[{\"k�\":2}],[[107,65533]]]\n"),
    (b"\xc3", "{\"k%s\": 2}", 1, "1\n"),
    (
        b"\xc3",
        "{\"a\": {\"k%s\": [\"%s\"]}}\n[1]",
        0,
        "[[{\"a\":{\"k�\":[\"�\"]}},[1]],[[97],[107,65533],[65533]]]\n",
    ),
    (b"\xc3", "{\"a\": {\"k%s\": [\"%s\"]}}\n[1]", 1, "2\n"),
    (
        b"\xc3",
        "\"%s\"\n\"b%s\"",
        0,
        "[[\"�\",\"b�\"],[[65533],[98,65533]]]\n",
    ),
    (b"\xc3", "\"%s\"\n\"b%s\"", 1, "2\n"),
    (b"\xed\xa0\x80", "\"%s\"", 0, "[[\"�\"],[[65533]]]\n"),
    (b"\xed\xa0\x80", "\"%s\"", 1, "1\n"),
    (
        b"\xed\xa0\x80",
        "[\"x%s\", 1]",
        0,
        "[[[\"x�\",1]],[[120,65533]]]\n",
    ),
    (b"\xed\xa0\x80", "[\"x%s\", 1]", 1, "1\n"),
    (
        b"\xed\xa0\x80",
        "{\"k%s\": 2}",
        0,
        "[[{\"k�\":2}],[[107,65533]]]\n",
    ),
    (b"\xed\xa0\x80", "{\"k%s\": 2}", 1, "1\n"),
    (
        b"\xed\xa0\x80",
        "{\"a\": {\"k%s\": [\"%s\"]}}\n[1]",
        0,
        "[[{\"a\":{\"k�\":[\"�\"]}},[1]],[[97],[107,65533],[65533]]]\n",
    ),
    (
        b"\xed\xa0\x80",
        "{\"a\": {\"k%s\": [\"%s\"]}}\n[1]",
        1,
        "2\n",
    ),
    (
        b"\xed\xa0\x80",
        "\"%s\"\n\"b%s\"",
        0,
        "[[\"�\",\"b�\"],[[65533],[98,65533]]]\n",
    ),
    (b"\xed\xa0\x80", "\"%s\"\n\"b%s\"", 1, "2\n"),
    (b"\xc0\xaf", "\"%s\"", 0, "[[\"��\"],[[65533,65533]]]\n"),
    (b"\xc0\xaf", "\"%s\"", 1, "1\n"),
    (
        b"\xc0\xaf",
        "[\"x%s\", 1]",
        0,
        "[[[\"x��\",1]],[[120,65533,65533]]]\n",
    ),
    (b"\xc0\xaf", "[\"x%s\", 1]", 1, "1\n"),
    (
        b"\xc0\xaf",
        "{\"k%s\": 2}",
        0,
        "[[{\"k��\":2}],[[107,65533,65533]]]\n",
    ),
    (b"\xc0\xaf", "{\"k%s\": 2}", 1, "1\n"),
    (
        b"\xc0\xaf",
        "{\"a\": {\"k%s\": [\"%s\"]}}\n[1]",
        0,
        "[[{\"a\":{\"k��\":[\"��\"]}},[1]],[[97],[107,65533,65533],[65533,65533]]]\n",
    ),
    (b"\xc0\xaf", "{\"a\": {\"k%s\": [\"%s\"]}}\n[1]", 1, "2\n"),
    (
        b"\xc0\xaf",
        "\"%s\"\n\"b%s\"",
        0,
        "[[\"��\",\"b��\"],[[65533,65533],[98,65533,65533]]]\n",
    ),
    (b"\xc0\xaf", "\"%s\"\n\"b%s\"", 1, "2\n"),
    (b"\xe1A", "\"%s\"", 0, "[[\"�\"],[[65533]]]\n"),
    (b"\xe1A", "\"%s\"", 1, "1\n"),
    (
        b"\xe1A",
        "[\"x%s\", 1]",
        0,
        "[[[\"x�\",1]],[[120,65533]]]\n",
    ),
    (b"\xe1A", "[\"x%s\", 1]", 1, "1\n"),
    (
        b"\xe1A",
        "{\"k%s\": 2}",
        0,
        "[[{\"k�\":2}],[[107,65533]]]\n",
    ),
    (b"\xe1A", "{\"k%s\": 2}", 1, "1\n"),
    (
        b"\xe1A",
        "{\"a\": {\"k%s\": [\"%s\"]}}\n[1]",
        0,
        "[[{\"a\":{\"k�\":[\"�\"]}},[1]],[[97],[107,65533],[65533]]]\n",
    ),
    (b"\xe1A", "{\"a\": {\"k%s\": [\"%s\"]}}\n[1]", 1, "2\n"),
    (
        b"\xe1A",
        "\"%s\"\n\"b%s\"",
        0,
        "[[\"�\",\"b�\"],[[65533],[98,65533]]]\n",
    ),
    (b"\xe1A", "\"%s\"\n\"b%s\"", 1, "2\n"),
    (b"\x80", "\"%s\"", 0, "[[\"�\"],[[65533]]]\n"),
    (b"\x80", "\"%s\"", 1, "1\n"),
    (b"\x80", "[\"x%s\", 1]", 0, "[[[\"x�\",1]],[[120,65533]]]\n"),
    (b"\x80", "[\"x%s\", 1]", 1, "1\n"),
    (b"\x80", "{\"k%s\": 2}", 0, "[[{\"k�\":2}],[[107,65533]]]\n"),
    (b"\x80", "{\"k%s\": 2}", 1, "1\n"),
    (
        b"\x80",
        "{\"a\": {\"k%s\": [\"%s\"]}}\n[1]",
        0,
        "[[{\"a\":{\"k�\":[\"�\"]}},[1]],[[97],[107,65533],[65533]]]\n",
    ),
    (b"\x80", "{\"a\": {\"k%s\": [\"%s\"]}}\n[1]", 1, "2\n"),
    (
        b"\x80",
        "\"%s\"\n\"b%s\"",
        0,
        "[[\"�\",\"b�\"],[[65533],[98,65533]]]\n",
    ),
    (b"\x80", "\"%s\"\n\"b%s\"", 1, "2\n"),
    (b"\xf0\x9f\x98", "\"%s\"", 0, "[[\"�\"],[[65533]]]\n"),
    (b"\xf0\x9f\x98", "\"%s\"", 1, "1\n"),
    (
        b"\xf0\x9f\x98",
        "[\"x%s\", 1]",
        0,
        "[[[\"x�\",1]],[[120,65533]]]\n",
    ),
    (b"\xf0\x9f\x98", "[\"x%s\", 1]", 1, "1\n"),
    (
        b"\xf0\x9f\x98",
        "{\"k%s\": 2}",
        0,
        "[[{\"k�\":2}],[[107,65533]]]\n",
    ),
    (b"\xf0\x9f\x98", "{\"k%s\": 2}", 1, "1\n"),
    (
        b"\xf0\x9f\x98",
        "{\"a\": {\"k%s\": [\"%s\"]}}\n[1]",
        0,
        "[[{\"a\":{\"k�\":[\"�\"]}},[1]],[[97],[107,65533],[65533]]]\n",
    ),
    (
        b"\xf0\x9f\x98",
        "{\"a\": {\"k%s\": [\"%s\"]}}\n[1]",
        1,
        "2\n",
    ),
    (
        b"\xf0\x9f\x98",
        "\"%s\"\n\"b%s\"",
        0,
        "[[\"�\",\"b�\"],[[65533],[98,65533]]]\n",
    ),
    (b"\xf0\x9f\x98", "\"%s\"\n\"b%s\"", 1, "2\n"),
    (b"\xfe\xff", "\"%s\"", 0, "[[\"��\"],[[65533,65533]]]\n"),
    (b"\xfe\xff", "\"%s\"", 1, "1\n"),
    (
        b"\xfe\xff",
        "[\"x%s\", 1]",
        0,
        "[[[\"x��\",1]],[[120,65533,65533]]]\n",
    ),
    (b"\xfe\xff", "[\"x%s\", 1]", 1, "1\n"),
    (
        b"\xfe\xff",
        "{\"k%s\": 2}",
        0,
        "[[{\"k��\":2}],[[107,65533,65533]]]\n",
    ),
    (b"\xfe\xff", "{\"k%s\": 2}", 1, "1\n"),
    (
        b"\xfe\xff",
        "{\"a\": {\"k%s\": [\"%s\"]}}\n[1]",
        0,
        "[[{\"a\":{\"k��\":[\"��\"]}},[1]],[[97],[107,65533,65533],[65533,65533]]]\n",
    ),
    (b"\xfe\xff", "{\"a\": {\"k%s\": [\"%s\"]}}\n[1]", 1, "2\n"),
    (
        b"\xfe\xff",
        "\"%s\"\n\"b%s\"",
        0,
        "[[\"��\",\"b��\"],[[65533,65533],[98,65533,65533]]]\n",
    ),
    (b"\xfe\xff", "\"%s\"\n\"b%s\"", 1, "2\n"),
    (b"\xe2\x82\xac", "\"%s\"", 0, "[[\"€\"],[[8364]]]\n"),
    (b"\xe2\x82\xac", "\"%s\"", 1, "1\n"),
    (
        b"\xe2\x82\xac",
        "[\"x%s\", 1]",
        0,
        "[[[\"x€\",1]],[[120,8364]]]\n",
    ),
    (b"\xe2\x82\xac", "[\"x%s\", 1]", 1, "1\n"),
    (
        b"\xe2\x82\xac",
        "{\"k%s\": 2}",
        0,
        "[[{\"k€\":2}],[[107,8364]]]\n",
    ),
    (b"\xe2\x82\xac", "{\"k%s\": 2}", 1, "1\n"),
    (
        b"\xe2\x82\xac",
        "{\"a\": {\"k%s\": [\"%s\"]}}\n[1]",
        0,
        "[[{\"a\":{\"k€\":[\"€\"]}},[1]],[[97],[107,8364],[8364]]]\n",
    ),
    (
        b"\xe2\x82\xac",
        "{\"a\": {\"k%s\": [\"%s\"]}}\n[1]",
        1,
        "2\n",
    ),
    (
        b"\xe2\x82\xac",
        "\"%s\"\n\"b%s\"",
        0,
        "[[\"€\",\"b€\"],[[8364],[98,8364]]]\n",
    ),
    (b"\xe2\x82\xac", "\"%s\"\n\"b%s\"", 1, "2\n"),
];

/// The programs the rows run: the value bound, with the code points of every string and key, and the
/// `$d::d` spelling of the same binding.
const PROGRAMS: [&str; 2] = [
    "import \"d\" as $d; $d | [., ([.. | strings, (objects | keys[])] | map(explode))]",
    "import \"d\" as $d; $d::d | length",
];

fn run(dir: &std::path::Path, args: &[&str]) -> Result<(String, String, i32)> {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .current_dir(dir)
        .env("LC_ALL", "C")
        .output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    ))
}

#[test]
fn a_data_module_that_is_not_utf8_reads_as_jq_reads_it_7604() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for &(sequence, shape, program, want) in ROWS {
        let mut bytes = Vec::new();
        for (at, piece) in shape.split("%s").enumerate() {
            if at > 0 {
                bytes.extend_from_slice(sequence);
            }
            bytes.extend_from_slice(piece.as_bytes());
        }
        std::fs::write(dir.path().join("d.json"), &bytes)?;
        let got = run(dir.path(), &["-nc", "-L", ".", PROGRAMS[program]])?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (want, "", 0),
            "{sequence:x?} in {shape:?}, program {program}"
        );
    }
    Ok(())
}

/// The issue's own two files, and a byte outside a string, which is still the parse error it is in
/// jq (exit 3; the parser's wording is #2703's recorded divergence).
#[test]
fn the_issues_data_modules_read_as_jq_reads_them_7604() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("d.json"), b"\"\xff\"")?;
    std::fs::write(dir.path().join("e.json"), b"{\"a\":\"\xff\"}")?;
    std::fs::write(dir.path().join("f.json"), b"[1] \xff")?;
    let got = run(dir.path(), &["-nc", "-L", ".", "import \"d\" as $d; $d"])?;
    assert_eq!(
        (got.0.as_str(), got.2),
        ("[\"\u{fffd}\"]\n", 0),
        "{}",
        got.1
    );
    let got = run(dir.path(), &["-nc", "-L", ".", "import \"e\" as $e; $e::e"])?;
    assert_eq!(
        (got.0.as_str(), got.2),
        ("[{\"a\":\"\u{fffd}\"}]\n", 0),
        "{}",
        got.1
    );
    let got = run(dir.path(), &["-nc", "-L", ".", "import \"f\" as $f; $f"])?;
    assert_eq!(got.0, "");
    assert_eq!(got.2, 3, "{}", got.1);
    assert!(
        got.1.starts_with("jq: error loading data file "),
        "{}",
        got.1
    );
    Ok(())
}
