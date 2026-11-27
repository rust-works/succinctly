//! Classify one difference between `succinctly jq` and the two jq releases.
//!
//! The owner's decisions (2026-11-27, ADR-0018 amendment #7963) set the classes:
//! - the reader follows **jq 1.7.1**, the pinned release, except for recorded 1.8.2 adoptions;
//! - **E1:** after the first fault, output is best effort above a floor;
//! - **E2:** a NUL outside a string is a parse error.
//!
//! The predicates are spike E's (`preds.py`), ported to run on whole transcripts and tightened
//! to the amendment:
//! - **E1** ([`e1_clean`]): through and including the first fault, per channel, succinctly
//!   equals the release: the values the release's reader gave before the fault ([`Facts`]), the
//!   message and its position, and the exit status when that fault ends the run. The exit status
//!   is zero exactly when jq 1.7.1's is. The release must have a fault.
//! - **E2** ([`e2_consistent`]): the stream holds a NUL outside a string. Where a release's first
//!   fault comes before the NUL, succinctly meets E1 against that release; otherwise succinctly's
//!   first fault is at the NUL's own line and column, and the values before it are jq 1.7.1's
//!   from the bytes before the NUL ([`values_before_nul`]).
//! - The floor of both: a value neither release prints is never accepted ([`wrong_value`]).

use std::sync::OnceLock;

use regex::bytes::Regex;
use serde_json::Value;

use crate::reader::Transcript;

/// How a case's standard output carries its values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Shape {
    /// The `-n` trace program: one `["V", v, ...]` or `["E", msg, ...]` line per item.
    pub trace: bool,
    /// `-s`: each value is a slurped array, compared member by member.
    pub slurp: bool,
    /// `--seq`: a parse error does not end the run.
    pub seq: bool,
    /// Spike E's line-prefix E1 ([`e1_clean`]); off by default.
    pub lenient_e1: bool,
}

/// What the oracle knows about a case beyond the three transcripts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// How many items jq 1.7.1's and jq 1.8.2's readers gave before their first parse error;
    /// `None` when a reader gave none (or it is not known).
    pub before_fault: [Option<usize>; 2],
    /// What jq 1.7.1's reader gives from the bytes wholly before the first NUL outside a string
    /// ([`values_before_nul`]), as `jq -c` prints each value; `None` without such a NUL.
    pub before_nul: Option<Vec<Vec<u8>>>,
    /// jq 1.7.1's and jq 1.8.2's transcripts for the case with every invalid escape made valid
    /// ([`escapes_made_valid`]), when the stream has one: what they print where succinctly
    /// forwards the string (#5308).
    pub escapes_valid: Option<(Transcript, Transcript)>,
}

/// The verdict on one case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Class {
    /// Succinctly equals both releases, which agree.
    Same,
    /// The releases differ and succinctly equals 1.7.1, the pinned one.
    Split171,
    /// The releases differ and succinctly equals 1.8.2 only: a release split, which is a
    /// deviation from the pin unless it is a recorded 1.8.2 adoption.
    Split182,
    /// Covered by E2: a NUL outside a string, reported as a fault at its own line and column.
    E2,
    /// Covered by E1: equal through the first fault, best effort after it.
    E1,
    /// A NUL outside a string that is the first fault, which succinctly does not report there,
    /// with no fault before it, the values before it kept and no value neither release prints:
    /// E2's ground, not implemented yet (#7966).
    E2Pending,
    /// A divergence `docs/compliance/jq/limitations.md` already records ([`recorded`] names it).
    Recorded,
    /// The releases agree and succinctly differs, outside E1 and E2: a bug.
    BugAgree,
    /// The releases differ and succinctly equals neither, outside E1 and E2.
    BugSplit,
}

impl Class {
    pub const ALL: [Class; 9] = [
        Class::Same,
        Class::Split171,
        Class::Split182,
        Class::E2,
        Class::E1,
        Class::E2Pending,
        Class::Recorded,
        Class::BugAgree,
        Class::BugSplit,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Class::Same => "same",
            Class::Split171 => "split=1.7.1",
            Class::Split182 => "split=1.8.2",
            Class::E2 => "E2",
            Class::E1 => "E1",
            Class::E2Pending => "E2-pending",
            Class::Recorded => "recorded",
            Class::BugAgree => "BUG(agree)",
            Class::BugSplit => "BUG(split)",
        }
    }

    pub fn is_bug(self) -> bool {
        matches!(self, Class::BugAgree | Class::BugSplit)
    }
}

/// How succinctly's transcript differs from a release's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// Standard output is a strict line prefix of the release's: values lost.
    ValueLost,
    /// The release's standard output is a strict line prefix of succinctly's: values added.
    ExtraValue,
    /// Standard output differs otherwise.
    DifferentValue,
    /// Same standard output; standard error differs (a message or its position).
    Stderr,
    /// Only the exit status differs.
    Status,
    /// No difference.
    None,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::ValueLost => "value lost",
            Kind::ExtraValue => "extra value",
            Kind::DifferentValue => "different value",
            Kind::Stderr => "stderr only",
            Kind::Status => "status only",
            Kind::None => "none",
        }
    }
}

/// How `s` differs from `r`.
pub fn kind(s: &Transcript, r: &Transcript) -> Kind {
    if s.stdout != r.stdout {
        let (sl, rl) = (lines(&s.stdout), lines(&r.stdout));
        if sl.len() < rl.len() && rl.starts_with(&sl) {
            Kind::ValueLost
        } else if rl.len() < sl.len() && sl.starts_with(&rl) {
            Kind::ExtraValue
        } else {
            Kind::DifferentValue
        }
    } else if s.stderr != r.stderr {
        Kind::Stderr
    } else if s.status != r.status {
        Kind::Status
    } else {
        Kind::None
    }
}

fn lines(b: &[u8]) -> Vec<&[u8]> {
    b.split(|&c| c == b'\n').filter(|l| !l.is_empty()).collect()
}

