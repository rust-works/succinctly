//! #7752: a number or literal followed directly by a closer or separator (`1}`), in a file after one
//! that did not open, printed the scalar and lost the parse error. jq's main loop stops once its
//! reader is past a failure, after the one value it was fetching; here that fetch meets the fault on
//! the byte that would complete the token, so jq returns the fault, not the token (#4311), and
//! reports it. The cut to one value past the failure came before the check that drops such a token.
//! Every expected string was captured from `/usr/bin/jq` 1.7.1, and jq 1.8.2 gives the same.

use anyhow::Result;

use crate::run_jq_in_dir;

fn files() -> Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    for (name, text) in [
        ("n.json", "1}\n"),
        ("two.json", "2\n"),
        ("sp.json", "1 }\n"),
        ("ok.json", "7\n8\n"),
        ("s0.json", "-1]\n"),
        ("s1.json", "1.,\n"),
        ("s2.json", "null:\n"),
        ("s3.json", "true}\n"),
        ("s4.json", "12]\n3\n"),
        ("s5.json", "\"a\"}\n"),
        ("s6.json", "[1]]\n"),
        ("m00.json", "5\n"),
        ("m01.json", "5\n"),
        ("m02.json", "5\n"),
        ("m03.json", "5\n"),
        ("m04.json", "5\n"),
        ("m05.json", "5\n"),
        ("m06.json", "5\n"),
        ("m07.json", "5\n"),
        ("m08.json", "5\n"),
        ("m09.json", "5\n"),
        ("m10.json", "5\n"),
        ("m11.json", "5\n"),
        ("m12.json", "5\n"),
        ("m13.json", "5\n"),
        ("m14.json", "5\n"),
        ("m15.json", "5\n"),
        ("m16.json", "5\n"),
        ("m17.json", "5\n"),
        ("m18.json", "5\n"),
        ("m19.json", "5\n"),
        ("m20.json", "5\n"),
        ("m21.json", "5\n"),
        ("m22.json", "5\n"),
        ("m23.json", "5\n"),
        ("m24.json", "5\n"),
        ("m25.json", "5\n"),
        ("m26.json", "5\n"),
        ("m27.json", "5\n"),
        ("m28.json", "5\n"),
        ("m29.json", "5\n"),
        ("m30.json", "5\n"),
        ("m31.json", "5\n"),
        ("m32.json", "5\n"),
        ("m33.json", "5\n"),
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
fn a_token_the_fault_completes_past_a_failure_is_not_read_7752() -> Result<()> {
    check(&[
        (&["-c", ".", "missing.json", "n.json"], None, "", "jq: error: Could not open file missing.json: No such file or directory\njq: parse error: Unmatched '}' at line 1, column 2\n", 2),
        (&["-c", ".", "two.json", "missing.json", "n.json"], None, "2\n", "jq: error: Could not open file missing.json: No such file or directory\njq: parse error: Unmatched '}' at line 2, column 2\n", 2),
        (&["-c", "-e", ".", "missing.json", "n.json"], None, "", "jq: error: Could not open file missing.json: No such file or directory\njq: parse error: Unmatched '}' at line 1, column 2\n", 2),
        (&["-r", ".", "missing.json", "n.json"], None, "", "jq: error: Could not open file missing.json: No such file or directory\njq: parse error: Unmatched '}' at line 1, column 2\n", 2),
        (&["-c", ".", "missing.json", "-"], Some("1}\n"), "", "jq: error: Could not open file missing.json: No such file or directory\njq: parse error: Unmatched '}' at line 1, column 2\n", 2),
        (&["-c", ".", "missing.json", "n.json", "m00.json", "m01.json", "m02.json", "m03.json", "m04.json", "m05.json", "m06.json", "m07.json", "m08.json", "m09.json", "m10.json", "m11.json", "m12.json", "m13.json", "m14.json", "m15.json", "m16.json", "m17.json", "m18.json", "m19.json", "m20.json", "m21.json", "m22.json", "m23.json", "m24.json", "m25.json", "m26.json", "m27.json", "m28.json", "m29.json", "m30.json", "m31.json", "m32.json", "m33.json"], None, "", "jq: error: Could not open file missing.json: No such file or directory\njq: parse error: Unmatched '}' at line 1, column 2\n", 2),
        (&["-c", ".", "missing.json", "s0.json", "ok.json"], None, "", "jq: error: Could not open file missing.json: No such file or directory\njq: parse error: Unmatched ']' at line 1, column 3\n", 2),
        (&["-c", ".", "missing.json", "s1.json", "ok.json"], None, "", "jq: error: Could not open file missing.json: No such file or directory\njq: parse error: Expected value before ',' at line 1, column 3\n", 2),
        (&["-c", ".", "missing.json", "s2.json", "ok.json"], None, "", "jq: error: Could not open file missing.json: No such file or directory\njq: parse error: Expected string key before ':' at line 1, column 5\n", 2),
        (&["-c", ".", "missing.json", "s3.json", "ok.json"], None, "", "jq: error: Could not open file missing.json: No such file or directory\njq: parse error: Unmatched '}' at line 1, column 5\n", 2),
        (&["-c", ".", "missing.json", "s4.json", "ok.json"], None, "", "jq: error: Could not open file missing.json: No such file or directory\njq: parse error: Unmatched ']' at line 1, column 3\n", 2),
        (&["-c", ".", "missing.json", "s5.json", "ok.json"], None, "\"a\"\n", "jq: error: Could not open file missing.json: No such file or directory\n", 2),
        (&["-c", ".", "missing.json", "s6.json", "ok.json"], None, "[1]\n", "jq: error: Could not open file missing.json: No such file or directory\n", 2),
        (&["-c", ".", "missing.json", "sp.json"], None, "1\n", "jq: error: Could not open file missing.json: No such file or directory\n", 2),
        (&["-c", ".", "dir", "n.json"], None, "", "jq: error: Is a directory\njq: parse error: Unmatched '}' at line 1, column 2\n", 2),
        (&["-c", ".", "two.json", "dir", "n.json"], None, "2\n", "jq: error: Is a directory\njq: parse error: Unmatched '}' at line 2, column 2\n", 2),
        (&["-c", "-e", ".", "dir", "n.json"], None, "", "jq: error: Is a directory\njq: parse error: Unmatched '}' at line 1, column 2\n", 2),
        (&["-r", ".", "dir", "n.json"], None, "", "jq: error: Is a directory\njq: parse error: Unmatched '}' at line 1, column 2\n", 2),
        (&["-c", ".", "dir", "-"], Some("1}\n"), "", "jq: error: Is a directory\njq: parse error: Unmatched '}' at line 1, column 2\n", 2),
        (&["-c", ".", "dir", "n.json", "m00.json", "m01.json", "m02.json", "m03.json", "m04.json", "m05.json", "m06.json", "m07.json", "m08.json", "m09.json", "m10.json", "m11.json", "m12.json", "m13.json", "m14.json", "m15.json", "m16.json", "m17.json", "m18.json", "m19.json", "m20.json", "m21.json", "m22.json", "m23.json", "m24.json", "m25.json", "m26.json", "m27.json", "m28.json", "m29.json", "m30.json", "m31.json", "m32.json", "m33.json"], None, "", "jq: error: Is a directory\njq: parse error: Unmatched '}' at line 1, column 2\n", 2),
        (&["-c", ".", "dir", "s0.json", "ok.json"], None, "", "jq: error: Is a directory\njq: parse error: Unmatched ']' at line 1, column 3\n", 2),
        (&["-c", ".", "dir", "s1.json", "ok.json"], None, "", "jq: error: Is a directory\njq: parse error: Expected value before ',' at line 1, column 3\n", 2),
        (&["-c", ".", "dir", "s2.json", "ok.json"], None, "", "jq: error: Is a directory\njq: parse error: Expected string key before ':' at line 1, column 5\n", 2),
        (&["-c", ".", "dir", "s3.json", "ok.json"], None, "", "jq: error: Is a directory\njq: parse error: Unmatched '}' at line 1, column 5\n", 2),
        (&["-c", ".", "dir", "s4.json", "ok.json"], None, "", "jq: error: Is a directory\njq: parse error: Unmatched ']' at line 1, column 3\n", 2),
        (&["-c", ".", "dir", "s5.json", "ok.json"], None, "\"a\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", ".", "dir", "s6.json", "ok.json"], None, "[1]\n", "jq: error: Is a directory\n", 2),
        (&["-c", ".", "dir", "sp.json"], None, "1\n", "jq: error: Is a directory\n", 2),
        (&["-c", ".", "n.json"], None, "", "jq: parse error: Unmatched '}' at line 1, column 2\n", 5),
    ])
}
