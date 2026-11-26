//! jq's input reader, in-process: two engines over the vendored C, and the jq main loops they
//! drive.
//!
//! - [`UtilReader`] calls jq's real `jq_util_input_*` (`util.c`) over named files. The
//!   4,095-byte (1.7.1) or 4,091-byte (1.8.2) `fgets` reads, 1.8.2's UTF-8 completion,
//!   `is_partial`, the file switch and `current_line` are jq's own code, so it is exact by
//!   construction. It can only start at the first byte of the first file. This is the oracle.
//! - [`PortReader`] is a Rust port of `jq_util_input_read_more` and `jq_util_input_next_input`
//!   over in-memory sources, driving the same release's `jv_parse.c` itself. It is the shape a
//!   mid-stream hand-off needs, and on jq 1.8.2 it can resume at a parser sync point
//!   ([`sync_check`]). It does not read `-R` input.
//!
//! The drivers reproduce what jq prints for three programs: `jq -c .` ([`run_main`]), a per-item
//! trace under `-n` ([`run_trace`], the text [`trace_program`] prints when jq runs it), and the
//! reader model's `inputs` program ([`run_inputs`]).
//!
//! jq's `jv` reference counts are not atomic: a reader belongs to the thread that made it, and
//! nothing here crosses threads. Every thread can run its own readers of either release.

#![allow(unsafe_code)] // FFI into the vendored jq C reader

use std::ffi::{c_char, c_int, CString};
use std::fmt;

use crate::ffi::{Api, JvParser, SjqInput, JQ_171, JQ_182};

/// A vendored jq release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Release {
    /// jq 1.7.1, the release succinctly pins (ADR-0018).
    V171,
    /// jq 1.8.2.
    V182,
}

impl Release {
    /// Both releases, the pinned one first.
    pub const ALL: [Release; 2] = [Release::V171, Release::V182];

    /// The release's version string, as `jq --version` prints it after `jq-`.
    pub fn version(self) -> &'static str {
        match self {
            Release::V171 => "1.7.1",
            Release::V182 => "1.8.2",
        }
    }

    /// The release named `1.7.1` or `1.8.2`.
    pub fn parse(s: &str) -> Option<Release> {
        Release::ALL.into_iter().find(|r| r.version() == s)
    }

    fn api(self) -> &'static Api {
        match self {
            Release::V171 => &JQ_171,
            Release::V182 => &JQ_182,
        }
    }
}

/// A failure of the oracle itself, never of the input: an allocation the shim could not make, a
/// file name holding a NUL, or a request a reader cannot serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn oom(what: &str) -> Error {
    Error(format!("the jq C reader could not allocate ({what})"))
}

/// Copy and free a buffer the shim returned.
///
/// # Safety
/// `p` is null or a buffer of at least `len` bytes that `api`'s `malloc` returned.
unsafe fn take(api: &Api, p: *mut c_char, len: usize) -> Vec<u8> {
    if p.is_null() {
        return Vec::new();
    }
    // SAFETY: the caller's contract.
    let v = unsafe { std::slice::from_raw_parts(p as *const u8, len) }.to_vec();
    // SAFETY: `p` came from the shim's malloc and is freed once.
    unsafe { (api.free_buf)(p) };
    v
}

/// The bytes of one shim `*_next` answer: 1 value, 2 error, 0 end of input, -1 out of memory.
fn item_of(api: &Api, kind: c_int, p: *mut c_char, n: usize, what: &str) -> Result<Option<Item>> {
    // SAFETY: `p`/`n` are what the shim's `take` wrote for this answer.
    let text = unsafe { take(api, p, n) };
    match kind {
        1 => Ok(Some(Item::Value(text))),
        2 => Ok(Some(Item::Error(text))),
        0 => Ok(None),
        _ => Err(oom(what)),
    }
}

/// `jv_dump_string(jv_string_sized(s), 0)`: jq's own JSON encoding of `s` as a string. The two
/// releases encode alike; jq 1.7.1's is used.
pub fn json_string(s: &[u8]) -> Result<Vec<u8>> {
    let api = Release::V171.api();
    let (mut p, mut n) = (std::ptr::null_mut(), 0);
    // SAFETY: `s` is valid for `s.len()` bytes; the out-pointers are valid.
    let k = unsafe { (api.json_string)(s.as_ptr() as *const c_char, s.len(), &mut p, &mut n) };
    // SAFETY: `p`/`n` are what the shim wrote.
    let text = unsafe { take(api, p, n) };
    if k == 1 {
        Ok(text)
    } else {
        Err(oom("a JSON string"))
    }
}

