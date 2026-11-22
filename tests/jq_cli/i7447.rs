//! #7447: a program reading `input`/`inputs` over one source whose first document is longer than the
//! lazy route's 256 KiB front starts on that front all the same: the start reads on until the document
//! ends and the rest is read a window at a time, where it used to read the whole source. Every row was
//! checked against jq 1.7.1 on the same bytes (jq 1.8.2 answers the same). That the run holds a window
//! and not the file is pinned in `jq_runner.rs`
//! (`a_first_document_longer_than_the_front_still_starts_on_it_7447`).

use anyhow::Result;

use crate::spawn_jq;

/// A 300,000-byte string, longer than the front, and then `rows` small records, one to a line.
fn stream(rows: &[String]) -> Vec<u8> {
    let mut bytes = format!("\"{}\"\n", "a".repeat(300_000)).into_bytes();
    bytes.extend_from_slice(rows.join("\n").as_bytes());
    bytes.push(b'\n');
    bytes
}

fn rows() -> Vec<String> {
    (0..2000)
        .map(|i| format!("{{\"id\":{i},\"name\":\"user {i}\"}}"))
        .collect()
}

/// `bytes` as a file named after `args`, or on standard input with `piped`: stdout, stderr with the
/// file's name as `<f>`, and the exit status.
fn run(bytes: &[u8], args: &[&str], piped: bool) -> Result<(String, String, i32)> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("long-first.json");
    std::fs::write(&path, bytes)?;
    let name = path.to_string_lossy().into_owned();
    let mut argv: Vec<&str> = args.to_vec();
    if !piped {
        argv.push(&name);
    }
    let (out, status) = spawn_jq(&argv, piped.then_some(bytes))?;
    Ok((
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).replace(&name, "<f>"),
        status,
    ))
}

#[test]
fn a_long_first_document_is_read_with_the_rest_as_jq_reads_it_7447() -> Result<()> {
    let bytes = stream(&rows());
    for piped in [false, true] {
        let (out, err, status) = run(&bytes, &["-n", "reduce inputs as $x (0; . + 1)"], piped)?;
        assert_eq!((out.as_str(), err.as_str(), status), ("2001\n", "", 0));
        let (out, _, _) = run(
            &bytes,
            &["-nc", "[first(inputs) | length, input_line_number]"],
            piped,
        )?;
        assert_eq!(out, "[300000,1]\n", "piped {piped}");
        let (out, _, _) = run(
            &bytes,
            &["-nc", "last(inputs) | [.id, input_line_number]"],
            piped,
        )?;
        assert_eq!(out, "[1999,2001]\n", "piped {piped}");
    }
    Ok(())
}

#[test]
fn a_fault_after_a_long_first_document_is_worded_as_jq_words_it_7447() -> Result<()> {
    let mut rows = rows();
    rows[1000] = "}".to_string();
    let bytes = stream(&rows);
    let (out, err, status) = run(&bytes, &["-nc", "[inputs | .id?] | length"], false)?;
    assert_eq!(out, "");
    assert_eq!(
        err,
        "jq: error (at <f>:1002): Unmatched '}' at line 1002, column 1\n"
    );
    assert_eq!(status, 5);
    let (out, _, status) = run(&bytes, &["-nc", "first(inputs) | length"], false)?;
    assert_eq!((out.as_str(), status), ("300000\n", 0));
    Ok(())
}

#[test]
fn a_long_first_document_across_lines_counts_its_lines_7447() -> Result<()> {
    // An array of 60,000 lines, with the next document after it on its last line.
    let array = format!(
        "[{}]",
        (0..60_000)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",\n")
    );
    let bytes = [array.as_bytes(), b" ", &stream(&rows())[300_003..]].concat();
    let (out, _, status) = run(
        &bytes,
        &["-nc", "[first(inputs) | length, input_line_number]"],
        false,
    )?;
    assert_eq!((out.as_str(), status), ("[60000,60000]\n", 0));
    let (out, _, _) = run(&bytes, &["-n", "reduce inputs as $x (0; . + 1)"], false)?;
    assert_eq!(out, "2001\n");
    Ok(())
}
