//! #7260: the per-record paths `@tsv`, a field lookup, `--stream` and the lazy `inputs` scan were
//! rewritten for speed: an `@tsv` row is written into one string, a key is compared without decoding
//! it, the stream reader hands out each event before reading on, and a lazy document of plain text
//! is checked in one pass. Every expectation here was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// Every escape `@tsv` writes (backslash, tab, line feed, carriage return, NUL), next to null,
/// booleans and numbers that keep their spelling.
#[test]
fn tsv_row_escapes_and_spells_every_field_7260() -> Result<()> {
    let input = r#"["a\\b","t\tx","n\nx","r\rx","z\u0000y",null,true,false,1.50,-0,1e3,100000000000000000001,"q\"q"]"#;
    let (stdout, stderr, code) = run_jq_full(&["-r", "@tsv"], Some(input))?;
    let fields: [&str; 13] = [
        r"a\\b",
        r"t\tx",
        r"n\nx",
        r"r\rx",
        r"z\0y",
        r"",
        r"true",
        r"false",
        r"1.50",
        r"-0",
        r"1E+3",
        r"100000000000000000001",
        r#"q"q"#,
    ];
    assert_eq!(stdout, format!("{}\n", fields.join("\t")));
    assert_eq!((stderr.as_str(), code), ("", 0));
    // A container field still rejects the row, in jq's wording.
    let (stdout, stderr, code) = run_jq_full(&["-r", "@tsv"], Some("[[1]]\n[{\"a\":1}]\n"))?;
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "jq: error (at <stdin>:1): array ([1]) is not valid in a csv row\n\
         jq: error (at <stdin>:2): object ({\"a\":1}) is not valid in a csv row\n"
    );
    assert_eq!(code, 5);
    Ok(())
}

/// A key that shares a prefix with the name, one spelled with an escape, and the last of two
/// that decode to the same name.
#[test]
fn field_lookup_matches_keys_as_they_decode_7260() -> Result<()> {
    let input = r#"{"abc":1,"ab":2,"a":3}
{"a\u0062":1,"ab":2}
{"ab":1,"a\u0062":2}
{"x":{"ab":5}}
"#;
    let (stdout, _, code) = run_jq_full(&["-c", "[.ab, .a, .abc, .x.ab]"], Some(input))?;
    assert_eq!(
        stdout,
        "[2,3,1,null]\n[2,null,null,null]\n[2,null,null,null]\n[null,null,null,5]\n"
    );
    assert_eq!(code, 0);
    Ok(())
}

/// A one-line document is one refill of a few hundred events: they come out in order, and a
/// program that reads one and stops reads one.
#[test]
fn stream_events_of_one_refill_come_out_in_order_7260() -> Result<()> {
    let line = format!(
        "[{}]\n",
        (0..300)
            .map(|i| format!("{{\"k\":{i}}}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let (stdout, _, code) = run_jq_full(&["-c", "--stream", "."], Some(&line))?;
    let mut expected: String = (0..300)
        .map(|i| format!("[[{i},\"k\"],{i}]\n[[{i},\"k\"]]\n"))
        .collect();
    expected.push_str("[[299]]\n");
    assert_eq!(stdout, expected);
    assert_eq!(code, 0);
    let (stdout, _, code) = run_jq_full(
        &[
            "-c",
            "--stream",
            "-n",
            "first(inputs), input, input_line_number",
        ],
        Some(&line),
    )?;
    assert_eq!(stdout, "[[0,\"k\"],0]\n[[0,\"k\"]]\n1\n");
    assert_eq!(code, 0);
    Ok(())
}

/// Plain records, then one the scan cannot read: the records before it are inputs, and the fault
/// is jq's, at its line.
#[test]
fn lazy_inputs_before_a_fault_are_read_and_the_fault_is_jqs_7260() -> Result<()> {
    let input = "{\"id\":1,\"v\":\"a\"}\n{\"id\":2,\"v\":\"b\"}\n{\"id\":3,\"v\":}\n{\"id\":4}\n";
    let (stdout, stderr, code) = run_jq_full(&["-c", "."], Some(input))?;
    assert_eq!(stdout, "{\"id\":1,\"v\":\"a\"}\n{\"id\":2,\"v\":\"b\"}\n");
    assert_eq!(
        stderr,
        "jq: parse error: Unmatched '}' at line 3, column 13\n"
    );
    assert_eq!(code, 5);
    let (stdout, stderr, code) = run_jq_full(&["-n", "-c", "[inputs]"], Some(input))?;
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "jq: error (at <stdin>:3): Unmatched '}' at line 3, column 13\n"
    );
    assert_eq!(code, 5);
    // Scalars, an empty array and a repeated key between plain records, each at its line.
    let input = "{\"a\":1}\n7\n\"s\"\n{\"b\":[1,{\"c\":null}]}\n[]\n{\"a\":1,\"a\":2}\n";
    let (stdout, _, code) = run_jq_full(&["-c", "[., input_line_number]"], Some(input))?;
    assert_eq!(
        stdout,
        "[{\"a\":1},1]\n[7,2]\n[\"s\",3]\n[{\"b\":[1,{\"c\":null}]},4]\n[[],5]\n[{\"a\":2},6]\n"
    );
    assert_eq!(code, 0);
    Ok(())
}
