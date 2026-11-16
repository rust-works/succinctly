//! Named pipes that feed a CLI run its input, for tests that need an operand that is not a regular
//! file (the route that reads each operand whole), and a deadline for the run they feed (#7304).
//!
//! Included by `jq_cli_tests.rs` (`#[path = "common/fifo_feed.rs"] mod fifo_feed;`). Three things
//! made the hand-rolled version of this hang a test until CI's 6-hour job limit:
//!
//! 1. **macOS can lose a pipe's end of file.** Under load, the reader of a named pipe has read every
//!    byte, the only writer has closed it, no process holds a write end, and `read` still blocks
//!    (about one pipe in 40,000 at a load average of 30-50, with or without a child process in the
//!    way). Opening the pipe for writing and closing it again hands the reader its end of file. So a
//!    writer here, once it has written and closed, keeps doing that ([`KICK`]) until no reader holds
//!    the pipe; it adds no bytes, so a reader that did see its end of file sees nothing new.
//! 2. **A blocking open waits forever when the other side never comes.** The old clean-up opened a
//!    pipe for reading when its writer had not finished, which blocked for good when the writer
//!    finished in between. Here the test side never opens a pipe in a way that can block: dropping a
//!    [`Feed`] opens it for reading with `O_NONBLOCK`, which lets a writer still waiting in its own
//!    (blocking) open go on and see that it is to stop, and does so until the writer has ended.
//!    The writer waits in the kernel, not in a polling loop: forty writers polling every half
//!    millisecond for each of several tests at once took a laptop's load average past 500.
//! 3. **`Command::output` has no deadline.** [`output`] waits for the child for [`DEADLINE`], then
//!    kills it and fails with the state of every pipe that fed it.

#![allow(dead_code)] // Each consumer uses a different subset.

