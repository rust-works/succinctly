//! #6350: `select(cond)` answers a `cond` in the direct grammar (`true`/`false`/`null`, `.`, `not`,
//! `and`/`or`, and a comparison between a literal and `.` or `type`) from the node, without the
//! sink, the fan-out and the owned copy the general route makes. `COND | .` is the same condition
//! outside that grammar (a pipe that is not `| not`), so it takes the general route, and the two
//! must keep and drop the same nodes. The expectations are what `/usr/bin/jq` 1.7.1 prints.

use anyhow::Result;

use crate::run_jq_full;

const DOC: &str = r#"{"a":null,"b":[null,true,false,0,1,-1,1.5,"a","","null",[],[1],[null],{},{"k":null}],"c":{"d":{"e":null,"f":"x"}},"g":1e1000,"h":"é"}"#;

const CONDS: &[&str] = &[
    ". == null",
    ". != null",
    "null == .",
    ". == true",
    ". != false",
    ". == 0",
    ". < 1",
    ". >= 1.5",
    ". == \"a\"",
    ". != \"\"",
    "\"a\" < .",
    ". == 1e1000",
    "type == \"null\"",
    "type != \"object\"",
    "type == \"string\" and . != \"\"",
    "type == \"array\" or type == \"object\"",
    ". == null or . == false",
    ". != null and . != false",
    "not",
    ".",
    "(. == null)",
    "true",
    "false",
    "null",
];

/// The direct route and the general route agree on every node, whichever way `select` is reached.
#[test]
fn test_select_agrees_with_its_general_route_6350() -> Result<()> {
    for cond in CONDS {
        for shape in [
            "[.. | select(COND)]",
            "[.. | select(COND)] | length",
            "[.[]? | select(COND)]",
            "map(select(COND))?",
            "[first(.. | select(COND))?]",
            "[.. | select(COND) | type]",
            "del(.. | select(COND)) | length",
        ] {
            let direct = shape.replace("COND", cond);
            let general = shape.replace("COND", &format!("{cond} | ."));
            let (a, ea, ca) = run_jq_full(&["-c", &direct], Some(DOC))?;
            let (b, eb, cb) = run_jq_full(&["-c", &general], Some(DOC))?;
            assert_eq!((ca, &a), (cb, &b), "{direct}: {ea} / {eb}");
        }
    }
    Ok(())
}

/// A few answers pinned against jq itself.
#[test]
fn test_select_direct_conditions_answer_as_jq_does_6350() -> Result<()> {
    const ROWS: &[(&str, &str)] = &[
        ("[.. | select(. == null)] | length", "5\n"),
        ("[.. | select(. != null)] | length", "22\n"),
        ("[.[]? | select(type == \"string\")]", "[\"é\"]\n"),
        ("[.b[] | select(. == true)]", "[true]\n"),
        ("[.b[] | select(. and . != 0)] | length", "12\n"),
        ("[.b[] | select(not)] | length", "2\n"),
        ("[.. | select(. == 1e1000)]", "[1E+1000]\n"),
        (
            "[.b[] | select(type == \"array\" or type == \"object\")] | length",
            "5\n",
        ),
    ];
    for &(program, expected) in ROWS {
        let (out, err, code) = run_jq_full(&["-c", program], Some(DOC))?;
        assert_eq!(code, 0, "{program}: {err}");
        assert_eq!(out, expected, "{program}");
    }
    Ok(())
}
