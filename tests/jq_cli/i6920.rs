//! #6920: `succinctly jq` reads one big regular file a window at a time on the plain route (no `-n`,
//! nothing that reads `input`), so its peak memory no longer grows with the file. A window ends at a
//! line break, a document that does not scan is read on before it counts as a fault, and the lines
//! dropped are counted, so every row here is what the same bytes give on stdin, which the whole-file
//! route reads: stdout, stderr (with the file's name for stdin's) and the exit status.
//!
//! The files are over 4 MiB (the smallest the route takes). `windowed_plain_file_takes_only_a_lone_regular_file_6920`
//! in `jq_runner.rs` pins which runs take it.

use anyhow::Result;

use crate::spawn_jq;

/// How many lines the fixtures have: about 6.6 MiB, well over the 4 MiB the route takes, so the
/// route's minimum can move a little without the tests quietly reading whole files.
const LINES: usize = 60_000;

/// `count` lines of `{"id":N,"name":"user N",...}`.
fn lines(count: usize) -> Vec<String> {
    (0..count)
        .map(|i| {
            format!(
                "{{\"id\":{i},\"name\":\"user {i}\",\"email\":\"user{i}@example.com\",\"tags\":[\"a\",\"b\",\"c\"],\"score\":{}}}",
                i % 1000
            )
        })
        .collect()
}

fn join(lines: &[String]) -> Vec<u8> {
    let mut bytes = lines.join("\n").into_bytes();
    bytes.push(b'\n');
    bytes
}

/// The run over `bytes` as a file and as stdin, compared; the stdout and stderr of the file's run.
fn same_as_stdin(bytes: &[u8], args: &[&str]) -> Result<(String, String, i32)> {
    assert!(
        bytes.len() > 4 * 1024 * 1024,
        "the file must be over the windowed route's minimum"
    );
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("big.json");
    std::fs::write(&path, bytes)?;
    let name = path.to_string_lossy().into_owned();
    let mut with_file: Vec<&str> = args.to_vec();
    with_file.push(&name);
    let (file, file_status) = spawn_jq(&with_file, None)?;
    let (stdin, stdin_status) = spawn_jq(args, Some(bytes))?;
    let file_err = String::from_utf8_lossy(&file.stderr).replace(&name, "<stdin>");
    let stdin_err = String::from_utf8_lossy(&stdin.stderr).into_owned();
    assert!(
        file.stdout == stdin.stdout,
        "{args:?}: stdout differs ({} and {} bytes)",
        file.stdout.len(),
        stdin.stdout.len()
    );
    assert_eq!(file_err, stdin_err, "{args:?}: stderr");
    assert_eq!(file_status, stdin_status, "{args:?}: exit status");
    Ok((
        String::from_utf8_lossy(&file.stdout).into_owned(),
        file_err,
        file_status,
    ))
}

#[test]
fn a_clean_file_reads_as_its_bytes_on_stdin_do_6920() -> Result<()> {
    let bytes = join(&lines(LINES));
    for args in [
        &["-c", "select(.id % 7919 == 0) | .id"][..],
        &["-r", "select(.id % 9973 == 0) | .name"],
        &["-c", "select(.id > 59990)"],
        &["-e", "select(.id == 59999) | .name"],
        &["-c", ".tags | length", "--arg", "x", "1"],
    ] {
        let (out, err, status) = same_as_stdin(&bytes, args)?;
        assert_eq!((err.as_str(), status), ("", 0), "{args:?}");
        assert!(!out.is_empty(), "{args:?} selects something");
    }
    // The identity fast path writes every document's bytes.
    let (out, _, status) = same_as_stdin(&bytes, &["-c", "."])?;
    assert_eq!(status, 0);
    assert_eq!(out.lines().count(), LINES);
    assert!(out.ends_with("\"score\":999}\n"));
    Ok(())
}

