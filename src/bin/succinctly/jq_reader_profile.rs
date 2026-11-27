//! The release whose input reader `succinctly jq` reproduces (ADR-0033, #7987).
//!
//! jq's answer at the edges of its input (where a read ends, how deep a document may nest, when
//! the stream is over) is a property of how its reader is built, and that differs between
//! releases. ADR-0018's #7963 amendment pins the reader to jq 1.7.1 with three listed exceptions
//! taken from jq 1.8.2. [`ReaderProfile`] writes those facts down once, and [`JQ_READER`] is the
//! profile the CLI ships: every route reads its reader constants from it, and from nothing else.
//!
//! A profile is one whole release, never a switch-by-switch mixture: ADR-0033's prototype paired
//! jq 1.7.1's read size with jq 1.8.2's NUL reading, and gave answers neither release gives on 7
//! inputs where both agree. [`JQ_READER`] is therefore [`ReaderProfile::release`] of jq 1.7.1, less
//! the fields [`EXCEPTIONS`] names, each with its issues and its reason. The test at the end of
//! this module holds the two to each other, so a fourth exception is a visible, reviewed change.
//!
//! The profile is the reader half of ADR-0033's split (decision 4): it stays in the CLI, with the
//! read arithmetic and standard input. The NaN payload (#7522) is not a reader fact and is not in
//! it (decision 2): it is a number-grammar rule, `json::validate::jq_special_number`'s alone.
//!
//! An exception that a route implements in only its shipped form, with no code for the other
//! release, is read by a `const` assertion at that route: changing the field then fails the build
//! at the code that would have to change with it.

/// A jq release whose reader a [`ReaderProfile`] describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// jq 1.8.2's profile is read by the exception test alone: it is where each exception's value
// comes from.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum Release {
    /// jq 1.7.1, the pinned reference (ADR-0018 rule 1, `/usr/bin/jq`).
    Jq171,
    /// jq 1.8.2, the release the three listed exceptions are taken from.
    Jq182,
}

/// The release-dependent facts of jq's input reader that the routes read. Each field names the
/// jq source it models.
///
/// It holds the facts the routes hard-code today. The reader's other release-dependent facts (the
/// position after the last read, #7607, and the closing refill) are still decided inside the
/// routes' own logic, not by a constant, and join the profile when ADR-0033's model (step 3)
/// reads them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReaderProfile {
    /// The release this profile reads as.
    #[cfg_attr(not(test), allow(dead_code))]
    pub release: Release,
    /// The most bytes one `fgets` call of `jq_util_input_read_more` (`src/util.c`) returns: jq
    /// 1.7.1 calls `fgets(state->buf, sizeof(state->buf), f)` on a `char buf[4096]`, and `fgets`
    /// keeps one byte for the terminating NUL, so 4,095. A line longer than this is read in pieces
    /// of this size (#4303). jq 1.8.2 reads `max_gets_len - 1` = 4,091 bytes and completes a read
    /// that ends inside a UTF-8 sequence.
    pub read_len: usize,
    /// `MAX_PARSING_DEPTH` (`src/jv_parse.c`): a `[` or `{` is refused when the parser's stack (one
    /// entry per array, one per object and one per pending key) already holds this many. 256 in jq
    /// 1.7.1, 10,000 in jq 1.8.2. The number itself is the parser's
    /// ([`succinctly::jq::JQ_MAX_PARSING_DEPTH`], which `fromjson` and `--argjson` read too): the
    /// parser half of ADR-0033's model moves to `src/jq/` at step 3.
    pub max_depth: usize,
    /// Whether `--seq` reads every record. jq 1.7.1's `jq_util_input_next_input` took the read that
    /// reached the end of the last file for the end of the stream, so an empty record that was the
    /// first thing that call scanned ended the stream silently (#2998); jq 1.8.2 reports the end
    /// only once the last file is closed, one read later (jqlang/jq#3279, commit 4e3088f).
    pub seq_reads_every_record: bool,
    /// Whether a `--stream --slurp` read takes the empty closing read before it closes the array.
    /// jq 1.7.1's slurp loop closed the array at the call that emptied the read that hit the end
    /// of the last file, so what that call's parser still held (the closing event of a container
    /// that ends the text, #7523, or the fault of a text that ends in one, #7807) came out after
    /// the array; jq 1.8.2 reads the closing read first (jqlang/jq#3279, commit 4e3088f).
    pub stream_slurp_reads_to_close: bool,
    /// Whether `-R` keeps a multi-byte character that straddles the end of a `fgets` read. jq
    /// 1.7.1 reads each piece of a long line as text of its own, so a character cut by the end of
    /// a 4,095-byte read is U+FFFD replacement characters (#5476); jq 1.8.2 keeps it
    /// (jqlang/jq#3389, fixed by jqlang/jq#3421).
    pub raw_keeps_straddling_character: bool,
}

/// One field in which [`JQ_READER`] departs from [`ReaderProfile::release`] of its release.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Exception {
    /// The [`ReaderProfile`] field, by name.
    pub field: &'static str,
    /// The release whose value the field takes.
    pub from: Release,
    /// The issues that adopted it.
    pub issues: &'static [u32],
    /// Why the pin does not hold there, as the compliance page records it.
    pub reason: &'static str,
}

