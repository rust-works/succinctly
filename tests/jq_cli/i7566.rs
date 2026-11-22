//! #7566: a UTF-8 byte-order mark at the front of the input that is cut across the first files
//! (`EF BB` in one file and `BF [2]` in the next, or a byte of it in each of three) is skipped, as jq
//! 1.7.1 and 1.8.2 skip it: jq reads its files as one stream and its parser matches the mark a byte
//! at a time, whichever buffer each byte comes in. It was `Invalid numeric literal at line 1, column 4`
//! (exit 5), because only a whole mark at the front of the first file with bytes was skipped. A stream
//! that ends inside a mark cut that way is empty input, as #7454 made a lone `EF` or `EF BB`.
//!
//! Every expectation was captured from jq 1.7.1 and 1.8.2, which agree on every row here.

use anyhow::Result;

use crate::small_window;
use crate::spawn_jq;

/// A program's flags and what jq prints for the stream `EF BB BF [2]`, however it is cut into files.
const SPLIT_MARK_THEN_ARRAY: [(&[&str], &str); 9] = [
    (&["-c", "."], "[2]\n"),
    (&["-c", "-s", "."], "[[2]]\n"),
    (&["-c", "-n", "[inputs]"], "[[2]]\n"),
    (&["-c", "-n", "input"], "[2]\n"),
    (&["-c", "--stream", "."], "[[0],2]\n[[0]]\n"),
    (&["-c", "-n", "--stream", "[inputs]"], "[[[0],2],[[0]]]\n"),
    (&["-c", "--stream-errors", "."], "[[0],2]\n[[0]]\n"),
    (&["-c", "--seq", "."], ""),
    (&["-e", "-c", ".[0]"], "2\n"),
];

/// The same flags over a stream that ends inside the mark: empty input.
const SPLIT_MARK_ALONE: [(&[&str], &str); 6] = [
    (&["-c", "."], ""),
    (&["-c", "-s", "."], "[]\n"),
    (&["-c", "-n", "[inputs]"], "[]\n"),
    (&["-c", "--stream", "."], ""),
    (&["-c", "-n", "--stream", "[inputs]"], "[]\n"),
    (&["-c", "--stream-errors", "."], ""),
];

fn run(args: &[&str]) -> Result<(String, String, i32)> {
    let (output, status) = spawn_jq(args, None)?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        status,
    ))
}

/// Every way to cut `text` into files at the positions `cuts` offers (each one cut or not), each
/// list also with a file with no bytes in front and one after the first file.
fn every_cut(text: &[u8], cuts: &[usize]) -> Vec<Vec<Vec<u8>>> {
    let mut lists = Vec::new();
    for mask in 0..1u32 << cuts.len() {
        let mut files = Vec::new();
        let mut from = 0;
        for (bit, &cut) in cuts.iter().enumerate() {
            if mask & (1 << bit) != 0 {
                files.push(text[from..cut].to_vec());
                from = cut;
            }
        }
        files.push(text[from..].to_vec());
        let mut empty_first = files.clone();
        empty_first.insert(0, Vec::new());
        let mut empty_second = files.clone();
        empty_second.insert(1, Vec::new());
        lists.extend([files, empty_first, empty_second]);
    }
    lists
}

fn write_files(dir: &std::path::Path, files: &[Vec<u8>]) -> Result<Vec<String>> {
    files
        .iter()
        .enumerate()
        .map(|(at, bytes)| {
            let path = dir.join(format!("f{at}.json"));
            std::fs::write(&path, bytes)?;
            Ok(path.to_string_lossy().into_owned())
        })
        .collect()
}

/// The `--seq` row's stderr: jq abandons the text, which has no record separator, and names its
/// column past the mark.
const SEQ_ABANDONED: &str =
    "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 3\n";

