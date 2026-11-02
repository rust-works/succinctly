//! #5263: a `reduce`/`foreach` assignment whose key, right side or update filter reads the
//! accumulator is written in place. These pin that the answers did not move -- above all that a
//! key which reads `.` reads the value its own stage starts from, as jq runs it (captured live
//! from jq 1.7.1).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_state_reading_operands_answer_as_jq_5263() -> Result<()> {
    let records = r#"[{"name":"a","score":1},{"name":"b","score":2},{"name":"a","score":3},{"name":"c","score":null}]"#;
    for (input, filter, want) in [
        (
            records,
            "reduce .[] as $r ({}; .[$r.name] |= (. // 0) + 1)",
            r#"{"a":2,"b":1,"c":1}"#,
        ),
        (
            records,
            "reduce .[] as $r ({}; .[$r.name] |= if . == null then 1 else . + 1 end)",
            r#"{"a":2,"b":1,"c":1}"#,
        ),
        (
            records,
            "reduce .[] as $r ([]; .[length] = $r.score)",
            "[1,2,3,null]",
        ),
        (
            records,
            "reduce .[] as $r ({}; .[length | tostring] = $r.score)",
            r#"{"0":1,"1":2,"2":3,"3":null}"#,
        ),
        (
            records,
            "reduce .[] as $r ({}; .[$r.name] = length)",
            r#"{"a":2,"b":1,"c":2}"#,
        ),
        (
            records,
            "[foreach .[] as $r ([]; .[length] = $r.score; length)]",
            "[1,2,3,4]",
        ),
        // A filter that calls a parameterless `def`, on the shipped route.
        (
            records,
            "def inc: (. // 0) + 1; reduce .[] as $r ({}; .[$r.name] |= inc)",
            r#"{"a":2,"b":1,"c":1}"#,
        ),
        (
            "null",
            r#"reduce range(2) as $i ({"a":[5,6,7],"b":0}; (.a | .[length]) = $i)"#,
            r#"{"a":[5,6,7,0,1],"b":0}"#,
        ),
        (
            "null",
            r#"reduce range(2) as $i ({"a":[5,6,7],"b":1}; .a[.b] = $i)"#,
            r#"{"a":[5,1,7],"b":1}"#,
        ),
        (
            "null",
            r#"reduce range(3) as $i ({"k":"z","a":{"z":0}}; (.a)[.k] += 1)"#,
            r#"{"k":"z","a":{"z":3}}"#,
        ),
    ] {
        let (stdout, stderr, code) =
            run_jq_full(&["-n", "-c", &format!("{input} | {filter}")], None)?;
        assert_eq!((stdout.trim_end(), code), (want, 0), "`{filter}`: {stderr}");
    }
    Ok(())
}

/// Targets that need the path-mode interpreter (a key that calls a `def`, `getpath`), which writes
/// them in place once the paths are known to be pure, and answers `length`/`type` of the state
/// itself instead of re-serializing it for the shipped evaluator. As jq 1.7.1 answers them
/// (captured live; the holdout refuses every dynamic target).
#[test]
fn test_interpreter_defs_length_and_type_answer_as_jq_5263() -> Result<()> {
    let records = r#"[{"name":"a","score":1},{"name":"b","score":2},{"name":"a","score":3},{"name":"c","score":null}]"#;
    for (input, filter, want) in [
        (
            records,
            "def f: ascii_downcase; reduce .[] as $r ({}; .[$r.name | f] = $r.score)",
            r#"{"a":3,"b":2,"c":null}"#,
        ),
        (
            records,
            "def f: ascii_downcase; reduce .[] as $r ({}; .[$r.name | f] |= (. // 0) + 1)",
            r#"{"a":2,"b":1,"c":1}"#,
        ),
        (
            records,
            "def f: length; reduce .[] as $r ([]; .[f] = $r.score)",
            "[1,2,3,null]",
        ),
        (
            records,
            r#"def f: if type == "string" then . else f end; reduce .[] as $r ({}; .[$r.name | f] = $r.score)"#,
            r#"{"a":3,"b":2,"c":null}"#,
        ),
        (
            records,
            r"reduce .[] as $r ({}; getpath([length | tostring]) = ($r.name | length))",
            r#"{"0":1,"1":1,"2":1,"3":1}"#,
        ),
        (
            records,
            "reduce .[] as $r ([]; getpath([length]) = (. | type))",
            r#"["array","array","array","array"]"#,
        ),
        (
            records,
            r#"reduce .[] as $r (""; getpath([]) = (. + $r.name | length | tostring))"#,
            r#""2""#,
        ),
    ] {
        let (stdout, stderr, code) =
            run_jq_full(&["-n", "-c", &format!("{input} | {filter}")], None)?;
        assert_eq!((stdout.trim_end(), code), (want, 0), "`{filter}`: {stderr}");
    }
    Ok(())
}

/// `length` and `type` of every kind of value, through the interpreter (a `getpath` target):
/// answered in place for a string (by codepoints), array, object and `null`, delegated for a number
/// and a boolean, which raises. As jq 1.7.1 answers them (captured live).
#[test]
fn test_interpreter_length_and_type_of_every_kind_answer_as_jq_5263() -> Result<()> {
    for (value, length, kind) in [
        ("null", "0", r#""null""#),
        (r#""é😀""#, "2", r#""string""#),
        (r#""""#, "0", r#""string""#),
        ("true", "", r#""boolean""#),
        ("12", "12", r#""number""#),
        ("-3.5", "3.5", r#""number""#),
        ("[1,2]", "2", r#""array""#),
        (r#"{"a":1}"#, "1", r#""object""#),
    ] {
        let program = |what: &str| {
            format!(r#"reduce ([{value}]) as [$v] (null; getpath(["k"]) = ($v | {what}))"#)
        };
        let (stdout, stderr, code) = run_jq_full(&["-n", "-c", &program("type")], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (format!(r#"{{"k":{kind}}}"#).as_str(), 0),
            "{value} | type: {stderr}"
        );
        let (stdout, stderr, code) = run_jq_full(&["-n", "-c", &program("length")], None)?;
        if length.is_empty() {
            assert_eq!((stdout.as_str(), code), ("", 5), "{value} | length");
            assert!(
                stderr.contains("boolean (true) has no length"),
                "{value}: {stderr}"
            );
        } else {
            assert_eq!(
                (stdout.trim_end(), code),
                (format!(r#"{{"k":{length}}}"#).as_str(), 0),
                "{value} | length: {stderr}"
            );
        }
    }
    Ok(())
}

/// A stage's key reads the stage's own input: in `(.a | .[.b])` it is `.a` indexed by the string
/// `"b"`, which raises -- the root's `.b` is not what the key sees.
#[test]
fn test_pipe_stage_key_reads_its_own_input_5263() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-n",
            "-c",
            r#"reduce range(2) as $i ({"a":[5,6,7],"b":0}; (.a | .[.b]) = $i)"#,
        ],
        None,
    )?;
    assert_eq!((stdout.as_str(), code), ("", 5), "{stderr}");
    assert!(
        stderr.contains(r#"Cannot index array with string "b""#),
        "{stderr}"
    );
    Ok(())
}
