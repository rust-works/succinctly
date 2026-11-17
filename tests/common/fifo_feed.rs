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
//!    kills it and fails with the state of every pipe that fed it. A run fed through an anonymous pipe
//!    on its standard input has the same hole (`wait_with_output` waits for as long as the child does),
//!    so [`output_stdin`] and [`spawn_jq_deadline`] give it the same deadline (#7314). Both retry as
//!    `spawn_with_signal_retry` does (a spawn that finds no binary, a child killed by a signal), and so
//!    does [`output_retrying`] for a run fed by named pipes, which makes new pipes for each attempt.

#![allow(dead_code)] // Each consumer uses a different subset.

use std::io::{ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use crate::cargo_run_exit::{exit_code_or_signal_death, succinctly_bin, MAX_CARGO_RETRIES};

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
const OPEN_FAILED: u8 = 5;

/// A named pipe and the thread that writes its bytes to whoever opens it, once.
pub struct Feed {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    state: Arc<AtomicU8>,
    kicks: Arc<AtomicU32>,
    errno: Arc<AtomicI32>,
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
        let errno = Arc::new(AtomicI32::new(0));
        let writer = {
            let (path, stop, state, kicks, errno) = (
                path.clone(),
                stop.clone(),
                state.clone(),
                kicks.clone(),
                errno.clone(),
            );
            std::thread::spawn(move || {
                std::thread::sleep(delay);
                let end = feed(&path, &bytes, &stop, &state, &kicks, &errno);
                state.store(end, Ordering::SeqCst);
            })
        };
        Self {
            path,
            stop,
            state,
            kicks,
            errno,
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
            // The writer could not open the pipe, which no reader can tell from a writer that never
            // came (`EMFILE` leaves the reader blocked in its open until the deadline): name the cause.
            OPEN_FAILED => format!(
                "writer could not open the pipe: {}",
                std::io::Error::from_raw_os_error(self.errno.load(Ordering::SeqCst))
            ),
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
fn feed(
    path: &Path,
    bytes: &[u8],
    stop: &AtomicBool,
    state: &AtomicU8,
    kicks: &AtomicU32,
    errno: &AtomicI32,
) -> u8 {
    let mut pipe = loop {
        match std::fs::OpenOptions::new().write(true).open(path) {
            Ok(pipe) => break pipe,
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            // The path is gone: nothing to feed.
            Err(e) if e.kind() == ErrorKind::NotFound => return STOPPED,
            // Anything else (`EMFILE`, `ENFILE`, not a pipe) is the writer's failure, not the run's:
            // keep the errno for the report, which would otherwise say only that the writer stopped.
            Err(e) => {
                errno.store(e.raw_os_error().unwrap_or(0), Ordering::SeqCst);
                return OPEN_FAILED;
            }
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
            // `ENXIO`: no reader holds the pipe any more. `ENOENT`: the pipe is gone with its directory.
            Err(e) if matches!(e.raw_os_error(), Some(libc::ENXIO | libc::ENOENT)) => {
                return RELEASED
            }
            Err(e) => {
                errno.store(e.raw_os_error().unwrap_or(0), Ordering::SeqCst);
                return OPEN_FAILED;
            }
        }
    }
}

/// `command.output()`, fed by `feeds`, within [`DEADLINE`]. Stdin is null, as `output` makes it.
pub fn output(command: &mut Command, feeds: &[Feed]) -> Result<Output> {
    output_within(command, feeds, DEADLINE)
}

/// [`output`] with another deadline. Past it the child is killed and the error names each pipe's state.
pub fn output_within(command: &mut Command, feeds: &[Feed], deadline: Duration) -> Result<Output> {
    run(command, feeds, Input::Null, deadline)
}

/// `command.output()` with `input` written to the child's standard input through an anonymous pipe,
/// within [`DEADLINE`] (#7314). Past it the child is killed and the run fails.
pub fn output_stdin(command: &mut Command, input: &[u8]) -> Result<Output> {
    output_stdin_within(command, input, DEADLINE)
}

/// [`output_stdin`] with another deadline.
pub fn output_stdin_within(
    command: &mut Command,
    input: &[u8],
    deadline: Duration,
) -> Result<Output> {
    run(command, &[], Input::Bytes(input), deadline)
}

/// [`output_stdin`] with `file` (a regular file, from wherever it has been positioned) as the child's
/// standard input, which is the windowed routes' input (#7314).
pub fn output_stdin_file(command: &mut Command, file: std::fs::File) -> Result<Output> {
    run(command, &[], Input::File(file), DEADLINE)
}

/// [`output`] for a run that may be killed by a signal (an OOM kill, a stall guard's process-group kill),
/// which `spawn_with_signal_retry` retries and a run fed by named pipes could not, since its pipes are
/// written once. `attempt` makes the run's command and a new set of feeds (new pipe paths: a feed is
/// spent by the run that read it) for each attempt, from 0. A run past its deadline is an error, not
/// retried: it was held up, not killed from outside.
pub fn output_retrying(
    mut attempt: impl FnMut(u32) -> (Command, Vec<Feed>),
) -> Result<(Output, i32)> {
    for n in 0..MAX_CARGO_RETRIES {
        let (mut command, feeds) = attempt(n);
        let output = output(&mut command, &feeds)?;
        drop(feeds);
        if let Some(code) = output.status.code() {
            return Ok((output, code));
        }
        if n + 1 >= MAX_CARGO_RETRIES {
            exit_code_or_signal_death(output.status, &output.stderr)?;
            unreachable!("exit_code_or_signal_death errors whenever status.code() is None");
        }
        std::thread::sleep(Duration::from_millis(100 * (u64::from(n) + 1)));
    }
    unreachable!()
}

/// `succinctly jq ARGS`, with `stdin_input` on an anonymous pipe when `Some` (null when `None`), the
/// binary's `LC_ALL=C` run of the `spawn_jq` in `jq_cli_tests.rs` (the same retries on a missing binary and
/// on a signal death) held to [`DEADLINE`] as `spawn_jq` is not (#7314).
pub fn spawn_jq_deadline(args: &[&str], stdin_input: Option<&[u8]>) -> Result<(Output, i32)> {
    for n in 0..MAX_CARGO_RETRIES {
        let mut command = Command::new(succinctly_bin());
        command.arg("jq").args(args).env("LC_ALL", "C");
        let input = stdin_input.map_or(Input::Null, Input::Bytes);
        let output = run(&mut command, &[], input, DEADLINE)?;
        if let Some(code) = output.status.code() {
            return Ok((output, code));
        }
        if n + 1 >= MAX_CARGO_RETRIES {
            exit_code_or_signal_death(output.status, &output.stderr)?;
            unreachable!("exit_code_or_signal_death errors whenever status.code() is None");
        }
        std::thread::sleep(Duration::from_millis(100 * (u64::from(n) + 1)));
    }
    unreachable!()
}

/// `Command::spawn`, trying again after a `NotFound` (a sibling `cargo test` can be relinking the binary
/// at this path, #550) as `spawn_with_signal_retry` does: nothing has been fed yet, so a new attempt is
/// the same run.
fn spawn(command: &mut Command) -> Result<Child> {
    for attempt in 0..MAX_CARGO_RETRIES {
        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(e) if e.kind() == ErrorKind::NotFound && attempt + 1 < MAX_CARGO_RETRIES => {
                std::thread::sleep(Duration::from_millis(100 * (u64::from(attempt) + 1)));
            }
            Err(e) => return Err(anyhow::Error::new(e).context("spawn succinctly")),
        }
    }
    unreachable!()
}

/// What a run's standard input is.
enum Input<'a> {
    Null,
    /// Bytes written to an anonymous pipe.
    Bytes(&'a [u8]),
    /// A file the child is handed as it is.
    File(std::fs::File),
}

/// The run: spawn, feed `input` to the standard input, drain stdout and stderr, and wait for the child no
/// longer than `deadline`.
fn run(
    command: &mut Command,
    feeds: &[Feed],
    input: Input<'_>,
    deadline: Duration,
) -> Result<Output> {
    let input = match input {
        Input::Null => {
            command.stdin(Stdio::null());
            None
        }
        Input::File(file) => {
            command.stdin(file);
            None
        }
        Input::Bytes(bytes) => {
            command.stdin(Stdio::piped());
            Some(bytes)
        }
    };
    let mut child = spawn(command.stdout(Stdio::piped()).stderr(Stdio::piped()))?;
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
    // On its own thread, so a child that writes while it is still reading cannot deadlock against it
    // (#7024). The thread ends when the child has read everything or is gone (`EPIPE`).
    let writer = input.map(|input| {
        let (mut stdin, input) = (child.stdin.take().expect("piped"), input.to_vec());
        std::thread::spawn(move || stdin.write_all(&input))
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > deadline {
            // Before the kill: the writer ends once the child is gone.
            let unread = writer.as_ref().is_some_and(|w| !w.is_finished());
            let _ = child.kill();
            let _ = child.wait();
            let mut report: Vec<String> = feeds.iter().map(Feed::describe).collect();
            if unread {
                report.push(
                    "its standard input: bytes still unwritten, the child not reading them"
                        .to_string(),
                );
            }
            bail!(
                "{command:?} did not end within {deadline:?} and was killed; its inputs:\n  {}",
                report.join("\n  ")
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
    if let Some(writer) = writer {
        while !writer.is_finished() {
            if start.elapsed() > deadline {
                bail!("{command:?} ended, but its standard input stayed open past {deadline:?}");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // A child that ended without reading its input closed the pipe: that is the run's business, as
        // `write_stdin_then_wait` takes it. Any other failure is not.
        match writer.join().expect("stdin writer") {
            Err(e) if e.kind() != ErrorKind::BrokenPipe => {
                return Err(anyhow::Error::new(e).context("write stdin"))
            }
            _ => {}
        }
    }
    Ok(Output {
        status,
        stdout: stdout.join().expect("stdout reader"),
        stderr: stderr.join().expect("stderr reader"),
    })
}
