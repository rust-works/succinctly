//! Structured edge inputs: a case is a pure function of `(generator, seed, index)`, so any case
//! can be regenerated from the three numbers a report prints.
//!
//! An `edge` case puts one *piece* (the bytes under test) where jq's reads cut: its first byte
//! lands at an offset in 4,085-4,100 or 8,180-8,200 (around the first and second 4,095- and
//! 4,091-byte `fgets` reads), at a file seam, or near the start. Before the piece comes a filler
//! that sets how the reads fall: one line of spaces, newline-ended values, one long string, `--seq`
//! records, or the inside of one long array. After it come a tail and, in a seam layout, more
//! files, an empty, missing or newline-only file among them. The flags are `-s`, `-n` (with the
//! trace program), `--seq` and their mixes; the entry is the files, standard input redirected from
//! a file, or a pipe.
//!
//! The `spike` generator is option G's own (`gen_case` in the spike's `jq_c_reader.rs`), kept so
//! the 800 cases it ran (`--seed 1`) can be replayed exactly.

/// What the piece holds: the case's family in the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Family {
    /// A NUL outside a string (E2's trigger).
    NulOutside,
    /// A NUL inside a string.
    NulInString,
    /// A BOM, whole, split or malformed.
    Bom,
    /// Invalid UTF-8, or valid multi-byte UTF-8 cut by a read.
    Utf8,
    /// A well-formed token that straddles the edge.
    Token,
    /// A syntax fault at the edge.
    Fault,
    /// Record separators (RS, 0x1e).
    Rs,
    /// No special bytes: a long line and a plain value at the edge.
    LongLine,
    /// Two to four pieces of any of the families above, back to back.
    Mix,
    /// The spike's generator: a mix of all of the above.
    Spike,
}

impl Family {
    pub const ALL: [Family; 10] = [
        Family::NulOutside,
        Family::NulInString,
        Family::Bom,
        Family::Utf8,
        Family::Token,
        Family::Fault,
        Family::Rs,
        Family::LongLine,
        Family::Mix,
        Family::Spike,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Family::NulOutside => "nul-outside-string",
            Family::NulInString => "nul-in-string",
            Family::Bom => "bom",
            Family::Utf8 => "utf8",
            Family::Token => "token-straddle",
            Family::Fault => "fault",
            Family::Rs => "rs",
            Family::LongLine => "long-line",
            Family::Mix => "mix",
            Family::Spike => "spike-mix",
        }
    }
}

/// Where the piece's first byte lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Edge {
    /// Within the first few bytes.
    Start,
    /// At an offset in 4,085-4,100.
    Read1,
    /// At an offset in 8,180-8,200.
    Read2,
    /// Straddling the seam between two files.
    Seam,
}

impl Edge {
    pub fn name(self) -> &'static str {
        match self {
            Edge::Start => "start",
            Edge::Read1 => "4085-4100",
            Edge::Read2 => "8180-8200",
            Edge::Seam => "seam",
        }
    }
}

/// How the input reaches jq.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Entry {
    /// The files, named on the command line.
    Files,
    /// The files' concatenation on standard input, redirected from a regular file.
    StdinFile,
    /// The same, through a pipe.
    StdinPipe,
}

impl Entry {
    pub fn name(self) -> &'static str {
        match self {
            Entry::Files => "files",
            Entry::StdinFile => "stdin<file",
            Entry::StdinPipe => "stdin|pipe",
        }
    }
}

/// The command-line flags; `-n` runs the trace program, the others `.`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Flags {
    pub null_input: bool,
    pub slurp: bool,
    pub seq: bool,
}

impl Flags {
    /// The mixes generated: none, `-s`, `-n`, `-n -s`, `--seq`, `--seq -s`, `--seq -n`.
    pub const ALL: [Flags; 7] = [
        Flags::new(false, false, false),
        Flags::new(false, true, false),
        Flags::new(true, false, false),
        Flags::new(true, true, false),
        Flags::new(false, false, true),
        Flags::new(false, true, true),
        Flags::new(true, false, true),
    ];

    pub const fn new(null_input: bool, slurp: bool, seq: bool) -> Flags {
        Flags {
            null_input,
            slurp,
            seq,
        }
    }

    /// The flags as jq's arguments.
    pub fn args(self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.seq {
            v.push("--seq");
        }
        if self.null_input {
            v.push("-n");
        }
        if self.slurp {
            v.push("-s");
        }
        v
    }

    pub fn name(self) -> String {
        let a = self.args();
        if a.is_empty() {
            "(none)".into()
        } else {
            a.join(" ")
        }
    }
}

/// One input file: its name and its bytes, or `None` for a file that does not exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub name: String,
    pub data: Option<Vec<u8>>,
}

