//! #7266: the 4 MiB a pipe's probe read are handed out a block (1 MiB) at a time from the buffer they
//! were read into, where they used to be one window with a block read onto them. Each row puts what a
//! window boundary can cut inside the probe (a fault, bytes that are not UTF-8, a document over several
//! lines, a line longer than a block, a BOM) and checks the run against the same bytes read whole: the
//! pipe named as `/dev/stdin`, which the windowed routes decline, so stdout, stderr (with that name read
//! as `<stdin>`) and the exit status are the whole-file route's.

#![cfg(unix)]

use anyhow::Result;

use crate::spawn_jq;

/// About 6 MiB of records, so the probe is full and the pipe goes on past it.
fn records() -> Vec<String> {
    (0..60_000)
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
    let (windowed, windowed_status) = spawn_jq(args, Some(bytes))?;
    let mut whole_args = args.to_vec();
    whole_args.push("/dev/stdin");
    let (whole, whole_status) = spawn_jq(&whole_args, Some(bytes))?;
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

const MIB: usize = 1 << 20;

#[test]
fn what_a_window_of_the_probe_cuts_reads_as_the_whole_stream_does_7266() -> Result<()> {
    let base = records();
    let programs: [&[&str]; 3] = [
        &["-c", "."],
        &["-r", "select(.id % 7919 == 0) | .name"],
        &["-c", "select(.id == 59999) | .id"],
    ];
    // Near each block's end inside the probe, and in its last block.
    for at in [MIB - 40, 2 * MIB + 7, 4 * MIB - 30] {
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
        // Past the two bytes put in above.
        let start = join(&base[..m]).len() + 2;
        lossy.splice(start + 1..start + 1, *b"\"q\":[1 2],");
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
    // A line longer than a block inside the probe, and a BOM at its front.
    let mut long = base.clone();
    long[100] = format!("{{\"id\":100,\"name\":\"{}\"}}", "y".repeat(MIB + MIB / 2));
    let mut bom = b"\xef\xbb\xbf".to_vec();
    bom.extend(join(&base));
    for args in programs {
        same(args, &join(&long))?;
        same(args, &bom)?;
    }
    Ok(())
}
