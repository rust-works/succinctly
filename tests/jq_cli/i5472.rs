//! #5472: a main input file that will not open is reported and skipped; jq reads on.
//!
//! Every row was captured from jq 1.7.1. jq reports `jq: error: Could not open file ...` once
//! per file, as its reader reaches it, and exits 2 at the end whatever else happened. Its main
//! loop also ends the run once an open has failed, after the one value it was fetching when it
//! hit the failure -- so past a missing file jq prints the first value of the next file that
//! has one, and no more.

use anyhow::Result;

use crate::run_jq_full;

/// A temp dir holding `data.json` (`{"a":1,"b":[1,2]}`), `two.json` (`2 3`) and `five.json`
/// (`2 3 4 5 6`), and the path of a file that is not there.
struct Files {
    _dir: tempfile::TempDir,
    data: String,
    two: String,
    five: String,
    gone: String,
    gone2: String,
}

impl Files {
    fn new() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let path = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
        let (data, two, five) = (path("data.json"), path("two.json"), path("five.json"));
        std::fs::write(&data, "{\"a\":1,\"b\":[1,2]}\n")?;
        std::fs::write(&two, "2 3\n")?;
        std::fs::write(&five, "2 3 4 5 6\n")?;
        Ok(Self {
            gone: path("gone.json"),
            gone2: path("gone2.json"),
            data,
            two,
            five,
            _dir: dir,
        })
    }
}

/// jq's report of a file it could not open.
fn open_error(path: &str) -> String {
    format!("jq: error: Could not open file {path}: No such file or directory\n")
}

#[test]
fn a_missing_file_does_not_stop_the_files_after_it_5472() -> Result<()> {
    let f = Files::new()?;
    let (out, err, code) = run_jq_full(&["-c", ".a", &f.gone, &f.data], None)?;
    assert_eq!((out.as_str(), code), ("1\n", 2), "stderr: {err:?}");
    assert_eq!(err, open_error(&f.gone));
    Ok(())
}

#[test]
fn a_missing_file_in_the_middle_or_at_the_end_5472() -> Result<()> {
    let f = Files::new()?;
    let (out, err, code) = run_jq_full(&["-c", ".a", &f.data, &f.gone, &f.data], None)?;
    assert_eq!((out.as_str(), code), ("1\n1\n", 2), "stderr: {err:?}");
    assert_eq!(err, open_error(&f.gone));

    let (out, err, code) = run_jq_full(&["-c", ".a", &f.data, &f.gone], None)?;
    assert_eq!((out.as_str(), code), ("1\n", 2), "stderr: {err:?}");
    assert_eq!(err, open_error(&f.gone));
    Ok(())
}

#[test]
fn every_missing_file_is_reported_in_order_5472() -> Result<()> {
    let f = Files::new()?;
    let (out, err, code) = run_jq_full(&["-c", ".", &f.gone, &f.gone2], None)?;
    assert_eq!((out.as_str(), code), ("", 2));
    assert_eq!(
        err,
        format!("{}{}", open_error(&f.gone), open_error(&f.gone2))
    );

    let (out, err, code) = run_jq_full(&["-c", ".a", &f.gone, &f.gone2, &f.data], None)?;
    assert_eq!((out.as_str(), code), ("1\n", 2));
    assert_eq!(
        err,
        format!("{}{}", open_error(&f.gone), open_error(&f.gone2))
    );
    Ok(())
}

/// jq's main loop stops once an open has failed, after the value it was fetching.
#[test]
fn past_a_missing_file_only_the_next_value_is_read_5472() -> Result<()> {
    let f = Files::new()?;
    let (out, err, code) = run_jq_full(&["-c", ".", &f.gone, &f.five], None)?;
    assert_eq!((out.as_str(), code), ("2\n", 2), "stderr: {err:?}");
    assert_eq!(err, open_error(&f.gone));

    // The files before the failure are read whole; the one value is the first past it.
    let (out, _, code) = run_jq_full(&["-c", ".", &f.two, &f.gone, &f.five], None)?;
    assert_eq!((out.as_str(), code), ("2\n3\n2\n", 2));

    // A file past the failure with no value gives the turn to the next one.
    let empty = format!("{}.empty", f.data);
    std::fs::write(&empty, "")?;
    let (out, _, code) = run_jq_full(&["-c", ".", &f.gone, &empty, &f.five, &f.five], None)?;
    assert_eq!((out.as_str(), code), ("2\n", 2));

    // A second failure past the one value is never reached, so it is not reported.
    let (out, err, code) = run_jq_full(&["-c", ".", &f.gone, &f.five, &f.gone2], None)?;
    assert_eq!((out.as_str(), code), ("2\n", 2));
    assert_eq!(err, open_error(&f.gone));
    Ok(())
}

#[test]
fn a_missing_file_is_reported_where_the_reader_reaches_it_5472() -> Result<()> {
    let f = Files::new()?;
    // `.a` fails on each of the two numbers in `two.json`, and only then does jq's reader
    // go looking for the next file.
    let (out, err, code) = run_jq_full(&["-c", ".a", &f.two, &f.gone], None)?;
    assert_eq!((out.as_str(), code), ("", 2));
    let at = |n: &str| format!("jq: error (at {n}:1): Cannot index number with string \"a\"\n");
    assert_eq!(
        err,
        format!("{}{}{}", at(&f.two), at(&f.two), open_error(&f.gone))
    );
    Ok(())
}

