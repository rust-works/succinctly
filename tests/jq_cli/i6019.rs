//! #6019: `-n -s` reads on the first `input`, as jq does, so a fault in the stream is an input of
//! its own and not the result of the whole run. Each fault is the error of one call, all of them
//! ahead of the array, which holds every value read (the ones after a fault too); a program that
//! never asks for an input prints its result. Every expectation was captured from jq 1.7.1
//! (`/usr/bin/jq`); the file names in stderr are shown as `<fN>`.

use anyhow::Result;

use crate::run_jq_over_byte_files;

/// `(files, program, stdout, stderr, exit code)`, run as `-nc -s`.
type Row = (
    &'static [&'static [u8]],
    &'static str,
    &'static str,
    &'static str,
    i32,
);

#[test]
fn a_fault_in_a_slurped_stream_is_raised_by_the_input_that_reaches_it_6019() -> Result<()> {
    let rows: &[Row] = &[
        (&[b"1 2 }"], "input", "", "jq: error (at <f0>:0): Unmatched '}' at line 1, column 5\n", 5),
        (&[b"1 2 }"], "[inputs]", "", "jq: error (at <f0>:0): Unmatched '}' at line 1, column 5\n", 5),
        (&[b"1 2 }"], "try input catch \"E\"", "\"E\"\n", "", 0),
        (&[b"1 2 }"], "input_line_number", "0\n", "", 0),
        (&[b"1 2 }"], "[limit(4;repeat(try input catch .))]", "[\"Unmatched '}' at line 1, column 5\",[1,2],\"break\",\"break\"]\n", "", 0),
        (&[b"1 2 }"], "1", "1\n", "", 0),
        (&[b"1 2 }"], "(try input catch null), input", "null\n[1,2]\n", "", 0),
        (&[b"1 2 }"], "[inputs]|length", "", "jq: error (at <f0>:0): Unmatched '}' at line 1, column 5\n", 5),
        (&[b"1 [2,"], "input", "", "jq: error (at <f0>:0): Unfinished JSON term at EOF at line 1, column 5\n", 5),
        (&[b"1 [2,"], "[inputs]", "", "jq: error (at <f0>:0): Unfinished JSON term at EOF at line 1, column 5\n", 5),
        (&[b"1 [2,"], "try input catch \"E\"", "\"E\"\n", "", 0),
        (&[b"1 [2,"], "input_line_number", "0\n", "", 0),
        (&[b"1 [2,"], "[limit(4;repeat(try input catch .))]", "[\"Unfinished JSON term at EOF at line 1, column 5\",[1],\"break\",\"break\"]\n", "", 0),
        (&[b"1 [2,"], "1", "1\n", "", 0),
        (&[b"1 [2,"], "(try input catch null), input", "null\n[1]\n", "", 0),
        (&[b"1 [2,"], "[inputs]|length", "", "jq: error (at <f0>:0): Unfinished JSON term at EOF at line 1, column 5\n", 5),
        (&[b"1 2 }\n3"], "input", "", "jq: error (at <f0>:1): Unmatched '}' at line 1, column 5\n", 5),
        (&[b"1 2 }\n3"], "[inputs]", "", "jq: error (at <f0>:1): Unmatched '}' at line 1, column 5\n", 5),
        (&[b"1 2 }\n3"], "try input catch \"E\"", "\"E\"\n", "", 0),
        (&[b"1 2 }\n3"], "input_line_number", "0\n", "", 0),
        (&[b"1 2 }\n3"], "[limit(4;repeat(try input catch .))]", "[\"Unmatched '}' at line 1, column 5\",[1,2,3],\"break\",\"break\"]\n", "", 0),
        (&[b"1 2 }\n3"], "1", "1\n", "", 0),
        (&[b"1 2 }\n3"], "(try input catch null), input", "null\n[1,2,3]\n", "", 0),
        (&[b"1 2 }\n3"], "[inputs]|length", "", "jq: error (at <f0>:1): Unmatched '}' at line 1, column 5\n", 5),
        (&[b"1 }\n2 }\n3"], "input", "", "jq: error (at <f0>:1): Unmatched '}' at line 1, column 3\n", 5),
        (&[b"1 }\n2 }\n3"], "[inputs]", "", "jq: error (at <f0>:1): Unmatched '}' at line 1, column 3\n", 5),
        (&[b"1 }\n2 }\n3"], "try input catch \"E\"", "\"E\"\n", "", 0),
        (&[b"1 }\n2 }\n3"], "input_line_number", "0\n", "", 0),
        (&[b"1 }\n2 }\n3"], "[limit(4;repeat(try input catch .))]", "[\"Unmatched '}' at line 1, column 3\",\"Unmatched '}' at line 1, column 6\",[1,2,3],\"break\"]\n", "", 0),
        (&[b"1 }\n2 }\n3"], "1", "1\n", "", 0),
        (&[b"1 }\n2 }\n3"], "(try input catch null), input", "null\n", "jq: error (at <f0>:2): Unmatched '}' at line 1, column 6\n", 5),
        (&[b"1 }\n2 }\n3"], "[inputs]|length", "", "jq: error (at <f0>:1): Unmatched '}' at line 1, column 3\n", 5),
        (&[b"}"], "input", "", "jq: error (at <f0>:0): Unmatched '}' at line 1, column 1\n", 5),
        (&[b"}"], "[inputs]", "", "jq: error (at <f0>:0): Unmatched '}' at line 1, column 1\n", 5),
        (&[b"}"], "try input catch \"E\"", "\"E\"\n", "", 0),
        (&[b"}"], "input_line_number", "0\n", "", 0),
        (&[b"}"], "[limit(4;repeat(try input catch .))]", "[\"Unmatched '}' at line 1, column 1\",[],\"break\",\"break\"]\n", "", 0),
        (&[b"}"], "1", "1\n", "", 0),
        (&[b"}"], "(try input catch null), input", "null\n[]\n", "", 0),
        (&[b"}"], "[inputs]|length", "", "jq: error (at <f0>:0): Unmatched '}' at line 1, column 1\n", 5),
        (&[b"1 2"], "input", "[1,2]\n", "", 0),
        (&[b"1 2"], "[inputs]", "[[1,2]]\n", "", 0),
        (&[b"1 2"], "try input catch \"E\"", "[1,2]\n", "", 0),
        (&[b"1 2"], "input_line_number", "0\n", "", 0),
        (&[b"1 2"], "[limit(4;repeat(try input catch .))]", "[[1,2],\"break\",\"break\",\"break\"]\n", "", 0),
        (&[b"1 2"], "1", "1\n", "", 0),
        (&[b"1 2"], "(try input catch null), input", "[1,2]\n", "jq: error (at <unknown>): break\n", 5),
        (&[b"1 2"], "[inputs]|length", "1\n", "", 0),
        (&[b""], "input", "[]\n", "", 0),
        (&[b""], "[inputs]", "[[]]\n", "", 0),
        (&[b""], "try input catch \"E\"", "[]\n", "", 0),
        (&[b""], "input_line_number", "0\n", "", 0),
        (&[b""], "[limit(4;repeat(try input catch .))]", "[[],\"break\",\"break\",\"break\"]\n", "", 0),
        (&[b""], "1", "1\n", "", 0),
        (&[b""], "(try input catch null), input", "[]\n", "jq: error (at <unknown>): break\n", 5),
        (&[b""], "[inputs]|length", "1\n", "", 0),
        (&[b"1 2", b"3 }"], "input", "", "jq: error (at <f1>:0): Unmatched '}' at line 1, column 6\n", 5),
        (&[b"1 2", b"3 }"], "[inputs]", "", "jq: error (at <f1>:0): Unmatched '}' at line 1, column 6\n", 5),
        (&[b"1 2", b"3 }"], "try input catch \"E\"", "\"E\"\n", "", 0),
        (&[b"1 2", b"3 }"], "input_line_number", "0\n", "", 0),
        (&[b"1 2", b"3 }"], "[limit(4;repeat(try input catch .))]", "[\"Unmatched '}' at line 1, column 6\",[1,23],\"break\",\"break\"]\n", "", 0),
        (&[b"1 2", b"3 }"], "1", "1\n", "", 0),
        (&[b"1 2", b"3 }"], "(try input catch null), input", "null\n[1,23]\n", "", 0),
        (&[b"1 2", b"3 }"], "[inputs]|length", "", "jq: error (at <f1>:0): Unmatched '}' at line 1, column 6\n", 5),
        (&[b"1 2 }", b"3 }"], "input", "", "jq: error (at <f0>:0): Unmatched '}' at line 1, column 5\n", 5),
        (&[b"1 2 }", b"3 }"], "[inputs]", "", "jq: error (at <f0>:0): Unmatched '}' at line 1, column 5\n", 5),
        (&[b"1 2 }", b"3 }"], "try input catch \"E\"", "\"E\"\n", "", 0),
        (&[b"1 2 }", b"3 }"], "input_line_number", "0\n", "", 0),
        (&[b"1 2 }", b"3 }"], "[limit(4;repeat(try input catch .))]", "[\"Unmatched '}' at line 1, column 5\",\"Unmatched '}' at line 1, column 8\",[1,2,3],\"break\"]\n", "", 0),
        (&[b"1 2 }", b"3 }"], "1", "1\n", "", 0),
        (&[b"1 2 }", b"3 }"], "(try input catch null), input", "null\n", "jq: error (at <f1>:0): Unmatched '}' at line 1, column 8\n", 5),
        (&[b"1 2 }", b"3 }"], "[inputs]|length", "", "jq: error (at <f0>:0): Unmatched '}' at line 1, column 5\n", 5),
        (&[b"1 }\n", b"2\n", b"}"], "input", "", "jq: error (at <f0>:1): Unmatched '}' at line 1, column 3\n", 5),
        (&[b"1 }\n", b"2\n", b"}"], "[inputs]", "", "jq: error (at <f0>:1): Unmatched '}' at line 1, column 3\n", 5),
        (&[b"1 }\n", b"2\n", b"}"], "try input catch \"E\"", "\"E\"\n", "", 0),
        (&[b"1 }\n", b"2\n", b"}"], "input_line_number", "0\n", "", 0),
        (&[b"1 }\n", b"2\n", b"}"], "[limit(4;repeat(try input catch .))]", "[\"Unmatched '}' at line 1, column 3\",\"Unmatched '}' at line 2, column 1\",[1,2],\"break\"]\n", "", 0),
        (&[b"1 }\n", b"2\n", b"}"], "1", "1\n", "", 0),
        (&[b"1 }\n", b"2\n", b"}"], "(try input catch null), input", "null\n", "jq: error (at <f2>:0): Unmatched '}' at line 2, column 1\n", 5),
        (&[b"1 }\n", b"2\n", b"}"], "[inputs]|length", "", "jq: error (at <f0>:1): Unmatched '}' at line 1, column 3\n", 5),
    ];
    for &(files, program, stdout, stderr, code) in rows {
        let (got_out, got_err, got_code, paths) =
            run_jq_over_byte_files(&["-nc", "-s", program], files)?;
        let mut got_err = got_err;
        for (i, path) in paths.iter().enumerate() {
            got_err = got_err.replace(path.as_str(), &format!("<f{i}>"));
        }
        assert_eq!(
            (got_out.as_str(), got_err.as_str(), got_code),
            (stdout, stderr, code),
            "{files:?} | {program}"
        );
    }
    Ok(())
}
