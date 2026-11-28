//! The default route's model against the model alone, and the state it carries against the state
//! from the stream's start.

use super::*;
use crate::cli::jq_runner::{
    model_read, note_bom_leads, parse_json_stream_prefix, split_json_values,
    stitch_json_file_seams, strip_leading_json_bom, utf8_lossy_document, StreamOrigin,
};

/// A small deterministic generator (xorshift).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// A stream of well-formed values, some of them strings holding invalid UTF-8, some lines long
/// enough to cross jq's 4,095-byte reads, perhaps a mark (whole or cut) in front and a fault at the
/// end that the splitter meets (`[1 2]` it would delimit unread, #3035): everything a malformed
/// value could hide from the semi-index is in a region the model reads.
fn stream(rng: &mut Rng) -> Vec<u8> {
    const VALUES: &[&[u8]] = &[
        b"1",
        b"[1,2]",
        b"{\"a\":\"b\"}",
        b"\"s\"",
        b"null",
        b"\"\xff\"",
        b"\"a\xc3\"",
        b"[\"\xe2\x82\"]",
        b"{\"\xe9\":1}",
        b"-1.5e3",
    ];
    let mut out = Vec::new();
    match rng.below(6) {
        0 => out.extend_from_slice(&UTF8_BOM),
        1 => out.extend_from_slice(b"\xef\xbb"),
        _ => {}
    }
    for _ in 0..=rng.below(12) {
        out.extend_from_slice(VALUES[rng.below(VALUES.len())]);
        match rng.below(8) {
            0 => out.extend(std::iter::repeat(b' ').take(4_080 + rng.below(30))),
            1 | 2 => out.push(b' '),
            _ => out.push(b'\n'),
        }
    }
    if rng.below(3) == 0 {
        out.extend_from_slice([&b"}\n"[..], b"1}\n", b"\"abc"][rng.below(3)]);
    }
    out
}

/// What the route answers for `files` read whole, with the model on: each value, and the fault the
/// run stops at, in `transcript_alone`'s words without the positions. The entries it carried state
/// into are left in [`ENTRIES`].
fn route(files: &[Vec<u8>]) -> Vec<String> {
    let mut raws = files.to_vec();
    let as_read = AsRead::of(&raws);
    let cuts = strip_leading_json_bom(raws.iter_mut());
    let mut seams = stitch_json_file_seams(raws.iter_mut(), false, false, &cuts);
    note_bom_leads(&mut seams, &cuts, raws.len());
    let mut model = DefaultModel::new(as_read, &cuts);
    let (texts, lossy): (Vec<Vec<u8>>, Vec<Option<LossySource>>) = raws
        .into_iter()
        .enumerate()
        .map(|(index, raw)| utf8_lossy_document(raw, seams.get_mut(index)))
        .unzip();
    let mut seen = Vec::new();
    for (file, text) in texts.iter().enumerate() {
        let (mut values, mut split_error) = split_json_values(text);
        let fault = if model.plans(split_error, lossy[file].is_some()) {
            let route_texts = Texts {
                files: &texts,
                lossy: &lossy,
                seams: &seams,
            };
            model.plan(file, &route_texts, &mut values, &mut split_error)
        } else {
            None
        };
        assert!(split_error.is_none() || fault.is_none());
        for (start, end) in values {
            let text = std::str::from_utf8(&text[start..end]).expect("substituted");
            match parse_json_stream_prefix(text, Some(StreamOrigin::ALONE)) {
                (mut one, None) if one.len() == 1 => {
                    seen.push(format!("doc {:?}", one.pop().expect("one")));
                }
                other => panic!("{text:?} is not one value: {other:?}"),
            }
        }
        if let Some(message) = fault {
            seen.push(format!("error {message}"));
            return seen;
        }
        if split_error.is_some() {
            // The route's own fault, which the model did not see: worded by the old reader.
            seen.push("error (route)".to_owned());
            return seen;
        }
    }
    seen
}

/// The model alone over `files` from the stream's start, through its first fault or the end.
fn alone(files: &[Vec<u8>]) -> Vec<String> {
    let slices: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
    let mut seen = Vec::new();
    for read in model_read::transcript_alone(&slices) {
        let (what, _) = read.rsplit_once(" at ").expect("a place");
        if what == "end" {
            break;
        }
        let fault = what.starts_with("error ");
        seen.push(what.to_owned());
        if fault {
            break;
        }
    }
    seen
}

/// `data` cut into 1 to 3 files, an empty one among them now and then.
fn cut(rng: &mut Rng, data: &[u8]) -> Vec<Vec<u8>> {
    let mut cuts: Vec<usize> = (0..rng.below(3))
        .map(|_| rng.below(data.len() + 1))
        .collect();
    cuts.sort_unstable();
    let mut files = Vec::new();
    let mut at = 0;
    for cut in cuts {
        files.push(data[at..cut].to_vec());
        at = cut;
    }
    files.push(data[at..].to_vec());
    files
}

/// ADR-0033 step 6: over input where every edge is a trigger, the route's values and fault (the
/// semi-index's between the regions, the model's in them) are the model's own from the stream's
/// start; and every state a region is entered with, carried from the last hand-back over the
/// stretch the semi-index read, is the state `skip_plain` gives over everything from the start.
#[test]
fn the_route_with_hand_back_reads_what_the_model_alone_reads_7987() {
    let mut rng = Rng(0x7987_0006);
    let mut carried = 0;
    for case in 0..600 {
        let data = stream(&mut rng);
        let files = cut(&mut rng, &data);
        ENTRIES.with(|log| log.borrow_mut().clear());
        let got = route(&files);
        assert_eq!(got, alone(&files), "case {case}: {files:?}");
        let entries = ENTRIES.with(|log| std::mem::take(&mut *log.borrow_mut()));
        if entries.is_empty() {
            continue;
        }
        // The stream the model read, as `Stream::of` rebuilds it, is the files as read.
        let stream = Stream {
            bytes: data.clone(),
            sources: std::iter::once(0)
                .chain(files.iter().scan(0, |at, file| {
                    *at += file.len();
                    Some(*at)
                }))
                .collect(),
            texts: Vec::new(),
        };
        let slices = stream.slices();
        let start = ReaderState::start(NON_SEQ);
        for (entry, state) in entries {
            let runs = stream.runs(&start, 0, entry);
            let stretch = PlainStretch {
                runs: &runs,
                files: slices.len(),
                value_in_final_buffer: false,
            };
            assert_eq!(
                start.skip_plain(&stretch, &JQ_READER, NON_SEQ),
                Some(state),
                "case {case}: the state carried to {entry}"
            );
            carried += 1;
        }
    }
    assert!(carried > 100, "only {carried} entries carried state");
}
