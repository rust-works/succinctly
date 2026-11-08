//! #6618: jq's parser counts containers against its depth ceiling, not the scalar the deepest one
//! holds, so a document 256 containers deep is read and one 257 deep is refused. The `--seq`
//! queue and `-s` routes, and plain `-n` with `inputs`, refused the 256 (one container short of
//! the ceiling, the scalar counted as a level), and the queue dropped the record with no word.
//! Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// `depth` nested arrays around `1`, then a string document after it.
fn nested(depth: usize, seq: bool) -> Vec<u8> {
    let mut text = Vec::new();
    if seq {
        text.push(0x1e);
    }
    text.extend(std::iter::repeat(b'[').take(depth));
    text.push(b'1');
    text.extend(std::iter::repeat(b']').take(depth));
    text.push(b'\n');
    if seq {
        text.push(0x1e);
    }
    text.extend_from_slice(b"\"y\"\n");
    text
}

#[test]
fn a_document_256_containers_deep_is_read_and_257_is_refused_6618() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let seq = dir.path().join("deep.seq");
    let json = dir.path().join("deep.json");
    let seq = seq.to_string_lossy().into_owned();
    let json = json.to_string_lossy().into_owned();
    // (flags, program, depth, stdout, stderr, status); `@FILE@` is the file the row reads.
    for (flags, program, depth, stdout, stderr, status) in [
        (&["--seq", "-nc"][..], "[inputs|type]", 255, "\u{1e}[\"array\",\"string\"]\n", "", 0),
        (&["--seq", "-sc"][..], "length", 255, "\u{1e}2\n", "", 0),
        (&["-nc"][..], "[inputs|type]", 255, "[\"array\",\"string\"]\n", "", 0),
        (&["--seq", "-nc"][..], "[inputs|type]", 256, "\u{1e}[\"array\",\"string\"]\n", "", 0),
        (&["--seq", "-sc"][..], "length", 256, "\u{1e}2\n", "", 0),
        (&["-nc"][..], "[inputs|type]", 256, "[\"array\",\"string\"]\n", "", 0),
        (&["--seq", "-nc"][..], "[inputs|type]", 257, "", "jq: error (at @FILE@:1): Exceeds depth limit for parsing at line 1, column 258 (need RS to resync)\n", 5),
        (&["-nc"][..], "[inputs|type]", 257, "", "jq: error (at @FILE@:1): Exceeds depth limit for parsing at line 1, column 257\n", 5),
    ] {
        let is_seq = flags.contains(&"--seq");
        let file = if is_seq { &seq } else { &json };
        std::fs::write(file, nested(depth, is_seq))?;
        let mut args = flags.to_vec();
        args.extend([program, file.as_str()]);
        let (out, err, code) = run_jq_full(&args, None)?;
        let expected_err = stderr.replace("@FILE@", file);
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (stdout, expected_err.as_str(), status),
            "{flags:?} {program} at depth {depth}"
        );
    }
    Ok(())
}