#[test]
fn exit_status_2_outranks_the_filter_and_dash_e_5472() -> Result<()> {
    let f = Files::new()?;
    let (out, _, code) = run_jq_full(&["-c", "-e", ".a", &f.gone, &f.data], None)?;
    assert_eq!((out.as_str(), code), ("1\n", 2));

    let (out, _, code) = run_jq_full(&["-c", "-e", ".zz", &f.gone, &f.data], None)?;
    assert_eq!((out.as_str(), code), ("null\n", 2));

    // An error in the filter is exit 5 without the missing file and 2 with it.
    let (out, err, code) = run_jq_full(&["-c", ".a|error", &f.gone, &f.data], None)?;
    assert_eq!((out.as_str(), code), ("", 2), "stderr: {err:?}");
    assert!(err.starts_with(&open_error(&f.gone)), "stderr: {err:?}");
    Ok(())
}

#[test]
fn halt_after_a_missing_file_exits_2_5472() -> Result<()> {
    let f = Files::new()?;
    // jq's exit status is 2 once an open has failed, over `halt`'s and `halt_error`'s own.
    for filter in ["halt", "halt_error(7)", ".|halt_error"] {
        let (out, err, code) = run_jq_full(&["-c", filter, &f.gone, &f.data], None)?;
        assert_eq!((out.as_str(), code), ("", 2), "{filter}: {err:?}");
        assert!(err.starts_with(&open_error(&f.gone)), "{filter}: {err:?}");
    }
    // A halt before the reader reaches the missing file never reports it: the file is not
    // opened as far as the run goes, and the halt's own code stands.
    let (out, err, code) = run_jq_full(&["-c", "halt_error(7)", &f.data, &f.gone], None)?;
    assert_eq!((out.as_str(), code), ("", 7), "stderr: {err:?}");
    assert!(!err.contains("Could not open"), "stderr: {err:?}");
    Ok(())
}

#[test]
fn input_filename_names_the_file_that_opened_5472() -> Result<()> {
    let f = Files::new()?;
    let (out, _, code) = run_jq_full(&["-c", "input_filename", &f.gone, &f.data], None)?;
    assert_eq!(
        (out.as_str(), code),
        (format!("{:?}\n", f.data).as_str(), 2)
    );
    Ok(())
}

#[test]
fn slurp_reads_every_file_that_opens_5472() -> Result<()> {
    let f = Files::new()?;
    let (out, err, code) = run_jq_full(&["-s", "-c", ".", &f.gone, &f.data, &f.gone2], None)?;
    assert_eq!((out.as_str(), code), ("[{\"a\":1,\"b\":[1,2]}]\n", 2));
    assert_eq!(
        err,
        format!("{}{}", open_error(&f.gone), open_error(&f.gone2))
    );

    // Nothing opened: jq slurps nothing, and still exits 2.
    let (out, err, code) = run_jq_full(&["-s", "-c", ".", &f.gone, &f.gone2], None)?;
    assert_eq!((out.as_str(), code), ("[]\n", 2));
    assert_eq!(
        err,
        format!("{}{}", open_error(&f.gone), open_error(&f.gone2))
    );
    Ok(())
}

#[test]
fn null_input_reads_no_file_unless_the_filter_asks_5472() -> Result<()> {
    let f = Files::new()?;
    let (out, err, code) = run_jq_full(&["-n", "1", &f.gone], None)?;
    assert_eq!((out.as_str(), err.as_str(), code), ("1\n", "", 0));

    let (out, err, code) = run_jq_full(&["-n", "-c", "[inputs]", &f.gone, &f.data], None)?;
    assert_eq!((out.as_str(), code), ("[{\"a\":1,\"b\":[1,2]}]\n", 2));
    assert_eq!(err, open_error(&f.gone));
    Ok(())
}

/// The route that materializes its input (`-R`) skips the file the same way.
#[test]
fn raw_input_skips_a_missing_file_5472() -> Result<()> {
    let f = Files::new()?;
    let (out, err, code) = run_jq_full(&["-R", "-c", ".", &f.gone, &f.data], None)?;
    assert_eq!(
        (out.as_str(), code),
        ("\"{\\\"a\\\":1,\\\"b\\\":[1,2]}\"\n", 2)
    );
    assert_eq!(err, open_error(&f.gone));
    Ok(())
}

/// jq has no DSV input mode, so there is no oracle: the streaming DSV route is only held to
/// the same skip-and-exit-2 shape.
#[test]
fn dsv_input_skips_a_missing_file_5472() -> Result<()> {
    let f = Files::new()?;
    let csv = format!("{}.csv", f.data);
    std::fs::write(&csv, "x,y\n")?;
    let (out, err, code) = run_jq_full(&["--input-dsv", ",", "-c", ".", &f.gone, &csv], None)?;
    assert_eq!((out.as_str(), code), ("[\"x\",\"y\"]\n", 2));
    assert_eq!(err, open_error(&f.gone));
    Ok(())
}
