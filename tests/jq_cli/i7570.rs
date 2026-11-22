//! #7570: under `--stream`, a closer right after a scalar completes two events at one byte (the
//! scalar's and the container's close). jq's parser hands out the first at that byte and the
//! second with no byte left to read, and jq's input loop reads the next `fgets` chunk before it
//! asks for an item whenever none is left. So when that byte is the last of a 4095-byte chunk, a run
//! that stops on the second event leaves standard input at the end of the chunk after it; the
//! #7455 restore left it at the end of the first.
//!
//! The texts are `S` spaces, a container whose first closer after a scalar stands at byte 4093,
//! 4094 or 4095, then ` 3\n4\n` (and, for the windowed route, 700,000 more lines). Every offset was
//! captured from `/usr/bin/jq` 1.7.1, read back from the shared descriptor after the run. jq 1.8.2
//! leaves the same offsets wherever its chunk (4091 bytes) ends at the same place: the rows at
//! byte 4094 and 4095, and every row past the first line.

#![cfg(unix)]

use std::io::{Seek, SeekFrom, Write};
use std::process::{Command, Stdio};

/// The containers, and for each place of its first closer after a scalar (bytes 4093, 4094 and
/// 4095), where `--stream -n '[limit(k; inputs)] | length'` leaves the file for k = 1, 2, ...
/// every event of the text, and where `--stream 'if length == 1 then halt else empty end'` (a halt
/// on the first close) leaves it.
/// For one place of the closer: the offsets after `limit(k; ...)` for each k, and after the halt.
type Place = (&'static [u64], u64);

const ROWS: &[(&str, [Place; 3])] = &[
    (
        "[2]",
        [
            (&[4095, 4095, 4097, 4099], 4095),
            (&[4095, 4098, 4098, 4100], 4098),
            (&[4099, 4099, 4099, 4101], 4099),
        ],
    ),
    (
        "{\"a\":2}",
        [
            (&[4095, 4095, 4097, 4099], 4095),
            (&[4095, 4098, 4098, 4100], 4098),
            (&[4099, 4099, 4099, 4101], 4099),
        ],
    ),
    (
        "[[[2]]]",
        [
            (&[4095, 4095, 4095, 4099, 4099, 4101], 4095),
            (&[4095, 4100, 4100, 4100, 4100, 4102], 4100),
            (&[4101, 4101, 4101, 4101, 4101, 4103], 4101),
        ],
    ),
    (
        "[\"s\"]",
        [
            (&[4095, 4095, 4097, 4099], 4095),
            (&[4095, 4098, 4098, 4100], 4098),
            (&[4099, 4099, 4099, 4101], 4099),
        ],
    ),
    (
        "[1,2]",
        [
            (&[4095, 4095, 4095, 4097, 4099], 4095),
            (&[4095, 4095, 4098, 4098, 4100], 4098),
            (&[4095, 4099, 4099, 4099, 4101], 4099),
        ],
    ),
    (
        "{\"a\":[2]}",
        [
            (&[4095, 4095, 4095, 4098, 4100], 4095),
            (&[4095, 4099, 4099, 4099, 4101], 4099),
            (&[4100, 4100, 4100, 4100, 4102], 4100),
        ],
    ),
];

/// The text for `container` with its first closer after a scalar at byte `at`.
fn text(container: &str, at: usize, windowed: bool) -> Vec<u8> {
    let closer = container
        .find([']', '}'])
        .expect("the container has a closer");
    let mut text = format!("{}{container} 3\n4\n", " ".repeat(at - closer)).into_bytes();
    if windowed {
        text.extend_from_slice(&b"4\n".repeat(700_000));
    }
    text
}

/// Where `succinctly jq ARGS` leaves `bytes` on standard input, started at the front.
fn offset_after(bytes: &[u8], args: &[&str]) -> u64 {
    let mut file = tempfile::tempfile().expect("a temporary file");
    file.write_all(bytes).expect("written");
    file.seek(SeekFrom::Start(0)).expect("seeked");
    let status = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .stdin(Stdio::from(file.try_clone().expect("a duplicate")))
        .stdout(Stdio::null())
        .status()
        .expect("ran");
    assert!(status.success(), "{args:?}");
    file.stream_position().expect("the offset")
}

#[test]
fn a_run_that_stops_on_an_event_leaves_jqs_offset_at_a_chunk_boundary_7570() {
    let mut failures = Vec::new();
    for (container, places) in ROWS {
        for ((limits, halt), at) in places.iter().zip(4093..) {
            let small = text(container, at, false);
            for (k, &want) in (1..).zip(limits.iter()) {
                let program = format!("[limit({k}; inputs)] | length");
                let got = offset_after(&small, &["-c", "--stream", "-n", &program]);
                if got != want {
                    failures.push(format!("{container} at {at}, limit {k}: {got}, jq {want}"));
                }
            }
            for windowed in [false, true] {
                let bytes = text(container, at, windowed);
                let program = "if length == 1 then halt else empty end";
                let got = offset_after(&bytes, &["-c", "--stream", program]);
                if got != *halt {
                    failures.push(format!(
                        "{container} at {at}, halt (windowed {windowed}): {got}, jq {halt}"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