/// One generated case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    pub index: u64,
    pub family: Family,
    pub edge: Edge,
    pub flags: Flags,
    pub entry: Entry,
    /// A short description of the layout and filler.
    pub shape: String,
    pub files: Vec<File>,
}

impl Case {
    /// The bytes jq sees end to end: the files that exist, in order.
    pub fn stream(&self) -> Vec<u8> {
        self.files
            .iter()
            .filter_map(|f| f.data.as_deref())
            .flatten()
            .copied()
            .collect()
    }
}

/// Which generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Generator {
    Edge,
    Spike,
}

impl Generator {
    pub fn parse(s: &str) -> Option<Generator> {
        match s {
            "edge" => Some(Generator::Edge),
            "spike" => Some(Generator::Spike),
            _ => None,
        }
    }

    pub fn case(self, seed: u64, index: u64) -> Case {
        match self {
            Generator::Edge => edge_case(seed, index),
            Generator::Spike => spike_case(seed, index),
        }
    }
}

/// splitmix64, seeded per case so a case does not depend on the ones before it.
struct Rng(u64);

impl Rng {
    fn for_case(seed: u64, index: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ index.wrapping_mul(0xd1b5_4a32_d192_ed03))
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

const NUL_OUTSIDE: &[&[u8]] = &[
    b"1\0",
    b"\0",
    b"12\0 3",
    b"[1,\0 2]",
    b"1\0x",
    b"\0\n",
    b"tru\0e",
    b"\"a\"\0",
    b"1\0\n2",
    b"\0\0",
    b"{\"a\":1\0}",
    b"-\0",
];
const NUL_IN_STRING: &[&[u8]] = &[
    b"\"a\0b\"",
    b"\"\0\"",
    b"\"ab\0",
    b"[\"\0\",1]",
    b"\"\\u0000\0\"",
];
const BOM: &[&[u8]] = &[
    b"\xef\xbb\xbf1",
    b"\xef\xbb\xbf",
    b"\xef\xbb",
    b"\xef",
    b"\xbb\xbf1",
    b"\xef1",
    b"\xef\xbb1",
    b"\xef\xbb\xbf\xef\xbb\xbf1",
    b"\xef\xbb\xbf\"a\"",
];
const UTF8: &[&[u8]] = &[
    b"\"\xff\"",
    b"\"\xc3\"",
    b"\"\xc3",
    b"\"\xe0\x80\x80\"",
    b"\"\xed\xa0\x80\"",
    b"\"\xf0\x9f\x98\x80\"",
    b"\"a\xe2\x82\"",
    b"\"\xe2\x82\xac\"",
    b"\xff",
    b"\xc3\xa9",
    b"\"\xc0\xaf\"",
    b"\"\xf4\x90\x80\x80\"",
    b"\"\x80\x80\"",
    b"\"\xe1\x80",
];
const TOKEN: &[&[u8]] = &[
    b"12345",
    b"-1.5e3",
    b"true",
    b"null",
    b"false",
    b"\"abcdef\"",
    b"\"\\u00e9\"",
    b"\"\\\\\"",
    b"[1,2]",
    b"{\"a\":1}",
    b"1.0e2",
    b"100000000000000000000",
    b"\"\\ud83d\\ude00\"",
    b"[]",
    b"{}",
];
const FAULT: &[&[u8]] = &[
    b"]",
    b"}",
    b"x",
    b"1 }",
    b"[1,",
    b"{\"a\"",
    b"\"unterminated",
    b"1.",
    b"1e",
    b"-",
    b",",
    b":",
    b"tru",
    b"nul",
    b"[1 2]",
    b"{1:2}",
    b"\"\\x\"",
    b"01",
];
const RS: &[&[u8]] = &[
    b"\x1e",
    b"\x1e1\n",
    b"1\x1e2",
    b"\x1e\x1e1",
    b"\"a\x1e\"",
    b"\x1e[1,",
    b"\x1e\n",
];
const LONG: &[&[u8]] = &[b"1", b"\"a\"", b"[0]", b"7 8"];

const TAILS: &[&[u8]] = &[b"", b"\n", b" 2\n", b"\n3\n", b"2", b" ", b"\n\"z\"\n"];
const LATER: &[&[u8]] = &[
    b"",
    b"9\n",
    b"9",
    b"8\n9\n",
    b"\xef\xbb\xbf4\n",
    b"\x1e5\n",
    b" 6",
];

fn pieces(family: Family) -> &'static [&'static [u8]] {
    match family {
        Family::NulOutside => NUL_OUTSIDE,
        Family::NulInString => NUL_IN_STRING,
        Family::Bom => BOM,
        Family::Utf8 => UTF8,
        Family::Token => TOKEN,
        Family::Fault => FAULT,
        Family::Rs => RS,
        Family::LongLine | Family::Mix | Family::Spike => LONG,
    }
}

