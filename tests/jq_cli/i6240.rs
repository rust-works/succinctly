//! #6240: inside a character class, a `\xHH` at 0x80 or above is a byte of a UTF-8 sequence, as it is
//! outside one (#6070), and not the code point U+00HH. Oniguruma builds a class from two sets, a bitset
//! of single bytes (what an ASCII character matches) and ranges of code points (what a multibyte
//! character matches): a byte that starts no sequence (`\x80`, `\xF8`) is a byte and matches nothing, a
//! decoded sequence is a code point, and a range between a byte and a code point is both. Every row
//! was captured from jq 1.7.1 (`/usr/bin/jq -n -r`) over the text below, as the code points of each
//! match.

use anyhow::Result;

use crate::run_jq_full;

/// `aAz-`, DEL, U+0080, U+00A9, U+00AB, U+00C0, U+00C3, `é`, `ë`, U+00F7, `€` and an emoji.
const TEXT: &str = "aAz-\u{7f}\u{80}\u{a9}\u{ab}\u{c0}\u{c3}\u{e9}\u{eb}\u{f7}\u{20ac}\u{1f600}";

#[test]
fn test_a_utf8_byte_run_in_a_class_reads_as_jq_does_6240() -> Result<()> {
    for (pattern, expected) in [
        (r"[\xC3\xA9]", "[233]"),
        (r"[\x80]", "[]"),
        (r"[\xA9]", "[]"),
        (r"[\xC3\xA9a]", "[97,233]"),
        (r"[a\xC3\xA9]", "[97,233]"),
        (r"[\xC3\xA9\xC3\xAB]", "[233,235]"),
        (r"[\xE2\x82\xAC]", "[8364]"),
        (r"[\xF0\x9F\x98\x80]", "[128512]"),
        (r"[\xC3\xA9\xA9]", "[233]"),
        (r"[\xC3\xA9\x80]", "[233]"),
        (r"[\xC2\x80]", "[128]"),
        (r"[\xC3\x80]", "[192]"),
        (r"[\xC0\x80]", "[]"),
        (r"[\xC1\xBF]", "[]"),
        (r"[\xED\xA0\x80]", "[]"),
        (r"[\xF4\x90\x80\x80]", "[]"),
        (r"[\xF5]", "[]"),
        (r"[\xF8]", "[]"),
        (r"[\xFF]", "[]"),
        (r"[\xF8\x80]", "[]"),
        (
            r"[^\xC3\xA9]",
            "[97,65,122,45,127,128,169,171,192,195,235,247,8364,128512]",
        ),
        (
            r"[^\x80]",
            "[97,65,122,45,127,128,169,171,192,195,233,235,247,8364,128512]",
        ),
        (
            r"[^\xC1\xBF]",
            "[97,65,122,45,127,128,169,171,192,195,233,235,247,8364,128512]",
        ),
        (r"[\xC3\xA9-\xC3\xAB]", "[233,235]"),
        (r"[\xC3\xA9-\xC3\xA9]", "[233]"),
        (r"[\xC3\xA9-\xC3\xAB-]", "[45,233,235]"),
        (r"[\xC3\xAB-\xC3\xAB-a]", "[97,45,235]"),
        (r"[a-\xC3\xA9]", "[97,122,127,128,169,171,192,195,233]"),
        (r"[a-\xC3\xA9-z]", "[97,122,45,127,128,169,171,192,195,233]"),
        (
            r"[\x41-\xC3\xA9]",
            "[97,65,122,127,128,169,171,192,195,233]",
        ),
        (r"[\x80-\xC3\xA9]", "[128,169,171,192,195,233]"),
        (r"[\xA9-\xC3\xA9]", "[169,171,192,195,233]"),
        (r"[\xC3\xA9-é]", "[233]"),
        (r"[é-\xC3\xAB]", "[233,235]"),
        (r"[\x{E9}-\xC3\xAB]", "[233,235]"),
        (r"[\xC3\xA9-\x{FF}]", "[233,235,247]"),
        (r"[\x80-\x90]", "[]"),
        (r"[\xA9-\xAB]", "[]"),
        (r"[a-\x90]", "[97,122,127]"),
        (r"[\x41-\x90]", "[97,65,122,127]"),
        (r"[\x7F-\xC1\xBF]", "[127]"),
        (r"[\xC1\xBF-\x7F]", "[127]"),
        (r"[\xC1\xBF-\xC3\xA9]", "[128,169,171,192,195,233]"),
        (r"[\xC1\xBF-\xC1\xBF]", "[]"),
        (
            r"[a-\xED\xA0\x80]",
            "[97,122,127,128,169,171,192,195,233,235,247,8364]",
        ),
        (r"[\xED\xA0\x80-\xEE\x80\x80]", "[]"),
        (r"[\xED\xA0\x80-\xED\xA0\x81]", "[]"),
        (
            r"[a-\xF4\x90\x80\x80]",
            "[97,122,127,128,169,171,192,195,233,235,247,8364,128512]",
        ),
        (r"[a-\xF5\x80\x80\x80]", "[97,122,127]"),
        (r"[\xC3\xA9-\xF7\xBF\xBF\xBF]", "[233,235,247]"),
        (r"[\xF8-\xFF]", "[]"),
        (r"[\xBF-\xC3\xA9]", "[192,195,233]"),
        (r"[a-c-\xED\xA0\x80]", "[97,45]"),
        (r"[\xC3\xA9-]", "[45,233]"),
        (r"[-\xC3\xA9]", "[45,233]"),
        (
            r"[^\xBF-\xF0\x90\x80\x80]",
            "[97,65,122,45,127,128,169,171,128512]",
        ),
        (
            r"[^\xC3\xA9-\xC3\xAB]",
            "[97,65,122,45,127,128,169,171,192,195,247,8364,128512]",
        ),
        (r"(?i)[\xC3\xA9]", "[233]"),
        (r"(?i)[\xC3\xA9-\xC3\xAB]", "[233,235]"),
        (r"[[:alpha:]\xC3\xA9]", "[97,65,122,192,195,233,235]"),
        (r"[\w\xC3\xA9]", "[97,65,122,192,195,233,235]"),
        (r"[\xC3\xA9]+", "[233]"),
        (r"[\xC3\xA9\xC3\xAB]{2}", "[233]"),
    ] {
        let (stdout, stderr, status) = run_jq_full(
            &[
                "-n",
                "-r",
                "--arg",
                "p",
                pattern,
                "--arg",
                "s",
                TEXT,
                r#"$s | [match($p; "g") | .string | explode[0]] | tojson"#,
            ],
            None,
        )?;
        assert_eq!(
            (stdout.trim_end(), status),
            (expected, 0),
            "{pattern}: {stderr}"
        );
    }
    Ok(())
}

