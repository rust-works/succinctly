//! #7569: after a UTF-8 byte-order mark at the front of the input, `input_line_number` and the
//! `(at FILE:N)` of a runtime error place jq's reads where jq 1.7.1 places them. jq's `fgets` reads
//! 4,095 bytes at a time and the mark is three of the first read's bytes, though its parser skips
//! them, so on a first line longer than 4,092 bytes the read that holds the line break, which raises
//! the count, starts 3 bytes earlier in the text than it does without the mark. Every route counted
//! the reads from the text after the mark, 3 bytes late. A mark cut across the first files (#7566)
//! puts each of its bytes in the first read of the file that holds it.
//!
//! Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`). jq 1.8.2 reads 4,091 bytes at a time
//! (the #7455 paragraph of `docs/compliance/jq/limitations.md`), so its counts on a long line differ on
//! most rows; that is a change of the read size, and 1.7.1 stays the reference.

use anyhow::Result;

use crate::small_window;
use crate::spawn_jq;

/// (bytes of the first line after the mark, how the mark is cut across files, flags, jq's stdout as
/// `<count>x<line>` runs, stderr, exit status).
/// One row: the length, the split, the flags, stdout's runs, stderr and the exit status.
type Row = (
    usize,
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
    i32,
);

const ROWS: &[Row] = &[
    (
        4091,
        "whole",
        &["-c", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4091,
        "whole",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[1,2046],[2,1]]",
        "",
        0,
    ),
    (
        4091,
        "whole",
        &["-c", "--stream", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4091,
        "whole",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[1,2046],[2,1]]",
        "",
        0,
    ),
    (
        4091,
        "whole",
        &["-c", "--stream-errors", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4091,
        "whole",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f0.json:2) (not a string): 2\n",
        5,
    ),
    (
        4091,
        "ef|bbbf",
        &["-c", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4091,
        "ef|bbbf",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[1,2046],[2,1]]",
        "",
        0,
    ),
    (
        4091,
        "ef|bbbf",
        &["-c", "--stream", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4091,
        "ef|bbbf",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[1,2046],[2,1]]",
        "",
        0,
    ),
    (
        4091,
        "ef|bbbf",
        &["-c", "--stream-errors", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4091,
        "ef|bbbf",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f1.json:2) (not a string): 2\n",
        5,
    ),
    (
        4091,
        "efbb|bf",
        &["-c", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4091,
        "efbb|bf",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[1,2046],[2,1]]",
        "",
        0,
    ),
    (
        4091,
        "efbb|bf",
        &["-c", "--stream", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4091,
        "efbb|bf",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[1,2046],[2,1]]",
        "",
        0,
    ),
    (
        4091,
        "efbb|bf",
        &["-c", "--stream-errors", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4091,
        "efbb|bf",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f1.json:2) (not a string): 2\n",
        5,
    ),
    (
        4091,
        "ef|bb|bf",
        &["-c", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4091,
        "ef|bb|bf",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[1,2046],[2,1]]",
        "",
        0,
    ),
    (
        4091,
        "ef|bb|bf",
        &["-c", "--stream", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4091,
        "ef|bb|bf",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[1,2046],[2,1]]",
        "",
        0,
    ),
    (
        4091,
        "ef|bb|bf",
        &["-c", "--stream-errors", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4091,
        "ef|bb|bf",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f2.json:2) (not a string): 2\n",
        5,
    ),
    (
        4092,
        "whole",
        &["-c", "input_line_number"],
        "2046x0 1x2",
        "",
        0,
    ),
    (
        4092,
        "whole",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,2046],[2,1]]",
        "",
        0,
    ),
    (
        4092,
        "whole",
        &["-c", "--stream", "input_line_number"],
        "2046x0 1x2",
        "",
        0,
    ),
    (
        4092,
        "whole",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,2046],[2,1]]",
        "",
        0,
    ),
    (
        4092,
        "whole",
        &["-c", "--stream-errors", "input_line_number"],
        "2046x0 1x2",
        "",
        0,
    ),
    (
        4092,
        "whole",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f0.json:2) (not a string): 2\n",
        5,
    ),
    (
        4092,
        "ef|bbbf",
        &["-c", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4092,
        "ef|bbbf",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[1,2046],[2,1]]",
        "",
        0,
    ),
    (
        4092,
        "ef|bbbf",
        &["-c", "--stream", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4092,
        "ef|bbbf",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[1,2046],[2,1]]",
        "",
        0,
    ),
    (
        4092,
        "ef|bbbf",
        &["-c", "--stream-errors", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4092,
        "ef|bbbf",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f1.json:2) (not a string): 2\n",
        5,
    ),
    (
        4092,
        "efbb|bf",
        &["-c", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4092,
        "efbb|bf",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[1,2046],[2,1]]",
        "",
        0,
    ),
    (
        4092,
        "efbb|bf",
        &["-c", "--stream", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4092,
        "efbb|bf",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[1,2046],[2,1]]",
        "",
        0,
    ),
    (
        4092,
        "efbb|bf",
        &["-c", "--stream-errors", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4092,
        "efbb|bf",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f1.json:2) (not a string): 2\n",
        5,
    ),
    (
        4092,
        "ef|bb|bf",
        &["-c", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4092,
        "ef|bb|bf",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[1,2046],[2,1]]",
        "",
        0,
    ),
    (
        4092,
        "ef|bb|bf",
        &["-c", "--stream", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4092,
        "ef|bb|bf",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[1,2046],[2,1]]",
        "",
        0,
    ),
    (
        4092,
        "ef|bb|bf",
        &["-c", "--stream-errors", "input_line_number"],
        "2046x1 1x2",
        "",
        0,
    ),
    (
        4092,
        "ef|bb|bf",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f2.json:2) (not a string): 2\n",
        5,
    ),
    (
        4093,
        "whole",
        &["-c", "input_line_number"],
        "2046x0 1x1 1x2",
        "",
        0,
    ),
    (
        4093,
        "whole",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,2046],[1,1],[2,1]]",
        "",
        0,
    ),
    (
        4093,
        "whole",
        &["-c", "--stream", "input_line_number"],
        "2046x0 1x1 1x2",
        "",
        0,
    ),
    (
        4093,
        "whole",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,2046],[1,1],[2,1]]",
        "",
        0,
    ),
    (
        4093,
        "whole",
        &["-c", "--stream-errors", "input_line_number"],
        "2046x0 1x1 1x2",
        "",
        0,
    ),
    (
        4093,
        "whole",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f0.json:2) (not a string): 2\n",
        5,
    ),
    (
        4093,
        "ef|bbbf",
        &["-c", "input_line_number"],
        "2046x0 1x1 1x2",
        "",
        0,
    ),
    (
        4093,
        "ef|bbbf",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,2046],[1,1],[2,1]]",
        "",
        0,
    ),
    (
        4093,
        "ef|bbbf",
        &["-c", "--stream", "input_line_number"],
        "2046x0 1x1 1x2",
        "",
        0,
    ),
    (
        4093,
        "ef|bbbf",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,2046],[1,1],[2,1]]",
        "",
        0,
    ),
    (
        4093,
        "ef|bbbf",
        &["-c", "--stream-errors", "input_line_number"],
        "2046x0 1x1 1x2",
        "",
        0,
    ),
    (
        4093,
        "ef|bbbf",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f1.json:2) (not a string): 2\n",
        5,
    ),
    (
        4093,
        "efbb|bf",
        &["-c", "input_line_number"],
        "2047x1 1x2",
        "",
        0,
    ),
    (
        4093,
        "efbb|bf",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[1,2047],[2,1]]",
        "",
        0,
    ),
    (
        4093,
        "efbb|bf",
        &["-c", "--stream", "input_line_number"],
        "2047x1 1x2",
        "",
        0,
    ),
    (
        4093,
        "efbb|bf",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[1,2047],[2,1]]",
        "",
        0,
    ),
    (
        4093,
        "efbb|bf",
        &["-c", "--stream-errors", "input_line_number"],
        "2047x1 1x2",
        "",
        0,
    ),
    (
        4093,
        "efbb|bf",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f1.json:2) (not a string): 2\n",
        5,
    ),
    (
        4093,
        "ef|bb|bf",
        &["-c", "input_line_number"],
        "2047x1 1x2",
        "",
        0,
    ),
    (
        4093,
        "ef|bb|bf",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[1,2047],[2,1]]",
        "",
        0,
    ),
    (
        4093,
        "ef|bb|bf",
        &["-c", "--stream", "input_line_number"],
        "2047x1 1x2",
        "",
        0,
    ),
    (
        4093,
        "ef|bb|bf",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[1,2047],[2,1]]",
        "",
        0,
    ),
    (
        4093,
        "ef|bb|bf",
        &["-c", "--stream-errors", "input_line_number"],
        "2047x1 1x2",
        "",
        0,
    ),
    (
        4093,
        "ef|bb|bf",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f2.json:2) (not a string): 2\n",
        5,
    ),
    (
        8188,
        "whole",
        &["-c", "input_line_number"],
        "4093x0 1x1 1x2",
        "",
        0,
    ),
    (
        8188,
        "whole",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,4093],[1,1],[2,1]]",
        "",
        0,
    ),
    (
        8188,
        "whole",
        &["-c", "--stream", "input_line_number"],
        "4093x0 1x1 1x2",
        "",
        0,
    ),
    (
        8188,
        "whole",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,4093],[1,1],[2,1]]",
        "",
        0,
    ),
    (
        8188,
        "whole",
        &["-c", "--stream-errors", "input_line_number"],
        "4093x0 1x1 1x2",
        "",
        0,
    ),
    (
        8188,
        "whole",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f0.json:2) (not a string): 2\n",
        5,
    ),
    (
        8188,
        "ef|bbbf",
        &["-c", "input_line_number"],
        "4094x0 1x2",
        "",
        0,
    ),
    (
        8188,
        "ef|bbbf",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,4094],[2,1]]",
        "",
        0,
    ),
    (
        8188,
        "ef|bbbf",
        &["-c", "--stream", "input_line_number"],
        "4094x0 1x2",
        "",
        0,
    ),
    (
        8188,
        "ef|bbbf",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,4094],[2,1]]",
        "",
        0,
    ),
    (
        8188,
        "ef|bbbf",
        &["-c", "--stream-errors", "input_line_number"],
        "4094x0 1x2",
        "",
        0,
    ),
    (
        8188,
        "ef|bbbf",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f1.json:2) (not a string): 2\n",
        5,
    ),
    (
        8188,
        "efbb|bf",
        &["-c", "input_line_number"],
        "2047x0 2047x1 1x2",
        "",
        0,
    ),
    (
        8188,
        "efbb|bf",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,2047],[1,2047],[2,1]]",
        "",
        0,
    ),
    (
        8188,
        "efbb|bf",
        &["-c", "--stream", "input_line_number"],
        "2047x0 2047x1 1x2",
        "",
        0,
    ),
    (
        8188,
        "efbb|bf",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,2047],[1,2047],[2,1]]",
        "",
        0,
    ),
    (
        8188,
        "efbb|bf",
        &["-c", "--stream-errors", "input_line_number"],
        "2047x0 2047x1 1x2",
        "",
        0,
    ),
    (
        8188,
        "efbb|bf",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f1.json:2) (not a string): 2\n",
        5,
    ),
    (
        8188,
        "ef|bb|bf",
        &["-c", "input_line_number"],
        "2047x0 2047x1 1x2",
        "",
        0,
    ),
    (
        8188,
        "ef|bb|bf",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,2047],[1,2047],[2,1]]",
        "",
        0,
    ),
    (
        8188,
        "ef|bb|bf",
        &["-c", "--stream", "input_line_number"],
        "2047x0 2047x1 1x2",
        "",
        0,
    ),
    (
        8188,
        "ef|bb|bf",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,2047],[1,2047],[2,1]]",
        "",
        0,
    ),
    (
        8188,
        "ef|bb|bf",
        &["-c", "--stream-errors", "input_line_number"],
        "2047x0 2047x1 1x2",
        "",
        0,
    ),
    (
        8188,
        "ef|bb|bf",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f2.json:2) (not a string): 2\n",
        5,
    ),
    (
        8189,
        "whole",
        &["-c", "input_line_number"],
        "4093x0 2x1 1x2",
        "",
        0,
    ),
    (
        8189,
        "whole",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,4093],[1,2],[2,1]]",
        "",
        0,
    ),
    (
        8189,
        "whole",
        &["-c", "--stream", "input_line_number"],
        "4093x0 2x1 1x2",
        "",
        0,
    ),
    (
        8189,
        "whole",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,4093],[1,2],[2,1]]",
        "",
        0,
    ),
    (
        8189,
        "whole",
        &["-c", "--stream-errors", "input_line_number"],
        "4093x0 2x1 1x2",
        "",
        0,
    ),
    (
        8189,
        "whole",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f0.json:2) (not a string): 2\n",
        5,
    ),
    (
        8189,
        "ef|bbbf",
        &["-c", "input_line_number"],
        "4094x0 1x1 1x2",
        "",
        0,
    ),
    (
        8189,
        "ef|bbbf",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,4094],[1,1],[2,1]]",
        "",
        0,
    ),
    (
        8189,
        "ef|bbbf",
        &["-c", "--stream", "input_line_number"],
        "4094x0 1x1 1x2",
        "",
        0,
    ),
    (
        8189,
        "ef|bbbf",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,4094],[1,1],[2,1]]",
        "",
        0,
    ),
    (
        8189,
        "ef|bbbf",
        &["-c", "--stream-errors", "input_line_number"],
        "4094x0 1x1 1x2",
        "",
        0,
    ),
    (
        8189,
        "ef|bbbf",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f1.json:2) (not a string): 2\n",
        5,
    ),
    (
        8189,
        "efbb|bf",
        &["-c", "input_line_number"],
        "4094x0 1x1 1x2",
        "",
        0,
    ),
    (
        8189,
        "efbb|bf",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,4094],[1,1],[2,1]]",
        "",
        0,
    ),
    (
        8189,
        "efbb|bf",
        &["-c", "--stream", "input_line_number"],
        "4094x0 1x1 1x2",
        "",
        0,
    ),
    (
        8189,
        "efbb|bf",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,4094],[1,1],[2,1]]",
        "",
        0,
    ),
    (
        8189,
        "efbb|bf",
        &["-c", "--stream-errors", "input_line_number"],
        "4094x0 1x1 1x2",
        "",
        0,
    ),
    (
        8189,
        "efbb|bf",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f1.json:2) (not a string): 2\n",
        5,
    ),
    (
        8189,
        "ef|bb|bf",
        &["-c", "input_line_number"],
        "4094x0 1x1 1x2",
        "",
        0,
    ),
    (
        8189,
        "ef|bb|bf",
        &[
            "-c",
            "-n",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,4094],[1,1],[2,1]]",
        "",
        0,
    ),
    (
        8189,
        "ef|bb|bf",
        &["-c", "--stream", "input_line_number"],
        "4094x0 1x1 1x2",
        "",
        0,
    ),
    (
        8189,
        "ef|bb|bf",
        &[
            "-c",
            "-n",
            "--stream",
            "[inputs|input_line_number]|group_by(.)|map([.[0],length])",
        ],
        "1x[[0,4094],[1,1],[2,1]]",
        "",
        0,
    ),
    (
        8189,
        "ef|bb|bf",
        &["-c", "--stream-errors", "input_line_number"],
        "4094x0 1x1 1x2",
        "",
        0,
    ),
    (
        8189,
        "ef|bb|bf",
        &["-c", "select(. > 1) | error"],
        "",
        "jq: error (at f2.json:2) (not a string): 2\n",
        5,
    ),
];