/// `JV_PARSE_SEQ`.
pub const PARSE_SEQ: i32 = 1;
/// `JV_PARSE_STREAMING`.
pub const PARSE_STREAMING: i32 = 2;
/// `JV_PARSE_STREAM_ERRORS`.
pub const PARSE_STREAM_ERRORS: i32 = 4;

/// One answer of `jq_util_input_next_input`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// The value, as `jq -c` prints it.
    Value(Vec<u8>),
    /// A parse error's message.
    Error(Vec<u8>),
}

/// The reader options of jq's command line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Opts {
    /// `JV_PARSE_*` bits: `--seq`, `--stream`, `--stream-errors`.
    pub flags: i32,
    /// `-R`.
    pub raw: bool,
    /// `-s`.
    pub slurp: bool,
}

/// What a reader engine offers the drivers.
pub trait Reader {
    /// The next item, `None` at end of input.
    fn next(&mut self) -> Result<Option<Item>>;
    /// `jq_util_input_errors`: files that could not be opened so far.
    fn errors(&self) -> i32;
    /// The `jq: error: Could not open file ...` lines printed since the last call.
    fn take_errs(&mut self) -> Vec<u8>;
    /// `input_filename`: `None` before the first file and for standard input.
    fn filename(&self) -> Result<Option<Vec<u8>>>;
    /// `input_line_number`.
    fn line(&self) -> u64;
}

/// jq's real `jq_util_input_*` over named files.
pub struct UtilReader {
    api: &'static Api,
    st: *mut SjqInput,
}

impl UtilReader {
    /// A reader of `files` in order; none means standard input, as in jq.
    pub fn new(release: Release, opts: Opts, files: &[String]) -> Result<Self> {
        let api = release.api();
        let names = files
            .iter()
            .map(|f| {
                CString::new(f.as_bytes())
                    .map_err(|_| Error(format!("a NUL in the file name {f:?}")))
            })
            .collect::<Result<Vec<_>>>()?;
        // SAFETY: plain values in; the result is null or an owned state.
        let st =
            unsafe { (api.input_new)(opts.flags, c_int::from(opts.raw), c_int::from(opts.slurp)) };
        if st.is_null() {
            return Err(oom("an input state"));
        }
        let r = UtilReader { api, st };
        if names.is_empty() {
            // SAFETY: `st` is live; the name is a NUL-terminated literal jq copies.
            unsafe { (api.input_add)(st, c"-".as_ptr()) };
        }
        for c in &names {
            // SAFETY: as above; jq copies the name.
            unsafe { (api.input_add)(st, c.as_ptr()) };
        }
        Ok(r)
    }
}

impl Drop for UtilReader {
    fn drop(&mut self) {
        // SAFETY: `st` is live and freed once.
        unsafe { (self.api.input_free)(self.st) }
    }
}

impl Reader for UtilReader {
    fn next(&mut self) -> Result<Option<Item>> {
        let (mut p, mut n) = (std::ptr::null_mut(), 0);
        // SAFETY: `st` is live; the out-pointers are valid.
        let k = unsafe { (self.api.input_next)(self.st, &mut p, &mut n) };
        item_of(self.api, k, p, n, "an input item")
    }
    fn errors(&self) -> i32 {
        // SAFETY: `st` is live.
        unsafe { (self.api.input_errors)(self.st) }
    }
    fn take_errs(&mut self) -> Vec<u8> {
        let mut n = 0;
        // SAFETY: `st` is live; the buffer it returns is ours.
        unsafe {
            let p = (self.api.input_take_errs)(self.st, &mut n);
            take(self.api, p, n)
        }
    }
    fn filename(&self) -> Result<Option<Vec<u8>>> {
        let (mut p, mut n) = (std::ptr::null_mut(), 0);
        // SAFETY: `st` is live; the out-pointers are valid.
        let k = unsafe { (self.api.input_filename)(self.st, &mut p, &mut n) };
        // SAFETY: `p`/`n` are what the shim wrote.
        let name = unsafe { take(self.api, p, n) };
        match k {
            1 => Ok(Some(name)),
            0 => Ok(None),
            _ => Err(oom("input_filename")),
        }
    }
    fn line(&self) -> u64 {
        // SAFETY: `st` is live.
        unsafe { (self.api.input_line)(self.st) as u64 }
    }
}