/// The members before and around a byte run decide whether it is the end of a range, the start of
/// one, or a member of its own.
#[test]
fn test_the_members_around_a_utf8_byte_run_in_a_class_6240() -> Result<()> {
    for (pattern, expected) in [
        (
            r"[\n-\xC3\xA9]",
            "[97,65,122,45,127,128,169,171,192,195,233]",
        ),
        (r"[\t\xC3\xA9]", "[233]"),
        (r"[\r\f\v\a\e\xC3\xA9]", "[233]"),
        (
            r"[\x{41}-\xC3\xA9]",
            "[97,65,122,127,128,169,171,192,195,233]",
        ),
        (r"[\x{E9}-\xC3\xAB]", "[233,235]"),
        (r"[\x{41}-\x80]", "[97,65,122,127]"),
        (r"[\t-\x80]", "[97,65,122,45,127]"),
        (r"[\\-\xC3\xA9]", "[97,122,127,128,169,171,192,195,233]"),
        (
            r"[\--\xC3\xA9]",
            "[97,65,122,45,127,128,169,171,192,195,233]",
        ),
        (r"[]-\xC3\xA9]", "[97,122,127,128,169,171,192,195,233]"),
        (r"[é\xC3\xA9]", "[233]"),
        (r"[é-\xC3\xAB]", "[233,235]"),
        (r"[\p{L}\xC3\xA9]", "[97,65,122,192,195,233,235]"),
        (
            r"[[:alpha:]\xC3\xA9-\xC3\xAB]",
            "[97,65,122,192,195,233,235]",
        ),
        (
            r"[\101-\xC3\xA9]",
            "[97,65,122,127,128,169,171,192,195,233]",
        ),
        (r"[\101\xC3\xA9]", "[65,233]"),
        (r"[a-c-\xC3\xA9]", "[97,45,233]"),
        (r"[a-c-\x80]", "[97,45]"),
        (r"[a-c-\xC3\xA9-\xC3\xAB]", "[97,45,233,235]"),
        (
            r"[a-\xC3\xA9-\xC3\xAB]",
            "[97,122,45,127,128,169,171,192,195,233,235]",
        ),
        (r"[a-\xC3\xA9-]", "[97,122,45,127,128,169,171,192,195,233]"),
        (r"[\xC3\xA9-\xC3\xAB-\xC3\xAD-]", "[45,233,235]"),
        (
            r"[^-\xC3\xA9]",
            "[97,65,122,127,128,169,171,192,195,235,247,8364,128512]",
        ),
        (
            r"[^\xC3\xA9-]",
            "[97,65,122,127,128,169,171,192,195,235,247,8364,128512]",
        ),
        (r"[\x]\xC3\xA9]", "[]"),
        (
            r"[\xC3\xA9\x{41}-\xC3\xAB]",
            "[97,65,122,127,128,169,171,192,195,233,235]",
        ),
        (r"[\xC3\xA9[\xC3\xAB]", "[233,235]"),
        (r"[\xC3\xA9&\xC3\xAB]", "[233,235]"),
        (
            r"[&-\xC3\xA9]",
            "[97,65,122,45,127,128,169,171,192,195,233]",
        ),
        (r"[[-\xC3\xA9]", "[97,122,127,128,169,171,192,195,233]"),
        (r"[\xC3\xA9-\x{ff}]", "[233,235,247]"),
        (r"[\xC3\xA9-\x{10FFFF}]", "[233,235,247,8364,128512]"),
        (r"[\x80-\x{ff}]", "[128,169,171,192,195,233,235,247]"),
        (r"[a-\x{7f}-\x80]", "[97,122,45,127]"),
        (r"[\x{20AC}-\xE2\x82\xAC]", "[8364]"),
        (r"[\xE2\x82\xAC-\x{20AC}]", "[8364]"),
        (
            r"[\1-\xC3\xA9]",
            "[97,65,122,45,127,128,169,171,192,195,233]",
        ),
        (r"[\0-\x80]", "[97,65,122,45,127]"),
        (r"[\1-\x90]", "[97,65,122,45,127]"),
        (r"[\101-\x90]", "[97,65,122,127]"),
        (r"[\b-\x90]", "[97,65,122,45,127]"),
        (
            r"[\b-\xC3\xA9]",
            "[97,65,122,45,127,128,169,171,192,195,233]",
        ),
        (r"[\cA-\x90]", "[97,65,122,45,127]"),
        (
            r"[\cA-\xC3\xA9]",
            "[97,65,122,45,127,128,169,171,192,195,233]",
        ),
        (r"[\x{41}-\x90]", "[97,65,122,127]"),
        (r"[\8-\xC3\xA9]", "[97,65,122,127,128,169,171,192,195,233]"),
        (r"[[:alpha:]a-\x90]", "[97,65,122,127,192,195,233,235]"),
        (
            r"[[:alpha:]a-\xC3\xA9]",
            "[97,65,122,127,128,169,171,192,195,233,235]",
        ),
        (r"[\r-\x90]", "[97,65,122,45,127]"),
        (r"[\f-\x90]", "[97,65,122,45,127]"),
        (r"[\v-\x90]", "[122,127]"),
        (r"[\a-\x90]", "[97,65,122,45,127]"),
        (r"[\e-\x90]", "[97,65,122,45,127]"),
        (
            r"[\d\w\s\p{L}\P{L}a-\x90]",
            "[97,65,122,45,127,128,169,171,192,195,233,235,247,8364,128512]",
        ),
        (
            r"[\D\W\S\p{L}a-\xC3\xA9]",
            "[97,65,122,45,127,128,169,171,192,195,233,235,247,8364,128512]",
        ),
        (r"[\h-\x90]", "[122,127]"),
        (r"[\g-\x90]", "[122,127]"),
        (r"[\q-\x90]", "[122,127]"),
        (r"[\H-\xC3\xA9]", "[97,122,127,128,169,171,192,195,233]"),
        (r"[\k-\x90]", "[122,127]"),
        (r"[\o-\x90]", "[122,127]"),
        (r"[\p-\x90]", "[122,127]"),
        (r"[\u-\x90]", "[122,127]"),
        (r"[\é-\xC3\xAB]", "[233,235]"),
        (r"[\x{80}-\x90]", "[128]"),
        (r"[\x1-\x90]", "[97,65,122,45,127]"),
        (r"[\x-\x90]", "[97,65,122,45,127]"),
        (r"[\xg-\x90]", "[122,127]"),
        (r"[\N-\x90]", "[97,122,127]"),
        (r"[\Q-\x90]", "[97,122,127]"),
    ] {
        let (stdout, stderr, status) = run_jq_full(
            &[
                "-n",
                "-r",
                "--arg",
                "p",
                pattern,
                "--arg",
                "s",
                TEXT,
                r#"$s | [match($p; "g") | .string | explode[0]] | tojson"#,
            ],
            None,
        )?;
        assert_eq!(
            (stdout.trim_end(), status),
            (expected, 0),
            "{pattern}: {stderr}"
        );
    }
    Ok(())
}