const BOM: &[u8] = b"\xef\xbb\xbf";

/// A first line of `len` bytes of `1 ` values, then a line break, `2` and a line break.
fn text(len: usize) -> Vec<u8> {
    let mut text: Vec<u8> = b"1 ".iter().copied().cycle().take(len).collect();
    text.extend_from_slice(b"\n2\n");
    text
}

/// The mark and `text` cut into files as `split` says.
fn files(split: &str, text: &[u8]) -> Vec<Vec<u8>> {
    let cuts: &[usize] = match split {
        "whole" => &[],
        "ef|bbbf" => &[1],
        "efbb|bf" => &[2],
        "ef|bb|bf" => &[1, 2],
        other => panic!("no split {other}"),
    };
    let mut stream = BOM.to_vec();
    stream.extend_from_slice(text);
    let mut files = Vec::new();
    let mut from = 0;
    for &cut in cuts {
        files.push(stream[from..cut].to_vec());
        from = cut;
    }
    files.push(stream[from..].to_vec());
    files
}

/// `stdout` as runs of equal lines, `<count>x<line>` each, the way the rows give it.
fn runs(stdout: &str) -> String {
    let mut runs: Vec<(usize, &str)> = Vec::new();
    for line in stdout.lines() {
        match runs.last_mut() {
            Some((count, last)) if *last == line => *count += 1,
            _ => runs.push((1, line)),
        }
    }
    runs.iter()
        .map(|(count, line)| format!("{count}x{line}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn the_reads_after_a_leading_bom_are_jqs_7569() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for &(len, split, args, stdout, stderr, status) in ROWS {
        let paths: Vec<String> = files(split, &text(len))
            .iter()
            .enumerate()
            .map(|(at, bytes)| {
                let name = format!("f{at}.json");
                std::fs::write(dir.path().join(&name), bytes)?;
                Ok(name)
            })
            .collect::<Result<_>>()?;
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .arg("jq")
            .args(args)
            .args(&paths)
            .current_dir(dir.path())
            .output()?;
        assert_eq!(
            (
                runs(&String::from_utf8_lossy(&output.stdout)),
                String::from_utf8_lossy(&output.stderr).into_owned(),
                output.status.code()
            ),
            (stdout.to_string(), stderr.to_string(), Some(status)),
            "{args:?} over a {len}-byte first line, the mark {split}"
        );
    }
    Ok(())
}

#[test]
fn the_reads_after_a_leading_bom_on_stdin_are_jqs_7569() -> Result<()> {
    for (len, want) in [(4092, "2046x0 1x2"), (4093, "2046x0 1x1 1x2")] {
        let mut input = BOM.to_vec();
        input.extend_from_slice(&text(len));
        let (output, status) = spawn_jq(&["-c", "input_line_number"], Some(&input))?;
        assert_eq!(
            (runs(&String::from_utf8_lossy(&output.stdout)), status),
            (want.to_string(), 0),
            "a {len}-byte first line"
        );
    }
    Ok(())
}

/// The routes that read a file a window at a time: the plain filter, and `inputs` on the lazy route.
/// A first line of 2,048 `1 ` values is 2,046 values on line 0 and 2 on line 1 in jq 1.7.1, and every
/// record after it is on its own line.
#[cfg(unix)]
#[test]
fn the_reads_after_a_leading_bom_are_jqs_on_the_windowed_routes_7569() -> Result<()> {
    let count = small_window::scaled(120_000);
    let mut stream = BOM.to_vec();
    stream.extend(b"1 ".iter().copied().cycle().take(4096));
    stream.push(b'\n');
    for id in 0..count {
        stream.extend_from_slice(
            format!("{{\"id\":{id},\"pad\":\"{}\"}}\n", "x".repeat(24)).as_bytes(),
        );
    }
    assert!(stream.len() >= small_window::MIN_BYTES, "{}", stream.len());
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("big.json");
    std::fs::write(&path, &stream)?;
    let path = path.to_string_lossy().into_owned();
    let mut want = String::from("2046x0 2x1");
    for line in 2..count + 2 {
        want.push_str(&format!(" 1x{line}"));
    }
    for args in [
        &["-c", "input_line_number"][..],
        &["-c", "-n", "inputs | input_line_number"],
    ] {
        let mut all = args.to_vec();
        all.push(&path);
        let (output, status) = small_window::spawn_jq(&all, None)?;
        assert_eq!(
            (runs(&String::from_utf8_lossy(&output.stdout)), status),
            (want.clone(), 0),
            "{args:?}"
        );
    }
    Ok(())
}
