//! #6451: a lone source a program reads with `input`/`inputs` starts on the front of it, and the
//! rest is read when the scan gets there. The streams here are longer than that front (256 KiB),
//! with a byte that is not UTF-8, or a fault, after it: the run reads them as it read the whole
//! file up front, from a file and from stdin. Every row was captured from jq 1.7.1 (`jq -nc`).

use anyhow::Result;

use crate::spawn_jq;

/// 12,000 short documents, 373,780 bytes, then `tail`.
fn stream(tail: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for i in 0..12_000 {
        bytes.extend_from_slice(format!("{{\"id\":{i},\"name\":\"user {i}\"}}\n").as_bytes());
    }
    bytes.extend_from_slice(tail);
    bytes
}

#[test]
fn a_lone_source_past_its_front_reads_as_jq_reads_it_6451() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let late_invalid = stream(b"{\"id\":\"\xff\xfe\"}\n{\"id\":12001}\n");
    let late_fault = stream(b"{\"id\":}\n{\"id\":12002}\n");
    let fault = |name: &str| {
        format!("jq: error (at {name}:12001): Unmatched '}}' at line 12001, column 7\n")
    };
    for (bytes, program, stdout, stderr, status) in [
        (
            &late_invalid,
            "[inputs|.id]|.[-2:]",
            "[\"\u{fffd}\u{fffd}\",12001]\n",
            None,
            0,
        ),
        (&late_invalid, "[inputs|.id]|length", "12002\n", None, 0),
        (&late_invalid, "first(inputs)|.id", "0\n", None, 0),
        (
            &late_invalid,
            "reduce inputs as $x (0; .+1), input_line_number",
            "12002\n12002\n",
            None,
            0,
        ),
        (&late_fault, "[inputs|.id]|.[-2:]", "", Some(()), 5),
        (&late_fault, "first(inputs)|.id", "0\n", None, 0),
        (
            &late_fault,
            "reduce inputs as $x (0; .+1), input_line_number",
            "",
            Some(()),
            5,
        ),
    ] {
        let path = dir.path().join("stream.json");
        std::fs::write(&path, bytes)?;
        let path = path.to_string_lossy().into_owned();
        for (input, name) in [(None, path.as_str()), (Some(bytes.as_slice()), "<stdin>")] {
            let args: Vec<&str> = match input {
                None => vec!["-nc", program, &path],
                Some(_) => vec!["-nc", program],
            };
            let (output, code) = spawn_jq(&args, input)?;
            assert_eq!(
                String::from_utf8(output.stdout)?,
                stdout,
                "{program} on {name}"
            );
            let expected = stderr.map(|()| fault(name)).unwrap_or_default();
            assert_eq!(
                String::from_utf8(output.stderr)?,
                expected,
                "{program} on {name}"
            );
            assert_eq!(code, status, "{program} on {name}");
        }
    }
    Ok(())
}