/// One jq parse-error message (spike E's `FAULT`, with jq's whole message list). The Rust regex has no look-ahead, so a match
/// may end on the `"` or `\n` after the message; [`first_fault`] trims and checks those.
fn fault_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            // Every message of jv_parse.c in both releases (spike E's list lacked a dozen, such
            // as `Expected another array element`), and succinctly's `Invalid JSON text`.
            r"(?-u)(?:Invalid |Unfinished |Expected |Unmatched '|Objects must|Object keys must",
            r"|Potentially truncated|Truncated value|Malformed BOM|Missing value in key:value pair",
            r"|Exceeds depth limit|Token too long|',' not as part|':' not as part|':' should follow|Comma not)",
            r#"[^"\n]*?(?:line \d+, column \d+(?: \(need RS to resync\))?|BOM|"|\n|\z)"#,
        ))
        .expect("a valid regex")
    })
}

/// The first parse-error message in `text`: (start, end) of the message, as Python's
/// `FAULT.search` finds it (its `(?=")` and `$` alternatives end the message before a `"`, and at
/// the end of the text or before a final `\n`).
pub fn first_fault(text: &[u8]) -> Option<(usize, usize)> {
    let re = fault_regex();
    let mut from = 0;
    while let Some(m) = re.find_at(text, from) {
        let end = m.end();
        match text[..end].last() {
            Some(b'"') => return Some((m.start(), end - 1)),
            Some(b'\n') if end == text.len() => return Some((m.start(), end - 1)),
            Some(b'\n') => from = m.start() + 1,
            _ => return Some((m.start(), end)),
        }
    }
    None
}

/// The text up to the end of the first fault message, or `None` when there is none.
fn head(text: &[u8]) -> Option<&[u8]> {
    first_fault(text).map(|(_, end)| &text[..end])
}

/// Python's `str.splitlines` over the latin-1 view, compared on the common prefix.
fn lines_prefix(a: &[u8], b: &[u8]) -> bool {
    let split = |t: &[u8]| -> Vec<Vec<u8>> {
        let mut v = Vec::new();
        let mut cur = Vec::new();
        let mut i = 0;
        while i < t.len() {
            match t[i] {
                b'\n' | b'\r' | 0x0b | 0x0c | 0x1c | 0x1d | 0x1e | 0x85 => {
                    if t[i] == b'\r' && t.get(i + 1) == Some(&b'\n') {
                        i += 1;
                    }
                    v.push(std::mem::take(&mut cur));
                }
                c => cur.push(c),
            }
            i += 1;
        }
        if !cur.is_empty() {
            v.push(cur);
        }
        v
    };
    let (la, lb) = (split(a), split(b));
    let n = la.len().min(lb.len());
    la[..n] == lb[..n]
}

fn terminating(stderr: &[u8]) -> bool {
    stderr.starts_with(b"jq: parse error") || stderr.windows(16).any(|w| w == b"\njq: parse error")
}

/// E1 against one release `r`, which must have a fault. `k` is how many items `r`'s reader gave
/// before its first error ([`Facts::before_fault`]); `r171` is the pinned release.
///
/// Through `r`'s first fault, per channel, succinctly equals `r`:
/// - a channel holding the fault (stderr for `jq -c .`, stdout for the `-n` trace) must have the
///   same text up to the end of its first fault message;
/// - standard output with no fault on either side must be `r`'s exactly when that fault ended
///   `r`'s run (`jq: parse error`), and otherwise start with the `k` values `r` printed before
///   the fault (`--seq`, where the run goes on);
/// - standard error with no fault on either side only needs one to be a line prefix of the other.
///
/// The exit status is `r`'s exactly when its fault ended the run, and in every case zero exactly
/// when jq 1.7.1's is (ADR-0018's #7963 amendment). Spike E's form (`lenient`) lets standard
/// output be any line prefix, which accepts values lost before the fault.
pub fn e1_clean(
    s: &Transcript,
    r: &Transcript,
    k: Option<usize>,
    r171: &Transcript,
    lenient: bool,
) -> bool {
    let (hr_o, hr_e) = (head(&r.stdout), head(&r.stderr));
    if hr_o.is_none() && hr_e.is_none() {
        return false;
    }
    for (out, sc, rc, hr) in [
        (true, &s.stdout, &r.stdout, hr_o),
        (false, &s.stderr, &r.stderr, hr_e),
    ] {
        match (hr, head(sc)) {
            (None, None) => {
                let ok = if lenient || !out {
                    lines_prefix(sc, rc)
                } else if terminating(&r.stderr) {
                    sc == rc
                } else {
                    let (sl, rl) = (lines(sc), lines(rc));
                    let k = k.unwrap_or(rl.len()).min(rl.len());
                    sl.len() >= k && sl[..k] == rl[..k]
                };
                if !ok {
                    return false;
                }
            }
            (a, b) if a == b => {}
            _ => return false,
        }
    }
    (!terminating(&r.stderr) || s.status == r.status) && (s.status == 0) == (r171.status == 0)
}

/// A jq error string the trace program can see as an item's payload.
fn is_err_value(v: &Value) -> bool {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(
            r"at line \d+|break|Malformed BOM|Invalid|Unfinished|Potentially|Expected|Unmatched|Truncated|Objects must|Cannot",
        )
        .expect("a valid regex")
    });
    matches!(v, Value::String(s) if re.is_match(s))
}

fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

/// One value of a transcript's standard output: a JSON value, or a line that is not one, kept as
/// its bytes so that it equals only the same line.
#[derive(Debug, Clone, PartialEq)]
enum Val {
    Json(Value),
    Raw(Vec<u8>),
}