#[test]
fn a_mark_cut_across_files_is_skipped_7566() -> Result<()> {
    let dir = tempfile::tempdir()?;
    // Cut anywhere in the mark, and between the mark and the value and inside the value too.
    for files in every_cut(b"\xef\xbb\xbf[2]", &[1, 2, 3, 4]) {
        let sub = tempfile::tempdir_in(dir.path())?;
        let paths = write_files(sub.path(), &files)?;
        for (args, stdout) in SPLIT_MARK_THEN_ARRAY {
            let mut all = args.to_vec();
            all.extend(paths.iter().map(String::as_str));
            let got = run(&all)?;
            let stderr = if args.contains(&"--seq") {
                SEQ_ABANDONED
            } else {
                ""
            };
            assert_eq!(
                (got.0.as_str(), got.1.as_str(), got.2),
                (stdout, stderr, 0),
                "{args:?} over {files:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_stream_that_ends_inside_a_mark_cut_across_files_is_empty_7566() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for text in [&b"\xef\xbb"[..], b"\xef\xbb\xbf"] {
        for files in every_cut(text, &(1..text.len()).collect::<Vec<_>>()) {
            let sub = tempfile::tempdir_in(dir.path())?;
            let paths = write_files(sub.path(), &files)?;
            for (args, stdout) in SPLIT_MARK_ALONE {
                let mut all = args.to_vec();
                all.extend(paths.iter().map(String::as_str));
                let got = run(&all)?;
                assert_eq!(
                    (got.0.as_str(), got.1.as_str(), got.2),
                    (stdout, "", 0),
                    "{args:?} over {files:?}"
                );
            }
        }
    }
    Ok(())
}

/// The mark is the front of the stream only: one that starts in a later file, after a whole one or
/// after a value, is bytes of the stream, and a mark that departs from itself in the next file is a
/// fault (`Malformed BOM` in jq, a wording that is #5804's: here only nothing printed and exit 5).
#[test]
fn a_mark_is_only_the_streams_front_7566() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for (files, stdout, stderr) in [
        (
            &[&b"\xef\xbb\xbf"[..], b"\xef\xbb\xbf[2]"][..],
            "",
            "jq: parse error: Invalid numeric literal at line 1, column 4\n",
        ),
        (
            &[b"\xef\xbb", b"\xbf\xef[2]"],
            "",
            "jq: parse error: Invalid numeric literal at line 1, column 2\n",
        ),
        (
            &[b"\xef\xbb\xbf", b"\xef"],
            "",
            "jq: parse error: Invalid numeric literal at EOF at line 1, column 1\n",
        ),
        (
            &[b"[1]", b"\xef", b"\xbb\xbf[2]"],
            "[1]\n",
            "jq: parse error: Invalid numeric literal at line 1, column 7\n",
        ),
    ] {
        let sub = tempfile::tempdir_in(dir.path())?;
        let files: Vec<Vec<u8>> = files.iter().map(|bytes| bytes.to_vec()).collect();
        let paths = write_files(sub.path(), &files)?;
        let mut all = vec!["-c", "."];
        all.extend(paths.iter().map(String::as_str));
        let got = run(&all)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (stdout, stderr, 5),
            "{files:?}"
        );
    }
    for files in [
        &[&b"\xef"[..], b"1"][..],
        &[b"\xef\xbb", b"1"],
        &[b"\xef", b"\xbb", b"1"],
    ] {
        let sub = tempfile::tempdir_in(dir.path())?;
        let files: Vec<Vec<u8>> = files.iter().map(|bytes| bytes.to_vec()).collect();
        let paths = write_files(sub.path(), &files)?;
        for args in [
            &["-c", "."][..],
            &["-c", "-s", "."],
            &["-c", "--stream", "."],
        ] {
            let mut all = args.to_vec();
            all.extend(paths.iter().map(String::as_str));
            let got = run(&all)?;
            assert_eq!((got.0.as_str(), got.2), ("", 5), "{args:?} over {files:?}");
        }
    }
    Ok(())
}

/// The routes that read several files a window at a time skip a mark at the front of the first file
/// with bytes; one cut across files is left to the route that reads them whole, which joins it. Each
/// run is the run over the same records with no mark, which is jq's answer (it skips the mark).
#[cfg(unix)]
#[test]
fn a_mark_cut_across_files_is_skipped_on_the_windowed_routes_7566() -> Result<()> {
    let count = small_window::scaled(120_000);
    let mut records = Vec::new();
    for id in 0..count {
        records.extend_from_slice(
            format!("{{\"id\":{id},\"pad\":\"{}\"}}\n", "x".repeat(24)).as_bytes(),
        );
    }
    assert!(
        records.len() >= small_window::MIN_BYTES,
        "{}",
        records.len()
    );
    let dir = tempfile::tempdir()?;
    let plain = dir.path().join("plain.json");
    std::fs::write(&plain, &records)?;
    let plain = plain.to_string_lossy().into_owned();
    let ids: String = (0..count).map(|id| format!("{id}\n")).collect();
    let programs: [(&[&str], String); 4] = [
        (&["-c", ".id"], ids.clone()),
        (&["-c", "-n", "[inputs] | length"], format!("{count}\n")),
        (&["-c", "-n", "first(inputs).id"], "0\n".into()),
        (&["-c", "-n", "input.id, input.id"], "0\n1\n".into()),
    ];
    for (at, front) in [
        &[&b"\xef"[..], b"\xbb\xbf"][..],
        &[b"\xef\xbb", b"\xbf"],
        &[b"\xef", b"\xbb", b"\xbf"],
        &[b"", b"\xef"],
    ]
    .iter()
    .enumerate()
    {
        let sub = dir.path().join(format!("cut{at}"));
        std::fs::create_dir(&sub)?;
        // The mark's last bytes open the records' file, unless the cut leaves them a file of their own.
        let mut files: Vec<Vec<u8>> = front.iter().map(|bytes| bytes.to_vec()).collect();
        let last = files.pop().unwrap_or_default();
        let mut big = last;
        if big.is_empty() || big == b"\xef" {
            // `EF` alone: the records' file starts with the mark's other two bytes.
            files.push(big);
            big = b"\xbb\xbf".to_vec();
        }
        big.extend_from_slice(&records);
        files.push(big);
        let paths = write_files(&sub, &files)?;
        for (args, want) in &programs {
            let mut all = args.to_vec();
            all.extend(paths.iter().map(String::as_str));
            let (output, status) = small_window::spawn_jq(&all, None)?;
            let mut alone = args.to_vec();
            alone.push(&plain);
            let (plain_output, plain_status) = small_window::spawn_jq(&alone, None)?;
            assert_eq!(plain_output.stdout, want.as_bytes(), "{alone:?}");
            assert_eq!(
                (
                    String::from_utf8_lossy(&output.stdout) == *want,
                    String::from_utf8_lossy(&output.stderr).into_owned(),
                    status
                ),
                (true, String::new(), plain_status),
                "{args:?} over {front:?}"
            );
        }
    }
    Ok(())
}
