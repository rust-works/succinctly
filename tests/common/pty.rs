//! A pseudo-terminal for CLI tests of input that is a terminal, and a deadline for the run it feeds.
//!
//! A test includes it from its per-issue file (`#[cfg(unix)] #[path = "../common/pty.rs"] mod pty;`),
//! so no shared list is edited. The terminal is opened in the test with `openpty(3)` (Linux and macOS
//! both have it, and name the device differently: `/dev/pts/N` and `/dev/ttysNNN`), never through
//! `script(1)`, whose options differ between the two. Its line discipline is the real one: what is typed
//! into the master side reaches a reader of the slave side a line at a time, and a `^D` ([`Pty::eof`])
//! at the start of a line is a read of zero bytes, once, as on a terminal a person types into (a `^D`
//! after some text of a line hands that text over without a newline). The slave side can be the run's
//! standard input ([`Pty::stdin`]) or named on its command line ([`Pty::path`]).
//!
//! Every run goes through [`Running`], which waits for the child within a deadline and kills it past
//! that, so a run that waits for more input than it is given fails instead of hanging the suite. A
//! child that reads a device that never ends can also be bounded in memory ([`Running::wait_within`]).

#![allow(dead_code)] // Each consumer uses a different subset.
#![allow(unsafe_code)] // STYLE-0004: `openpty`, `tcgetattr`/`tcsetattr`, `ttyname_r` and `fcntl` on descriptors the test owns.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How long a run fed by a terminal may take before it is killed and the test fails. A run of these
/// tests takes well under a second alone and a few seconds on a loaded machine.
pub const DEADLINE: Duration = Duration::from_secs(60);

/// A pseudo-terminal: the master side, which the test types into, and the slave side, which the run
/// reads, with echo off (nothing typed comes back on the master side to fill it).
pub struct Pty {
    master: File,
    slave: File,
    path: PathBuf,
}

impl Pty {
    pub fn open() -> Self {
        let mut master = -1;
        let mut slave = -1;
        // glibc declares the termios and window size `const`, Apple's libc does not.
        #[cfg(target_os = "linux")]
        let (termios, size) = (std::ptr::null(), std::ptr::null());
        #[cfg(not(target_os = "linux"))]
        let (termios, size) = (std::ptr::null_mut(), std::ptr::null_mut());
        // SAFETY: `openpty` writes the two descriptors; the name, termios and window size are not asked for.
        let made =
            unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), termios, size) };
        assert_eq!(made, 0, "openpty: {}", std::io::Error::last_os_error());
        // SAFETY: both descriptors were just opened and are owned by nothing else.
        let (master, slave) = unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
        // SAFETY: `termios` is written by `tcgetattr` before it is read, and only on success.
        unsafe {
            let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
            assert_eq!(libc::tcgetattr(slave.as_raw_fd(), termios.as_mut_ptr()), 0);
            let mut termios = termios.assume_init();
            termios.c_lflag &= !libc::ECHO;
            assert_eq!(
                libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &termios),
                0
            );
        }
        let mut name = [0u8; 256];
        // SAFETY: `name` is valid for writes of its length.
        let named =
            unsafe { libc::ttyname_r(slave.as_raw_fd(), name.as_mut_ptr().cast(), name.len()) };
        assert_eq!(named, 0, "ttyname_r");
        let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
        let path = PathBuf::from(String::from_utf8_lossy(&name[..end]).into_owned());
        Self {
            master,
            slave,
            path,
        }
    }

    /// The slave side's device path, for a run that names the terminal as an input file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The slave side as a run's standard input.
    pub fn stdin(&self) -> Stdio {
        Stdio::from(self.slave.try_clone().expect("dup the terminal"))
    }

    /// Types `bytes` into the terminal.
    pub fn type_bytes(&self, bytes: &[u8]) {
        (&self.master)
            .write_all(bytes)
            .expect("type into the terminal");
    }

    /// Types `^D`: the end of the line typed so far, which a reader gets without a newline, or, at
    /// the start of a line, one read of zero bytes.
    pub fn eof(&self) {
        self.type_bytes(&[4]);
    }

    /// A thread that types `line` into the terminal again and again until the returned flag is set
    /// (or the terminal is closed): a terminal that streams lines, as a serial device or a program
    /// writing to a terminal does. The master side is made non-blocking, so the thread never waits
    /// on a reader that has gone.
    pub fn stream(&self, line: &'static [u8]) -> Streamer {
        let master = self.master.try_clone().expect("dup the master side");
        // SAFETY: `fcntl` on a descriptor this test owns.
        unsafe {
            let flags = libc::fcntl(master.as_raw_fd(), libc::F_GETFL);
            libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut master = master;
                let mut at = 0;
                while !stop.load(Ordering::SeqCst) {
                    match master.write(&line[at..]) {
                        Ok(n) => at = (at + n) % line.len(),
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
            })
        };
        Streamer {
            stop,
            thread: Some(thread),
        }
    }
}

