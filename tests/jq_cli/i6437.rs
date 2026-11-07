//! #6437: `-R`'s lines are made strings as the main loop gets to them, for a program that reads no
//! `input`, where every line's string used to be built before the program ran (3.5x the file held).
//! What each line reads as is unchanged: the line a file ends without a newline goes on into the
//! next file, `input_filename` and an error's `(at file:line)` name where the line is, and a file
//! that does not open ends the run after the line its reader moved on from. Every row was captured
//! from jq 1.7.1 (`jq -R`).

use anyhow::Result;

use crate::spawn_jq;

#[test]
fn raw_input_lines_are_read_as_the_main_loop_gets_to_them_6437() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let write = |name: &str, text: &str| -> Result<String> {
        let path = dir.path().join(name);
        std::fs::write(&path, text)?;
        Ok(path.to_string_lossy().into_owned())
    };
    let a = write("a.txt", "one\ntwo\nthree")?;
    let b = write("b.txt", "four\n")?;
    let missing = dir
        .path()
        .join("missing.txt")
        .to_string_lossy()
        .into_owned();
    let named = |text: &str| {
        text.replace('A', &a)
            .replace('B', &b)
            .replace('M', &missing)
    };
    for (args, stdout, stderr, status) in [
        (
            vec!["-c", "[.,input_filename]", &a, &b],
            "[\"one\",\"A\"]\n[\"two\",\"A\"]\n[\"threefour\",\"B\"]\n",
            "",
            0,
        ),
        (
            vec![
                "-c",
                "if . == \"two\" then error(\"bad \\(.)\") else . end",
                &a,
                &b,
            ],
            "\"one\"\n\"threefour\"\n",
            "jq: error (at A:2): bad two\n",
            0,
        ),
        (
            vec!["-c", ".", &a, &missing, &b],
            "\"one\"\n\"two\"\n\"threefour\"\n",
            "jq: error: Could not open file M: No such file or directory\n",
            2,
        ),
        (vec!["-c", "length", &b, &a], "4\n3\n3\n5\n", "", 0),
    ] {
        let mut argv = vec!["-R"];
        argv.extend(&args);
        let (output, code) = spawn_jq(&argv, None)?;
        assert_eq!(String::from_utf8(output.stdout)?, named(stdout), "{args:?}");
        assert_eq!(String::from_utf8(output.stderr)?, named(stderr), "{args:?}");
        assert_eq!(code, status, "{args:?}");
    }
    Ok(())
}