use std::io::{ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

/// How long a pipe-fed run may take before it is killed and the test fails. A run of these tests takes
/// well under a second alone and a few seconds on a loaded machine.
pub const DEADLINE: Duration = Duration::from_secs(60);

/// How often a writer that has written everything hands its reader the end of file again (1).
pub const KICK: Duration = Duration::from_millis(50);

const WAITING: u8 = 0;
const WRITING: u8 = 1;
const WRITTEN: u8 = 2;
const RELEASED: u8 = 3;
const STOPPED: u8 = 4;

/// A named pipe and the thread that writes its bytes to whoever opens it, once.
pub struct Feed {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    state: Arc<AtomicU8>,
    kicks: Arc<AtomicU32>,
    writer: Option<JoinHandle<()>>,
}

impl Feed {
    /// Makes the pipe at `path` (with `mkfifo`) and starts writing `bytes` to its reader.
    pub fn new(path: PathBuf, bytes: Vec<u8>) -> Self {
        Self::after(path, bytes, Duration::ZERO)
    }

    /// As [`Feed::new`], with the writer's first open made only after `delay`.
    pub fn after(path: PathBuf, bytes: Vec<u8>, delay: Duration) -> Self {
        let made = Command::new("mkfifo")
            .arg(&path)
            .status()
            .expect("run mkfifo");
        assert!(made.success(), "mkfifo {}", path.display());
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(AtomicU8::new(WAITING));
        let kicks = Arc::new(AtomicU32::new(0));
        let writer = {
            let (path, stop, state, kicks) =
                (path.clone(), stop.clone(), state.clone(), kicks.clone());
            std::thread::spawn(move || {
                std::thread::sleep(delay);
                let end = feed(&path, &bytes, &stop, &state, &kicks);
                state.store(end, Ordering::SeqCst);
            })
        };
        Self {
            path,
            stop,
            state,
            kicks,
            writer: Some(writer),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How many times the writer has handed a reader that still held the pipe its end of file again.
    pub fn kicks(&self) -> u32 {
        self.kicks.load(Ordering::SeqCst)
    }

    /// The pipe's name and how far its writer got.
    pub fn describe(&self) -> String {
        let name = self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let state = match self.state.load(Ordering::SeqCst) {
            WAITING => "never opened by a reader".to_string(),
            WRITING => "being written, the reader still reading".to_string(),
            WRITTEN => format!(
                "written and closed, still held open by its reader (end of file handed again {} times)",
                self.kicks.load(Ordering::SeqCst)
            ),
            RELEASED => "let go by its reader".to_string(),
            _ => "writer stopped".to_string(),
        };
        format!("{name}: {state}")
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let Some(writer) = self.writer.take() else {
            return;
        };
        // A writer whose reader never came waits in its open, and one whose reader left mid-write ends
        // on its own: a reader opened without blocking ends the wait, and nothing here can block on a
        // writer that has already gone (2).
        let start = Instant::now();
        while !writer.is_finished() {
            let reader = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&self.path);
            std::thread::sleep(Duration::from_millis(1));
            drop(reader);
            if start.elapsed() > DEADLINE {
                // Not a panic: this may run while a failed test unwinds.
                eprintln!(
                    "the writer of {} did not end; left running",
                    self.path.display()
                );
                return;
            }
        }
        let _ = writer.join();
    }
}

fn open_for_writing(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
}

/// The writer: open once a reader has the pipe (a blocking open, which [`Feed`]'s drop can end), write
/// everything, close, then hand the reader its end of file again until it lets go. Returns the state it
/// ended in.
fn feed(path: &Path, bytes: &[u8], stop: &AtomicBool, state: &AtomicU8, kicks: &AtomicU32) -> u8 {
    let mut pipe = loop {
        match std::fs::OpenOptions::new().write(true).open(path) {
            Ok(pipe) => break pipe,
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            // The path is gone, or not a pipe: nothing to feed.
            Err(_) => return STOPPED,
        }
    };
    if stop.load(Ordering::SeqCst) {
        // Opened by the drop's reader: the run never came for these bytes.
        return STOPPED;
    }
    state.store(WRITING, Ordering::SeqCst);
    if pipe.write_all(bytes).is_err() {
        // `EPIPE`: the reader left before the end, which is the run's business.
        return RELEASED;
    }
    drop(pipe);
    state.store(WRITTEN, Ordering::SeqCst);
    loop {
        std::thread::sleep(KICK);
        if stop.load(Ordering::SeqCst) {
            return STOPPED;
        }
        match open_for_writing(path) {
            Ok(again) => {
                drop(again);
                kicks.fetch_add(1, Ordering::SeqCst);
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            // `ENXIO`: no reader holds the pipe any more.
            Err(_) => return RELEASED,
        }
    }
}

/// `command.output()`, fed by `feeds`, within [`DEADLINE`]. Stdin is null, as `output` makes it.
pub fn output(command: &mut Command, feeds: &[Feed]) -> Result<Output> {
    output_within(command, feeds, DEADLINE)
}

/// [`output`] with another deadline. Past it the child is killed and the error names each pipe's state.
pub fn output_within(command: &mut Command, feeds: &[Feed], deadline: Duration) -> Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let drain = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = drain(child.stdout.take().map(|p| Box::new(p) as _));
    let stderr = drain(child.stderr.take().map(|p| Box::new(p) as _));
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            let pipes: Vec<String> = feeds.iter().map(Feed::describe).collect();
            bail!(
                "{command:?} did not end within {deadline:?} and was killed; its named pipes:\n  {}",
                pipes.join("\n  ")
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    // The child is gone; a copy of its stdout or stderr held by another process would keep these open.
    while !(stdout.is_finished() && stderr.is_finished()) {
        if start.elapsed() > deadline {
            bail!("{command:?} ended, but its stdout or stderr stayed open past {deadline:?}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(Output {
        status,
        stdout: stdout.join().expect("stdout reader"),
        stderr: stderr.join().expect("stderr reader"),
    })
}
