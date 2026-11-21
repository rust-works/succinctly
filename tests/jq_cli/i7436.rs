//! #7436: a `-f` program file, a `--rawfile` file or a `--slurpfile` file holding a byte that is not
//! valid UTF-8 is read as jq 1.7.1 and 1.8.2 read it, with U+FFFD, where it was refused with `Could not
//! open ...: stream did not contain valid UTF-8` (exit 2). jq's `jv_load_file` reads `-f` and
//! `--rawfile` 4,096 bytes at a time and decodes each read on its own, so a sequence cut by the end of
//! a read is pinned at the edges of the first two reads; `--slurpfile` is parsed, its strings decoded
//! as the main input's are. A program whose U+FFFD is outside a string or a comment is a compile error
//! (exit 3), as in jq.
//!
//! Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`). jq 1.8.2 agrees on every row but the
//! compile errors, whose layout 1.8 changed (a feature: 1.7.1 stays the reference).

use anyhow::Result;

use crate::spawn_jq;

/// `--rawfile`: (sequence, bytes of `a` before it, text after it, jq's line).
const RAWFILE: &[(&[u8], usize, &str, &str)] = &[
    (b"\xff", 0, "", "[1,[0],[65533]]\n"),
    (b"\xff", 0, "B", "[2,[0],[65533,66]]\n"),
    (b"\xff", 0, "BC\n", "[4,[0],[66,67,10]]\n"),
    (b"\xff", 4094, "", "[4095,[4094],[97,97,65533]]\n"),
    (b"\xff", 4094, "B", "[4096,[4094],[97,65533,66]]\n"),
    (b"\xff", 4094, "BC\n", "[4098,[4094],[66,67,10]]\n"),
    (b"\xff", 4095, "", "[4096,[4095],[97,97,65533]]\n"),
    (b"\xff", 4095, "B", "[4097,[4095],[97,65533,66]]\n"),
    (b"\xff", 4095, "BC\n", "[4099,[4095],[66,67,10]]\n"),
    (b"\xff", 4096, "", "[4097,[4096],[97,97,65533]]\n"),
    (b"\xff", 4096, "B", "[4098,[4096],[97,65533,66]]\n"),
    (b"\xff", 4096, "BC\n", "[4100,[4096],[66,67,10]]\n"),
    (b"\xff", 8191, "", "[8192,[8191],[97,97,65533]]\n"),
    (b"\xff", 8191, "B", "[8193,[8191],[97,65533,66]]\n"),
    (b"\xff", 8191, "BC\n", "[8195,[8191],[66,67,10]]\n"),
    (b"\xc3", 0, "", "[1,[0],[65533]]\n"),
    (b"\xc3", 0, "B", "[2,[0],[65533,66]]\n"),
    (b"\xc3", 0, "BC\n", "[4,[0],[66,67,10]]\n"),
    (b"\xc3", 4094, "", "[4095,[4094],[97,97,65533]]\n"),
    (b"\xc3", 4094, "B", "[4096,[4094],[97,65533,66]]\n"),
    (b"\xc3", 4094, "BC\n", "[4098,[4094],[66,67,10]]\n"),
    (b"\xc3", 4095, "", "[4096,[4095],[97,97,65533]]\n"),
    (b"\xc3", 4095, "B", "[4097,[4095],[97,65533,66]]\n"),
    (b"\xc3", 4095, "BC\n", "[4099,[4095],[66,67,10]]\n"),
    (b"\xc3", 4096, "", "[4097,[4096],[97,97,65533]]\n"),
    (b"\xc3", 4096, "B", "[4098,[4096],[97,65533,66]]\n"),
    (b"\xc3", 4096, "BC\n", "[4100,[4096],[66,67,10]]\n"),
    (b"\xc3", 8191, "", "[8192,[8191],[97,97,65533]]\n"),
    (b"\xc3", 8191, "B", "[8193,[8191],[97,65533,66]]\n"),
    (b"\xc3", 8191, "BC\n", "[8195,[8191],[66,67,10]]\n"),
    (b"\xe1\x41", 0, "", "[1,[0],[65533]]\n"),
    (b"\xe1\x41", 0, "B", "[3,[0],[65533,65,66]]\n"),
    (b"\xe1\x41", 0, "BC\n", "[5,[0],[66,67,10]]\n"),
    (b"\xe1\x41", 4094, "", "[4095,[4094],[97,97,65533]]\n"),
    (b"\xe1\x41", 4094, "B", "[4096,[4094],[97,65533,66]]\n"),
    (b"\xe1\x41", 4094, "BC\n", "[4098,[4094],[66,67,10]]\n"),
    (b"\xe1\x41", 4095, "", "[4096,[4095],[97,97,65533]]\n"),
    (b"\xe1\x41", 4095, "B", "[4098,[4095],[65533,65,66]]\n"),
    (b"\xe1\x41", 4095, "BC\n", "[4100,[4095],[66,67,10]]\n"),
    (b"\xe1\x41", 4096, "", "[4097,[4096],[97,97,65533]]\n"),
    (b"\xe1\x41", 4096, "B", "[4099,[4096],[65533,65,66]]\n"),
    (b"\xe1\x41", 4096, "BC\n", "[4101,[4096],[66,67,10]]\n"),
    (b"\xe1\x41", 8191, "", "[8192,[8191],[97,97,65533]]\n"),
    (b"\xe1\x41", 8191, "B", "[8194,[8191],[65533,65,66]]\n"),
    (b"\xe1\x41", 8191, "BC\n", "[8196,[8191],[66,67,10]]\n"),
    (b"\xf0\x9f\x98", 0, "", "[1,[0],[65533]]\n"),
    (b"\xf0\x9f\x98", 0, "B", "[2,[0],[65533,66]]\n"),
    (b"\xf0\x9f\x98", 0, "BC\n", "[4,[0],[66,67,10]]\n"),
    (b"\xf0\x9f\x98", 4094, "", "[4095,[4094],[97,97,65533]]\n"),
    (b"\xf0\x9f\x98", 4094, "B", "[4096,[4094],[97,65533,66]]\n"),
    (b"\xf0\x9f\x98", 4094, "BC\n", "[4098,[4094],[66,67,10]]\n"),
    (b"\xf0\x9f\x98", 4095, "", "[4096,[4095],[97,97,65533]]\n"),
    (b"\xf0\x9f\x98", 4095, "B", "[4097,[4095],[97,65533,66]]\n"),
    (b"\xf0\x9f\x98", 4095, "BC\n", "[4099,[4095],[66,67,10]]\n"),
    (b"\xf0\x9f\x98", 4096, "", "[4097,[4096],[97,97,65533]]\n"),
    (b"\xf0\x9f\x98", 4096, "B", "[4098,[4096],[97,65533,66]]\n"),
    (b"\xf0\x9f\x98", 4096, "BC\n", "[4100,[4096],[66,67,10]]\n"),
    (b"\xf0\x9f\x98", 8191, "", "[8192,[8191],[97,97,65533]]\n"),
    (b"\xf0\x9f\x98", 8191, "B", "[8193,[8191],[97,65533,66]]\n"),
    (b"\xf0\x9f\x98", 8191, "BC\n", "[8195,[8191],[66,67,10]]\n"),
    (b"\xe2\x82\xac", 0, "", "[1,[],[8364]]\n"),
    (b"\xe2\x82\xac", 0, "B", "[2,[],[8364,66]]\n"),
    (b"\xe2\x82\xac", 0, "BC\n", "[4,[],[66,67,10]]\n"),
    (b"\xe2\x82\xac", 4094, "", "[4095,[],[97,97,8364]]\n"),
    (b"\xe2\x82\xac", 4094, "B", "[4096,[],[97,8364,66]]\n"),
    (b"\xe2\x82\xac", 4094, "BC\n", "[4098,[],[66,67,10]]\n"),
    (b"\xe2\x82\xac", 4095, "", "[4096,[],[97,97,8364]]\n"),
    (b"\xe2\x82\xac", 4095, "B", "[4097,[],[97,8364,66]]\n"),
    (b"\xe2\x82\xac", 4095, "BC\n", "[4099,[],[66,67,10]]\n"),
    (b"\xe2\x82\xac", 4096, "", "[4097,[],[97,97,8364]]\n"),
    (b"\xe2\x82\xac", 4096, "B", "[4098,[],[97,8364,66]]\n"),
    (b"\xe2\x82\xac", 4096, "BC\n", "[4100,[],[66,67,10]]\n"),
    (b"\xe2\x82\xac", 8191, "", "[8192,[],[97,97,8364]]\n"),
    (b"\xe2\x82\xac", 8191, "B", "[8193,[],[97,8364,66]]\n"),
    (b"\xe2\x82\xac", 8191, "BC\n", "[8195,[],[66,67,10]]\n"),
];
/// `-f`: (sequence, program shape with `%s` where it goes, jq's stdout, stderr, exit).
const FROM_FILE: &[(&[u8], &str, &str, &str, i32)] = &[
    (b"\xff", "\"%s\" | explode", "[65533]\n", "", 0),
    (b"\xff", "# %s\n1", "1\n", "", 0),
    (b"\xff", ". %s", "", "jq: error: syntax error, unexpected INVALID_CHARACTER (Unix shell quoting issues?) at <top-level>, line 1:\n. �  \njq: error: try .[\"field\"] instead of .field for unusually named fields at <top-level>, line 1:\n. �\njq: 2 compile errors\n", 3),
    (b"\xff", "\"a\" | . + \"%s\" | length", "2\n", "", 0),
    (b"\xc3", "\"%s\" | explode", "[65533]\n", "", 0),
    (b"\xc3", "# %s\n1", "1\n", "", 0),
    (b"\xc3", ". %s", "", "jq: error: syntax error, unexpected INVALID_CHARACTER (Unix shell quoting issues?) at <top-level>, line 1:\n. �  \njq: error: try .[\"field\"] instead of .field for unusually named fields at <top-level>, line 1:\n. �\njq: 2 compile errors\n", 3),
    (b"\xc3", "\"a\" | . + \"%s\" | length", "2\n", "", 0),
    (b"\xed\xa0\x80", "\"%s\" | explode", "[65533]\n", "", 0),
    (b"\xed\xa0\x80", "# %s\n1", "1\n", "", 0),
    (b"\xed\xa0\x80", ". %s", "", "jq: error: syntax error, unexpected INVALID_CHARACTER (Unix shell quoting issues?) at <top-level>, line 1:\n. �  \njq: error: try .[\"field\"] instead of .field for unusually named fields at <top-level>, line 1:\n. �\njq: 2 compile errors\n", 3),
    (b"\xed\xa0\x80", "\"a\" | . + \"%s\" | length", "2\n", "", 0),
    (b"\xc0\xaf", "\"%s\" | explode", "[65533,65533]\n", "", 0),
    (b"\xc0\xaf", "# %s\n1", "1\n", "", 0),
    (b"\xc0\xaf", ". %s", "", "jq: error: syntax error, unexpected INVALID_CHARACTER (Unix shell quoting issues?) at <top-level>, line 1:\n. ��  \njq: error: try .[\"field\"] instead of .field for unusually named fields at <top-level>, line 1:\n. ��\njq: 2 compile errors\n", 3),
    (b"\xc0\xaf", "\"a\" | . + \"%s\" | length", "3\n", "", 0),
    (b"\xe1\x41", "\"%s\" | explode", "[65533,65]\n", "", 0),
    (b"\xe1\x41", "# %s\n1", "1\n", "", 0),
    (b"\xe1\x41", ". %s", "", "jq: error: syntax error, unexpected INVALID_CHARACTER (Unix shell quoting issues?) at <top-level>, line 1:\n. �A  \njq: error: try .[\"field\"] instead of .field for unusually named fields at <top-level>, line 1:\n. �A\njq: 2 compile errors\n", 3),
    (b"\xe1\x41", "\"a\" | . + \"%s\" | length", "3\n", "", 0),
    (b"\x80", "\"%s\" | explode", "[65533]\n", "", 0),
    (b"\x80", "# %s\n1", "1\n", "", 0),
    (b"\x80", ". %s", "", "jq: error: syntax error, unexpected INVALID_CHARACTER (Unix shell quoting issues?) at <top-level>, line 1:\n. �  \njq: error: try .[\"field\"] instead of .field for unusually named fields at <top-level>, line 1:\n. �\njq: 2 compile errors\n", 3),
    (b"\x80", "\"a\" | . + \"%s\" | length", "2\n", "", 0),
    (b"\xf0\x9f\x98", "\"%s\" | explode", "[65533]\n", "", 0),
    (b"\xf0\x9f\x98", "# %s\n1", "1\n", "", 0),
    (b"\xf0\x9f\x98", ". %s", "", "jq: error: syntax error, unexpected INVALID_CHARACTER (Unix shell quoting issues?) at <top-level>, line 1:\n. �  \njq: error: try .[\"field\"] instead of .field for unusually named fields at <top-level>, line 1:\n. �\njq: 2 compile errors\n", 3),
    (b"\xf0\x9f\x98", "\"a\" | . + \"%s\" | length", "2\n", "", 0),
    (b"\xe2\x82\xac", "\"%s\" | explode", "[8364]\n", "", 0),
    (b"\xe2\x82\xac", "# %s\n1", "1\n", "", 0),
    (b"\xe2\x82\xac", ". %s", "", "jq: error: syntax error, unexpected INVALID_CHARACTER (Unix shell quoting issues?) at <top-level>, line 1:\n. €  \njq: error: try .[\"field\"] instead of .field for unusually named fields at <top-level>, line 1:\n. €\njq: 2 compile errors\n", 3),
    (b"\xe2\x82\xac", "\"a\" | . + \"%s\" | length", "2\n", "", 0),
];
/// `--slurpfile`: (sequence, file shape, jq's line).
const SLURPFILE: &[(&[u8], &str, &str)] = &[
    (b"\xff", "\"%s\"", "[[\"�\"],[[65533]]]\n"),
    (b"\xff", "[\"x%s\", 1]", "[[[\"x�\",1]],[[120,65533]]]\n"),
    (b"\xff", "{\"k%s\": 2}", "[[{\"k�\":2}],[[107,65533]]]\n"),
    (b"\xc3", "\"%s\"", "[[\"�\"],[[65533]]]\n"),
    (b"\xc3", "[\"x%s\", 1]", "[[[\"x�\",1]],[[120,65533]]]\n"),
    (b"\xc3", "{\"k%s\": 2}", "[[{\"k�\":2}],[[107,65533]]]\n"),
    (b"\xed\xa0\x80", "\"%s\"", "[[\"�\"],[[65533]]]\n"),
    (
        b"\xed\xa0\x80",
        "[\"x%s\", 1]",
        "[[[\"x�\",1]],[[120,65533]]]\n",
    ),
    (
        b"\xed\xa0\x80",
        "{\"k%s\": 2}",
        "[[{\"k�\":2}],[[107,65533]]]\n",
    ),
    (b"\xc0\xaf", "\"%s\"", "[[\"��\"],[[65533,65533]]]\n"),
    (
        b"\xc0\xaf",
        "[\"x%s\", 1]",
        "[[[\"x��\",1]],[[120,65533,65533]]]\n",
    ),
    (
        b"\xc0\xaf",
        "{\"k%s\": 2}",
        "[[{\"k��\":2}],[[107,65533,65533]]]\n",
    ),
    (b"\xe1\x41", "\"%s\"", "[[\"�\"],[[65533]]]\n"),
    (
        b"\xe1\x41",
        "[\"x%s\", 1]",
        "[[[\"x�\",1]],[[120,65533]]]\n",
    ),
    (
        b"\xe1\x41",
        "{\"k%s\": 2}",
        "[[{\"k�\":2}],[[107,65533]]]\n",
    ),
    (b"\x80", "\"%s\"", "[[\"�\"],[[65533]]]\n"),
    (b"\x80", "[\"x%s\", 1]", "[[[\"x�\",1]],[[120,65533]]]\n"),
    (b"\x80", "{\"k%s\": 2}", "[[{\"k�\":2}],[[107,65533]]]\n"),
    (b"\xf0\x9f\x98", "\"%s\"", "[[\"�\"],[[65533]]]\n"),
    (
        b"\xf0\x9f\x98",
        "[\"x%s\", 1]",
        "[[[\"x�\",1]],[[120,65533]]]\n",
    ),
    (
        b"\xf0\x9f\x98",
        "{\"k%s\": 2}",
        "[[{\"k�\":2}],[[107,65533]]]\n",
    ),
    (b"\xe2\x82\xac", "\"%s\"", "[[\"€\"],[[8364]]]\n"),
    (
        b"\xe2\x82\xac",
        "[\"x%s\", 1]",
        "[[[\"x€\",1]],[[120,8364]]]\n",
    ),
    (
        b"\xe2\x82\xac",
        "{\"k%s\": 2}",
        "[[{\"k€\":2}],[[107,8364]]]\n",
    ),
];

