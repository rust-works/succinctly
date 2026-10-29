//! #5483: yq's `alias` answers the node's `Value`: the anchor's name for an alias node (`*name`),
//! and for any other scalar its text as written (`1.50`, `0x1F`, `~`; a quoted or block scalar
//! decoded), `""` for a mapping, a sequence and an empty value. Rows were captured from yq
//! v4.53.3.

use anyhow::Result;

use crate::{run_jq_stdin, run_yq_stdin};

#[test]
fn test_alias_getter_matches_yq_5483() -> Result<()> {
    let rows: &[(&str, &str, &[&str], &str)] = &[
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2\nd: *y\n",
            ".b | alias",
            &[],
            "x\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2\nd: *y\n",
            ".a | alias",
            &[],
            "\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2\nd: *y\n",
            ".d | alias",
            &[],
            "y\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2\nd: *y\n",
            ".c | alias",
            &[],
            "2\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2\nd: *y\n",
            "[.. | alias]",
            &[],
            "- \"\"\n- \"\"\n- \"1\"\n- x\n- \"2\"\n- y\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2\nd: *y\n",
            "[.[] | alias]",
            &["-o=json", "-I=0"],
            "[\"\",\"x\",\"2\",\"y\"]\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2\nd: *y\n",
            "[.[] | select(alias == \"y\") | path]",
            &[],
            "- - d\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2\nd: *y\n",
            "[.[] | select(alias != \"\") | path]",
            &[],
            "- - b\n- - c\n- - d\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2\nd: *y\n",
            "(.[] | select(alias == \"x\")) = 9",
            &[],
            "a: &x {p: 1}\nb: 9\nc: &y 2\nd: *y\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2\nd: *y\n",
            "del(.[] | select(alias != \"\"))",
            &[],
            "a: &x {p: 1}\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2\nd: *y\n",
            ".d | {\"n\": alias, \"k\": kind}",
            &[],
            "n: y\nk: alias\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2\nd: *y\n",
            ".d | alias == \"y\"",
            &[],
            "true\n",
        ),
        (
            "a: 1\n",
            "alias",
            &[],
            "\n",
        ),
        (
            "int: 1\nfloat: 1.50\nhex: 0x1F\nbool: true\ntilde: ~\nnul: null\nempty:\ndq: \"q\\tx\"\nsq: 'it''s'\nplain: hello world\nlit: |\n  line1\n  line2\nfold: >-\n  a\n  b\ntagged: !!str 1\nanch: &a 5\nflowseq: [1, 2]\nflowmap: {k: v}\nblockseq:\n  - 1\nmap:\n  k: v\nts: 2001-12-14\nbin: !!binary aGVsbG8=\nlong: 12345678901234567890\ninf: .inf\nneg: -0\nali: *a\n",
            "[.[] | alias]",
            &["-o=json", "-I=0"],
            "[\"1\",\"1.50\",\"0x1F\",\"true\",\"~\",\"null\",\"\",\"q\\tx\",\"it's\",\"hello world\",\"line1\\nline2\\n\",\"a b\",\"1\",\"5\",\"\",\"\",\"\",\"\",\"2001-12-14\",\"aGVsbG8=\",\"12345678901234567890\",\".inf\",\"-0\",\"a\"]\n",
        ),
        (
            "int: 1\nfloat: 1.50\nhex: 0x1F\nbool: true\ntilde: ~\nnul: null\nempty:\ndq: \"q\\tx\"\nsq: 'it''s'\nplain: hello world\nlit: |\n  line1\n  line2\nfold: >-\n  a\n  b\ntagged: !!str 1\nanch: &a 5\nflowseq: [1, 2]\nflowmap: {k: v}\nblockseq:\n  - 1\nmap:\n  k: v\nts: 2001-12-14\nbin: !!binary aGVsbG8=\nlong: 12345678901234567890\ninf: .inf\nneg: -0\nali: *a\n",
            "[.. | alias]",
            &["-o=json", "-I=0"],
            "[\"\",\"1\",\"1.50\",\"0x1F\",\"true\",\"~\",\"null\",\"\",\"q\\tx\",\"it's\",\"hello world\",\"line1\\nline2\\n\",\"a b\",\"1\",\"5\",\"\",\"1\",\"2\",\"\",\"v\",\"\",\"1\",\"\",\"v\",\"2001-12-14\",\"aGVsbG8=\",\"12345678901234567890\",\".inf\",\"-0\",\"a\"]\n",
        ),
        (
            "int: 1\nfloat: 1.50\nhex: 0x1F\nbool: true\ntilde: ~\nnul: null\nempty:\ndq: \"q\\tx\"\nsq: 'it''s'\nplain: hello world\nlit: |\n  line1\n  line2\nfold: >-\n  a\n  b\ntagged: !!str 1\nanch: &a 5\nflowseq: [1, 2]\nflowmap: {k: v}\nblockseq:\n  - 1\nmap:\n  k: v\nts: 2001-12-14\nbin: !!binary aGVsbG8=\nlong: 12345678901234567890\ninf: .inf\nneg: -0\nali: *a\n",
            ".int |= alias",
            &[],
            "int: \"1\"\nfloat: 1.50\nhex: 0x1F\nbool: true\ntilde: ~\nnul: null\nempty:\ndq: \"q\\tx\"\nsq: 'it''s'\nplain: hello world\nlit: |\n  line1\n  line2\nfold: >-\n  a b\ntagged: !!str 1\nanch: &a 5\nflowseq: [1, 2]\nflowmap: {k: v}\nblockseq:\n  - 1\nmap:\n  k: v\nts: 2001-12-14\nbin: !!binary aGVsbG8=\nlong: 12345678901234567890\ninf: .inf\nneg: -0\nali: *a\n",
        ),
        (
            "int: 1\nfloat: 1.50\nhex: 0x1F\nbool: true\ntilde: ~\nnul: null\nempty:\ndq: \"q\\tx\"\nsq: 'it''s'\nplain: hello world\nlit: |\n  line1\n  line2\nfold: >-\n  a\n  b\ntagged: !!str 1\nanch: &a 5\nflowseq: [1, 2]\nflowmap: {k: v}\nblockseq:\n  - 1\nmap:\n  k: v\nts: 2001-12-14\nbin: !!binary aGVsbG8=\nlong: 12345678901234567890\ninf: .inf\nneg: -0\nali: *a\n",
            ".plain |= alias",
            &[],
            "int: 1\nfloat: 1.50\nhex: 0x1F\nbool: true\ntilde: ~\nnul: null\nempty:\ndq: \"q\\tx\"\nsq: 'it''s'\nplain: hello world\nlit: |\n  line1\n  line2\nfold: >-\n  a b\ntagged: !!str 1\nanch: &a 5\nflowseq: [1, 2]\nflowmap: {k: v}\nblockseq:\n  - 1\nmap:\n  k: v\nts: 2001-12-14\nbin: !!binary aGVsbG8=\nlong: 12345678901234567890\ninf: .inf\nneg: -0\nali: *a\n",
        ),
        (
            "a: &x 1\nb: *x\n---\nc: &y 2\nd: *y\n",
            ".[] | alias",
            &[],
            "1\nx\n---\n2\ny\n",
        ),
    ];
    for &(input, filter, args, want) in rows {
        let (out, code) = run_yq_stdin(filter, input, args)?;
        assert_eq!(
            (out.as_str(), code),
            (want, 0),
            "{args:?} {filter} over {input:?}"
        );
    }
    Ok(())
}

#[test]
fn test_alias_is_not_a_jq_builtin_5483() -> Result<()> {
    // jq 1.7.1: `alias/0 is not defined` (exit 3).
    let (out, code) = run_jq_stdin("alias", "{}", &["-c"])?;
    assert_eq!((out.as_str(), code), ("", 3));
    Ok(())
}