/// The values a transcript's standard output carries, in order: each line of `jq -c .`, or the
/// payload of each `["V", v, ...]` line of the trace program. A line that does not parse, or a
/// trace line that is neither `V` nor `E`, is a [`Val::Raw`]. With `until_fault`, it stops at the
/// trace's first `E` line (outside the trace, standard output holds no faults).
fn values(stdout: &[u8], shape: Shape, until_fault: bool) -> Vec<Val> {
    let mut out = Vec::new();
    let mut faulted = false;
    for line in stdout.split(|&c| c == b'\n') {
        let line: Vec<u8> = line.iter().copied().filter(|&c| c != 0x1e).collect();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&latin1(&line)) else {
            out.push(Val::Raw(line));
            continue;
        };
        let v = if shape.trace {
            match v {
                // After a fault under `-n -s`, both releases' next `input` slurps what is left,
                // often nothing: that `[]` is jq's own answer, not a value of the input.
                Value::Array(a)
                    if a.len() >= 2
                        && a[0] == "V"
                        && faulted
                        && a[1] == Value::Array(Vec::new()) =>
                {
                    continue
                }
                Value::Array(mut a) if a.len() >= 2 && a[0] == "V" => {
                    let v = a.swap_remove(1);
                    if is_err_value(&v) {
                        continue;
                    }
                    v
                }
                Value::Array(a) if !a.is_empty() && a[0] == "E" => {
                    if until_fault {
                        break;
                    }
                    faulted = true;
                    continue;
                }
                _ => {
                    out.push(Val::Raw(line));
                    continue;
                }
            }
        } else {
            v
        };
        match v {
            // A slurped array is compared by its members, so one cut short by a fault is a
            // prefix, not another value.
            Value::Array(members) if shape.slurp => out.extend(members.into_iter().map(Val::Json)),
            v => out.push(Val::Json(v)),
        }
    }
    out
}

/// A JSON number as sign, significant digits (no leading or trailing zeros) and exponent, so that
/// `1`, `1.0` and `10e-1` are one number and `1.000000000000000000001` is not `1`. `None` when
/// the text is not a JSON number or its exponent does not fit.
fn decimal(text: &str) -> Option<(bool, String, i64)> {
    let (neg, t) = match text.strip_prefix('-') {
        Some(t) => (true, t),
        None => (false, text),
    };
    let (mant, exp) = match t.find(['e', 'E']) {
        Some(i) => (&t[..i], t[i + 1..].parse::<i64>().ok()?),
        None => (t, 0),
    };
    let (int, frac) = mant.split_once('.').unwrap_or((mant, ""));
    if int.is_empty() || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let digits = format!("{int}{frac}");
    let lead = digits.len() - digits.trim_start_matches('0').len();
    let digits = digits.trim_start_matches('0');
    let trimmed = digits.trim_end_matches('0');
    if trimmed.is_empty() {
        return Some((false, String::new(), 0));
    }
    // The value is 0.<digits> x 10^(exp + len(int) - lead): normalise to that exponent.
    let point = i64::try_from(int.len()).ok()? - i64::try_from(lead).ok()?;
    Some((neg, trimmed.to_string(), exp.checked_add(point)?))
}

/// JSON equality with numbers compared as decimals (`1` == `1.0`, `1E1000` == `1E+1000`), never
/// through `f64`.
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.to_string(), y.to_string());
            x == y || decimal(&x).is_some_and(|d| Some(d) == decimal(&y))
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same_value(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y)
                    .all(|((k1, v1), (k2, v2))| k1 == k2 && same_value(v1, v2))
        }
        _ => a == b,
    }
}

fn same_val(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::Json(x), Val::Json(y)) => same_value(x, y),
        (Val::Raw(x), Val::Raw(y)) => x == y,
        _ => false,
    }
}

fn subsequence(p: &[Val], q: &[Val]) -> bool {
    let mut it = q.iter();
    p.iter().all(|e| it.by_ref().any(|f| same_val(e, f)))
}

fn starts_with(p: &[Val], q: &[Val]) -> bool {
    p.len() >= q.len() && p.iter().zip(q).all(|(a, b)| same_val(a, b))
}

/// Succinctly prints a value `r` does not: its values are not a subsequence of `r`'s.
fn wrong_value_vs(s: &Transcript, r: &Transcript, shape: Shape) -> bool {
    !subsequence(
        &values(&s.stdout, shape, false),
        &values(&r.stdout, shape, false),
    )
}

/// The floor: succinctly prints a value neither release prints (not merely a missing one). A
/// line that is not JSON is a value too, so it counts unless a release prints that same line.
///
/// This approximates the floor's first two bullets (resume at the next line or record; every
/// value lies wholly after the fault): a value assembled across the fault, or read from a
/// resume point jq does not use, is caught only when no release prints an equal value.
pub fn wrong_value(s: &Transcript, r171: &Transcript, r182: &Transcript, shape: Shape) -> bool {
    wrong_value_vs(s, r171, shape) && wrong_value_vs(s, r182, shape)
}

/// The first NUL that jq's tokenizer would see outside a string, with the files read as one
/// stream (jq's parser never learns where a file ends): its index, and its line and column as
/// jq's parser counts them (a line's first byte is column 1; a leading BOM is not counted).
/// String state only, no reset at faults.
fn nul_outside(stream: &[u8]) -> Option<(usize, (u64, u64))> {
    let mut st = 0; // 0 normal, 1 string, 2 escape
    let mut at = None;
    for (i, &b) in stream.iter().enumerate() {
        match st {
            0 if b == b'"' => st = 1,
            0 if b == 0 => {
                at = Some(i);
                break;
            }
            1 if b == b'\\' => st = 2,
            1 if b == b'"' => st = 0,
            2 => st = 1,
            _ => {}
        }
    }
    let i = at?;
    let mut pre = &stream[..i];
    if pre.starts_with(b"\xef\xbb\xbf") {
        pre = &pre[3..];
    }
    let line = pre.iter().filter(|&&b| b == b'\n').count() as u64 + 1;
    let col = match pre.iter().rposition(|&b| b == b'\n') {
        Some(nl) => pre.len() - nl - 1,
        None => pre.len(),
    } as u64;
    Some((i, (line, col + 1)))
}

/// E2's trigger: a NUL that jq's tokenizer would see outside a string (`nul_outside`).
pub fn nul_outside_string(stream: &[u8]) -> bool {
    nul_outside(stream).is_some()
}

