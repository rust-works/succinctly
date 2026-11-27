//! The reader model against jq 1.7.1, and `skip_plain` against `step`.

use super::*;
use crate::cli::jq_reader_profile::JQ_READER;

const NON_SEQ: ReadMode = ReadMode { seq: false };

/// A row's files: literal pieces, and runs of one byte.
#[derive(Clone, Copy)]
enum Piece {
    Lit(&'static [u8]),
    Run(u8, usize),
}
use Piece::{Lit, Run};

/// A row of the differential table: input files and what jq 1.7.1 prints for them
/// (`scripts/gen_reader_model_cases.py` says how each was captured).
struct Case {
    name: &'static str,
    /// The row was read under `--seq`.
    seq: bool,
    files: &'static [&'static [Piece]],
    /// `/usr/bin/jq -nc '[limit(12; repeat(try [input, input_line_number, input_filename] catch
    /// [., input_line_number, input_filename]))]' f0 f1 ...`.
    jq171: &'static str,
    /// A recorded rule the model follows where jq 1.7.1 does not, and the answer under it, which
    /// is `succinctly jq`'s (the rule is implemented on every route).
    departs: Option<(&'static str, &'static str)>,
}

include!("jq171_cases.rs");

fn build(files: &[&[Piece]]) -> Vec<Vec<u8>> {
    files
        .iter()
        .map(|pieces| {
            let mut bytes = Vec::new();
            for piece in *pieces {
                match *piece {
                    Lit(text) => bytes.extend_from_slice(text),
                    Run(byte, count) => bytes.resize(bytes.len() + count, byte),
                }
            }
            bytes
        })
        .collect()
}

/// `text` as a JSON string, as jq's `-c` prints a message.
fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The first `calls` answers of `input` over `files`, as the model reads them.
fn model_reads(
    files: &[&[u8]],
    profile: &ReaderProfile,
    mode: ReadMode,
    calls: usize,
) -> Vec<Read> {
    let mut state = ReaderState::start(mode);
    let mut reads = Vec::new();
    while reads.len() < calls {
        state.step(files, profile, mode, &mut reads);
    }
    reads.truncate(calls);
    reads
}

/// The row's program over `files`, as the model reads them, printed as jq's `-c` prints it. A
/// value is its text in the inputs: every row's values are written as jq prints them.
fn render(files: &[&[u8]], mode: ReadMode) -> String {
    let stream: Vec<u8> = files.concat();
    let rendered: Vec<String> = model_reads(files, &JQ_READER, mode, 12)
        .iter()
        .map(|read| {
            let first = match read.item {
                Item::Value(span) => {
                    String::from_utf8_lossy(&stream[span.start..span.end]).into_owned()
                }
                Item::Fault(fault) => json_string(&fault.message()),
                Item::End => json_string("break"),
            };
            let name = read
                .at
                .file
                .map_or_else(|| String::from("null"), |file| format!("\"f{file}\""));
            format!("[{first},{},{name}]", read.at.line)
        })
        .collect();
    format!("[{}]", rendered.join(","))
}

/// ADR-0033 step 3: the model, unused, answers every row of the differential table as jq 1.7.1
/// does, except where a recorded rule departs from it, where it answers by the rule.
#[test]
fn the_model_reads_every_row_as_jq_1_7_1_7987() {
    let mut failed = Vec::new();
    for case in CASES {
        let files = build(case.files);
        let files: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
        let got = render(&files, ReadMode { seq: case.seq });
        let want = case.departs.map_or(case.jq171, |(_, expected)| expected);
        if got != want {
            failed.push(format!("{}:\n  model: {got}\n  want:  {want}", case.name));
        }
    }
    assert!(
        failed.is_empty(),
        "{} of {} rows differ:\n{}",
        failed.len(),
        CASES.len(),
        failed.join("\n")
    );
}

