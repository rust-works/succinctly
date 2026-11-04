//! #6147: under `--seq` without `-n` or `-s`, the main loop prints the reader's parse errors as
//! ignored when it reaches them, after the values before it, for every program and not only one
//! that reads `input`/`inputs` (#6137): `halt` on an earlier value suppresses the warnings behind
//! it, and a runtime error on an earlier value comes ahead of the warning. Rows, messages and exit
//! statuses were captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `(stdin, filter, stdout, stderr, exit status)`.
const ROWS: &[(&str, &str, &str, &str, i32)] = &[
    ("\x1e1\n\x1e[1,]\n\x1e2\n", "if . == 1 then halt else . end", "", "", 0),
    ("\x1e1\n\x1e[1,]\n\x1e2\n", "if . == 1 then error(\"E\") else . end", "\x1e2\n", "jq: error (at <stdin>:1): E\njq: ignoring parse error: Expected another array element at line 2, column 5 (need RS to resync)\n", 0),
    ("\x1e1\n\x1e[1,]\n\x1e2\n", ".", "\x1e1\n\x1e2\n", "jq: ignoring parse error: Expected another array element at line 2, column 5 (need RS to resync)\n", 0),
    ("\x1e[1,]\n\x1e2\n", ".", "\x1e2\n", "jq: ignoring parse error: Expected another array element at line 1, column 5 (need RS to resync)\n", 0),
    ("\x1e1\n\x1e2\n\x1e[1,]\n", ".", "\x1e1\n\x1e2\n", "jq: ignoring parse error: Expected another array element at line 3, column 5 (need RS to resync)\n", 0),
    ("\x1e1\n\x1e[1,]\n\x1e[2,]\n\x1e3\n", "if . == 1 then error(\"E\") else . end", "\x1e3\n", "jq: error (at <stdin>:1): E\njq: ignoring parse error: Expected another array element at line 2, column 5 (need RS to resync)\njq: ignoring parse error: Expected another array element at line 3, column 5 (need RS to resync)\n", 0),
    ("\x1e1\n\x1e[1,]\n\x1e2\n\x1e[,]\n\x1e3", "if . == 2 then halt else . end", "\x1e1\n", "jq: ignoring parse error: Expected another array element at line 2, column 5 (need RS to resync)\n", 0),
    ("\x1e1\n\x1e[1,]\n\x1e2\n", "input_line_number", "\x1e1\n\x1e3\n", "jq: ignoring parse error: Expected another array element at line 2, column 5 (need RS to resync)\n", 0),
    ("\x1e1\n\x1e[1,]\n\x1e2\n", "if . == 2 then halt_error else . end", "\x1e1\n", "jq: ignoring parse error: Expected another array element at line 2, column 5 (need RS to resync)\n2\n", 5),
];

#[test]
fn test_seq_warnings_come_out_as_the_main_loop_meets_them_6147() -> Result<()> {
    for (stdin, filter, stdout, stderr, code) in ROWS {
        let args = ["-c", "--seq", filter];
        let (out, err, status) = run_jq_full(&args, Some(stdin))?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (*stdout, *stderr, *code),
            "{args:?} {stdin:?}"
        );
    }
    Ok(())
}
