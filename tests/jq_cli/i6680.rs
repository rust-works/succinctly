//! #6680: `--stream` and `-n --stream --stream-errors` over a file whose last value is a container
//! with a scalar before its closer (`[1,2]`, `{"a":1}`) and no byte after it. jq's parser holds the
//! closer's event back until the next read, which is the open of the next file: a file that does
//! not open, or a directory, is an empty read to its reader, the closer is read there, and the
//! main loop ends. A file after it is not read and a failure after it is not reported. succinctly
//! put the closer in the file it came from, read on and reported them. A closer that follows a
//! container (`[[1]]`) or has a byte after it (`[1,2]\n`) is read with its own token, and nothing
//! changes. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

const GONE: &str = "jq: error: Could not open file @D@/gone: No such file or directory\n";
const GONE_AND_GONE2: &str = "jq: error: Could not open file @D@/gone: No such file or directory\n\
                              jq: error: Could not open file @D@/gone2: No such file or directory\n";
const EVENTS: &str = "[[0],1]\n[[1],2]\n[[1]]\n";
const COLLECTED: &str = "[[[0],1],[[1],2],[[1]]]\n";

/// `(file text, flags and program, files after it, stdout, stderr)`; every run is
/// `jq -c FLAGS... F.JSON FILES...` with the exit status 2.
type Row<'a> = (&'a str, &'a [&'a str], &'a [&'a str], &'a str, &'a str);

#[test]
fn a_held_back_closer_ends_the_run_at_the_next_failed_open_6680() -> Result<()> {
    let rows: &[Row] = &[
        // The issue's rows: the directory behind a missing file is never reached.
        (
            "[1,2]",
            &["-n", "--stream", "[inputs]"],
            &["gone", "dir"],
            COLLECTED,
            GONE,
        ),
        ("[1,2]", &["--stream", "."], &["dir", "dir2"], EVENTS, ""),
        (
            "[1,2]",
            &["--stream", "[inputs]"],
            &["gone", "dir"],
            "[[[1],2],[[1]]]\n",
            GONE,
        ),
        // `--stream-errors` reads through the same chunked reader under `-n`.
        (
            "[1,2]",
            &["-n", "--stream-errors", "[inputs]"],
            &["gone", "dir"],
            COLLECTED,
            GONE,
        ),
        // A failure behind the first is not reported, and a file behind it is not read.
        (
            "[1,2]",
            &["--stream", "."],
            &["gone", "gone2"],
            EVENTS,
            GONE,
        ),
        (
            "[1,2]",
            &["--stream", "."],
            &["gone", "g.json"],
            EVENTS,
            GONE,
        ),
        ("[1,2]", &["--stream", "."], &["dir", "g.json"], EVENTS, ""),
        (
            "{\"a\":1}",
            &["--stream", "."],
            &["gone", "gone2"],
            "[[\"a\"],1]\n[[\"a\"]]\n",
            GONE,
        ),
        // Not held back: a byte after the closer, or a closer that follows a container.
        (
            "[1,2]\n",
            &["--stream", "."],
            &["gone", "gone2"],
            EVENTS,
            GONE_AND_GONE2,
        ),
        (
            "[[1]]",
            &["--stream", "."],
            &["gone", "gone2"],
            "[[0,0],1]\n[[0,0]]\n[[0]]\n",
            GONE_AND_GONE2,
        ),
    ];
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    std::fs::create_dir(dir.path().join("dir2"))?;
    std::fs::write(dir.path().join("g.json"), "{\"a\":1}")?;
    for (text, flags, after, stdout, stderr) in rows {
        std::fs::write(dir.path().join("f.json"), text)?;
        let mut args: Vec<String> = vec!["-c".into()];
        args.extend(flags.iter().map(|flag| (*flag).to_string()));
        args.push(format!("{d}/f.json"));
        args.extend(after.iter().map(|name| format!("{d}/{name}")));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (out, err, code) = run_jq_full(&args, None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*stdout, stderr.replace("@D@", &d).as_str(), 2),
            "{text:?} {flags:?} then {after:?}"
        );
    }
    Ok(())
}
