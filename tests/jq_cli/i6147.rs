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

const W2: &str = "jq: ignoring parse error: Expected another array element at line 2, column 5 (need RS to resync)\n";

/// `(flags, stdin, filter, stdout, stderr, exit status)`: the shapes around the queue route. A clean
/// stream has no fault to order and keeps #5674's route; `-e` carries the exit status through the
/// main loop; `-s` warns as it reads; `-R` takes the stream over from the `--seq` reader.
type FlagRow = (
    &'static [&'static str],
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    i32,
);

const FLAG_ROWS: &[FlagRow] = &[
    (&["-c", "--seq"], "\x1e{\"a\":1}\n\x1e{\"a\":2}\n", ".a", "\x1e1\n\x1e2\n", "", 0),
    (&["-c", "-e", "--seq"], "\x1e{\"a\":null}\n\x1e{\"a\":false}\n", ".a", "\x1enull\n\x1efalse\n", "", 1),
    (&["-c", "-e", "--seq"], "\x1e1\n\x1e[1,]\n\x1e2\n", "if . == 1 then halt else . end", "", "", 0),
    (&["-c", "-e", "--seq"], "\x1e1\n\x1e[1,]\n\x1e2\n", "if . == 2 then error(\"E\") else . end", "\x1e1\n", "jq: ignoring parse error: Expected another array element at line 2, column 5 (need RS to resync)\njq: error (at <stdin>:3): E\n", 5),
    (&["-c", "-s", "--seq"], "\x1e1\n\x1e[1,]\n\x1e2\n", ".", "\x1e[1,2]\n", W2, 0),
    (&["-c", "-R", "--seq"], "\x1e1\n\x1e[1,]\n\x1e2\n", ".", "\x1e\"\\u001e1\"\n\x1e\"\\u001e[1,]\"\n\x1e\"\\u001e2\"\n", "", 0),
];

#[test]
fn test_seq_warnings_hold_across_flags_and_a_clean_stream_6147() -> Result<()> {
    for (flags, stdin, filter, stdout, stderr, code) in FLAG_ROWS {
        let mut args = flags.to_vec();
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