/// An in-memory source for [`PortReader`]: `None` is a file that cannot be opened.
#[derive(Debug, Clone)]
pub struct Source {
    pub name: String,
    pub data: Option<Vec<u8>>,
}

struct Open {
    data: Vec<u8>,
    pos: usize,
    eof: bool,
}

impl Open {
    /// `fgets(buf, n, f)` over a regular file: up to `n - 1` bytes, ending after a `\n`. Sets
    /// `eof` exactly when stdio would: a read was attempted at the end of the data.
    fn fgets(&mut self, n: usize, out: &mut Vec<u8>) -> bool {
        out.clear();
        while out.len() < n - 1 {
            match self.data.get(self.pos) {
                None => {
                    self.eof = true;
                    break;
                }
                Some(&b) => {
                    self.pos += 1;
                    out.push(b);
                    if b == b'\n' {
                        break;
                    }
                }
            }
        }
        !out.is_empty()
    }

    /// `fread(buf, 1, n, f)`.
    fn fread(&mut self, n: usize, out: &mut Vec<u8>) {
        let k = n.min(self.data.len() - self.pos);
        out.extend_from_slice(&self.data[self.pos..self.pos + k]);
        self.pos += k;
        if k < n {
            self.eof = true;
        }
    }
}

/// Everything a [`PortReader`] needs to resume at a sync point: the reader's own state (which
/// file, where in it, `current_line`, failures) and the parser's (line, column, BOM matcher, RS
/// wait, whitespace flag), plus how many items came before it.
#[derive(Debug, Clone)]
pub struct Seed {
    curr_file: usize,
    cur: Option<(usize, bool)>,
    filename: Option<Vec<u8>>,
    line: u64,
    failures: i32,
    parser: [c_int; 5],
    /// The items the run gave before this point.
    pub items: usize,
}

/// A Rust port of `jq_util_input_read_more` and the parser half of `jq_util_input_next_input`,
/// over in-memory sources, driving one release's parser directly with that release's read rules.
pub struct PortReader {
    release: Release,
    api: &'static Api,
    files: Vec<Source>,
    curr_file: usize,
    cur: Option<Open>,
    buf: Vec<u8>,
    chunk: Vec<u8>,
    filename: Option<Vec<u8>>,
    line: u64,
    failures: i32,
    errs: Vec<u8>,
    parser: *mut JvParser,
    slurped: Option<Vec<Vec<u8>>>,
    /// Sync points seen so far, when recording.
    pub sync_points: Option<Vec<Seed>>,
    emitted: usize,
}

impl PortReader {
    /// A reader of `files`. `-R` is [`UtilReader`]'s alone.
    pub fn new(release: Release, opts: Opts, files: Vec<Source>) -> Result<Self> {
        if opts.raw {
            return Err(Error(
                "the port engine drives the parser only; -R needs the util engine".into(),
            ));
        }
        let api = release.api();
        // SAFETY: a plain value in; the result is null or an owned parser.
        let parser = unsafe { (api.parser_new)(opts.flags) };
        if parser.is_null() {
            return Err(oom("a parser"));
        }
        Ok(PortReader {
            release,
            api,
            files,
            curr_file: 0,
            cur: None,
            buf: vec![0xff; 4096],
            chunk: Vec::with_capacity(4096),
            filename: None,
            line: 0,
            failures: 0,
            errs: Vec::new(),
            parser,
            slurped: opts.slurp.then(Vec::new),
            sync_points: None,
            emitted: 0,
        })
    }

    /// A reader resumed at `seed`, with a fresh parser seeded through the vendored patch.
    pub fn seeded(release: Release, opts: Opts, files: Vec<Source>, seed: &Seed) -> Result<Self> {
        let mut r = Self::new(release, opts, files)?;
        r.curr_file = seed.curr_file;
        r.cur = match seed.cur {
            None => None,
            Some((pos, eof)) => {
                let data = seed
                    .curr_file
                    .checked_sub(1)
                    .and_then(|i| r.files.get(i))
                    .and_then(|f| f.data.clone())
                    .ok_or_else(|| Error("a seed names a source that is not open".into()))?;
                Some(Open { data, pos, eof })
            }
        };
        r.filename = seed.filename.clone();
        r.line = seed.line;
        r.failures = seed.failures;
        r.emitted = seed.items;
        // SAFETY: the parser is live; `seed.parser` holds five ints.
        unsafe { (r.api.parser_seed)(r.parser, seed.parser.as_ptr()) };
        Ok(r)
    }

