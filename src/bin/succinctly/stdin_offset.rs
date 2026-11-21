//! Where `succinctly jq` leaves a regular file on standard input that it did not read to the end
//! (#7455).
//!
//! jq reads its input through stdio, one `fgets` chunk at a time (a line, or 4095 bytes of a longer
//! one), and stdio puts a seekable descriptor back at the end of what it handed out when the
//! process exits. So a run that stops early leaves the file just past the chunk that completed the
//! last item it read: `{ jq -n input; jq -n input; } < f` reads the first document, then the
//! second. A run that read to the end leaves it there, and so does one that met a parse error
//! (where jq's offset is the error's chunk, which this does not model).
//!
//! The routes read standard input in windows or whole, well past what the program takes, so the
//! offset is put back once, when the run ends: the items the program took are counted again in
//! the file, from where the run found it, and the descriptor is moved to the end of the chunk that
//! completed the last of them. Standard input read as a `-` among input files is counted after the
//! files named before it, read again from their paths (#7568). A run that never reads standard
//! input, reads it to its end, or
//! reads a pipe makes no system call for it here; a pipe costs one `fstat` once it has stopped
//! early.

#![allow(unsafe_code)] // `read`, `fstat`, `lseek` and `pread` on descriptor 0
#![cfg_attr(not(unix), allow(dead_code))] // only Unix descriptors have an offset to put back

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

/// What one item the program reads is, for counting the items again in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Items {
    /// A JSON value: the plain reader's input.
    Values,
    /// A `--stream` event: a scalar, an empty container, or the close of a container.
    Events,
    /// A `-R` line.
    Lines,
}

/// How the run reads standard input, set once its input files are known.
#[derive(Clone, Copy, Debug)]
struct Run {
    items: Items,
    /// Whether the main loop reads the inputs (no `-n`): it reads on to the end unless the program
    /// halts.
    main_loop: bool,
}

thread_local! {
    static RUN: Cell<Option<Run>> = const { Cell::new(None) };
    /// The offset standard input stood at when a route that reads it by a duplicate of the
    /// descriptor looked (`regular_stdin_left`).
    static ORIGIN: Cell<Option<u64>> = const { Cell::new(None) };
    /// The bytes [`Stdin`] read.
    static READ: Cell<u64> = const { Cell::new(0) };
    /// The inputs the main loop handed to the program, on the routes that do not read them
    /// through the `input` queue.
    static TAKEN: Cell<u64> = const { Cell::new(0) };
    static HALTED: Cell<bool> = const { Cell::new(false) };
    /// The input files named before the first `-`, when standard input is read as `-` among them
    /// (#7568): jq's reader gets to standard input only once it has read these.
    static BEFORE: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}

/// Forgets everything an earlier run on this thread recorded.
pub(crate) fn start_run() {
    RUN.with(|r| r.set(None));
    BEFORE.with(|b| b.borrow_mut().clear());
    ORIGIN.with(|o| o.set(None));
    READ.with(|r| r.set(0));
    TAKEN.with(|t| t.set(0));
    HALTED.with(|h| h.set(false));
    succinctly::jq::clear_input_reads();
}

/// Records how the run reads standard input: `None` when it reads none, or reads it in a way whose
/// offset is left where the read left it (named files with no `-`, `--slurp`, `--seq`, DSV,
/// `--stream-errors`). `files` are the input files: standard input is the first `-` among them,
/// or the only input when there are none.
pub(crate) fn set_run(items: Option<Items>, main_loop: bool, files: &[PathBuf]) {
    let items = match files.iter().position(|path| path.as_os_str() == "-") {
        Some(dash) => {
            BEFORE.with(|b| *b.borrow_mut() = files[..dash].to_vec());
            items
        }
        None if files.is_empty() => items,
        None => None,
    };
    RUN.with(|r| r.set(items.map(|items| Run { items, main_loop })));
}

/// Records the offset a regular file on standard input stood at before the run read it.
pub(crate) fn note_origin(at: u64) {
    ORIGIN.with(|o| {
        if o.get().is_none() {
            o.set(Some(at));
        }
    });
}

/// Counts one input the main loop hands to the program.
pub(crate) fn note_taken() {
    TAKEN.with(|t| t.set(t.get() + 1));
}

/// Records that the program halted (`halt`, `halt_error`).
pub(crate) fn note_halt() {
    HALTED.with(|h| h.set(true));
}

