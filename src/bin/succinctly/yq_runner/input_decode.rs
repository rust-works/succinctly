//! yq's `-p base64` and `-p uri` input formats (#6925).
//!
//! Each reads a whole input as one string, which yq makes a plain `!!str` node
//! (`createStringScalarNode`). The run reads that string as the JSON string holding its bytes,
//! that is, as `-p json`: a JSON string and yq's decoded node evaluate and print alike (tag, style,
//! the quoting the YAML encoder picks), and every route after the read (streaming, the DOM,
//! `--eval-all`, `-i`) is the JSON route's. Decoded bytes that are not UTF-8 stay as they are in the
//! JSON text, so they meet the `-p json` handling of #1616.

use crate::InputFormat;

/// An input format yq decodes to one string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StringInput {
    /// `decoder_base64.go`: Go's streaming base64 decoder over the trimmed, padded input.
    Base64,
    /// `decoder_uri.go`: `url.QueryUnescape` of the whole input.
    Uri,
}

impl StringInput {
    /// The run's string format: an explicit `-p base64` / `-p uri`, or under `-p auto` the first
    /// file's extension, which yq lowercases and matches against the format names (`a.base64`,
    /// `a.URI`). Every file of a run is read with that one format (#4394).
    pub(super) fn for_run(format: InputFormat, files: &[String]) -> Option<Self> {
        match format {
            InputFormat::Base64 => Some(Self::Base64),
            InputFormat::Uri => Some(Self::Uri),
            InputFormat::Auto => match super::lowercase_extension(files.first()?).as_deref() {
                Some("base64") => Some(Self::Base64),
                Some("uri") => Some(Self::Uri),
                _ => None,
            },
            InputFormat::Yaml | InputFormat::Json => None,
        }
    }

    /// The code [`Self::from_code`] reads back, for the run's static.
    pub(super) fn code(format: Option<Self>) -> u8 {
        match format {
            None => 0,
            Some(Self::Base64) => 1,
            Some(Self::Uri) => 2,
        }
    }

    pub(super) fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Base64),
            2 => Some(Self::Uri),
            _ => None,
        }
    }

    /// `raw` decoded, as the text of the JSON string holding it; `Err` is yq's message for an
    /// input that does not decode, which it prints after `bad file '<name>': `.
    pub(super) fn transcode(self, raw: &[u8]) -> Result<Vec<u8>, String> {
        let decoded = match self {
            Self::Base64 => decode_base64(raw)?,
            Self::Uri => decode_uri(raw)?,
        };
        Ok(json_string(&decoded))
    }
}

/// `bytes` as a JSON string: `"` and `\` escaped, a control character below U+0020 written as an
/// escape, every other byte as it is (a byte that is not UTF-8 included).
fn json_string(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 2);
    out.push(b'"');
    for &byte in bytes {
        match byte {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0x00..=0x1F => out.extend_from_slice(format!("\\u{byte:04x}").as_bytes()),
            _ => out.push(byte),
        }
    }
    out.push(b'"');
    out
}

/// `url.QueryUnescape` of the whole input, the definition yq-mode `@urid` uses too (`+` is a
/// space, `invalid URL escape "%zz"`).
fn decode_uri(raw: &[u8]) -> Result<Vec<u8>, String> {
    succinctly::jq::go_query_unescape(raw).map_err(|e| e.to_string())
}

/// Why Go's base64 stream stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StreamEnd {
    /// `io.EOF`: the input ended on a whole quantum.
    Eof,
    /// `io.ErrUnexpectedEOF`: one to three characters were left over.
    UnexpectedEof,
    /// `base64.CorruptInputError`: the offset is into the chunk the decoder was handed.
    Corrupt(usize),
}

impl StreamEnd {
    fn message(self) -> String {
        match self {
            Self::Eof => String::new(),
            Self::UnexpectedEof => "unexpected EOF".to_owned(),
            Self::Corrupt(at) => format!("illegal base64 data at input byte {at}"),
        }
    }
}

