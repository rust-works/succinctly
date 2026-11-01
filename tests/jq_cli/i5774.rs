//! #5774: a `reduce`/`foreach` body that reads a container named argument shares the value
//! where it is read through (`$d[$i].age`, `$d | length`, `$d[1:]`) instead of rebuilding the
//! whole literal at every step (50 steps over 0.7 MB of `--slurpfile` was 417 ms, jq 9 ms). The
//! rows pin that a fold answers as the literal did: a read, a bare `$d` appended or stored, a
//! binding of the same name and a write all agree with jq 1.7.1 (`/usr/bin/jq`), which the
//! expected outputs are captured from.

use anyhow::Result;

use crate::run_jq_full;

const D: &str = "[{\"id\":1,\"age\":30,\"t\":[1,2]},{\"id\":2,\"age\":41,\"t\":[3]},{\"id\":3,\"age\":25,\"t\":[]}]";

/// `(filter, stdout)` over `--argjson d D`, every one exiting 0 under `-nc`.
const ROWS: &[(&str, &str)] = &[
    ("reduce range(3) as $i (0; . + $d[$i].age)", "96\n"),
    ("reduce range(3) as $i (0; . + $d[0].age)", "90\n"),
    (
        "[foreach range(3) as $i (0; . + $d[$i].age)]",
        "[30,71,96]\n",
    ),
    ("reduce range(3) as $i ([]; . + [$d[$i].id])", "[1,2,3]\n"),
    ("reduce range(3) as $i ([]; . + $d[$i].t)", "[1,2,3]\n"),
    ("reduce $d[] as $r (0; . + $r.age)", "96\n"),
    ("reduce range(3) as $i (0; . + ($d | length))", "9\n"),
    ("reduce range(3) as $i (0; . + $d[1:][0].age)", "123\n"),
    (
        "reduce range(3) as $i ({}; .[\"k\\($i)\"] = $d[$i].age)",
        "{\"k0\":30,\"k1\":41,\"k2\":25}\n",
    ),
    ("reduce range(3) as $i ([]; . + [$d]) | length", "3\n"),
    ("reduce range(3) as $i ($d; .) | length", "3\n"),
    ("[foreach range(3) as $i ($d[0]; .; .id)]", "[1,1,1]\n"),
    ("reduce range(3) as $i (0; . + ($d[$i] | .age))", "96\n"),
    ("reduce range(2) as $i (0; . + ([5] as $d | $d[0]))", "10\n"),
    (
        "reduce range(3) as $i (0; . + ($d | map(.age) | add))",
        "288\n",
    ),
    (
        "reduce range(3) as $i (0; . + ($d | has(1) | if . then 1 else 0 end))",
        "3\n",
    ),
    ("reduce range(3) as $i (0; . + ($d[$i].t | length))", "3\n"),
    (
        "reduce range(2) as $i ([]; . + ($d[0] | keys))",
        "[\"age\",\"id\",\"t\",\"age\",\"id\",\"t\"]\n",
    ),
    (
        "reduce range(3) as $i (0; if $d[$i] == $d[$i] then . + 1 else . end)",
        "3\n",
    ),
    (
        "reduce range(3) as $i ([]; . + [$d[$i]]) | .[1].age",
        "41\n",
    ),
    ("reduce range(3) as $i (null; $d[$i].t)", "[]\n"),
    ("[foreach range(3) as $i (null; $d[$i]; .id)]", "[1,2,3]\n"),
    (
        "reduce range(4) as $i (0; . + ($d[$i].age // 100))",
        "196\n",
    ),
    (
        "reduce range(3) as $i (0; . + ([$d[$i].t[]?] | length))",
        "3\n",
    ),
];

#[test]
fn a_fold_that_reads_a_container_named_argument_answers_as_jq_5774() -> Result<()> {
    for (filter, expected) in ROWS {
        let (stdout, stderr, code) = run_jq_full(&["-nc", "--argjson", "d", D, filter], None)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{filter}: {stderr}"
        );
    }
    Ok(())
}

/// `--slurpfile` is a container argument too: the same fold over a slurped file.
#[test]
fn a_fold_over_a_slurped_file_answers_as_jq_5774() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("d.json");
    std::fs::write(&file, format!("{D}\n"))?;
    let file = file.to_str().unwrap();
    for (filter, expected) in [
        ("reduce range(3) as $i (0; . + $d[0][$i].age)", "96\n"),
        ("reduce $d[0][] as $r (0; . + $r.id)", "6\n"),
        ("reduce range(2) as $i (0; . + ($d | length))", "2\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", "--slurpfile", "d", file, filter], None)?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}: {stderr}");
    }
    Ok(())
}
