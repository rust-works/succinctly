//! #6455: each of several files is scanned a chunk at a time as the program reads it, not only the
//! last. The first file here is longer than the first chunks, with a fault after them or a number at
//! its end that the next file's first bytes go on with (`12` then `34` is `1234`): the run reads them
//! as jq does. Every row was captured from jq 1.7.1 (`jq -nc`).

use anyhow::Result;

use crate::spawn_jq;

/// 20,000 short documents, 248,890 bytes, then `tail`.
fn stream(tail: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for i in 0..20_000 {
        bytes.extend_from_slice(format!("{{\"id\":{i}}}\n").as_bytes());
    }
    bytes.extend_from_slice(tail);
    bytes
}

#[test]
fn an_earlier_file_past_its_first_chunks_reads_as_jq_reads_it_6455() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let write = |name: &str, bytes: &[u8]| -> Result<String> {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes)?;
        Ok(path.to_string_lossy().into_owned())
    };
    let fault = write("fault.json", &stream(b"{\"id\":}\n{\"id\":20001}\n"))?;
    let bare = write("bare.json", &stream(b"12"))?;
    let next = write("next.json", b"34\n{\"id\":-1}\n")?;
    let resumed = "[range(20003) as $i | try input catch \"err\"] | .[-3:], input_line_number";
    for (first, program, stdout, stderr, status) in [
        (&fault, "first(inputs)|.id", "0\n", false, 0),
        (&fault, "[inputs|numbers]", "", true, 5),
        (
            &fault,
            resumed,
            "[\"err\",{\"id\":20001},34]\n1\n",
            false,
            0,
        ),
        (&bare, "first(inputs)|.id", "0\n", false, 0),
        (&bare, "[inputs|numbers]", "[1234]\n", false, 0),
        (&bare, resumed, "[1234,{\"id\":-1},\"err\"]\n2\n", false, 0),
        (
            &bare,
            "reduce inputs as $x (0; .+1), input_line_number",
            "20002\n2\n",
            false,
            0,
        ),
    ] {
        let (output, code) = spawn_jq(&["-nc", program, first, &next], None)?;
        assert_eq!(
            String::from_utf8(output.stdout)?,
            stdout,
            "{program} on {first}"
        );
        let expected = if stderr {
            format!("jq: error (at {fault}:20001): Unmatched '}}' at line 20001, column 7\n")
        } else {
            String::new()
        };
        assert_eq!(
            String::from_utf8(output.stderr)?,
            expected,
            "{program} on {first}"
        );
        assert_eq!(code, status, "{program} on {first}");
    }
    Ok(())
}
