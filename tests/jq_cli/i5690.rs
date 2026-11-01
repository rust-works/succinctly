//! #5690: a container named argument (`--argjson`, `--slurpfile`, `$ARGS`) is spliced into a
//! program that only reads it as one shared value, not as a literal tree rebuilt at every `$name`
//! and built again when the program runs (10 MB of `--slurpfile` was 177 ms, now 75 ms). The rows
//! pin that a read answers as the literal did, and that a program with a fold, an assignment, a
//! path expression or a definition keeps the literal (#5673's rows pin those targets). Outputs are
//! captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `(--argjson d, filter, stdout)`, every one exiting 0 under `-nc`.
const ROWS: &[(&str, &str, &str)] = &[
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "$d", "{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "$d|length", "5\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "$d|keys", "[\"a\",\"c\",\"e\",\"f\",\"n\"]\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "$d|tojson", "\"{\\\"a\\\":[1,2,{\\\"b\\\":3}],\\\"c\\\":\\\"x\\\",\\\"n\\\":null,\\\"e\\\":{},\\\"f\\\":[]}\"\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "[$d,$d]|.[0]==.[1]", "true\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "$d==$d", "true\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "$d|[..]|length", "10\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "{x:$d}|.x.a[2].b", "3\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "$d.a|map(numbers)|add", "3\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "$d|to_entries|length", "5\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "$d|.a as [$p,$q]|$p+$q", "3\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "[limit(2;$d.a[])]", "[1,2]\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "first($d.a[])", "1\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "$d|tostream|length", "2\n2\n2\n1\n1\n2\n2\n2\n2\n1\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "[$d|tostream]|fromstream(.[])|.==$d", "true\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "$d|contains({a:[1]})", "true\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "$d|.c as $c|[$c,$c]", "[\"x\",\"x\"]\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "$d|has(\"n\")", "true\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "[$d|.[]?]|length", "5\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "$d // 0", "{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "try ($d|error) catch .|.a[0]", "1\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "def f: $d.c; f", "\"x\"\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "$d|[paths]|length", "9\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "$d|del(.a)|keys", "[\"c\",\"e\",\"f\",\"n\"]\n"),
    ("{\"a\":[1,2,{\"b\":3}],\"c\":\"x\",\"n\":null,\"e\":{},\"f\":[]}", "$d|.c=1|.c", "1\n"),
    ("[1,[2,[3,[4]]],\"s\",null,true,{\"k\":[1.5]}]", "$d", "[1,[2,[3,[4]]],\"s\",null,true,{\"k\":[1.5]}]\n"),
    ("[1,[2,[3,[4]]],\"s\",null,true,{\"k\":[1.5]}]", "$d|flatten", "[1,2,3,4,\"s\",null,true,{\"k\":[1.5]}]\n"),
    ("[1,[2,[3,[4]]],\"s\",null,true,{\"k\":[1.5]}]", "$d|map(type)", "[\"number\",\"array\",\"string\",\"null\",\"boolean\",\"object\"]\n"),
    ("[1,[2,[3,[4]]],\"s\",null,true,{\"k\":[1.5]}]", "$d|.[1]|.[1]|.[1]", "[4]\n"),
    ("[1,[2,[3,[4]]],\"s\",null,true,{\"k\":[1.5]}]", "$d|[.[]|numbers]", "[1]\n"),
    ("[1,[2,[3,[4]]],\"s\",null,true,{\"k\":[1.5]}]", "$d|index(\"s\")", "2\n"),
    ("[1,[2,[3,[4]]],\"s\",null,true,{\"k\":[1.5]}]", "$d|.[5].k[0]", "1.5\n"),
    ("[1,[2,[3,[4]]],\"s\",null,true,{\"k\":[1.5]}]", "$d|length", "6\n"),
    ("[1,[2,[3,[4]]],\"s\",null,true,{\"k\":[1.5]}]", "$d|reverse|.[0].k", "[1.5]\n"),
    ("[1,[2,[3,[4]]],\"s\",null,true,{\"k\":[1.5]}]", "$d|any(.==true)", "true\n"),
    ("[1,[2,[3,[4]]],\"s\",null,true,{\"k\":[1.5]}]", "[$d,$d]|unique|length", "1\n"),
    ("[{\"id\":1,\"v\":\"a\"},{\"id\":2,\"v\":\"b\"},{\"id\":3,\"v\":\"c\"}]", "$d|map(select(.id>1))|map(.v)", "[\"b\",\"c\"]\n"),
    ("[{\"id\":1,\"v\":\"a\"},{\"id\":2,\"v\":\"b\"},{\"id\":3,\"v\":\"c\"}]", "$d|group_by(.id>1)|length", "2\n"),
    ("[{\"id\":1,\"v\":\"a\"},{\"id\":2,\"v\":\"b\"},{\"id\":3,\"v\":\"c\"}]", "$d|map(.v)|join(\",\")", "\"a,b,c\"\n"),
    ("[{\"id\":1,\"v\":\"a\"},{\"id\":2,\"v\":\"b\"},{\"id\":3,\"v\":\"c\"}]", "$d|.[1:]|length", "2\n"),
    ("[{\"id\":1,\"v\":\"a\"},{\"id\":2,\"v\":\"b\"},{\"id\":3,\"v\":\"c\"}]", "$d|map(.id)|add", "6\n"),
    ("[{\"id\":1,\"v\":\"a\"},{\"id\":2,\"v\":\"b\"},{\"id\":3,\"v\":\"c\"}]", "[$d[]|select(.v==\"b\")]", "[{\"id\":2,\"v\":\"b\"}]\n"),
    ("[{\"id\":1,\"v\":\"a\"},{\"id\":2,\"v\":\"b\"},{\"id\":3,\"v\":\"c\"}]", "$d|INDEX(.id)|keys", "[\"1\",\"2\",\"3\"]\n"),
    ("[{\"id\":1,\"v\":\"a\"},{\"id\":2,\"v\":\"b\"},{\"id\":3,\"v\":\"c\"}]", "$d|sort_by(-.id)|.[0].v", "\"c\"\n"),
    ("[{\"id\":1,\"v\":\"a\"},{\"id\":2,\"v\":\"b\"},{\"id\":3,\"v\":\"c\"}]", "$d|(map(.id)|max)", "3\n"),
    ("[{\"id\":1,\"v\":\"a\"},{\"id\":2,\"v\":\"b\"},{\"id\":3,\"v\":\"c\"}]", "$d|to_entries|map(.key)", "[0,1,2]\n"),
];

#[test]
fn a_program_that_reads_a_container_named_argument_answers_as_jq_5690() -> Result<()> {
    for (d, filter, expected) in ROWS {
        let (stdout, stderr, code) = run_jq_full(&["-nc", "--argjson", "d", d, filter], None)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{filter} over {d}: {stderr}"
        );
    }
    Ok(())
}

