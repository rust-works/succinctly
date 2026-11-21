//! #7568: with `-` among the input files, a run that stops early leaves a regular file on standard
//! input where jq 1.7.1 leaves it: untouched when jq's reader never got to `-` (the last item it
//! handed out came from a file before it), at the end of the `fgets` chunk that completed the last
//! item when that item came from standard input, and at its end when the run read past it. It used
//! to be left at its end whatever the run took. jq's reader takes the files as one text, so a value
//! a file leaves unfinished is completed by the next one (`1` then `2\n` is `12`), and a close that
//! a file's last byte completes right after a scalar's `--stream` event is handed out only once the
//! reader has read the next file's first chunk (#7570).
//!
//! Every row was captured from `/usr/bin/jq` 1.7.1 (and agrees with jq 1.8.2), reading the shared
//! descriptor's offset back after the run; standard input holds three lines `{"id":N}`, and `F` is a
//! file holding `NAMED[i]`. Rows where jq meets a parse error are left out (that shape is not
//! restored).

#![cfg(unix)]

use std::io::{Seek, SeekFrom, Write};
use std::process::{Command, Stdio};

const STDIN: &[u8] = b"{\"id\":0}\n{\"id\":1}\n{\"id\":2}\n";

const NAMED: &[&[u8]] = &[
    b"{\"a\":1}\n",
    b"{\"a\":1}",
    b"",
    b"1",
    b"[2]",
    b"[2]\n",
    b"\"x\"\n\"y\"\n",
];

