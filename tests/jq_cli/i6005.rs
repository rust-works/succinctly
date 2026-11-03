//! #6005: `-n -s --stream` reads on the first `input`, as jq does, so a fault in the stream is an
//! input of its own and not the result of the whole run. A fault in the middle of the stream is
//! raised by the first `input` and the array of the events before it (and after it, since jq goes on
//! with the next line) comes with the next one; a fault at the end of the stream comes after the
//! array. A program that never asks for an input prints its result. Every expectation was captured
//! from jq 1.7.1 (`/usr/bin/jq`); the file names in stderr are shown as `<fN>`.

use anyhow::Result;

use crate::run_jq_over_byte_files;

/// `(files, program, stdout, stderr, exit code)`, run as `-nc --stream -s`.
type Row = (
    &'static [&'static [u8]],
    &'static str,
    &'static str,
    &'static str,
    i32,
);

#[test]
fn a_fault_in_a_slurped_stream_is_raised_by_the_input_that_reaches_it_6005() -> Result<()> {
    let rows: &[Row] = &[
        (&[b"1 2 }"], "input", "", "jq: error (at <f0>:0): Unmatched '}' at the top-level at line 1, column 5\n", 5),
        (&[b"1 2 }"], "[inputs]", "", "jq: error (at <f0>:0): Unmatched '}' at the top-level at line 1, column 5\n", 5),
        (&[b"1 2 }"], "try input catch \"E\"", "\"E\"\n", "", 0),
        (&[b"1 2 }"], "input_line_number", "0\n", "", 0),
        (&[b"1 2 }"], "[limit(4;repeat(try input catch .))]", "[\"Unmatched '}' at the top-level at line 1, column 5\",[[[],1],[[],2]],\"break\",\"break\"]\n", "", 0),
        (&[b"1 2 }"], "1", "1\n", "", 0),
        (&[b"1 2 }"], "input_filename", "null\n", "", 0),
        (&[b"1 [2,"], "input", "[[[],1],[[0],2]]\n", "", 0),
        (&[b"1 [2,"], "[inputs]", "", "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 5\n", 5),
        (&[b"1 [2,"], "try input catch \"E\"", "[[[],1],[[0],2]]\n", "", 0),
        (&[b"1 [2,"], "input_line_number", "0\n", "", 0),
        (&[b"1 [2,"], "[limit(4;repeat(try input catch .))]", "[[[[],1],[[0],2]],\"Unfinished JSON term at EOF at line 1, column 5\",\"break\",\"break\"]\n", "", 0),
        (&[b"1 [2,"], "1", "1\n", "", 0),
        (&[b"1 [2,"], "input_filename", "null\n", "", 0),
        (&[b"1 2 3 }\n4"], "input", "", "jq: error (at <f0>:1): Unmatched '}' at the top-level at line 1, column 7\n", 5),
        (&[b"1 2 3 }\n4"], "[inputs]", "", "jq: error (at <f0>:1): Unmatched '}' at the top-level at line 1, column 7\n", 5),
        (&[b"1 2 3 }\n4"], "try input catch \"E\"", "\"E\"\n", "", 0),
        (&[b"1 2 3 }\n4"], "input_line_number", "0\n", "", 0),
        (&[b"1 2 3 }\n4"], "[limit(4;repeat(try input catch .))]", "[\"Unmatched '}' at the top-level at line 1, column 7\",[[[],1],[[],2],[[],3],[[],4]],\"break\",\"break\"]\n", "", 0),
        (&[b"1 2 3 }\n4"], "1", "1\n", "", 0),
        (&[b"1 2 3 }\n4"], "input_filename", "null\n", "", 0),
        (&[b"1\n2\n}\n3"], "input", "", "jq: error (at <f0>:3): Unmatched '}' at the top-level at line 3, column 1\n", 5),
        (&[b"1\n2\n}\n3"], "[inputs]", "", "jq: error (at <f0>:3): Unmatched '}' at the top-level at line 3, column 1\n", 5),
        (&[b"1\n2\n}\n3"], "try input catch \"E\"", "\"E\"\n", "", 0),
        (&[b"1\n2\n}\n3"], "input_line_number", "0\n", "", 0),
        (&[b"1\n2\n}\n3"], "[limit(4;repeat(try input catch .))]", "[\"Unmatched '}' at the top-level at line 3, column 1\",[[[],1],[[],2],[[],3]],\"break\",\"break\"]\n", "", 0),
        (&[b"1\n2\n}\n3"], "1", "1\n", "", 0),
        (&[b"1\n2\n}\n3"], "input_filename", "null\n", "", 0),
        (&[b"1 }\n2 }\n3"], "input", "", "jq: error (at <f0>:1): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&[b"1 }\n2 }\n3"], "[inputs]", "", "jq: error (at <f0>:1): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&[b"1 }\n2 }\n3"], "try input catch \"E\"", "\"E\"\n", "", 0),
        (&[b"1 }\n2 }\n3"], "input_line_number", "0\n", "", 0),
        (&[b"1 }\n2 }\n3"], "[limit(4;repeat(try input catch .))]", "[\"Unmatched '}' at the top-level at line 1, column 3\",\"Unmatched '}' at the top-level at line 1, column 6\",[[[],1],[[],2],[[],3]],\"break\"]\n", "", 0),
        (&[b"1 }\n2 }\n3"], "1", "1\n", "", 0),
        (&[b"1 }\n2 }\n3"], "input_filename", "null\n", "", 0),
        (&[b"}", b"1"], "input", "", "jq: error (at <f0>:0): Unmatched '}' at the top-level at line 1, column 1\n", 5),
        (&[b"}", b"1"], "[inputs]", "", "jq: error (at <f0>:0): Unmatched '}' at the top-level at line 1, column 1\n", 5),
        (&[b"}", b"1"], "try input catch \"E\"", "\"E\"\n", "", 0),
        (&[b"}", b"1"], "input_line_number", "0\n", "", 0),
        (&[b"}", b"1"], "[limit(4;repeat(try input catch .))]", "[\"Unmatched '}' at the top-level at line 1, column 1\",[[[],1]],\"break\",\"break\"]\n", "", 0),
        (&[b"}", b"1"], "1", "1\n", "", 0),
        (&[b"}", b"1"], "input_filename", "null\n", "", 0),
        (&[b"1 2", b"3 }"], "input", "", "jq: error (at <f1>:0): Unmatched '}' at the top-level at line 1, column 6\n", 5),
        (&[b"1 2", b"3 }"], "[inputs]", "", "jq: error (at <f1>:0): Unmatched '}' at the top-level at line 1, column 6\n", 5),
        (&[b"1 2", b"3 }"], "try input catch \"E\"", "\"E\"\n", "", 0),
        (&[b"1 2", b"3 }"], "input_line_number", "0\n", "", 0),
        (&[b"1 2", b"3 }"], "[limit(4;repeat(try input catch .))]", "[\"Unmatched '}' at the top-level at line 1, column 6\",[[[],1],[[],23]],\"break\",\"break\"]\n", "", 0),
        (&[b"1 2", b"3 }"], "1", "1\n", "", 0),
        (&[b"1 2", b"3 }"], "input_filename", "null\n", "", 0),
        (&[b"1 2", b"[3,"], "input", "[[[],1],[[],2],[[0],3]]\n", "", 0),
        (&[b"1 2", b"[3,"], "[inputs]", "", "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 6\n", 5),
        (&[b"1 2", b"[3,"], "try input catch \"E\"", "[[[],1],[[],2],[[0],3]]\n", "", 0),
        (&[b"1 2", b"[3,"], "input_line_number", "0\n", "", 0),
        (&[b"1 2", b"[3,"], "[limit(4;repeat(try input catch .))]", "[[[[],1],[[],2],[[0],3]],\"Unfinished JSON term at EOF at line 1, column 6\",\"break\",\"break\"]\n", "", 0),
        (&[b"1 2", b"[3,"], "1", "1\n", "", 0),
        (&[b"1 2", b"[3,"], "input_filename", "null\n", "", 0),
        (&[b"[1,2] [3"], "input", "", "jq: error (at <f0>:0): Unfinished JSON term at EOF at line 1, column 8\n", 5),
        (&[b"[1,2] [3"], "[inputs]", "", "jq: error (at <f0>:0): Unfinished JSON term at EOF at line 1, column 8\n", 5),
        (&[b"[1,2] [3"], "try input catch \"E\"", "\"E\"\n", "", 0),
        (&[b"[1,2] [3"], "input_line_number", "0\n", "", 0),
        (&[b"[1,2] [3"], "[limit(4;repeat(try input catch .))]", "[\"Unfinished JSON term at EOF at line 1, column 8\",[[[0],1],[[1],2],[[1]]],\"break\",\"break\"]\n", "", 0),
        (&[b"[1,2] [3"], "1", "1\n", "", 0),
        (&[b"[1,2] [3"], "input_filename", "null\n", "", 0),
        (&[b"1 2 3"], "input", "[[[],1],[[],2],[[],3]]\n", "", 0),
        (&[b"1 2 3"], "[inputs]", "[[[[],1],[[],2],[[],3]]]\n", "", 0),
        (&[b"1 2 3"], "try input catch \"E\"", "[[[],1],[[],2],[[],3]]\n", "", 0),
        (&[b"1 2 3"], "input_line_number", "0\n", "", 0),
        (&[b"1 2 3"], "[limit(4;repeat(try input catch .))]", "[[[[],1],[[],2],[[],3]],\"break\",\"break\",\"break\"]\n", "", 0),
        (&[b"1 2 3"], "1", "1\n", "", 0),
        (&[b"1 2 3"], "input_filename", "null\n", "", 0),
        (&[b""], "input", "[]\n", "", 0),
        (&[b""], "[inputs]", "[[]]\n", "", 0),
        (&[b""], "try input catch \"E\"", "[]\n", "", 0),
        (&[b""], "input_line_number", "0\n", "", 0),
        (&[b""], "[limit(4;repeat(try input catch .))]", "[[],\"break\",\"break\",\"break\"]\n", "", 0),
        (&[b""], "1", "1\n", "", 0),
        (&[b""], "input_filename", "null\n", "", 0),
    ];
    for &(files, program, stdout, stderr, code) in rows {
        let (got_out, got_err, got_code, paths) =
            run_jq_over_byte_files(&["-nc", "--stream", "-s", program], files)?;
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
