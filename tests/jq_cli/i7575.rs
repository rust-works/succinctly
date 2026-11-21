//! #7575: a run that meets a parse error leaves a regular file on standard input where jq 1.7.1
//! leaves it. jq's reader hands the error out as an item, at the byte where its parser refuses the
//! text; its parser is then reset and the rest of the `fgets` chunk (a line, or 4095 bytes of a
//! longer one) that held the byte is dropped, so a run that reads on (`try input catch ...`) takes
//! its next input from the chunk after it. A run that stops at the error (the main loop, an
//! uncaught error) leaves the file at the end of the error's chunk. It used to be left where the
//! run's own reads left it, its end here.
//!
//! Every offset was captured from `/usr/bin/jq` 1.7.1, read back from the shared descriptor after
//! the run; jq 1.8.2 agrees except on the 6,000-byte line, where its chunk is 4091 bytes.

#![cfg(unix)]

use std::io::{Read, Seek, SeekFrom, Write};
use std::process::{Command, Stdio};

const TEXTS: &[&[u8]] = &[
    b"{\"id\":0}\n{\"id\":1}\n}\n{\"id\":3}\n{\"id\":4}\n",
    b"{\"id\":0}\n[1,}\n{\"id\":2}\n{\"id\":3}\n",
    b"1\n2 }\n3\n4\n",
    b"1\ntru\n3\n4\n",
    b"1\n[1,\n}\n2]\n3\n4\n",
    b"1 2 } 3 4\n5\n6\n",
    // `[1] ` 1,500 times, then a stray `}` at byte 6,000, on one line.
    LONG,
    b"\"a\"\n\"b\x01\"\n\"c\"\n\"d\"\n",
];

const LONG: &[u8] = &{
    let mut text = [0u8; 6000 + b"} [2]\n3\n4\n".len()];
    let mut i = 0;
    while i < 6000 {
        text[i] = b"[1] "[i % 4];
        i += 1;
    }
    let tail = b"} [2]\n3\n4\n";
    let mut j = 0;
    while j < tail.len() {
        text[6000 + j] = tail[j];
        j += 1;
    }
    text
};