/// The thread [`Pty::stream`] started; dropping it stops the thread.
pub struct Streamer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Streamer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// What a finished run printed and how it ended.
#[derive(Debug)]
pub struct Ran {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl Ran {
    /// `(stdout, stderr, exit code)`; a run killed by a signal fails the test.
    pub fn triple(&self) -> (String, String, i32) {
        let code = self
            .status
            .code()
            .unwrap_or_else(|| panic!("killed by a signal: {:?} ({self:?})", self.status));
        (self.stdout.clone(), self.stderr.clone(), code)
    }
}

/// A child whose output is read as it comes and which is killed when dropped before it ended.
pub struct Running {
    child: Child,
    stdout: Option<JoinHandle<Vec<u8>>>,
    stderr: Option<JoinHandle<Vec<u8>>>,
}

fn drain(mut from: impl Read + Send + 'static) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = from.read_to_end(&mut bytes);
        bytes
    })
}

/// Starts `command` with its standard output and error piped; standard input is whatever the command
/// was given (null when nothing was).
pub fn spawn(command: &mut Command) -> Running {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the run");
    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);
    Running {
        child,
        stdout,
        stderr,
    }
}

/// The resident memory of process `pid`, in KiB, by `ps` (Linux and macOS both have `-o rss=`).
fn resident_kib(pid: u32) -> Option<u64> {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

impl Running {
    /// Whether the run has ended within `wait`, without killing it if it has not.
    pub fn ended_within(&mut self, wait: Duration) -> bool {
        let start = Instant::now();
        loop {
            if self.child.try_wait().expect("poll the run").is_some() {
                return true;
            }
            if start.elapsed() >= wait {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Waits for the run within [`DEADLINE`] and returns its output; past the deadline it is killed and
    /// the test fails, naming `what`.
    pub fn wait(self, what: &str) -> Ran {
        self.wait_within(what, DEADLINE, None)
    }

    /// [`Running::wait`] within `deadline`, and, with `max_kib`, killed and failed as soon as its resident
    /// memory passes that (a run that reads an endless device whole would otherwise take the machine's).
    pub fn wait_within(mut self, what: &str, deadline: Duration, max_kib: Option<u64>) -> Ran {
        let start = Instant::now();
        let mut checked = Instant::now();
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("poll the run") {
                break status;
            }
            if let Some(max) = max_kib {
                if checked.elapsed() >= Duration::from_millis(100) {
                    checked = Instant::now();
                    if let Some(kib) = resident_kib(self.child.id()) {
                        if kib > max {
                            let _ = self.child.kill();
                            let _ = self.child.wait();
                            panic!("{what}: holds {kib} KiB, past {max} KiB: killed");
                        }
                    }
                }
            }
            if start.elapsed() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("{what}: still running after {deadline:?}: killed");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let stdout = self.stdout.take().map(|t| t.join().unwrap_or_default());
        let stderr = self.stderr.take().map(|t| t.join().unwrap_or_default());
        Ran {
            status,
            stdout: String::from_utf8_lossy(&stdout.unwrap_or_default()).into_owned(),
            stderr: String::from_utf8_lossy(&stderr.unwrap_or_default()).into_owned(),
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