/// The table covers what step 3 promises to prove (read boundaries, a fault on a long line, a
/// mark and a partial one, depth 256 and 257, a NUL outside and inside a string), and every row
/// that departs from jq 1.7.1 names a recorded rule and really departs.
#[test]
fn the_differential_table_covers_the_reader_edges_7987() {
    assert!(CASES.len() >= 90, "{} rows", CASES.len());
    for needed in [
        "line_of_4095_then_newline",
        "token_cut_by_the_read",
        "fault_on_a_long_line_resumes_at_8190",
        "bom_whole",
        "bom_malformed_after_two",
        "bom_partial_one_at_end",
        "depth_256_objects",
        "depth_256_arrays_and_objects",
        "depth_257_arrays",
        "depth_257_objects",
        "nul_between_values",
        "nul_in_string",
    ] {
        assert!(CASES.iter().any(|case| case.name == needed), "{needed}");
    }
    for case in CASES {
        if let Some((rule, expected)) = case.departs {
            assert!(
                rule.starts_with('#') || rule.starts_with("E2"),
                "{}",
                case.name
            );
            assert_ne!(expected, case.jq171, "{}", case.name);
        }
    }
}

/// Every reader fact is the profile's: the read length and the depth limit move the answer when
/// the profile moves them.
#[test]
fn the_model_reads_its_facts_from_the_profile_7987() {
    let short = ReaderProfile {
        read_len: 4,
        max_depth: 2,
        ..JQ_READER
    };
    let reads = |files: &[&[u8]], profile: &ReaderProfile, calls| -> Vec<String> {
        let stream = files.concat();
        model_reads(files, profile, NON_SEQ, calls)
            .iter()
            .map(|read| match read.item {
                Item::Value(span) => {
                    String::from_utf8_lossy(&stream[span.start..span.end]).into_owned()
                }
                Item::Fault(fault) => fault.message(),
                Item::End => String::from("break"),
            })
            .collect()
    };
    // `1 }` and the rest of its read: jq's whole line, or here four bytes, so `2` goes with it.
    let files: &[&[u8]] = &[b"1 }2 3 45\n"];
    assert_eq!(
        reads(files, &JQ_READER, 3),
        ["1", "Unmatched '}' at line 1, column 3", "break"]
    );
    assert_eq!(
        reads(files, &short, 4),
        ["1", "Unmatched '}' at line 1, column 3", "3", "45"]
    );
    let files: &[&[u8]] = &[b"[[]] [[[]]]\n"];
    assert_eq!(
        reads(files, &short, 2),
        [
            "[[]]",
            "Exceeds depth limit for parsing at line 1, column 8"
        ]
    );
    assert_eq!(reads(files, &JQ_READER, 2), ["[[]]", "[[[]]]"]);
}

/// A stuck `Malformed BOM` answers every call without reading, and `step` says so.
#[test]
fn a_malformed_mark_is_every_call_without_a_read_7987() {
    let files: &[&[u8]] = &[b"\xEF\xBB1\n", b"2\n"];
    let mut state = ReaderState::start(NON_SEQ);
    let mut reads = Vec::new();
    assert_eq!(
        state.step(files, &JQ_READER, NON_SEQ, &mut reads),
        Stop::Stuck
    );
    let stuck = state;
    for _ in 0..3 {
        assert_eq!(
            state.step(files, &JQ_READER, NON_SEQ, &mut reads),
            Stop::Stuck
        );
        assert_eq!(state, stuck);
    }
    assert_eq!(reads.len(), 4);
    for read in &reads {
        let Item::Fault(fault) = read.item else {
            panic!("{read:?}");
        };
        assert_eq!(fault.message(), "Malformed BOM");
        assert_eq!(
            read.at,
            Position {
                file: Some(0),
                line: 1
            }
        );
    }
    // A stuck state is no plain stretch.
    let run = PlainRun {
        file: 0,
        len: 2,
        newlines: 1,
        tail: 0,
        to_end: false,
        bom: false,
    };
    let stretch = PlainStretch {
        runs: &[run],
        files: 2,
        value_in_final_buffer: false,
    };
    assert_eq!(stuck.skip_plain(&stretch, &JQ_READER, NON_SEQ), None);
}

// ---------------------------------------------------------------------------------------------
// The `skip_plain == step` property.
// ---------------------------------------------------------------------------------------------

/// xorshift64, so a failing case can be rerun from its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// A generated plain stream: its values, in order, as `(span, bare)` where `bare` is a number or
/// literal (completed by the byte after it), and the stream cut into files.
struct Generated {
    files: Vec<Vec<u8>>,
    values: Vec<(Span, bool)>,
}