    fn note_sync_point(&mut self) -> Result<()> {
        if self.sync_points.is_none() || self.slurped.is_some() {
            return Ok(());
        }
        let mut st = [0 as c_int; 5];
        // SAFETY: the parser is live; `st` holds five ints.
        match unsafe { (self.api.parser_sync_point)(self.parser, st.as_mut_ptr()) } {
            1 => {}
            0 => return Ok(()),
            _ => {
                return Err(Error(format!(
                    "jq {}'s vendored parser has no sync points",
                    self.release.version()
                )))
            }
        }
        let seed = Seed {
            curr_file: self.curr_file,
            cur: self.cur.as_ref().map(|c| (c.pos, c.eof)),
            filename: self.filename.clone(),
            line: self.line,
            failures: self.failures,
            parser: st,
            items: self.emitted,
        };
        if let Some(points) = self.sync_points.as_mut() {
            points.push(seed);
        }
        Ok(())
    }

    /// `jq_util_input_read_more`: fills `buf[..len]`, returns (`len`, `is_last`).
    fn read_more(&mut self) -> (usize, bool) {
        let v171 = self.release == Release::V171;
        if self.cur.as_ref().is_none_or(|c| c.eof) {
            if self.cur.take().is_some() && v171 {
                self.filename = None;
                self.line = 0;
            }
            if self.curr_file < self.files.len() {
                let f = &self.files[self.curr_file];
                self.curr_file += 1;
                self.filename = Some(f.name.as_bytes().to_vec());
                self.line = 0;
                match &f.data {
                    Some(d) => {
                        self.cur = Some(Open {
                            data: d.clone(),
                            pos: 0,
                            eof: false,
                        })
                    }
                    None => {
                        self.errs.extend_from_slice(
                            format!(
                                "jq: error: Could not open file {}: No such file or directory\n",
                                f.name
                            )
                            .as_bytes(),
                        );
                        self.failures += 1;
                    }
                }
            }
        }
        let mut len = 0;
        if let Some(c) = self.cur.as_mut() {
            // 1.7.1: fgets(buf, 4096); 1.8.2: fgets(buf, 4096 - 4).
            let gets = if v171 { 4096 } else { 4092 };
            if c.fgets(gets, &mut self.chunk) {
                let nl = self.chunk.last() == Some(&b'\n');
                if nl {
                    self.line += 1;
                    len = self.chunk.len();
                } else if let (true, Some(nul)) = (v171, self.chunk.iter().position(|&b| b == 0)) {
                    // 1.7.1's `strlen(buf)` when no newline and a parser is attached.
                    len = nul;
                } else {
                    // At EOF the backwards scan for the terminating NUL finds the data length.
                    len = self.chunk.len();
                    if !c.eof && !v171 {
                        // SAFETY: `len >= 1` (fgets returned bytes); both pointers are in `chunk`.
                        let missing = unsafe {
                            (self.api.utf8_missing)(
                                self.chunk.as_ptr().add(len - 1) as *const c_char,
                                self.chunk.as_ptr() as *const c_char,
                            )
                        };
                        if missing > 0 {
                            c.fread(missing as usize, &mut self.chunk);
                            len = self.chunk.len();
                        }
                    }
                }
                self.buf.clear();
                self.buf.extend_from_slice(&self.chunk);
            }
        }
        let is_last = self.curr_file == self.files.len()
            && match &self.cur {
                None => true,
                Some(c) => v171 && c.eof,
            };
        (len, is_last)
    }
}

impl Drop for PortReader {
    fn drop(&mut self) {
        // SAFETY: the parser is live and freed once.
        unsafe { (self.api.parser_free)(self.parser) }
    }
}

