//! #7277: the 4 MiB a `--seq` pipe's probe read are handed out a block of records at a time from the
//! buffer they were read into. Each row puts what a window boundary inside the probe can meet (a record
//! over several lines, a record longer than a block and than the probe, a record that is not plain
//! framing, a record with no LF, `RS RS`, bytes before the first RS, bytes that are not UTF-8) at several
//! places in the probe, and checks the run against the same bytes read whole: the pipe named as `-`,
//! which the windowed routes decline (#7164's reference), so stdout, stderr and the exit status are the
//! whole-stream route's.

#![cfg(unix)]

use anyhow::Result;

use crate::fifo_feed::spawn_jq_deadline;

const MIB: usize = 1 << 20;

fn record(i: usize) -> String {
    format!(
        "\x1e{{\"id\":{i},\"name\":\"user {i}\",\"pad\":\"{}\"}}\n",
        "x".repeat(60)
    )
}

/// About 6 MiB of plain records, as a list so a record can be swapped.
fn records() -> Vec<String> {
    (0..60_000).map(record).collect()
}

/// The windowed run over `bytes` on a pipe, checked against the whole run.
fn same(args: &[&str], bytes: &[u8]) -> Result<()> {
    assert!(
        bytes.len() > 4 * MIB,
        "the pipe must hold enough to be windowed"
    );
    let (windowed, windowed_status) = spawn_jq_deadline(args, Some(bytes))?;
    let mut whole_args = args.to_vec();
    whole_args.push("-");
    let (whole, whole_status) = spawn_jq_deadline(&whole_args, Some(bytes))?;
    assert!(
        windowed.stdout == whole.stdout,
        "{args:?}: stdout differs ({} and {} bytes)",
        windowed.stdout.len(),
        whole.stdout.len()
    );
    assert_eq!(
        String::from_utf8_lossy(&windowed.stderr),
        String::from_utf8_lossy(&whole.stderr),
        "{args:?}: stderr"
    );
    assert_eq!(windowed_status, whole_status, "{args:?}: exit status");
    Ok(())
}

/// The record that holds byte `at` of the joined records.
fn record_at(records: &[String], at: usize) -> usize {
    let mut end = 0;
    records
        .iter()
        .position(|r| {
            end += r.len();
            end > at
        })
        .expect("in the stream")
}

#[test]
fn what_a_window_of_a_seq_probe_meets_reads_as_the_whole_stream_does_7277() -> Result<()> {
    let base = records();
    let programs: [&[&str]; 2] = [&["--seq", "-c", "."], &["--seq", "-c", ".id?"]];
    let multi_line = |i: usize| format!("\x1e{{\"id\":{i},\n \"v\":[1,\n2,\n3]}}\n");
    let long = |len: usize| format!("\x1e[{}0]\n", "1,\n".repeat(len / 3));
    let mut shapes: Vec<Vec<u8>> = Vec::new();
    for at in [MIB - 30, 2 * MIB + 7, 4 * MIB - 40] {
        let n = record_at(&base, at);
        let with = |swap: String| -> Vec<u8> {
            let mut r = base.clone();
            r[n] = swap;
            r.concat().into_bytes()
        };
        shapes.push(with(multi_line(n)));
        shapes.push(with(long(MIB + MIB / 2)));
        // Not plain framing: no LF before the next RS, an empty record, a fault, invalid UTF-8.
        shapes.push(with(format!("\x1e{{\"id\":{n}}}")));
        shapes.push(with("\x1e\x1e{\"id\":0}\n".to_string()));
        shapes.push(with(format!("\x1e{{\"id\":{n},\"t\":[1 2]}}\n")));
        let mut bad = with(String::new());
        let start = base[..n].iter().map(String::len).sum::<usize>();
        bad.splice(start..start, *b"\x1e{\"s\":\"\xff\xfe\"}\n");
        shapes.push(bad);
    }
    // A record longer than the probe, and bytes before the first RS.
    let mut longer = base.clone();
    longer[10] = long(5 * MIB);
    shapes.push(longer.concat().into_bytes());
    let mut before = b"junk\n".to_vec();
    before.extend(base.concat().as_bytes());
    shapes.push(before);
    for shape in &shapes {
        for args in programs {
            same(args, shape)?;
        }
    }
    Ok(())
}