const ROWS: &[(usize, &[&str], u64)] = &[
    (0, &["-c", "-n", "input", "F", "-"], 0),
    (0, &["-c", "-n", "first(inputs)", "F", "-"], 0),
    (0, &["-c", "-n", "[limit(2; inputs)]", "F", "-"], 9),
    (0, &["-c", "-n", "[limit(4; inputs)]", "F", "-"], 27),
    (0, &["-c", "halt", "F", "-"], 0),
    (
        0,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "F",
            "-",
        ],
        9,
    ),
    (
        0,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "F", "-"],
        0,
    ),
    (
        0,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "F", "-"],
        0,
    ),
    (
        0,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "F", "-"],
        9,
    ),
    (0, &["-c", "-R", "-n", "input", "F", "-"], 0),
    (0, &["-c", "-R", "-n", "[limit(2; inputs)]", "F", "-"], 9),
    (0, &["-c", "-n", "input", "-", "F"], 9),
    (0, &["-c", "-n", "first(inputs)", "-", "F"], 9),
    (0, &["-c", "-n", "[limit(2; inputs)]", "-", "F"], 18),
    (0, &["-c", "-n", "[limit(4; inputs)]", "-", "F"], 27),
    (0, &["-c", "halt", "-", "F"], 9),
    (
        0,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "-",
            "F",
        ],
        9,
    ),
    (
        0,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "-", "F"],
        9,
    ),
    (
        0,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "-", "F"],
        9,
    ),
    (
        0,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "-", "F"],
        18,
    ),
    (0, &["-c", "-R", "-n", "input", "-", "F"], 9),
    (0, &["-c", "-R", "-n", "[limit(2; inputs)]", "-", "F"], 18),
    (0, &["-c", "-n", "input", "-", "-"], 9),
    (0, &["-c", "-n", "first(inputs)", "-", "-"], 9),
    (0, &["-c", "-n", "[limit(2; inputs)]", "-", "-"], 18),
    (0, &["-c", "-n", "[limit(4; inputs)]", "-", "-"], 27),
    (0, &["-c", "halt", "-", "-"], 9),
    (
        0,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "-",
            "-",
        ],
        9,
    ),
    (
        0,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "-", "-"],
        9,
    ),
    (
        0,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "-", "-"],
        9,
    ),
    (
        0,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "-", "-"],
        18,
    ),
    (0, &["-c", "-R", "-n", "input", "-", "-"], 9),
    (0, &["-c", "-R", "-n", "[limit(2; inputs)]", "-", "-"], 18),
    (0, &["-c", "-n", "input", "F", "F", "-"], 0),
    (0, &["-c", "-n", "first(inputs)", "F", "F", "-"], 0),
    (0, &["-c", "-n", "[limit(2; inputs)]", "F", "F", "-"], 0),
    (0, &["-c", "-n", "[limit(4; inputs)]", "F", "F", "-"], 18),
    (0, &["-c", "halt", "F", "F", "-"], 0),
    (
        0,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "F",
            "F",
            "-",
        ],
        9,
    ),
    (
        0,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "F", "F", "-"],
        0,
    ),
    (
        0,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "F", "F", "-"],
        0,
    ),
    (
        0,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "F", "F", "-"],
        0,
    ),
    (0, &["-c", "-R", "-n", "input", "F", "F", "-"], 0),
    (
        0,
        &["-c", "-R", "-n", "[limit(2; inputs)]", "F", "F", "-"],
        0,
    ),
    (0, &["-c", "-n", "input", "-"], 9),
    (0, &["-c", "-n", "first(inputs)", "-"], 9),
    (0, &["-c", "-n", "[limit(2; inputs)]", "-"], 18),
    (0, &["-c", "-n", "[limit(4; inputs)]", "-"], 27),
    (0, &["-c", "halt", "-"], 9),
    (
        0,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "-",
        ],
        9,
    ),
    (0, &["-c", "--stream", "-n", "[limit(1; inputs)]", "-"], 9),
    (0, &["-c", "--stream", "-n", "[limit(2; inputs)]", "-"], 9),
    (0, &["-c", "--stream", "-n", "[limit(3; inputs)]", "-"], 18),
    (0, &["-c", "-R", "-n", "input", "-"], 9),
    (0, &["-c", "-R", "-n", "[limit(2; inputs)]", "-"], 18),
    (1, &["-c", "-n", "input", "F", "-"], 0),
    (1, &["-c", "-n", "first(inputs)", "F", "-"], 0),
    (1, &["-c", "-n", "[limit(2; inputs)]", "F", "-"], 9),
    (1, &["-c", "-n", "[limit(4; inputs)]", "F", "-"], 27),
    (1, &["-c", "halt", "F", "-"], 0),
    (
        1,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "F",
            "-",
        ],
        9,
    ),
    (
        1,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "F", "-"],
        0,
    ),
    (
        1,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "F", "-"],
        9,
    ),
    (
        1,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "F", "-"],
        9,
    ),
    (1, &["-c", "-R", "-n", "input", "F", "-"], 9),
    (1, &["-c", "-R", "-n", "[limit(2; inputs)]", "F", "-"], 18),
    (1, &["-c", "-n", "input", "-", "F"], 9),
    (1, &["-c", "-n", "first(inputs)", "-", "F"], 9),
    (1, &["-c", "-n", "[limit(2; inputs)]", "-", "F"], 18),
    (1, &["-c", "-n", "[limit(4; inputs)]", "-", "F"], 27),
    (1, &["-c", "halt", "-", "F"], 9),
    (
        1,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "-",
            "F",
        ],
        9,
    ),
    (
        1,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "-", "F"],
        9,
    ),
    (
        1,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "-", "F"],
        9,
    ),
    (
        1,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "-", "F"],
        18,
    ),
    (1, &["-c", "-R", "-n", "input", "-", "F"], 9),
    (1, &["-c", "-R", "-n", "[limit(2; inputs)]", "-", "F"], 18),
    (1, &["-c", "-n", "input", "F", "F", "-"], 0),
    (1, &["-c", "-n", "first(inputs)", "F", "F", "-"], 0),
    (1, &["-c", "-n", "[limit(2; inputs)]", "F", "F", "-"], 0),
    (1, &["-c", "-n", "[limit(4; inputs)]", "F", "F", "-"], 18),
    (1, &["-c", "halt", "F", "F", "-"], 0),
    (
        1,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "F",
            "F",
            "-",
        ],
        9,
    ),
    (
        1,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "F", "F", "-"],
        0,
    ),
    (
        1,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "F", "F", "-"],
        0,
    ),
    (
        1,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "F", "F", "-"],
        0,
    ),
    (1, &["-c", "-R", "-n", "input", "F", "F", "-"], 9),
    (
        1,
        &["-c", "-R", "-n", "[limit(2; inputs)]", "F", "F", "-"],
        18,
    ),
    (2, &["-c", "-n", "input", "F", "-"], 9),
    (2, &["-c", "-n", "first(inputs)", "F", "-"], 9),
    (2, &["-c", "-n", "[limit(2; inputs)]", "F", "-"], 18),
    (2, &["-c", "-n", "[limit(4; inputs)]", "F", "-"], 27),
    (2, &["-c", "halt", "F", "-"], 9),
    (
        2,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "F",
            "-",
        ],
        9,
    ),
    (
        2,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "F", "-"],
        9,
    ),
    (
        2,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "F", "-"],
        9,
    ),
    (
        2,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "F", "-"],
        18,
    ),
    (2, &["-c", "-R", "-n", "input", "F", "-"], 9),
    (2, &["-c", "-R", "-n", "[limit(2; inputs)]", "F", "-"], 18),
    (2, &["-c", "-n", "input", "-", "F"], 9),
    (2, &["-c", "-n", "first(inputs)", "-", "F"], 9),
    (2, &["-c", "-n", "[limit(2; inputs)]", "-", "F"], 18),
    (2, &["-c", "-n", "[limit(4; inputs)]", "-", "F"], 27),
    (2, &["-c", "halt", "-", "F"], 9),
    (
        2,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "-",
            "F",
        ],
        9,
    ),
    (
        2,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "-", "F"],
        9,
    ),
    (
        2,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "-", "F"],
        9,
    ),
    (
        2,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "-", "F"],
        18,
    ),
    (2, &["-c", "-R", "-n", "input", "-", "F"], 9),
    (2, &["-c", "-R", "-n", "[limit(2; inputs)]", "-", "F"], 18),
    (2, &["-c", "-n", "input", "F", "F", "-"], 9),
    (2, &["-c", "-n", "first(inputs)", "F", "F", "-"], 9),
    (2, &["-c", "-n", "[limit(2; inputs)]", "F", "F", "-"], 18),
    (2, &["-c", "-n", "[limit(4; inputs)]", "F", "F", "-"], 27),
    (2, &["-c", "halt", "F", "F", "-"], 9),
    (
        2,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "F",
            "F",
            "-",
        ],
        9,
    ),
    (
        2,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "F", "F", "-"],
        9,
    ),
    (
        2,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "F", "F", "-"],
        9,
    ),
    (
        2,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "F", "F", "-"],
        18,
    ),
    (2, &["-c", "-R", "-n", "input", "F", "F", "-"], 9),
    (
        2,
        &["-c", "-R", "-n", "[limit(2; inputs)]", "F", "F", "-"],
        18,
    ),
    (3, &["-c", "-n", "input", "F", "-"], 9),
    (3, &["-c", "-n", "first(inputs)", "F", "-"], 9),
    (3, &["-c", "-n", "[limit(2; inputs)]", "F", "-"], 9),
    (3, &["-c", "-n", "[limit(4; inputs)]", "F", "-"], 27),
    (3, &["-c", "halt", "F", "-"], 9),
    (
        3,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "F",
            "-",
        ],
        9,
    ),
    (
        3,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "F", "-"],
        9,
    ),
    (
        3,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "F", "-"],
        9,
    ),
    (
        3,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "F", "-"],
        9,
    ),
    (3, &["-c", "-R", "-n", "input", "F", "-"], 9),
    (3, &["-c", "-R", "-n", "[limit(2; inputs)]", "F", "-"], 18),
    (3, &["-c", "-n", "input", "-", "F"], 9),
    (3, &["-c", "-n", "first(inputs)", "-", "F"], 9),
    (3, &["-c", "-n", "[limit(2; inputs)]", "-", "F"], 18),
    (3, &["-c", "-n", "[limit(4; inputs)]", "-", "F"], 27),
    (3, &["-c", "halt", "-", "F"], 9),
    (
        3,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "-",
            "F",
        ],
        9,
    ),
    (
        3,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "-", "F"],
        9,
    ),
    (
        3,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "-", "F"],
        9,
    ),
    (
        3,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "-", "F"],
        18,
    ),
    (3, &["-c", "-R", "-n", "input", "-", "F"], 9),
    (3, &["-c", "-R", "-n", "[limit(2; inputs)]", "-", "F"], 18),
    (3, &["-c", "-n", "input", "F", "F", "-"], 9),
    (3, &["-c", "-n", "first(inputs)", "F", "F", "-"], 9),
    (3, &["-c", "-n", "[limit(2; inputs)]", "F", "F", "-"], 9),
    (3, &["-c", "-n", "[limit(4; inputs)]", "F", "F", "-"], 27),
    (3, &["-c", "halt", "F", "F", "-"], 9),
    (
        3,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "F",
            "F",
            "-",
        ],
        9,
    ),
    (
        3,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "F", "F", "-"],
        9,
    ),
    (
        3,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "F", "F", "-"],
        9,
    ),
    (
        3,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "F", "F", "-"],
        9,
    ),
    (3, &["-c", "-R", "-n", "input", "F", "F", "-"], 9),
    (
        3,
        &["-c", "-R", "-n", "[limit(2; inputs)]", "F", "F", "-"],
        18,
    ),
    (4, &["-c", "-n", "input", "F", "-"], 0),
    (4, &["-c", "-n", "first(inputs)", "F", "-"], 0),
    (4, &["-c", "-n", "[limit(2; inputs)]", "F", "-"], 9),
    (4, &["-c", "-n", "[limit(4; inputs)]", "F", "-"], 27),
    (4, &["-c", "halt", "F", "-"], 0),
    (
        4,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "F",
            "-",
        ],
        0,
    ),
    (
        4,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "F", "-"],
        0,
    ),
    (
        4,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "F", "-"],
        9,
    ),
    (
        4,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "F", "-"],
        9,
    ),
    (4, &["-c", "-R", "-n", "input", "F", "-"], 9),
    (4, &["-c", "-R", "-n", "[limit(2; inputs)]", "F", "-"], 18),
    (4, &["-c", "-n", "input", "-", "F"], 9),
    (4, &["-c", "-n", "first(inputs)", "-", "F"], 9),
    (4, &["-c", "-n", "[limit(2; inputs)]", "-", "F"], 18),
    (4, &["-c", "-n", "[limit(4; inputs)]", "-", "F"], 27),
    (4, &["-c", "halt", "-", "F"], 9),
    (
        4,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "-",
            "F",
        ],
        9,
    ),
    (
        4,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "-", "F"],
        9,
    ),
    (
        4,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "-", "F"],
        9,
    ),
    (
        4,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "-", "F"],
        18,
    ),
    (4, &["-c", "-R", "-n", "input", "-", "F"], 9),
    (4, &["-c", "-R", "-n", "[limit(2; inputs)]", "-", "F"], 18),
    (4, &["-c", "-n", "input", "F", "F", "-"], 0),
    (4, &["-c", "-n", "first(inputs)", "F", "F", "-"], 0),
    (4, &["-c", "-n", "[limit(2; inputs)]", "F", "F", "-"], 0),
    (4, &["-c", "-n", "[limit(4; inputs)]", "F", "F", "-"], 18),
    (4, &["-c", "halt", "F", "F", "-"], 0),
    (
        4,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "F",
            "F",
            "-",
        ],
        0,
    ),
    (
        4,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "F", "F", "-"],
        0,
    ),
    (
        4,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "F", "F", "-"],
        0,
    ),
    (
        4,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "F", "F", "-"],
        0,
    ),
    (4, &["-c", "-R", "-n", "input", "F", "F", "-"], 9),
    (
        4,
        &["-c", "-R", "-n", "[limit(2; inputs)]", "F", "F", "-"],
        18,
    ),
    (5, &["-c", "-n", "input", "F", "-"], 0),
    (5, &["-c", "-n", "first(inputs)", "F", "-"], 0),
    (5, &["-c", "-n", "[limit(2; inputs)]", "F", "-"], 9),
    (5, &["-c", "-n", "[limit(4; inputs)]", "F", "-"], 27),
    (5, &["-c", "halt", "F", "-"], 0),
    (
        5,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "F",
            "-",
        ],
        0,
    ),
    (
        5,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "F", "-"],
        0,
    ),
    (
        5,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "F", "-"],
        0,
    ),
    (
        5,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "F", "-"],
        9,
    ),
    (5, &["-c", "-R", "-n", "input", "F", "-"], 0),
    (5, &["-c", "-R", "-n", "[limit(2; inputs)]", "F", "-"], 9),
    (5, &["-c", "-n", "input", "-", "F"], 9),
    (5, &["-c", "-n", "first(inputs)", "-", "F"], 9),
    (5, &["-c", "-n", "[limit(2; inputs)]", "-", "F"], 18),
    (5, &["-c", "-n", "[limit(4; inputs)]", "-", "F"], 27),
    (5, &["-c", "halt", "-", "F"], 9),
    (
        5,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "-",
            "F",
        ],
        9,
    ),
    (
        5,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "-", "F"],
        9,
    ),
    (
        5,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "-", "F"],
        9,
    ),
    (
        5,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "-", "F"],
        18,
    ),
    (5, &["-c", "-R", "-n", "input", "-", "F"], 9),
    (5, &["-c", "-R", "-n", "[limit(2; inputs)]", "-", "F"], 18),
    (5, &["-c", "-n", "input", "F", "F", "-"], 0),
    (5, &["-c", "-n", "first(inputs)", "F", "F", "-"], 0),
    (5, &["-c", "-n", "[limit(2; inputs)]", "F", "F", "-"], 0),
    (5, &["-c", "-n", "[limit(4; inputs)]", "F", "F", "-"], 18),
    (5, &["-c", "halt", "F", "F", "-"], 0),
    (
        5,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "F",
            "F",
            "-",
        ],
        0,
    ),
    (
        5,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "F", "F", "-"],
        0,
    ),
    (
        5,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "F", "F", "-"],
        0,
    ),
    (
        5,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "F", "F", "-"],
        0,
    ),
    (5, &["-c", "-R", "-n", "input", "F", "F", "-"], 0),
    (
        5,
        &["-c", "-R", "-n", "[limit(2; inputs)]", "F", "F", "-"],
        0,
    ),
    (6, &["-c", "-n", "input", "F", "-"], 0),
    (6, &["-c", "-n", "first(inputs)", "F", "-"], 0),
    (6, &["-c", "-n", "[limit(2; inputs)]", "F", "-"], 0),
    (6, &["-c", "-n", "[limit(4; inputs)]", "F", "-"], 18),
    (6, &["-c", "halt", "F", "-"], 0),
    (
        6,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "F",
            "-",
        ],
        0,
    ),
    (
        6,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "F", "-"],
        0,
    ),
    (
        6,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "F", "-"],
        0,
    ),
    (
        6,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "F", "-"],
        9,
    ),
    (6, &["-c", "-R", "-n", "input", "F", "-"], 0),
    (6, &["-c", "-R", "-n", "[limit(2; inputs)]", "F", "-"], 0),
    (6, &["-c", "-n", "input", "-", "F"], 9),
    (6, &["-c", "-n", "first(inputs)", "-", "F"], 9),
    (6, &["-c", "-n", "[limit(2; inputs)]", "-", "F"], 18),
    (6, &["-c", "-n", "[limit(4; inputs)]", "-", "F"], 27),
    (6, &["-c", "halt", "-", "F"], 9),
    (
        6,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "-",
            "F",
        ],
        9,
    ),
    (
        6,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "-", "F"],
        9,
    ),
    (
        6,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "-", "F"],
        9,
    ),
    (
        6,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "-", "F"],
        18,
    ),
    (6, &["-c", "-R", "-n", "input", "-", "F"], 9),
    (6, &["-c", "-R", "-n", "[limit(2; inputs)]", "-", "F"], 18),
    (6, &["-c", "-n", "input", "F", "F", "-"], 0),
    (6, &["-c", "-n", "first(inputs)", "F", "F", "-"], 0),
    (6, &["-c", "-n", "[limit(2; inputs)]", "F", "F", "-"], 0),
    (6, &["-c", "-n", "[limit(4; inputs)]", "F", "F", "-"], 0),
    (6, &["-c", "halt", "F", "F", "-"], 0),
    (
        6,
        &[
            "-c",
            "if . == 1 or . == {\"a\":1} then empty else halt end",
            "F",
            "F",
            "-",
        ],
        0,
    ),
    (
        6,
        &["-c", "--stream", "-n", "[limit(1; inputs)]", "F", "F", "-"],
        0,
    ),
    (
        6,
        &["-c", "--stream", "-n", "[limit(2; inputs)]", "F", "F", "-"],
        0,
    ),
    (
        6,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "F", "F", "-"],
        0,
    ),
    (6, &["-c", "-R", "-n", "input", "F", "F", "-"], 0),
    (
        6,
        &["-c", "-R", "-n", "[limit(2; inputs)]", "F", "F", "-"],
        0,
    ),
];

