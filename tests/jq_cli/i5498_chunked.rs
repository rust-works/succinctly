//! #5498: the stream reader reads a file or stdin a chunk at a time (jq's `fgets` reads), so it
//! holds neither the input nor its events. These inputs cross every boundary that read has: a line
//! much longer than 4095 bytes, a multi-byte character cut by one, a line of exactly 4095 and of
//! 4096 bytes, a leading BOM, blank lines and many short lines. The expected values follow from how
//! the inputs are built, and the same programs run over a file and over stdin.

use anyhow::Result;

use std::io::Write;
use std::process::{Command, Stdio};

use crate::{run_jq_full, succinctly_bin};

/// `succinctly jq <args>` over `stdin`, which a thread writes while the output is read: the chunked
/// read takes stdin as it goes, so a harness that writes it all before it reads any output (as
/// `run_jq_full` does) deadlocks once the output outgrows a pipe.
fn run_jq_piped(args: &[&str], stdin: &str) -> Result<(String, String, i32)> {
    let mut child = Command::new(succinctly_bin())
        .arg("jq")
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().expect("a piped stdin");
    let bytes = stdin.as_bytes().to_vec();
    let writer = std::thread::spawn(move || input.write_all(&bytes));
    let output = child.wait_with_output()?;
    writer.join().expect("the stdin writer")?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        output.status.code().unwrap_or(-1),
    ))
}

/// `(name, text, events, last line, string lengths)`: the number of `--stream` events (leaves and
/// closing events), what `input_line_number` reads at the last event, and the lengths of the
/// strings the document holds, in order. Each value is what `/usr/bin/jq` 1.7.1 answers.
type Case = (&'static str, String, usize, usize, Vec<usize>);

fn cases() -> Vec<Case> {
    let long_line = format!(
        "[{}]\n",
        (0..30_000)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let multibyte = format!(
        "[\"{}\"]\n{{\"k\":\"{}\"}}\n",
        "\u{e9}".repeat(5_000),
        "\u{20ac}".repeat(3_000)
    );
    let ndjson: String = (0..20_000).map(|n| format!("{{\"n\":{n}}}\n")).collect();
    let exactly_a_chunk = format!("[{}]\n[1]\n", "1".repeat(4095 - 3));
    let one_over = format!("[{}]\n[1]\n", "1".repeat(4096 - 3));
    vec![
        ("a 170 KB line", long_line, 30_001, 1, vec![]),
        (
            "multi-byte characters across chunks",
            multibyte,
            4,
            2,
            vec![5_000, 3_000],
        ),
        ("20,000 short lines", ndjson.clone(), 40_000, 20_000, vec![]),
        ("a BOM", format!("\u{feff}{ndjson}"), 40_000, 20_000, vec![]),
        (
            "a line of exactly 4095 bytes",
            exactly_a_chunk,
            4,
            2,
            vec![],
        ),
        ("a line of 4096 bytes", one_over, 4, 2, vec![]),
        (
            "blank lines",
            "\n\n[1]\n\n\n{\"a\":[]}\n\n".to_string(),
            4,
            6,
            vec![],
        ),
        (
            "no final newline",
            "[1,[2,3]] {\"a\":4}".to_string(),
            7,
            0,
            vec![],
        ),
    ]
}

#[test]
fn a_chunked_read_gives_every_event_and_line_5498() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.json");
    let file = path.to_string_lossy().into_owned();
    let mut runs = 0;
    for (name, text, events, last_line, lengths) in cases() {
        std::fs::write(&path, &text)?;
        let programs: [(&[&str], String); 4] = [
            (
                &["-nc", "--stream", "[inputs]|length"],
                format!("{events}\n"),
            ),
            (
                &["-nc", "--stream-errors", "[inputs]|length"],
                format!("{events}\n"),
            ),
            (
                &["-nc", "--stream", "[inputs|input_line_number]|last"],
                format!("{last_line}\n"),
            ),
            (
                &[
                    "-nc",
                    "--stream",
                    "[inputs|select(length==2)|.[1]|strings|length]",
                ],
                format!("{}\n", serde_json::to_string(&lengths)?),
            ),
        ];
        // Each program over the file and over stdin must answer the same.
        for (args, want) in programs {
            let mut with_file = args.to_vec();
            with_file.push(&file);
            let by_file = run_jq_full(&with_file, None)?;
            assert_eq!(
                by_file,
                (want.clone(), String::new(), 0),
                "{name}: {args:?} over a file"
            );
            let by_stdin = run_jq_piped(args, &text)?;
            assert_eq!(
                by_stdin,
                (want, String::new(), 0),
                "{name}: {args:?} over stdin"
            );
            runs += 2;
        }
        // The plain walk pops one event at a time: one line each.
        let (stdout, stderr, code) = run_jq_full(&["-c", "--stream", ".", &file], None)?;
        assert_eq!((stderr.as_str(), code), ("", 0), "{name}: a plain walk");
        assert_eq!(
            stdout.lines().count(),
            events,
            "{name}: a plain walk over a file"
        );
        let (stdout, _, _) = run_jq_piped(&["-c", "--stream", "."], &text)?;
        assert_eq!(
            stdout.lines().count(),
            events,
            "{name}: a plain walk over stdin"
        );
        runs += 2;
    }
    assert_eq!(runs, cases().len() * 5 * 2);
    Ok(())
}

/// A BOM at the front of the first file that has bytes is skipped, and one anywhere else is a
/// byte of a value (#5291): the chunked read puts back what it read to look for it.
#[test]
fn the_bom_is_skipped_once_at_the_front_of_the_chunked_read_5498() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let write = |name: &str, text: &str| -> Result<String> {
        let path = dir.path().join(name);
        std::fs::write(&path, text)?;
        Ok(path.to_string_lossy().into_owned())
    };
    let (empty, first, second) = (
        write("empty.json", "")?,
        write("first.json", "\u{feff}[1]\n")?,
        write("second.json", "[2]\n")?,
    );
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "--stream", ".", &empty, &first, &second], None)?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        ("[[0],1]\n[[0]]\n[[0],2]\n[[0]]\n", "", 0)
    );
    // A second BOM is a byte of the second file's value, which jq cannot read.
    let bom_again = write("bom_again.json", "\u{feff}[3]\n")?;
    let (stdout, stderr, code) = run_jq_full(&["-c", "--stream", ".", &first, &bom_again], None)?;
    assert_eq!(stdout, "[[0],1]\n[[0]]\n");
    assert_eq!(code, 5, "{stderr}");
    Ok(())
}