impl Reader for PortReader {
    fn next(&mut self) -> Result<Option<Item>> {
        let v171 = self.release == Release::V171;
        // Locals of one `jq_util_input_next_input` call, as in C: `is_last` changes only when a
        // read happens.
        let mut is_last = false;
        let mut has_more = false;
        loop {
            // SAFETY: the parser is live.
            if unsafe { (self.api.parser_remaining)(self.parser) } == 0 {
                self.note_sync_point()?;
                let (len, last) = self.read_more();
                is_last = last;
                let len = c_int::try_from(len).map_err(|_| Error("a read over 2 GiB".into()))?;
                // SAFETY: `buf` holds `len` bytes and outlives their use: the parser reads it
                // before the next `read_more` refills it.
                unsafe {
                    (self.api.parser_set_buf)(
                        self.parser,
                        self.buf.as_ptr() as *const c_char,
                        len,
                        c_int::from(!is_last),
                    )
                };
            }
            let (mut p, mut n) = (std::ptr::null_mut(), 0);
            // SAFETY: the parser is live; the out-pointers are valid.
            let k = unsafe { (self.api.parser_next)(self.parser, &mut p, &mut n) };
            let item = item_of(self.api, k, p, n, "a parsed value")?;
            if let Some(s) = self.slurped.as_mut() {
                if v171 {
                    // SAFETY: the parser is live.
                    has_more = unsafe { (self.api.parser_remaining)(self.parser) } != 0;
                }
                match item {
                    Some(Item::Value(v)) => s.push(v),
                    Some(e @ Item::Error(_)) => return Ok(Some(e)),
                    None => {}
                }
            } else if let Some(item) = item {
                self.emitted += 1;
                return Ok(Some(item));
            }
            if is_last && !has_more {
                break;
            }
        }
        if let Some(s) = self.slurped.take() {
            // `jv_dump_string` of the array: `[` + the values' own dumps + `]`.
            let mut out = b"[".to_vec();
            for (i, v) in s.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                out.extend_from_slice(v);
            }
            out.push(b']');
            return Ok(Some(Item::Value(out)));
        }
        Ok(None)
    }
    fn errors(&self) -> i32 {
        self.failures
    }
    fn take_errs(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.errs)
    }
    fn filename(&self) -> Result<Option<Vec<u8>>> {
        Ok(self.filename.clone())
    }
    fn line(&self) -> u64 {
        self.line
    }
}

/// What a jq run printed and its exit status.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Transcript {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub status: i32,
}

/// jq's `main` loop for the program `.` with `-c`, with or without `--seq` (which also writes an
/// RS before every value).
pub fn run_main(r: &mut dyn Reader, seq: bool) -> Result<Transcript> {
    let mut t = Transcript::default();
    while r.errors() == 0 {
        let it = r.next()?;
        t.stderr.extend_from_slice(&r.take_errs());
        match it {
            None => break,
            Some(Item::Value(v)) => {
                if seq {
                    t.stdout.push(0x1e);
                }
                t.stdout.extend_from_slice(&v);
                t.stdout.push(b'\n');
            }
            Some(Item::Error(m)) => {
                if !seq {
                    // jq's JQ_ERROR_UNKNOWN: the main loop stops at the first fault.
                    t.status = 5;
                    t.stderr.extend_from_slice(b"jq: parse error: ");
                    t.stderr.extend_from_slice(&m);
                    t.stderr.push(b'\n');
                    break;
                }
                t.stderr.extend_from_slice(b"jq: ignoring parse error: ");
                t.stderr.extend_from_slice(&m);
                t.stderr.push(b'\n');
            }
        }
    }
    t.stderr.extend_from_slice(&r.take_errs());
    if r.errors() != 0 {
        t.status = 2;
    }
    Ok(t)
}

/// The jq program whose `jq -n -c` output [`run_trace`] reproduces: one line per reader item, with
/// the position `input_filename` / `input_line_number` report right after it, for at most `lim`
/// items (`Malformed BOM` consumes nothing, so `input` repeats it forever). At end of input
/// `f_input` raises the error `"break"` (not "No more inputs") in both 1.7.1 and 1.8.2. `repeat`
/// is a tail call, so it runs in constant stack.
pub fn trace_program(lim: u64) -> String {
    format!(
        r#"label $d | limit({lim}; repeat(try ["V", input] catch ["E", .])) | if . == ["E","break"] then break $d else . + [input_filename, input_line_number] end"#
    )
}

/// One line of [`trace_program`]'s output.
fn trace_line(item: &Item, filename: Option<&[u8]>, line: u64, out: &mut Vec<u8>) -> Result<()> {
    match item {
        Item::Value(v) => {
            out.extend_from_slice(b"[\"V\",");
            out.extend_from_slice(v);
        }
        Item::Error(m) => {
            out.extend_from_slice(b"[\"E\",");
            out.extend_from_slice(&json_string(m)?);
        }
    }
    out.push(b',');
    match filename {
        Some(f) => out.extend_from_slice(&json_string(f)?),
        None => out.extend_from_slice(b"null"),
    }
    out.extend_from_slice(format!(",{line}]\n").as_bytes());
    Ok(())
}