/// A byte jq's tokenizer adds to a literal (a number, `true`, ...): anything but whitespace, a
/// structural character, a quote, an RS or a NUL.
fn literal_byte(b: u8) -> bool {
    !matches!(
        b,
        b' ' | b'\t' | b'\r' | b'\n' | b'[' | b']' | b'{' | b'}' | b',' | b':' | b'"' | 0x1e | 0
    )
}

/// What jq 1.7.1's reader gives from the bytes wholly before the first NUL outside a string, up to
/// its first parse error, as `jq -c` prints each value. A literal the NUL ends is not one of them:
/// E2 makes the NUL a parse error, so there is no NUL-ended number. `None` without such a NUL.
pub fn values_before_nul(stream: &[u8], seq: bool) -> crate::reader::Result<Option<Vec<Vec<u8>>>> {
    use crate::reader::{Item, Opts, PortReader, Reader, Release, Source, PARSE_SEQ};
    let Some((i, _)) = nul_outside(stream) else {
        return Ok(None);
    };
    let mut end = i;
    while end > 0 && literal_byte(stream[end - 1]) {
        end -= 1;
    }
    let opts = Opts {
        flags: if seq { PARSE_SEQ } else { 0 },
        raw: false,
        slurp: false,
    };
    let source = Source {
        name: "before-nul".into(),
        data: Some(stream[..end].to_vec()),
    };
    let mut r = PortReader::new(Release::V171, opts, vec![source])?;
    let mut out = Vec::new();
    while let Some(Item::Value(v)) = r.next()? {
        out.push(v);
    }
    Ok(Some(out))
}

fn position(msg: &[u8]) -> Option<(u64, u64)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"line (\d+), column (\d+)").expect("a valid regex"));
    let c = re.captures(msg)?;
    let n = |i: usize| std::str::from_utf8(&c[i]).ok()?.parse().ok();
    Some((n(1)?, n(2)?))
}

/// Does `r`'s first fault come strictly before the NUL at `npos` (jq's line and column)? A
/// `Malformed BOM` always does; a fault `at EOF` never does.
fn fault_before_nul(r: &Transcript, npos: (u64, u64)) -> bool {
    for t in [&r.stdout, &r.stderr] {
        let Some((a, b)) = first_fault(t) else {
            continue;
        };
        let msg = &t[a..b];
        if msg.windows(13).any(|w| w == b"Malformed BOM") {
            return true;
        }
        if msg.windows(7).any(|w| w == b" at EOF") {
            continue;
        }
        if position(msg).is_some_and(|p| p < npos) {
            return true;
        }
    }
    false
}

/// The first fault message of a run: in the trace's standard output under `-n`, else on standard
/// error.
fn fault_msg(t: &Transcript, shape: Shape) -> Option<&[u8]> {
    let text = if shape.trace { &t.stdout } else { &t.stderr };
    first_fault(text).map(|(a, b)| &text[a..b])
}

/// The values succinctly owes before a NUL that is the first fault: jq 1.7.1's from the bytes
/// before it, none at all when a slurp ends there (`-s` prints nothing at a fault, `-n -s` reports
/// it on the first `input`). `prefix` accepts more values after them: under `--seq` the run goes
/// on past the fault, and E2-pending may read past the NUL as a release does.
fn kept_before_nul(s: &Transcript, shape: Shape, facts: &Facts, prefix: bool) -> bool {
    let Some(pre) = &facts.before_nul else {
        return false;
    };
    let pre: Vec<Val> = pre
        .iter()
        .flat_map(|v| {
            let one = Shape {
                trace: false,
                slurp: false,
                ..shape
            };
            values(v, one, false)
        })
        .collect();
    let runs_on = shape.seq && !shape.trace;
    let owed = if shape.slurp && !runs_on {
        Vec::new()
    } else {
        pre.clone()
    };
    let got = values(&s.stdout, shape, true);
    if prefix || runs_on {
        starts_with(&got, &owed) || (prefix && starts_with(&got, &pre))
    } else {
        got.len() == owed.len() && starts_with(&got, &owed)
    }
}

/// E2 for this case. Where a release's first fault comes before the NUL, that fault is the first
/// fault and succinctly must meet E1 against that release. Otherwise the NUL is the first fault:
/// succinctly reports a fault at the NUL's own line and column, prints exactly the values before
/// it (`kept_before_nul`), no value neither release prints, and exits non-zero when that fault
/// ends the run (`jq -c .` without `--seq`), else zero exactly when jq 1.7.1 does.
pub fn e2_consistent(
    stream: &[u8],
    s: &Transcript,
    r171: &Transcript,
    r182: &Transcript,
    shape: Shape,
    facts: &Facts,
) -> bool {
    let Some((_, npos)) = nul_outside(stream) else {
        return false;
    };
    if wrong_value(s, r171, r182, shape) {
        return false;
    }
    let early: Vec<(&Transcript, Option<usize>)> = [r171, r182]
        .into_iter()
        .zip(facts.before_fault)
        .filter(|(r, _)| fault_before_nul(r, npos))
        .collect();
    if !early.is_empty() {
        return early
            .into_iter()
            .all(|(r, k)| e1_clean(s, r, k, r171, shape.lenient_e1));
    }
    let at_nul = fault_msg(s, shape).and_then(position) == Some(npos);
    let ends_run = !shape.trace && !shape.seq;
    let status = if ends_run {
        s.status != 0
    } else {
        (s.status == 0) == (r171.status == 0)
    };
    at_nul && status && kept_before_nul(s, shape, facts, false)
}

/// E2's ground that succinctly does not implement yet (#7966): the NUL is the first fault (no
/// release faults before it), succinctly reports no fault before it, keeps the values before it
/// (reading past it as a release may), prints no value neither release prints, and exits zero
/// exactly when one release does.
fn e2_pending(
    stream: &[u8],
    s: &Transcript,
    r171: &Transcript,
    r182: &Transcript,
    shape: Shape,
    facts: &Facts,
) -> bool {
    let Some((_, npos)) = nul_outside(stream) else {
        return false;
    };
    if [r171, r182].iter().any(|r| fault_before_nul(r, npos)) {
        return false;
    }
    if fault_msg(s, shape)
        .and_then(position)
        .is_some_and(|p| p < npos)
    {
        return false;
    }
    !wrong_value(s, r171, r182, shape)
        && kept_before_nul(s, shape, facts, true)
        && [r171, r182]
            .iter()
            .any(|r| (s.status == 0) == (r.status == 0))
}

