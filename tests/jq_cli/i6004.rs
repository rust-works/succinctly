//! #6004: a fault that `input` raises, met in bytes moved across several files (a file that is only an
//! open tail hands it on to the next, #4305), names the file jq's read of those bytes was in: the
//! file the bytes came from, not the later file they now sit in. Every expected marker was captured
//! from jq 1.7.1 (`/usr/bin/jq`); `{DIR}` is the directory the files (`a`, `b`, `c`, `d`) are in.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_fault_in_moved_bytes_names_the_file_it_was_read_in_6004() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().to_string_lossy().into_owned();
    // (files, the first line jq's stderr has)
    let rows: &[(&[&[u8]], &str)] = &[
        // `[` rides through `b`, whose invalid byte is the fault, onto `c`.
        (
            &[b"[", b"\xe1\n", b"]"],
            "jq: error (at {DIR}/b:1): Invalid numeric literal at line 2, column 0",
        ),
        (
            &[b"{1", b":", b"2}"],
            "jq: error (at {DIR}/b:0): Object keys must be strings at line 1, column 3",
        ),
        // Four files, the fault in the first, in the third.
        (
            &[b"{}\n{1:", b"2", b"}{\"a\"", b":\n1}\t23\t"],
            "jq: error (at {DIR}/a:1): Object keys must be strings at line 2, column 3",
        ),
        (
            &[b"\"a\\nb", b"\"[\t{", b"]{\"a\":\n1", b"} "],
            "jq: error (at {DIR}/c:1): Unmatched ']' at line 1, column 10",
        ),
        // A fault in the file's own bytes is that file's, whatever was moved onto it.
        (
            &[b"[1,", b"2,\n", b"x]"],
            "jq: error (at {DIR}/c:0): Invalid numeric literal at line 2, column 2",
        ),
        (
            &[b"[\n", b"}\n", b"1]\n"],
            "jq: error (at {DIR}/b:1): Unmatched '}' at line 2, column 1",
        ),
    ];
    for (files, want) in rows {
        let paths: Vec<String> = files
            .iter()
            .zip(["a", "b", "c", "d"])
            .map(|(bytes, name)| {
                let path = dir.path().join(name);
                std::fs::write(&path, bytes)?;
                Ok(path.to_string_lossy().into_owned())
            })
            .collect::<Result<_>>()?;
        let mut args = vec!["-nc", "[inputs]"];
        args.extend(paths.iter().map(String::as_str));
        let (stdout, stderr, code) = run_jq_full(&args, None)?;
        assert_eq!(
            (stdout.as_str(), stderr.lines().next(), code),
            ("", Some(want.replace("{DIR}", &prefix).as_str()), 5),
            "{files:?}"
        );
    }
    Ok(())
}