/// White space whose lengths put newlines and values at and around the 4,095-byte read edges
/// (`read_len`), and short runs.
fn white_space(rng: &mut Rng, read_len: usize) -> Vec<u8> {
    let edge = [
        read_len - 2,
        read_len - 1,
        read_len,
        read_len + 1,
        2 * read_len - 1,
        2 * read_len,
        2 * read_len + 1,
    ];
    match rng.below(10) {
        0 => {
            // A long run of spaces, then maybe a newline right at its end.
            let mut run = vec![b' '; rng.pick(&edge).saturating_sub(rng.below(3)).max(1)];
            if rng.below(2) == 0 {
                run.push(b'\n');
            }
            run
        }
        1 => b"\n\n".to_vec(),
        2 => b"\r\n".to_vec(),
        3 => b"\t".to_vec(),
        4 | 5 => b"\n".to_vec(),
        _ => b" ".to_vec(),
    }
}

/// A plain stream: values separated by white space (none between two values a bracket or quote
/// already separates), maybe a whole mark at the front, maybe no white space at the end; cut into
/// one to five files at random bytes (inside tokens too), empty files among them.
fn generate(rng: &mut Rng, read_len: usize) -> Generated {
    const VALUES: &[(&[u8], bool)] = &[
        (b"1", true),
        (b"23", true),
        (b"-4.5e1", true),
        (b"true", true),
        (b"false", true),
        (b"null", true),
        (b"\"ab\"", false),
        (b"\"x\\\"y\"", false),
        (b"\"\"", false),
        (b"[1,2]", false),
        (b"[]", false),
        (b"{\"k\":[3,{}]}", false),
        (b"[\"a b\",\n4]", false),
    ];
    let mut stream = Vec::new();
    if rng.below(4) == 0 {
        stream.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    let bom = stream.len();
    if rng.below(3) == 0 {
        stream.extend(white_space(rng, read_len));
    }
    let mut values = Vec::new();
    let count = 1 + rng.below(12);
    let mut last_bare = false;
    for i in 0..count {
        let &(text, bare) = rng.pick(VALUES);
        if i > 0 && (bare && last_bare || rng.below(3) > 0) {
            stream.extend(white_space(rng, read_len));
        }
        let start = stream.len();
        stream.extend_from_slice(text);
        values.push((
            Span {
                start,
                end: stream.len(),
            },
            bare,
        ));
        last_bare = bare;
    }
    if rng.below(3) > 0 {
        stream.extend(white_space(rng, read_len));
    }
    // Cut points, never inside the mark (a mark cut across files is a trigger, not plain).
    let mut cuts: Vec<usize> = (0..rng.below(5))
        .map(|_| {
            if stream.len() > bom && rng.below(4) > 0 {
                bom + rng.below(stream.len() - bom + 1)
            } else if rng.below(2) == 0 {
                0
            } else {
                stream.len()
            }
        })
        .collect();
    cuts.sort_unstable();
    let mut files = Vec::new();
    let mut from = 0;
    for cut in cuts {
        files.push(stream[from..cut].to_vec());
        from = cut;
    }
    files.push(stream[from..].to_vec());
    Generated { files, values }
}

/// The facts a route holds about the bytes from where `from` stands to `(file, offset)`.
fn runs_between(files: &[&[u8]], from: &ReaderState, file: usize, offset: usize) -> Vec<PlainRun> {
    let (first, mut start) = match from.input {
        Some(open) if !open.eof => (from.next_file - 1, open.offset),
        _ => (from.next_file, 0),
    };
    let mut runs = Vec::new();
    for (index, source) in files.iter().enumerate().take(file + 1).skip(first) {
        let end = if index == file { offset } else { source.len() };
        let bytes = &source[start..end];
        let newlines = bytes.iter().filter(|&&b| b == b'\n').count();
        let tail = bytes
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(bytes.len(), |at| bytes.len() - at - 1);
        runs.push(PlainRun {
            file: index,
            len: bytes.len(),
            newlines,
            tail,
            to_end: end == source.len(),
            bom: bytes.starts_with(&[0xEF, 0xBB, 0xBF]),
        });
        start = 0;
    }
    runs
}

/// Whether a value completes in the buffer jq marks as its last, derived from the stream's layout
/// and not from the model: the last file's last read when it is short (it met the end of the
/// file), else the empty read after it; the end of input completes a bare token there.
fn value_in_final_buffer(gen: &Generated, read_len: usize) -> bool {
    let total: usize = gen.files.iter().map(Vec::len).sum();
    let last = gen.files.last().expect("a file");
    let tail = last
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(last.len(), |at| last.len() - at - 1);
    let short = last.is_empty() || tail % read_len != 0;
    let final_start = if short {
        total - tail % read_len
    } else {
        total
    };
    gen.values.iter().any(|&(span, bare)| {
        let completes = if bare { span.end } else { span.end - 1 };
        (bare && span.end == total) || (completes >= final_start && completes < total)
    })
}

/// Where a state stands in the inputs: the open file and its read position, or the front of the
/// next file.
fn stands(state: &ReaderState) -> (usize, usize) {
    match state.input {
        Some(open) => (state.next_file - 1, open.offset),
        None => (state.next_file, 0),
    }
}

/// One generated case: `step` from the start to the end of the input, then `skip_plain` from
/// each sync point `step` stopped at to each later one, and to the end.
fn check_case(seed: u64, read_len: usize) {
    let mut rng = Rng(seed);
    let gen = generate(&mut rng, read_len);
    let profile = ReaderProfile {
        read_len,
        ..JQ_READER
    };
    let files: Vec<&[u8]> = gen.files.iter().map(Vec::as_slice).collect();
    let nfiles = files.len();
    let mut state = ReaderState::start(NON_SEQ);
    // Every sync point `step` stops at before the end, and the state after the end.
    let mut stops = vec![state];
    let mut reads = Vec::new();
    let end = loop {
        assert_eq!(
            state.step(&files, &profile, NON_SEQ, &mut reads),
            Stop::Synced,
            "{seed}"
        );
        if reads.last().is_some_and(|read| read.item == Item::End) {
            break state;
        }
        stops.push(state);
        assert!(stops.len() < 100_000, "{seed}: no end");
    };
    // A plain stream reads as its values and then the end.
    let spans: Vec<Item> = gen
        .values
        .iter()
        .map(|&(span, _)| Item::Value(span))
        .collect();
    let items: Vec<Item> = reads.iter().map(|read| read.item).collect();
    assert_eq!(items[..items.len() - 1], spans[..], "{seed}");
    assert_eq!(items.last(), Some(&Item::End), "{seed}");

    // The sync points `skip_plain` can name: the first at each place in the inputs, short of the
    // end of the last file (a stretch that reaches it goes on to the end of input).
    let mut points = vec![stops[0]];
    for pair in stops.windows(2) {
        let (file, offset) = stands(&pair[1]);
        let at_end = file + 1 == nfiles && offset == files[file].len();
        if stands(&pair[1]) != stands(&pair[0]) && !at_end && !pair[1].parser.eof {
            points.push(pair[1]);
        }
    }
    let stretch_of = |runs: &[PlainRun], to_end: bool| {
        (
            runs.to_vec(),
            to_end && value_in_final_buffer(&gen, read_len),
        )
    };
    for (j, from) in points.iter().enumerate() {
        // To every later point, from the start and from a few others.
        if j == 0 || rng.below(4) == 0 {
            for to in &points[j + 1..] {
                let (file, offset) = stands(to);
                let (runs, flag) = stretch_of(&runs_between(&files, from, file, offset), false);
                let stretch = PlainStretch {
                    runs: &runs,
                    files: nfiles,
                    value_in_final_buffer: flag,
                };
                assert_eq!(
                    from.skip_plain(&stretch, &profile, NON_SEQ),
                    Some(*to),
                    "seed {seed}, read_len {read_len}: {from:?} -> {to:?} over {runs:?}"
                );
            }
        }
        // To the end of the input.
        let last = nfiles - 1;
        let (runs, flag) = stretch_of(&runs_between(&files, from, last, files[last].len()), true);
        let stretch = PlainStretch {
            runs: &runs,
            files: nfiles,
            value_in_final_buffer: flag,
        };
        assert_eq!(
            from.skip_plain(&stretch, &profile, NON_SEQ),
            Some(end),
            "seed {seed}, read_len {read_len}: {from:?} -> the end over {runs:?} ({flag})"
        );
    }
}

/// ADR-0033: for every plain stretch, `skip_plain` reaches the state `step` reaches, from every
/// sync point to every later one and to the end of the input, over generated streams of lengths
/// around one and two reads, newlines at the read edges, values cut across files, empty files and
/// a mark; at jq's 4,095-byte read and at short reads that put many edges in a small stream.
#[test]
fn skip_plain_is_step_over_every_plain_stretch_7987() {
    for seed in 1..=400u64 {
        check_case(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15), JQ_READER.read_len);
    }
    for read_len in [3, 4, 7, 16] {
        for seed in 1..=400u64 {
            check_case(
                seed.wrapping_mul(0xD1B5_4A32_D192_ED03) ^ read_len as u64,
                read_len,
            );
        }
    }
}

