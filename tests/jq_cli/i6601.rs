//! #6601: `--input-dsv` reads its files as one stream of records, as jq reads its files and as `-R`
//! joins a line to the next file's: a quoted field that runs past the end of a file continues in
//! the next, and the row is the one the files joined give. `input_filename` names the file the
//! record ends in. jq 1.7.1 has no DSV input, so each row is compared with the same bytes in one
//! file.

use anyhow::Result;

use crate::run_jq_full;

/// Runs `flags` with `--input-dsv ,` over the files `names` in `dir`.
fn run(dir: &str, flags: &[&str], names: &[&str]) -> Result<(String, String, i32)> {
    let files: Vec<String> = names.iter().map(|name| format!("{dir}/{name}")).collect();
    let mut args: Vec<&str> = vec!["-c", "--input-dsv", ","];
    args.extend_from_slice(flags);
    args.extend(files.iter().map(String::as_str));
    run_jq_full(&args, None)
}

#[test]
fn a_quoted_record_spanning_files_is_one_row_6601() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("span1.csv", "a,\"b\n"),
        ("span2.csv", "c\",d\n"),
        ("span.csv", "a,\"b\nc\",d\n"),
        ("ok.csv", "x,y\n"),
        ("mid1.csv", "p,\"q\n"),
        ("mid2.csv", "r\n"),
        ("mid3.csv", "s\",t\n"),
        ("esc1.csv", "a,\"b\"\"\n"),
        ("esc2.csv", "c\",d\n"),
        ("empty.csv", ""),
        ("crlf1.csv", "a,\"b\r\n"),
        ("crlf2.csv", "c\"\r\n"),
        ("nn1.csv", "x,y"),
        ("nn2.csv", "z,w\n"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    let joined = "[\"a\",\"b\\nc\",\"d\"]\n";
    for (flags, names, stdout) in [
        // The streaming route, a program that reads `input`, and `-s`.
        (&["."][..], &["span1.csv", "span2.csv"][..], joined),
        (&["."][..], &["span.csv"][..], joined),
        (
            &["-n", "[inputs]"][..],
            &["span1.csv", "span2.csv"][..],
            "[[\"a\",\"b\\nc\",\"d\"]]\n",
        ),
        (
            &["-n", "input"][..],
            &["span1.csv", "span2.csv"][..],
            joined,
        ),
        (
            &["-s", "."][..],
            &["span1.csv", "span2.csv"][..],
            "[[\"a\",\"b\\nc\",\"d\"]]\n",
        ),
        // A record spanning three files, and an empty file between.
        (
            &["."][..],
            &["mid1.csv", "mid2.csv", "mid3.csv"][..],
            "[\"p\",\"q\\nr\\ns\",\"t\"]\n",
        ),
        (
            &["."][..],
            &["span1.csv", "empty.csv", "span2.csv"][..],
            joined,
        ),
        // A doubled quote is not a toggle that matters: the field is still open.
        (
            &["."][..],
            &["esc1.csv", "esc2.csv"][..],
            "[\"a\",\"b\\\"\\nc\",\"d\"]\n",
        ),
        // Files that each end on a record are untouched.
        (
            &["."][..],
            &["ok.csv", "span.csv"][..],
            "[\"x\",\"y\"]\n[\"a\",\"b\\nc\",\"d\"]\n",
        ),
        // `-R` with `--input-dsv` reads the same rows.
        (&["-R", "."][..], &["span1.csv", "span2.csv"][..], joined),
        // A last row with no newline stays its own row: only an open quote joins.
        (
            &["."][..],
            &["nn1.csv", "nn2.csv"][..],
            "[\"x\",\"y\"]\n[\"z\",\"w\"]\n",
        ),
        // CRLF records read as the joined file does.
        (
            &["."][..],
            &["crlf1.csv", "crlf2.csv"][..],
            "[\"a\",\"\\\"b\\r\\nc\\\"\\r\"]\n",
        ),
        // A record still open at the end of the last file is read as it stood.
        (
            &["."][..],
            &["ok.csv", "span1.csv"][..],
            "[\"x\",\"y\"]\n[\"a\",\"\\\"b\\n\"]\n",
        ),
    ] {
        let (out, err, code) = run(&d, flags, names)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (stdout, "", 0),
            "{flags:?} {names:?}"
        );
    }
    Ok(())
}

#[test]
fn a_spanning_record_is_named_for_the_file_it_ends_in_6601() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::write(dir.path().join("ok.csv"), "x,y\n")?;
    std::fs::write(dir.path().join("span1.csv"), "a,\"b\n")?;
    std::fs::write(dir.path().join("span2.csv"), "c\",d\n")?;
    let (out, err, code) = run(
        &d,
        &["[., input_filename]"],
        &["ok.csv", "span1.csv", "span2.csv"],
    )?;
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        (
            format!(
                "[[\"x\",\"y\"],\"{d}/ok.csv\"]\n[[\"a\",\"b\\nc\",\"d\"],\"{d}/span2.csv\"]\n"
            )
            .as_str(),
            "",
            0
        )
    );
    Ok(())
}
