//! #7267: an array built from a value computed per element of a document array (`[.xs[] | floor]`)
//! keeps one 32-byte owned value per element while it is built, not an 80-byte per-element result
//! beside a second, doubling vector. The batch now coalesces owned results into runs and drops empty
//! ones, so these rows pin what must not move with it: order across cursors and computed values,
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
