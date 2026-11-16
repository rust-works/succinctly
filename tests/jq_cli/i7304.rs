//! #7304: the named pipes that feed the tests' runs (`tests/common/fifo_feed.rs`) never leave a test
//! waiting forever. A run held up by a write end that stays open fails at its deadline, naming the
//! pipe; a writer whose reader never came, or left early, ends when its feed is dropped; and every byte
//! still reaches a reader that reads to the end.

#![cfg(unix)]

use std::os::unix::fs::OpenOptionsExt;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::fifo_feed::{self, Feed};

/// A write end held open past the writer's close is the shape of the hang: the reader never sees its
/// end of file. Here a second descriptor (read and write, so it opens without a partner) holds it,
/// as a leaked one would; the run fails at its deadline instead of waiting forever, and says which
/// pipe it was in and that the end of file was handed again.
#[test]
fn a_write_end_left_open_fails_the_run_at_its_deadline_7304() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let fifo = dir.path().join("p00");
    let feed = Feed::new(fifo.clone(), b"{\"a\":1}\n".to_vec());
    let held = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&fifo)?;
    let start = Instant::now();
    let run = fifo_feed::output_within(
        Command::new(crate::succinctly_bin())
            .args(["jq", "-c", ".a"])
            .arg(&fifo),
        std::slice::from_ref(&feed),
        Duration::from_secs(2),
    );
    let waited = start.elapsed();
    drop(held);
    let message = format!(
        "{:#}",
        run.expect_err("the run cannot end while a write end is open")
    );
    assert!(waited < Duration::from_secs(20), "failed after {waited:?}");
    assert!(message.contains("did not end within 2s"), "{message}");
    assert!(
        message.contains("p00: written and closed, still held open by its reader"),
        "{message}"
    );
    assert!(!message.contains("handed again 0 times"), "{message}");
    Ok(())
}

/// Pipes the run never opens, and one it leaves part-way, do not keep their writers: dropping the feeds
/// returns at once (the clean-up this replaced opened each such pipe for reading, which waited forever
/// when the writer ended first).
#[test]
fn writers_end_when_the_run_never_opens_their_pipes_or_leaves_early_7304() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let big = vec![b'x'; 4 << 20];
    let feeds: Vec<Feed> = (0..4)
        .map(|n| Feed::new(dir.path().join(format!("p{n:02}")), big.clone()))
        .collect();
    let start = Instant::now();
    let output = fifo_feed::output(
        Command::new("head").args(["-c", "5"]).arg(feeds[0].path()),
        &feeds,
    )?;
    assert_eq!(output.stdout, b"xxxxx");
    drop(feeds);
    let waited = start.elapsed();
    assert!(
        waited < Duration::from_secs(20),
        "the feeds took {waited:?} to end"
    );
    Ok(())
}

/// Every byte reaches a reader that reads each pipe to its end, through more than a pipe holds.
#[test]
fn a_reader_of_several_pipes_gets_every_byte_7304() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let files: Vec<Vec<u8>> = (0..4u8)
        .map(|n| {
            (0..(1 << 20) + 7 * n as usize)
                .map(|i| (i % 251) as u8 ^ n)
                .collect()
        })
        .collect();
    let feeds: Vec<Feed> = files
        .iter()
        .enumerate()
        .map(|(n, bytes)| Feed::new(dir.path().join(format!("p{n:02}")), bytes.clone()))
        .collect();
    let output = fifo_feed::output(
        Command::new("cat").args(feeds.iter().map(Feed::path)),
        &feeds,
    )?;
    assert!(output.status.success());
    assert_eq!(output.stdout, files.concat());
    Ok(())
}
