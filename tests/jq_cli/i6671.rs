//! #6671: a `-n -s` program over a text with one fault, at its end, reaches a directory given last
//! on the read after the one that raised the fault, as with several faults (#6625, #6656): the
//! slurp hands out the fault and then the array, and the array's read says `Is a directory`. Every
//! row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// One row: the flags and program, the files, then jq's stdout, stderr and exit status.
type Row<'a> = (&'a [&'a str], &'a [&'a str], &'a str, &'a str, i32);

#[test]
fn a_slurp_with_one_fault_at_its_end_says_a_directory_last_on_the_array_read_6671() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    std::fs::create_dir(dir.path().join("dir2"))?;
    for (name, text) in [
        ("t1.json", "[1,"),
        ("abc.json", "\"abc"),
        ("nul.json", "nul"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    #[rustfmt::skip]
    let rows: &[Row<'_>] = &[
        (&["-c", "-n", "-s", "try input catch ."][..], &["t1", "dir"][..], "\"Unfinished JSON term at EOF at line 1, column 3\"\n", "", 2),
        (&["-c", "-n", "-s", "try input catch ."][..], &["t1", "dir", "dir2"][..], "\"Unfinished JSON term at EOF at line 1, column 3\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "try input catch ."][..], &["t1", "miss", "dir"][..], "\"Unfinished JSON term at EOF at line 1, column 3\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "try input catch ."][..], &["t1", "miss"][..], "\"Unfinished JSON term at EOF at line 1, column 3\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["t1", "dir"][..], "\"Unfinished JSON term at EOF at line 1, column 3\"\n[]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["t1", "dir", "dir2"][..], "\"Unfinished JSON term at EOF at line 1, column 3\"\n[]\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["t1", "miss", "dir"][..], "\"Unfinished JSON term at EOF at line 1, column 3\"\n[]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["t1", "miss"][..], "\"Unfinished JSON term at EOF at line 1, column 3\"\n[]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["abc", "dir"][..], "\"Unfinished string at EOF at line 1, column 4\"\n[]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["abc", "dir", "dir2"][..], "\"Unfinished string at EOF at line 1, column 4\"\n[]\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["abc", "miss", "dir"][..], "\"Unfinished string at EOF at line 1, column 4\"\n[]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["abc", "miss"][..], "\"Unfinished string at EOF at line 1, column 4\"\n[]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["nul", "dir"][..], "\"Invalid literal at EOF at line 1, column 3\"\n[]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["nul", "dir", "dir2"][..], "\"Invalid literal at EOF at line 1, column 3\"\n[]\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["nul", "miss", "dir"][..], "\"Invalid literal at EOF at line 1, column 3\"\n[]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["nul", "miss"][..], "\"Invalid literal at EOF at line 1, column 3\"\n[]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .)"][..], &["miss", "t1", "dir"][..], "\"Unfinished JSON term at EOF at line 1, column 3\"\n[]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .), (try input catch .)"][..], &["t1", "dir"][..], "\"Unfinished JSON term at EOF at line 1, column 3\"\n[]\n\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .), (try input catch .)"][..], &["t1", "dir", "dir2"][..], "\"Unfinished JSON term at EOF at line 1, column 3\"\n[]\n\"break\"\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .), (try input catch .)"][..], &["t1", "miss", "dir"][..], "\"Unfinished JSON term at EOF at line 1, column 3\"\n[]\n\"break\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .), (try input catch .), (try input catch .)"][..], &["t1", "miss"][..], "\"Unfinished JSON term at EOF at line 1, column 3\"\n[]\n\"break\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
    ];
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
        let want_err = stderr.replace("@DIR@", &d);
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (stdout, want_err.as_str(), status),
            "{flags:?} {names:?}"
        );
    }
    Ok(())
}
