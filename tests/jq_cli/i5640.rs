//! #5640: under `-n --seq`, `input`/`inputs` raise the `--seq` reader's parse errors (a record cut
//! off at EOF, a number at EOF without its RS, a stream starting with a malformed BOM) where a plain
//! run only warns `jq: ignoring parse error: ...`. Each is raised behind the values read before it,
//! and the reads after it resume (#4311); under `-s` every one is raised before the slurped array.
//! The error names `<unknown>` instead of `file:line` when jq raises the EOF report in the read after
//! the one that returned the stream's last value. Rows, messages, locations and exit statuses were
//! captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `(stdin, filter, slurp, stdout, stderr, exit status)`.
const ROWS: &[(&str, &str, bool, &str, &str, i32)] = &[
    ("\x1e[1,\n", "[inputs]", false, "", "jq: error (at <stdin>:1): Unfinished JSON term at EOF at line 2, column 0\n", 5),
    ("\x1e1\n\x1e[1,\n", "[inputs]", false, "", "jq: error (at <stdin>:2): Unfinished JSON term at EOF at line 3, column 0\n", 5),
    ("\x1e{\n", "input", false, "", "jq: error (at <stdin>:1): Unfinished JSON term at EOF at line 2, column 0\n", 5),
    ("\x1e\"abc", "[inputs]", false, "", "jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 5\n", 5),
    ("\x1e1", "[inputs]", false, "", "jq: error (at <stdin>:0): Potentially truncated top-level numeric value at EOF at line 1, column 2\n", 5),
    ("\u{feff}1\n", "[inputs]", false, "", "jq: error (at <stdin>:1): Unfinished abandoned text at EOF at line 2, column 0\n", 5),
    ("\u{feff}", "[inputs]", false, "", "jq: error (at <stdin>:0): Unfinished abandoned text at EOF at line 1, column 0\n", 5),
    ("\x1e1\n\x1e}\n\x1e2\n", "[limit(5;repeat(try input catch \"E\"))]", false, "\x1e[1,\"E\",2,\"E\",\"E\"]\n", "", 0),
    ("\x1e1\n\x1e}\n\x1e2\n", "input,input,input", false, "\x1e1\n", "jq: error (at <stdin>:2): Unmatched '}' at line 2, column 2 (need RS to resync)\n", 5),
    ("\x1e1\n\x1e{\"a\":}\n\x1e3\n", "[limit(5;repeat(try input catch .))]", false, "\x1e[1,\"Unmatched '}' at line 2, column 7 (need RS to resync)\",3,\"break\",\"break\"]\n", "", 0),
    ("\x1e1\n\x1e[1,\n", "[.,input]", false, "\x1e[null,1]\n", "", 0),
    ("\x1e1[", "[inputs]", false, "", "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 3\n", 5),
    ("\x1e1", "[limit(3;repeat(try input catch .))]", false, "\x1e[\"Potentially truncated top-level numeric value at EOF at line 1, column 2\",\"break\",\"break\"]\n", "", 0),
    ("\x1e1\n\x1e}\n\x1e2\n\x1e]\n\x1e3\n", "[limit(6;repeat(try input catch \"E\"))]", true, "\x1e[\"E\",\"E\",[1,2,3],\"E\",\"E\",\"E\"]\n", "", 0),
    ("\x1e1\n\x1e}\n\x1e2\n", "[inputs|.+1]", true, "", "jq: error (at <stdin>:2): Unmatched '}' at line 2, column 2 (need RS to resync)\n", 5),
    ("\x1e1\n\x1e[1,\n", "[limit(4;repeat(try input catch .))]", true, "\x1e[\"Unfinished JSON term at EOF at line 3, column 0\",[1],\"break\",\"break\"]\n", "", 0),
    ("\x1e\"abc", "[inputs]", true, "", "jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 5\n", 5),
    ("\u{feff}1\n", "[inputs]", true, "", "jq: error (at <stdin>:1): Unfinished abandoned text at EOF at line 2, column 0\n", 5),
    ("\x1e1 \x1e", "[inputs]", false, "\x1e[1]\n", "", 0),
    ("\x1e1 \x1e", "[inputs]", true, "\x1e[[1]]\n", "", 0),
];

#[test]
fn test_null_input_seq_inputs_raise_the_readers_parse_errors_5640() -> Result<()> {
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
