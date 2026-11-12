//! #7051: yq's flags are Go `pflag` ones, so a repeated value flag takes the last value
//! (`yq -o json -o yaml .` prints YAML), and `-C` forces colour whichever of `-C` and `-M` comes last.
//! `succinctly yq` is a clap program, which refuses a second occurrence of a single-value argument
//! unless it overrides itself, and let `-M` win over `-C`. Every row was captured from yq v4.53.3
//! (`yq FLAGS .` over the given input). The guard that every single-value flag takes its last
//! occurrence is `test_every_single_value_yq_flag_takes_its_last_occurrence_7051` in `main.rs`.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

/// `(input, flags, stdout)`.
const ROWS: &[(&str, &[&str], &str)] = &[
    ("a: [1]\n", &["-o", "json", "-o", "yaml"], "a: [1]\n"),
    (
        "a: [1]\n",
        &["-o", "yaml", "-o", "json", "-I", "0"],
        "{\"a\":[1]}\n",
    ),
    (
        "a: [1]\n",
        &["-I", "4", "-I", "0", "-o", "json"],
        "{\"a\":[1]}\n",
    ),
    (
        "a: [1]\n",
        &["-I", "0", "-I", "4", "-o", "json"],
        "{\n    \"a\": [\n        1\n    ]\n}\n",
    ),
    (
        "a: [1]\n",
        &["-o", "json", "-o", "json", "-I", "0"],
        "{\"a\":[1]}\n",
    ),
    (
        "{\"a\":[1]}\n",
        &["-p", "yaml", "-p", "json", "-o", "yaml"],
        "a:\n  - 1\n",
    ),
    (
        "{\"a\":[1]}\n",
        &["-p", "json", "-p", "yaml", "-o", "json", "-I", "0"],
        "{\"a\":[1]}\n",
    ),
    (
        "a: [1]\n",
        &[
            "--properties-separator",
            ":",
            "--properties-separator",
            "=",
            "-o",
            "props",
        ],
        "a.0=1\n",
    ),
    (
        "a: [1]\n",
        &[
            "--shell-key-separator",
            "-",
            "--shell-key-separator",
            ".",
            "-o",
            "shell",
        ],
        "a.0=1\n",
    ),
    (
        "a: [1]\n",
        &["--lua-prefix", "a", "--lua-prefix", "b", "-o", "lua"],
        "b{\n\t[\"a\"] = {\n\t\t1,\n\t};\n};\n",
    ),
    (
        "a: [1]\n",
        &["--lua-suffix", "a", "--lua-suffix", "b", "-o", "lua"],
        "return {\n\t[\"a\"] = {\n\t\t1,\n\t};\n}b",
    ),
    (
        "- 1\n- 2\n",
        &["--csv-separator", ";", "--csv-separator", "|", "-o", "csv"],
        "1|2\n",
    ),
    (
        "a: [1]\n",
        &[
            "--xml-attribute-prefix",
            "a",
            "--xml-attribute-prefix",
            "b",
            "-o",
            "xml",
        ],
        "<a>1</a>\n",
    ),
    (
        "a: [1]\n",
        &["-C", "-M"],
        "\x1b[36ma\x1b[0m: [\x1b[95m1\x1b[0m]\n",
    ),
    (
        "a: [1]\n",
        &["-M", "-C"],
        "\x1b[36ma\x1b[0m: [\x1b[95m1\x1b[0m]\n",
    ),
    (
        "a: [1]\n",
        &["-C", "-M", "-C"],
        "\x1b[36ma\x1b[0m: [\x1b[95m1\x1b[0m]\n",
    ),
    (
        "a: [1]\n",
        &["-M", "-C", "-M"],
        "\x1b[36ma\x1b[0m: [\x1b[95m1\x1b[0m]\n",
    ),
    ("a: [1]\n", &["-M"], "a: [1]\n"),
    (
        "a: [1]\n",
        &["-C"],
        "\x1b[36ma\x1b[0m: [\x1b[95m1\x1b[0m]\n",
    ),
];

#[test]
fn a_repeated_value_flag_takes_the_last_one_and_c_beats_m_7051() -> Result<()> {
    for &(input, flags, want) in ROWS {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", input, flags)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{flags:?}: {stderr}");
    }
    Ok(())
}
