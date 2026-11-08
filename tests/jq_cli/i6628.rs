//! #6628: `-s` over a document past jq's parse depth limit prints jq's own position, `at line L,
//! column C`, of the bracket it refuses, counted across the files as jq reads them. Standard input
//! cannot be read again, so a slurp of stdin keeps the message without it (see `limitations.md`).
//! Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

fn deep(n: usize) -> String {
    format!("{}1{}", "[".repeat(n), "]".repeat(n))
}

#[test]
fn a_slurp_past_the_depth_limit_names_the_refused_bracket_6628() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    let objects = |n: usize| format!("{}1{}", "{\"a\":".repeat(n), "}".repeat(n));
    let files: [(&str, String); 11] = [
        ("d257.json", format!("{}\n", deep(257))),
        ("d256.json", format!("{}\n", deep(256))),
        ("d300.json", format!("{}\n", deep(300))),
        ("o257.json", format!("{}\n", objects(257))),
        ("second.json", format!("1\n[2]\n{}\n", deep(257))),
        ("blank.json", format!("1\n\n  \n{}\n", deep(257))),
        (
            "indoc.json",
            format!("[1,\n2,\n{}{}\n]\n", "[".repeat(257), "]".repeat(257)),
        ),
        (
            "thirdline.json",
            format!(
                "{{\"a\":\n1}}\n{{\"b\":[\n{}{}]}}\n",
                "[".repeat(257),
                "]".repeat(257)
            ),
        ),
        ("bom.json", format!("\u{feff}{}\n", deep(257))),
        ("crlf.json", format!("1\r\n{}\r\n", deep(257))),
        ("one.json", "1\n[2]\n".to_string()),
    ];
    for (name, text) in &files {
        std::fs::write(dir.path().join(name), text)?;
    }
    let fault = |at: &str| format!("jq: parse error: Exceeds depth limit for parsing at {at}\n");
    let gone = format!("jq: error: Could not open file {d}/gone.json: No such file or directory\n");
    let rows: [(&[&str], String, i32); 13] = [
        (&["d257.json"], fault("line 1, column 257"), 5),
        (&["d300.json"], fault("line 1, column 257"), 5),
        (&["o257.json"], fault("line 1, column 641"), 5),
        (&["second.json"], fault("line 3, column 257"), 5),
        (&["blank.json"], fault("line 4, column 257"), 5),
        (&["indoc.json"], fault("line 3, column 256"), 5),
        (&["thirdline.json"], fault("line 4, column 254"), 5),
        (&["one.json", "d257.json"], fault("line 3, column 257"), 5),
        (&["d257.json", "one.json"], fault("line 1, column 257"), 5),
        (&["bom.json"], fault("line 1, column 257"), 5),
        (&["crlf.json"], fault("line 2, column 257"), 5),
        // jq reads no further than the refused document, so a file after it is never opened.
        (&["d257.json", "gone.json"], fault("line 1, column 257"), 5),
        // A file that did not open before it is said first, and fails the run with status 2.
        (
            &["gone.json", "d257.json"],
            format!("{gone}{}", fault("line 1, column 257")),
            2,
        ),
    ];
    for (names, stderr, status) in rows {
        let paths: Vec<String> = names.iter().map(|name| format!("{d}/{name}")).collect();
        let mut args = vec!["-sc", "length"];
        args.extend(paths.iter().map(String::as_str));
        let (out, err, code) = run_jq_full(&args, None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            ("", stderr.as_str(), status),
            "{names:?}"
        );
    }
    // 256 levels still parse, and a document that is not past the limit is read as ever.
    let (out, err, code) = run_jq_full(&["-sc", "length", &format!("{d}/d256.json")], None)?;
    assert_eq!((out.as_str(), err.as_str(), code), ("1\n", "", 0));
    Ok(())
}
