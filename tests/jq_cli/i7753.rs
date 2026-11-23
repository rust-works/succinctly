//! #7753: a file that does not open was reported before a fault in an earlier file when the faulty
//! document was still open at the end of its file, and the run exited 2. jq opens the files in turn,
//! only once its parser has read every byte of the one before, so a fault in those bytes (`{"a",`)
//! is raised first and the file after it is never opened: exit 5. The stitch of #4305 carried the
//! faulty document onto the next file, where the fault was then met past the failure. Every expected
//! string was captured from `/usr/bin/jq` 1.7.1, and jq 1.8.2 gives the same.

use anyhow::Result;

use crate::run_jq_in_dir;

fn files() -> Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    for (name, text) in [
        ("o.json", "{\"a\","),
        ("o2.json", ":2}\n"),
        ("g.json", "1\n2\n"),
        ("s1.json", "{\""),
        ("s2.json", "\n\""),
        ("c.json", "2\n"),
        ("z.json", "[0,\"s\n\"\"]\n"),
        ("em.json", ""),
        ("k.json", "{\"a\":1}\n[1,2"),
        ("k2.json", ",x]\n"),
        ("e00.json", "3\n"),
        ("e01.json", "3\n"),
        ("e02.json", "3\n"),
        ("e03.json", "3\n"),
        ("e04.json", "3\n"),
        ("e05.json", "3\n"),
        ("e06.json", "3\n"),
        ("e07.json", "3\n"),
        ("e08.json", "3\n"),
        ("e09.json", "3\n"),
        ("e10.json", "3\n"),
        ("e11.json", "3\n"),
        ("e12.json", "3\n"),
        ("e13.json", "3\n"),
        ("e14.json", "3\n"),
        ("e15.json", "3\n"),
        ("e16.json", "3\n"),
        ("e17.json", "3\n"),
        ("e18.json", "3\n"),
        ("e19.json", "3\n"),
        ("e20.json", "3\n"),
        ("e21.json", "3\n"),
        ("e22.json", "3\n"),
        ("e23.json", "3\n"),
        ("e24.json", "3\n"),
        ("e25.json", "3\n"),
        ("e26.json", "3\n"),
        ("e27.json", "3\n"),
        ("e28.json", "3\n"),
        ("e29.json", "3\n"),
        ("e30.json", "3\n"),
        ("e31.json", "3\n"),
        ("e32.json", "3\n"),
        ("e33.json", "3\n"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    std::fs::create_dir(dir.path().join("dir"))?;
    Ok(dir)
}

/// (arguments, standard input, stdout, stderr, exit status).
type Row = (
    &'static [&'static str],
    Option<&'static str>,
    &'static str,
    &'static str,
    i32,
);

fn check(rows: &[Row]) -> Result<()> {
    let dir = files()?;
    for &(args, input, stdout, stderr, code) in rows {
        let (out, err, status) = run_jq_in_dir(dir.path(), args, input)?;
        assert_eq!(out, stdout, "{args:?} stdout");
        assert_eq!(err, stderr, "{args:?} stderr");
        assert_eq!(status, code, "{args:?} status");
    }
    Ok(())
}