/// For each text, the program, and where jq 1.7.1 leaves standard input after it.
const ROWS: &[(usize, &[&str], u64)] = &[
    (0, &["-c", "."], 20),
    (0, &["-c", "-n", "[inputs]"], 20),
    (
        0,
        &[
            "-c",
            "-n",
            "[range(1) as $_ | try input catch \"E\"] | length",
        ],
        9,
    ),
    (
        0,
        &[
            "-c",
            "-n",
            "[range(2) as $_ | try input catch \"E\"] | length",
        ],
        18,
    ),
    (
        0,
        &[
            "-c",
            "-n",
            "[range(3) as $_ | try input catch \"E\"] | length",
        ],
        20,
    ),
    (
        0,
        &[
            "-c",
            "-n",
            "[range(4) as $_ | try input catch \"E\"] | length",
        ],
        29,
    ),
    (
        0,
        &[
            "-c",
            "-n",
            "[range(5) as $_ | try input catch \"E\"] | length",
        ],
        38,
    ),
    (
        0,
        &[
            "-c",
            "-n",
            "[range(6) as $_ | try input catch \"E\"] | length",
        ],
        38,
    ),
    (1, &["-c", "."], 14),
    (1, &["-c", "-n", "[inputs]"], 14),
    (
        1,
        &[
            "-c",
            "-n",
            "[range(1) as $_ | try input catch \"E\"] | length",
        ],
        9,
    ),
    (
        1,
        &[
            "-c",
            "-n",
            "[range(2) as $_ | try input catch \"E\"] | length",
        ],
        14,
    ),
    (
        1,
        &[
            "-c",
            "-n",
            "[range(3) as $_ | try input catch \"E\"] | length",
        ],
        23,
    ),
    (
        1,
        &[
            "-c",
            "-n",
            "[range(4) as $_ | try input catch \"E\"] | length",
        ],
        32,
    ),
    (
        1,
        &[
            "-c",
            "-n",
            "[range(5) as $_ | try input catch \"E\"] | length",
        ],
        32,
    ),
    (
        1,
        &[
            "-c",
            "-n",
            "[range(6) as $_ | try input catch \"E\"] | length",
        ],
        32,
    ),
    (2, &["-c", "."], 6),
    (2, &["-c", "-n", "[inputs]"], 6),
    (
        2,
        &[
            "-c",
            "-n",
            "[range(1) as $_ | try input catch \"E\"] | length",
        ],
        2,
    ),
    (
        2,
        &[
            "-c",
            "-n",
            "[range(2) as $_ | try input catch \"E\"] | length",
        ],
        6,
    ),
    (
        2,
        &[
            "-c",
            "-n",
            "[range(3) as $_ | try input catch \"E\"] | length",
        ],
        6,
    ),
    (
        2,
        &[
            "-c",
            "-n",
            "[range(4) as $_ | try input catch \"E\"] | length",
        ],
        8,
    ),
    (
        2,
        &[
            "-c",
            "-n",
            "[range(5) as $_ | try input catch \"E\"] | length",
        ],
        10,
    ),
    (
        2,
        &[
            "-c",
            "-n",
            "[range(6) as $_ | try input catch \"E\"] | length",
        ],
        10,
    ),
    (3, &["-c", "."], 6),
    (3, &["-c", "-n", "[inputs]"], 6),
    (
        3,
        &[
            "-c",
            "-n",
            "[range(1) as $_ | try input catch \"E\"] | length",
        ],
        2,
    ),
    (
        3,
        &[
            "-c",
            "-n",
            "[range(2) as $_ | try input catch \"E\"] | length",
        ],
        6,
    ),
    (
        3,
        &[
            "-c",
            "-n",
            "[range(3) as $_ | try input catch \"E\"] | length",
        ],
        8,
    ),
    (
        3,
        &[
            "-c",
            "-n",
            "[range(4) as $_ | try input catch \"E\"] | length",
        ],
        10,
    ),
    (
        3,
        &[
            "-c",
            "-n",
            "[range(5) as $_ | try input catch \"E\"] | length",
        ],
        10,
    ),
    (
        3,
        &[
            "-c",
            "-n",
            "[range(6) as $_ | try input catch \"E\"] | length",
        ],
        10,
    ),
    (4, &["-c", "."], 8),
    (4, &["-c", "-n", "[inputs]"], 8),
    (
        4,
        &[
            "-c",
            "-n",
            "[range(1) as $_ | try input catch \"E\"] | length",
        ],
        2,
    ),
    (
        4,
        &[
            "-c",
            "-n",
            "[range(2) as $_ | try input catch \"E\"] | length",
        ],
        8,
    ),
    (
        4,
        &[
            "-c",
            "-n",
            "[range(3) as $_ | try input catch \"E\"] | length",
        ],
        11,
    ),
    (
        4,
        &[
            "-c",
            "-n",
            "[range(4) as $_ | try input catch \"E\"] | length",
        ],
        13,
    ),
    (
        4,
        &[
            "-c",
            "-n",
            "[range(5) as $_ | try input catch \"E\"] | length",
        ],
        15,
    ),
    (
        4,
        &[
            "-c",
            "-n",
            "[range(6) as $_ | try input catch \"E\"] | length",
        ],
        15,
    ),
    (5, &["-c", "."], 10),
    (5, &["-c", "-n", "[inputs]"], 10),
    (
        5,
        &[
            "-c",
            "-n",
            "[range(1) as $_ | try input catch \"E\"] | length",
        ],
        10,
    ),
    (
        5,
        &[
            "-c",
            "-n",
            "[range(2) as $_ | try input catch \"E\"] | length",
        ],
        10,
    ),
    (
        5,
        &[
            "-c",
            "-n",
            "[range(3) as $_ | try input catch \"E\"] | length",
        ],
        10,
    ),
    (
        5,
        &[
            "-c",
            "-n",
            "[range(4) as $_ | try input catch \"E\"] | length",
        ],
        12,
    ),
    (
        5,
        &[
            "-c",
            "-n",
            "[range(5) as $_ | try input catch \"E\"] | length",
        ],
        14,
    ),
    (
        5,
        &[
            "-c",
            "-n",
            "[range(6) as $_ | try input catch \"E\"] | length",
        ],
        14,
    ),
    (6, &["-c", "."], 6006),
    (6, &["-c", "-n", "[inputs]"], 6006),
    (
        6,
        &[
            "-c",
            "-n",
            "[range(1) as $_ | try input catch \"E\"] | length",
        ],
        4095,
    ),
    (
        6,
        &[
            "-c",
            "-n",
            "[range(2) as $_ | try input catch \"E\"] | length",
        ],
        4095,
    ),
    (
        6,
        &[
            "-c",
            "-n",
            "[range(3) as $_ | try input catch \"E\"] | length",
        ],
        4095,
    ),
    (
        6,
        &[
            "-c",
            "-n",
            "[range(4) as $_ | try input catch \"E\"] | length",
        ],
        4095,
    ),
    (
        6,
        &[
            "-c",
            "-n",
            "[range(5) as $_ | try input catch \"E\"] | length",
        ],
        4095,
    ),
    (
        6,
        &[
            "-c",
            "-n",
            "[range(6) as $_ | try input catch \"E\"] | length",
        ],
        4095,
    ),
    (7, &["-c", "."], 9),
    (7, &["-c", "-n", "[inputs]"], 9),
    (
        7,
        &[
            "-c",
            "-n",
            "[range(1) as $_ | try input catch \"E\"] | length",
        ],
        4,
    ),
    (
        7,
        &[
            "-c",
            "-n",
            "[range(2) as $_ | try input catch \"E\"] | length",
        ],
        9,
    ),
    (
        7,
        &[
            "-c",
            "-n",
            "[range(3) as $_ | try input catch \"E\"] | length",
        ],
        13,
    ),
    (
        7,
        &[
            "-c",
            "-n",
            "[range(4) as $_ | try input catch \"E\"] | length",
        ],
        17,
    ),
    (
        7,
        &[
            "-c",
            "-n",
            "[range(5) as $_ | try input catch \"E\"] | length",
        ],
        17,
    ),
    (
        7,
        &[
            "-c",
            "-n",
            "[range(6) as $_ | try input catch \"E\"] | length",
        ],
        17,
    ),
];

