//! #7266: the 4 MiB a pipe's probe read are handed out a block (1 MiB) at a time from the buffer they
//! were read into, where they used to be one window with a block read onto them. Each row puts what a
//! window boundary can cut inside the probe (a fault, bytes that are not UTF-8, a document over several
//! lines, a line longer than a block, a BOM) and checks the run against the same bytes read whole: the
//! pipe named as `/dev/stdin`, which the windowed routes decline, so stdout, stderr (with that name read
//! as `<stdin>`) and the exit status are the whole-file route's.
//!
//! Every size here is a number of windows. In a debug build the window is 64 KiB and the fixtures
//! a sixteenth of the size (`small_window`, #7376); a release build runs the same rows over the
//! 1 MiB window, where the probe is the 4 MiB the first line speaks of.

#![cfg(unix)]

use anyhow::Result;

use crate::small_window::{scaled, spawn_jq as spawn_jq_deadline, WINDOW};

/// How many records the fixtures have: about six windows (6 MiB on the shipped one), so the probe is
/// full and the pipe goes on past it.
const RECORDS: usize = scaled(60_000);

fn records() -> Vec<String> {
    (0..RECORDS)
        .map(|i| {
            format!(
                "{{\"id\":{i},\"name\":\"user {i}\",\"pad\":\"{}\"}}",
                "x".repeat(60)
            )
        })
        .collect()
}

fn join(lines: &[String]) -> Vec<u8> {
    let mut bytes = lines.join("\n").into_bytes();
    bytes.push(b'\n');
    bytes
}

/// The windowed run over `bytes` on a pipe, checked against the whole run: its stderr and status.
fn same(args: &[&str], bytes: &[u8]) -> Result<(String, i32)> {
    let (windowed, windowed_status) = spawn_jq_deadline(args, Some(bytes))?;
    let mut whole_args = args.to_vec();
    whole_args.push("/dev/stdin");
    let (whole, whole_status) = spawn_jq_deadline(&whole_args, Some(bytes))?;
    let whole_err = String::from_utf8_lossy(&whole.stderr).replace("/dev/stdin", "<stdin>");
    let err = String::from_utf8_lossy(&windowed.stderr).into_owned();
    assert!(
        windowed.stdout == whole.stdout,
        "{args:?}: stdout differs ({} and {} bytes)",
        windowed.stdout.len(),
        whole.stdout.len()
    );
    assert_eq!(err, whole_err, "{args:?}: stderr");
    assert_eq!(windowed_status, whole_status, "{args:?}: exit status");
    Ok((err, windowed_status))
}

/// The record that holds byte `at` of `join(lines)`.
fn line_at(lines: &[String], at: usize) -> usize {
    let mut end = 0;
    lines
        .iter()
        .position(|line| {
            end += line.len() + 1;
            end > at
        })
        .expect("in the stream")
}

#[test]
fn what_a_window_of_the_probe_cuts_reads_as_the_whole_stream_does_7266() -> Result<()> {
    let base = records();
    assert!(
        join(&base).len() > 5 * WINDOW,
        "the pipe must go on past the probe"
    );
    let some = format!("select(.id % {} == 0) | .name", RECORDS / 8 + 1);
    let last = format!("select(.id == {}) | .id", RECORDS - 1);
    let programs: [&[&str]; 3] = [&["-c", "."], &["-r", &some], &["-c", &last]];
    // Near each block's end inside the probe, and in its last block.
    for at in [WINDOW - 40, 2 * WINDOW + 7, 4 * WINDOW - 30] {
        let n = line_at(&base, at);
        // A fault in a member the filter never reads, worded once the stream ends inside a document
        // (the first fault of the stream, which the probe's window held and dropped long before).
        let mut fault = base.clone();
        fault[n] = format!("{{\"id\":{n},\"name\":\"x\",\"t\":[1 2]}}");
        let mut fault = join(&fault);
        fault.extend_from_slice(b"{\"id\":");
        // Bytes that are not UTF-8, then a fault on a later line, placed in the bytes as read.
        let mut lossy = join(&base);
        let start = join(&base[..n]).len();
        let inside_pad = start + base[n].len() - 5;
        lossy.splice(inside_pad..inside_pad, *b"\xff\xfe");
        let m = n + 3;
        // Past the two bytes put in above, and the `{` that opens the record.
        let after_brace = join(&base[..m]).len() + 2 + 1;
        lossy.splice(after_brace..after_brace, *b"\"q\":[1 2],");
        lossy.extend_from_slice(b"{\"id\":");
        // A document over several lines across the block's end.
        let mut pretty = base.clone();
        pretty[n] = format!("{{\"id\":{n},\n  \"name\":\"p\",\n  \"v\":[1,\n2,\n3]}}");
        for args in programs {
            let (err, status) = same(args, &fault)?;
            assert!(
                err.contains(&format!("at line {}", n + 1)),
                "{at} {args:?}: {err}"
            );
            assert_ne!(status, 0);
            let (err, _) = same(args, &lossy)?;
            assert!(
                err.contains(&format!("at line {}", m + 1)),
                "{at} {args:?}: {err}"
            );
            same(args, &join(&pretty))?;
        }
    }
    // The same shapes across a block's end by construction. The rows above meet them only when a
    // record happens to lie across one, which the 1 MiB window's fixture did at 2 MiB + 7 and the
    // 64 KiB window's does nowhere (#7376): a window ends at the last line break before the block's
    // end, so the record that holds the end's byte is the first line of the next window, and a
    // document of a hundred lines that starts 20 bytes or more before the end has a line break on
    // each side of it.
    for end in [WINDOW, 2 * WINDOW, 4 * WINDOW] {
        let n = line_at(&base, end);
        let mut fault = base.clone();
        fault[n] = format!(
            "{{\"id\":{n},\"name\":\"{}\",\"t\":[1 2]}}",
            "x".repeat(120)
        );
        let mut fault = join(&fault);
        fault.extend_from_slice(b"{\"id\":");
        let p = line_at(&base, end - 20);
        let mut pretty = base.clone();
        pretty[p] = format!(
            "{{\"id\":{p},\n  \"name\":\"p\",\n  \"v\":[{}3]}}",
            "1,\n".repeat(100)
        );
        for args in programs {
            let (err, status) = same(args, &fault)?;
            assert!(
                err.contains(&format!("at line {}", n + 1)),
                "{end} {args:?}: {err}"
            );
            assert_ne!(status, 0);
            same(args, &join(&pretty))?;
        }
    }
    // A line longer than a block inside the probe, and a BOM at its front.
    let mut long = base.clone();
    long[100] = format!(
        "{{\"id\":100,\"name\":\"{}\"}}",
        "y".repeat(WINDOW + WINDOW / 2)
    );
    let mut bom = b"\xef\xbb\xbf".to_vec();
    bom.extend(join(&base));
    for args in programs {
        same(args, &join(&long))?;
        same(args, &bom)?;
    }
    Ok(())
}