/// yq's `-p base64`: the input with Go's `strings.TrimSpace` applied, padded with `=` to a multiple
/// of four bytes (counting any line break inside it), read through `base64.NewDecoder` by a
/// `bytes.Buffer`. Go's stream decoder hands each chunk of the input to `Encoding.Decode` on its
/// own, so where the chunks fall decides the answer for a malformed input: an error's offset counts
/// from its chunk's start, and padding that ends one chunk is accepted with more data after it.
/// [`GoBase64Stream`] reproduces the chunking, which follows the sizes `bytes.Buffer` grows to.
fn decode_base64(raw: &[u8]) -> Result<Vec<u8>, String> {
    let src = go_trim_space(raw);
    let mut stream = GoBase64Stream::new(src, (4 - src.len() % 4) % 4);
    // `bytes.Buffer.ReadFrom`: before each read, grow so at least `MinRead` (512) bytes are free,
    // then offer the decoder the whole free capacity.
    let mut out = Vec::new();
    let mut capacity = 0usize;
    loop {
        if capacity - out.len() < 512 {
            // `growSlice`: room for 512 more, at least twice the old capacity, rounded up to the
            // allocator's size class.
            capacity = go_round_up_size((out.len() + 512).max(2 * capacity));
        }
        let free = capacity - out.len();
        match stream.read(&mut out, free) {
            Ok(()) => {}
            Err(StreamEnd::Eof) => return Ok(out),
            Err(end) => return Err(end.message()),
        }
    }
}

/// Go's `runtime.roundupsize` for an allocation with no pointers: the smallest size class that
/// holds `size`, or past the largest class, `size` rounded up to the 8 KiB page.
fn go_round_up_size(size: usize) -> usize {
    const CLASSES: [usize; 67] = [
        8, 16, 24, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224, 240, 256, 288, 320,
        352, 384, 416, 448, 480, 512, 576, 640, 704, 768, 896, 1024, 1152, 1280, 1408, 1536, 1792,
        2048, 2304, 2688, 3072, 3200, 3456, 4096, 4864, 5376, 6144, 6528, 6784, 6912, 8192, 9472,
        9728, 10240, 10880, 12288, 13568, 14336, 16384, 18432, 19072, 20480, 21760, 24576, 27264,
        28672, 32768,
    ];
    // `maxSmallSize - mallocHeaderSize`: a larger request is a large object, page-aligned.
    if size <= 32768 - 8 {
        if let Some(&class) = CLASSES.iter().find(|&&class| class >= size) {
            return class;
        }
    }
    (size + 8191) & !8191
}

/// Go's `base64.NewDecoder(base64.StdEncoding, strings.NewReader(src))`: a `newlineFilteringReader`
/// over the string, and the decoder's 1024-byte buffer of input not yet decoded.
struct GoBase64Stream<'a> {
    /// The trimmed input; the `=` padding after it is not copied.
    src: &'a [u8],
    /// The length of the padded string the reader reads.
    len: usize,
    /// The `strings.Reader`'s offset.
    pos: usize,
    /// Input read but not decoded: what is left of a chunk that was not a whole number of quanta.
    buf: [u8; 1024],
    nbuf: usize,
    /// The reader has answered `io.EOF`.
    read_eof: bool,
    /// Decoded bytes that did not fit the last read's `p`.
    pending: Vec<u8>,
    /// The error every later read answers.
    end: Option<StreamEnd>,
}

impl<'a> GoBase64Stream<'a> {
    fn new(src: &'a [u8], padding: usize) -> Self {
        Self {
            src,
            len: src.len() + padding,
            pos: 0,
            buf: [0; 1024],
            nbuf: 0,
            read_eof: false,
            pending: Vec::new(),
            end: None,
        }
    }

    /// `newlineFilteringReader.Read` into `buf[from..to]`: one `strings.Reader` read of that many
    /// bytes with every `\r` and `\n` removed, read again while a read held nothing else. Answers
    /// how many bytes it kept; a read at the end of the string is `io.EOF` with none.
    fn filtered_read(&mut self, from: usize, to: usize) -> usize {
        loop {
            if self.pos >= self.len {
                self.read_eof = true;
                return 0;
            }
            let take = (to - from).min(self.len - self.pos);
            let mut kept = from;
            for at in self.pos..self.pos + take {
                let byte = self.src.get(at).copied().unwrap_or(b'=');
                if byte != b'\r' && byte != b'\n' {
                    self.buf[kept] = byte;
                    kept += 1;
                }
            }
            self.pos += take;
            if kept > from {
                return kept - from;
            }
        }
    }