/// What jq refuses in a class, with its reason. The exit code is 5 and the message names the reason;
/// jq's own text is `Regex failure: <reason>`, ours `invalid regex: <reason>`.
#[test]
fn test_a_bad_utf8_byte_run_or_range_in_a_class_is_refused_6240() -> Result<()> {
    for (pattern, reason) in [
        (r"[\xC3]", "too short multibyte code string"),
        (r"[\xC1]", "too short multibyte code string"),
        (r"[\xC3a]", "too short multibyte code string"),
        (r"[\xC3\x{41}]", "too short multibyte code string"),
        (r"[\xE2\x82]", "too short multibyte code string"),
        (r"[\xE2\x41]", "too short multibyte code string"),
        (r"[\xC3\xA9\xC3]", "too short multibyte code string"),
        (r"[\xC3\xA9-\xC3]", "too short multibyte code string"),
        (r"[\xC3\x41]", "invalid code point value"),
        (r"[\xC3\xC3]", "invalid code point value"),
        (r"[\xC0\x41]", "invalid code point value"),
        (r"[\xE2\x41\x80]", "invalid code point value"),
        (r"[\xC3\x1]", "invalid code point value"),
        (r"[\xC3\xg]", "invalid code point value"),
        (r"[\xC3\xA9-z]", "empty range in char class"),
        (r"[\xC3\xA9-\x41]", "empty range in char class"),
        (r"[\xC3\xA9-\xAB]", "empty range in char class"),
        (r"[\xC3\xA9-\x80]", "empty range in char class"),
        (r"[\xBF-\x80]", "empty range in char class"),
        (r"[\x80-a]", "empty range in char class"),
        (r"[\xED\xA0\x80-\xC3\xA9]", "empty range in char class"),
        (r"[\xF7\xBF\xBF\xBF-a]", "empty range in char class"),
        (r"[\x41-\xC0\x80]", "empty range in char class"),
    ] {
        let (stdout, stderr, status) =
            run_jq_full(&["-n", "--arg", "p", pattern, r#""a" | test($p)"#], None)?;
        assert_eq!((stdout.as_str(), status), ("", 5), "{pattern}: {stderr}");
        assert!(
            stderr.contains(&format!("invalid regex: {reason}")),
            "{pattern}: {stderr}"
        );
    }
    Ok(())
}