/// `n` bytes of well-formed filler of kind `kind`, and whether it opened an array the case
/// must close after the piece.
fn filler(kind: &str, n: usize) -> (Vec<u8>, bool) {
    match kind {
        "sp" => (vec![b' '; n], false),
        "nd" => {
            let mut v = b"0\n".repeat(n / 2);
            v.resize(n, b' ');
            (v, false)
        }
        "str" if n >= 3 => {
            let mut v = vec![b'"'];
            v.resize(n - 2, b'a');
            v.extend_from_slice(b"\" ");
            (v, false)
        }
        "seq" => {
            let mut v = b"\x1e0\n".repeat(n / 3);
            v.resize(n, b' ');
            (v, false)
        }
        "arr" if n >= 2 => {
            let mut v = vec![b'['];
            while v.len() + 2 <= n {
                v.extend_from_slice(b"0,");
            }
            v.resize(n, b' ');
            (v, true)
        }
        _ => (vec![b' '; n], false),
    }
}

fn edge_case(seed: u64, index: u64) -> Case {
    let mut r = Rng::for_case(seed, index);
    let family = *r.pick(&Family::ALL[..9]);
    let flags = if family == Family::Rs && r.chance(50) {
        *r.pick(&Flags::ALL[4..])
    } else {
        *r.pick(&Flags::ALL)
    };
    let edge = *r.pick(&[
        Edge::Start,
        Edge::Read1,
        Edge::Read1,
        Edge::Read2,
        Edge::Seam,
        Edge::Seam,
    ]);
    let mut piece = if family == Family::Mix {
        let mut v = Vec::new();
        for _ in 0..2 + r.below(3) {
            let f = *r.pick(&Family::ALL[..8]);
            v.extend_from_slice(r.pick(pieces(f)));
            if r.chance(30) {
                v.push(*r.pick(b" \n"));
            }
        }
        v
    } else {
        r.pick(pieces(family)).to_vec()
    };
    if flags.seq && r.chance(50) {
        piece.insert(0, 0x1e);
    }
    let fills: &[&str] = if flags.seq {
        &["sp", "nd", "str", "seq", "seq", "arr"]
    } else {
        &["sp", "sp", "nd", "str", "arr"]
    };
    let fill = *r.pick(fills);
    let bom_first = family != Family::Bom && r.chance(8);
    let mut pre: Vec<u8> = if bom_first {
        b"\xef\xbb\xbf".to_vec()
    } else {
        Vec::new()
    };
    if flags.seq && r.chance(60) {
        pre.extend_from_slice(b"\x1e0\n");
    }

    // The piece's first byte lands at `at`, `split` bytes into it is the seam (if any).
    let at = match edge {
        Edge::Start => pre.len() + r.below(6),
        Edge::Read1 => 4085 + r.below(16),
        Edge::Read2 => 8180 + r.below(21),
        Edge::Seam => {
            if r.chance(50) {
                pre.len() + r.below(4)
            } else {
                4085 + r.below(16)
            }
        }
    };
    let (fill_bytes, opened) = filler(fill, at.saturating_sub(pre.len()));
    let mut body = pre;
    body.extend_from_slice(&fill_bytes);
    let piece_at = body.len();
    body.extend_from_slice(&piece);
    if opened {
        body.extend_from_slice(b",1]");
    }
    body.extend_from_slice(r.pick(TAILS));

    let mut files = Vec::new();
    let layout;
    if edge == Edge::Seam {
        // Cut inside the piece (or right before or after it).
        let cut = (piece_at + r.below(piece.len() + 1)).min(body.len());
        let (a, b) = body.split_at(cut);
        files.push(File {
            name: "a.json".into(),
            data: Some(a.to_vec()),
        });
        if r.chance(25) {
            let (mid, label) = *r.pick(&[
                (Some(&b""[..]), "empty"),
                (None, "missing"),
                (Some(&b"\n"[..]), "newline"),
            ]);
            files.push(File {
                name: "m.json".into(),
                data: mid.map(<[u8]>::to_vec),
            });
            layout = format!("seam3({label})");
        } else {
            layout = "seam".into();
        }
        let mut tail = b.to_vec();
        tail.extend_from_slice(r.pick(LATER));
        files.push(File {
            name: "b.json".into(),
            data: Some(tail),
        });
    } else if r.chance(25) {
        files.push(File {
            name: "a.json".into(),
            data: Some(body),
        });
        files.push(File {
            name: "b.json".into(),
            data: Some(r.pick(&LATER[1..]).to_vec()),
        });
        layout = "infile+later".into();
    } else {
        files.push(File {
            name: "a.json".into(),
            data: Some(body),
        });
        layout = "infile".into();
    }
    let missing = files.iter().any(|f| f.data.is_none());
    let entry = if missing {
        Entry::Files
    } else {
        *r.pick(&[
            Entry::Files,
            Entry::Files,
            Entry::StdinFile,
            Entry::StdinPipe,
        ])
    };
    Case {
        index,
        family,
        edge,
        flags,
        entry,
        shape: format!(
            "{layout} fill={fill}{}",
            if bom_first { " bom-first" } else { "" }
        ),
        files,
    }
}

