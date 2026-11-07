//! #6480 (its plan's step 4): a last value that completes only at the end of the input, when the
//! last file named did not open. A bare number or literal in the last bytes (`12`, the `2` of
//! `1\n2`) or a `-R` line with no newline is complete only once jq's parser asks for more, which
//! sends its reader past the last file: the value is read at the missing file, line 0, and the open
//! error is said at that read, after the values before it. A string or container ends on its own
//! closing byte and a value before white space at its next byte, so both keep their file. Every
//! row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_last_value_that_completes_at_the_end_is_read_at_a_missing_last_file_6480() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("num.json", "12"),
        ("nonl.json", "1\n2"),
        ("lit.json", "true"),
        ("ok.json", "1\n2\n"),
        ("str.json", "\"s\""),
        ("sp.json", "12 "),
        ("mix.json", "[1] 2"),
        ("empty.json", ""),
        ("ab.txt", "a\nb"),
        ("three.json", "1\n2\n3"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    // Deeper than the lazy route reads, which hands the stream to the eager queue.
    std::fs::write(
        dir.path().join("deep.json"),
        "[".repeat(120) + &"]".repeat(120) + " 12",
    )?;
    // Text that is not UTF-8.
    std::fs::write(dir.path().join("iu.json"), b"\"\xff\" 12")?;
    std::fs::write(dir.path().join("iutxt.txt"), b"a\xff\nb")?;
    // More than one chunk of the lazy route's scan, ending in a bare number.
    std::fs::write(dir.path().join("many.json"), "1\n".repeat(100_000) + "7")?;
    for (flags, program, names, stdout, stderr, status) in [
        (&[][..], "[., input_filename, input_line_number]", &["num", "gone"][..], "[12,\"@DIR@/gone.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], "[., input_filename, input_line_number]", &["nonl", "gone"][..], "[1,\"@DIR@/nonl.json\",1]\n[2,\"@DIR@/gone.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], "[., input_filename, input_line_number]", &["lit", "gone"][..], "[true,\"@DIR@/gone.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], "[., input_filename, input_line_number]", &["mix", "gone"][..], "[[1],\"@DIR@/mix.json\",0]\n[2,\"@DIR@/gone.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], "[., input_filename, input_line_number]", &["ok", "num", "gone"][..], "[1,\"@DIR@/ok.json\",1]\n[2,\"@DIR@/ok.json\",2]\n[12,\"@DIR@/gone.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], "[., input_filename, input_line_number]", &["num", "empty", "gone"][..], "[12,\"@DIR@/gone.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], "[., input_filename, input_line_number]", &["num", "m1", "m2"][..], "[12,\"@DIR@/m2.json\",0]\n", "jq: error: Could not open file @DIR@/m1.json: No such file or directory\njq: error: Could not open file @DIR@/m2.json: No such file or directory\n", 2),
        (&[][..], "[., input_filename, input_line_number]", &["str", "gone"][..], "[\"s\",\"@DIR@/str.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], "[., input_filename, input_line_number]", &["sp", "gone"][..], "[12,\"@DIR@/sp.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], "[., input_filename]", &["num", "gone"][..], "[12,\"@DIR@/gone.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], "[., input_filename]", &["nonl", "gone"][..], "[1,\"@DIR@/nonl.json\"]\n[2,\"@DIR@/gone.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], "error", &["num", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0) (not a string): 12\n", 2),
        (&[][..], "error", &["nonl", "gone"][..], "", "jq: error (at @DIR@/nonl.json:1) (not a string): 1\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0) (not a string): 2\n", 2),
        (&[][..], "error", &["ok", "num", "gone"][..], "", "jq: error (at @DIR@/ok.json:1) (not a string): 1\njq: error (at @DIR@/ok.json:2) (not a string): 2\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0) (not a string): 12\n", 2),
        (&[][..], "error", &["num", "empty", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0) (not a string): 12\n", 2),
        (&[][..], "error", &["str", "gone"][..], "", "jq: error (at @DIR@/str.json:0): s\njq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], "input_filename", &["nonl", "gone"][..], "\"@DIR@/nonl.json\"\n\"@DIR@/gone.json\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], "[., input_filename, input_line_number] | select(.[0] == 7)", &["many", "gone"][..], "[7,\"@DIR@/gone.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-n"][..], "input | [., input_filename, input_line_number]", &["num", "gone"][..], "[12,\"@DIR@/gone.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-n"][..], "input | [., input_filename, input_line_number]", &["nonl", "gone"][..], "[1,\"@DIR@/nonl.json\",1]\n", "", 0),
        (&["-n"][..], "input | [., input_filename, input_line_number]", &["ok", "num", "gone"][..], "[1,\"@DIR@/ok.json\",1]\n", "", 0),
        (&["-n"][..], "[inputs | [., input_filename, input_line_number]]", &["nonl", "gone"][..], "[[1,\"@DIR@/nonl.json\",1],[2,\"@DIR@/gone.json\",0]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-n"][..], "[inputs | [., input_filename, input_line_number]]", &["ok", "num", "gone"][..], "[[1,\"@DIR@/ok.json\",1],[2,\"@DIR@/ok.json\",2],[12,\"@DIR@/gone.json\",0]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-n"][..], "[inputs | [., input_filename, input_line_number]] | last", &["many", "gone"][..], "[7,\"@DIR@/gone.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-n"][..], "input, (input | error)", &["nonl", "gone"][..], "1\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0) (not a string): 2\n", 2),
        (&["-n"][..], "first(inputs)", &["nonl", "gone"][..], "1\n", "", 0),
        (&["-n"][..], "first(inputs)", &["many", "gone"][..], "1\n", "", 0),
        (&["-R"][..], "[., input_filename, input_line_number]", &["ab", "gone"][..], "[\"a\",\"@DIR@/ab.txt\",1]\n[\"b\",\"@DIR@/gone.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-R"][..], "[., input_filename]", &["ab", "gone"][..], "[\"a\",\"@DIR@/ab.txt\"]\n[\"b\",\"@DIR@/gone.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-R", "-n"][..], "[inputs | [., input_filename, input_line_number]]", &["ab", "gone"][..], "[[\"a\",\"@DIR@/ab.txt\",1],[\"b\",\"@DIR@/gone.json\",0]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-R", "-n"][..], "input", &["ab", "gone"][..], "\"a\"\n", "", 0),
        (&[][..], "error", &["gone", "three", "gone2"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/three.json:1) (not a string): 1\n", 2),
        (&[][..], ".", &["gone", "three", "gone2"][..], "1\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], "[., input_filename]", &["gone", "three", "gone2"][..], "[1,\"@DIR@/three.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], "[., input_filename]", &["gone", "num", "gone2"][..], "[12,\"@DIR@/gone2.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&[][..], "error", &["gone", "num", "gone2"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\njq: error (at @DIR@/gone2.json:0) (not a string): 12\n", 2),
        (&["-n"][..], "[inputs | [input_filename, input_line_number]]", &["deep", "gone"][..], "[[\"@DIR@/deep.json\",0],[\"@DIR@/gone.json\",0]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], "input_line_number as $l | [input_filename, $l]", &["deep", "gone"][..], "[\"@DIR@/deep.json\",0]\n[\"@DIR@/gone.json\",0]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-n"][..], "[inputs | [., input_filename, input_line_number]]", &["iu", "gone"][..], "[[\"�\",\"@DIR@/iu.json\",0],[12,\"@DIR@/gone.json\",0]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], "input_line_number as $l | [., input_filename]", &["iu", "gone"][..], "[\"�\",\"@DIR@/iu.json\"]\n[12,\"@DIR@/gone.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-R"][..], "[., input_filename]", &["iutxt", "gone"][..], "[\"a�\",\"@DIR@/iutxt.txt\"]\n[\"b\",\"@DIR@/gone.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-R", "-n"][..], "[inputs | [., input_filename]]", &["iutxt", "gone"][..], "[[\"a�\",\"@DIR@/iutxt.txt\"],[\"b\",\"@DIR@/gone.json\"]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),

    ] {
        let paths: Vec<String> = names
            .iter()
            .map(|&name| match name {
                "ab" | "iutxt" => format!("{d}/{name}.txt"),
                name => format!("{d}/{name}.json"),
            })
            .collect();
        let mut args = vec!["-c"];
        args.extend_from_slice(flags);
        args.push(program);
        args.extend(paths.iter().map(String::as_str));
        let (out, err, code) = run_jq_full(&args, None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (
                stdout.replace("@DIR@", &d).as_str(),
                stderr.replace("@DIR@", &d).as_str(),
                status
            ),
            "{flags:?} {program} {names:?}"
        );
    }
    Ok(())
}