#[test]
fn a_fault_before_a_file_that_does_not_open_is_raised_first_7753() -> Result<()> {
    check(&[
        (&["-c", ".", "o.json", "missing.json", "o2.json"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 5\n", 5),
        (&["-c", ".", "g.json", "o.json", "missing.json", "o2.json"], None, "1\n2\n", "jq: parse error: Objects must consist of key:value pairs at line 3, column 5\n", 5),
        (&["-c", ".", "o.json", "missing.json", "missing2.json", "o2.json"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 5\n", 5),
        (&["-c", "-e", ".", "o.json", "missing.json", "o2.json"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 5\n", 5),
        (&["-r", "[., input_filename]", "o.json", "missing.json", "o2.json"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 5\n", 5),
        (&["-c", ".", "s1.json", "s2.json", "missing.json", "c.json"], None, "", "jq: parse error: Invalid string: control characters from U+0000 through U+001F must be escaped at line 2, column 1\n", 5),
        (&["-c", ".", "z.json", "missing.json", "em.json"], None, "", "jq: parse error: Invalid string: control characters from U+0000 through U+001F must be escaped at line 2, column 1\n", 5),
        (&["-c", ".", "k.json", "missing.json", "k2.json"], None, "{\"a\":1}\n", "jq: error: Could not open file missing.json: No such file or directory\njq: parse error: Invalid numeric literal at line 2, column 7\n", 2),
        (&["-c", ".", "o.json", "missing.json", "e00.json", "e01.json", "e02.json", "e03.json", "e04.json", "e05.json", "e06.json", "e07.json", "e08.json", "e09.json", "e10.json", "e11.json", "e12.json", "e13.json", "e14.json", "e15.json", "e16.json", "e17.json", "e18.json", "e19.json", "e20.json", "e21.json", "e22.json", "e23.json", "e24.json", "e25.json", "e26.json", "e27.json", "e28.json", "e29.json", "e30.json", "e31.json", "e32.json", "e33.json", "o2.json"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 5\n", 5),
        (&["-c", ".", "-", "missing.json", "o2.json"], Some("{\"a\","), "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 5\n", 5),
        (&["-c", ".", "o.json", "dir", "o2.json"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 5\n", 5),
        (&["-c", ".", "g.json", "o.json", "dir", "o2.json"], None, "1\n2\n", "jq: parse error: Objects must consist of key:value pairs at line 3, column 5\n", 5),
        (&["-c", ".", "o.json", "dir", "missing2.json", "o2.json"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 5\n", 5),
        (&["-c", "-e", ".", "o.json", "dir", "o2.json"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 5\n", 5),
        (&["-r", "[., input_filename]", "o.json", "dir", "o2.json"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 5\n", 5),
        (&["-c", ".", "s1.json", "s2.json", "dir", "c.json"], None, "", "jq: parse error: Invalid string: control characters from U+0000 through U+001F must be escaped at line 2, column 1\n", 5),
        (&["-c", ".", "z.json", "dir", "em.json"], None, "", "jq: parse error: Invalid string: control characters from U+0000 through U+001F must be escaped at line 2, column 1\n", 5),
        (&["-c", ".", "k.json", "dir", "k2.json"], None, "{\"a\":1}\n", "jq: error: Is a directory\njq: parse error: Invalid numeric literal at line 2, column 7\n", 2),
        (&["-c", ".", "o.json", "dir", "e00.json", "e01.json", "e02.json", "e03.json", "e04.json", "e05.json", "e06.json", "e07.json", "e08.json", "e09.json", "e10.json", "e11.json", "e12.json", "e13.json", "e14.json", "e15.json", "e16.json", "e17.json", "e18.json", "e19.json", "e20.json", "e21.json", "e22.json", "e23.json", "e24.json", "e25.json", "e26.json", "e27.json", "e28.json", "e29.json", "e30.json", "e31.json", "e32.json", "e33.json", "o2.json"], None, "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 5\n", 5),
        (&["-c", ".", "-", "dir", "o2.json"], Some("{\"a\","), "", "jq: parse error: Objects must consist of key:value pairs at line 1, column 5\n", 5),
        (&["-c", ".", "g.json", "missing.json", "o.json", "o2.json"], None, "1\n2\n", "jq: error: Could not open file missing.json: No such file or directory\njq: parse error: Objects must consist of key:value pairs at line 3, column 5\n", 2),
    ])
}

/// The same over a file past 1 MiB, which the plain route reads a window at a time (#6920): the
/// window that ends the file holds the fault, and the failure after it is never reached.
#[test]
fn a_fault_before_a_file_that_does_not_open_is_raised_first_windowed_7753() -> Result<()> {
    let dir = files()?;
    let pad = "{\"pad\":1}\n".repeat(110_000);
    std::fs::write(dir.path().join("big.json"), format!("{pad}{{\"a\","))?;
    for bad in ["missing.json", "dir"] {
        let (out, err, status) =
            run_jq_in_dir(dir.path(), &["-c", ".", "big.json", bad, "o2.json"], None)?;
        assert!(out == pad, "{bad}: stdout is the pad");
        assert_eq!(
            err,
            "jq: parse error: Objects must consist of key:value pairs at line 110001, column 5\n",
            "{bad} stderr"
        );
        assert_eq!(status, 5, "{bad} status");
    }
    Ok(())
}
