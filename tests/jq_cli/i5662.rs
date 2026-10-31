//! #5662: an empty `--seq` record (`\x1e` then whitespace, or `\x1e\x1e`) is not the end of the stream
//! for `-n`'s `input`/`inputs`. jq's main loop stops there silently (#2998), but `input` reads through
//! the same driver and raises `break` -- the error an exhausted stream raises -- and the next read
//! scans on. `inputs` ends at that `break` (a following `inputs` resumes), so `[inputs]` is `[]` while a
//! `try input catch .` loop sees `"break"` and then the values behind the record. Rows, messages and
//! exit statuses were captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `(stdin, filter, slurp, stdout, stderr, exit status)`.
const ROWS: &[(&str, &str, bool, &str, &str, i32)] = &[
    ("\x1e\x1e[1]", "[limit(5;repeat(try input catch .))]", false, "\x1e[\"break\",[1],\"break\",\"break\",\"break\"]\n", "", 0),
    ("\x1e\x1e[1]", "[inputs]", false, "\x1e[]\n", "", 0),
    ("\x1e\x1e[1]", "[inputs],[inputs]", false, "\x1e[]\n\x1e[[1]]\n", "", 0),
    ("\x1e\x1e[1]", "input,input", false, "", "jq: error (at <stdin>:0): break\n", 5),
    ("\x1e\x1e2  \"x\"", "[limit(5;repeat(try input catch .))]", false, "\x1e[\"break\",2,\"x\",\"break\",\"break\"]\n", "", 0),
    ("\x1e\x1e2  \"x\"", "[inputs]", false, "\x1e[]\n", "", 0),
    ("\x1e\x1e2  \"x\"", "[inputs],[inputs]", false, "\x1e[]\n\x1e[2,\"x\"]\n", "", 0),
    ("\x1e\x1e2  \"x\"", "input,input", false, "", "jq: error (at <stdin>:0): break\n", 5),
    ("\x1e\n\x1e[1][1]", "[limit(5;repeat(try input catch .))]", false, "\x1e[\"break\",[1],[1],\"break\",\"break\"]\n", "", 0),
    ("\x1e\n\x1e[1][1]", "[inputs]", false, "\x1e[]\n", "", 0),
    ("\x1e\n\x1e[1][1]", "[inputs],[inputs]", false, "\x1e[]\n\x1e[[1],[1]]\n", "", 0),
    ("\x1e\n\x1e[1][1]", "input,input", false, "", "jq: error (at <stdin>:1): break\n", 5),
    ("\x1e1\x1e\x1e2\n", "[limit(5;repeat(try input catch .))]", false, "\x1e[\"Potentially truncated top-level numeric value at line 1, column 3\",2,\"break\",\"break\",\"break\"]\n", "", 0),
    ("\x1e1\x1e\x1e2\n", "[inputs]", false, "", "jq: error (at <stdin>:1): Potentially truncated top-level numeric value at line 1, column 3\n", 5),
    ("\x1e1\x1e\x1e2\n", "[inputs],[inputs]", false, "", "jq: error (at <stdin>:1): Potentially truncated top-level numeric value at line 1, column 3\n", 5),
    ("\x1e1\x1e\x1e2\n", "input,input", false, "", "jq: error (at <stdin>:1): Potentially truncated top-level numeric value at line 1, column 3\n", 5),
    ("\x1e1\n\x1e \n\x1e2\n", "[limit(5;repeat(try input catch .))]", false, "\x1e[1,2,\"break\",\"break\",\"break\"]\n", "", 0),
    ("\x1e1\n\x1e \n\x1e2\n", "[inputs]", false, "\x1e[1,2]\n", "", 0),
    ("\x1e1\n\x1e \n\x1e2\n", "[inputs],[inputs]", false, "\x1e[1,2]\n\x1e[]\n", "", 0),
    ("\x1e1\n\x1e \n\x1e2\n", "input,input", false, "\x1e1\n\x1e2\n", "", 0),
    ("\x1e\x1e", "[limit(5;repeat(try input catch .))]", false, "\x1e[\"break\",\"break\",\"break\",\"break\",\"break\"]\n", "", 0),
    ("\x1e\x1e", "[inputs]", false, "\x1e[]\n", "", 0),
    ("\x1e\x1e", "[inputs],[inputs]", false, "\x1e[]\n\x1e[]\n", "", 0),
    ("\x1e\x1e", "input,input", false, "", "jq: error (at <stdin>:0): break\n", 5),
    ("\x1e\x1e1\x1e\x1e2\x1e\x1e3\n", "[limit(5;repeat(try input catch .))]", false, "\x1e[\"Potentially truncated top-level numeric value at line 1, column 4\",\"Potentially truncated top-level numeric value at line 1, column 7\",3,\"break\",\"break\"]\n", "", 0),
    ("\x1e\x1e1\x1e\x1e2\x1e\x1e3\n", "[inputs]", false, "", "jq: error (at <stdin>:1): Potentially truncated top-level numeric value at line 1, column 4\n", 5),
    ("\x1e\x1e1\x1e\x1e2\x1e\x1e3\n", "[inputs],[inputs]", false, "", "jq: error (at <stdin>:1): Potentially truncated top-level numeric value at line 1, column 4\n", 5),
    ("\x1e\x1e1\x1e\x1e2\x1e\x1e3\n", "input,input", false, "", "jq: error (at <stdin>:1): Potentially truncated top-level numeric value at line 1, column 4\n", 5),
    ("\x1e\x1e[1]", "[inputs]", true, "\x1e[[[1]]]\n", "", 0),
    ("\x1e1\x1e\x1e2\n", "[limit(4;repeat(try input catch .))]", true, "\x1e[\"Potentially truncated top-level numeric value at line 1, column 3\",[2],\"break\",\"break\"]\n", "", 0),
];

#[test]
fn test_null_input_seq_reads_on_past_an_empty_record_5662() -> Result<()> {
    for (stdin, filter, slurp, stdout, stderr, code) in ROWS {
        let mut args = vec!["-cn", "--seq"];
        if *slurp {
            args.push("-s");
        }
        args.push(filter);
        let (out, err, status) = run_jq_full(&args, Some(stdin))?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (*stdout, *stderr, *code),
            "{args:?} {stdin:?}"
        );
    }
    Ok(())
}
