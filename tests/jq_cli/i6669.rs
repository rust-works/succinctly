//! #6669: a `--input-dsv` record that is still open at the end of the last file that opened asks the
//! reader for the next file, so every failure behind it is reported, as `-R` does for a last line with
//! no newline (`jq -R . gone nonl.txt gone2` reports both, captured from jq 1.7.1; jq has no DSV
//! input). A row that is complete where its file ends (a last row with no newline and no open quote)
//! stops the main loop at the first failure, as before. Each DSV row is run beside its `-R` row.

use anyhow::Result;

use crate::run_jq_full;

/// Runs `flags` and `program` over the files `names` in `dir`.
fn run(dir: &str, flags: &[&str], program: &str, names: &[&str]) -> Result<(String, String, i32)> {
    let files: Vec<String> = names.iter().map(|name| format!("{dir}/{name}")).collect();
    let mut args: Vec<&str> = flags.to_vec();
    args.push(program);
    args.extend(files.iter().map(String::as_str));
    run_jq_full(&args, None)
}

/// The names of the files `err` reports as not opened, in order.
fn reported(err: &str) -> Vec<&str> {
    err.lines()
        .filter_map(|line| line.strip_prefix("jq: error: Could not open file "))
        .filter_map(|rest| rest.split(':').next())
        .map(|path| path.rsplit('/').next().unwrap_or(path))
        .collect()
}

#[test]
fn an_open_record_asks_for_the_next_file_as_r_does_6669() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    // `o.csv` ends inside a quote; `nonl.txt` is the `-R` line with no newline.
    std::fs::write(dir.path().join("o.csv"), "x,\"y")?;
    std::fs::write(dir.path().join("ao.csv"), "a\nx,\"y")?;
    std::fs::write(dir.path().join("nonl.txt"), "x")?;
    std::fs::write(dir.path().join("anl.txt"), "a\nx")?;
    // A DSV last row with no newline and no open quote is complete where its file ends, as is
    // `-R`'s line with its newline: neither asks for more.
    std::fs::write(dir.path().join("done.csv"), "x")?;
    std::fs::write(dir.path().join("done.txt"), "x\n")?;

    // (flags, program, DSV files, `-R` files, reported by both).
    for (flags, program, dsv, raw, want) in [
        (
            &[][..],
            ".",
            &["gone.csv", "o.csv", "gone2.csv"][..],
            &["gone.txt", "nonl.txt", "gone2.txt"][..],
            &["gone.csv", "gone2.csv"][..],
        ),
        (
            &[][..],
            ".",
            &["gone.csv", "o.csv", "gone2.csv", "gone3.csv"][..],
            &["gone.txt", "nonl.txt", "gone2.txt", "gone3.txt"][..],
            &["gone.csv", "gone2.csv", "gone3.csv"][..],
        ),
        // A program that mentions `input` reads through the queue.
        (
            &[][..],
            ".,(if false then input else empty end)",
            &["gone.csv", "o.csv", "gone2.csv"][..],
            &["gone.txt", "nonl.txt", "gone2.txt"][..],
            &["gone.csv", "gone2.csv"][..],
        ),
        (
            &["-n"][..],
            "first(inputs)",
            &["o.csv", "gone.csv"][..],
            &["nonl.txt", "gone.txt"][..],
            &["gone.csv"][..],
        ),
        // The open record is the file's last row, after a complete one that ends the main loop.
        (
            &[][..],
            "input",
            &["gone.csv", "ao.csv", "gone2.csv"][..],
            &["gone.txt", "anl.txt", "gone2.txt"][..],
            &["gone.csv", "gone2.csv"][..],
        ),
        // The first row is complete, so the main loop stops at the first failure.
        (
            &[][..],
            ".",
            &["gone.csv", "ao.csv", "gone2.csv"][..],
            &["gone.txt", "anl.txt", "gone2.txt"][..],
            &["gone.csv"][..],
        ),
        // A last row with no newline and no open quote is complete where its file ends.
        (
            &[][..],
            ".",
            &["gone.csv", "done.csv", "gone2.csv"][..],
            &["gone.txt", "done.txt", "gone2.txt"][..],
            &["gone.csv"][..],
        ),
    ] {
        let (_, dsv_err, dsv_code) =
            run(&d, &[flags, &["--input-dsv", ","]].concat(), program, dsv)?;
        let (_, raw_err, raw_code) = run(&d, &[flags, &["-R"]].concat(), program, raw)?;
        let names: Vec<String> = want.iter().map(ToString::to_string).collect();
        assert_eq!(
            reported(&dsv_err),
            want,
            "{flags:?} {program} {dsv:?}: {dsv_err}"
        );
        assert_eq!(
            reported(&raw_err)
                .iter()
                .map(|name| name.replace(".txt", ".csv"))
                .collect::<Vec<_>>(),
            names,
            "{flags:?} {program} {raw:?}: {raw_err}"
        );
        assert_eq!((dsv_code, raw_code), (2, 2), "{flags:?} {program} {dsv:?}");
    }
    Ok(())
}