    /// `decoder.Read` with a `p` of `room` bytes, appending what it decodes to `out`.
    fn read(&mut self, out: &mut Vec<u8>, room: usize) -> Result<(), StreamEnd> {
        if !self.pending.is_empty() {
            let n = room.min(self.pending.len());
            out.extend(self.pending.drain(..n));
            return Ok(());
        }
        if let Some(end) = self.end {
            return Err(end);
        }
        // Refill to at least one quantum, offering up to `len(p) / 3 * 4` bytes (4 to 1024).
        while self.nbuf < 4 && !self.read_eof {
            let want = (room / 3 * 4).clamp(4, self.buf.len());
            self.nbuf += self.filtered_read(self.nbuf, want);
        }
        if self.nbuf < 4 {
            let end = if self.nbuf > 0 {
                StreamEnd::UnexpectedEof
            } else {
                StreamEnd::Eof
            };
            self.end = Some(end);
            return Err(end);
        }
        let chunk = self.nbuf / 4 * 4;
        let corrupt = if chunk / 4 * 3 > room {
            // Decoded into the decoder's own buffer; `p` takes what fits, the rest waits.
            let corrupt = go_decode(&self.buf[..chunk], &mut self.pending);
            let n = room.min(self.pending.len());
            out.extend(self.pending.drain(..n));
            corrupt
        } else {
            go_decode(&self.buf[..chunk], out)
        };
        self.buf.copy_within(chunk..self.nbuf, 0);
        self.nbuf -= chunk;
        if let Some(at) = corrupt {
            let end = StreamEnd::Corrupt(at);
            self.end = Some(end);
            return Err(end);
        }
        Ok(())
    }
}

