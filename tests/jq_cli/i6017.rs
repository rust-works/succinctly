//! #6017, #7501: `--indent` and `-L` take their value as jq does. `--indent` reads jq 1.8's rule
//! (#7501, upstream fix jqlang/jq#3195): an optional sign and ASCII digits, nothing else, in -1 to 7
//! (`-1` is a tab); anything else (empty, white space, `3x`, `3.0`, `0x2`, `abc`, `--`, a value past
//! a `long`) is refused with exit 2. jq 1.7.1 read it with C's `atoi` (#6017) and ran those. `-L ""`
//! is a search-path element, not a missing value. Expectations are what jq 1.8.2
//! (`/opt/homebrew/bin/jq`) printed (stdout and exit status; the error wording is clap's here),
//! except the values that read as 0: jq 1.8 also stopped printing `--indent 0` compactly (#3232,
//! "Fix `--indent 0` implicitly enabling `--compact-output`"), which is not adopted here, so those
//! rows keep jq 1.7.1's output.

use anyhow::Result;

use crate::run_jq_full;

/// `(arguments, stdin, expected stdout, expected exit status)`.
#[test]
fn the_option_values_are_read_as_jq_reads_them() -> Result<()> {
    let rows: &[(&[&str], &str, &str, i32)] = &[
        (&["--indent", "", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "0", "."], "{\"a\":[1]}", "{\"a\":[1]}\n", 0),
        (&["--indent", "1", "."], "{\"a\":[1]}", "{\n \"a\": [\n  1\n ]\n}\n", 0),
        (&["--indent", "2", "."], "{\"a\":[1]}", "{\n  \"a\": [\n    1\n  ]\n}\n", 0),
        (&["--indent", "7", "."], "{\"a\":[1]}", "{\n       \"a\": [\n              1\n       ]\n}\n", 0),
        (&["--indent", "8", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "-1", "."], "{\"a\":[1]}", "{\n\t\"a\": [\n\t\t1\n\t]\n}\n", 0),
        (&["--indent", "-2", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "+2", "."], "{\"a\":[1]}", "{\n  \"a\": [\n    1\n  ]\n}\n", 0),
        (&["--indent", " 2", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "2 ", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "3x", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "x3", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "abc", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "2.5", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "1e1", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "007", "."], "{\"a\":[1]}", "{\n       \"a\": [\n              1\n       ]\n}\n", 0),
        (&["--indent", "0x2", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "99999999999", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "-99999999999", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "99999999999999999999", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "-99999999999999999999", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "4294967295", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "4294967296", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "4294967303", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "-4294967295", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "\t3", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", " -1", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "+-3", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "-0", "."], "{\"a\":[1]}", "{\"a\":[1]}\n", 0),
        (&["--indent", "3", "-c", "."], "{\"a\":[1]}", "{\"a\":[1]}\n", 0),
        (&["-c", "--indent", "3", "."], "{\"a\":[1]}", "{\n   \"a\": [\n      1\n   ]\n}\n", 0),
        (&["--tab", "--indent", "2", "."], "{\"a\":[1]}", "{\n  \"a\": [\n    1\n  ]\n}\n", 0),
        (&["--indent", "2", "--tab", "."], "{\"a\":[1]}", "{\n\t\"a\": [\n\t\t1\n\t]\n}\n", 0),
        (&["--indent", "-1", "--indent", "2", "."], "{\"a\":[1]}", "{\n  \"a\": [\n    1\n  ]\n}\n", 0),
        (&["--indent", "2", "--indent", "-1", "."], "{\"a\":[1]}", "{\n\t\"a\": [\n\t\t1\n\t]\n}\n", 0),
        (&["--indent", "-1", "-c", "."], "{\"a\":[1]}", "{\"a\":[1]}\n", 0),
        (&["-C", "--indent", "-1", "."], "{\"a\":[1]}", "[1;39m{\n\t[0m[1;34m\"a\"[0m[1;39m: [0m[1;39m[\n\t\t[0;39m1[0m[1;39m\n\t[1;39m][0m[1;39m\n[1;39m}[0m\n", 0),
        (&["--indent", "-1", "--seq", "."], "{\"a\":[1]}", "", 0),
        (&["--indent", "-1", "--stream", "."], "{\"a\":[1]}", "[\n\t[\n\t\t\"a\",\n\t\t0\n\t],\n\t1\n]\n[\n\t[\n\t\t\"a\",\n\t\t0\n\t]\n]\n[\n\t[\n\t\t\"a\"\n\t]\n]\n", 0),
        (&["--indent", "-1", "-s", "."], "{\"a\":[1]}", "[\n\t{\n\t\t\"a\": [\n\t\t\t1\n\t\t]\n\t}\n]\n", 0),
        (&["--indent", "-1", "-r", ".a"], "{\"a\":[1]}", "[\n\t1\n]\n", 0),
        (&["--indent", "--", "."], "{\"a\":[1]}", "", 2),
        (&["--indent"], "{\"a\":[1]}", "", 2),
        (&["-L", "", "-n", "1"], "", "1\n", 0),
        (&["-L", "", "-n", "import \"nope\" as n; 1"], "", "", 3),
        (&["-L", "", "-nc", "get_search_list"], "", "[\"\"]\n", 0),
        (&["--indent", "-c", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "--tab", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "-C", "."], "{\"a\":[1]}", "", 2),
        (&["--indent", "1", "--indent", "-c", "."], "{\"a\":[1]}", "", 2),
    ];
    for &(args, stdin, want_out, want_code) in rows {
        let (out, _, code) = run_jq_full(args, Some(stdin))?;
        assert_eq!(
            (out.as_str(), code),
            (want_out, want_code),
            "jq {args:?} <<< {stdin:?}"
        );
    }
    Ok(())
}
