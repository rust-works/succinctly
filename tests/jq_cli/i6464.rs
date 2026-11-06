//! #6464: under `-n`, only `input`/`inputs` make jq open an input file. `input_line_number` alone
//! reads the position of whatever was read, so a missing file is never reached and never reported.
//! Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn null_input_line_number_does_not_open_the_files_6464() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let gone = dir.path().join("gone.json").to_string_lossy().into_owned();
    let data = dir.path().join("data.json").to_string_lossy().into_owned();
    std::fs::write(&data, "1 2\n")?;
    for (flags, filter, files, expected) in [
        (&["-nc"][..], "input_line_number", &[&gone][..], "0\n"),
        (&["-nc"], "[input_line_number, input_filename]", &[&gone], "[0,null]\n"),
        (&["-nc"], "input_line_number", &[&gone, &data], "0\n"),
        (&["-nce"], "input_line_number", &[&gone], "0\n"),
        (&["-nc", "--seq"], "input_line_number", &[&gone], "\u{1e}0\n"),
        // `input` still opens the file, and the line counts what it read.
        (&["-nc"], "[input_line_number, input, input_line_number]", &[&data], "[0,1,1]\n"),
    ] {
        let mut args: Vec<&str> = flags.to_vec();
        args.push(filter);
        args.extend(files.iter().map(|f| f.as_str()));
        let (out, err, code) = run_jq_full(&args, None)?;
        assert_eq!((out.as_str(), err.as_str(), code), (expected, "", 0), "{flags:?} {filter}");
    }
    // An `input` that reaches the missing file reports it and exits 2.
    let (out, err, code) = run_jq_full(&["-nc", "input", &gone], None)?;
    assert_eq!((out.as_str(), code), ("", 2), "stderr: {err:?}");
    assert!(err.starts_with(&format!(
        "jq: error: Could not open file {gone}: No such file or directory\n"
    )));
    Ok(())
}

/// jq's main loop reads no further input once its reader has read from a file past one that did
/// not open, and the value that took it there is still processed: `-R` stops after the first line
/// of the next file, as a JSON document stream does (#5472). Rows from jq 1.7.1.
#[test]
fn raw_input_stops_after_the_first_line_past_a_missing_file_6464() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
    let (gone, lines) = (path("gone.txt"), path("lines.txt"));
    std::fs::write(&lines, "line1\nline2\n\nline4")?;
    for (files, expected) in [
        (vec![&gone, &lines], "\"line1\"\n"),
        // The last line has no newline, so it joins the first line of the file after the gap.
        (
            vec![&lines, &gone, &lines],
            "\"line1\"\n\"line2\"\n\"\"\n\"line4line1\"\n",
        ),
        // Nothing past the gap: every line before it is read.
        (
            vec![&lines, &gone],
            "\"line1\"\n\"line2\"\n\"\"\n\"line4\"\n",
        ),
    ] {
        let mut args = vec!["-R", "."];
        args.extend(files.iter().map(|f| f.as_str()));
        let (out, err, code) = run_jq_full(&args, None)?;
        assert_eq!((out.as_str(), code), (expected, 2), "{files:?} stderr: {err:?}");
        assert!(err.starts_with("jq: error: Could not open file "), "{err:?}");
    }
    Ok(())
}