/// The value of a base64 digit of the standard alphabet.
fn base64_digit(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// `base64.StdEncoding.Decode` of one chunk, appending the bytes it writes to `out`; for malformed
/// input, the offset `CorruptInputError` names.
fn go_decode(src: &[u8], out: &mut Vec<u8>) -> Option<usize> {
    let mut si = 0;
    while si < src.len() {
        let (next, corrupt) = go_decode_quantum(src, si, out);
        si = next;
        if corrupt.is_some() {
            return corrupt;
        }
    }
    None
}

/// `Encoding.decodeQuantum`: up to four digits from `src[si..]`, padding included, appended to `out`
/// as up to three bytes. Go's also skips `\r` and `\n`; the filtering reader has removed them from
/// every chunk, so that is left out here.
fn go_decode_quantum(src: &[u8], mut si: usize, out: &mut Vec<u8>) -> (usize, Option<usize>) {
    let mut digits = [0u8; 4];
    let mut len = 4;
    let mut trailing = None;
    let mut j = 0;
    while j < 4 {
        if si == src.len() {
            if j == 0 {
                return (si, None);
            }
            // A partial quantum at the end: the standard encoding requires padding.
            return (si, Some(si - j));
        }
        let byte = src[si];
        si += 1;
        if let Some(digit) = base64_digit(byte) {
            digits[j] = digit;
            j += 1;
            continue;
        }
        if byte != b'=' {
            return (si, Some(si - 1));
        }
        match j {
            // Padding where no digit can end.
            0 | 1 => return (si, Some(si - 1)),
            // "==" is expected; the first "=" is consumed.
            2 => {
                if si == src.len() {
                    return (si, Some(src.len()));
                }
                if src[si] != b'=' {
                    return (si, Some(si - 1));
                }
                si += 1;
            }
            _ => {}
        }
        if si < src.len() {
            // Data after the padding.
            trailing = Some(si);
        }
        len = j;
        break;
    }
    let value = u32::from(digits[0]) << 18
        | u32::from(digits[1]) << 12
        | u32::from(digits[2]) << 6
        | u32::from(digits[3]);
    let bytes = [(value >> 16) as u8, (value >> 8) as u8, value as u8];
    out.extend_from_slice(&bytes[..len - 1]);
    (si, trailing)
}

/// Go's `strings.TrimSpace`: leading and trailing Unicode white space removed (Rust's
/// `char::is_whitespace` is the same `White_Space` property). A byte that is not UTF-8 is not
/// white space and stops the trim.
fn go_trim_space(bytes: &[u8]) -> &[u8] {
    let mut start = 0;
    while let Some(c) = first_char(&bytes[start..]) {
        if !c.is_whitespace() {
            break;
        }
        start += c.len_utf8();
    }
    let mut end = bytes.len();
    while end > start {
        match last_char(&bytes[start..end]) {
            Some(c) if c.is_whitespace() => end -= c.len_utf8(),
            _ => break,
        }
    }
    &bytes[start..end]
}

/// The character `bytes` starts with, if they start with a valid UTF-8 sequence.
fn first_char(bytes: &[u8]) -> Option<char> {
    let head = &bytes[..bytes.len().min(4)];
    let valid = match core::str::from_utf8(head) {
        Ok(text) => text,
        Err(e) => core::str::from_utf8(&head[..e.valid_up_to()]).ok()?,
    };
    valid.chars().next()
}

/// The character `bytes` end with, if they end with a valid UTF-8 sequence.
fn last_char(bytes: &[u8]) -> Option<char> {
    let start = (1..=bytes.len().min(4))
        .map(|k| bytes.len() - k)
        .find(|&at| bytes[at] & 0xC0 != 0x80)?;
    let text = core::str::from_utf8(&bytes[start..]).ok()?;
    let mut chars = text.chars();
    let c = chars.next()?;
    chars.next().is_none().then_some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base64(input: &str) -> Result<String, String> {
        decode_base64(input.as_bytes()).map(|bytes| String::from_utf8(bytes).unwrap())
    }

    fn uri(input: &[u8]) -> Result<Vec<u8>, String> {
        decode_uri(input)
    }

    #[test]
    fn base64_matches_yq_on_short_inputs() {
        // Each captured from yq v4.53.3 (`yq -p base64 -o json .`).
        assert_eq!(base64("aGVsbG8="), Ok("hello".into()));
        assert_eq!(base64("aGVsbG8=\n"), Ok("hello".into()));
        assert_eq!(base64("  aGk=  "), Ok("hi".into()));
        assert_eq!(base64("aGk"), Ok("hi".into()));
        assert_eq!(base64(""), Ok(String::new()));
        assert_eq!(base64("aG\nk="), Err("unexpected EOF".into()));
        assert_eq!(
            base64("aGk=\naGk="),
            Err("illegal base64 data at input byte 4".into())
        );
        assert_eq!(
            base64("aGk=aGk="),
            Err("illegal base64 data at input byte 4".into())
        );
        assert_eq!(
            base64("!!!"),
            Err("illegal base64 data at input byte 0".into())
        );
    }

    #[test]
    fn base64_trims_unicode_white_space_but_not_a_bad_byte() {
        assert_eq!(base64("\u{a0}aGk=\u{3000}"), Ok("hi".into()));
        assert_eq!(
            decode_base64(b"aGk=\xff"),
            Err("illegal base64 data at input byte 4".into())
        );
    }

    #[test]
    fn round_up_size_follows_the_size_classes() {
        assert_eq!(go_round_up_size(512), 512);
        assert_eq!(go_round_up_size(1022), 1024);
        assert_eq!(go_round_up_size(1025), 1152);
        assert_eq!(go_round_up_size(32760), 32768);
        assert_eq!(go_round_up_size(32761), 32768);
        assert_eq!(go_round_up_size(32769), 40960);
        assert_eq!(go_round_up_size(40961), 49152);
    }

    #[test]
    fn uri_unescapes_like_query_unescape() {
        assert_eq!(uri(b"a+b%41\n"), Ok(b"a bA\n".to_vec()));
        assert_eq!(uri(b"a%FFb"), Ok(b"a\xffb".to_vec()));
        assert_eq!(uri(b"%zz"), Err(r#"invalid URL escape "%zz""#.into()));
        assert_eq!(uri(b"ab%"), Err(r#"invalid URL escape "%""#.into()));
        assert_eq!(uri(b"ab%\n"), Err(r#"invalid URL escape "%\n""#.into()));
        assert_eq!(uri(b"ab%\xc3"), Err(r#"invalid URL escape "%\xc3""#.into()));
        assert_eq!(
            uri("ab%é".as_bytes()),
            Err(r#"invalid URL escape "%é""#.into())
        );
    }

    #[test]
    fn json_string_escapes_only_what_json_needs() {
        assert_eq!(
            json_string(b"a\"b\\c\n\x01\x7f\xff"),
            b"\"a\\\"b\\\\c\\n\\u0001\x7f\xff\""
        );
    }
}
