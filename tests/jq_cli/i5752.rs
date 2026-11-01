//! #5752: `strptime`'s `%b`, `%B` and `%h` read a month's full name if the input starts with one,
//! else its three-letter abbreviation, either way in any letter case, and no more. The letters
//! that follow belong to the rest of the format, so `Marx` and `Marc` are `Mar` and a stray letter,
//! not a longer spelling of March. Each row is what jq 1.7.1 (`/usr/bin/jq`) answers:
//! `(format, text, stdout, stderr, exit status)`.

use anyhow::Result;

use crate::run_jq_full;

const NO_MATCH: i32 = 5;

const ROWS: &[(&str, &str, &str, &str, i32)] = &[
    (
        "%d %B %Y",
        "05 March 2015",
        "[2015,2,5,0,0,0,4,63]\n",
        "",
        0,
    ),
    ("%d %B %Y", "05 Mar 2015", "[2015,2,5,0,0,0,4,63]\n", "", 0),
    (
        "%d %B %Y",
        "05 MARCH 2015",
        "[2015,2,5,0,0,0,4,63]\n",
        "",
        0,
    ),
    (
        "%d %B %Y",
        "05 march 2015",
        "[2015,2,5,0,0,0,4,63]\n",
        "",
        0,
    ),
    ("%d %B %Y", "05 mAr 2015", "[2015,2,5,0,0,0,4,63]\n", "", 0),
    (
        "%d %b %Y",
        "05 March 2015",
        "[2015,2,5,0,0,0,4,63]\n",
        "",
        0,
    ),
    ("%d %h %Y", "05 Mar 2015", "[2015,2,5,0,0,0,4,63]\n", "", 0),
    (
        "%d %B %Y",
        "31 December 1999",
        "[1999,11,31,0,0,0,5,364]\n",
        "",
        0,
    ),
    ("%d %B %Y", "01 Sep 2015", "[2015,8,1,0,0,0,2,243]\n", "", 0),
    (
        "%d %B %Y",
        "01 September 2015",
        "[2015,8,1,0,0,0,2,243]\n",
        "",
        0,
    ),
    ("%d %B %Y", "29 Feb 2016", "[2016,1,29,0,0,0,1,59]\n", "", 0),
    ("%d %B %Y", "07 May 2015", "[2015,4,7,0,0,0,4,126]\n", "", 0),
    (
        "%d %B %Y",
        "07 June 2015",
        "[2015,5,7,0,0,0,0,157]\n",
        "",
        0,
    ),
    ("%d %B %Y", "07 Jun 2015", "[2015,5,7,0,0,0,0,157]\n", "", 0),
    ("%d%b%Y", "05Mar2015", "[2015,2,5,0,0,0,4,63]\n", "", 0),
    (
        "%d %B %Y",
        "05 Mart 2015",
        "",
        "jq: error (at <stdin>:0): date \"05 Mart 2015\" does not match format \"%d %B %Y\"\n",
        NO_MATCH,
    ),
    (
        "%d %B %Y",
        "05 Marx 2015",
        "",
        "jq: error (at <stdin>:0): date \"05 Marx 2015\" does not match format \"%d %B %Y\"\n",
        NO_MATCH,
    ),
    (
        "%d %B %Y",
        "05 Marchx 2015",
        "",
        "jq: error (at <stdin>:0): date \"05 Marchx 2015\" does not match format \"%d %B %Y\"\n",
        NO_MATCH,
    ),
    (
        "%d %B %Y",
        "05 Marc 2015",
        "",
        "jq: error (at <stdin>:0): date \"05 Marc 2015\" does not match format \"%d %B %Y\"\n",
        NO_MATCH,
    ),
    (
        "%d %B %Y",
        "05 Ma 2015",
        "",
        "jq: error (at <stdin>:0): date \"05 Ma 2015\" does not match format \"%d %B %Y\"\n",
        NO_MATCH,
    ),
    (
        "%d %B %Y",
        "05 Septembe 2015",
        "",
        "jq: error (at <stdin>:0): date \"05 Septembe 2015\" does not match format \"%d %B %Y\"\n",
        NO_MATCH,
    ),
    (
        "%d %B %Y",
        "05 Sept 2015",
        "",
        "jq: error (at <stdin>:0): date \"05 Sept 2015\" does not match format \"%d %B %Y\"\n",
        NO_MATCH,
    ),
    (
        "%d %B %Y",
        "05 Mayy 2015",
        "",
        "jq: error (at <stdin>:0): date \"05 Mayy 2015\" does not match format \"%d %B %Y\"\n",
        NO_MATCH,
    ),
    (
        "%d %B %Y",
        "05 Xyz 2015",
        "",
        "jq: error (at <stdin>:0): date \"05 Xyz 2015\" does not match format \"%d %B %Y\"\n",
        NO_MATCH,
    ),
    (
        "%d %b %Y",
        "05 Mart 2015",
        "",
        "jq: error (at <stdin>:0): date \"05 Mart 2015\" does not match format \"%d %b %Y\"\n",
        NO_MATCH,
    ),
    (
        "%d %h %Y",
        "05 Marx 2015",
        "",
        "jq: error (at <stdin>:0): date \"05 Marx 2015\" does not match format \"%d %h %Y\"\n",
        NO_MATCH,
    ),
    (
        "%b",
        "Marx",
        "",
        "jq: error (at <stdin>:0): date \"Marx\" does not match format \"%b\"\n",
        NO_MATCH,
    ),
    (
        "%B",
        "March!",
        "",
        "jq: error (at <stdin>:0): date \"March!\" does not match format \"%B\"\n",
        NO_MATCH,
    ),
    (
        "%d %b",
        "05 Març",
        "",
        "jq: error (at <stdin>:0): date \"05 Març\" does not match format \"%d %b\"\n",
        NO_MATCH,
    ),
    (
        "%b%d",
        "Mar05x",
        "",
        "jq: error (at <stdin>:0): date \"Mar05x\" does not match format \"%b%d\"\n",
        NO_MATCH,
    ),
    (
        "%d%b%Y",
        "05Marx2015",
        "",
        "jq: error (at <stdin>:0): date \"05Marx2015\" does not match format \"%d%b%Y\"\n",
        NO_MATCH,
    ),
];

#[test]
fn test_strptime_month_name_is_a_full_name_or_its_abbreviation_5752() -> Result<()> {
    for (format, text, stdout, stderr, status) in ROWS {
        let stdin = serde_json::to_string(text)?;
        let program = format!("strptime({})", serde_json::to_string(format)?);
        let (out, err, code) = run_jq_full(&["-c", &program], Some(&stdin))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*stdout, *stderr, *status),
            "strptime({format:?}) over {text:?}"
        );
    }
    Ok(())
}

/// `mktime` over a parsed month agrees with the abbreviation and the full name alike.
#[test]
fn test_strptime_full_name_and_abbreviation_agree_5752() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &[
            "-nc",
            "[\"05 Mar 2015\", \"05 March 2015\"] | map(strptime(\"%d %B %Y\") | mktime) | unique",
        ],
        None,
    )?;
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        ("[1425513600]\n", "", 0)
    );
    Ok(())
}
