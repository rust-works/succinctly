//! #7267: an array built from a value computed per element of a document array (`[.xs[] | floor]`)
//! keeps one 32-byte owned value per element while it is built, not an 80-byte per-element result
//! beside a second, doubling vector, once a long enough run of elements has yielded computed values
//! only. These rows pin what must not move with it: order across cursors and computed values,
//! `empty`, several values from one element, an error after a prefix (raised, caught, suppressed) and a
//! `halt_error` part-way. Every expected text was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_per_element_collections_match_jq_7267() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cases: [(&str, &str, &str, &str, i32); 16] = [
        (
            r#"{"xs":[1.5,2.5,-0.5]}"#,
            "[.xs[] | floor]",
            "[1,2,-1]\n",
            "",
            0,
        ),
        (
            r#"{"xs":[1,"a",2,{"b":1},3,null]}"#,
            r#"[.xs[] | if type == "number" then . + 1 else . end]"#,
            "[2,\"a\",3,{\"b\":1},4,null]\n",
            "",
            0,
        ),
        (
            r#"{"xs":[1,"a",2,{"b":1},3,null]}"#,
            r#"[.xs[] | if type == "number" then . * 10 else empty end]"#,
            "[10,20,30]\n",
            "",
            0,
        ),
        (
            r#"{"xs":[1,2,3,0]}"#,
            "[.xs[] | range(.)]",
            "[0,0,1,0,1,2]\n",
            "",
            0,
        ),
        (
            r#"{"xs":[1,2,3,0]}"#,
            "[.xs[] | select(. > 1) | . + 1]",
            "[3,4]\n",
            "",
            0,
        ),
        (r#"{"xs":[1,2,3,0]}"#, "[.xs[] | empty]", "[]\n", "", 0),
        (
            r#"{"xs":[1,2,3,0]}"#,
            r#"[.xs[] | if . == 3 then error("boom") else . + 1 end]"#,
            "",
            "jq: error (at <file>:0): boom\n",
            5,
        ),
        (
            r#"{"xs":[1,2,3,0]}"#,
            r#"try [.xs[] | if . == 3 then error("boom") else . + 1 end] catch ."#,
            "\"boom\"\n",
            "",
            0,
        ),
        (
            r#"{"xs":[1,2,3,0]}"#,
            r#"[.xs[] | if . == 3 then error("boom") else . + 1 end]?"#,
            "",
            "",
            0,
        ),
        (
            r#"{"xs":[1,2,3,0]}"#,
            r#"[.xs[] | (if . == 3 then error("boom") else . + 1 end)?]"#,
            "[2,3,1]\n",
            "",
            0,
        ),
        (
            r#"{"xs":[1,2,3,0]}"#,
            r#".xs[] | if . == 3 then error("boom") else . + 1 end"#,
            "2\n3\n",
            "jq: error (at <file>:0): boom\n",
            5,
        ),
        (
            r#"{"xs":[1,2,3,0]}"#,
            "[.xs[] | . + 1] | add",
            "10\n",
            "",
            0,
        ),
        (
            r#"{"xs":["ab","",[1,2],{"k":1},null]}"#,
            "[.xs[] | length]",
            "[2,0,2,1,0]\n",
            "",
            0,
        ),
        (
            r#"{"xs":["ab","",[1,2],{"k":1},null]}"#,
            "[.xs[] | tostring]",
            "[\"ab\",\"\",\"[1,2]\",\"{\\\"k\\\":1}\",\"null\"]\n",
            "",
            0,
        ),
        (
            r#"{"xs":[1,2,3,0]}"#,
            "[.xs[] | if . == 2 then halt_error else . end]",
            "",
            "2\n",
            5,
        ),
        (
            r#"{"xs":[1,2,3,0]}"#,
            "[.xs[] | . as $x | [$x, $x + 1][]]",
            "[1,2,2,3,3,4,0,1]\n",
            "",
            0,
        ),
    ];
    for (n, (input, filter, stdout, stderr, code)) in cases.into_iter().enumerate() {
        let path = dir.path().join(format!("in{n}.json"));
        std::fs::write(&path, input)?;
        let file = path.to_str().expect("utf-8 temp path");
        let got = run_jq_full(&["-c", filter, file], None)?;
        let want = (stdout.to_string(), stderr.replace("<file>", file), code);
        assert_eq!(got, want, "{filter} over {input}");
    }
    Ok(())
}

/// A fold recursing through a per-element pipe as deep as main's native stack allows it, captured
/// from jq 1.7.1 (`0`): the frame of `fold_pipe_stages`, which every level of this recursion keeps
/// on the stack, grew by more than half in one version of #7267 and the stack guard refused it at
/// depth 3,200 (#7267 review; main refuses past about 3,250 on an Apple M4 Pro and 3,370 on an AMD
/// Ryzen 9 7950X in a release build). A debug build reserves a larger stack, so there the row only
/// pins the output.
#[test]
fn test_deep_recursion_through_a_per_element_fold_7267() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("d.json");
    std::fs::write(&path, "[1,2,3]")?;
    let filter = ". as $d | def f(n): if n == 0 then 0 else [$d[] | if . == 1 then ($d | f(n - 1)) else . end] | .[0] end; f(3200)";
    let got = run_jq_full(&[filter, path.to_str().expect("utf-8 temp path")], None)?;
    assert_eq!(got, ("0\n".to_string(), String::new(), 0));
    Ok(())
}

/// The same recursion from inside a fold that has moved its computed values to their own vector,
/// captured from jq 1.7.1 (`1`): the 99 values before the recursing element move after 64, so each
/// level keeps the moved fold's frames on the stack. In 4fcc22658 that path ran in a second,
/// out-of-line frame and the stack guard refused it at about depth 3,185, where main answers to
/// 3,255 on an Apple M4 Pro in a release build (#7267 review); 3,240 lies between the two. A debug
/// build's 2 GiB stack does not make up for its larger frames (this shape refuses past about 3,200
/// there on an Apple M5 Max), so there a shallower row only pins the output.
#[test]
fn test_deep_recursion_from_a_moved_fold_7267() -> Result<()> {
    let depth = if cfg!(debug_assertions) { 2_000 } else { 3_240 };
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("d.json");
    let doc: Vec<String> = (1..=200).map(|n| n.to_string()).collect();
    std::fs::write(&path, format!("[{}]", doc.join(",")))?;
    let filter = format!(
        ". as $d | def f(n): if n == 0 then 0 else [$d[] | if . == 100 then ($d | f(n - 1)) else . + 0 end] | .[0] end; f({depth})"
    );
    let got = run_jq_full(&[&filter, path.to_str().expect("utf-8 temp path")], None)?;
    assert_eq!(got, ("1\n".to_string(), String::new(), 0));
    Ok(())
}