/// The spike's xorshift64*.
struct SpikeRng(u64);

impl SpikeRng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Option G's `gen_case`, which drew every case from one stream: case `index` is the `index`-th
/// draw after seeding, so this replays the stream up to it (cheap: a few hundred draws a case).
fn spike_case(seed: u64, index: u64) -> Case {
    const PIECES: &[&[u8]] = &[
        b"1",
        b"23",
        b"\"ab\"",
        b"[1,2]",
        b"{\"a\":1}",
        b"true",
        b"null",
        b"\0",
        b"1\0x",
        b"\xef\xbb\xbf",
        b"\xef\xbb",
        b"\xff",
        b"\"\xe0\x80\x80\"",
        b"\"\xf0\x9f\x98\x80\"",
        b"\"\xc3",
        b"}",
        b"l",
        b"tr",
        b"\n",
        b"\r\n",
        b" ",
        b"[",
        b"]",
        b"\"",
    ];
    let mut rng = SpikeRng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut files = Vec::new();
    for i in 0..=index {
        files.clear();
        let nfiles = 1 + rng.below(3) as usize;
        for _ in 0..nfiles {
            let mut f = Vec::new();
            let target = [0usize, 10, 4080, 4085, 4088, 4090, 4091, 4094, 4095, 8180]
                [rng.below(10) as usize];
            if target > 0 {
                let pad = target - rng.below(8) as usize;
                if rng.below(3) == 0 {
                    f.push(b'"');
                    f.extend(std::iter::repeat_n(b'a', pad.saturating_sub(2)));
                    f.push(b'"');
                } else {
                    f.extend(std::iter::repeat_n(b' ', pad));
                }
            }
            for _ in 0..rng.below(8) {
                f.extend_from_slice(PIECES[rng.below(PIECES.len() as u64) as usize]);
                if rng.below(2) == 0 {
                    f.push(b' ');
                }
            }
            files.push(f);
        }
        if i == index {
            break;
        }
    }
    let n = files.len();
    Case {
        index,
        family: Family::Spike,
        edge: if n > 1 { Edge::Seam } else { Edge::Read1 },
        flags: Flags::new(false, false, false),
        entry: Entry::Files,
        shape: format!("spike files={n}"),
        files: files
            .into_iter()
            .enumerate()
            .map(|(i, d)| File {
                name: format!("f{i}"),
                data: Some(d),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_case_is_a_function_of_seed_and_index() {
        for i in [0, 1, 17, 999] {
            assert_eq!(Generator::Edge.case(3, i), Generator::Edge.case(3, i));
            assert_eq!(Generator::Spike.case(1, i), Generator::Spike.case(1, i));
        }
        assert_ne!(
            Generator::Edge.case(3, 1).files,
            Generator::Edge.case(4, 1).files
        );
    }

    #[test]
    fn edge_pieces_land_in_the_named_band() {
        // Read1/Read2 pieces start inside their band: the filler plus prefix has exactly that length.
        let mut seen = 0;
        for i in 0..2000 {
            let c = Generator::Edge.case(1, i);
            let band = match c.edge {
                Edge::Read1 => 4085..=4100,
                Edge::Read2 => 8180..=8200,
                _ => continue,
            };
            let first = c.files[0].data.as_ref().expect("a.json exists");
            assert!(
                first.len() > *band.start(),
                "case {i}: {} bytes",
                first.len()
            );
            seen += 1;
        }
        assert!(seen > 500, "{seen}");
    }

    #[test]
    fn every_family_and_flag_mix_is_generated() {
        let mut fam = std::collections::BTreeSet::new();
        let mut flags = std::collections::BTreeSet::new();
        for i in 0..3000 {
            let c = Generator::Edge.case(1, i);
            fam.insert(c.family);
            flags.insert(c.flags);
        }
        assert_eq!(fam.len(), 9);
        assert_eq!(flags.len(), Flags::ALL.len());
    }
}