/// The full verdict: equal to a release first, then E2, then E1 (against 1.7.1, then 1.8.2),
/// then a bug. `shape` says how to read the values out of standard output; `facts` is what the
/// oracle adds.
pub fn classify(
    stream: &[u8],
    s: &Transcript,
    r171: &Transcript,
    r182: &Transcript,
    shape: Shape,
    facts: &Facts,
) -> Class {
    let agree = r171 == r182;
    if s == r171 {
        return if agree { Class::Same } else { Class::Split171 };
    }
    if e2_consistent(stream, s, r171, r182, shape, facts) {
        return Class::E2;
    }
    let floor = !wrong_value(s, r171, r182, shape);
    let [k171, k182] = facts.before_fault;
    let lenient = shape.lenient_e1;
    if s == r182 {
        // Equal to 1.8.2 only. Against the pinned 1.7.1, a difference after the first fault is
        // E1's; any other is a deviation from the pin (a recorded adoption, or not).
        return if floor && e1_clean(s, r171, k171, r171, lenient) {
            Class::E1
        } else {
            Class::Split182
        };
    }
    if floor && (e1_clean(s, r171, k171, r171, lenient) || e1_clean(s, r182, k182, r171, lenient)) {
        return Class::E1;
    }
    if e2_pending(stream, s, r171, r182, shape, facts) {
        return Class::E2Pending;
    }
    if recorded(stream, s, r171, r182, shape, facts).is_some() {
        return Class::Recorded;
    }
    if agree {
        Class::BugAgree
    } else {
        Class::BugSplit
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn count(hay: &[u8], needle: &[u8]) -> usize {
    hay.windows(needle.len()).filter(|w| *w == needle).count()
}

/// Where the strings of `text` hold an escape jq's parser rejects: `\x`, `\q`, a backslash
/// before a raw control character or at the end, a `\u` without four hex digits, and a high
/// surrogate `\uD800`-`\uDBFF` not followed by a low one (a lone low surrogate jq accepts, as
/// U+FFFD). Each is the index of the backslash and the length to overwrite with `\\` to make the
/// string valid without moving a byte.
fn bad_escapes(text: &[u8]) -> Vec<(usize, usize)> {
    let hex4 = |i: usize| -> Option<u32> {
        let h = text.get(i..i + 4)?;
        if !h.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        u32::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()
    };
    let mut out = Vec::new();
    let mut in_string = false;
    let mut i = 0;
    while i < text.len() {
        match (in_string, text[i]) {
            (false, b'"') => in_string = true,
            (true, b'"') => in_string = false,
            (true, b'\\') => {
                match text.get(i + 1) {
                    Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => {}
                    Some(b'u') => match hex4(i + 2) {
                        None => out.push((i, 2)),
                        Some(0xd800..=0xdbff) => {
                            let low = text.get(i + 6..i + 8) == Some(b"\\u")
                                && matches!(hex4(i + 8), Some(0xdc00..=0xdfff));
                            if low {
                                i += 6;
                            } else {
                                out.push((i, 2));
                            }
                            i += 4;
                        }
                        Some(_) => i += 4,
                    },
                    Some(_) => out.push((i, 2)),
                    None => out.push((i, 1)),
                }
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// Does `text` hold an escape jq's parser rejects (`bad_escapes`)?
pub fn bad_escape(text: &[u8]) -> bool {
    !bad_escapes(text).is_empty()
}

/// `stream` with every escape jq rejects overwritten with `\\` (`bad_escapes`): the same length
/// and line breaks except where a backslash ended a line, and every string valid. Run through jq,
/// it shows what jq prints where succinctly forwards such a string (#5308).
pub fn escapes_made_valid(stream: &[u8]) -> Vec<u8> {
    let mut out = stream.to_vec();
    for (i, n) in bad_escapes(stream) {
        for b in &mut out[i..i + n] {
            *b = b'\\';
        }
    }
    out
}

/// Standard output without the lines that forward a string jq's parser rejects.
fn without_echoes(stdout: &[u8]) -> Vec<u8> {
    stdout
        .split_inclusive(|&c| c == b'\n')
        .filter(|l| !bad_escape(l))
        .flatten()
        .copied()
        .collect()
}

fn escape_fault(msg: &[u8]) -> bool {
    contains(msg, b"Invalid escape")
        || contains(msg, b"Invalid \\uXXXX escape")
        || contains(msg, b"surrogate pair escape")
}

/// #5308's echo against release `r`, whose first fault must be the escape. With the lines that
/// forward a rejected string taken out of succinctly's standard output:
/// - the head through `r`'s fault is exact: the output `r` printed before it (all of it when the
///   fault ended `r`'s run, else its first `k` values), and its standard error before it;
/// - no value is one neither release prints once the escapes are valid ([`Facts::escapes_valid`],
///   or the releases themselves when it is not known);
/// - the exit status is zero exactly when jq 1.7.1's is on that valid stream: the documented echo
///   exits 0 where jq rejects the document.
fn escape_echo(
    s: &Transcript,
    r: &Transcript,
    k: Option<usize>,
    valid: (&Transcript, &Transcript),
    shape: Shape,
) -> bool {
    let Some(msg) = fault_msg(r, shape) else {
        return false;
    };
    if !escape_fault(msg) {
        return false;
    }
    let s2 = Transcript {
        stdout: without_echoes(&s.stdout),
        stderr: s.stderr.clone(),
        status: s.status,
    };
    if wrong_value(&s2, valid.0, valid.1, shape) || (s.status == 0) != (valid.0.status == 0) {
        return false;
    }
    let (sl, rl) = (lines(&s2.stdout), lines(&r.stdout));
    let owed = if shape.trace {
        rl.iter()
            .position(|l| first_fault(l).is_some())
            .unwrap_or(rl.len())
    } else if terminating(&r.stderr) {
        rl.len()
    } else {
        k.unwrap_or(rl.len()).min(rl.len())
    };
    let err_head = match first_fault(&r.stderr) {
        Some((a, _)) => {
            let line = r.stderr[..a]
                .iter()
                .rposition(|&c| c == b'\n')
                .map_or(0, |i| i + 1);
            s.stderr.starts_with(&r.stderr[..line])
        }
        None => lines_prefix(&s.stderr, &r.stderr),
    };
    sl.len() >= owed && sl[..owed] == rl[..owed] && err_head
}

/// The recorded divergence a difference falls under, if any. Each is a deliberate, documented
/// difference of the whole jq front end, not of the reader's edges:
/// - `#5804`: jq words a text that opens with part of a BOM `Malformed BOM`; the primary input
///   here words it as the token it reads (limitations.md, "Malformed BOM"). E6 in spike E.
/// - `#5308`: a string with an invalid escape (`"\x"`, `"\u12"`, a backslash before a raw control
///   character, a lone high surrogate) is echoed by a filter that only forwards it,
///   where jq's parser rejects it (limitations.md, #5308); see `escape_echo`.
/// - `#2103`: a fault inside a value is found when something reads it, and worded `Invalid JSON
///   text: ...` as a runtime error, where jq's parser reports it (ADR-0018's #2103 amendment,
///   limitations.md "A fault found by walking to it leaves the prefix on stdout").
///
/// The floor holds for all three: a value neither release prints is a bug.
pub fn recorded(
    stream: &[u8],
    s: &Transcript,
    r171: &Transcript,
    r182: &Transcript,
    shape: Shape,
    facts: &Facts,
) -> Option<&'static str> {
    let either = |f: &dyn Fn(&Transcript) -> bool| f(r171) || f(r182);
    let floor = !wrong_value(s, r171, r182, shape);
    let status_like = |r: &Transcript| (s.status == 0) == (r.status == 0);
    let boms =
        |t: &Transcript| count(&t.stdout, b"Malformed BOM") + count(&t.stderr, b"Malformed BOM");
    // Only the first read of the fault is worded differently: jq's parser stays in it, and every
    // later `input` says `Malformed BOM` here too.
    if floor && either(&|r| boms(r) > boms(s) && status_like(r)) {
        return Some("#5804 BOM wording");
    }
    if bad_escape(stream) {
        let valid = match &facts.escapes_valid {
            Some((a, b)) => (a, b),
            None => (r171, r182),
        };
        let [k171, k182] = facts.before_fault;
        if escape_echo(s, r171, k171, valid, shape) || escape_echo(s, r182, k182, valid, shape) {
            return Some("#5308 invalid escape forwarded");
        }
    }
    if floor
        && contains(&s.stderr, b"Invalid JSON text")
        && either(&|r| contains(&r.stderr, b"jq: parse error") && r.status == s.status)
    {
        return Some("#2103 fault found by the walk");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(o: &str, e: &str, rc: i32) -> Transcript {
        Transcript {
            stdout: o.as_bytes().to_vec(),
            stderr: e.as_bytes().to_vec(),
            status: rc,
        }
    }

    #[test]
    fn first_fault_ends_at_the_position_a_quote_or_the_end() {
        let e = b"jq: parse error: Invalid numeric literal at line 1, column 5\n";
        let (a, b) = first_fault(e).expect("a fault");
        assert_eq!(&e[a..b], b"Invalid numeric literal at line 1, column 5");
        let o = br#"["E","Unfinished string at EOF at line 2, column 0","f",1]"#;
        let (a, b) = first_fault(o).expect("a fault");
        assert_eq!(&o[a..b], b"Unfinished string at EOF at line 2, column 0");
        // As Python's `$`, a message that runs into a newline before the end does not match.
        assert_eq!(first_fault(b"Malformed BOM (while parsing)\nmore"), None);
        let m = b"x Malformed BOM (while parsing)\n";
        let (a, b) = first_fault(m).expect("a fault");
        assert_eq!(&m[a..b], b"Malformed BOM (while parsing)");
        assert_eq!(first_fault(b"1\n2\n"), None);
    }

    #[test]
    fn e1_accepts_a_difference_after_the_fault_only() {
        let r = t(
            "1\n",
            "jq: ignoring parse error: Invalid literal at line 2, column 3\n",
            0,
        );
        // Lost a value after the fault: accepted.
        let s = t(
            "",
            "jq: ignoring parse error: Invalid literal at line 2, column 3\n",
            0,
        );
        assert!(e1_clean(&s, &r, Some(0), &r, false));
        // ... but not when the reader gave it before the fault.
        assert!(!e1_clean(&s, &r, Some(1), &r, false));
        // A different fault position: not accepted.
        let s = t(
            "1\n",
            "jq: ignoring parse error: Invalid literal at line 2, column 4\n",
            0,
        );
        assert!(!e1_clean(&s, &r, Some(0), &r, false));
        // No fault in the release: E1 does not apply.
        let r = t("1\n", "", 0);
        assert!(!e1_clean(&t("", "", 0), &r, None, &r, false));
    }

    #[test]
    fn a_value_neither_release_prints_is_a_bug() {
        let r = t(
            "1\n",
            "jq: parse error: Invalid literal at line 1, column 3\n",
            5,
        );
        let s = t(
            "1\n2\n",
            "jq: parse error: Invalid literal at line 1, column 3\n",
            5,
        );
        assert_eq!(
            classify(b"1 x", &s, &r, &r, Shape::default(), &Facts::default()),
            Class::BugAgree
        );
        let s = t(
            "",
            "jq: parse error: Invalid literal at line 1, column 3\n",
            5,
        );
        // A value lost before the fault that ended jq's run is not E1's: jq printed it first.
        assert_eq!(
            classify(b"1 x", &s, &r, &r, Shape::default(), &Facts::default()),
            Class::BugAgree
        );
        let lenient = Shape {
            lenient_e1: true,
            ..Shape::default()
        };
        assert_eq!(
            classify(b"1 x", &s, &r, &r, lenient, &Facts::default()),
            Class::E1
        );
        // Under `--seq` the run goes on past a fault, so standard output is only a line prefix.
        let r = t(
            "1\n2\n",
            "jq: ignoring parse error: Invalid literal at line 1, column 3\n",
            0,
        );
        let s = t(
            "1\n",
            "jq: ignoring parse error: Invalid literal at line 1, column 3\n",
            0,
        );
        assert_eq!(
            classify(b"1 x 2", &s, &r, &r, Shape::default(), &k(1)),
            Class::E1
        );
    }

    #[test]
    fn e2_needs_a_nul_outside_a_string_and_a_fault() {
        assert!(nul_outside_string(b"1\x00"));
        assert!(!nul_outside_string(b"\"a\x00\""));
        assert!(nul_outside_string(b"\"a\\\"\" \x00"));
        let r = t("1\n", "", 0);
        let s = t(
            "",
            "jq: parse error: Invalid numeric literal at line 1, column 2\n",
            5,
        );
        // The `1` the NUL ends is not a value of E2's (no NUL-ended number).
        let before_nul = values_before_nul(b"1\x00", false).expect("the 1.7.1 reader");
        assert_eq!(before_nul, Some(Vec::new()));
        let facts = Facts {
            before_nul,
            ..Facts::default()
        };
        assert_eq!(
            classify(b"1\x00", &s, &r, &r, Shape::default(), &facts),
            Class::E2
        );
        assert_eq!(
            values_before_nul(b"1 [2]\n\"a\x00\" 3", false).expect("the 1.7.1 reader"),
            None,
            "a NUL inside a string is not E2's"
        );
        assert_eq!(
            values_before_nul(b"1 [2]\n\"a\" 3 \x00", false).expect("the 1.7.1 reader"),
            Some(vec![
                b"1".to_vec(),
                b"[2]".to_vec(),
                b"\"a\"".to_vec(),
                b"3".to_vec()
            ])
        );
        assert_eq!(
            classify(
                b"\"1\x00\"",
                &s,
                &r,
                &r,
                Shape::default(),
                &Facts::default()
            ),
            Class::BugAgree
        );
    }

    #[test]
    fn splits_follow_the_pin() {
        let a = t("1\n", "", 0);
        let b = t("2\n", "", 0);
        assert_eq!(
            classify(b"", &a, &a, &b, Shape::default(), &Facts::default()),
            Class::Split171
        );
        assert_eq!(
            classify(b"", &b, &a, &b, Shape::default(), &Facts::default()),
            Class::Split182
        );
        assert_eq!(
            classify(
                b"",
                &t("3\n", "", 0),
                &a,
                &b,
                Shape::default(),
                &Facts::default()
            ),
            Class::BugSplit
        );
        assert_eq!(
            classify(b"", &a, &a, &a, Shape::default(), &Facts::default()),
            Class::Same
        );
    }

    #[test]
    fn trace_values_are_the_payloads() {
        let o = "[\"V\",1,\"f\",1]\n[\"E\",\"Invalid literal at line 1, column 2\",\"f\",1]\n[\"V\",[2],\"f\",2]\n";
        let v = values(
            o.as_bytes(),
            Shape {
                trace: true,
                ..Shape::default()
            },
            false,
        );
        assert_eq!(
            v,
            vec![
                Val::Json(serde_json::json!(1)),
                Val::Json(serde_json::json!([2]))
            ]
        );
    }

    /// `before_fault` for releases that both gave `n` items before their first error.
    fn k(n: usize) -> Facts {
        Facts {
            before_fault: [Some(n), Some(n)],
            ..Facts::default()
        }
    }

    fn tb(o: &[u8], e: &[u8], rc: i32) -> Transcript {
        Transcript {
            stdout: o.to_vec(),
            stderr: e.to_vec(),
            status: rc,
        }
    }

    // Review finding 1: the #5308 branch caught any difference on a stream with a bad escape.
    #[test]
    fn a_recorded_escape_echo_keeps_the_head_the_floor_and_the_status() {
        let d = Shape::default();
        // The reviewer's probe: a different value, message and status is not the echo.
        let r = tb(
            b"1\n2\n",
            b"jq: ignoring parse error: Invalid escape at line 3, column 4\n",
            0,
        );
        let s = tb(b"7\n", b"wrong\n", 3);
        assert_eq!(
            classify(b"1\n2\n\"\\x\"\n", &s, &r, &r, d, &k(2)),
            Class::BugAgree
        );
        // The recorded echo: the value before the escape, then the string as it was written,
        // exit 0, as jq exits on the stream with the escape made valid.
        let stream = b"1\n\"\\x\"\n";
        let r = tb(
            b"1\n",
            b"jq: parse error: Invalid escape at line 2, column 4\n",
            5,
        );
        let valid = tb(b"1\n\"\\\\\"\n", b"", 0);
        let facts = Facts {
            escapes_valid: Some((valid.clone(), valid)),
            ..k(1)
        };
        let s = tb(b"1\n\"\\x\"\n", b"", 0);
        assert_eq!(classify(stream, &s, &r, &r, d, &facts), Class::Recorded);
        // ... but not with a value lost before it, a value no release prints, or a failing status.
        let s = tb(b"\"\\x\"\n", b"", 0);
        assert_eq!(classify(stream, &s, &r, &r, d, &facts), Class::BugAgree);
        let s = tb(b"1\n\"\\x\"\n9\n", b"", 0);
        assert_eq!(classify(stream, &s, &r, &r, d, &facts), Class::BugAgree);
        let s = tb(b"1\n\"\\x\"\n", b"", 5);
        assert_eq!(classify(stream, &s, &r, &r, d, &facts), Class::BugAgree);
    }

    // Review finding 2: E2 accepted lost values and a fault away from the NUL.
    #[test]
    fn e2_needs_the_fault_at_the_nul_and_every_value_before_it() {
        let d = Shape::default();
        let stream = b"1\n2\n\x00";
        let r = tb(b"1\n2\n", b"", 0);
        let facts = Facts {
            before_nul: Some(vec![b"1".to_vec(), b"2".to_vec()]),
            ..Facts::default()
        };
        // The reviewer's probe: no values, a fault at line 1, column 1.
        let s = tb(
            b"",
            b"jq: parse error: Invalid numeric literal at line 1, column 1\n",
            5,
        );
        assert_eq!(classify(stream, &s, &r, &r, d, &facts), Class::BugAgree);
        // The fault at the NUL's own line and column, but the values before it lost.
        let at_nul = b"jq: parse error: Invalid character at line 3, column 1\n";
        let s = tb(b"", at_nul, 5);
        assert_eq!(classify(stream, &s, &r, &r, d, &facts), Class::BugAgree);
        // E2 itself.
        let s = tb(b"1\n2\n", at_nul, 5);
        assert_eq!(classify(stream, &s, &r, &r, d, &facts), Class::E2);
    }

    // Review finding 3: E1 accepted values lost before a fault that does not end the run.
    #[test]
    fn e1_keeps_every_item_before_a_fault_that_does_not_end_the_run() {
        let d = Shape::default();
        // The reviewer's `--seq` probe.
        let stream = b"\x1e1\n\x1e2\n\x1ex\n\x1e3\n";
        let fault = b"jq: ignoring parse error: Invalid literal at line 3, column 2\n";
        let r = tb(b"\x1e1\n\x1e2\n\x1e3\n", fault, 0);
        let s = tb(b"\x1e1\n", fault, 0);
        assert_eq!(classify(stream, &s, &r, &r, d, &k(2)), Class::BugAgree);
        // Losing the value after the fault is E1's.
        let s = tb(b"\x1e1\n\x1e2\n", fault, 0);
        assert_eq!(classify(stream, &s, &r, &r, d, &k(2)), Class::E1);
        // The reviewer's other probe: only 1.8.2 has the fault, one item before it.
        let r171 = tb(b"1\n", b"", 0);
        let fault = b"jq: ignoring parse error: Unfinished string at EOF at line 2, column 0\n";
        let r182 = tb(b"1\n", fault, 0);
        let s = tb(b"", fault, 0);
        let facts = Facts {
            before_fault: [None, Some(1)],
            ..Facts::default()
        };
        assert_eq!(
            classify(b"1\n\"a", &s, &r171, &r182, d, &facts),
            Class::BugSplit
        );
    }

    // Review finding 4: a line that is not JSON switched the floor off.
    #[test]
    fn a_line_that_is_not_json_is_a_wrong_value() {
        let fault = b"jq: ignoring parse error: Invalid literal at line 2, column 2\n";
        let r = tb(b"\x1e1\n", fault, 0);
        let s = tb(b"\x1e1\n\x1e[1,\n", fault, 0);
        assert_eq!(
            classify(b"\x1e1\n\x1ex\n", &s, &r, &r, Shape::default(), &k(1)),
            Class::BugAgree
        );
    }

    // Review finding 5: a string that looked like an error message escaped the floor.
    #[test]
    fn an_ordinary_string_is_a_value_even_if_it_reads_like_an_error() {
        let fault = b"jq: ignoring parse error: Invalid literal at line 2, column 2\n";
        let r = tb(b"1\n", fault, 0);
        let s = tb(b"1\n\"breakfast\"\n", fault, 0);
        assert_eq!(
            classify(b"1\nx\n", &s, &r, &r, Shape::default(), &k(1)),
            Class::BugAgree
        );
    }

    // Review finding 6: E1 and E2 compared the exit status with the release they matched, not 1.7.1.
    #[test]
    fn the_exit_status_is_zero_exactly_when_jq_1_7_1_s_is() {
        let d = Shape::default();
        // E1 through 1.8.2's fault, which ends its run; jq 1.7.1 has none and exits 0.
        let r171 = tb(b"1\n", b"", 0);
        let fault = b"jq: parse error: Unfinished string at EOF at line 2, column 0\n";
        let r182 = tb(b"1\n", fault, 5);
        let s = tb(b"1\n", &[&fault[..], b"more\n"].concat(), 5);
        let facts = Facts {
            before_fault: [None, Some(1)],
            ..Facts::default()
        };
        assert_eq!(
            classify(b"1\n\"a", &s, &r171, &r182, d, &facts),
            Class::BugSplit
        );
        // E2's E1 branch: 1.8.2's fault comes before the NUL; jq 1.7.1 exits 0.
        let fault = b"jq: parse error: Invalid literal at line 1, column 2\n";
        let r182 = tb(b"", fault, 5);
        let r171 = tb(b"", b"", 0);
        let s = tb(b"", &[&fault[..], b"more\n"].concat(), 5);
        let facts = Facts {
            before_fault: [None, Some(0)],
            before_nul: Some(Vec::new()),
            ..Facts::default()
        };
        assert_eq!(
            classify(b"x\n\x00", &s, &r171, &r182, d, &facts),
            Class::BugSplit
        );
    }

    // Review finding 7: numbers compared as f64, and `1E1000` switched the floor off.
    #[test]
    fn numbers_compare_as_decimals() {
        let n = |t: &str| serde_json::from_str::<Value>(t).expect("a number");
        assert!(!same_value(&n("1.000000000000000000001"), &n("1")));
        assert!(same_value(&n("1"), &n("1.0")));
        assert!(same_value(&n("100"), &n("1e2")));
        assert!(same_value(&n("1E1000"), &n("1E+1000")));
        assert!(same_value(&n("-0.5"), &n("-5e-1")));
        assert!(!same_value(&n("1E1000"), &n("1.7976931348623157e+308")));
        let r = tb(b"1E1000\n", b"", 0);
        assert!(wrong_value(
            &tb(b"1E1001\n", b"", 0),
            &r,
            &r,
            Shape::default()
        ));
        assert!(!wrong_value(
            &tb(b"1E+1000\n", b"", 0),
            &r,
            &r,
            Shape::default()
        ));
    }
}