#[test]
fn a_runtime_error_names_the_line_it_is_on_6920() -> Result<()> {
    let bytes = join(&lines(LINES));
    // The error names jq's line: the count through the read that completes the document.
    let (out, err, status) = same_as_stdin(
        &bytes,
        &["-c", "if .id == 30000 then error(\"boom\") else empty end"],
    )?;
    assert_eq!(
        (out.as_str(), err.as_str()),
        ("", "jq: error (at <stdin>:30001): boom\n")
    );
    let _ = status;
    // Many errors, on lines in several windows.
    let (_, err, status) = same_as_stdin(
        &bytes,
        &[
            "-c",
            "if .id % 9000 == 0 and .id > 0 then error(\"boom\") else empty end",
        ],
    )?;
    let _ = status;
    assert_eq!(
        err.lines().collect::<Vec<_>>(),
        [9000, 18000, 27000, 36000, 45000, 54000]
            .iter()
            .map(|n| format!("jq: error (at <stdin>:{}): boom", n + 1))
            .collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn a_fault_after_many_documents_is_worded_as_jq_words_it_6920() -> Result<()> {
    let mut mid = lines(LINES);
    mid[30_000] = "}".to_string();
    let (out, err, status) = same_as_stdin(&join(&mid), &["-c", "select(.id % 10000 == 0) | .id"])?;
    assert_eq!(out, "0\n10000\n20000\n");
    assert_eq!(
        err,
        "jq: parse error: Unmatched '}' at line 30001, column 1\n"
    );
    assert_ne!(status, 0);

    // A document the file ends inside.
    let mut eof = lines(LINES);
    eof.push("{\"id\":1,".to_string());
    let bytes = join(&eof);
    let (out, err, status) = same_as_stdin(
        &bytes[..bytes.len() - 1],
        &["-c", "select(.id == 59999) | .id"],
    )?;
    assert_eq!(out, "59999\n");
    assert!(err.contains("Unfinished JSON term"), "{err:?}");
    assert_eq!(status, 5);

    // A number the fault's own byte completes is never read.
    let mut bare = lines(LINES);
    bare[30_000] = "12}".to_string();
    let (out, err, status) = same_as_stdin(&join(&bare), &["-c", "select(.id == 29999) | .id"])?;
    assert_eq!(out, "29999\n");
    assert!(err.contains("Unmatched '}'"), "{err:?}");
    assert_eq!(status, 5);
    // The `12` itself is never read: no document of the stream is a number.
    let (out, _, _) = same_as_stdin(&join(&bare), &["-c", "select(type == \"number\")"])?;
    assert_eq!(out, "");
    Ok(())
}

#[test]
fn a_bom_and_bytes_that_are_not_utf8_read_as_they_do_whole_6920() -> Result<()> {
    let mut with_bom = b"\xef\xbb\xbf".to_vec();
    with_bom.extend(join(&lines(LINES)));
    let (out, _, status) = same_as_stdin(&with_bom, &["-c", "select(.id == 0) | .name"])?;
    assert_eq!((out.as_str(), status), ("\"user 0\"\n", 0));

    let mut bytes = join(&lines(LINES));
    // A lone 0xff inside a string, a truncated multi-byte character and a stray byte, in three windows.
    for (line, patch) in [
        (10_000usize, &b"\xff"[..]),
        (25_000, b"\xe2\x82"),
        (40_000, b"\xc3"),
    ] {
        let at = bytes
            .windows(format!("\"user {line}\"").len())
            .position(|w| w == format!("\"user {line}\"").as_bytes())
            .unwrap();
        let mut patched = bytes[..=at].to_vec();
        patched.extend_from_slice(patch);
        patched.extend_from_slice(&bytes[at + 1..]);
        bytes = patched;
    }
    let (out, err, status) = same_as_stdin(
        &bytes,
        &[
            "-c",
            "select(.id == 10000 or .id == 25000 or .id == 40000) | .name",
        ],
    )?;
    assert_eq!(status, 0, "{err:?}");
    // Each run of invalid bytes inside a string becomes one U+FFFD, in three different windows.
    assert_eq!(
        out,
        "\"\u{fffd}user 10000\"\n\"\u{fffd}user 25000\"\n\"\u{fffd}user 40000\"\n"
    );

    // Invalid bytes early, and a refusal late: the depth message is worded from the file as jq's
    // parser read it, which a window that was substituted cannot give.
    let mut deep = lines(LINES);
    deep[LINES / 2] = format!("{}{}", "[".repeat(300), "]".repeat(300));
    let mut bytes = join(&deep);
    let at = bytes.windows(8).position(|w| w == b"\"user 5\"").unwrap();
    let mut patched = bytes[..=at].to_vec();
    patched.push(0xff);
    patched.extend_from_slice(&bytes[at + 1..]);
    bytes = patched;
    let (out, err, status) = same_as_stdin(&bytes, &["-c", "select(.id) | .id"])?;
    assert_eq!(out.lines().count(), LINES / 2);
    assert!(err.contains("Exceeds depth limit"), "{err:?}");
    assert_ne!(status, 0);
    Ok(())
}

#[test]
fn documents_that_span_lines_and_share_them_read_as_they_do_whole_6920() -> Result<()> {
    // Pretty-printed documents: a window must not cut one in two.
    let pretty: Vec<u8> = (0..110_000)
        .map(|i| format!("{{\n  \"id\": {i},\n  \"tags\": [\n    1,\n    2\n  ]\n}}\n"))
        .collect::<String>()
        .into_bytes();
    let (out, err, status) = same_as_stdin(&pretty, &["-c", "select(.id % 20000 == 0) | .id"])?;
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        ("0\n20000\n40000\n60000\n80000\n100000\n", "", 0)
    );
    // One line holding every document.
    let one_line: Vec<u8> = (0..450_000)
        .map(|i| format!("{{\"id\":{i}}} "))
        .collect::<String>()
        .into_bytes();
    let (out, _, status) = same_as_stdin(&one_line, &["-c", "select(.id == 449999) | .id"])?;
    assert_eq!((out.as_str(), status), ("449999\n", 0));
    // One document bigger than a window, between two small ones.
    let mut big = b"{\"first\":1}\n[".to_vec();
    big.extend(
        (0..1_200_000)
            .map(|i| format!("{i},"))
            .collect::<String>()
            .into_bytes(),
    );
    big.extend_from_slice(b"0]\n{\"last\":2}\n");
    let (out, _, status) = same_as_stdin(
        &big,
        &["-c", "if type == \"object\" then . else length end"],
    )?;
    assert_eq!(
        (out.as_str(), status),
        ("{\"first\":1}\n1200001\n{\"last\":2}\n", 0)
    );
    Ok(())
}

#[test]
fn a_document_past_the_depth_limit_is_refused_where_it_is_6920() -> Result<()> {
    let mut deep = lines(LINES);
    deep[30_000] = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
    let (out, err, status) = same_as_stdin(&join(&deep), &["-c", "select(.id) | .id"])?;
    assert!(
        out.starts_with("0\n1\n2\n"),
        "{:?}",
        &out[..out.len().min(20)]
    );
    assert_eq!(out.lines().count(), 30_000);
    assert!(err.contains("Exceeds depth limit"), "{err:?}");
    assert!(err.contains("at line 30001"), "{err:?}");
    assert_ne!(status, 0);
    Ok(())
}

/// A named pipe is read as it always was: the route's gate must not open it first, which would pair
/// with the writer, close, and leave nothing for the open that reads (`jq . <(cmd)` is this).
#[cfg(unix)]
#[test]
fn a_named_pipe_still_delivers_its_bytes_6920() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fifo = dir.path().join("pipe");
    assert!(std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()?
        .success());
    let writer_path = fifo.clone();
    let writer = std::thread::spawn(move || {
        // Written after the reader has had time to be started: a gate that opened the pipe and dropped
        // it would have paired with this open and closed it on the writer.
        std::thread::sleep(std::time::Duration::from_millis(300));
        std::fs::write(&writer_path, b"{\"a\":1}\n{\"a\":2}\n")
    });
    let path = fifo.to_string_lossy().into_owned();
    let (output, status) = spawn_jq(&["-c", ".a", &path], None)?;
    writer.join().expect("writer thread")?;
    assert_eq!(
        (String::from_utf8_lossy(&output.stdout).as_ref(), status),
        ("1\n2\n", 0)
    );
    Ok(())
}

