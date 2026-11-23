//! #7790: `strftime("%OB")` is the locale's alternative month name in macOS's libc, the nominative
//! one a month standing alone takes, where `%B` is the genitive of a date (`leden` and `ledna` under
//! `cs_CZ`). jq prints it; `succinctly jq` dropped the `O` and printed `%B`. `strptime("%OB")` reads
//! those names too (`leden`, not `ledna`), and read the `%B` ones.
//!
//! Each row is, at three instants, `strftime` of `%OB`, `%B`, `strflocaltime("%OB")`, `%Ob`, `%Oh`,
//! `%-OB` and `%EB`, then `mktime` of what `strptime("%OB")` reads from the `%OB` and from the `%B`
//! name (or `ERR`), captured from jq 1.7.1 on macOS (`/usr/bin/jq`, `TZ=UTC`; jq 1.8.2 prints the
//! same). The localized rows run on macOS only: a glibc jq was not captured.

use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

const PROGRAM: &str = "[0,1700000000,1720000000] | map(. as $t | [strftime(\"%OB\"), \
                       strftime(\"%B\"), strflocaltime(\"%OB\"), strftime(\"%Ob\"), \
                       strftime(\"%Oh\"), strftime(\"%-OB\"), strftime(\"%EB\"), \
                       (strftime(\"%OB\") | try (strptime(\"%OB\") | mktime) catch \"ERR\"), \
                       (strftime(\"%B\") | try (strptime(\"%OB\") | mktime) catch \"ERR\")])";

fn run(env: &[(&str, &str)]) -> Result<String> {
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(succinctly_bin());
            command
                .env_remove("LANG")
                .env_remove("LC_TIME")
                .env_remove("LC_ALL")
                .envs(env.iter().copied())
                .args(["jq", "-nc", PROGRAM]);
            command
        },
        None,
    )?;
    assert_eq!(code, 0, "{env:?}");
    Ok(String::from_utf8(output.stdout)?)
}

#[test]
fn alternative_month_names_in_the_c_locale_7790() -> Result<()> {
    assert_eq!(
        run(&[("TZ", "UTC"), ("LC_ALL", "C")])?,
        "[[\"January\",\"January\",\"January\",\"Jan\",\"Jan\",\"January\",\"January\",\"ERR\",\"ERR\"],[\"November\",\"November\",\"November\",\"Nov\",\"Nov\",\"November\",\"November\",-2182809600,-2182809600],[\"July\",\"July\",\"July\",\"Jul\",\"Jul\",\"July\",\"July\",-2193436800,-2193436800]]\n"
    );
    Ok(())
}

#[cfg(target_vendor = "apple")]
fn installed(name: &str) -> bool {
    std::path::Path::new("/usr/share/locale")
        .join(name)
        .join("LC_TIME")
        .exists()
}

