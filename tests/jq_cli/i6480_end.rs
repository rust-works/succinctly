//! #6480 (its plan's step 3): when the last file named did not open, jq's reader ends at that file.
//! A read that reaches the end of the input leaves jq's position there, at line 0, so
//! `input_filename`, `input_line_number` and the `(at ...)` marker of an error after it name the
//! missing file, under `-s` too. Here a file that did not open had no source, so they named the last
//! file that did, `null` or `<stdin>`, and `-s` said `<unknown>`. Only the last name matters: an
//! empty file after the missing one, or a directory last, is where the reader ends. Every row was
//! captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn the_end_of_the_input_is_at_a_last_file_that_did_not_open_6480() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("ok.json", "1\n2\n"),
        ("num.json", "12"),
        ("str.json", "\"s\""),
        ("empty.json", ""),
        ("ab.txt", "a\nb"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    std::fs::create_dir(dir.path().join("adir"))?;
    for (flags, program, names, stdout, stderr, status) in [
        (&["-n"][..], "[inputs], input_filename", &["ok", "gone"][..], "[1,2]\n\"@DIR@/gone.json\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-n"][..], "[inputs], input_filename", &["str", "gone"][..], "[\"s\"]\n\"@DIR@/gone.json\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-n"][..], "[inputs], input_filename", &["gone"][..], "[]\n\"@DIR@/gone.json\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-n"][..], "[inputs] | input_line_number", &["ok", "gone"][..], "0\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-n"][..], "input, input", &["num", "gone"][..], "12\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): break\n", 2),
        (&["-n"][..], "[inputs], input_filename", &["ok", "m1", "m2"][..], "[1,2]\n\"@DIR@/m2.json\"\n", "jq: error: Could not open file @DIR@/m1.json: No such file or directory\njq: error: Could not open file @DIR@/m2.json: No such file or directory\n", 2),
        (&["-n"][..], "[inputs], input_filename", &["ok", "empty", "gone"][..], "[1,2]\n\"@DIR@/gone.json\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-n"][..], "[inputs], input_filename", &["ok", "gone", "empty"][..], "[1,2]\n\"@DIR@/empty.json\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-n"][..], "[inputs], input_filename", &["ok", "adir", "gone"][..], "[1,2]\n\"@DIR@/gone.json\"\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-n"][..], "[inputs], input_filename", &["ok", "gone", "adir"][..], "[1,2]\n\"@DIR@/adir\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-n"][..], "[inputs] | error(\"y\")", &["ok", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): y\n", 2),
        (&["-n"][..], "first(inputs), input_filename", &["ok", "gone"][..], "1\n\"@DIR@/ok.json\"\n", "", 0),
        (&["-n"][..], "input_filename", &["ok", "gone"][..], "null\n", "", 0),
        (&["-s"][..], "[., input_filename]", &["ok", "gone"][..], "[[1,2],\"@DIR@/gone.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-s"][..], "[., (try input catch .), input_filename]", &["ok", "gone"][..], "[[1,2],\"break\",\"@DIR@/gone.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-s"][..], "error(\"x\")", &["ok", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): x\n", 2),
        (&["-s"][..], "., input", &["ok", "gone"][..], "[1,2]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): break\n", 2),
        (&["-s"][..], "input_filename", &["gone"][..], "\"@DIR@/gone.json\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-s"][..], "[., input_filename]", &["gone", "m2"][..], "[[],\"@DIR@/m2.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/m2.json: No such file or directory\n", 2),
        (&["-s", "-R"][..], "[., input_filename]", &["ok", "gone"][..], "[\"1\\n2\\n\",\"@DIR@/gone.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-R", "-n"][..], "[inputs], input_filename, input_line_number", &["ab", "gone"][..], "[\"a\",\"b\"]\n\"@DIR@/gone.json\"\n0\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&[][..], "[., input_filename]", &["ok", "gone"][..], "[1,\"@DIR@/ok.json\"]\n[2,\"@DIR@/ok.json\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
    ] {
        let paths: Vec<String> = names
            .iter()
            .map(|&name| match name {
                "adir" => format!("{d}/adir"),
                "ab" => format!("{d}/ab.txt"),
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
