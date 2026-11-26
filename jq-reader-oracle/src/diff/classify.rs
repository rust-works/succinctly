//! Classify one difference between `succinctly jq` and the two jq releases.
//!
//! The owner's decisions (2026-11-27, ADR-0018 amendment #7963) set the classes:
//! - the reader follows **jq 1.7.1**, the pinned release, except for recorded 1.8.2 adoptions;
//! - **E1:** after the first fault, output is best effort above a floor;
//! - **E2:** a NUL outside a string is a parse error.
//!
//! The predicates are spike E's (`preds.py`), ported to run on whole transcripts:
//! - **E1** ([`e1_clean`]): through and including the first fault, per channel, succinctly
//!   equals the release; the exit status has the same zero-ness, and is exact if that fault ends
//!   the run. A channel with no fault on either side only needs one to be a line prefix of the
//!   other (its tail is past the fault). The release must have a fault.
//! - **E2** ([`e2_consistent`]): the stream holds a NUL outside a string; succinctly reports a
//!   fault; it prints no value that neither release prints; and where a release's first fault
//!   comes before the NUL, succinctly meets E1 against that release.
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
    /// Spike E's line-prefix E1 ([`e1_clean`]); off by default.
    pub lenient_e1: bool,
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
    /// Covered by E2: a NUL outside a string, reported as a fault.
    E2,
    /// Covered by E1: equal through the first fault, best effort after it.
    E1,
    /// A NUL outside a string that succinctly does not report as a fault, printing no value
    /// neither release prints: E2's ground, not implemented yet (#7966).
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

/// E1 against one release `r`. `r` must have a fault.
///
/// Spike E's form lets a channel with no fault on either side be a line prefix of the release's.
/// That accepts values *lost before* a terminating fault, when jq's main loop printed them first
/// and stopped at the fault: a loss the owner's E1 ("best effort after the first fault") does not
/// grant. So unless `lenient`, standard output must be the release's exactly when the release's
/// fault ended the run there (`jq: parse error`, no `--seq`).
pub fn e1_clean(s: &Transcript, r: &Transcript, lenient: bool) -> bool {
    let (hr_o, hr_e) = (head(&r.stdout), head(&r.stderr));
    if hr_o.is_none() && hr_e.is_none() {
        return false;
    }
    for (sc, rc, hr) in [(&s.stdout, &r.stdout, hr_o), (&s.stderr, &r.stderr, hr_e)] {
        let hs = head(sc);
        match (hr, hs) {
            (None, None) => {
                let strict = !lenient && std::ptr::eq(sc, &s.stdout) && terminating(&r.stderr);
                if (strict && sc != rc) || !lines_prefix(sc, rc) {
                    return false;
                }
            }
            (a, b) if a == b => {}
            _ => return false,
        }
    }
    if terminating(&r.stderr) {
        s.status == r.status
    } else {
        (s.status == 0) == (r.status == 0)
    }
}

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

/// The values a transcript's standard output carries, in order: each line of `jq -c .`, or the
/// payload of each `["V", v, ...]` line of the trace program. Error strings are dropped. `None`
/// when a line does not parse (then nothing can be concluded: lenient).
fn values(stdout: &[u8], shape: Shape) -> Option<Vec<Value>> {
    let mut out = Vec::new();
    let mut faulted = false;
    for line in stdout.split(|&c| c == b'\n') {
        let line: Vec<u8> = line.iter().copied().filter(|&c| c != 0x1e).collect();
        if line.is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(&latin1(&line)).ok()?;
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
                Value::Array(mut a) if a.len() >= 2 && a[0] == "V" => a.swap_remove(1),
                Value::Array(a) if !a.is_empty() && a[0] == "E" => {
                    faulted = true;
                    continue;
                }
                _ => return None,
            }
        } else {
            v
        };
        match v {
            // A slurped array is compared by its members, so one cut short by a fault is a
            // prefix, not another value.
            Value::Array(members) if shape.slurp => {
                out.extend(members.into_iter().filter(|m| !is_err_value(m)))
            }
            v if !is_err_value(&v) => out.push(v),
            _ => {}
        }
    }
    Some(out)
}

/// JSON equality with numbers compared as numbers (`1` == `1.0`), as Python's is.
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x == y || x.as_f64() == y.as_f64(),
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

fn subsequence(p: &[Value], q: &[Value]) -> bool {
    let mut it = q.iter();
    p.iter().all(|e| it.by_ref().any(|f| same_value(e, f)))
}

