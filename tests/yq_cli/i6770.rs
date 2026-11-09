//! #6770: yq's `map(f)` splats its node and runs `f` once over the whole list of members
//! (`mapOperator`, `GetMatchingNodes(splatted, RHS)`), so a union in `f` runs each branch over every
//! member before the next one: `map(.a, .b)` is every `.a`, then every `.b`. `succinctly yq` ran `f`
//! per member, so the branches interleaved. A union-bodied `map` over a sequence or a non-empty
//! mapping now takes the walk `[.[] | f]` already takes; a scalar, `null` and an empty mapping keep the
//! `map` arm, which never runs `f` on them. Every answer was captured from yq v4.53.3
//! (`-o=json -I=0`).

use anyhow::Result;

use crate::{run_jq_stdin, run_yq_stdin};

const DOCS: [(&str, &str); 8] = [
    ("arr", "b: [1, 2, 3]\n"),
    (
        "objs",
        "b:\n  - {n: 1, m: 9}\n  - {n: 2, m: 8}\n  - {n: 3, m: 7}\n",
    ),
    ("map", "b: {x: 1, y: 2, z: 3}\n"),
    ("empty", "b: {}\n"),
    ("emptyseq", "b: []\n"),
    ("scalar", "b: 5\n"),
    ("null", "b: ~\n"),
    ("two", "a: [1, 2, 3]\nb: 1\n"),
];

fn doc(name: &str) -> &'static str {
    DOCS.iter()
        .find(|(n, _)| *n == name)
        .map(|(_, d)| *d)
        .expect("a known document")
}

#[test]
fn test_yq_map_with_a_union_body_runs_each_branch_over_the_whole_list_6770() -> Result<()> {
    for (name, filter, want) in [
        ("arr", ".b | map(. + 1, . + 2)", "[2,3,4,3,4,5]\n"),
        ("arr", ".b | map(select(. > 1), 9)", "[2,3,9,9,9]\n"),
        ("arr", ".b | map(1, 2)", "[1,1,1,2,2,2]\n"),
        (
            "arr",
            ".b | map((. + 1, . + 2) | . * 2)",
            "[4,6,8,6,8,10]\n",
        ),
        ("arr", ".b | map(. + 1 | (., . * 10))", "[2,3,4,20,30,40]\n"),
        ("arr", ".b | map(. as $x | $x, $x + 1)", "[1,2,3,1,1,1]\n"),
        ("arr", ".b | map(. // 9, .)", "[1,2,3,1,2,3]\n"),
        ("arr", ".b | map(select(. == 2), .)", "[2,1,2,3]\n"),
        ("objs", ".b | map(.n, .m)", "[1,2,3,9,8,7]\n"),
        ("objs", ".b | map(.n, .m, .n)", "[1,2,3,9,8,7,1,2,3]\n"),
        (
            "objs",
            ".b | map(keys, length)",
            "[[\"n\",\"m\"],[\"n\",\"m\"],[\"n\",\"m\"],2,2,2]\n",
        ),
        (
            "objs",
            ".b | map(has(\"n\"), has(\"z\"))",
            "[true,true,true,false,false,false]\n",
        ),
        (
            "objs",
            ".b | map(tag, kind)",
            "[\"!!map\",\"!!map\",\"!!map\",\"map\",\"map\",\"map\"]\n",
        ),
        ("map", ".b | map(. + 1, . + 2)", "[2,3,4,3,4,5]\n"),
        ("arr", ".b | map(. + 1)", "[2,3,4]\n"),
        ("arr", ".b | map(select(. > 1))", "[2,3]\n"),
        ("arr", ".b | map_values(. + 1, . + 2)", "[2,3,4]\n"),
        ("arr", "[.b[] | (. + 1, . + 2)]", "[2,3,4,3,4,5]\n"),
        ("arr", ".b | map([. , 0])", "[[1,0],[2,0],[3,0]]\n"),
        ("arr", ".b | map(. + 1, . + 2) | length", "6\n"),
        ("empty", ".b | map(1, 2)", "{}\n"),
        ("emptyseq", ".b | map(1, 2)", "[]\n"),
        ("scalar", ".b | map(1, error(\"boom\"))", "5\n"),
        ("null", ".b | map(1, 2)", "[]\n"),
        ("arr", ".b | map((. , 0)) | .[3]", "0\n"),
        ("two", "(.a | map(., 0)) |= 5", "{\"a\":[1,2,3],\"b\":1}\n"),
        ("two", "(.a | map(., 0)) = 5", "{\"a\":[1,2,3],\"b\":1}\n"),
        ("two", ".a | map(., 0) | path", "[]\n"),
        ("two", "map(., 0) = 5", "{\"a\":[1,2,3],\"b\":1}\n"),
        ("two", ".a | map(., 0)", "[1,2,3,0,0,0]\n"),
        ("two", ".a | map(map(., 0), 9)", "[1,2,3,9,9,9]\n"),
        ("two", ". as $r | .a | map(., 0)", "[1,2,3,0,0,0]\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, doc(name), &["-o=json", "-I=0"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{name}: {filter}");
    }
    Ok(())
}

/// jq runs the body per member, so a union interleaves: `succinctly jq` is unchanged.
#[test]
fn test_jq_map_with_a_union_body_still_interleaves_6770() -> Result<()> {
    let (stdout, code) = run_jq_stdin("map(., 0)", "[1,2,3]", &["-c"])?;
    assert_eq!((stdout.as_str(), code), ("[1,0,2,0,3,0]\n", 0));
    Ok(())
}

/// The call is rewritten with its body once, so nesting stays linear: 24 levels is 2^24 copies of
/// the innermost body if each level keeps a second one.
#[test]
fn test_yq_a_nest_of_union_bodied_maps_stays_linear_6770() -> Result<()> {
    let mut filter = String::from("map(., .)");
    for _ in 0..24 {
        filter = format!("map({filter}, .)");
    }
    let (stdout, code) = run_yq_stdin(&filter, "[]\n", &["-o=json", "-I=0"])?;
    assert_eq!((stdout.as_str(), code), ("[]\n", 0));
    Ok(())
}