/// `skip_plain` refuses what it cannot answer: `--seq`, a run that does not end at a read edge, a
/// file skipped, and a stretch that does not reach the end of a file it leaves.
#[test]
fn skip_plain_refuses_what_is_not_its_closed_form_7987() {
    let start = ReaderState::start(NON_SEQ);
    let run = |file, len, newlines, tail, to_end| PlainRun {
        file,
        len,
        newlines,
        tail,
        to_end,
        bom: false,
    };
    let skip = |state: &ReaderState, runs: &[PlainRun], mode| {
        state.skip_plain(
            &PlainStretch {
                runs,
                files: 3,
                value_in_final_buffer: false,
            },
            &JQ_READER,
            mode,
        )
    };
    let good = [run(0, 10, 1, 0, false)];
    assert!(skip(&start, &good, NON_SEQ).is_some());
    assert_eq!(skip(&start, &good, ReadMode { seq: true }), None);
    assert_eq!(skip(&start, &[run(0, 10, 1, 3, false)], NON_SEQ), None);
    assert_eq!(skip(&start, &[run(1, 10, 1, 0, false)], NON_SEQ), None);
    assert_eq!(
        skip(
            &start,
            &[run(0, 10, 1, 0, false), run(1, 2, 1, 0, false)],
            NON_SEQ
        ),
        None
    );
    assert_eq!(skip(&start, &[run(0, 10, 0, 3, false)], NON_SEQ), None);
    assert_eq!(skip(&start, &[], NON_SEQ), None);
    let after = skip(&start, &good, NON_SEQ).expect("plain");
    assert_eq!(skip(&after, &[run(1, 1, 1, 0, true)], NON_SEQ), None);
}

