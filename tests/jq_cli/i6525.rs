//! #6525: a run whose program reads `input` reports a file that did not open when jq's reader
//! reaches it, not before the first value. jq's main loop reads the first file before the program
//! does and stops one value past the first failure, so a file after that is never opened and never
//! reported, whatever the program goes on to read. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn an_input_reading_program_reports_an_unopened_file_when_the_reader_reaches_it_6525() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("nums.json", "1\n2\n3\n"),
        ("ok.json", "[1,2]\n"),
        ("empty.json", ""),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for (flags, names, stdout, stderr, status) in [
        (&["-c", "[., input_filename, input_line_number]"][..], &["gone", "nums", "gone2"][..], "[1,\"@DIR@/nums.json\",1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "try input catch ."][..], &["gone", "nums", "gone2"][..], "2\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "first(inputs), input_filename"][..], &["gone", "nums", "gone2"][..], "2\n\"@DIR@/nums.json\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "."][..], &["gone", "nums", "gone2"][..], "1\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "[., input]"][..], &["gone", "nums", "gone2"][..], "[1,2]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "[inputs]"][..], &["gone", "nums", "gone2"][..], "[2,3]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["-c", "[., input]"][..], &["nums", "gone", "ok", "gone2"][..], "[1,2]\n[3,[1,2]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "[., input]"][..], &["ok", "gone", "nums"][..], "[[1,2],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "[., input_filename]"][..], &["ok", "gone", "nums", "gone2"][..], "[[1,2],\"@DIR@/ok.json\"]\n[1,\"@DIR@/nums.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "input"][..], &["ok", "gone", "nums", "gone2"][..], "1\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "[inputs]"][..], &["ok", "gone", "nums", "gone2"][..], "[1,2,3]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["-c", "first(inputs)"][..], &["ok", "gone", "gone2"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["-c", "[., input]"][..], &["empty", "gone", "nums", "gone2"][..], "[1,2]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "[., input]"][..], &["gone", "empty", "nums", "gone2"][..], "[1,2]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "[., input_line_number]"][..], &["ok", "gone"][..], "[[1,2],1]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "input"][..], &["gone", "gone2", "ok"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\njq: error (at @DIR@/ok.json:1): break\n", 2),
        (&["-c", "-n", "input"][..], &["ok", "gone", "gone2"][..], "[1,2]\n", "", 0),
        (&["-c", "-n", "[inputs]"][..], &["nums", "gone", "ok", "gone2"][..], "[1,2,3,[1,2]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["-c", "-n", "first(inputs)"][..], &["gone", "nums", "gone2"][..], "1\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "input, input"][..], &["ok", "gone", "nums", "gone2"][..], "[1,2]\n1\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
    ] {
        let files: Vec<String> = names.iter().map(|name| format!("{d}/{name}.json")).collect();
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