fn run(dir: &std::path::Path, args: &[&str]) -> Result<(String, String, i32)> {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command
        .arg("jq")
        .args(args)
        .current_dir(dir)
        .env("LC_ALL", "C");
    let output = command.output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    ))
}

#[test]
fn a_rawfile_that_is_not_utf8_reads_as_jq_reads_it_7436() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for &(sequence, at, tail, want) in RAWFILE {
        let mut bytes = vec![b'a'; at];
        bytes.extend_from_slice(sequence);
        bytes.extend_from_slice(tail.as_bytes());
        std::fs::write(dir.path().join("r.txt"), &bytes)?;
        let got = run(
            dir.path(),
            &[
                "-nc",
                "$x | [length, (explode | indices(65533)), (explode | .[-3:])]",
                "--rawfile",
                "x",
                "r.txt",
            ],
        )?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (want, "", 0),
            "{sequence:x?} after {at} bytes, then {tail:?}"
        );
    }
    Ok(())
}

#[test]
fn a_program_file_that_is_not_utf8_reads_as_jq_reads_it_7436() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for &(sequence, shape, stdout, stderr, status) in FROM_FILE {
        let (before, after) = shape.split_once("%s").expect("a slot");
        let mut program = before.as_bytes().to_vec();
        program.extend_from_slice(sequence);
        program.extend_from_slice(after.as_bytes());
        program.push(b'\n');
        std::fs::write(dir.path().join("p.jq"), &program)?;
        let got = run(dir.path(), &["-nc", "-f", "p.jq"])?;
        // A character that is not ASCII after `.` is a compile error in both, worded otherwise here
        // whatever the character, a valid one included (`jq -n '. €'`): only the exit status and
        // the empty output are this issue's.
        let stderr = if shape == ". %s" {
            got.1.as_str()
        } else {
            stderr
        };
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (stdout, stderr, status),
            "{sequence:x?} in {shape:?}"
        );
    }
    Ok(())
}

