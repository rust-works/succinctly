//! #5936: jq reads each file's bytes in reads of their own, so a fault met in bytes that came from
//! more than one file (a file that is only an open tail hands it on to the next) drops the rest of
//! the read it was in, which ends at that file's end, not at the next newline of the file the bytes
//! now sit in. Every expected string was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

const MESSAGES: &str = "[limit(8; repeat(try input catch .))]";
const LINES: &str =
    "[limit(8; repeat(try (input | [., input_line_number]) catch [\"E\", input_line_number]))]";

fn run_files(filter: &str, files: &[&[u8]]) -> Result<(String, i32)> {
    let dir = tempfile::tempdir()?;
    let mut args = vec!["-nc".to_string(), filter.to_string()];
    for (i, bytes) in files.iter().enumerate() {
        let path = dir.path().join(format!("f{i}"));
        std::fs::write(&path, bytes)?;
        args.push(path.to_string_lossy().into_owned());
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (stdout, _, code) = run_jq_full(&args, None)?;
    Ok((stdout, code))
}

#[test]
fn a_fault_in_bytes_from_two_files_resumes_at_the_end_of_each_files_read_5936() -> Result<()> {
    // `[1,\n` and then `x<TAB>` both ride onto the third file, so the first fault is met in `x`'s
    // read, which ends at the end of the second file.
    let files: [&[u8]; 3] = [b"}\n[1,\n", b"x\t", b"[1,\nx  }}[}  "];
    for (filter, want) in [
        (MESSAGES, "[\"Unmatched '}' at line 1, column 1\",\"Invalid numeric literal at line 2, column 2\",\"Invalid numeric literal at line 3, column 2\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (LINES, "[[\"E\",1],[\"E\",0],[\"E\",1],[\"E\",0],[\"E\",0],[\"E\",0],[\"E\",0],[\"E\",0]]\n"),
    ] {
        assert_eq!(run_files(filter, &files)?, (want.to_string(), 0), "{filter}");
    }
    Ok(())
}

#[test]
fn a_chain_of_three_or_four_files_is_placed_as_jq_does_5936() -> Result<()> {
    // The open tail `[` rides through the second file, which holds a closed value and its own
    // open string `"` + `\`, onto the third and then the fourth.
    let files: [&[u8]; 4] = [b"[", b"\n]\t[\n\"", b"\\\n\"", b"a{] "];
    for (filter, want) in [
        (MESSAGES, "[[],\"Invalid escape at line 4, column 1\",\"Invalid numeric literal at line 4, column 3\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (LINES, "[[[],2],[\"E\",1],[\"E\",0],[\"E\",0],[\"E\",0],[\"E\",0],[\"E\",0],[\"E\",0]]\n"),
    ] {
        assert_eq!(run_files(filter, &files)?, (want.to_string(), 0), "{filter}");
    }
    // A string and a number cut across three files.
    let files: [&[u8]; 3] = [b"[\n\"\\\n\"a{", b"] 23nu", b"ll "];
    for (filter, want) in [
        (MESSAGES, "[\"Invalid escape at line 3, column 1\",\"Unmatched ']' at line 3, column 2\",\"Invalid numeric literal at line 3, column 5\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n"),
        (LINES, "[[\"E\",2],[\"E\",0],[\"E\",0],[\"E\",0],[\"E\",0],[\"E\",0],[\"E\",0],[\"E\",0]]\n"),
    ] {
        assert_eq!(run_files(filter, &files)?, (want.to_string(), 0), "{filter}");
    }
    Ok(())
}

/// An empty file between the open tail and the file that closes it hands on the same bytes, which
/// are one file's, so the read still ends at the end of that file, as it did before the chain was
/// modelled.
#[test]
fn an_empty_file_between_does_not_change_where_the_read_ends_5936() -> Result<()> {
    let want = "[{\"a\":1},\"Invalid numeric literal at line 1, column 11\",\"Invalid numeric literal at line 2, column 2\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n";
    for files in [
        [&b"{\"a\":1}  "[..], b"x\t", b"", b"[1,\nx }}"],
        [&b"{\"a\":1}  "[..], b"x\t", b"[1,\n", b"x }}"],
    ] {
        assert_eq!(
            run_files(MESSAGES, &files)?,
            (want.to_string(), 0),
            "{files:?}"
        );
    }
    Ok(())
}
