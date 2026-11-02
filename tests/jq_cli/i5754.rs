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