/// Succinctly prints a value `r` does not: its values are not a subsequence of `r`'s.
fn wrong_value_vs(s: &Transcript, r: &Transcript, shape: Shape) -> bool {
    match (values(&s.stdout, shape), values(&r.stdout, shape)) {
        (Some(sv), Some(rv)) => !subsequence(&sv, &rv),
        _ => false,
    }
}

/// The floor: succinctly prints a value neither release prints (not merely a missing one).
pub fn wrong_value(s: &Transcript, r171: &Transcript, r182: &Transcript, shape: Shape) -> bool {
    wrong_value_vs(s, r171, shape) && wrong_value_vs(s, r182, shape)
}

/// E2's trigger: a NUL that jq's tokenizer would see outside a string, with the files read as one
/// stream (jq's parser never learns where a file ends). String state only, no reset at faults.
pub fn nul_outside_string(stream: &[u8]) -> bool {
    let mut st = 0; // 0 normal, 1 string, 2 escape
    for &b in stream {
        match st {
            0 if b == b'"' => st = 1,
            0 if b == 0 => return true,
            1 if b == b'\\' => st = 2,
            1 if b == b'"' => st = 0,
            2 => st = 1,
            _ => {}
        }
    }
    false
}

/// The (line, column) of the first NUL, counted after a leading BOM.
fn first_nul_pos(stream: &[u8]) -> Option<(u64, u64)> {
    let i = stream.iter().position(|&b| b == 0)?;
    let mut pre = &stream[..i];
    if pre.starts_with(b"\xef\xbb\xbf") {
        pre = &pre[3..];
    }
    let line = pre.iter().filter(|&&b| b == b'\n').count() as u64 + 1;
    let col = match pre.iter().rposition(|&b| b == b'\n') {
        Some(nl) => pre.len() - nl - 1,
        None => pre.len(),
    } as u64;
    Some((line, col))
}

fn position(msg: &[u8]) -> Option<(u64, u64)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"line (\d+), column (\d+)").expect("a valid regex"));
    let c = re.captures(msg)?;
    let n = |i: usize| std::str::from_utf8(&c[i]).ok()?.parse().ok();
    Some((n(1)?, n(2)?))
}

/// Does `r`'s first fault come strictly before the first NUL? A `Malformed BOM` always does.
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

/// E2 for this case.
pub fn e2_consistent(
    stream: &[u8],
    s: &Transcript,
    r171: &Transcript,
    r182: &Transcript,
    shape: Shape,
) -> bool {
    if !nul_outside_string(stream) || (head(&s.stdout).is_none() && head(&s.stderr).is_none()) {
        return false;
    }
    if wrong_value(s, r171, r182, shape) {
        return false;
    }
    let Some(npos) = first_nul_pos(stream) else {
        return false;
    };
    [r171, r182]
        .into_iter()
        .all(|r| !fault_before_nul(r, npos) || e1_clean(s, r, shape.lenient_e1))
}

