//! #5673: a container-valued named argument (`--argjson`, `--slurpfile`, `$ARGS`) reaches the
//! program as a literal tree rebuilt at every `$name`. Splicing it in as one shared marker instead
//! (the `. as $x` form) was tried for speed and gives up what the literal has: the path-mode
//! interpreter refuses a dynamic path target over such a value (`$d | (.[] | numbers) |= . + 1`,
//! `$d | del(.. | nulls)`), and under `?`/`//` that is silently a different answer. These rows pin
//! every way the program reads the value, plus those targets, so a change to how a named argument
//! is spliced has to keep them. Outputs are captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

const D: &str = r#"{"a":[1,2,{"b":3}],"c":"x","n":null}"#;

/// `(filter, stdout)`, every one exiting 0.
const ROWS: &[(&str, &str)] = &[
    ("$d.a[2].b", "3\n"),
    (
        "$d | .a[0] = 9",
        "{\"a\":[9,2,{\"b\":3}],\"c\":\"x\",\"n\":null}\n",
    ),
    ("$d | del(.a)", "{\"c\":\"x\",\"n\":null}\n"),
    ("$d | keys", "[\"a\",\"c\",\"n\"]\n"),
    ("[$d, $d] | .[0] == .[1]", "true\n"),
    (r#"$d == {"a":[1,2,{"b":3}],"c":"x","n":null}"#, "true\n"),
    (r#"$d | [paths | join(".")?] | length"#, "7\n"),
    (r#"$d | getpath(["a",2,"b"])"#, "3\n"),
    (
        "reduce range(3) as $i ($d; .a[0] = $i) | .a",
        "[2,2,{\"b\":3}]\n",
    ),
    ("[limit(2; $d.a[])]", "[1,2]\n"),
    ("$s", "[{\"id\":1,\"v\":[1,2]},[3]]\n"),
    ("$s | length", "2\n"),
    ("$s[0].v[1]", "2\n"),
    ("$s | map(type)", "[\"object\",\"array\"]\n"),
    ("$ARGS.named | keys", "[\"d\",\"s\"]\n"),
    ("$ARGS.named.d.c", "\"x\"\n"),
    ("$ARGS.named.s[1]", "[3]\n"),
    ("$d as {a: [$x, $y]} | $x + $y", "3\n"),
    ("def f($v): $v.c; f($d)", "\"x\"\n"),
    (
        "$d | tojson",
        "\"{\\\"a\\\":[1,2,{\\\"b\\\":3}],\\\"c\\\":\\\"x\\\",\\\"n\\\":null}\"\n",
    ),
    (
        r#"[$d.n // "dflt", $d.zz // "dflt"]"#,
        "[\"dflt\",\"dflt\"]\n",
    ),
    ("$d.a | map(numbers) | add", "3\n"),
];

#[test]
fn a_container_named_argument_answers_as_the_literal_tree_did_5673() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let slurped = dir.path().join("s.json");
    std::fs::write(&slurped, "{\"id\":1,\"v\":[1,2]}\n[3]\n")?;
    let slurped = slurped.to_str().unwrap();
    for (filter, expected) in ROWS {
        let (stdout, stderr, code) = run_jq_full(
            &[
                "-nc",
                "--argjson",
                "d",
                D,
                "--slurpfile",
                "s",
                slurped,
                filter,
            ],
            None,
        )?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{filter}: {stderr}"
        );
    }
    Ok(())
}

/// A path that ends on, or passes through, a spliced value is refused with jq's own text.
#[test]
#[cfg_attr(feature = "unshared-containers", ignore = "holdout refusal (#5246)")]
fn a_path_through_a_container_named_argument_is_refused_as_jq_does_5673() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-nc", "--argjson", "d", D, "try path($d) catch ."], None)?;
    assert_eq!(
        (stdout.as_str(), code),
        (
            "\"Invalid path expression with result {\\\"a\\\":[1,2,{\\\"b\\\":3}],\\\"c\\\":\\\"x\\\"...\"\n",
            0
        ),
        "{stderr}"
    );
    let (stdout, stderr, code) = run_jq_full(&["-nc", "--argjson", "d", D, "path($d.a[0])"], None)?;
    assert_eq!((stdout.as_str(), code), ("", 5), "{stderr}");
    assert_eq!(
        stderr.trim_end(),
        "jq: error (at <unknown>): Invalid path expression near attempt to access element \"a\" of {\"a\":[1,2,{\"b\":3}],\"c\":\"x\"..."
    );
    Ok(())
}

/// A scalar and a short string stay literals; a value used twice is the same value both times.
#[test]
fn scalar_named_arguments_and_repeated_uses_are_unchanged_5673() -> Result<()> {
    let (stdout, _, code) = run_jq_full(
        &[
            "-nc",
            "--arg",
            "s",
            "x",
            "--argjson",
            "n",
            "7",
            "--argjson",
            "d",
            "[1,[2]]",
            "[$s, $n, $d, $d[1], ($d | .[1][0])]",
        ],
        None,
    )?;
    assert_eq!((stdout.as_str(), code), ("[\"x\",7,[1,[2]],[2],2]\n", 0));
    Ok(())
}

/// Dynamic path targets (`first`, `getpath`, `..`, `numbers` in an assignment or `del` target) over
/// a container argument. `path_mode::route` sends the whole program to the interpreter, which has
/// to be able to take the value in: `(args, filter, stdout)`.
#[test]
#[cfg_attr(feature = "unshared-containers", ignore = "holdout refusal (#5246)")]
fn dynamic_path_targets_over_a_container_named_argument_5673() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let slurped = dir.path().join("s.json");
    std::fs::write(&slurped, "{\"id\":1}\n[2]\n")?;
    let slurped = slurped.to_str().unwrap();
    let rows: &[(&[&str], &str, &str)] = &[
        (
            &["--argjson", "d", "[1,2]"],
            "$d | (.[] | numbers) |= . + 1",
            "[2,3]\n",
        ),
        (
            &["--argjson", "d", "[1,null,[null,2]]"],
            "$d | del(.. | nulls)",
            "[1,[2]]\n",
        ),
        (
            &["--argjson", "d", "[[1,2],[3]]"],
            "$d | getpath([0,1]) = 9",
            "[[1,9],[3]]\n",
        ),
        (
            &["--slurpfile", "s", slurped],
            "$s | (.. | numbers) |= . + 1",
            "[{\"id\":2},[3]]\n",
        ),
        (
            &["--argjson", "d", "[1,2]"],
            "($d | first |= 9)? // 0",
            "[9,2]\n",
        ),
        (
            &["--argjson", "d", "[1,2]"],
            "path(1)?, ($d | [paths])",
            "[[0],[1]]\n",
        ),
    ];
    for (named, filter, expected) in rows {
        let mut args = vec!["-nc"];
        args.extend_from_slice(named);
        args.push(filter);
        let (stdout, stderr, code) = run_jq_full(&args, None)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{filter}: {stderr}"
        );
    }
    // `$ARGS.positional` is a container argument too.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-nc",
            "$ARGS.positional | first |= ascii_upcase",
            "--args",
            "a",
            "b",
        ],
        None,
    )?;
    assert_eq!((stdout.as_str(), code), ("[\"A\",\"b\"]\n", 0), "{stderr}");
    Ok(())
}