/// Standard input, read straight from descriptor 0 with no buffer of its own, so that the bytes it
/// has read are the bytes the descriptor moved by. A descriptor that was not opened for reading reads
/// as empty, as Rust's own standard input reads it.
pub(crate) struct Stdin;

impl std::io::Read for Stdin {
    #[cfg(unix)]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // macOS refuses a count above `INT_MAX`, as Rust's own standard input allows for.
        let len = buf.len().min(i32::MAX as usize - 1);
        // SAFETY: `buf` is valid for writes of `len` bytes.
        let n = unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), len) };
        if n < 0 {
            let error = std::io::Error::last_os_error();
            return if error.raw_os_error() == Some(libc::EBADF) {
                Ok(0)
            } else {
                Err(error)
            };
        }
        READ.with(|r| r.set(r.get() + n as u64));
        Ok(n as usize)
    }

    #[cfg(not(unix))]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::io::Read::read(&mut std::io::stdin(), buf)
    }
}

/// Puts a regular file on standard input back where jq would leave it, if the run stopped before
/// its end. Called once, when the run is over.
pub(crate) fn restore() {
    let Some(run) = RUN.with(Cell::get) else {
        return;
    };
    let origin = ORIGIN.with(Cell::get);
    let read = READ.with(Cell::get);
    if origin.is_none() && read == 0 {
        return;
    }
    let reads = succinctly::jq::input_reads();
    if reads.past_end || reads.fault {
        return;
    }
    // A main loop that does not halt reads to the end, unless a file before the `-` does not open:
    // jq's main loop then stops once it has the first input read past that file (#7568).
    let until_unopened = run.main_loop && !HALTED.with(Cell::get);
    if until_unopened && BEFORE.with(|b| b.borrow().is_empty()) {
        return;
    }
    #[cfg(unix)]
    restore_unix(
        run.items,
        reads.documents + TAKEN.with(Cell::get),
        until_unopened,
        origin,
        read,
    );
    #[cfg(not(unix))]
    let _ = (run.items, origin, read);
}

#[cfg(unix)]
fn restore_unix(items: Items, taken: u64, until_unopened: bool, origin: Option<u64>, read: u64) {
    // SAFETY: `stat` is written by `fstat` before it is read, and only on success.
    let Some(stat) = (unsafe {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        (libc::fstat(libc::STDIN_FILENO, stat.as_mut_ptr()) == 0).then(|| stat.assume_init())
    }) else {
        return;
    };
    if stat.st_mode & libc::S_IFMT != libc::S_IFREG {
        return;
    }
    // SAFETY: `lseek` reads and moves only the descriptor's offset.
    let Ok(now) = u64::try_from(unsafe { libc::lseek(libc::STDIN_FILENO, 0, libc::SEEK_CUR) })
    else {
        return;
    };
    let Some(start) = origin.or_else(|| now.checked_sub(read)) else {
        return;
    };
    let mut block = vec![0u8; 64 << 10];
    let mut scan = Scan::new(items, taken);
    // #7568: the files before a `-` are read first, by jq's reader and here.
    let before = BEFORE.with(|b| b.borrow().clone());
    let mut unopened = false;
    for path in &before {
        if scan.found().is_some() {
            break;
        }
        match scan_named_file(&mut scan, path, &mut block) {
            None => return,
            Some(opened) => unopened |= !opened,
        }
    }
    if until_unopened && !unopened {
        return;
    }
    if scan.found().is_some() {
        // The last item was handed out before jq's reader got to standard input.
        if start != now {
            if let Ok(start) = libc::off_t::try_from(start) {
                // SAFETY: as above.
                unsafe { libc::lseek(libc::STDIN_FILENO, start, libc::SEEK_SET) };
            }
        }
        return;
    }
    let stdin_at = scan.pos;
    let at_eof = u64::try_from(stat.st_size).is_ok_and(|size| size <= now);
    let mut at = start;
    let end = loop {
        if let Some(end) = scan.found() {
            break end;
        }
        if at >= now {
            if !at_eof {
                return;
            }
            scan.end_of_file();
            match scan.found() {
                Some(end) => break end,
                None => return,
            }
        }
        let want = usize::try_from(now - at).map_or(block.len(), |left| left.min(block.len()));
        let Ok(offset) = libc::off_t::try_from(at) else {
            return;
        };
        // SAFETY: `block` is valid for writes of `want` bytes.
        let n = unsafe { libc::pread(libc::STDIN_FILENO, block.as_mut_ptr().cast(), want, offset) };
        let Ok(n) = usize::try_from(n) else {
            return;
        };
        if n == 0 {
            return;
        }
        scan.feed(&block[..n]);
        at += n as u64;
    };
    let end = start + (end - stdin_at);
    if end != now {
        if let Ok(end) = libc::off_t::try_from(end) {
            // SAFETY: as above.
            unsafe { libc::lseek(libc::STDIN_FILENO, end, libc::SEEK_SET) };
        }
    }
}

