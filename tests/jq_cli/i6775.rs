//! #6775: under `-n -s --stream-errors` a token one file leaves open (`ptr` holds `[1,`, `pa` holds
//! `[1]`: the text `[1,[1]`) ends the text with a fault event, and the slurp's array is read first.
//! jq's reader reads that event only by going on past the last file: for a directory given last it
//! has been through the directory then, so the read that returns the event says `Is a directory`,
//! where a program that stops after the array never makes the read (`ptr pa dir` with one `try
//! input` is silent). succinctly read the event as a value of the first file and said it a read
//! late, so a program that read the event and nothing after it never printed it. The event is also
//! the read that places `input_filename` at a last file that did not open (`miss`), as it already did
//! outside `-s`. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// One row: the flags and program, the files under the fixture directory, and jq 1.7.1's stdout,
/// stderr and exit status, with `@DIR@` for the directory.
type Row<'a> = (&'a [&'a str], &'a [&'a str], &'a str, &'a str, i32);

fn check_rows(rows: &[Row<'_>]) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    for (name, text) in [("ptr", "[1,"), ("pa", "[1]"), ("empty", "")] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for &(flags, names, stdout, stderr, status) in rows {
        let files: Vec<String> = names.iter().map(|name| format!("{d}/{name}")).collect();
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
fn the_fault_event_a_slurp_reads_at_the_end_goes_through_the_last_directory_6775() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-n", "-s", "--stream-errors", "[., (try input catch .), (try input catch .)]"][..], &["ptr", "pa", "dir"][..], "[null,[[[0],1],[[1,0],1],[[1,0]]],[\"Unfinished JSON term at EOF at line 1, column 6\",[1]]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--stream-errors", "[., (try input catch .), (try input catch .), (try input catch .)]"][..], &["ptr", "pa", "dir"][..], "[null,[[[0],1],[[1,0],1],[[1,0]]],[\"Unfinished JSON term at EOF at line 1, column 6\",[1]],\"break\"]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--stream-errors", "[., (try input catch .)]"][..], &["ptr", "pa", "dir"][..], "[null,[[[0],1],[[1,0],1],[[1,0]]]]\n", "", 2),
        (&["-c", "-n", "-s", "--stream-errors", "[., (try input catch .), (try input catch .), input_filename]"][..], &["ptr", "pa", "dir"][..], "[null,[[[0],1],[[1,0],1],[[1,0]]],[\"Unfinished JSON term at EOF at line 1, column 6\",[1]],null]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--stream-errors", "[., (try input catch .), (try input catch .), input_line_number]"][..], &["ptr", "pa", "dir"][..], "[null,[[[0],1],[[1,0],1],[[1,0]]],[\"Unfinished JSON term at EOF at line 1, column 6\",[1]],0]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--stream-errors", "[(try input catch .), (try input catch .), input_filename, input_line_number]"][..], &["ptr", "pa", "dir"][..], "[[[[0],1],[[1,0],1],[[1,0]]],[\"Unfinished JSON term at EOF at line 1, column 6\",[1]],null,0]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--stream-errors", "[., (try input catch .), (try input catch .)]"][..], &["ptr", "pa", "pa", "dir"][..], "[null,[[[0],1],[[1,0],1],[[1,0]],[[1,0],1],[[1,0]]],[\"Unfinished JSON term at EOF at line 1, column 9\",[1]]]\n", "jq: error: Is a directory\n", 2),
    ];
    check_rows(&rows)
}

#[test]
fn a_fault_event_at_a_missing_last_file_names_it_for_input_filename_6775() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-n", "-s", "--stream-errors", "[., (try input catch .), (try input catch .), input_filename]"][..], &["ptr", "pa", "miss"][..], "[null,[[[0],1],[[1,0],1],[[1,0]]],[\"Unfinished JSON term at EOF at line 1, column 6\",[1]],\"@DIR@/miss\"]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream-errors", "[(try input catch .), (try input catch .), input_filename, input_line_number]"][..], &["ptr", "empty", "pa", "miss"][..], "[[[[0],1],[[1,0],1],[[1,0]]],[\"Unfinished JSON term at EOF at line 1, column 6\",[1]],\"@DIR@/miss\",0]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream-errors", "[., (try input catch .), (try input catch .)]"][..], &["ptr", "pa", "miss"][..], "[null,[[[0],1],[[1,0],1],[[1,0]]],[\"Unfinished JSON term at EOF at line 1, column 6\",[1]]]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
    ];
    check_rows(&rows)
}

#[test]
fn the_failed_files_around_it_and_the_other_slurps_do_not_move_6775() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-n", "-s", "--stream-errors", "[., (try input catch .), (try input catch .)]"][..], &["pa", "dir"][..], "[null,[[[0],1],[[0]]],\"break\"]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--stream-errors", "[., (try input catch .), (try input catch .)]"][..], &["ptr", "dir"][..], "[null,[[[0],1],[\"Unfinished JSON term at EOF at line 1, column 3\",[1]]],\"break\"]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--stream-errors", "[., (try input catch .), (try input catch .)]"][..], &["ptr", "pa", "dir", "miss"][..], "[null,[[[0],1],[[1,0],1],[[1,0]],[\"Unfinished JSON term at EOF at line 1, column 6\",[1]]],\"break\"]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream-errors", "[., (try input catch .), (try input catch .)]"][..], &["dir", "ptr", "pa"][..], "[null,[[[0],1],[[1,0],1]],[[1,0]]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--stream-errors", "[., (try input catch .), (try input catch .)]"][..], &["ptr", "pa", "dir", "dir"][..], "[null,[[[0],1],[[1,0],1],[[1,0]],[\"Unfinished JSON term at EOF at line 1, column 6\",[1]]],\"break\"]\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "-s", "--stream-errors", "."][..], &["ptr", "pa", "dir"][..], "[[[0],1],[[1,0],1],[[1,0]]]\n", "", 2),
        (&["-c", "-n", "-s", "--stream", "[., (try input catch .), (try input catch .)]"][..], &["ptr", "pa", "dir"][..], "[null,[[[0],1],[[1,0],1],[[1,0]]],\"Unfinished JSON term at EOF at line 1, column 6\"]\n", "jq: error: Is a directory\n", 2),
    ];
    check_rows(&rows)
}