// ---------------------------------------------------------------------------------------------
// The read arithmetic the routes call (#7987 step 4): `LineReads` against `Fgets` one read at a
// time, as `step`'s `read_more` takes them.
// ---------------------------------------------------------------------------------------------

/// Files of `a`s and newlines whose lines run to around one and two reads, and short ones.
fn line_files(rng: &mut Rng, read_len: usize) -> Vec<Vec<u8>> {
    let lens = [
        0,
        1,
        read_len.saturating_sub(1),
        read_len,
        read_len + 1,
        2 * read_len - 1,
        2 * read_len,
        2 * read_len + 1,
    ];
    (0..=rng.below(3))
        .map(|_| {
            let mut file = Vec::new();
            for _ in 0..rng.below(5) {
                file.resize(file.len() + *rng.pick(&lens) + rng.below(2), b'a');
                if rng.below(4) != 0 {
                    file.push(b'\n');
                }
            }
            file
        })
        .collect()
}

/// Where each line's reads fall and where the read that meets a file's end starts are
/// [`LineReads`]'s closed forms of what [`Fgets`] reaches one read at a time: the reads `step`
/// takes. The routes' read formulas (`EofTail`'s, `LineCounter`'s, the `--seq` reader's) call these,
/// so this holds them to the model's own reads.
#[test]
fn line_reads_are_the_reads_step_takes_7987() {
    for read_len in [1, 2, 3, 4, 7, 16, JQ_READER.read_len] {
        let profile = ReaderProfile {
            read_len,
            ..JQ_READER
        };
        let reads = LineReads::of(&profile);
        for seed in 1..=300u64 {
            let mut rng = Rng(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ read_len as u64);
            let files = line_files(&mut rng, read_len);
            let sources: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
            let mut reader =
                Reader::resume(ReaderState::start(NON_SEQ), &sources, &profile, NON_SEQ);
            // Per file: where its last line starts, the reads of that line so far, and where the
            // read that met its end started.
            let mut line_start = 0;
            let mut line_reads = 0;
            let mut met_end = vec![None; files.len()];
            loop {
                let (chunk, is_last) = reader.read_more();
                let file = reader.next_file.checked_sub(1);
                let (Some(file), Some(open)) = (file, reader.input) else {
                    assert!(is_last && chunk.is_empty(), "seed {seed}");
                    break;
                };
                let bytes = sources[file];
                let start = open.offset - chunk.len();
                if start == 0 {
                    line_start = 0;
                    line_reads = 0;
                }
                let context = format!("read_len {read_len}, seed {seed}, file {file} at {start}");
                if !chunk.is_empty() {
                    assert_eq!(reads.index(start - line_start), line_reads, "{context}");
                    let last = start + chunk.len() - 1;
                    assert_eq!(reads.index(last - line_start), line_reads, "{context}");
                    let newline = bytes[start..].iter().position(|&b| b == b'\n');
                    let end = (line_start + reads.end(start - line_start))
                        .min(newline.map_or(bytes.len(), |at| start + at + 1))
                        .min(bytes.len());
                    assert_eq!(start + chunk.len(), end, "{context}");
                    let read = Fgets::of(&bytes[start..], read_len).expect("a read");
                    assert_eq!(read.len, chunk.len(), "{context}");
                    assert_eq!(read.meets_end(read_len), open.eof, "{context}");
                    line_reads += 1;
                    if read.newline {
                        line_start = start + chunk.len();
                        line_reads = 0;
                    }
                }
                if open.eof && met_end[file].is_none() {
                    met_end[file] = Some(start);
                }
                if is_last {
                    break;
                }
            }
            for (file, bytes) in files.iter().enumerate() {
                let line_start = bytes
                    .iter()
                    .rposition(|&b| b == b'\n')
                    .map_or(0, |at| at + 1);
                assert_eq!(
                    Some(reads.start_before(line_start, bytes.len())),
                    met_end[file],
                    "read_len {read_len}, seed {seed}, file {file}: {:?}",
                    files.iter().map(Vec::len).collect::<Vec<_>>()
                );
            }
        }
    }
}

