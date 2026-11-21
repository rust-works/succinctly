//! #7530: a module cycle is `circular import of <path>`, as jq 1.8.2 words it (CVE-2026-44777).
//! jq 1.7.1 recurses until SIGSEGV on every row below, so 1.8.2 is the only reference. Every
//! expected text was captured from jq 1.8.2 (`/opt/homebrew/bin/jq -L . -nc PROGRAM`) in a
//! directory holding the same files; `<D>` stands for that directory's canonical path.

use anyhow::Result;
use std::process::Command;

use crate::{spawn_with_signal_retry, succinctly_bin};

const MODULES: &[(&str, &str)] = &[
    ("cycle_a.jq", "import \"cycle_b\" as b;\ndef f: null;\n"),
    ("cycle_b.jq", "import \"cycle_a\" as a;\ndef f: null;\n"),
    (
        "cycle_self.jq",
        "import \"cycle_self\" as s;\ndef f: null;\n",
    ),
    ("ic_a.jq", "include \"ic_b\";\ndef a: 1;\n"),
    ("ic_b.jq", "include \"ic_a\";\ndef b: 2;\n"),
    ("m1.jq", "include \"m2\";\ndef m1: 1;\n"),
    ("m2.jq", "include \"m3\";\ndef m2: 1;\n"),
    ("m3.jq", "include \"m1\";\ndef m3: 1;\n"),
    ("alia.jq", "include \"./alia\";\ndef x: 1;\n"),
    ("top.jq", "import \"m1\" as m1;\ndef t: 1;\n"),
    ("d1.json", "{\"a\":1}\n"),
    ("badd.json", "not json"),
    ("p1.jq", "include \"p2\";\ninclude \"p3\";\ndef p1: 1;\n"),
    ("p2.jq", "include \"p1\";\ndef p2: 1;\n"),
    ("p3.jq", "include \"p1\";\ndef p3: 1;\n"),
    (
        "q1.jq",
        "include \"q2\";\nimport \"badd\" as $b;\ninclude \"q4\";\ndef q1: 1;\n",
    ),
    ("q2.jq", "include \"q1\";\ninclude \"nx\";\ndef q2: 1;\n"),
    ("q4.jq", "include \"q1\";\ndef q4: 1;\n"),
    (
        "datamod.jq",
        "import \"d1\" as $d;\nimport \"d1\" as $e;\ndef f: $d;\n",
    ),
];

#[test]
fn test_circular_import_wording_matches_jq_182_7530() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let dir = std::fs::canonicalize(temp_dir.path())?;
    for (name, text) in MODULES {
        std::fs::write(dir.join(name), text)?;
    }
    let d = dir.display().to_string();
    let circular = |file: &str| format!("jq: error: circular import of <D>/{file}\n\n");
    let not_found = |name: &str| format!("jq: error: module not found: {name}\n\n");
    // The detail after the file name is succinctly's own wording (`limitations.md`), so it is
    // blanked on both sides.
    let data_fail = "jq: error loading data file <D>/badd.json: <detail>\n\n";
    let rows: Vec<(&str, String)> = vec![
        // Two- and three-module cycles, `import` and `include`, self-import.
        (
            r#"import "cycle_a" as a; null"#,
            circular("cycle_a.jq") + "jq: 1 compile error\n",
        ),
        (
            r#"import "cycle_self" as s; null"#,
            circular("cycle_self.jq") + "jq: 1 compile error\n",
        ),
        (
            r#"include "ic_a"; a"#,
            circular("ic_a.jq") + "jq: 1 compile error\n",
        ),
        (
            r#"include "m1"; m1"#,
            circular("m1.jq") + "jq: 1 compile error\n",
        ),
        // A module that merely leads to the cycle is not named.
        (
            r#"import "top" as t; null"#,
            circular("m1.jq") + "jq: 1 compile error\n",
        ),
        // The relative spelling names the resolved file.
        (
            r#"include "alia"; x"#,
            circular("alia.jq") + "jq: 1 compile error\n",
        ),
        // Last-declared first, each cycle counted for its own level.
        (
            r#"import "cycle_a" as a; import "m1" as m; null"#,
            circular("m1.jq") + &circular("cycle_a.jq") + "jq: 2 compile errors\n",
        ),
        // A module reached once is not a cycle the second time: `p3` leads to `p1`, which
        // includes `p3` again (the cycle), and `p2` then finds `p1` already loaded.
        (
            r#"include "p2"; include "p3"; 1"#,
            circular("p3.jq") + "jq: 1 compile error\n",
        ),
        // A data import that is fine, and a missing module before and after the cycle.
        (
            r#"import "d1" as $d; import "cycle_a" as a; null"#,
            circular("cycle_a.jq") + "jq: 1 compile error\n",
        ),
        (
            r#"import "nonexist" as n; import "cycle_a" as a; null"#,
            circular("cycle_a.jq") + &not_found("nonexist") + "jq: 1 compile error\n",
        ),
        // A missing module declared last is walked first and stops the level.
        (
            r#"import "cycle_a" as a; import "nonexist" as n; null"#,
            not_found("nonexist") + "jq: 1 compile error\n",
        ),
        // A data import that fails, around a cycle; a module with a failing data import and a
        // missing module inside the cycle.
        (
            r#"import "badd" as $d; import "cycle_a" as a; null"#,
            circular("cycle_a.jq") + data_fail + "jq: 2 compile errors\n",
        ),
        (
            r#"import "cycle_a" as a; import "badd" as $d; null"#,
            data_fail.to_string() + &circular("cycle_a.jq") + "jq: 2 compile errors\n",
        ),
        (
            r#"include "q1"; 1"#,
            circular("q1.jq") + data_fail + &not_found("nx") + "jq: 3 compile errors\n",
        ),
    ];
    for (program, expected) in rows {
        let (output, code) = spawn_with_signal_retry(
            || {
                let mut command = Command::new(succinctly_bin());
                command.args(["jq", "-L"]).arg(&dir).args(["-nc", program]);
                command
            },
            None,
        )?;
        let stderr = String::from_utf8(output.stderr)?
            .replace(&d, "<D>")
            .lines()
            .map(|line| match line.split_once(".json: ") {
                Some((head, _)) if head.starts_with("jq: error loading data file") => {
                    format!("{head}.json: <detail>\n")
                }
                _ => format!("{line}\n"),
            })
            .collect::<String>();
        assert_eq!(stderr, expected, "{program}");
        assert_eq!(code, 3, "{program}");
        assert_eq!(output.stdout, b"", "{program}");
    }
    Ok(())
}
