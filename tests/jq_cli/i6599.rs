//! #6599: `--seq -s -n` beside a file that does not open. The program's `input` reads the stream, so
//! nothing prints while it is read: a failure is said by the read that reaches the file, and a fault
//! met in a file before it stops the program first (exit 5, the failure never reported). A fault
//! names the file it was met in, and one at the end of the text is raised at a last file that did not
//! open. No main loop reads the stream under `-n`, so no failure loses the array: a fault the slurp's
//! own call did not meet comes after it. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

const FOUR: &str =
    "[(try input catch .), (try input catch .), (try input catch .), (try input catch .)]";

#[test]
fn a_seq_slurp_under_null_input_places_an_unopened_file_where_its_reads_reach_it_6599() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("bad", "\u{1e}1\n\u{1e}{\"a\n\u{1e}2\n"),
        ("norsend", "\u{1e}7"),
        ("defer", "\u{1e}1{"),
        ("s2", "\u{1e}[1]\n\u{1e}[2]\n"),
        ("trunc", "\u{1e}{\"a\n"),
        ("endrs", "\u{1e}1\n\u{1e}{\"a\n\u{1e}"),
    ] {
        std::fs::write(dir.path().join(format!("{name}.json")), text)?;
    }
    std::fs::create_dir(dir.path().join("adir.json"))?;
    for (program, names, stdout, stderr, status) in [
        ("input", &["bad", "gone"][..], "", "jq: error (at @DIR@/bad.json:3): Truncated value at line 3, column 1\n", 5),
        ("input", &["endrs", "gone"][..], "", "jq: error (at @DIR@/endrs.json:2): Truncated value at line 3, column 1\n", 5),
        ("[inputs]", &["bad", "gone"][..], "", "jq: error (at @DIR@/bad.json:3): Truncated value at line 3, column 1\n", 5),
        (FOUR, &["bad", "gone"][..], "\u{1e}[\"Truncated value at line 3, column 1\",[1,2],\"break\",\"break\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        ("input", &["bad", "gone", "adir"][..], "", "jq: error (at @DIR@/bad.json:3): Truncated value at line 3, column 1\n", 5),
        ("input", &["gone", "bad"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/bad.json:3): Truncated value at line 3, column 1\n", 2),
        ("[inputs]", &["gone", "bad"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/bad.json:3): Truncated value at line 3, column 1\n", 2),
        ("input", &["gone", "norsend"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/norsend.json:0): Potentially truncated top-level numeric value at EOF at line 1, column 2\n", 2),
        ("input", &["gone", "trunc", "s2"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/s2.json:1): Truncated value at line 2, column 1\n", 2),
        ("input", &["bad", "gone", "s2"][..], "", "jq: error (at @DIR@/bad.json:3): Truncated value at line 3, column 1\n", 5),
        ("input", &["norsend", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Potentially truncated top-level numeric value at EOF at line 1, column 2\n", 2),
        ("input", &["defer", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished JSON term at EOF at line 1, column 3\n", 2),
        ("input", &["trunc", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished string at EOF at line 2, column 0\n", 2),
        ("input", &["s2", "trunc", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished string at EOF at line 4, column 0\n", 2),
        ("input", &["defer", "gone", "gone2"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\njq: error (at @DIR@/gone2.json:0): Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (FOUR, &["gone", "defer"][..], "\u{1e}[[1],\"Unfinished JSON term at EOF at line 1, column 3\",\"break\",\"break\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (FOUR, &["s2", "gone", "defer"][..], "\u{1e}[[[1],[2],1],\"Unfinished JSON term at EOF at line 3, column 3\",\"break\",\"break\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (FOUR, &["defer"][..], "\u{1e}[[1],\"Unfinished JSON term at EOF at line 1, column 3\",\"break\",\"break\"]\n", "", 0),
        (FOUR, &["s2", "defer", "gone"][..], "\u{1e}[\"Unfinished JSON term at EOF at line 3, column 3\",[[1],[2],1],\"break\",\"break\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (FOUR, &["trunc", "gone", "s2"][..], "\u{1e}[\"Truncated value at line 2, column 1\",[[1],[2]],\"break\",\"break\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        ("input_filename", &["gone", "s2"][..], "\u{1e}null\n", "", 0),
        ("input", &["s2", "gone"][..], "\u{1e}[[1],[2]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        ("[inputs]", &["s2", "gone", "adir"][..], "\u{1e}[[[1],[2]]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Is a directory\n", 2),
        ("first(inputs)", &["s2", "gone", "adir"][..], "\u{1e}[[1],[2]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
    ] {
        let files: Vec<String> = names.iter().map(|name| format!("{d}/{name}.json")).collect();
        let mut args = vec!["--seq", "-c", "-s", "-n", program];
        args.extend(files.iter().map(String::as_str));
        let (out, err, code) = run_jq_full(&args, None)?;
        let expected = (stdout.replace("@DIR@", &d), stderr.replace("@DIR@", &d));
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (expected.0.as_str(), expected.1.as_str(), status),
            "{program} {names:?}"
        );
    }
    Ok(())
}