/// The shapes `EofTail` documents, at jq's 4,095-byte read: a last line that fills its reads leaves
/// an empty read to meet the end, and one short of that is met by the read that holds its last byte.
#[test]
fn the_read_that_meets_the_end_at_4095_bytes_7987() {
    let reads = JQ_LINE_READS;
    assert_eq!(JQ_READER.read_len, 4095);
    // `1` and 4,093 spaces: one short read.
    assert_eq!(reads.start_before(0, 4094), 0);
    // 4,094 spaces and `1`: one full read, and the empty one after it.
    assert_eq!(reads.start_before(0, 4095), 4095);
    assert_eq!(reads.start_before(0, 4096), 4095);
    // After a newline the reads start again.
    assert_eq!(reads.start_before(10, 10 + 2 * 4095), 10 + 2 * 4095);
    assert_eq!(reads.start_before(10, 11 + 2 * 4095), 10 + 2 * 4095);
    assert_eq!((reads.index(4094), reads.end(4094)), (0, 4095));
    assert_eq!((reads.index(4095), reads.end(4095)), (1, 8190));
    let full = Fgets::of(&[b'a'; 5000], 4095).expect("a read");
    assert_eq!(
        full,
        Fgets {
            len: 4095,
            newline: false
        }
    );
    assert!(full.fills(4095) && !full.meets_end(4095));
    let line = Fgets::of(b"ab\ncd", 4095).expect("a read");
    assert_eq!(
        line,
        Fgets {
            len: 3,
            newline: true
        }
    );
    assert!(!line.fills(4095) && !line.meets_end(4095));
    let short = Fgets::of(b"cd", 4095).expect("a read");
    assert!(!short.fills(4095) && short.meets_end(4095));
    // A read with less room: the first of a source whose mark a route took off its text (#7569).
    assert_eq!(
        Fgets::of(&[b'a'; 5000], 4092).map(|read| read.len),
        Some(4092)
    );
    assert_eq!(Fgets::of(b"", 4095), None);
}
