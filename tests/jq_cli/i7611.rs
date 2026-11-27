//! #7611: `-n inputs` (and `input`, `input_line_number`) over a text whose first fault is not a
//! depth refusal and which goes on into a deeply nested remainder (`[x[x[x...` with its closers at
//! the end) was quadratic in that remainder: after each fault jq drops the rest of its read and
//! parses on from the next one, and every such read was split, indexed and validated to the end of
//! the text again, where jq stops at the next fault. #7470 cut that work at a depth refusal; it now
//! stops at the first fault of any kind. Every row below was captured from jq 1.7.1
//! (`/usr/bin/jq`) over stdin. jq 1.8.2 answers the same but for 7 rows of the program that reads
//! every fault in turn (`try input catch .`): jq 1.8 parses on from the byte after a fault instead
//! of the next read, which neither the old nor the new reader here models.

use std::time::{Duration, Instant};

use anyhow::Result;

use super::i7967::masked;
use crate::run_jq_full;

/// The programs of the rows, by index: the routes that read on after a fault, one that reads every
/// fault in turn (`try input catch .`), and the plain route.
const PROGRAMS: [&[&str]; 5] = [
    &["-n", "input"],
    &["-c", "input_line_number"],
    &["-n", "-c", "[inputs]"],
    &["-n", "-c", "[limit(4; repeat(try input catch .))]"],
    &["-c", "."],
];

/// The texts of the rows: a bad token in each of `n` nested arrays or objects, a stray close
/// bracket, a control character in a string, an empty array element and a non-string key before
/// `n` nested arrays, and two faulty deep documents over several lines.
fn shape(name: &str, n: usize) -> String {
    let deep = |open: &str, close: &str| format!("{}{}", open.repeat(n), close.repeat(n));
    match name {
        "xarr" => deep("[x", "]"),
        "xobj" => deep("{\"a\":x", "}"),
        "close" => format!("1 ] {}2{}", "[1,".repeat(n), "]".repeat(n)),
        "string" => format!("[\"a\u{1}b\",{}]", deep("[", "]")),
        "comma" => format!("[1,,{}2{}]", "[1,".repeat(n), "]".repeat(n)),
        "lines" => format!("{}\n\"y\"\n{}\n4\n", deep("[x", "]"), deep("{\"a\":x", "}")),
        "key" => format!("{{1:{}}}", deep("[x", "]")),
        _ => unreachable!("{name}"),
    }
}

