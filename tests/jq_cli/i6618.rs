//! #6618: a record or document at jq's own nesting ceiling, 256 containers, is read by every route
//! that builds its value, as jq reads it. The ceiling of the materializer counted the scalar inside
//! the 256th container as one level more, so `--seq` (queue and `-s`), `-n input` and `--slurpfile`
//! dropped or refused it, where one more container is jq's own refusal. Every expected string was
//! captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `containers` nested containers around the shape's innermost value: a scalar, an object holding
/// one, or an empty array (which jq counts as a container, so it has no scalar to spare a level).
fn nested(shape: &str, containers: usize) -> String {
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
            let doc = nested(shape, containers);
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
            let doc = nested(shape, 257);
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
/// scalar is one value, as jq reads it, and 257 is refused.
#[test]
fn slurpfile_reads_a_value_at_the_nesting_ceiling_6618() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("deep.json");
    std::fs::write(&file, format!("{}\n", nested("scalar", 256)))?;
    let path = file.to_string_lossy().into_owned();
    let (out, err, code) = run_jq_full(&["-nc", "--slurpfile", "v", &path, "$v|length"], None)?;
    assert_eq!((out.as_str(), err.as_str(), code), ("1\n", "", 0));
    std::fs::write(&file, format!("{}\n", nested("scalar", 257)))?;
    let (out, _, code) = run_jq_full(&["-nc", "--slurpfile", "v", &path, "$v|length"], None)?;
    assert_eq!((out.as_str(), code), ("", 2));
    Ok(())
}
