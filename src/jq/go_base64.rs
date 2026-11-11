//! A port of yq's base64 decoder (`decoder_base64.go`, v4.53.3) over Go's `encoding/base64` stream
//! decoder (#6925, #6997): yq's `@base64d` and its `-p base64` input format both read a string
//! through it.
//!
//! yq trims the text with `strings.TrimSpace`, pads it with `=` to a multiple of four bytes by its
//! *raw* length (a line break inside it counted), and reads it through `base64.NewDecoder` into a
//! `bytes.Buffer`. That decoder drops `\r` and `\n` as it reads and hands each chunk of what is left
//! to `Encoding.Decode` on its own, so where the chunks fall decides the answer for a malformed
//! input: an error's offset counts from its chunk's start, and padding that ends one chunk is
//! accepted with more data after it. The chunks follow the sizes `bytes.Buffer` grows to, which
//! follow the Go allocator's size classes.

use alloc::vec::Vec;

/// Why Go's decoder refused the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoBase64Error {
    /// `io.ErrUnexpectedEOF` (`unexpected EOF`): the text ended one to three characters into a
    /// quantum, which a line break inside it does, because the padding counted it.
    UnexpectedEof,
    /// `base64.CorruptInputError` (`illegal base64 data at input byte N`), `N` counted from the start
    /// of the chunk the decoder was reading.
    Corrupt(usize),
}

/// `text` decoded as yq decodes it, or Go's error.
pub fn go_base64_decode(text: &[u8]) -> Result<Vec<u8>, GoBase64Error> {
    decode_base64(text)
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

/// yq's `-p base64`: the input with Go's `strings.TrimSpace` applied, padded with `=` to a multiple
/// of four bytes (counting any line break inside it), read through `base64.NewDecoder` by a
/// `bytes.Buffer`. Go's stream decoder hands each chunk of the input to `Encoding.Decode` on its
/// own, so where the chunks fall decides the answer for a malformed input: an error's offset counts
/// from its chunk's start, and padding that ends one chunk is accepted with more data after it.
/// [`GoBase64Stream`] reproduces the chunking, which follows the sizes `bytes.Buffer` grows to.
fn decode_base64(raw: &[u8]) -> Result<Vec<u8>, GoBase64Error> {
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
            Err(StreamEnd::UnexpectedEof) => return Err(GoBase64Error::UnexpectedEof),
            Err(StreamEnd::Corrupt(at)) => return Err(GoBase64Error::Corrupt(at)),
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
///
/// Two parts of Go's decoder are left out because `bytes.Buffer` never reaches them: the output a
/// read decodes past `len(p)` (a refill offers at most `len(p) / 3 * 4` bytes, which decode to at
/// most `len(p)`), and a read after an error (`ReadFrom` stops at the first).
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
        // Refill to at least one quantum, offering up to `len(p) / 3 * 4` bytes (4 to 1024).
        while self.nbuf < 4 && !self.read_eof {
            let want = (room / 3 * 4).clamp(4, self.buf.len());
            self.nbuf += self.filtered_read(self.nbuf, want);
        }
        match self.nbuf {
            0 => return Err(StreamEnd::Eof),
            1..=3 => return Err(StreamEnd::UnexpectedEof),
            _ => {}
        }
        let chunk = self.nbuf / 4 * 4;
        let corrupt = go_decode(&self.buf[..chunk], out);
        self.buf.copy_within(chunk..self.nbuf, 0);
        self.nbuf -= chunk;
        corrupt.map_or(Ok(()), |at| Err(StreamEnd::Corrupt(at)))
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
/// as up to three bytes. Go's also skips `\r` and `\n`, which the filtering reader has removed from
/// every chunk, and so never meets the end of a chunk inside a quantum: a chunk is whole quanta.
fn go_decode_quantum(src: &[u8], mut si: usize, out: &mut Vec<u8>) -> (usize, Option<usize>) {
    let mut digits = [0u8; 4];
    let mut len = 4;
    let mut trailing = None;
    let mut j = 0;
    while j < 4 {
        // Go's answer for a quantum the chunk ends inside, which a whole-quanta chunk never has.
        let Some(&byte) = src.get(si) else {
            return (si, Some(si - j)); // patchcov: coverage tolerate-line reason="unreachable: a chunk is whole quanta with its line breaks removed, so it never ends inside one; kept as Go's answer (#6925)"
        };
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
                    return (si, Some(src.len())); // patchcov: coverage tolerate-line reason="unreachable: the first `=` of `==` is at most the third byte of a whole quantum, so a byte follows it; kept as Go's answer (#6925)"
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

    fn base64(input: &str) -> Result<String, GoBase64Error> {
        go_base64_decode(input.as_bytes()).map(|bytes| String::from_utf8(bytes).unwrap())
    }

    #[test]
    fn base64_matches_yq_on_short_inputs() {
        // Each captured from yq v4.53.3 (`yq -p base64 -o json .`, `@base64d`).
        assert_eq!(base64("aGVsbG8="), Ok("hello".into()));
        assert_eq!(base64("aGVsbG8=\n"), Ok("hello".into()));
        assert_eq!(base64("  aGk=  "), Ok("hi".into()));
        assert_eq!(base64("aGk"), Ok("hi".into()));
        assert_eq!(base64(""), Ok(String::new()));
        assert_eq!(base64("aG\nk="), Err(GoBase64Error::UnexpectedEof));
        assert_eq!(base64("aGVs\rbG8="), Err(GoBase64Error::UnexpectedEof));
        assert_eq!(base64("aGk=\naGk="), Err(GoBase64Error::Corrupt(4)));
        assert_eq!(base64("aGk=aGk="), Err(GoBase64Error::Corrupt(4)));
        assert_eq!(base64("!!!"), Err(GoBase64Error::Corrupt(0)));
    }

    #[test]
    fn base64_trims_unicode_white_space_but_not_a_bad_byte() {
        assert_eq!(base64("\u{a0}aGk=\u{3000}"), Ok("hi".into()));
        assert_eq!(
            go_base64_decode(b"aGk=\xff"),
            Err(GoBase64Error::Corrupt(4))
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
}