/// `(n, shape, program, exit status, stdout, stderr)`, from jq 1.7.1.
#[rustfmt::skip]
const ROWS: &[(usize, &str, usize, i32, &str, &str)] = &[
    (300, "xarr", 0, 5, "", "jq: error (at <stdin>:0): Invalid numeric literal at line 1, column 3\n"),
    (300, "xarr", 1, 5, "", "jq: parse error: Invalid numeric literal at line 1, column 3\n"),
    (300, "xarr", 2, 5, "", "jq: error (at <stdin>:0): Invalid numeric literal at line 1, column 3\n"),
    (300, "xarr", 3, 0, "[\"Invalid numeric literal at line 1, column 3\",\"break\",\"break\",\"break\"]\n", ""),
    (300, "xarr", 4, 5, "", "jq: parse error: Invalid numeric literal at line 1, column 3\n"),
    (300, "xobj", 0, 5, "", "jq: error (at <stdin>:0): Invalid numeric literal at line 1, column 7\n"),
    (300, "xobj", 1, 5, "", "jq: parse error: Invalid numeric literal at line 1, column 7\n"),
    (300, "xobj", 2, 5, "", "jq: error (at <stdin>:0): Invalid numeric literal at line 1, column 7\n"),
    (300, "xobj", 3, 0, "[\"Invalid numeric literal at line 1, column 7\",\"break\",\"break\",\"break\"]\n", ""),
    (300, "xobj", 4, 5, "", "jq: parse error: Invalid numeric literal at line 1, column 7\n"),
    (300, "close", 0, 0, "1\n", ""),
    (300, "close", 1, 5, "0\n", "jq: parse error: Unmatched ']' at line 1, column 3\n"),
    (300, "close", 2, 5, "", "jq: error (at <stdin>:0): Unmatched ']' at line 1, column 3\n"),
    (300, "close", 3, 0, "[1,\"Unmatched ']' at line 1, column 3\",\"break\",\"break\"]\n", ""),
    (300, "close", 4, 5, "1\n", "jq: parse error: Unmatched ']' at line 1, column 3\n"),
    (300, "string", 0, 5, "", "jq: error (at <stdin>:0): Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6\n"),
    (300, "string", 1, 5, "", "jq: parse error: Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6\n"),
    (300, "string", 2, 5, "", "jq: error (at <stdin>:0): Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6\n"),
    (300, "string", 3, 0, "[\"Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6\",\"break\",\"break\",\"break\"]\n", ""),
    (300, "string", 4, 5, "", "jq: parse error: Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6\n"),
    (300, "comma", 0, 5, "", "jq: error (at <stdin>:0): Expected value before ',' at line 1, column 4\n"),
    (300, "comma", 1, 5, "", "jq: parse error: Expected value before ',' at line 1, column 4\n"),
    (300, "comma", 2, 5, "", "jq: error (at <stdin>:0): Expected value before ',' at line 1, column 4\n"),
    (300, "comma", 3, 0, "[\"Expected value before ',' at line 1, column 4\",\"break\",\"break\",\"break\"]\n", ""),
    (300, "comma", 4, 5, "", "jq: parse error: Expected value before ',' at line 1, column 4\n"),
    (300, "lines", 0, 5, "", "jq: error (at <stdin>:1): Invalid numeric literal at line 1, column 3\n"),
    (300, "lines", 1, 5, "", "jq: parse error: Invalid numeric literal at line 1, column 3\n"),
    (300, "lines", 2, 5, "", "jq: error (at <stdin>:1): Invalid numeric literal at line 1, column 3\n"),
    (300, "lines", 3, 0, "[\"Invalid numeric literal at line 1, column 3\",\"y\",\"Invalid numeric literal at line 2, column 7\",4]\n", ""),
    (300, "lines", 4, 5, "", "jq: parse error: Invalid numeric literal at line 1, column 3\n"),
    (300, "key", 0, 5, "", "jq: error (at <stdin>:0): Object keys must be strings at line 1, column 3\n"),
    (300, "key", 1, 5, "", "jq: parse error: Object keys must be strings at line 1, column 3\n"),
    (300, "key", 2, 5, "", "jq: error (at <stdin>:0): Object keys must be strings at line 1, column 3\n"),
    (300, "key", 3, 0, "[\"Object keys must be strings at line 1, column 3\",\"break\",\"break\",\"break\"]\n", ""),
    (300, "key", 4, 5, "", "jq: parse error: Object keys must be strings at line 1, column 3\n"),
    (3000, "xarr", 0, 5, "", "jq: error (at <stdin>:0): Invalid numeric literal at line 1, column 3\n"),
    (3000, "xarr", 1, 5, "", "jq: parse error: Invalid numeric literal at line 1, column 3\n"),
    (3000, "xarr", 2, 5, "", "jq: error (at <stdin>:0): Invalid numeric literal at line 1, column 3\n"),
    (3000, "xarr", 3, 0, "[\"Invalid numeric literal at line 1, column 3\",\"Invalid numeric literal at line 1, column 5\",\"Unmatched ']' at line 1, column 6\",\"break\"]\n", ""),
    (3000, "xarr", 4, 5, "", "jq: parse error: Invalid numeric literal at line 1, column 3\n"),
    (3000, "xobj", 0, 5, "", "jq: error (at <stdin>:0): Invalid numeric literal at line 1, column 7\n"),
    (3000, "xobj", 1, 5, "", "jq: parse error: Invalid numeric literal at line 1, column 7\n"),
    (3000, "xobj", 2, 5, "", "jq: error (at <stdin>:0): Invalid numeric literal at line 1, column 7\n"),
    (3000, "xobj", 3, 0, "[\"Invalid numeric literal at line 1, column 7\",\":x{\",\"Invalid numeric literal at line 1, column 14\",\"Invalid numeric literal at line 1, column 21\"]\n", ""),
    (3000, "xobj", 4, 5, "", "jq: parse error: Invalid numeric literal at line 1, column 7\n"),
    (3000, "close", 0, 0, "1\n", ""),
    (3000, "close", 1, 5, "0\n", "jq: parse error: Unmatched ']' at line 1, column 3\n"),
    (3000, "close", 2, 5, "", "jq: error (at <stdin>:0): Unmatched ']' at line 1, column 3\n"),
    (3000, "close", 3, 0, "[1,\"Unmatched ']' at line 1, column 3\",\"Expected value before ',' at line 1, column 4\",\"Expected value before ',' at line 1, column 5\"]\n", ""),
    (3000, "close", 4, 5, "1\n", "jq: parse error: Unmatched ']' at line 1, column 3\n"),
    (3000, "string", 0, 5, "", "jq: error (at <stdin>:0): Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6\n"),
    (3000, "string", 1, 5, "", "jq: parse error: Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6\n"),
    (3000, "string", 2, 5, "", "jq: error (at <stdin>:0): Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6\n"),
    (3000, "string", 3, 0, "[\"Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6\",\"Unmatched ']' at line 1, column 7\",\"break\",\"break\"]\n", ""),
    (3000, "string", 4, 5, "", "jq: parse error: Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6\n"),
    (3000, "comma", 0, 5, "", "jq: error (at <stdin>:0): Expected value before ',' at line 1, column 4\n"),
    (3000, "comma", 1, 5, "", "jq: parse error: Expected value before ',' at line 1, column 4\n"),
    (3000, "comma", 2, 5, "", "jq: error (at <stdin>:0): Expected value before ',' at line 1, column 4\n"),
    (3000, "comma", 3, 0, "[\"Expected value before ',' at line 1, column 4\",\"Expected value before ',' at line 1, column 5\",\"Expected value before ',' at line 1, column 6\",\"break\"]\n", ""),
    (3000, "comma", 4, 5, "", "jq: parse error: Expected value before ',' at line 1, column 4\n"),
    (3000, "lines", 0, 5, "", "jq: error (at <stdin>:0): Invalid numeric literal at line 1, column 3\n"),
    (3000, "lines", 1, 5, "", "jq: parse error: Invalid numeric literal at line 1, column 3\n"),
    (3000, "lines", 2, 5, "", "jq: error (at <stdin>:0): Invalid numeric literal at line 1, column 3\n"),
    (3000, "lines", 3, 0, "[\"Invalid numeric literal at line 1, column 3\",\"Invalid numeric literal at line 1, column 5\",\"Unmatched ']' at line 1, column 6\",\"y\"]\n", ""),
    (3000, "lines", 4, 5, "", "jq: parse error: Invalid numeric literal at line 1, column 3\n"),
    (3000, "key", 0, 5, "", "jq: error (at <stdin>:0): Object keys must be strings at line 1, column 3\n"),
    (3000, "key", 1, 5, "", "jq: parse error: Object keys must be strings at line 1, column 3\n"),
    (3000, "key", 2, 5, "", "jq: error (at <stdin>:0): Object keys must be strings at line 1, column 3\n"),
    (3000, "key", 3, 0, "[\"Object keys must be strings at line 1, column 3\",\"Invalid numeric literal at line 1, column 6\",\"Unmatched ']' at line 1, column 7\",\"break\"]\n", ""),
    (3000, "key", 4, 5, "", "jq: parse error: Object keys must be strings at line 1, column 3\n"),
    (30000, "xarr", 0, 5, "", "jq: error (at <stdin>:0): Invalid numeric literal at line 1, column 3\n"),
    (30000, "xarr", 1, 5, "", "jq: parse error: Invalid numeric literal at line 1, column 3\n"),
    (30000, "xarr", 2, 5, "", "jq: error (at <stdin>:0): Invalid numeric literal at line 1, column 3\n"),
    (30000, "xarr", 3, 0, "[\"Invalid numeric literal at line 1, column 3\",\"Invalid numeric literal at line 1, column 5\",\"Invalid numeric literal at line 1, column 8\",\"Invalid numeric literal at line 1, column 10\"]\n", ""),
    (30000, "xarr", 4, 5, "", "jq: parse error: Invalid numeric literal at line 1, column 3\n"),
    (30000, "xobj", 0, 5, "", "jq: error (at <stdin>:0): Invalid numeric literal at line 1, column 7\n"),
    (30000, "xobj", 1, 5, "", "jq: parse error: Invalid numeric literal at line 1, column 7\n"),
    (30000, "xobj", 2, 5, "", "jq: error (at <stdin>:0): Invalid numeric literal at line 1, column 7\n"),
    (30000, "xobj", 3, 0, "[\"Invalid numeric literal at line 1, column 7\",\":x{\",\"Invalid numeric literal at line 1, column 14\",\"Invalid numeric literal at line 1, column 21\"]\n", ""),
    (30000, "xobj", 4, 5, "", "jq: parse error: Invalid numeric literal at line 1, column 7\n"),
    (30000, "close", 0, 0, "1\n", ""),
    (30000, "close", 1, 5, "0\n", "jq: parse error: Unmatched ']' at line 1, column 3\n"),
    (30000, "close", 2, 5, "", "jq: error (at <stdin>:0): Unmatched ']' at line 1, column 3\n"),
    (30000, "close", 3, 0, "[1,\"Unmatched ']' at line 1, column 3\",\"Expected value before ',' at line 1, column 4\",\"Expected value before ',' at line 1, column 5\"]\n", ""),
    (30000, "close", 4, 5, "1\n", "jq: parse error: Unmatched ']' at line 1, column 3\n"),
    (30000, "string", 0, 5, "", "jq: error (at <stdin>:0): Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6\n"),
    (30000, "string", 1, 5, "", "jq: parse error: Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6\n"),
    (30000, "string", 2, 5, "", "jq: error (at <stdin>:0): Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6\n"),
    (30000, "string", 3, 0, "[\"Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6\",\"Exceeds depth limit for parsing at line 1, column 263\",\"Exceeds depth limit for parsing at line 1, column 520\",\"Exceeds depth limit for parsing at line 1, column 777\"]\n", ""),
    (30000, "string", 4, 5, "", "jq: parse error: Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6\n"),
    (30000, "comma", 0, 5, "", "jq: error (at <stdin>:0): Expected value before ',' at line 1, column 4\n"),
    (30000, "comma", 1, 5, "", "jq: parse error: Expected value before ',' at line 1, column 4\n"),
    (30000, "comma", 2, 5, "", "jq: error (at <stdin>:0): Expected value before ',' at line 1, column 4\n"),
    (30000, "comma", 3, 0, "[\"Expected value before ',' at line 1, column 4\",\"Expected value before ',' at line 1, column 5\",\"Expected value before ',' at line 1, column 6\",\"Expected value before ',' at line 1, column 7\"]\n", ""),
    (30000, "comma", 4, 5, "", "jq: parse error: Expected value before ',' at line 1, column 4\n"),
    (30000, "lines", 0, 5, "", "jq: error (at <stdin>:0): Invalid numeric literal at line 1, column 3\n"),
    (30000, "lines", 1, 5, "", "jq: parse error: Invalid numeric literal at line 1, column 3\n"),
    (30000, "lines", 2, 5, "", "jq: error (at <stdin>:0): Invalid numeric literal at line 1, column 3\n"),
    (30000, "lines", 3, 0, "[\"Invalid numeric literal at line 1, column 3\",\"Invalid numeric literal at line 1, column 5\",\"Invalid numeric literal at line 1, column 8\",\"Invalid numeric literal at line 1, column 10\"]\n", ""),
    (30000, "lines", 4, 5, "", "jq: parse error: Invalid numeric literal at line 1, column 3\n"),
    (30000, "key", 0, 5, "", "jq: error (at <stdin>:0): Object keys must be strings at line 1, column 3\n"),
    (30000, "key", 1, 5, "", "jq: parse error: Object keys must be strings at line 1, column 3\n"),
    (30000, "key", 2, 5, "", "jq: error (at <stdin>:0): Object keys must be strings at line 1, column 3\n"),
    (30000, "key", 3, 0, "[\"Object keys must be strings at line 1, column 3\",\"Invalid numeric literal at line 1, column 6\",\"Invalid numeric literal at line 1, column 8\",\"Invalid numeric literal at line 1, column 11\"]\n", ""),
    (30000, "key", 4, 5, "", "jq: parse error: Object keys must be strings at line 1, column 3\n"),
];

