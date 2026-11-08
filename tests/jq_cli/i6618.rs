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

/// `containers` nested containers around the shape's innermost value: a scalar, an object holding
/// one, or an empty array (which jq counts as a container, so it has no scalar to spare a level).
fn nested_shape(shape: &str, containers: usize) -> String {
    match shape {
        "scalar" => format!("{}1{}", "[".repeat(containers), "]".repeat(containers)),
        "object" => format!(
            "{}{{\"a\":1}}{}",
            "[".repeat(containers - 1),
            "]".repeat(containers - 1)
        ),
        "empty" => format!(
            "{}[]{}",
            "[".repeat(containers - 1),
            "]".repeat(containers - 1)
        ),
        _ => unreachable!("{shape}"),
    }
}

/// Each shape at 255, 256 and 257 containers through each route: at 256 and below jq answers with
/// the value, at 257 it refuses (exit 5 for a read that raises, a warning for `--seq`'s main loop).
#[test]
fn a_value_at_the_nesting_ceiling_is_read_as_jq_reads_it_6618() -> Result<()> {
    const RS: &str = "\u{1e}";
    // (route, args, framed with RS)
    let routes: [(&str, &[&str], bool); 4] = [
        ("seq inputs", &["--seq", "-nc", "[inputs|type]"], true),
        ("seq main", &["--seq", "-c", "type"], true),
        ("seq slurp", &["--seq", "-sc", "length"], true),
        ("-n input", &["-nc", "input|type"], false),
    ];
    // (shape, containers, then per route: stdout, exit status, whether it fails with the depth refusal)
    let ok: [(&str, usize); 5] = [
        ("scalar", 255),
        ("scalar", 256),
        ("object", 256),
        ("empty", 256),
        ("empty", 255),
    ];
    for (shape, containers) in ok {
        for (route, args, framed) in routes {
            let doc = nested_shape(shape, containers);
            let input = if framed {
                format!("{RS}{doc}\n")
            } else {
                format!("{doc}\n")
            };
            let (out, err, code) = run_jq_full(args, Some(&input))?;
            let want = match route {
                "seq inputs" => format!("{RS}[\"array\"]\n"),
                "seq main" => format!("{RS}\"array\"\n"),
                "seq slurp" => format!("{RS}1\n"),
                _ => "\"array\"\n".to_string(),
            };
            assert_eq!(
                (out.as_str(), err.as_str(), code),
                (want.as_str(), "", 0),
                "{route} {shape} x{containers}"
            );
        }
    }
    for shape in ["scalar", "empty"] {
        for (route, args, framed) in routes {
            let doc = nested_shape(shape, 257);
            let input = if framed {
                format!("{RS}{doc}\n")
            } else {
                format!("{doc}\n")
            };
            let (out, err, code) = run_jq_full(args, Some(&input))?;
            let (want_out, want_code) = match route {
                "seq inputs" | "-n input" => (String::new(), 5),
                "seq main" => (String::new(), 0),
                _ => (format!("{RS}0\n"), 0),
            };
            assert_eq!(
                (out.as_str(), code),
                (want_out.as_str(), want_code),
                "{route} {shape} x257"
            );
            assert!(
                err.contains("Exceeds depth limit for parsing"),
                "{route} {shape} x257: {err}"
            );
        }
    }
    Ok(())
}

/// `--slurpfile` reads the file's values through the same materializer: 256 containers around a
/// scalar, around an object holding one, or around an empty array is one value, as jq reads it,
/// and 257 is refused.
#[test]
fn slurpfile_reads_a_value_at_the_nesting_ceiling_6618() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("deep.json");
    let path = file.to_string_lossy().into_owned();
    for shape in ["scalar", "object", "empty"] {
        std::fs::write(&file, format!("{}\n", nested_shape(shape, 256)))?;
        let (out, err, code) = run_jq_full(&["-nc", "--slurpfile", "v", &path, "$v|length"], None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            ("1\n", "", 0),
            "{shape}"
        );
    }
    std::fs::write(&file, format!("{}\n", nested_shape("scalar", 257)))?;
    let (out, _, code) = run_jq_full(&["-nc", "--slurpfile", "v", &path, "$v|length"], None)?;
    assert_eq!((out.as_str(), code), ("", 2));
    Ok(())
}
