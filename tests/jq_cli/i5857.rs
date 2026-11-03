//! #5857: `-n` with `input`/`inputs` over a clean plain-JSON stream of several files converts each
//! document when the program reads it, as one file does (#5661). What the program sees is what
//! it saw when the whole stream was read first, and what jq 1.7.1 answers (expected values
//! captured from `/usr/bin/jq`): each document, the line and file `input_line_number` and
//! `input_filename` name after it, a token that runs on from one file into the next, an empty
//! or white-space-only file, and the `(at ...)` marker an error names once the stream is read
//! to its end.

use anyhow::Result;

use crate::run_jq_over_byte_files;

const ALL: &str = "[inputs|[., input_line_number, (input_filename|split(\"/\")|last)]]";
const END: &str = "[inputs], (try input catch .), input_line_number, (input_filename|if . == null then \"null\" else split(\"/\")|last end)";
const FIRST: &str = "first(inputs), input_line_number";
const TWO: &str = "[input, input_line_number, (input_filename|split(\"/\")|last), input, input_line_number, (input_filename|split(\"/\")|last)]";
const MARK: &str = "[inputs]|error(\"x\")";

/// `(files, program, stdout, stderr, exit code)`; the files are named `f0.json`, `f1.json`, ...
type Row = (
    &'static [&'static [u8]],
    &'static str,
    &'static str,
    &'static str,
    i32,
);

const ROWS: &[Row] = &[
        (&[b"1\n2\n", b"3\n4\n"], ALL, "[[1,1,\"f0.json\"],[2,2,\"f0.json\"],[3,1,\"f1.json\"],[4,2,\"f1.json\"]]\n", "", 0),
        (&[b"1\n2\n", b"3\n4\n"], END, "[1,2,3,4]\n\"break\"\n0\n\"null\"\n", "", 0),
        (&[b"[1,\n2", b"]\n3\n"], ALL, "[[[1,2],1,\"f1.json\"],[3,2,\"f1.json\"]]\n", "", 0),
        (&[b"12", b"3\n4\n"], ALL, "[[123,1,\"f1.json\"],[4,2,\"f1.json\"]]\n", "", 0),
        (&[b"[1,\n2", b"]\n3"], END, "[[1,2],3]\n\"break\"\n0\n\"null\"\n", "", 0),
        (&[b"1\n", b"", b"2\n"], ALL, "[[1,1,\"f0.json\"],[2,1,\"f2.json\"]]\n", "", 0),
        (&[b"1\n2\n", b""], END, "[1,2]\n\"break\"\n0\n\"null\"\n", "", 0),
        (&[b"1\n2\n", b"  \n"], END, "[1,2]\n\"break\"\n0\n\"null\"\n", "", 0),
        (&[b"", b"1\n", b"\n\n2\n"], ALL, "[[1,1,\"f1.json\"],[2,3,\"f2.json\"]]\n", "", 0),
        (&[b"1\n", b"2\n", b"3\n"], FIRST, "1\n1\n", "", 0),
        (&[b"{\"a\":\n1}\n", b"\"b\"\n[\n2]\n"], ALL, "[[{\"a\":1},2,\"f0.json\"],[\"b\",1,\"f1.json\"],[[2],3,\"f1.json\"]]\n", "", 0),
        (&[b"1 2", b" 3\n4 5\n6", b"7\n"], ALL, "[[1,0,\"f0.json\"],[2,1,\"f1.json\"],[3,1,\"f1.json\"],[4,2,\"f1.json\"],[5,2,\"f1.json\"],[67,1,\"f2.json\"]]\n", "", 0),
        (&[b"\"a", b"b\"\n1\n"], ALL, "[[\"ab\",1,\"f1.json\"],[1,2,\"f1.json\"]]\n", "", 0),
        (&[b"1\n2\n", b"3\n4\n"], TWO, "[1,1,\"f0.json\",2,2,\"f0.json\"]\n", "", 0),
        (&[b"1\n2\n", b"3\n", b"4\n"], END, "[1,2,3,4]\n\"break\"\n0\n\"null\"\n", "", 0),
        (&[b"1\n", b"2\n", b"}\n"], ALL, "", "jq: error (at f2.json:1): Unmatched '}' at line 3, column 1\n", 5),
        (&[b"1\n", b"2"], END, "[1,2]\n\"break\"\n0\n\"null\"\n", "", 0),
        (&[b"1\n", b"2 "], END, "[1,2]\n\"break\"\n0\n\"null\"\n", "", 0),
        (&[b"1\n2\n", b"3\n4\n"], MARK, "", "jq: error (at f1.json:2): x\n", 5),
        (&[b"1\n2\n", b"3\n4"], MARK, "", "jq: error (at <unknown>): x\n", 5),
        (&[b"1\n2\n", b"3\n4 "], MARK, "", "jq: error (at <unknown>): x\n", 5),
        (&[b"[1,\n2", b"]\n3\n"], MARK, "", "jq: error (at f1.json:2): x\n", 5),
        (&[b"12", b"3\n\n\n"], MARK, "", "jq: error (at f1.json:3): x\n", 5),
        (&[b"1", b"2"], MARK, "", "jq: error (at <unknown>): x\n", 5),
        (&[b"1\n", b""], MARK, "", "jq: error (at f1.json:0): x\n", 5),
        (&[b"1\n", b"\n\n", b""], MARK, "", "jq: error (at f2.json:0): x\n", 5),
        (&[b"1", b"", b""], MARK, "", "jq: error (at <unknown>): x\n", 5),
];

#[test]
fn a_stream_of_several_files_read_under_null_input_matches_jq_5857() -> Result<()> {
    for (files, program, want_out, want_err, want_code) in ROWS {
        let (stdout, stderr, code, paths) = run_jq_over_byte_files(&["-nc", program], files)?;
        // The temporary names are random; the rows name the files by their position, in the
        // path an error prints and in the base name `input_filename|split("/")|last` gives.
        let name = |path: &String, index: usize| (path.clone(), format!("f{index}.json"));
        let (mut stdout, mut stderr) = (stdout, stderr);
        for (index, path) in paths.iter().enumerate() {
            let (path, renamed) = name(path, index);
            stderr = stderr.replace(&path, &renamed);
            let base = path.rsplit('/').next().unwrap_or(&path);
            stdout = stdout.replace(base, &renamed);
        }
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (*want_out, *want_err, *want_code),
            "{files:?} {program}"
        );
    }
    Ok(())
}
