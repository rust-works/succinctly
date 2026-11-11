//! #6924: `succinctly yq -o lua`, with `--lua-prefix`, `--lua-suffix`, `--lua-globals` and
//! `--lua-unquoted`. Every answer was captured from yq v4.53.3; a row is kept only when both agree on
//! stdout and the exit status. Left out because they still differ (see `docs/compliance/yq/limitations.md`):
//! a literal, folded or single-quoted string, a key that is not a string, a comment, a timestamp or
//! `!!binary` (yq refuses them), and the YAML 1.1 spellings `1_000` and `0b11`.

use anyhow::Result;

use crate::run_yq_stdin;

/// (flags, input, yq's stdout, yq's exit status).
const ROWS: &[(&[&str], &str, &str, i32)] = &[
    (&[], "a: 1\nb: [x, y]\nc: {d: e, f: true, g: null, h: 1.5, i: \"q\\\"r\"}\n", "return {\n\t[\"a\"] = 1;\n\t[\"b\"] = {\n\t\t\"x\",\n\t\t\"y\",\n\t};\n\t[\"c\"] = {\n\t\t[\"d\"] = \"e\";\n\t\t[\"f\"] = true;\n\t\t[\"g\"] = nil;\n\t\t[\"h\"] = 1.5;\n\t\t[\"i\"] = \"q\\\"r\";\n\t};\n};\n", 0),
    (&[], "hello\n", "return \"hello\";\n", 0),
    (&[], "null\n", "return nil;\n", 0),
    (&[], "[]\n", "return {};\n", 0),
    (&[], "{}\n", "return {};\n", 0),
    (&[], "a: {}\nb: []\n", "return {\n\t[\"a\"] = {};\n\t[\"b\"] = {};\n};\n", 0),
    (&[], "[1, [2, 3], {a: 1}]\n", "return {\n\t1,\n\t{\n\t\t2,\n\t\t3,\n\t},\n\t{\n\t\t[\"a\"] = 1;\n\t},\n};\n", 0),
    (&[], "a: \"x\\ny\"\nb: \"t\\tab\"\nc: \"back\\\\slash\"\nd: \"\"\ne: \"é\"\nf: \"\\u0001\\u007f \\\"q\\\" 's'\"\ng: \"\\a\\b\\v\\f\\r\"\n", "return {\n\t[\"a\"] = \"x\\ny\";\n\t[\"b\"] = \"t\\tab\";\n\t[\"c\"] = \"back\\\\slash\";\n\t[\"d\"] = \"\";\n\t[\"e\"] = \"é\";\n\t[\"f\"] = \"\\001\\127 \\\"q\\\" \\'s\\'\";\n\t[\"g\"] = \"\\a\\b\\v\\f\\r\";\n};\n", 0),
    (&[], "a: [0x1F, 0o17, 1e3, .inf, -.inf, .nan, True, FALSE, +5, 1.50, 100.0, 1E5, 010]\n", "return {\n\t[\"a\"] = {\n\t\t0x1f,\n\t\t15,\n\t\t1e3,\n\t\t(1/0),\n\t\t(-1/0),\n\t\t(0/0),\n\t\ttrue,\n\t\tfalse,\n\t\t+5,\n\t\t1.50,\n\t\t100.0,\n\t\t1E5,\n\t\t010,\n\t};\n};\n", 0),
    (&[], "a: ~\nb: null\n", "return {\n\t[\"a\"] = nil;\n\t[\"b\"] = nil;\n};\n", 0),
    (&[], "---\na: 1\n---\nb: 2\n", "return {\n\t[\"a\"] = 1;\n};\nreturn {\n\t[\"b\"] = 2;\n};\n", 0),
    (&["--lua-unquoted"], "a-b: 1\nc: 2\n_d: 3\n9x: 4\nend: 5\nnil: 6\n", "return {\n\t[\"a-b\"] = 1;\n\tc = 2;\n\t_d = 3;\n\t[\"9x\"] = 4;\n\t[\"end\"] = 5;\n\t[\"nil\"] = 6;\n};\n", 0),
    (&["--lua-unquoted"], "\"\": 1\nx_y: 2\n", "return {\n\t = 1;\n\tx_y = 2;\n};\n", 0),
    (&["--lua-globals"], "a: 1\nb: {c: 2}\nd: [x]\n\"k:\": 3\nend: 4\n", "a = 1;\nb = {\n\t[\"c\"] = 2;\n};\nd = {\n\t\"x\",\n};\n_ENV[\"k:\"] = 3;\n_ENV[\"end\"] = 4;\n", 0),
    (&["--lua-globals"], "[1]\n", "", 1),
    (&["--lua-globals"], "hello\n", "", 1),
    (&["--lua-prefix=local x = "], "a: 1\n", "local x = {\n\t[\"a\"] = 1;\n};\n", 0),
    (&["--lua-prefix=local x = ", "--lua-suffix=\\n-- end\\n"], "a: 1\n", "local x = {\n\t[\"a\"] = 1;\n}\n-- end\n", 0),
    (&["--lua-prefix=a\\tb\\\\c\\'d\\\"e\\x", "--lua-suffix=;"], "[1]\n", "a\tb\\c'd\"e\\x{\n\t1,\n};", 0),
    (&["--lua-unquoted", "--lua-prefix=", "--lua-suffix="], "a: 1\n", "{\n\ta = 1;\n}", 0),
    (&["--lua-globals", "--lua-prefix=X", "--lua-suffix=Y"], "a: 1\n", "a = 1;\n", 0),
    (&["--lua-prefix=\\x41\\0\\"], "[1]\n", "\\x41\\0\\{\n\t1,\n};\n", 0),
    (&["--lua-globals"], "---\na: 1\n---\nb: 2\n", "a = 1;\nb = 2;\n", 0),
    (&["--lua-unquoted"], "é: 1\nA1: 2\n_ENV: 3\n", "return {\n\t[\"é\"] = 1;\n\tA1 = 2;\n\t_ENV = 3;\n};\n", 0),
];

#[test]
fn lua_rows_match_yq_6924() -> Result<()> {
    for (flags, input, expected, code) in ROWS {
        let mut args = vec!["-o=lua"];
        args.extend(flags.iter().copied());
        let (stdout, status) = run_yq_stdin(".", input, &args)?;
        assert_eq!(
            (stdout.as_str(), status),
            (*expected, *code),
            "-o=lua {flags:?} over {input:?}"
        );
    }
    Ok(())
}

#[test]
fn lua_short_name_6924() -> Result<()> {
    let one = run_yq_stdin(".", "a: 1\n", &["-o=l"])?;
    let other = run_yq_stdin(".", "a: 1\n", &["-o=lua"])?;
    assert_eq!(one, other);
    Ok(())
}
