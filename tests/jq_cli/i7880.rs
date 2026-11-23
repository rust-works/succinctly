//! #7880: an argument's value is data, and whether it holds `__jq`, the key of jq's label value,
//! decides nothing for a program that cannot raise it. For a program that holds an `error`, a
//! value that holds `__jq` (`--arg`, `--argjson`, `--slurpfile`, `--rawfile`, or a positional
//! argument read through `$ARGS`) is the label value it would be if the program spelled it:
//! jq's `label`, and the labels of `first`, `limit` and `isempty`, stop on it. Before #7880 that
//! held only where the value happened to be substituted into the program as a literal, and never
//! for the environment (`$ENV`, `env`), which a program that raises now counts the same way, so
//! `--argjson v '{"__jq":0}'` with `[limit(3; 1, error($v), 2)]` raised the object (exit 5) where
//! jq answers `[1]`: the `was_wrong` tests hold every such row of the matrix this was found with.
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`), and jq 1.8.2 answers each the same.

use anyhow::Result;

use crate::{run_jq_full, spawn_jq_with_env};

/// `(args, stdout, stderr, exit)`, as jq 1.7.1 answers.
type Row = (&'static [&'static str], &'static str, &'static str, i32);

/// `(flag, file contents, program, stdout, stderr, exit)`: the program is run as
/// `-nc <flag> <name> <file> <program>`, the name being `v` for `--slurpfile`, and `s` or `k` for
/// `--rawfile` (whichever the program reads).
type FileRow = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    i32,
);

/// `(variable, value, program, stdout, stderr, exit)`: the program is run as `-nc <program>` with
/// the environment variable set.
type EnvRow = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    i32,
);

fn check_env(rows: &[EnvRow]) -> Result<()> {
    for &(variable, value, program, stdout, stderr, code) in rows {
        let (output, status) = spawn_jq_with_env(&["-nc", program], variable, value, None)?;
        assert_eq!(
            (
                String::from_utf8(output.stdout)?.as_str(),
                String::from_utf8(output.stderr)?.as_str(),
                status
            ),
            (stdout, stderr, code),
            "{variable}={value} {program}"
        );
    }
    Ok(())
}

fn check(rows: &[Row]) -> Result<()> {
    for &(args, stdout, stderr, code) in rows {
        let (out, err, status) = run_jq_full(args, None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (stdout, stderr, code),
            "{args:?}"
        );
    }
    Ok(())
}

fn check_files(rows: &[FileRow]) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("value");
    let path = file.to_str().expect("a UTF-8 temporary path");
    for &(flag, contents, program, stdout, stderr, code) in rows {
        std::fs::write(&file, contents)?;
        let name = if flag == "--slurpfile" {
            "v"
        } else if program.contains("$k") {
            "k"
        } else {
            "s"
        };
        let (out, err, status) = run_jq_full(&["-nc", flag, name, path, program], None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (stdout, stderr, code),
            "{flag} {contents:?} {program}"
        );
    }
    Ok(())
}

/// The rows that answered differently from jq before #7880: the value was spliced as a shared
/// marker, or read through `$ARGS`, and the route walk never saw it.
#[test]
fn test_argument_value_label_sentinel_was_wrong_7880() -> Result<()> {
    check(&[
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":0}"#,
                r"[isempty(error($v))]",
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":0}"#,
                r"[limit(3; 1, error($v), 2)]",
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":0}"#,
                r"[isempty(error($ARGS.named.v))]",
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":0}"#,
                r"[limit(3; 1, error($ARGS.named.v), 2)]",
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"[{"__jq":0}]"#,
                r"[isempty(error($v[0]))]",
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"[{"__jq":0}]"#,
                r"[limit(3; 1, error($v[0]), 2)]",
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jq":0}"#,
                r"[isempty(error(($ARGS.named.s|fromjson)))]",
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jq":0}"#,
                r"[limit(3; 1, error(($ARGS.named.s|fromjson)), 2)]",
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[isempty(error($ARGS.positional[0]))]",
                r"--jsonargs",
                r#"{"__jq":0}"#,
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[limit(3; 1, error($ARGS.positional[0]), 2)]",
                r"--jsonargs",
                r#"{"__jq":0}"#,
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[isempty(error(($ARGS.positional[0]|fromjson)))]",
                r"--args",
                r#"{"__jq":0}"#,
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[limit(3; 1, error(($ARGS.positional[0]|fromjson)), 2)]",
                r"--args",
                r#"{"__jq":0}"#,
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"k",
                r"__jq",
                r"[isempty(error({($ARGS.named.k):0}))]",
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"k",
                r"__jq",
                r"[limit(3; 1, error({($ARGS.named.k):0}), 2)]",
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[isempty(error({($ARGS.positional[0]):0}))]",
                r"--args",
                r"__jq",
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[limit(3; 1, error({($ARGS.positional[0]):0}), 2)]",
                r"--args",
                r"__jq",
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":1}"#,
                r"[limit(3; repeat(1))] | length, [first(error($v))]",
            ],
            r"3
[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":1}"#,
                r"[limit(3; repeat(1))] | length, [first(error($ARGS.named.v))]",
            ],
            r"3
[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"[{"__jq":1}]"#,
                r"[limit(3; repeat(1))] | length, [first(error($v[0]))]",
            ],
            r"3
[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jq":1}"#,
                r"[limit(3; repeat(1))] | length, [first(error(($ARGS.named.s|fromjson)))]",
            ],
            r"3
[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[limit(3; repeat(1))] | length, [first(error($ARGS.positional[0]))]",
                r"--jsonargs",
                r#"{"__jq":1}"#,
            ],
            r"3
[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[limit(3; repeat(1))] | length, [first(error(($ARGS.positional[0]|fromjson)))]",
                r"--args",
                r#"{"__jq":1}"#,
            ],
            r"3
[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"k",
                r"__jq",
                r"[limit(3; repeat(1))] | length, [first(error({($ARGS.named.k):1}))]",
            ],
            r"3
[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[limit(3; repeat(1))] | length, [first(error({($ARGS.positional[0]):1}))]",
                r"--args",
                r"__jq",
            ],
            r"3
[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":0.0}"#,
                r"[isempty(error($v))]",
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":0.0}"#,
                r"[limit(3; 1, error($v), 2)]",
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":0.0}"#,
                r"[isempty(error($ARGS.named.v))]",
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":0.0}"#,
                r"[limit(3; 1, error($ARGS.named.v), 2)]",
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"[{"__jq":0.0}]"#,
                r"[isempty(error($v[0]))]",
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"[{"__jq":0.0}]"#,
                r"[limit(3; 1, error($v[0]), 2)]",
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jq":0.0}"#,
                r"[isempty(error(($ARGS.named.s|fromjson)))]",
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jq":0.0}"#,
                r"[limit(3; 1, error(($ARGS.named.s|fromjson)), 2)]",
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[isempty(error($ARGS.positional[0]))]",
                r"--jsonargs",
                r#"{"__jq":0.0}"#,
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[limit(3; 1, error($ARGS.positional[0]), 2)]",
                r"--jsonargs",
                r#"{"__jq":0.0}"#,
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[isempty(error(($ARGS.positional[0]|fromjson)))]",
                r"--args",
                r#"{"__jq":0.0}"#,
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[limit(3; 1, error(($ARGS.positional[0]|fromjson)), 2)]",
                r"--args",
                r#"{"__jq":0.0}"#,
            ],
            r"[1]
",
            r"",
            0,
        ),
    ])
}

/// The same for a `--slurpfile` value.
#[test]
fn test_argument_file_label_sentinel_was_wrong_7880() -> Result<()> {
    check_files(&[
        (
            r"--slurpfile",
            r#"{"__jq":0}
"#,
            r"[isempty(error($v[0]))]",
            r"[]
",
            r"",
            0,
        ),
        (
            r"--slurpfile",
            r#"{"__jq":0}
"#,
            r"[limit(3; 1, error($v[0]), 2)]",
            r"[1]
",
            r"",
            0,
        ),
        (
            r"--slurpfile",
            r#"{"__jq":0}
"#,
            r"[isempty(error(($v|.[0])))]",
            r"[]
",
            r"",
            0,
        ),
        (
            r"--slurpfile",
            r#"{"__jq":0}
"#,
            r"[limit(3; 1, error(($v|.[0])), 2)]",
            r"[1]
",
            r"",
            0,
        ),
        (
            r"--slurpfile",
            r#"{"__jq":1}
"#,
            r"[limit(3; repeat(1))] | length, [first(error($v[0]))]",
            r"3
[]
",
            r"",
            0,
        ),
        (
            r"--slurpfile",
            r#"{"__jq":1}
"#,
            r"[limit(3; repeat(1))] | length, [first(error(($v|.[0])))]",
            r"3
[]
",
            r"",
            0,
        ),
        (
            r"--slurpfile",
            r#"{"__jq":0.0}
"#,
            r"[isempty(error($v[0]))]",
            r"[]
",
            r"",
            0,
        ),
        (
            r"--slurpfile",
            r#"{"__jq":0.0}
"#,
            r"[limit(3; 1, error($v[0]), 2)]",
            r"[1]
",
            r"",
            0,
        ),
        (
            r"--slurpfile",
            r#"{"__jq":0.0}
"#,
            r"[isempty(error(($v|.[0])))]",
            r"[]
",
            r"",
            0,
        ),
        (
            r"--slurpfile",
            r#"{"__jq":0.0}
"#,
            r"[limit(3; 1, error(($v|.[0])), 2)]",
            r"[1]
",
            r"",
            0,
        ),
    ])
}

/// Rows that already answered as jq does, while the walk read the literal the value was
/// substituted as: they keep their answers now that it does not.
#[test]
fn test_argument_value_label_sentinel_kept_7880() -> Result<()> {
    check(&[
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":0}"#,
                r"[label $f | 1, error($v), 2]",
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":0}"#,
                r"[label $f | first(error($v)), 7]",
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":0}"#,
                r"[label $f | try error($v) catch ., 3]",
            ],
            r#"[{"__jq":0},3]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"--argjson", r"v", r#"{"__jq":0}"#, r"$v | tojson"],
            r#""{\"__jq\":0}"
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":0}"#,
                r"[label $f | try break $f catch .], $v",
            ],
            r#"[{"__jq":0}]
{"__jq":0}
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":0,"x":1}"#,
                r"[label $f | 1, error($v), 2]",
            ],
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":0,"x":1}
"#,
            5,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":0,"x":1}"#,
                r"[label $f | first(error($v)), 7]",
            ],
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":0,"x":1}
"#,
            5,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":0,"x":1}"#,
                r"[label $f | try error($v) catch ., 3]",
            ],
            r#"[{"__jq":0,"x":1},3]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":0,"x":1}"#,
                r"$v | tojson",
            ],
            r#""{\"__jq\":0,\"x\":1}"
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jq":0,"x":1}"#,
                r"[label $f | try break $f catch .], $v",
            ],
            r#"[{"__jq":0}]
{"__jq":0,"x":1}
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jx":0}"#,
                r"[label $f | 1, error($v), 2]",
            ],
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jx":0}"#,
                r"[label $f | first(error($v)), 7]",
            ],
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jx":0}"#,
                r"[label $f | try error($v) catch ., 3]",
            ],
            r#"[{"__jx":0},3]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"--argjson", r"v", r#"{"__jx":0}"#, r"$v | tojson"],
            r#""{\"__jx\":0}"
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--argjson",
                r"v",
                r#"{"__jx":0}"#,
                r"[label $f | try break $f catch .], $v",
            ],
            r#"[{"__jq":0}]
{"__jx":0}
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jq":0}"#,
                r"[label $f | 1, error(($s|fromjson)), 2]",
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jq":0}"#,
                r"[label $f | first(error(($s|fromjson))), 7]",
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jq":0}"#,
                r"[label $f | try error(($s|fromjson)) catch ., 3]",
            ],
            r#"[{"__jq":0},3]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jq":0}"#,
                r"($s|fromjson) | tojson",
            ],
            r#""{\"__jq\":0}"
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jq":0}"#,
                r"[label $f | try break $f catch .], ($s|fromjson)",
            ],
            r#"[{"__jq":0}]
{"__jq":0}
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jq":0,"x":1}"#,
                r"[label $f | 1, error(($s|fromjson)), 2]",
            ],
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":0,"x":1}
"#,
            5,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jq":0,"x":1}"#,
                r"[label $f | first(error(($s|fromjson))), 7]",
            ],
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":0,"x":1}
"#,
            5,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jq":0,"x":1}"#,
                r"[label $f | try error(($s|fromjson)) catch ., 3]",
            ],
            r#"[{"__jq":0,"x":1},3]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jq":0,"x":1}"#,
                r"($s|fromjson) | tojson",
            ],
            r#""{\"__jq\":0,\"x\":1}"
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jq":0,"x":1}"#,
                r"[label $f | try break $f catch .], ($s|fromjson)",
            ],
            r#"[{"__jq":0}]
{"__jq":0,"x":1}
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jx":0}"#,
                r"[label $f | 1, error(($s|fromjson)), 2]",
            ],
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jx":0}"#,
                r"[label $f | first(error(($s|fromjson))), 7]",
            ],
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jx":0}"#,
                r"[label $f | try error(($s|fromjson)) catch ., 3]",
            ],
            r#"[{"__jx":0},3]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jx":0}"#,
                r"($s|fromjson) | tojson",
            ],
            r#""{\"__jx\":0}"
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"s",
                r#"{"__jx":0}"#,
                r"[label $f | try break $f catch .], ($s|fromjson)",
            ],
            r#"[{"__jq":0}]
{"__jx":0}
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"k",
                r"__jq",
                r"[label $f | 1, error({($k):0}), 2]",
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"k",
                r"__jq",
                r"[label $f | first(error({($k):0})), 7]",
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"k",
                r"__jq",
                r"[label $f | try error({($k):0}) catch ., 3]",
            ],
            r#"[{"__jq":0},3]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"--arg", r"k", r"__jq", r"{($k):0} | tojson"],
            r#""{\"__jq\":0}"
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"k",
                r"__jq",
                r"[label $f | try break $f catch .], {($k):0}",
            ],
            r#"[{"__jq":0}]
{"__jq":0}
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"k",
                r"__jx",
                r"[label $f | 1, error({($k):0}), 2]",
            ],
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"k",
                r"__jx",
                r"[label $f | first(error({($k):0})), 7]",
            ],
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"k",
                r"__jx",
                r"[label $f | try error({($k):0}) catch ., 3]",
            ],
            r#"[{"__jx":0},3]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"--arg", r"k", r"__jx", r"{($k):0} | tojson"],
            r#""{\"__jx\":0}"
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"--arg",
                r"k",
                r"__jx",
                r"[label $f | try break $f catch .], {($k):0}",
            ],
            r#"[{"__jq":0}]
{"__jx":0}
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | 1, error($ARGS.positional[0]), 2]",
                r"--jsonargs",
                r#"{"__jq":0}"#,
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | first(error($ARGS.positional[0])), 7]",
                r"--jsonargs",
                r#"{"__jq":0}"#,
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | try error($ARGS.positional[0]) catch ., 3]",
                r"--jsonargs",
                r#"{"__jq":0}"#,
            ],
            r#"[{"__jq":0},3]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"$ARGS.positional[0] | tojson",
                r"--jsonargs",
                r#"{"__jq":0}"#,
            ],
            r#""{\"__jq\":0}"
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | try break $f catch .], $ARGS.positional[0]",
                r"--jsonargs",
                r#"{"__jq":0}"#,
            ],
            r#"[{"__jq":0}]
{"__jq":0}
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | 1, error($ARGS.positional[0]), 2]",
                r"--jsonargs",
                r#"{"__jq":0,"x":1}"#,
            ],
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":0,"x":1}
"#,
            5,
        ),
        (
            &[
                r"-nc",
                r"[label $f | first(error($ARGS.positional[0])), 7]",
                r"--jsonargs",
                r#"{"__jq":0,"x":1}"#,
            ],
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":0,"x":1}
"#,
            5,
        ),
        (
            &[
                r"-nc",
                r"[label $f | try error($ARGS.positional[0]) catch ., 3]",
                r"--jsonargs",
                r#"{"__jq":0,"x":1}"#,
            ],
            r#"[{"__jq":0,"x":1},3]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"$ARGS.positional[0] | tojson",
                r"--jsonargs",
                r#"{"__jq":0,"x":1}"#,
            ],
            r#""{\"__jq\":0,\"x\":1}"
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | try break $f catch .], $ARGS.positional[0]",
                r"--jsonargs",
                r#"{"__jq":0,"x":1}"#,
            ],
            r#"[{"__jq":0}]
{"__jq":0,"x":1}
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | 1, error($ARGS.positional[0]), 2]",
                r"--jsonargs",
                r#"{"__jx":0}"#,
            ],
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            &[
                r"-nc",
                r"[label $f | first(error($ARGS.positional[0])), 7]",
                r"--jsonargs",
                r#"{"__jx":0}"#,
            ],
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            &[
                r"-nc",
                r"[label $f | try error($ARGS.positional[0]) catch ., 3]",
                r"--jsonargs",
                r#"{"__jx":0}"#,
            ],
            r#"[{"__jx":0},3]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"$ARGS.positional[0] | tojson",
                r"--jsonargs",
                r#"{"__jx":0}"#,
            ],
            r#""{\"__jx\":0}"
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | try break $f catch .], $ARGS.positional[0]",
                r"--jsonargs",
                r#"{"__jx":0}"#,
            ],
            r#"[{"__jq":0}]
{"__jx":0}
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | 1, error({($ARGS.positional[0]):0}), 2]",
                r"--args",
                r"__jq",
            ],
            r"[1]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | first(error({($ARGS.positional[0]):0})), 7]",
                r"--args",
                r"__jq",
            ],
            r"[]
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | try error({($ARGS.positional[0]):0}) catch ., 3]",
                r"--args",
                r"__jq",
            ],
            r#"[{"__jq":0},3]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"{($ARGS.positional[0]):0} | tojson",
                r"--args",
                r"__jq",
            ],
            r#""{\"__jq\":0}"
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | try break $f catch .], {($ARGS.positional[0]):0}",
                r"--args",
                r"__jq",
            ],
            r#"[{"__jq":0}]
{"__jq":0}
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | 1, error({($ARGS.positional[0]):0}), 2]",
                r"--args",
                r"__jx",
            ],
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            &[
                r"-nc",
                r"[label $f | first(error({($ARGS.positional[0]):0})), 7]",
                r"--args",
                r"__jx",
            ],
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            &[
                r"-nc",
                r"[label $f | try error({($ARGS.positional[0]):0}) catch ., 3]",
                r"--args",
                r"__jx",
            ],
            r#"[{"__jx":0},3]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"{($ARGS.positional[0]):0} | tojson",
                r"--args",
                r"__jx",
            ],
            r#""{\"__jx\":0}"
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[label $f | try break $f catch .], {($ARGS.positional[0]):0}",
                r"--args",
                r"__jx",
            ],
            r#"[{"__jq":0}]
{"__jx":0}
"#,
            r"",
            0,
        ),
    ])
}

/// The same for `--rawfile` and `--slurpfile` values.
#[test]
fn test_argument_file_label_sentinel_kept_7880() -> Result<()> {
    check_files(&[
        (
            r"--rawfile",
            r#"{"__jq":0}"#,
            r"[label $f | 1, error(($s|fromjson)), 2]",
            r"[1]
",
            r"",
            0,
        ),
        (
            r"--rawfile",
            r#"{"__jq":0}"#,
            r"[limit(3; 1, error(($s|fromjson)), 2)]",
            r"[1]
",
            r"",
            0,
        ),
        (
            r"--rawfile",
            r#"{"__jq":0}"#,
            r"($s|fromjson) | tojson",
            r#""{\"__jq\":0}"
"#,
            r"",
            0,
        ),
        (
            r"--rawfile",
            r#"{"__jx":0}"#,
            r"[label $f | 1, error(($s|fromjson)), 2]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            r"--rawfile",
            r#"{"__jx":0}"#,
            r"[limit(3; 1, error(($s|fromjson)), 2)]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            r"--rawfile",
            r#"{"__jx":0}"#,
            r"($s|fromjson) | tojson",
            r#""{\"__jx\":0}"
"#,
            r"",
            0,
        ),
        (
            r"--rawfile",
            r"__jq",
            r"[label $f | 1, error({($k):0}), 2]",
            r"[1]
",
            r"",
            0,
        ),
        (
            r"--rawfile",
            r"__jq",
            r"[limit(3; 1, error({($k):0}), 2)]",
            r"[1]
",
            r"",
            0,
        ),
        (
            r"--rawfile",
            r"__jq",
            r"{($k):0} | tojson",
            r#""{\"__jq\":0}"
"#,
            r"",
            0,
        ),
        (
            r"--rawfile",
            r"__jx",
            r"[label $f | 1, error({($k):0}), 2]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            r"--rawfile",
            r"__jx",
            r"[limit(3; 1, error({($k):0}), 2)]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            r"--rawfile",
            r"__jx",
            r"{($k):0} | tojson",
            r#""{\"__jx\":0}"
"#,
            r"",
            0,
        ),
        (
            r"--slurpfile",
            r#"{"__jq":0}
"#,
            r"[label $f | 1, error($v[0]), 2]",
            r"[1]
",
            r"",
            0,
        ),
        (
            r"--slurpfile",
            r#"{"__jq":0}
"#,
            r"$v[0] | tojson",
            r#""{\"__jq\":0}"
"#,
            r"",
            0,
        ),
        (
            r"--slurpfile",
            r#"{"__jx":0}
"#,
            r"[label $f | 1, error($v[0]), 2]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            r"--slurpfile",
            r#"{"__jx":0}
"#,
            r"[limit(3; 1, error($v[0]), 2)]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            r"--slurpfile",
            r#"{"__jx":0}
"#,
            r"$v[0] | tojson",
            r#""{\"__jx\":0}"
"#,
            r"",
            0,
        ),
    ])
}

/// The environment, read through `$ENV` and `env` by a program that raises: every row with
/// `__jq` whose answer is not an error answered differently from jq before #7880.
#[test]
fn test_environment_label_sentinel_7880() -> Result<()> {
    check_env(&[
        (
            r"V7880",
            r#"{"__jq":0}"#,
            r"[label $f | 1, error(($ENV.V7880|fromjson)), 2]",
            r"[1]
",
            r"",
            0,
        ),
        (
            r"V7880",
            r#"{"__jq":0}"#,
            r"[limit(3; 1, error(($ENV.V7880|fromjson)), 2)]",
            r"[1]
",
            r"",
            0,
        ),
        (
            r"V7880",
            r#"{"__jq":0}"#,
            r"[isempty(error(($ENV.V7880|fromjson)))]",
            r"[]
",
            r"",
            0,
        ),
        (
            r"V7880",
            r#"{"__jq":0}"#,
            r"[label $f | first(error(($ENV.V7880|fromjson))), 7]",
            r"[]
",
            r"",
            0,
        ),
        (
            r"V7880",
            r#"{"__jq":0}"#,
            r"[label $f | try error(($ENV.V7880|fromjson)) catch ., 3]",
            r#"[{"__jq":0},3]
"#,
            r"",
            0,
        ),
        (
            r"V7880",
            r#"{"__jq":0}"#,
            r"($ENV.V7880|fromjson) | tojson",
            r#""{\"__jq\":0}"
"#,
            r"",
            0,
        ),
        (
            r"V7880",
            r#"{"__jq":0,"x":1}"#,
            r"[label $f | 1, error(($ENV.V7880|fromjson)), 2]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":0,"x":1}
"#,
            5,
        ),
        (
            r"V7880",
            r#"{"__jq":0,"x":1}"#,
            r"[limit(3; 1, error(($ENV.V7880|fromjson)), 2)]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":0,"x":1}
"#,
            5,
        ),
        (
            r"V7880",
            r#"{"__jq":0,"x":1}"#,
            r"[isempty(error(($ENV.V7880|fromjson)))]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":0,"x":1}
"#,
            5,
        ),
        (
            r"V7880",
            r#"{"__jq":0,"x":1}"#,
            r"[label $f | first(error(($ENV.V7880|fromjson))), 7]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":0,"x":1}
"#,
            5,
        ),
        (
            r"V7880",
            r#"{"__jq":0,"x":1}"#,
            r"[label $f | try error(($ENV.V7880|fromjson)) catch ., 3]",
            r#"[{"__jq":0,"x":1},3]
"#,
            r"",
            0,
        ),
        (
            r"V7880",
            r#"{"__jq":0,"x":1}"#,
            r"($ENV.V7880|fromjson) | tojson",
            r#""{\"__jq\":0,\"x\":1}"
"#,
            r"",
            0,
        ),
        (
            r"V7880",
            r#"{"__jx":0}"#,
            r"[label $f | 1, error(($ENV.V7880|fromjson)), 2]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            r"V7880",
            r#"{"__jx":0}"#,
            r"[limit(3; 1, error(($ENV.V7880|fromjson)), 2)]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            r"V7880",
            r#"{"__jx":0}"#,
            r"[isempty(error(($ENV.V7880|fromjson)))]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            r"V7880",
            r#"{"__jx":0}"#,
            r"[label $f | first(error(($ENV.V7880|fromjson))), 7]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            r"V7880",
            r#"{"__jx":0}"#,
            r"[label $f | try error(($ENV.V7880|fromjson)) catch ., 3]",
            r#"[{"__jx":0},3]
"#,
            r"",
            0,
        ),
        (
            r"V7880",
            r#"{"__jx":0}"#,
            r"($ENV.V7880|fromjson) | tojson",
            r#""{\"__jx\":0}"
"#,
            r"",
            0,
        ),
        (
            r"V7880",
            r#"{"__jq":0}"#,
            r"[label $f | 1, error((env.V7880|fromjson)), 2]",
            r"[1]
",
            r"",
            0,
        ),
        (
            r"V7880",
            r#"{"__jq":0}"#,
            r"[limit(3; 1, error((env.V7880|fromjson)), 2)]",
            r"[1]
",
            r"",
            0,
        ),
        (
            r"V7880",
            r#"{"__jq":0}"#,
            r"[isempty(error((env.V7880|fromjson)))]",
            r"[]
",
            r"",
            0,
        ),
        (
            r"V7880",
            r#"{"__jq":0}"#,
            r"[label $f | first(error((env.V7880|fromjson))), 7]",
            r"[]
",
            r"",
            0,
        ),
        (
            r"V7880",
            r#"{"__jq":0}"#,
            r"[label $f | try error((env.V7880|fromjson)) catch ., 3]",
            r#"[{"__jq":0},3]
"#,
            r"",
            0,
        ),
        (
            r"V7880",
            r#"{"__jq":0}"#,
            r"(env.V7880|fromjson) | tojson",
            r#""{\"__jq\":0}"
"#,
            r"",
            0,
        ),
        (
            r"K7880",
            r"__jq",
            r"[label $f | 1, error({($ENV.K7880):0}), 2]",
            r"[1]
",
            r"",
            0,
        ),
        (
            r"K7880",
            r"__jq",
            r"[limit(3; 1, error({($ENV.K7880):0}), 2)]",
            r"[1]
",
            r"",
            0,
        ),
        (
            r"K7880",
            r"__jq",
            r"[isempty(error({($ENV.K7880):0}))]",
            r"[]
",
            r"",
            0,
        ),
        (
            r"K7880",
            r"__jq",
            r"[label $f | first(error({($ENV.K7880):0})), 7]",
            r"[]
",
            r"",
            0,
        ),
        (
            r"K7880",
            r"__jq",
            r"[label $f | try error({($ENV.K7880):0}) catch ., 3]",
            r#"[{"__jq":0},3]
"#,
            r"",
            0,
        ),
        (
            r"K7880",
            r"__jq",
            r"{($ENV.K7880):0} | tojson",
            r#""{\"__jq\":0}"
"#,
            r"",
            0,
        ),
        (
            r"K7880",
            r"__jq",
            r"[label $f | 1, error({($ENV.K7880):1}), 2]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":1}
"#,
            5,
        ),
        (
            r"K7880",
            r"__jq",
            r"[limit(3; 1, error({($ENV.K7880):1}), 2)]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":1}
"#,
            5,
        ),
        (
            r"K7880",
            r"__jq",
            r"[isempty(error({($ENV.K7880):1}))]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jq":1}
"#,
            5,
        ),
        (
            r"K7880",
            r"__jq",
            r"[label $f | first(error({($ENV.K7880):1})), 7]",
            r"[7]
",
            r"",
            0,
        ),
        (
            r"K7880",
            r"__jq",
            r"[label $f | try error({($ENV.K7880):1}) catch ., 3]",
            r#"[{"__jq":1},3]
"#,
            r"",
            0,
        ),
        (
            r"K7880",
            r"__jq",
            r"{($ENV.K7880):1} | tojson",
            r#""{\"__jq\":1}"
"#,
            r"",
            0,
        ),
        (
            r"K7880",
            r"__jx",
            r"[label $f | 1, error({($ENV.K7880):0}), 2]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            r"K7880",
            r"__jx",
            r"[limit(3; 1, error({($ENV.K7880):0}), 2)]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            r"K7880",
            r"__jx",
            r"[isempty(error({($ENV.K7880):0}))]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            r"K7880",
            r"__jx",
            r"[label $f | first(error({($ENV.K7880):0})), 7]",
            r"",
            r#"jq: error (at <unknown>) (not a string): {"__jx":0}
"#,
            5,
        ),
        (
            r"K7880",
            r"__jx",
            r"[label $f | try error({($ENV.K7880):0}) catch ., 3]",
            r#"[{"__jx":0},3]
"#,
            r"",
            0,
        ),
        (
            r"K7880",
            r"__jx",
            r"{($ENV.K7880):0} | tojson",
            r#""{\"__jx\":0}"
"#,
            r"",
            0,
        ),
        (
            r"K7880",
            r"__jq",
            r"[label $f | 1, error({(env.K7880):0}), 2]",
            r"[1]
",
            r"",
            0,
        ),
        (
            r"K7880",
            r"__jq",
            r"[limit(3; 1, error({(env.K7880):0}), 2)]",
            r"[1]
",
            r"",
            0,
        ),
        (
            r"K7880",
            r"__jq",
            r"[isempty(error({(env.K7880):0}))]",
            r"[]
",
            r"",
            0,
        ),
        (
            r"K7880",
            r"__jq",
            r"[label $f | first(error({(env.K7880):0})), 7]",
            r"[]
",
            r"",
            0,
        ),
        (
            r"K7880",
            r"__jq",
            r"[label $f | try error({(env.K7880):0}) catch ., 3]",
            r#"[{"__jq":0},3]
"#,
            r"",
            0,
        ),
        (
            r"K7880",
            r"__jq",
            r"{(env.K7880):0} | tojson",
            r#""{\"__jq\":0}"
"#,
            r"",
            0,
        ),
    ])
}