/// [`trace_program`]`(lim)`'s output under `jq -n -c`: every item until end of input. Under
/// `--seq` jq writes an RS before every output value, the trace's arrays included. The exit status
/// is 2 once a file failed to open, else 0.
pub fn run_trace(r: &mut dyn Reader, lim: u64, seq: bool) -> Result<Transcript> {
    let mut t = Transcript::default();
    for _ in 0..lim {
        let Some(item) = r.next()? else { break };
        t.stderr.extend_from_slice(&r.take_errs());
        if seq {
            t.stdout.push(0x1e);
        }
        trace_line(&item, r.filename()?.as_deref(), r.line(), &mut t.stdout)?;
    }
    t.stderr.extend_from_slice(&r.take_errs());
    t.status = if r.errors() != 0 { 2 } else { 0 };
    Ok(t)
}

/// The program of the reader model's captured `inputs_*`/`slurp_*` expectations, which
/// [`run_inputs`] reproduces under `jq -n -c`.
pub fn inputs_program(limit: usize) -> String {
    format!(
        "[limit({limit}; repeat(try [input, input_line_number, input_filename] catch [., input_line_number, input_filename]))]"
    )
}

/// [`inputs_program`]`(limit)`'s output under `jq -n -c` (without the trailing newline's RS).
pub fn run_inputs(r: &mut dyn Reader, limit: usize) -> Result<Vec<u8>> {
    let mut out = b"[".to_vec();
    for i in 0..limit {
        if i > 0 {
            out.push(b',');
        }
        out.push(b'[');
        match r.next()? {
            Some(Item::Value(v)) => out.extend_from_slice(&v),
            Some(Item::Error(m)) => out.extend_from_slice(&json_string(&m)?),
            None => out.extend_from_slice(b"\"break\""),
        }
        out.extend_from_slice(format!(",{},", r.line()).as_bytes());
        match r.filename()? {
            Some(f) => out.extend_from_slice(&json_string(&f)?),
            None => out.extend_from_slice(b"null"),
        }
        out.push(b']');
    }
    out.extend_from_slice(b"]\n");
    Ok(out)
}

/// One full run's items, each with its position.
/// An item with the `input_filename` and `input_line_number` right after it.
type Traced = (Item, Option<Vec<u8>>, u64);

fn traced_all(r: &mut PortReader, lim: u64) -> Result<Vec<Traced>> {
    let mut v = Vec::new();
    for _ in 0..lim {
        let Some(item) = r.next()? else { break };
        v.push((item, r.filename()?, r.line()));
    }
    Ok(v)
}

/// The outcome of [`sync_check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncCheck {
    pub items: usize,
    pub sync_points: usize,
    pub checked: usize,
    pub mismatches: usize,
}

/// Record every sync point of a full port-engine run over `sources`, resume a fresh seeded reader
/// at up to `checks` of them (always the last), and count those whose rest differs from the full
/// run's. jq 1.8.2 only: 1.7.1 is vendored without the sync-point patch.
pub fn sync_check(
    release: Release,
    opts: Opts,
    sources: &[Source],
    checks: usize,
    lim: u64,
) -> Result<SyncCheck> {
    let mut full = PortReader::new(release, opts, sources.to_vec())?;
    full.sync_points = Some(Vec::new());
    let all = traced_all(&mut full, lim)?;
    let errs = full.errors();
    let points = full.sync_points.take().unwrap_or_default();
    let step = points.len().div_ceil(checks.max(1)).max(1);
    let (mut checked, mut mismatches) = (0, 0);
    for (i, seed) in points.iter().enumerate() {
        if i % step != 0 && i + 1 != points.len() {
            continue;
        }
        checked += 1;
        let mut r = PortReader::seeded(release, opts, sources.to_vec(), seed)?;
        let rest = traced_all(&mut r, lim)?;
        if rest[..] != all[seed.items.min(all.len())..] || r.errors() != errs {
            mismatches += 1;
        }
    }
    Ok(SyncCheck {
        items: all.len(),
        sync_points: points.len(),
        checked,
        mismatches,
    })
}
