//! #6585: `--input-dsv` handles an input file that did not open as `-R` does (#6464, #6522, #6525,
//! #6531). A missing file is reported when a read reaches it, the main loop ends after the one
//! row read from past the first failure, and a failure past the file that row comes from is never
//! reached. `-s` reads every file first and still reports every failure up front. jq 1.7.1 has no
//! DSV input: the `-R` rows beside each DSV row are the behaviour it follows, captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

const GONE: &str = "jq: error: Could not open file @DIR@/gone.txt: No such file or directory\n";
const GONE2: &str = "jq: error: Could not open file @DIR@/gone2.txt: No such file or directory\n";

/// Runs `flags` over the files `names` (each a path in `dir`; `gone` and `gone2` do not exist).
fn run(dir: &str, flags: &[&str], names: &[&str]) -> Result<(String, String, i32)> {
    let files: Vec<String> = names.iter().map(|name| format!("{dir}/{name}")).collect();
    let mut args: Vec<&str> = flags.to_vec();
    args.extend(files.iter().map(String::as_str));
    run_jq_full(&args, None)
}

#[test]
fn input_dsv_reports_an_unopened_file_where_r_does_6585() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::write(dir.path().join("x.csv"), "a,b\n1,2\n")?;
    std::fs::write(dir.path().join("x.txt"), "l1\nl2\n")?;
    let (gone, gone2) = (GONE.replace("@DIR@", &d), GONE2.replace("@DIR@", &d));
    let both = format!("{gone}{gone2}");
    let (r1, r2) = ("\"l1\"\n", "\"l2\"\n");
    let (d1, d2) = ("[\"a\",\"b\"]\n", "[\"1\",\"2\"]\n");

    // (program, files, DSV stdout, `-R` stdout, stderr), each row run over `x.csv` and `x.txt`.
    for (program, names, dsv_out, raw_out, stderr) in [
        (
            ".",
            &["gone.txt", "F", "gone2.txt"][..],
            d1.to_string(),
            r1.to_string(),
            &gone,
        ),
        (
            ".",
            &["F", "gone.txt", "F", "gone2.txt"][..],
            format!("{d1}{d2}{d1}"),
            format!("{r1}{r2}{r1}"),
            &gone,
        ),
        (
            "input",
            &["gone.txt", "F", "gone2.txt"][..],
            d2.to_string(),
            r2.to_string(),
            &gone,
        ),
        (
            "input",
            &["F", "gone.txt", "F", "gone2.txt"][..],
            format!("{d2}{d2}"),
            format!("{r2}{r2}"),
            &gone,
        ),
        (
            "[.,input]",
            &["gone.txt", "F", "gone2.txt"][..],
            "[[\"a\",\"b\"],[\"1\",\"2\"]]\n".to_string(),
            "[\"l1\",\"l2\"]\n".to_string(),
            &gone,
        ),
        // A failure at the end of the list is reached when the reader finds no more rows.
        (
            ".",
            &["F", "gone.txt"][..],
            format!("{d1}{d2}"),
            format!("{r1}{r2}"),
            &gone,
        ),
        // `-n` reads on past every file: the program asked for them all.
        (
            "[inputs]|length",
            &["F", "gone.txt", "F", "gone2.txt"][..],
            "4\n".to_string(),
            "4\n".to_string(),
            &both,
        ),
    ] {
        let program_flags: &[&str] = if program.starts_with("[inputs]") {
            &["-n", "-c"]
        } else {
            &["-c"]
        };
        for (mode, ext, out) in [
            (&["--input-dsv", ","][..], "csv", &dsv_out),
            (&["-R"][..], "txt", &raw_out),
        ] {
            let names: Vec<String> = names
                .iter()
                .map(|n| {
                    n.replace('F', &format!("x.{ext}"))
                        .replace("gone.txt", &format!("gone.{ext}"))
                        .replace("gone2.txt", &format!("gone2.{ext}"))
                })
                .collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            let mut flags: Vec<&str> = program_flags.to_vec();
            flags.extend_from_slice(mode);
            flags.push(program);
            let (got_out, got_err, code) = run(&d, &flags, &names)?;
            let want_err = stderr
                .replace("gone.txt", &format!("gone.{ext}"))
                .replace("gone2.txt", &format!("gone2.{ext}"));
            assert_eq!(
                (got_out.as_str(), got_err.as_str(), code),
                (out.as_str(), want_err.as_str(), 2),
                "{flags:?} {names:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn input_dsv_slurp_still_reports_every_unopened_file_up_front_6585() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::write(dir.path().join("x.csv"), "a,b\n1,2\n")?;
    let (out, err, code) = run(
        &d,
        &["-c", "-s", "--input-dsv", ",", "."],
        &["gone.csv", "x.csv", "gone2.csv"],
    )?;
    assert_eq!(out, "[[\"a\",\"b\"],[\"1\",\"2\"]]\n");
    assert_eq!(
        err,
        format!(
            "jq: error: Could not open file {d}/gone.csv: No such file or directory\njq: error: Could not open file {d}/gone2.csv: No such file or directory\n"
        )
    );
    assert_eq!(code, 2);
    Ok(())
}

#[test]
fn input_dsv_validate_keeps_reporting_every_unopened_file_up_front_6585() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::write(dir.path().join("x.csv"), "a,b\n1,2\n")?;
    let (out, err, code) = run(
        &d,
        &["-c", "--validate", "--input-dsv", ",", "."],
        &["gone.csv", "x.csv", "gone2.csv"],
    )?;
    assert_eq!(out, "[\"a\",\"b\"]\n[\"1\",\"2\"]\n");
    assert_eq!(err.matches("Could not open file").count(), 2, "{err}");
    assert_eq!(code, 2);
    Ok(())
}

#[test]
fn input_dsv_rows_are_read_from_the_file_they_end_in_past_a_missing_one_6585() -> Result<()> {
    // A quoted field with a newline in it is one record, so the first file's record is whole in it
    // and the run ends after the row of the file past the missing one. A record left open at the
    // end of a file is joined to the next file's (#6601) and is read from the file it ends in, so a
    // missing file between the two is reported when that row is read, as `-R` does for a line with
    // no newline, and the run ends after that row.
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::write(dir.path().join("q.csv"), "\"a\nb\",c\n")?;
    std::fs::write(dir.path().join("o.csv"), "x,\"y")?;
    std::fs::write(dir.path().join("z.csv"), "z,y\n")?;
    let flags = ["-c", "--input-dsv", ",", "."];
    let (out, err, code) = run(&d, &flags, &["q.csv", "gone.csv", "z.csv", "gone2.csv"])?;
    assert_eq!(out, "[\"a\\nb\",\"c\"]\n[\"z\",\"y\"]\n");
    assert_eq!(
        err,
        format!("jq: error: Could not open file {d}/gone.csv: No such file or directory\n")
    );
    assert_eq!(code, 2);
    let (out, err, code) = run(&d, &flags, &["gone.csv", "o.csv", "gone2.csv", "z.csv"])?;
    // One row, the open record of `o.csv` completed by `z.csv`; how the unterminated quote reads is
    // not what this pins. The row is read from `z.csv`, past `gone2.csv`, so both are reported.
    assert!(out.starts_with("[\"x\","), "{out}");
    assert_eq!(out.lines().count(), 1, "{out}");
    assert_eq!(
        err,
        format!(
            "jq: error: Could not open file {d}/gone.csv: No such file or directory\njq: error: Could not open file {d}/gone2.csv: No such file or directory\n"
        )
    );
    assert_eq!(code, 2);
    Ok(())
}
