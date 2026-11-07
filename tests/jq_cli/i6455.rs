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

/// #6455: a fault a later chunk meets in the stream's last file names `<unknown>` where jq does: a
/// number straight before the last byte, an opener, completes in the read that reaches the end
/// (#6004). The lazy route's fault handoff read the stream as one that went on past that file. From
/// one file and as the last of two; jq 1.7.1's rows.
#[test]
fn a_late_fault_at_the_end_of_the_stream_is_placed_as_jq_places_it_6455() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut opener = Vec::new();
    for i in 0..8_000 {
        opener.extend_from_slice(format!("{{\"id\":{i}}}\n").as_bytes());
    }
    opener.extend_from_slice(b"5[");
    let path = dir.path().join("opener.json");
    std::fs::write(&path, &opener)?;
    let opener = path.to_string_lossy().into_owned();
    let first = dir.path().join("first.json");
    std::fs::write(&first, "{\"id\":-1}\n")?;
    let first = first.to_string_lossy().into_owned();
    for (files, line) in [
        (vec![opener.as_str()], 8001),
        (vec![first.as_str(), &opener], 8002),
    ] {
        let mut args = vec!["-n", "[inputs]|length"];
        args.extend(&files);
        let (output, code) = spawn_jq(&args, None)?;
        assert_eq!(String::from_utf8(output.stdout)?, "", "{files:?}");
        assert_eq!(
            String::from_utf8(output.stderr)?,
            format!(
                "jq: error (at <unknown>): Unfinished JSON term at EOF at line {line}, column 2\n"
            ),
            "{files:?}"
        );
        assert_eq!(code, 5, "{files:?}");
    }
    Ok(())
}
