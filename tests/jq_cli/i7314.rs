//! #7314: the follow-ups to the FIFO test helper (`tests/common/fifo_feed.rs`, #7304). A run fed
//! through an anonymous pipe on its standard input has the helper's deadline too; a run that finds no
//! binary, or is killed by a signal, is tried again as `spawn_with_signal_retry` does; and a writer that
//! cannot open its pipe says why.

#![cfg(unix)]

use std::cell::Cell;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::fifo_feed::{self, Feed};

/// The deadline of a run fed through an anonymous pipe can fire: a child that never reads its input
/// leaves the writer blocked on a full pipe, and the run is killed and fails at its deadline, saying
/// the input was left unwritten, instead of waiting for as long as the child does.
#[test]
fn a_child_that_never_reads_its_stdin_fails_the_run_at_its_deadline_7314() -> Result<()> {
    let big = vec![b'x'; 4 << 20];
    let start = Instant::now();
    let run = fifo_feed::output_stdin_within(
        Command::new("sleep").arg("600"),
        &big,
        Duration::from_secs(2),
    );
    let waited = start.elapsed();
    let message = format!("{:#}", run.expect_err("the child never reads its input"));
    assert!(waited < Duration::from_secs(20), "failed after {waited:?}");
    assert!(message.contains("did not end within 2s"), "{message}");
    assert!(
        message.contains("standard input: bytes still unwritten"),
        "{message}"
    );
    Ok(())
}

/// The bytes written to a pipe on standard input all arrive, through more than a pipe holds.
#[test]
fn a_run_fed_on_stdin_gets_every_byte_7314() -> Result<()> {
    let bytes: Vec<u8> = (0..(3 << 20)).map(|i| (i % 251) as u8).collect();
    let output = fifo_feed::output_stdin(&mut Command::new("cat"), &bytes)?;
    assert!(output.status.success());
    assert_eq!(output.stdout, bytes);
    Ok(())
}

/// `spawn_jq_deadline` answers as `spawn_jq` does: the run, and its exit status.
#[test]
fn spawn_jq_deadline_runs_the_binary_on_its_stdin_7314() -> Result<()> {
    let (output, status) = fifo_feed::spawn_jq_deadline(&["-c", ".a"], Some(b"{\"a\":[1,2]}\n"))?;
    assert_eq!((output.stdout.as_slice(), status), (&b"[1,2]\n"[..], 0));
    let (output, status) = fifo_feed::spawn_jq_deadline(&["-n", "1+1"], None)?;
    assert_eq!((output.stdout.as_slice(), status), (&b"2\n"[..], 0));
    Ok(())
}

/// A child killed by a signal is run again, with the feeds the attempt made, and the second attempt's
/// answer is the run's; one killed every time fails naming the signal.
#[test]
fn a_run_killed_by_a_signal_is_tried_again_7314() -> Result<()> {
    let attempts = Cell::new(0);
    let (output, status) = fifo_feed::output_retrying(|attempt| {
        attempts.set(attempts.get() + 1);
        let mut command = Command::new("sh");
        command.arg("-c").arg(if attempt == 0 {
            "kill -KILL $$"
        } else {
            "echo second"
        });
        (command, Vec::new())
    })?;
    assert_eq!(
        (output.stdout.as_slice(), status, attempts.get()),
        (&b"second\n"[..], 0, 2)
    );

    let attempts = Cell::new(0);
    let err = fifo_feed::output_retrying(|_| {
        attempts.set(attempts.get() + 1);
        let mut command = Command::new("sh");
        command.arg("-c").arg("kill -KILL $$");
        (command, Vec::new())
    })
    .expect_err("killed on every attempt");
    assert_eq!(attempts.get(), crate::MAX_CARGO_RETRIES);
    assert!(format!("{err:#}").contains("signal"), "{err:#}");
    Ok(())
}

/// A binary that is not there is looked for again after 100 ms and 200 ms (as `spawn_with_signal_retry`
/// does, for a binary a sibling `cargo test` is relinking) before the spawn fails.
#[test]
fn a_spawn_that_finds_no_binary_is_tried_again_7314() {
    let start = Instant::now();
    let err = fifo_feed::output(&mut Command::new("/nonexistent/succinctly-7314"), &[])
        .expect_err("there is no such binary");
    let waited = start.elapsed();
    assert!(
        waited >= Duration::from_millis(300),
        "failed after {waited:?}"
    );
    assert!(format!("{err:#}").contains("spawn succinctly"), "{err:#}");
}

/// A writer that cannot open its pipe reports the `errno`, not only that it stopped: here the path
/// holds a directory by the time the writer opens it (`EISDIR`, where `EMFILE` would look the same to
/// the reader, blocked in its own open).
#[test]
fn a_writer_that_cannot_open_its_pipe_reports_why_7314() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("p00");
    let feed = Feed::after(path.clone(), b"x".to_vec(), Duration::from_millis(500));
    std::fs::remove_file(&path)?;
    std::fs::create_dir(&path)?;
    let start = Instant::now();
    let said = loop {
        let said = feed.describe();
        if !said.contains("never opened by a reader") || start.elapsed() > Duration::from_secs(20) {
            break said;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        said.contains("writer could not open the pipe: Is a directory"),
        "{said}"
    );
    Ok(())
}