#[test]
fn a_slurpfile_that_is_not_utf8_reads_as_jq_reads_it_7436() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for &(sequence, shape, want) in SLURPFILE {
        let (before, after) = shape.split_once("%s").expect("a slot");
        let mut bytes = before.as_bytes().to_vec();
        bytes.extend_from_slice(sequence);
        bytes.extend_from_slice(after.as_bytes());
        std::fs::write(dir.path().join("s.json"), &bytes)?;
        let got = run(
            dir.path(),
            &[
                "-nc",
                "$x | [., ([.. | strings, (objects | keys[])] | map(explode))]",
                "--slurpfile",
                "x",
                "s.json",
            ],
        )?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (want, "", 0),
            "{sequence:x?} in {shape:?}"
        );
    }
    Ok(())
}

/// The reads that make up the file are jq's, with a stdin pipe in place of the file too: a program
/// on `/dev/stdin` and the issue's own three files.
#[test]
fn the_issues_files_read_as_jq_reads_them_7436() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("p.jq"), b"\"\xff\" | [., length]\n")?;
    std::fs::write(dir.path().join("r.txt"), b"a\xffb\n")?;
    std::fs::write(dir.path().join("s.json"), b"\"\xff\"")?;
    for (args, want) in [
        (&["-nc", "-f", "p.jq"][..], "[\"\u{fffd}\",1]\n"),
        (
            &["-nc", "$x", "--rawfile", "x", "r.txt"],
            "\"a\u{fffd}b\\n\"\n",
        ),
        (
            &["-nc", "$x", "--slurpfile", "x", "s.json"],
            "[\"\u{fffd}\"]\n",
        ),
    ] {
        let got = run(dir.path(), args)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (want, "", 0),
            "{args:?}"
        );
    }
    let (output, status) = spawn_jq(&["-nc", "-f", "/dev/stdin"], Some(b"\"\xc3\" | explode\n"))?;
    assert_eq!(
        (String::from_utf8_lossy(&output.stdout).as_ref(), status),
        ("[65533]\n", 0)
    );
    Ok(())
}
