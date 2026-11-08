//! #6625: a directory given last is still reported by the reads (#6615) when another failure came
//! before it, and when `-n` reads a text that faults before it.
//!
//! An earlier failure (a file that will not open, or a directory that is not last) sends the run up
//! front, where every failure is said at once and a last directory is said not at all; two
//! directories at the end said once. A `-n` program reads on past a fault it catches, so a failure
//! after the fault is the later reads' to reach, and a slurp's fault is the first of two reads
//! (the second is the fault at the end of the text). Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// One row: the flags, the files, then jq's stdout, stderr and exit status.
type Row<'a> = (&'a [&'a str], &'a [&'a str], &'a str, &'a str, i32);

/// Creates the files the rows name (`miss` and `miss2` stay absent) and runs each row.
fn check(rows: &[Row<'_>]) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    std::fs::create_dir(dir.path().join("dir2"))?;
    for (name, text) in [
        ("one.json", "1\n"),
        ("bare.json", "1"),
        ("lines.json", "a\nb"),
        ("linesn.json", "a\nb\n"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for &(flags, names, stdout, stderr, status) in rows {
        let files: Vec<String> = names
            .iter()
            .map(|name| match *name {
                "dir" | "dir2" => format!("{d}/{name}"),
                _ => format!("{d}/{name}.json"),
            })
            .collect();
        let mut args: Vec<&str> = flags.to_vec();
        args.extend(files.iter().map(String::as_str));
        let (out, err, code) = run_jq_full(&args, None)?;
        let (want_out, want_err) = (stdout.replace("@DIR@", &d), stderr.replace("@DIR@", &d));
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (want_out.as_str(), want_err.as_str(), status),
            "{flags:?} {names:?}"
        );
    }
    Ok(())
}

#[test]
fn a_directory_last_is_reported_after_other_failures_6625() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-n", "[inputs]"][..], &["bare", "miss", "dir"][..], "[1]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "[inputs]"][..], &["one", "miss", "dir"][..], "[1]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "[inputs]"][..], &["bare", "dir", "dir2"][..], "[1]\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "[inputs]"][..], &["bare", "miss", "dir", "dir2"][..], "[1]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "[inputs]"][..], &["bare", "dir", "miss", "dir2"][..], "[1]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "[inputs]"][..], &["bare", "miss", "miss2", "dir"][..], "[1]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "first(inputs)"][..], &["bare", "dir", "dir2"][..], "1\n", "jq: error: Is a directory\n", 2),
        (&["-c", "input"][..], &["bare", "dir", "dir2"][..], "", "jq: error: Is a directory\njq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "input"][..], &["bare", "miss", "dir"][..], "", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "[., input]"][..], &["bare", "miss", "dir"][..], "", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "[inputs]"][..], &["bare", "dir", "dir2"][..], "[]\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "1"][..], &["bare", "miss", "dir"][..], "1\n", "", 0),
        (&["-c", "-n", "1"][..], &["bare", "dir", "dir2"][..], "1\n", "", 0),
    ];
    check(&rows)
}

#[test]
fn a_directory_last_is_reported_after_a_fault_a_program_reads_past_6625() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-n", "(try input catch .), (try input catch .), (try input catch .)"][..], &["bare", "miss", "dir"][..], "1\n\"break\"\n\"break\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "(try input catch .), (try input catch .), (try input catch .)"][..], &["lines", "dir", "dir2"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n\"break\"\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "(try input catch .), (try input catch .), (try input catch .)"][..], &["lines", "miss", "dir"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n\"break\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "(try input catch .), (try input catch .), (try input catch .)"][..], &["linesn", "dir", "dir2"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at line 3, column 0\"\n\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "input"][..], &["bare", "miss", "dir"][..], "1\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .), (try input catch .)"][..], &["lines", "dir"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n[]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .), (try input catch .)"][..], &["lines", "miss", "dir"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n[]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .), (try input catch .)"][..], &["lines", "dir", "dir2"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n[]\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["lines", "dir"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n", "", 2),
        (&["-c", "-n", "-s", "try input catch ."][..], &["lines", "dir"][..], "\"Invalid numeric literal at line 2, column 0\"\n", "", 0),
        (&["-c", "-n", "-s", "input"][..], &["lines", "dir"][..], "", "jq: error (at @DIR@/lines.json:1): Invalid numeric literal at line 2, column 0\n", 5),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .), (try input catch .)"][..], &["linesn", "dir"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at line 3, column 0\"\n[]\n", "", 2),
        (&["-c", "-n", "-R", "[inputs]"][..], &["bare", "miss", "dir"][..], "[\"1\"]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-R", "input"][..], &["lines", "miss", "dir"][..], "\"b\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
    ];
    check(&rows)
}