/// The nine locales macOS's libc has alternative month names for, then four where `%OB` is the
/// full name (`hr_HR`, `fi_FI` and `ca_ES` among them, whose `%B` is a genitive too).
#[cfg(target_vendor = "apple")]
const LOCALIZED: &[(&str, &str)] = &[
    (
        "be_BY.UTF-8",
        "[[\"студзень\",\"студзеня\",\"студзень\",\"сту\",\"сту\",\"студзень\",\"студзеня\",\"ERR\",\"ERR\"],[\"лістапад\",\"лістапада\",\"лістапад\",\"ліс\",\"ліс\",\"лістапад\",\"лістапада\",-2182809600,\"ERR\"],[\"ліпень\",\"ліпеня\",\"ліпень\",\"ліп\",\"ліп\",\"ліпень\",\"ліпеня\",-2193436800,\"ERR\"]]",
    ),
    (
        "cs_CZ.UTF-8",
        "[[\"leden\",\"ledna\",\"leden\",\"led\",\"led\",\"leden\",\"ledna\",\"ERR\",\"ERR\"],[\"listopad\",\"listopadu\",\"listopad\",\"lis\",\"lis\",\"listopad\",\"listopadu\",-2182809600,\"ERR\"],[\"červenec\",\"července\",\"červenec\",\"čvc\",\"čvc\",\"červenec\",\"července\",\"ERR\",\"ERR\"]]",
    ),
    (
        "el_GR.UTF-8",
        "[[\"Ιανουάριος\",\"Ιανουαρίου\",\"Ιανουάριος\",\"Ιαν\",\"Ιαν\",\"Ιανουάριος\",\"Ιανουαρίου\",\"ERR\",\"ERR\"],[\"Νοέμβριος\",\"Νοεμβρίου\",\"Νοέμβριος\",\"Νοε\",\"Νοε\",\"Νοέμβριος\",\"Νοεμβρίου\",-2182809600,\"ERR\"],[\"Ιούλιος\",\"Ιουλίου\",\"Ιούλιος\",\"Ιουλ\",\"Ιουλ\",\"Ιούλιος\",\"Ιουλίου\",-2193436800,\"ERR\"]]",
    ),
    (
        "kk_KZ.UTF-8",
        "[[\"қантар\",\"қаңтар\",\"қантар\",\"қаң.\",\"қаң.\",\"қантар\",\"қаңтар\",\"ERR\",\"ERR\"],[\"қараша\",\"қараша\",\"қараша\",\"қар.\",\"қар.\",\"қараша\",\"қараша\",-2182809600,-2182809600],[\"шілде\",\"шілде\",\"шілде\",\"шіл.\",\"шіл.\",\"шілде\",\"шілде\",-2193436800,-2193436800]]",
    ),
    (
        "lt_LT.UTF-8",
        "[[\"sausis\",\"sausio\",\"sausis\",\"saus.\",\"saus.\",\"sausis\",\"sausio\",\"ERR\",\"ERR\"],[\"lapkritis\",\"lapkričio\",\"lapkritis\",\"lapkr.\",\"lapkr.\",\"lapkritis\",\"lapkričio\",-2182809600,\"ERR\"],[\"liepa\",\"liepos\",\"liepa\",\"liep.\",\"liep.\",\"liepa\",\"liepos\",-2193436800,\"ERR\"]]",
    ),
    (
        "mn_MN.UTF-8",
        "[[\"Нэгдүгээр сар\",\"Нэгдүгээр сар\",\"Нэгдүгээр сар\",\"1-р сар\",\"1-р сар\",\"Нэгдүгээр сар\",\"Нэгдүгээр сар\",\"ERR\",\"ERR\"],[\"Арван нэгдүгээр сар\",\"Арван нэгдүгээр сар\",\"Арван нэгдүгээр сар\",\"11-р сар\",\"11-р сар\",\"Арван нэгдүгээр сар\",\"Арван нэгдүгээр сар\",-2182809600,-2182809600],[\"Долоодугаар сар\",\"Долдугаар сар\",\"Долоодугаар сар\",\"7-р сар\",\"7-р сар\",\"Долоодугаар сар\",\"Долдугаар сар\",-2193436800,\"ERR\"]]",
    ),
    (
        "pl_PL.UTF-8",
        "[[\"styczeń\",\"stycznia\",\"styczeń\",\"sty\",\"sty\",\"styczeń\",\"stycznia\",\"ERR\",\"ERR\"],[\"listopad\",\"listopada\",\"listopad\",\"lis\",\"lis\",\"listopad\",\"listopada\",-2182809600,\"ERR\"],[\"lipiec\",\"lipca\",\"lipiec\",\"lip\",\"lip\",\"lipiec\",\"lipca\",-2193436800,\"ERR\"]]",
    ),
    (
        "ru_RU.UTF-8",
        "[[\"январь\",\"января\",\"январь\",\"янв.\",\"янв.\",\"январь\",\"января\",\"ERR\",\"ERR\"],[\"ноябрь\",\"ноября\",\"ноябрь\",\"нояб.\",\"нояб.\",\"ноябрь\",\"ноября\",-2182809600,\"ERR\"],[\"июль\",\"июля\",\"июль\",\"июля\",\"июля\",\"июль\",\"июля\",-2193436800,\"ERR\"]]",
    ),
    (
        "uk_UA.UTF-8",
        "[[\"січень\",\"січня\",\"січень\",\"січ.\",\"січ.\",\"січень\",\"січня\",\"ERR\",\"ERR\"],[\"листопад\",\"листопада\",\"листопад\",\"лист.\",\"лист.\",\"листопад\",\"листопада\",-2182809600,\"ERR\"],[\"липень\",\"липня\",\"липень\",\"лип.\",\"лип.\",\"липень\",\"липня\",-2193436800,\"ERR\"]]",
    ),
    (
        "hr_HR.UTF-8",
        "[[\"siječnja\",\"siječnja\",\"siječnja\",\"sij\",\"sij\",\"siječnja\",\"siječnja\",\"ERR\",\"ERR\"],[\"studenoga\",\"studenoga\",\"studenoga\",\"stu\",\"stu\",\"studenoga\",\"studenoga\",-2182809600,-2182809600],[\"srpnja\",\"srpnja\",\"srpnja\",\"srp\",\"srp\",\"srpnja\",\"srpnja\",-2193436800,-2193436800]]",
    ),
    (
        "fi_FI.UTF-8",
        "[[\"tammikuuta\",\"tammikuuta\",\"tammikuuta\",\"tammikuuta\",\"tammikuuta\",\"tammikuuta\",\"tammikuuta\",\"ERR\",\"ERR\"],[\"marraskuuta\",\"marraskuuta\",\"marraskuuta\",\"marraskuuta\",\"marraskuuta\",\"marraskuuta\",\"marraskuuta\",-2182809600,-2182809600],[\"heinäkuuta\",\"heinäkuuta\",\"heinäkuuta\",\"heinäkuuta\",\"heinäkuuta\",\"heinäkuuta\",\"heinäkuuta\",-2193436800,-2193436800]]",
    ),
    (
        "ca_ES.UTF-8",
        "[[\"de gener\",\"de gener\",\"de gener\",\"gen.\",\"gen.\",\"de gener\",\"de gener\",\"ERR\",\"ERR\"],[\"de novembre\",\"de novembre\",\"de novembre\",\"nov.\",\"nov.\",\"de novembre\",\"de novembre\",-2182809600,-2182809600],[\"de juliol\",\"de juliol\",\"de juliol\",\"jul.\",\"jul.\",\"de juliol\",\"de juliol\",-2193436800,-2193436800]]",
    ),
    (
        "de_DE.UTF-8",
        "[[\"Januar\",\"Januar\",\"Januar\",\"Jan.\",\"Jan.\",\"Januar\",\"Januar\",\"ERR\",\"ERR\"],[\"November\",\"November\",\"November\",\"Nov.\",\"Nov.\",\"November\",\"November\",-2182809600,-2182809600],[\"Juli\",\"Juli\",\"Juli\",\"Juli\",\"Juli\",\"Juli\",\"Juli\",-2193436800,-2193436800]]",
    ),
];

#[cfg(target_vendor = "apple")]
#[test]
fn percent_ob_is_the_locales_alternative_month_name_on_macos_7790() -> Result<()> {
    for &(locale, want) in LOCALIZED {
        if !installed(locale) {
            eprintln!("skipping {locale}: not installed");
            continue;
        }
        assert_eq!(
            run(&[("TZ", "UTC"), ("LC_ALL", locale)])?,
            format!("{want}\n"),
            "{locale}"
        );
    }
    Ok(())
}
