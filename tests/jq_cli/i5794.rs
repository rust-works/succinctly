//! #5794: `walk(f)` on an object reads only the first output of `f` per member, as jq's
//! `map_values(w) | f` does (#5791), so an error `f` raises after that output is not reached there.
//! Each row reads a file, then the same text from stdin; stdout and exit status are captured from
//! jq 1.7.1 (`/usr/bin/jq`). The error diagnostic goes to stderr and is not compared.

use anyhow::Result;

use crate::run_jq_full;

/// `(input, filter, stdout, exit status)`.
const ROWS: &[(&str, &str, &str, i32)] = &[
    ("{\"a\":1}", "walk((.,error(\"x\")))", "{\"a\":1}\n", 5),
    ("{\"a\":{}}", "walk((.,error(\"x\")))", "{\"a\":{}}\n", 5),
    ("{\"a\":1}", "walk(1,error(\"x\"))", "1\n", 5),
    ("{}", "walk((.,error(\"x\")))", "{}\n", 5),
    // Nothing is produced before the error: jq raises with no output.
    ("{\"a\":1}", "walk((error(\"x\"),1))", "", 5),
    ("{\"a\":1}", "walk(empty,error(\"x\"))", "", 5),
];

#[test]
fn walk_object_reads_the_first_output_of_f_per_member_5794() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.json");
    for (input, filter, stdout, code) in ROWS {
        std::fs::write(&path, format!("{input}\n"))?;
        let (out, err, status) = run_jq_full(&["-c", filter, path.to_str().unwrap()], None)?;
        assert_eq!(
            (out.as_str(), status),
            (*stdout, *code),
            "{filter} over {input} (file): {err}"
        );
        let (out, err, status) = run_jq_full(&["-c", filter], Some(&format!("{input}\n")))?;
        assert_eq!(
            (out.as_str(), status),
            (*stdout, *code),
            "{filter} over {input} (stdin): {err}"
        );
    }
    Ok(())
}