/// Where `succinctly jq ARGS` leaves `bytes` on standard input, started at the front.
fn offset_after(bytes: &[u8], args: &[&str]) -> u64 {
    let mut file = tempfile::tempfile().expect("a temporary file");
    file.write_all(bytes).expect("written");
    file.seek(SeekFrom::Start(0)).expect("seeked");
    Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .stdin(Stdio::from(file.try_clone().expect("a duplicate")))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("ran");
    file.stream_position().expect("the offset")
}

#[test]
fn a_parse_error_is_an_item_whose_chunk_is_dropped_7575() {
    let mut failures = Vec::new();
    for &(text, args, want) in ROWS {
        let got = offset_after(TEXTS[text], args);
        if got != want {
            failures.push(format!("text {text}, {args:?}: {got}, jq {want}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn a_standard_input_the_output_shares_is_left_where_the_run_left_it_7575() {
    // `jq -n '[limit(2; inputs)]' <> f >&0`: one open file for both. Where jq leaves it depends on
    // when stdio flushed the output (jq 1.7.1 on macOS: 38), so the offset is not put back: the
    // reads took all 27 bytes and the output went on after them.
    let mut file = tempfile::tempfile().expect("a temporary file");
    file.write_all(b"{\"id\":0}\n{\"id\":1}\n{\"id\":2}\n")
        .expect("written");
    file.seek(SeekFrom::Start(0)).expect("seeked");
    let status = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["jq", "-c", "-n", "[limit(2; inputs)]"])
        .stdin(Stdio::from(file.try_clone().expect("a duplicate")))
        .stdout(Stdio::from(file.try_clone().expect("a duplicate")))
        .status()
        .expect("ran");
    assert!(status.success());
    let at = file.stream_position().expect("the offset");
    let mut all = String::new();
    file.seek(SeekFrom::Start(0)).expect("seeked");
    file.read_to_string(&mut all).expect("read");
    assert_eq!(
        (at, all.as_str()),
        (
            47,
            "{\"id\":0}\n{\"id\":1}\n{\"id\":2}\n[{\"id\":0},{\"id\":1}]\n"
        )
    );
}