/// `--slurpfile` and `$ARGS` are container arguments too.
#[test]
fn slurpfile_and_args_reads_answer_as_jq_5690() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let slurped = dir.path().join("s.json");
    std::fs::write(&slurped, "{\"id\":1,\"v\":[1,2]}\n[3]\n")?;
    let slurped = slurped.to_str().unwrap();
    for (args, filter, expected) in [
        (vec!["--slurpfile", "s", slurped], "$s|length", "2\n"),
        (vec!["--slurpfile", "s", slurped], "$s[0].v[1]", "2\n"),
        (vec!["--slurpfile", "s", slurped], "$s|map(type)", "[\"object\",\"array\"]\n"),
        (vec!["--slurpfile", "s", slurped], "[$s,$s]|.[0]==.[1]", "true\n"),
        (vec!["--args"], "$ARGS.positional|length", "2\n"),
        (vec!["--args"], "$ARGS|keys", "[\"named\",\"positional\"]\n"),
    ] {
        let mut full = vec!["-nc"];
        full.extend(args.iter().copied());
        full.push(filter);
        if args == ["--args"] {
            full.extend(["p", "q"]);
        }
        let (stdout, stderr, code) = run_jq_full(&full, None)?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}: {stderr}");
    }
    Ok(())
}

/// A fold, an assignment, a path expression and a definition keep the literal: the same
/// containers answered through each, so a gate that let one through would change an answer or an
/// exit status here (`(.[] | numbers) |= ...` over a shared value is an error).
#[test]
fn programs_that_write_or_walk_paths_keep_the_literal_5690() -> Result<()> {
    for (d, filter, expected) in [
        ("[1,2]", "$d | (.[] | numbers) |= . + 1", "[2,3]\n"),
        ("[1,null,[null,2]]", "$d | del(.. | nulls)", "[1,[2]]\n"),
        ("[[1,2],[3]]", "$d | getpath([0,1]) = 9", "[[1,9],[3]]\n"),
        ("[1,2]", "[$d | paths]", "[[0],[1]]\n"),
        ("[1,2]", "reduce range(3) as $i ([]; . + $d) | length", "6\n"),
        ("[1,2]", "[foreach range(2) as $i (0; .+1; [$d[0], .])]", "[[1,1],[1,2]]\n"),
        ("[1,2]", "def f: $d | length; f", "2\n"),
        ("[3,1,2]", "$d | walk(.) | length", "3\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", "--argjson", "d", d, filter], None)?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}: {stderr}");
    }
    Ok(())
}
