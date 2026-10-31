//! #5655: a container where an object key belongs (`{[1]`, `{{}`) is a value to jq's plain reader,
//! which meets the fault at the token after it; `--stream`'s parser refuses the bracket where it
//! opens. #5645 moved a number or literal to the next token for both and left this case at the
//! bracket. Rows are `(stdin, plain output, --stream output)` captured from jq 1.7.1
//! (`/usr/bin/jq`) with `-nc '[limit(6; repeat(try input catch "E"))]'`.

use anyhow::Result;

use crate::run_jq_full;

const CATCH_INPUTS: &str = "[limit(6; repeat(try input catch \"E\"))]";

const ROWS: &[(&str, &str, &str)] = &[
    (
        "{[1]\n2\n3",
        r#"["E",3,"E","E","E","E"]"#,
        r#"["E",[[],2],[[],3],"E","E","E"]"#,
    ),
    (
        "{{}\n2\n3",
        r#"["E",3,"E","E","E","E"]"#,
        r#"["E",[[],2],[[],3],"E","E","E"]"#,
    ),
    // The container spans lines: the fault is met after its closing bracket.
    (
        "{[\n1]\n2\n3",
        r#"["E",3,"E","E","E","E"]"#,
        r#"["E","E",[[],2],[[],3],"E","E"]"#,
    ),
    (
        "{ [8]\n9",
        r#"["E","E","E","E","E","E"]"#,
        r#"["E",[[],9],"E","E","E","E"]"#,
    ),
    // A structural byte after it is met at itself, across blank lines.
    (
        "{[1] \n\n:\n5\n6",
        r#"["E",5,6,"E","E","E"]"#,
        r#"["E","E",[[],5],[[],6],"E","E"]"#,
    ),
    (
        "{[1]\n]\n3",
        r#"["E",3,"E","E","E","E"]"#,
        r#"["E","E",[[],3],"E","E","E"]"#,
    ),
    // A fault inside the container is met first, and nothing after it: the end of input.
    (
        "{[1,x\n2\n3",
        r#"["E",2,3,"E","E","E"]"#,
        r#"["E",[[],2],[[],3],"E","E","E"]"#,
    ),
    (
        "{[1 2]\n3\n4",
        r#"["E",3,4,"E","E","E"]"#,
        r#"["E",[[],3],[[],4],"E","E","E"]"#,
    ),
    (
        "{[1]",
        r#"["E","E","E","E","E","E"]"#,
        r#"["E","E","E","E","E","E"]"#,
    ),
    (
        "{[1]\n\n",
        r#"["E","E","E","E","E","E"]"#,
        r#"["E","E","E","E","E","E"]"#,
    ),
    // After a comma, and with an earlier member (its events come before the fault under
    // `--stream`, #5635).
    (
        "{\"a\":1,[1]\n2\n3",
        r#"["E",3,"E","E","E","E"]"#,
        r#"[[["a"],1],"E",[[],2],[[],3],"E","E"]"#,
    ),
    (
        "{\"a\":1,{}\n\"k\"\n4",
        r#"["E",4,"E","E","E","E"]"#,
        r#"[[["a"],1],"E",[[],"k"],[[],4],"E","E"]"#,
    ),
];

#[test]
fn test_a_container_in_key_position_is_met_by_each_reader_where_jq_meets_it_5655() -> Result<()> {
    for (stdin, plain, stream) in ROWS {
        for (flags, expected) in [(vec!["-nc"], plain), (vec!["-nc", "--stream"], stream)] {
            let mut args = flags.clone();
            args.push(CATCH_INPUTS);
            let (out, err, code) = run_jq_full(&args, Some(stdin))?;
            assert_eq!(
                (out.trim_end(), err.as_str(), code),
                (*expected, "", 0),
                "{flags:?} {stdin:?}"
            );
        }
    }
    Ok(())
}