/// Feeds `scan` an input file named before standard input's `-`, read again from its path, and ends
/// it there (#7568). A path that does not open, or a directory, is a source with no bytes, as jq's
/// reader reads it. The file standard input holds, named by its path, is opened anew, by jq as here,
/// so its reads do not move descriptor 0. `false` for anything whose bytes cannot be read again as
/// jq read them: a pipe or device, or a path naming a descriptor (`/dev/stdin`, `/dev/fd/N`), which
/// may share standard input's offset. `Some(false)` for a file jq's reader counts as an input error
/// (it does not open, or is a directory), which ends jq's main loop one input later.
#[cfg(unix)]
fn scan_named_file(scan: &mut Scan, path: &PathBuf, block: &mut [u8]) -> Option<bool> {
    use std::io::Read;
    if path.starts_with("/dev/fd") || path.starts_with("/dev/stdin") {
        return None;
    }
    let opened = match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => {
            let Ok(mut file) = std::fs::File::open(path) else {
                scan.end_of_file();
                return Some(false);
            };
            while scan.found().is_none() {
                match file.read(block) {
                    Ok(0) => break,
                    Ok(n) => scan.feed(&block[..n]),
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => return None,
                }
            }
            true
        }
        Ok(meta) if !meta.is_dir() => return None,
        _ => false,
    };
    if scan.found().is_none() {
        scan.end_of_file();
    }
    Some(opened)
}

/// jq's `fgets` chunk: a line, or this many bytes of a longer one.
const CHUNK: u64 = crate::cli::jq_seq_reader::JQ_FGETS_CHUNK as u64;

/// Counts the items of a text again, a block at a time, for where jq's reader stood once it had
/// handed out the last one the program took: the end of the `fgets` chunk whose byte completed it
/// (the closing bracket or quote, or the byte after a number or literal), or for `-R`, the end of
/// its line.
struct Scan {
    items: Items,
    /// The items still to complete.
    left: u64,
    /// The offset of the next byte, from the start of the text.
    pos: u64,
    /// Where the chunk holding `pos` starts.
    chunk_start: u64,
    /// The offset jq stands at, once the last item completed and its chunk ended.
    found: Option<u64>,
    /// Whether the last item has completed, and its chunk is being read to its end.
    completed: bool,
    /// The items the byte being read has completed so far.
    completed_here: u8,
    /// Whether the last item was not the first one its byte completed (the close after a scalar's
    /// `--stream` event), so that jq's parser handed it out with no byte left to read.
    completed_second: bool,
    in_string: bool,
    escape: bool,
    in_literal: bool,
    /// Whether the string being read is an object's key, which is no `--stream` event.
    string_is_key: bool,
    /// Whether the next string in the innermost object is a key.
    expect_key: bool,
    /// The open containers, innermost last: `true` for an object.
    open: Vec<bool>,
    /// Whether a scalar inside a container has been read and its `--stream` event not yet handed
    /// out: jq's streaming parser hands it out at the `,` or closer after it.
    pending: bool,
    /// Whether the bytes read so far are the front of a BOM, which jq's parser skips (and its
    /// chunk counts).
    in_bom: bool,
}

impl Scan {
    fn new(items: Items, taken: u64) -> Self {
        Self {
            items,
            left: taken,
            pos: 0,
            chunk_start: 0,
            found: (taken == 0).then_some(0),
            completed: false,
            completed_here: 0,
            completed_second: false,
            in_string: false,
            escape: false,
            in_literal: false,
            string_is_key: false,
            expect_key: false,
            open: Vec::new(),
            pending: false,
            in_bom: items != Items::Lines,
        }
    }

    fn found(&self) -> Option<u64> {
        self.found
    }

    fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if self.found.is_some() {
                return;
            }
            if self.completed {
                // The byte that completed the last item was not its chunk's last.
                self.completed_second = false;
                self.end_chunk_at(b);
                self.pos += 1;
                continue;
            }
            self.in_bom &= self.pos < 3 && b == b"\xef\xbb\xbf"[self.pos as usize];
            self.completed_here = 0;
            if !self.in_bom {
                match self.items {
                    Items::Lines => {
                        if b == b'\n' {
                            self.complete();
                        }
                    }
                    Items::Values | Items::Events => self.json_byte(b),
                }
            }
            if self.completed {
                self.end_chunk_at(b);
            } else if b == b'\n' || self.pos + 1 - self.chunk_start == CHUNK {
                self.chunk_start = self.pos + 1;
            }
            self.pos += 1;
        }
    }

    /// Ends the chunk once the last item has completed: at a newline, or after a chunk's worth of
    /// bytes (a `-R` line runs to its newline).
    ///
    /// jq's input loop reads the next chunk before it asks the parser for an item whenever the
    /// parser has no byte left, so an item handed out with none left (the second one the chunk's
    /// last byte completed: the close after a scalar's `--stream` event) stands at the end of the
    /// chunk after it (#7570).
    fn end_chunk_at(&mut self, b: u8) {
        let full = self.items != Items::Lines && self.pos + 1 - self.chunk_start == CHUNK;
        if full && std::mem::take(&mut self.completed_second) {
            self.chunk_start = self.pos + 1;
        } else if b == b'\n' || full {
            self.found = Some(self.pos + 1);
        }
    }

    /// The end of an input file: jq's reader moves on to the next one, whose first chunk is a new
    /// one (#7568). A last item completed in this file stands at its end, unless the file's last
    /// byte completed it with nothing left to read (the close after a scalar, as above): jq's
    /// reader then reads the next file's first chunk before it hands the item out.
    fn end_of_file(&mut self) {
        if self.found.is_some() {
            return;
        }
        if self.completed && !std::mem::take(&mut self.completed_second) {
            self.found = Some(self.pos);
        } else {
            self.chunk_start = self.pos;
        }
    }

    fn complete(&mut self) {
        self.left -= 1;
        self.completed_here += 1;
        if self.left == 0 {
            self.completed = true;
            self.completed_second = self.completed_here > 1;
        }
    }

    /// A scalar value has been read: a top-level one is an item, and a `--stream` event waits for
    /// the next `,` or closer.
    fn scalar(&mut self) {
        if self.open.is_empty() {
            self.complete();
        } else if self.items == Items::Events {
            self.pending = true;
        }
    }

    /// Hands out the `--stream` event of the scalar before a `,` or closer.
    fn flush(&mut self) {
        if std::mem::take(&mut self.pending) {
            self.complete();
        }
    }

    fn json_byte(&mut self, b: u8) {
        if self.in_string {
            if self.escape {
                self.escape = false;
            } else if b == b'\\' {
                self.escape = true;
            } else if b == b'"' {
                self.in_string = false;
                if !self.string_is_key {
                    self.scalar();
                }
            }
            return;
        }
        let literal = !matches!(
            b,
            b' ' | b'\t' | b'\n' | b'\r' | b'[' | b']' | b'{' | b'}' | b',' | b':' | b'"'
        );
        if self.in_literal {
            if literal {
                return;
            }
            self.in_literal = false;
            self.scalar();
            if self.completed {
                return;
            }
        }
        match b {
            b'"' => {
                self.in_string = true;
                self.string_is_key = self.expect_key;
            }
            b'[' | b'{' => {
                self.open.push(b == b'{');
                self.expect_key = b == b'{';
            }
            b']' | b'}' => {
                self.flush();
                if self.completed {
                    return;
                }
                self.open.pop();
                self.expect_key = false;
                // A close is a `--stream` event, and completes a value at the top level.
                if self.items == Items::Events || self.open.is_empty() {
                    self.complete();
                }
            }
            b',' => {
                self.flush();
                self.expect_key = self.open.last() == Some(&true);
            }
            b':' => self.expect_key = false,
            _ if literal => self.in_literal = true,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Items, Scan};

    /// Where the scan stops over `text` fed in blocks of `block` bytes, for `taken` items.
    fn end(items: Items, text: &[u8], taken: u64, block: usize) -> Option<u64> {
        let mut scan = Scan::new(items, taken);
        for piece in text.chunks(block) {
            scan.feed(piece);
        }
        scan.found()
    }

    fn ends(items: Items, text: &[u8], taken: u64) -> Option<u64> {
        let whole = end(items, text, taken, text.len().max(1));
        for block in [1, 2, 7, 4096] {
            assert_eq!(end(items, text, taken, block), whole, "block {block}");
        }
        whole
    }

    #[test]
    fn a_value_leaves_the_reader_at_the_end_of_its_line_7455() {
        let text = b"{\"id\":0}\n{\"id\":1}\n{\"id\":2}\n";
        assert_eq!(ends(Items::Values, text, 0), Some(0));
        assert_eq!(ends(Items::Values, text, 1), Some(9));
        assert_eq!(ends(Items::Values, text, 2), Some(18));
        // The last value's line ends the text, so the read stood at its end.
        assert_eq!(ends(Items::Values, text, 3), Some(27));
        assert_eq!(ends(Items::Values, text, 4), None);
        // A line holding several values is read whole, and so are the blank lines after a
        // value's line only once the next value is asked for.
        assert_eq!(ends(Items::Values, b"1 2 3\n4 5\n", 2), Some(6));
        assert_eq!(ends(Items::Values, b"1\n\n\n2\n", 1), Some(2));
        assert_eq!(ends(Items::Values, b"\"a\"\n\"b\"\n", 1), Some(4));
        assert_eq!(ends(Items::Values, b"[1,[2]]\n{\"a\":{}}\n", 1), Some(8));
        // A value with no line break after it stands at the end.
        assert_eq!(ends(Items::Values, b"1 2", 2), None);
        assert_eq!(ends(Items::Values, b"{}{}", 1), None);
        // A BOM is skipped, but its bytes are in the first chunk.
        assert_eq!(ends(Items::Values, b"\xef\xbb\xbf1\n2\n", 1), Some(5));
    }

    #[test]
    fn a_long_line_is_read_in_chunks_of_4095_bytes_7455() {
        let mut text = b"[1] ".repeat(3000);
        text.push(b'\n');
        assert_eq!(ends(Items::Values, &text, 1), Some(4095));
        assert_eq!(ends(Items::Values, &text, 1024), Some(4095));
        assert_eq!(ends(Items::Values, &text, 1025), Some(8190));
        let long = format!("{{\"a\":\"{}\"}}\n1\n", "x".repeat(10_000));
        assert_eq!(ends(Items::Values, long.as_bytes(), 1), Some(10_009));
    }

    #[test]
    fn an_event_is_a_scalar_an_empty_container_or_a_close_7455() {
        let text = b"{\"a\":[1,{\"b\":\"x\"}],\"c\":[]}\n2\n";
        // [["a",0],1] at the `,` after 1, [["a",1,"b"],"x"] at its quote, then the closes of the
        // inner object and the array, [["c"],[]], and the close of the outer object.
        for taken in 1..=6 {
            assert_eq!(ends(Items::Events, text, taken), Some(27), "{taken}");
        }
        assert_eq!(ends(Items::Events, text, 7), Some(29));
        assert_eq!(ends(Items::Events, b"{\"id\":0}\n{\"id\":1}\n", 2), Some(9));
        assert_eq!(
            ends(Items::Events, b"{\"id\":0}\n{\"id\":1}\n", 3),
            Some(18)
        );
        let mut wide = format!("{{\"id\":0,\"p\":\"{}\"}}", "x".repeat(5000)).into_bytes();
        wide.push(b'\n');
        assert_eq!(ends(Items::Events, &wide, 1), Some(4095));
        assert_eq!(ends(Items::Events, &wide, 3), Some(wide.len() as u64));
    }

    #[test]
    fn a_scalar_event_in_a_container_waits_for_the_next_comma_or_closer_7455() {
        // Captured from jq 1.7.1: `limit(1; inputs)` under `--stream -n` leaves these offsets.
        assert_eq!(ends(Items::Events, b"[1\n,2]\n[9]\n", 1), Some(7));
        assert_eq!(ends(Items::Events, b"[\"s\"\n,2]\n[9]\n", 1), Some(9));
        assert_eq!(ends(Items::Events, b"{\"a\":\"x\"\n}\n[9]\n", 1), Some(11));
        assert_eq!(ends(Items::Events, b"{\n  \"id\": 0\n}\n", 1), Some(14));
        // An empty container's event is its own closer.
        assert_eq!(ends(Items::Events, b"[[]\n,2]\n[9]\n", 1), Some(4));
        assert_eq!(ends(Items::Events, b"\"top\"\n[9]\n", 1), Some(6));
    }

    #[test]
    fn a_second_event_completed_by_a_chunks_last_byte_stands_after_the_next_chunk_7570() {
        // Captured from jq 1.7.1 and 1.8.2: `[2]` ends at byte 4094, the last of the first chunk.
        // Its scalar event is handed out at the `]`; its close is handed out once the next chunk
        // (` 3\n`) has been read.
        let f = format!("{}[2] 3\n4\n", "1 ".repeat(2046)).into_bytes();
        assert_eq!(ends(Items::Events, &f, 2047), Some(4095));
        assert_eq!(ends(Items::Events, &f, 2048), Some(4098));
        assert_eq!(ends(Items::Events, &f, 2049), Some(4098));
        // `2]` ends at byte 4094: the scalar and the innermost close, then the next two closes.
        let g = format!("{}[[[2]]] 3\n4\n", "1 ".repeat(2045)).into_bytes();
        assert_eq!(ends(Items::Events, &g, 2046), Some(4095));
        for taken in 2047..=2050 {
            assert_eq!(ends(Items::Events, &g, taken), Some(4100), "{taken}");
        }
        // The next chunk is itself a full one when the line runs on.
        let long = format!("{}[2]{}\n", "1 ".repeat(2046), " 3".repeat(3000)).into_bytes();
        assert_eq!(ends(Items::Events, &long, 2048), Some(8190));
        // A close completed with its scalar anywhere else in a chunk ends with that chunk.
        let early = format!("[2]{}\n", " 3".repeat(3000)).into_bytes();
        assert_eq!(ends(Items::Events, &early, 2), Some(4095));
        // One item per byte: an empty array's event at the chunk's last byte, and values.
        let empty = format!("{}[[]] 3\n4\n", "1 ".repeat(2046)).into_bytes();
        assert_eq!(ends(Items::Events, &empty, 2047), Some(4095));
        assert_eq!(ends(Items::Values, &f, 2047), Some(4095));
    }

    /// Where the scan stops over `files` read one after the other, for `taken` items.
    fn ends_over(items: Items, files: &[&[u8]], taken: u64) -> Option<u64> {
        let mut scan = Scan::new(items, taken);
        for file in files {
            for piece in file.chunks(3) {
                scan.feed(piece);
            }
            scan.end_of_file();
        }
        scan.found()
    }

    #[test]
    fn the_files_are_read_as_one_text_with_a_chunk_ending_at_each_files_end_7568() {
        let stdin = b"{\"id\":0}\n{\"id\":1}\n";
        // A value that ends its file with no line break stands at the file's end.
        assert_eq!(ends_over(Items::Values, &[b"{\"a\":1}", stdin], 1), Some(7));
        assert_eq!(
            ends_over(Items::Values, &[b"{\"a\":1}", stdin], 2),
            Some(16)
        );
        // A literal a file leaves unfinished is completed by the next one.
        assert_eq!(ends_over(Items::Values, &[b"1", b"2\n3\n"], 1), Some(3));
        // A close completed by a file's last byte right after a scalar is handed out once the next
        // file's first chunk is read, an empty file's included; with a line break after it, the
        // chunk ends at that.
        assert_eq!(ends_over(Items::Events, &[b"[2]", stdin], 1), Some(3));
        assert_eq!(ends_over(Items::Events, &[b"[2]", stdin], 2), Some(12));
        assert_eq!(ends_over(Items::Events, &[b"[2]", b"", stdin], 2), Some(3));
        assert_eq!(ends_over(Items::Events, &[b"[2]\n", stdin], 2), Some(4));
        // A file whose last chunk is a full one is read once more, to its end, first.
        let full = format!("{}[2]", " ".repeat(4092)).into_bytes();
        assert_eq!(ends_over(Items::Events, &[&full, stdin], 2), Some(4095));
        // A raw line runs on into the next file.
        assert_eq!(
            ends_over(Items::Lines, &[b"abc", b"def\nghi\n"], 1),
            Some(7)
        );
        assert_eq!(ends_over(Items::Lines, &[b"abc\n", b"def\n"], 1), Some(4));
    }

    #[test]
    fn a_raw_line_runs_to_its_newline_7455() {
        let mut text = "y".repeat(9000).into_bytes();
        text.extend_from_slice(b"\nz\nlast");
        assert_eq!(ends(Items::Lines, &text, 0), Some(0));
        assert_eq!(ends(Items::Lines, &text, 1), Some(9001));
        assert_eq!(ends(Items::Lines, &text, 2), Some(9003));
        assert_eq!(ends(Items::Lines, &text, 3), None);
    }
}