/// Every field in which [`JQ_READER`] departs from jq 1.7.1: the three adoptions of jq 1.8.2 that
/// ADR-0018's #7963 amendment lists, each recorded on `docs/compliance/jq/limitations.md`.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const EXCEPTIONS: &[Exception] = &[
    Exception {
        field: "seq_reads_every_record",
        from: Release::Jq182,
        issues: &[7718],
        reason: "jq 1.7.1 could end a `--seq` stream silently at an empty record and drop the \
                 records behind it (#2998); jq 1.8.2 reads every record, on every route that \
                 reads `--seq`",
    },
    Exception {
        field: "stream_slurp_reads_to_close",
        from: Release::Jq182,
        issues: &[7523, 7807],
        reason: "`--stream --slurp` keeps jq 1.8.2's order (the last closing event in the array, \
                 #7523) and its fault (met before the array closes, #7807), which upstream fixed \
                 as a bug (jqlang/jq#3279)",
    },
    Exception {
        field: "raw_keeps_straddling_character",
        from: Release::Jq182,
        issues: &[5476],
        reason: "matching jq 1.7.1 would corrupt valid text: a character straddling the \
                 4,095-byte `-R` read is U+FFFD there, and jq 1.8.2 keeps it (jqlang/jq#3389)",
    },
];

impl ReaderProfile {
    /// `release`'s reader as released.
    pub(crate) const fn release(release: Release) -> Self {
        match release {
            Release::Jq171 => Self {
                release,
                read_len: 4095,
                max_depth: succinctly::jq::JQ_MAX_PARSING_DEPTH,
                seq_reads_every_record: false,
                stream_slurp_reads_to_close: false,
                raw_keeps_straddling_character: false,
            },
            Release::Jq182 => Self {
                release,
                read_len: 4091,
                max_depth: 10_000,
                seq_reads_every_record: true,
                stream_slurp_reads_to_close: true,
                raw_keeps_straddling_character: true,
            },
        }
    }

    /// What the CLI ships: jq 1.7.1, less [`EXCEPTIONS`].
    const fn shipped() -> Self {
        Self {
            seq_reads_every_record: true,
            stream_slurp_reads_to_close: true,
            raw_keeps_straddling_character: true,
            ..Self::release(Release::Jq171)
        }
    }

    /// Every field but the release, by name, as a number, for the exception test. The
    /// destructuring makes a new field a compile error here until it is listed.
    #[cfg(test)]
    fn fields(&self) -> [(&'static str, usize); 5] {
        let Self {
            release: _,
            read_len,
            max_depth,
            seq_reads_every_record,
            stream_slurp_reads_to_close,
            raw_keeps_straddling_character,
        } = *self;
        [
            ("read_len", read_len),
            ("max_depth", max_depth),
            (
                "seq_reads_every_record",
                usize::from(seq_reads_every_record),
            ),
            (
                "stream_slurp_reads_to_close",
                usize::from(stream_slurp_reads_to_close),
            ),
            (
                "raw_keeps_straddling_character",
                usize::from(raw_keeps_straddling_character),
            ),
        ]
    }
}

/// The reader the CLI ships: jq 1.7.1, less the three listed exceptions ([`EXCEPTIONS`]).
pub(crate) const JQ_READER: ReaderProfile = ReaderProfile::shipped();

#[cfg(test)]
mod tests {
    use super::{ReaderProfile, Release, EXCEPTIONS, JQ_READER};

    /// ADR-0033: the shipped profile is jq 1.7.1 with exactly the three exceptions the #7963
    /// amendment lists, each named with its issues and taking jq 1.8.2's value. A fourth exception
    /// has to be added to [`EXCEPTIONS`] and to this test, so it cannot slip in as a field change.
    #[test]
    fn shipped_profile_is_jq_1_7_1_with_the_three_listed_exceptions_7987() {
        assert_eq!(JQ_READER.release, Release::Jq171);
        let pinned = ReaderProfile::release(Release::Jq171);
        let newer = ReaderProfile::release(Release::Jq182);

        let departing: Vec<&str> = JQ_READER
            .fields()
            .iter()
            .zip(pinned.fields())
            .filter(|(shipped, pinned)| shipped.1 != pinned.1)
            .map(|(shipped, _)| shipped.0)
            .collect();
        let listed: Vec<&str> = EXCEPTIONS.iter().map(|e| e.field).collect();
        assert_eq!(
            departing, listed,
            "every departure from 1.7.1 is listed, and no more"
        );

        let named: Vec<(&str, &[u32])> = EXCEPTIONS.iter().map(|e| (e.field, e.issues)).collect();
        assert_eq!(
            named,
            [
                ("seq_reads_every_record", &[7718][..]),
                ("stream_slurp_reads_to_close", &[7523, 7807][..]),
                ("raw_keeps_straddling_character", &[5476][..]),
            ],
        );

        for exception in EXCEPTIONS {
            assert_eq!(exception.from, Release::Jq182, "{}", exception.field);
            assert!(!exception.reason.is_empty(), "{}", exception.field);
            let value = |profile: &ReaderProfile| {
                profile
                    .fields()
                    .into_iter()
                    .find(|(name, _)| *name == exception.field)
                    .map(|(_, value)| value)
            };
            assert_eq!(
                value(&JQ_READER),
                value(&newer),
                "{} takes 1.8.2's value",
                exception.field
            );
        }
    }

    /// The facts the routes read are jq 1.7.1's: a 4,095-byte `fgets` read (`char buf[4096]`) and
    /// a parser stack of 256 (`MAX_PARSING_DEPTH`).
    #[test]
    fn shipped_reads_and_depth_are_jq_1_7_1s_7987() {
        assert_eq!(JQ_READER.read_len, 4095);
        assert_eq!(JQ_READER.max_depth, 256);
        assert_eq!(JQ_READER.max_depth, succinctly::jq::JQ_MAX_PARSING_DEPTH);
    }
}