/// The full verdict: equal to a release first, then E2, then E1 (against 1.7.1, then 1.8.2),
/// then a bug. `shape` says how to read the values out of standard output.
pub fn classify(
    stream: &[u8],
    s: &Transcript,
    r171: &Transcript,
    r182: &Transcript,
    shape: Shape,
) -> Class {
    let agree = r171 == r182;
    if s == r171 {
        return if agree { Class::Same } else { Class::Split171 };
    }
    if e2_consistent(stream, s, r171, r182, shape) {
        return Class::E2;
    }
    let floor = !wrong_value(s, r171, r182, shape);
    if s == r182 {
        // Equal to 1.8.2 only. Against the pinned 1.7.1, a difference after the first fault is
        // E1's; any other is a deviation from the pin (a recorded adoption, or not).
        return if floor && e1_clean(s, r171, shape.lenient_e1) {
            Class::E1
        } else {
            Class::Split182
        };
    }
    if floor && (e1_clean(s, r171, shape.lenient_e1) || e1_clean(s, r182, shape.lenient_e1)) {
        return Class::E1;
    }
    if floor && nul_outside_string(stream) && head(&s.stdout).is_none() && head(&s.stderr).is_none()
    {
        return Class::E2Pending;
    }
    if recorded(stream, s, r171, r182, shape).is_some() {
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

fn mentions(t: &Transcript, needle: &[u8]) -> bool {
    contains(&t.stdout, needle) || contains(&t.stderr, needle)
}

/// A string in `stream` holds an escape JSON does not have (`\x`, `\q`, ...).
fn invalid_escape(stream: &[u8]) -> bool {
    let mut in_string = false;
    let mut i = 0;
    while i < stream.len() {
        match (in_string, stream[i]) {
            (false, b'"') => in_string = true,
            (true, b'"') => in_string = false,
            (true, b'\\') => {
                match stream.get(i + 1) {
                    Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => {}
                    // `\u` needs four hex digits; a lone high surrogate is caught by jq's
                    // `surrogate pair` wording below.
                    Some(b'u') => {
                        let hex = stream.get(i + 2..i + 6);
                        if !hex.is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit)) {
                            return true;
                        }
                    }
                    _ => return true,
                }
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// The recorded divergence a difference falls under, if any. Each is a deliberate, documented
/// difference of the whole jq front end, not of the reader's edges:
/// - `#5804`: jq words a text that opens with part of a BOM `Malformed BOM`; the primary input
///   here words it as the token it reads (limitations.md, "Malformed BOM"). E6 in spike E.
/// - `#5308`: a string with an invalid escape (`"\x"`, `"\u12"`, a backslash before a raw control
///   character, a lone high surrogate) is echoed by a filter that only forwards it,
///   where jq's parser rejects it (limitations.md, #5308).
/// - `#2103`: a fault inside a value is found when something reads it, and worded `Invalid JSON
///   text: ...` as a runtime error, where jq's parser reports it (ADR-0018's #2103 amendment,
///   limitations.md "A fault found by walking to it leaves the prefix on stdout").
///
/// The floor still holds for the first and third: a value neither release prints is a bug.
pub fn recorded(
    stream: &[u8],
    s: &Transcript,
    r171: &Transcript,
    r182: &Transcript,
    shape: Shape,
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
    // The string is echoed whether or not a later read (a fault after it) reports the escape.
    let escape_fault = |r: &Transcript| {
        mentions(r, b"Invalid escape")
            || mentions(r, b"Invalid \\uXXXX escape")
            || mentions(r, b"surrogate pair escape")
    };
    if (invalid_escape(stream) || mentions(r171, b"surrogate pair escape")) && either(&escape_fault)
    {
        return Some("#5308 invalid escape forwarded");
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
        assert!(e1_clean(&s, &r, false));
        // A different fault position: not accepted.
        let s = t(
            "1\n",
            "jq: ignoring parse error: Invalid literal at line 2, column 4\n",
            0,
        );
        assert!(!e1_clean(&s, &r, false));
        // No fault in the release: E1 does not apply.
        assert!(!e1_clean(&t("", "", 0), &t("1\n", "", 0), false));
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
            classify(b"1 x", &s, &r, &r, Shape::default()),
            Class::BugAgree
        );
        let s = t(
            "",
            "jq: parse error: Invalid literal at line 1, column 3\n",
            5,
        );
        // A value lost before the fault that ended jq's run is not E1's: jq printed it first.
        assert_eq!(
            classify(b"1 x", &s, &r, &r, Shape::default()),
            Class::BugAgree
        );
        let lenient = Shape {
            lenient_e1: true,
            ..Shape::default()
        };
        assert_eq!(classify(b"1 x", &s, &r, &r, lenient), Class::E1);
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
        assert_eq!(classify(b"1 x 2", &s, &r, &r, Shape::default()), Class::E1);
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
        assert_eq!(classify(b"1\x00", &s, &r, &r, Shape::default()), Class::E2);
        assert_eq!(
            classify(b"\"1\x00\"", &s, &r, &r, Shape::default()),
            Class::BugAgree
        );
    }

    #[test]
    fn splits_follow_the_pin() {
        let a = t("1\n", "", 0);
        let b = t("2\n", "", 0);
        assert_eq!(classify(b"", &a, &a, &b, Shape::default()), Class::Split171);
        assert_eq!(classify(b"", &b, &a, &b, Shape::default()), Class::Split182);
        assert_eq!(
            classify(b"", &t("3\n", "", 0), &a, &b, Shape::default()),
            Class::BugSplit
        );
        assert_eq!(classify(b"", &a, &a, &a, Shape::default()), Class::Same);
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
        )
        .expect("parses");
        assert_eq!(v, vec![serde_json::json!(1), serde_json::json!([2])]);
    }
}
