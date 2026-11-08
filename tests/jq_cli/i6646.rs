//! #6646: `--seq --stream` over a file whose last record is a container with a scalar before its
//! closer (`\x1e[1]`, `\x1e{"a":1}`) and no byte after it. jq's parser holds the closer's event back
//! until the next read, which is the open of the file that follows; the main loop ends there, so
//! the failures after that one are not reported. A closer that follows a container (`[[1]]`), no
//! scalar (`[]`) or a byte after it (`[1]\n`) is read with its own token, and every failure is
//! reported. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

const RS: &str = "\u{1e}";

const ONE_MISSING: &str = "jq: error: Could not open file @D@/gone: No such file or directory\n";
const TWO_MISSING: &str = "jq: error: Could not open file @D@/gone: No such file or directory\n\
                           jq: error: Could not open file @D@/gone2: No such file or directory\n";

/// `(flag, file text, stdout, stderr)`; each row runs `--seq -c FLAG . FILE gone gone2`.
type Row<'a> = (&'a str, String, String, &'a str);

#[test]
fn a_held_back_closer_ends_the_failures_reached_6646() -> Result<()> {
    let rows: Vec<Row> = vec![
        (
            "--stream",
            format!("{RS}[1]"),
            format!("{RS}[[0],1]\n{RS}[[0]]\n"),
            ONE_MISSING,
        ),
        (
            "--stream-errors",
            format!("{RS}{{\"a\":1}}"),
            format!("{RS}[[\"a\"],1]\n{RS}[[\"a\"]]\n"),
            ONE_MISSING,
        ),
        (
            "--stream",
            format!("{RS}[1]\n{RS}[2]"),
            format!("{RS}[[0],1]\n{RS}[[0]]\n{RS}[[0],2]\n{RS}[[0]]\n"),
            ONE_MISSING,
        ),
        (
            "--stream",
            format!("{RS}[[1]]"),
            format!("{RS}[[0,0],1]\n{RS}[[0,0]]\n{RS}[[0]]\n"),
            TWO_MISSING,
        ),
        (
            "--stream",
            format!("{RS}[1]\n"),
            format!("{RS}[[0],1]\n{RS}[[0]]\n"),
            TWO_MISSING,
        ),
        (
            "--stream",
            format!("{RS}[]"),
            format!("{RS}[[],[]]\n"),
            TWO_MISSING,
        ),
    ];
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (flag, text, stdout, stderr) in rows {
        std::fs::write(dir.path().join("f.json"), &text)?;
        let f = format!("{d}/f.json");
        let (gone, gone2) = (format!("{d}/gone"), format!("{d}/gone2"));
        let (out, err, code) = run_jq_full(&["--seq", "-c", flag, ".", &f, &gone, &gone2], None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (stdout.as_str(), stderr.replace("@D@", &d).as_str(), 2),
            "{flag} over {text:?}"
        );
    }
    Ok(())
}