/// Where `succinctly jq ARGS` leaves standard input (holding `stdin`, from `offset`), with `F` in
/// `args` naming a file holding `named`, and its exit status.
fn offset_after(stdin: &[u8], offset: u64, named: &[u8], args: &[&str]) -> (u64, i32) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("F");
    std::fs::write(&path, named).expect("written");
    let mut file = tempfile::tempfile().expect("a temporary file");
    file.write_all(stdin).expect("written");
    file.seek(SeekFrom::Start(offset)).expect("seeked");
    let status = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args.iter().map(|&arg| {
            if arg == "F" {
                path.as_os_str()
            } else {
                arg.as_ref()
            }
        }))
        .stdin(Stdio::from(file.try_clone().expect("a duplicate")))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("ran");
    (
        file.stream_position().expect("the offset"),
        status.code().unwrap_or(-1),
    )
}

#[test]
fn a_dash_among_the_input_files_leaves_stdin_where_jq_does_7568() {
    let mut failures = Vec::new();
    for &(named, args, want) in ROWS {
        let (got, status) = offset_after(STDIN, 0, NAMED[named], args);
        if (got, status) != (want, 0) {
            failures.push(format!(
                "{args:?} over {:?}: {got} (exit {status}), jq {want}",
                String::from_utf8_lossy(NAMED[named])
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} rows: {failures:#?}",
        failures.len()
    );
}

#[test]
fn a_dash_after_a_file_that_does_not_open_and_from_an_offset_7568() {
    // Captured from jq 1.7.1: the missing file is reported (exit 2), and standard input is read
    // from where it stood.
    let (got, status) = offset_after(STDIN, 0, b"", &["-c", "-n", "input", "no-such-file", "-"]);
    assert_eq!((got, status), (9, 2));
    assert_eq!(
        offset_after(STDIN, 9, b"", &["-c", "-n", "input", "-"]),
        (18, 0)
    );
    assert_eq!(
        offset_after(STDIN, 9, b"1\n", &["-c", "-n", "input", "F", "-"]),
        (9, 0)
    );
    assert_eq!(
        offset_after(STDIN, 9, b"1\n", &["-c", "halt", "-", "F"]),
        (18, 0)
    );
}
