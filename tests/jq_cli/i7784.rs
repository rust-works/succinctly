//! #7784: `%+` in `strftime`/`strflocaltime` and `strptime`. macOS's libc expands it to the
//! locale's `date_fmt` (`%a %b %e %H:%M:%S %Z %Y` in C) both ways, so jq on macOS prints the
//! localized form and reads it back; `strptime("%+")` never matched here, and `%+` was printed in
//! the C layout under every locale. Every row was captured from jq 1.7.1 on macOS (`/usr/bin/jq`,
//! the same in jq 1.8.2). A glibc jq prints `%+` as written and cannot read it; off macOS
//! `succinctly jq` keeps the C layout both ways, so the localized rows run on macOS only.

use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

fn run(program: &str, env: &[(&str, &str)]) -> Result<String> {
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(succinctly_bin());
            command
                .env_remove("LANG")
                .env_remove("LC_TIME")
                .env_remove("LC_ALL")
                .envs(env.iter().copied())
                .args(["jq", "-nc", program]);
            command
        },
        None,
    )?;
    assert_eq!(code, 0, "{program} {env:?}");
    Ok(String::from_utf8(output.stdout)?)
}

/// Each format at three instants: what it prints and what `strptime` with the same format and
/// `mktime` make of that.
const ROUND_TRIP: &str = "[0,1700000000,1720000000] | map(. as $t | strflocaltime($f) as $o \
                          | [$t, $o, (try ($o | strptime($f) | mktime) catch \"ERR\")])";

fn round_trip(f: &str, env: &[(&str, &str)]) -> Result<String> {
    run(&format!("\"{f}\" as $f | {ROUND_TRIP}"), env)
}

#[test]
fn percent_plus_reads_back_in_the_c_locale_7784() -> Result<()> {
    let c = [("TZ", "UTC"), ("LC_ALL", "C")];
    assert_eq!(
        round_trip("%+", &c)?,
        "[[0,\"Thu Jan  1 00:00:00 UTC 1970\",0],[1700000000,\"Tue Nov 14 22:13:20 UTC 2023\",1700000000],[1720000000,\"Wed Jul  3 09:46:40 UTC 2024\",1720000000]]\n"
    );
    // A `+` followed by white space, a `%` or the end is the conversion; one before a specifier
    // is a flag (`%+Y`).
    assert_eq!(
        round_trip("%+ x", &c)?,
        "[[0,\"Thu Jan  1 00:00:00 UTC 1970 x\",0],[1700000000,\"Tue Nov 14 22:13:20 UTC 2023 x\",1700000000],[1720000000,\"Wed Jul  3 09:46:40 UTC 2024 x\",1720000000]]\n"
    );
    assert_eq!(
        round_trip("x%+", &c)?,
        "[[0,\"xThu Jan  1 00:00:00 UTC 1970\",0],[1700000000,\"xTue Nov 14 22:13:20 UTC 2023\",1700000000],[1720000000,\"xWed Jul  3 09:46:40 UTC 2024\",1720000000]]\n"
    );
    assert_eq!(
        round_trip("%+%%", &c)?,
        "[[0,\"Thu Jan  1 00:00:00 UTC 1970%\",0],[1700000000,\"Tue Nov 14 22:13:20 UTC 2023%\",1700000000],[1720000000,\"Wed Jul  3 09:46:40 UTC 2024%\",1720000000]]\n"
    );
    assert_eq!(
        round_trip("%E+", &c)?,
        "[[0,\"Thu Jan  1 00:00:00 UTC 1970\",0],[1700000000,\"Tue Nov 14 22:13:20 UTC 2023\",1700000000],[1720000000,\"Wed Jul  3 09:46:40 UTC 2024\",1720000000]]\n"
    );
    assert_eq!(run(r#""2015" | strptime("%+Y") | .[0]"#, &c)?, "2015\n");
    Ok(())
}

#[cfg(target_vendor = "apple")]
fn installed(name: &str) -> bool {
    std::path::Path::new("/usr/share/locale")
        .join(name)
        .join("LC_TIME")
        .exists()
}

/// The localized layout, printed and read back (`Europe/Berlin`; `%Z` is the zone's, and the
/// summer instant is `CEST` here and in jq 1.8.2, where jq 1.7.1 says `CET`, #3054).
#[cfg(target_vendor = "apple")]
#[test]
fn percent_plus_is_the_locales_date_fmt_on_macos_7784() -> Result<()> {
    for (locale, want) in [
        (
            "de_DE.UTF-8",
            "[[0,\"Do.  1 Jan. 1970 01:00:00 CET\",3600],[1700000000,\"Di. 14 Nov. 2023 23:13:20 CET\",1700003600],[1720000000,\"Mi.  3 Juli 2024 11:46:40 CEST\",1720007200]]\n",
        ),
        (
            "fr_FR.UTF-8",
            "[[0,\"jeu.  1 janv. 1970 01:00:00 CET\",3600],[1700000000,\"mar. 14 nov. 2023 23:13:20 CET\",1700003600],[1720000000,\"mer.  3 juil. 2024 11:46:40 CEST\",1720007200]]\n",
        ),
        (
            "ja_JP.UTF-8",
            "[[0,\"1970年 1月 1日 木曜日 01時00分00秒 CET\",3600],[1700000000,\"2023年 11月14日 火曜日 23時13分20秒 CET\",1700003600],[1720000000,\"2024年 7月 3日 水曜日 11時46分40秒 CEST\",1720007200]]\n",
        ),
    ] {
        if !installed(locale) {
            eprintln!("skipping {locale}: not installed");
            continue;
        }
        let env = [("TZ", "Europe/Berlin"), ("LC_ALL", locale)];
        assert_eq!(round_trip("%+", &env)?, want, "{locale}");
    }
    Ok(())
}

/// `%v` takes the locale's month abbreviation on macOS (#7784), as jq prints it there.
#[cfg(target_vendor = "apple")]
#[test]
fn percent_v_names_the_locales_month_on_macos_7784() -> Result<()> {
    for (locale, want) in [
        (
            "de_DE.UTF-8",
            "[\" 1-Jan.-1970\",\"14-Nov.-2023\",\" 3-Juli-2024\"]\n",
        ),
        (
            "cs_CZ.UTF-8",
            "[\" 1-led-1970\",\"14-lis-2023\",\" 3-čvc-2024\"]\n",
        ),
    ] {
        if !installed(locale) {
            eprintln!("skipping {locale}: not installed");
            continue;
        }
        let env = [("TZ", "UTC"), ("LC_ALL", locale)];
        assert_eq!(
            run("[0,1700000000,1720000000] | map(strftime(\"%v\"))", &env)?,
            want,
            "{locale}"
        );
    }
    Ok(())
}
