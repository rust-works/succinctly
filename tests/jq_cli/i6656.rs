//! #6656: a `-n` program that catches a text's parse errors and goes on reading reaches the input
//! files that would not open after the fault, as jq's reader does: each read is told whether its
//! fault is at the end of the text, and the read that finds the end says what it reached. Without
//! `try` the program stops at the fault and never does. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// One row: the flags and program, the files, then jq's stdout, stderr and exit status.
type Row<'a> = (&'a [&'a str], &'a [&'a str], &'a str, &'a str, i32);

#[test]
fn a_file_that_will_not_open_after_a_fault_is_reached_by_the_reads_that_follow_it_6656(
) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    for (name, text) in [
        ("one.json", "1\n"),
        ("lines.json", "a\nb"),
        ("linesn.json", "a\nb\n"),
        ("empty.json", ""),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    #[rustfmt::skip]
    let rows: &[Row<'_>] = &[
        (&["-c", "-n", "try input catch ."][..], &["lines", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n", "", 0),
        (&["-c", "-n", "try input catch ."][..], &["linesn", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n", "", 0),
        (&["-c", "-n", "try input catch ."][..], &["lines", "miss", "miss2"][..], "\"Invalid numeric literal at line 2, column 0\"\n", "", 0),
        (&["-c", "-n", "try input catch ."][..], &["lines", "dir", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n", "", 0),
        (&["-c", "-n", "try input catch ."][..], &["empty", "lines", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n", "", 0),
        (&["-c", "-n", "try input catch ."][..], &["lines", "empty", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n", "", 0),
        (&["-c", "-n", "try input catch ."][..], &["one", "lines", "miss"][..], "1\n", "", 0),
        (&["-c", "-n", "(try input catch .), (try input catch .)"][..], &["lines", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "(try input catch .), (try input catch .)"][..], &["linesn", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at line 3, column 0\"\n", "", 0),
        (&["-c", "-n", "(try input catch .), (try input catch .)"][..], &["lines", "miss", "miss2"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\n", 2),
        (&["-c", "-n", "(try input catch .), (try input catch .)"][..], &["lines", "dir", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "(try input catch .), (try input catch .)"][..], &["empty", "lines", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "(try input catch .), (try input catch .)"][..], &["lines", "empty", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "(try input catch .), (try input catch .)"][..], &["one", "lines", "miss"][..], "1\n\"Invalid numeric literal at line 3, column 0\"\n", "", 0),
        (&["-c", "-n", "(try input catch .), (try input catch .), (try input catch .)"][..], &["lines", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n\"break\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "(try input catch .), (try input catch .), (try input catch .)"][..], &["linesn", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at line 3, column 0\"\n\"break\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "(try input catch .), (try input catch .), (try input catch .)"][..], &["lines", "miss", "miss2"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n\"break\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\n", 2),
        (&["-c", "-n", "(try input catch .), (try input catch .), (try input catch .)"][..], &["lines", "dir", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n\"break\"\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "(try input catch .), (try input catch .), (try input catch .)"][..], &["empty", "lines", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n\"break\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "(try input catch .), (try input catch .), (try input catch .)"][..], &["lines", "empty", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n\"break\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "try input catch ."][..], &["lines", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n", "", 0),
        (&["-c", "-n", "-s", "try input catch ."][..], &["linesn", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n", "", 0),
        (&["-c", "-n", "-s", "try input catch ."][..], &["lines", "miss", "miss2"][..], "\"Invalid numeric literal at line 2, column 0\"\n", "", 0),
        (&["-c", "-n", "-s", "try input catch ."][..], &["lines", "dir", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n", "", 0),
        (&["-c", "-n", "-s", "try input catch ."][..], &["empty", "lines", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n", "", 0),
        (&["-c", "-n", "-s", "try input catch ."][..], &["lines", "empty", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n", "", 0),
        (&["-c", "-n", "-s", "try input catch ."][..], &["one", "lines", "miss"][..], "\"Invalid numeric literal at line 3, column 0\"\n", "", 0),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["lines", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["linesn", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at line 3, column 0\"\n", "", 0),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["lines", "miss", "miss2"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["lines", "dir", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["empty", "lines", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["lines", "empty", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["one", "lines", "miss"][..], "\"Invalid numeric literal at line 3, column 0\"\n\"Invalid numeric literal at EOF at line 3, column 1\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .), (try input catch .)"][..], &["lines", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n[]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .), (try input catch .)"][..], &["linesn", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at line 3, column 0\"\n[]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .), (try input catch .)"][..], &["lines", "miss", "miss2"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n[]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .), (try input catch .)"][..], &["lines", "dir", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n[]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .), (try input catch .)"][..], &["empty", "lines", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n[]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .), (try input catch .)"][..], &["lines", "empty", "miss"][..], "\"Invalid numeric literal at line 2, column 0\"\n\"Invalid numeric literal at EOF at line 2, column 1\"\n[]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .), (try input catch .)"][..], &["one", "lines", "miss"][..], "\"Invalid numeric literal at line 3, column 0\"\n\"Invalid numeric literal at EOF at line 3, column 1\"\n[1]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
    ];
    for &(flags, names, stdout, stderr, status) in rows {
        let files: Vec<String> = names
            .iter()
            .map(|name| match *name {
                "dir" => format!("{d}/dir"),
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