#[test]
fn test_reading_past_a_fault_into_a_deep_text_matches_jq_on_every_input_route_7611() -> Result<()> {
    for &(n, name, program, code, stdout, stderr) in ROWS {
        let text = shape(name, n);
        let got = run_jq_full(PROGRAMS[program], Some(&text))?;
        assert_eq!(
            (masked(&got.0), masked(&got.1), got.2),
            (masked(stdout), masked(stderr), code),
            "{name} x {n}, {:?}",
            PROGRAMS[program]
        );
    }
    Ok(())
}

/// A bad token in each of 2,000,000 nested arrays (6 MB): each read after the first fault is
/// parsed only through the next fault, so the reads take a fraction of a second in a debug build,
/// where parsing every read to the end of the text took far longer. The bound sits between.
#[test]
fn test_reading_past_a_fault_into_a_deep_text_is_not_quadratic_7611() -> Result<()> {
    let text = shape("xarr", 2_000_000);
    for program in [
        &["-n", "-c", "[inputs]"][..],
        &["-n", "input"],
        &["-c", "input_line_number"],
    ] {
        let started = Instant::now();
        let (stdout, stderr, code) = run_jq_full(program, Some(&text))?;
        assert_eq!(code, 5, "{program:?}: {stdout:?} {stderr:?}");
        assert!(
            stderr.contains("Invalid numeric literal at line 1, column 3"),
            "{program:?}: {stderr:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{program:?} took {:?}",
            started.elapsed()
        );
    }
    Ok(())
}