/// `/dev/stdin` names a descriptor the run already holds: it reads as stdin does, a fault included.
#[cfg(unix)]
#[test]
fn dev_stdin_reads_as_stdin_does_6920() -> Result<()> {
    let mut with_fault = lines(LINES);
    with_fault[20_000] = "}".to_string();
    let bytes = join(&with_fault);
    let args = ["-c", "select(.id % 10000 == 0) | .id"];
    let (plain, plain_status) = spawn_jq(&args, Some(&bytes))?;
    let (aliased, aliased_status) = spawn_jq(&[args[0], args[1], "/dev/stdin"], Some(&bytes))?;
    assert_eq!(plain.stdout, aliased.stdout);
    assert_eq!(plain_status, aliased_status);
    assert!(String::from_utf8_lossy(&aliased.stderr).contains("Unmatched '}' at line 20001"));
    Ok(())
}

/// A big file that is one line (a minified document) is read as whole: correct, and the window
/// reader neither searches it again after every read nor holds more than it did.
#[test]
fn a_minified_document_reads_as_it_does_whole_6920() -> Result<()> {
    let one_line = format!(
        "{{\"users\":[{}]}}",
        (0..100_000)
            .map(|n| format!("{{\"id\":{n},\"name\":\"user {n}\",\"tags\":[\"a\",\"b\"]}}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let (out, err, status) = same_as_stdin(
        one_line.as_bytes(),
        &["-c", ".users | map(select(.id % 10000 == 0) | .id)"],
    )?;
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        ("[0,10000,20000,30000,40000,50000,60000,70000,80000,90000]\n", "", 0)
    );
    Ok(())
}
