//! #6480 (its plan's step 5): a fault at the end of the text, when the last file named did not
//! open. `[1,` waits for more, so jq's parser asks its reader past the last file before it gives
//! up, and `input` raises `Unfinished JSON term at EOF` at the missing file, line 0, which names
//! it in the `(at ...)` marker, under `-n`, `-n -s`, or for an `input` the program reads itself.
//! A fault earlier in the text, or a missing file before the last
//! that opened, keeps the fault's own file, and jq's main loop prints the fault with no position.
//! Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_fault_at_the_end_of_the_text_is_raised_at_a_missing_last_file_6480() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("t1", "[1,"),
        ("small", "1 5["),
        ("nl", "[1,\n"),
        ("okt", "1\n2\n[1,"),
        ("ustr", "\"abc"),
        ("empty", ""),
        ("key", "{\"a\":"),
        ("bad", "[1,}\n2"),
        ("ok", "1\n2\n"),
        ("b", "1\n[1,"),
    ] {
        std::fs::write(dir.path().join(format!("{name}.json")), text)?;
    }
    // More than one chunk of the lazy route's scan, and deeper than it reads.
    std::fs::write(dir.path().join("many.json"), "1\n".repeat(100_000) + "[1,")?;
    std::fs::write(
        dir.path().join("deep.json"),
        "[".repeat(120) + &"]".repeat(120) + " [1,",
    )?;
    for (flags, program, names, stdout, stderr, status) in [
        (&["-n"][..], "[inputs] | length", &["t1", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&["-n"][..], "[inputs] | length", &["small", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-n"][..], "[inputs] | length", &["nl", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished JSON term at EOF at line 2, column 0\n", 2),
        (&["-n"][..], "[inputs] | length", &["okt", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished JSON term at EOF at line 3, column 3\n", 2),
        (&["-n"][..], "[inputs] | length", &["ustr", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished string at EOF at line 1, column 4\n", 2),
        (&["-n"][..], "[inputs] | length", &["key", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished JSON term at EOF at line 1, column 5\n", 2),
        (&["-n"][..], "[inputs] | length", &["t1", "m1", "m2"][..], "", "jq: error: Could not open file @DIR@/m1.json: No such file or directory\njq: error: Could not open file @DIR@/m2.json: No such file or directory\njq: error (at @DIR@/m2.json:0): Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&["-n"][..], "[inputs] | length", &["t1", "empty", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&["-n"][..], "input, input", &["t1", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&["-n"][..], "input, input, input", &["small", "gone"][..], "1\n5\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-n"][..], "[inputs | [., input_filename]]", &["okt", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished JSON term at EOF at line 3, column 3\n", 2),
        (&["-n"][..], "try input catch ., input_filename, input_line_number", &["t1", "gone"][..], "\"Unfinished JSON term at EOF at line 1, column 3\"\n\"@DIR@/gone.json\"\n0\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-n"][..], "first(inputs)", &["okt", "gone"][..], "1\n", "", 0),
        (&["-n"][..], "[inputs] | length", &["many", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished JSON term at EOF at line 100001, column 3\n", 2),
        (&["-n"][..], "[inputs] | length", &["deep", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished JSON term at EOF at line 1, column 244\n", 2),
        (&["-n"][..], "[inputs] | length", &["t1", "gone", "ok"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/ok.json:2): Expected separator between values at line 3, column 0\n", 2),
        (&["-n"][..], "[inputs] | length", &["bad", "gone"][..], "", "jq: error (at @DIR@/bad.json:1): Unmatched '}' at line 1, column 4\n", 5),
        (&["-n"][..], "[inputs] | length", &["t1"][..], "", "jq: error (at @DIR@/t1.json:0): Unfinished JSON term at EOF at line 1, column 3\n", 5),
        (&["-n", "-s"][..], "input", &["t1", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&["-n", "-s"][..], "try input catch ., input", &["t1", "gone"][..], "\"Unfinished JSON term at EOF at line 1, column 3\"\n[]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], "[., input]", &["b", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished JSON term at EOF at line 2, column 3\n", 2),
        (&[][..], "try input catch input_filename", &["b", "gone"][..], "\"@DIR@/gone.json\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], ".", &["t1", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&[][..], "input_line_number as $l | .", &["okt", "gone"][..], "1\n2\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 3, column 3\n", 2),

    ] {
        let paths: Vec<String> = names.iter().map(|name| format!("{d}/{name}.json")).collect();
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
