//! #5754: `strftime`/`strflocaltime` name days and months, and lay out `%c`/`%x`/`%X`/`%r`,
//! in the locale `LC_ALL`/`LC_TIME`/`LANG` select, as jq's `setlocale(LC_ALL, "")` makes them.
//! A locale that is not installed, or a category that is invalid, leaves the whole process
//! in C. Each expectation is what jq 1.7.1 (`/usr/bin/jq`) prints on macOS; a case whose
//! locale the host lacks is skipped.

use std::process::Command;

use anyhow::Result;

/// 2015-03-05 23:53:41 UTC, a Thursday.
const STAMP: &str = "1425599621";

/// Whether `locale -a` lists `name` (compared without case, `UTF-8` as `utf8`).
fn installed(name: &str) -> bool {
    let normalise = |s: &str| s.to_ascii_lowercase().replace("utf-8", "utf8");
    let Ok(out) = Command::new("locale").arg("-a").output() else {
        return false;
    };
    let want = normalise(name);
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|line| normalise(line) == want)
}

/// `STAMP | filter` with `-r`, in the environment `env` (an `LC_ALL`-free base otherwise).
fn run(filter: &str, env: &[(&str, &str)]) -> Result<String> {
    let mut command = Command::new(crate::succinctly_bin());
    command
        .args(["jq", "-r", filter])
        .env_remove("LC_ALL")
        .env_remove("LC_TIME")
        .env_remove("LANG")
        .env_remove("LC_MESSAGES")
        .envs(env.iter().copied())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    let mut child = command.spawn()?;
    {
        use std::io::Write;
        child
            .stdin
            .take()
            .expect("stdin is piped")
            .write_all(STAMP.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    Ok(String::from_utf8(out.stdout)?)
}

#[test]
fn c_locale_is_english() -> Result<()> {
    for env in [
        &[][..],
        &[("LC_ALL", "C")],
        &[("LANG", "C")],
        &[("LC_ALL", "POSIX")],
    ] {
        assert_eq!(
            run(r#"strftime("%A|%a|%B|%b|%p|%c|%x|%X|%r")"#, env)?,
            "Thursday|Thu|March|Mar|PM|Thu Mar  5 23:53:41 2015|03/05/15|23:53:41|11:53:41 PM\n"
        );
    }
    Ok(())
}

#[test]
fn german_names_follow_each_variable_in_precedence_order() -> Result<()> {
    if !installed("de_DE.UTF-8") {
        return Ok(());
    }
    let filter = r#"strftime("%A %B")"#;
    for env in [
        &[("LC_ALL", "de_DE.UTF-8")][..],
        &[("LC_TIME", "de_DE.UTF-8")],
        &[("LANG", "de_DE.UTF-8")],
        // `LC_ALL` beats `LC_TIME`, which beats `LANG`.
        &[("LC_ALL", "de_DE.UTF-8"), ("LC_TIME", "C"), ("LANG", "C")],
        &[("LC_TIME", "de_DE.UTF-8"), ("LANG", "C")],
    ] {
        assert_eq!(run(filter, env)?, "Donnerstag März\n", "{env:?}");
    }
    for env in [
        &[("LC_ALL", "C"), ("LANG", "de_DE.UTF-8")][..],
        &[("LC_TIME", "C"), ("LANG", "de_DE.UTF-8")],
    ] {
        assert_eq!(run(filter, env)?, "Thursday March\n", "{env:?}");
    }
    Ok(())
}

#[test]
fn strflocaltime_names_follow_the_locale_too() -> Result<()> {
    if !installed("de_DE.UTF-8") {
        return Ok(());
    }
    assert_eq!(
        run(
            r#"strflocaltime("%A %B")"#,
            &[("LC_ALL", "de_DE.UTF-8"), ("TZ", "UTC")]
        )?,
        "Donnerstag März\n"
    );
    // A broken-down array takes the same names.
    assert_eq!(
        run(
            r#"[2015,2,5,23,53,41,4,63] | strftime("%A %B")"#,
            &[("LC_ALL", "de_DE.UTF-8")]
        )?,
        "Donnerstag März\n"
    );
    Ok(())
}

#[test]
fn en_us_date_layout_differs_from_c_only_in_x() -> Result<()> {
    if !installed("en_US.UTF-8") {
        return Ok(());
    }
    assert_eq!(
        run(r#"strftime("%A|%B|%x")"#, &[("LC_ALL", "en_US.UTF-8")])?,
        "Thursday|March|03/05/2015\n"
    );
    Ok(())
}

#[test]
fn numeric_specifiers_are_not_localized() -> Result<()> {
    if !installed("de_DE.UTF-8") {
        return Ok(());
    }
    assert_eq!(
        run(
            r#"strftime("%Y-%m-%d %H:%M:%S %j %u %Z")"#,
            &[("LC_ALL", "de_DE.UTF-8")]
        )?,
        "2015-03-05 23:53:41 064 4 UTC\n"
    );
    Ok(())
}

#[test]
fn an_uninstalled_locale_is_c() -> Result<()> {
    assert_eq!(
        run(r#"strftime("%A %B")"#, &[("LC_ALL", "xx_XX.UTF-8")])?,
        "Thursday March\n"
    );
    Ok(())
}

/// jq's `setlocale(LC_ALL, "")` is all or nothing: one invalid category and every category
/// is C, `LC_TIME` included.
#[test]
fn one_invalid_category_makes_the_whole_process_c() -> Result<()> {
    if !installed("de_DE.UTF-8") {
        return Ok(());
    }
    assert_eq!(
        run(
            r#"strftime("%A %B")"#,
            &[("LANG", "de_DE.UTF-8"), ("LC_MESSAGES", "xx_XX")]
        )?,
        "Thursday March\n"
    );
    Ok(())
}

/// glibc's `en_US` `%c` ends in the zone (`... PM GMT`); the zone it names must be the local
/// one, not the placeholder a format without `%Z` gets. macOS's `%c` has no zone at all.
#[test]
fn a_locale_layout_that_prints_the_zone_names_the_local_one() -> Result<()> {
    if !installed("en_US.UTF-8") {
        return Ok(());
    }
    let out = run(
        r#"strflocaltime("%c")"#,
        &[("LC_ALL", "en_US.UTF-8"), ("TZ", "Asia/Tokyo")],
    )?;
    assert!(!out.contains("UTC"), "{out:?}");
    Ok(())
}

// ---- `strptime` reads the locale's names too ----

/// `strptime(fmt)` of `input` with the first six fields printed (the weekday of a format that
/// sets no date is platform libc's, #5340), in the environment `env`: `Ok` fields, or `None`
/// when jq rejects the text (exit 5).
fn strptime(input: &str, fmt: &str, env: &[(&str, &str)]) -> Result<Option<String>> {
    let out = Command::new(crate::succinctly_bin())
        .args([
            "jq",
            "-c",
            "-n",
            "--arg",
            "i",
            input,
            "--arg",
            "f",
            fmt,
            "$i|strptime($f)|.[:6]",
        ])
        .env_remove("LC_ALL")
        .env_remove("LC_TIME")
        .env_remove("LANG")
        .env_remove("LC_MESSAGES")
        .envs(env.iter().copied())
        .output()?;
    Ok(match out.status.code() {
        Some(0) => Some(String::from_utf8(out.stdout)?.trim_end().to_string()),
        Some(5) => None,
        other => panic!("{input:?} | strptime({fmt:?}) exited {other:?}"),
    })
}

const MARCH_5: &str = "[2015,2,5,0,0,0]";

/// What the C locale reads for a day or month name: the full name or the three-letter
/// abbreviation (either for `%a` and `%A`, `%b`, `%B` and `%h`), in any ASCII case, and nothing
/// else. Rows from jq 1.7.1 on macOS; `Xyz`, `Thurs` and `Th` were once read as any run of letters.
#[test]
fn c_locale_reads_a_full_or_abbreviated_name_and_nothing_else() -> Result<()> {
    for (text, fmt, want) in [
        ("Thu Mar 05 2015", "%a %b %d %Y", Some(MARCH_5)),
        ("Thursday March 05 2015", "%A %B %d %Y", Some(MARCH_5)),
        ("Thursday Mar 05 2015", "%a %b %d %Y", Some(MARCH_5)),
        ("Thu March 05 2015", "%A %B %d %Y", Some(MARCH_5)),
        ("thu mar 05 2015", "%a %b %d %Y", Some(MARCH_5)),
        ("THU MAR 05 2015", "%A %B %d %Y", Some(MARCH_5)),
        ("Xyz Mar 05 2015", "%a %b %d %Y", None),
        ("Thurs Mar 05 2015", "%a %b %d %Y", None),
        ("Th Mar 05 2015", "%a %b %d %Y", None),
    ] {
        assert_eq!(strptime(text, fmt, &[])?.as_deref(), want, "{text} | {fmt}");
    }
    Ok(())
}

/// `%p` reads `AM` or `PM` in either case; text that is neither, or that runs on past it, is no
/// match (jq 1.7.1 on macOS).
#[test]
fn c_locale_reads_am_or_pm_and_nothing_else() -> Result<()> {
    for (text, want) in [
        ("11:30 PM", Some("[1900,0,0,23,30,0]")),
        ("11:30 pm", Some("[1900,0,0,23,30,0]")),
        ("11:30 Pm", Some("[1900,0,0,23,30,0]")),
        ("12:30 AM", Some("[1900,0,0,0,30,0]")),
        ("12:30 PM", Some("[1900,0,0,12,30,0]")),
        ("01:30 AM", Some("[1900,0,0,1,30,0]")),
        ("01:30 PM", Some("[1900,0,0,13,30,0]")),
        ("11:30 PMX", None),
        ("11:30 xx", None),
        ("11:30", None),
    ] {
        assert_eq!(strptime(text, "%I:%M %p", &[])?.as_deref(), want, "{text}");
    }
    Ok(())
}

/// The locale's own names, compared ASCII-case-insensitively with every other character exact.
#[test]
fn german_names_are_read_and_english_ones_are_not() -> Result<()> {
    if !installed("de_DE.UTF-8") {
        return Ok(());
    }
    let env = [("LC_ALL", "de_DE.UTF-8")];
    for (text, fmt, want) in [
        ("Donnerstag März 05 2015", "%A %B %d %Y", Some(MARCH_5)),
        ("Donnerstag, 05. März 2015", "%A, %d. %B %Y", Some(MARCH_5)),
        ("donnerstag märz 05 2015", "%A %B %d %Y", Some(MARCH_5)),
        // The full name is read for the abbreviated specifiers too.
        ("Donnerstag März 05 2015", "%a %b %d %Y", Some(MARCH_5)),
        // `Ä` is not ASCII, so its case is not folded.
        ("DONNERSTAG MÄRZ 05 2015", "%A %B %d %Y", None),
        ("Thursday March 05 2015", "%A %B %d %Y", None),
        ("Thu Mar 05 2015", "%a %b %d %Y", None),
    ] {
        assert_eq!(
            strptime(text, fmt, &env)?.as_deref(),
            want,
            "{text} | {fmt}"
        );
    }
    Ok(())
}

/// macOS spells the German abbreviations with a full stop (`Do.`), glibc without; the name
/// `strftime` prints is the one `strptime` reads.
#[cfg(target_vendor = "apple")]
#[test]
fn german_abbreviations_are_the_ones_strftime_prints() -> Result<()> {
    if !installed("de_DE.UTF-8") {
        return Ok(());
    }
    let env = [("LC_ALL", "de_DE.UTF-8")];
    assert_eq!(
        strptime("Do. März 05 2015", "%a %b %d %Y", &env)?.as_deref(),
        Some(MARCH_5)
    );
    assert_eq!(strptime("Do März 05 2015", "%a %b %d %Y", &env)?, None);
    assert_eq!(strptime("Do. Mär 05 2015", "%a %b %d %Y", &env)?, None);
    Ok(())
}

/// Under `de_DE` the library prints nothing for `%p`, and `strptime` then reads the empty
/// string as the am marker: `AM` and `PM` match nothing and are left over, so a format that
/// ends in `%p` rejects them (jq 1.7.1 on macOS).
#[cfg(target_vendor = "apple")]
#[test]
fn an_empty_am_marker_matches_nothing() -> Result<()> {
    if !installed("de_DE.UTF-8") {
        return Ok(());
    }
    let env = [("LC_ALL", "de_DE.UTF-8")];
    assert_eq!(
        strptime("11:30 ", "%I:%M %p", &env)?.as_deref(),
        Some("[1900,0,0,11,30,0]")
    );
    assert_eq!(strptime("11:30 PM", "%I:%M %p", &env)?, None);
    Ok(())
}

#[test]
fn en_us_names_are_english() -> Result<()> {
    if !installed("en_US.UTF-8") {
        return Ok(());
    }
    assert_eq!(
        strptime(
            "Thu Mar 05 2015",
            "%a %b %d %Y",
            &[("LC_ALL", "en_US.UTF-8")]
        )?
        .as_deref(),
        Some(MARCH_5)
    );
    Ok(())
}

/// A locale that is not installed is C for `strptime` as for `strftime`.
#[test]
fn an_uninstalled_locale_reads_english_names() -> Result<()> {
    let env = [("LC_ALL", "xx_XX.UTF-8")];
    assert_eq!(
        strptime("Thu Mar 05 2015", "%a %b %d %Y", &env)?.as_deref(),
        Some(MARCH_5)
    );
    assert_eq!(strptime("Xyz Mar 05 2015", "%a %b %d %Y", &env)?, None);
    Ok(())
}
