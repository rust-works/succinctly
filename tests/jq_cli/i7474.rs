//! #7474: a builtin whose argument jq binds once per value (`ltrimstr(g)`, `test(g)`, `pow(a; b)`,
//! `setpath(p; v)`, ...) ran a generator argument to the end before its first output, so
//! `first(ltrimstr(repeat("x")))` never answered and `ltrimstr(inputs | tostring)` read every
//! input. The parser now binds such an argument first, `g as $v | B($v)`, in jq's own nesting
//! order (`Builtin::value_args_mut`).
//!
//! [`CASES`] is generated, not hand-picked: every builtin of arity >= 1 in
//! `tests/data/jq-builtin-names.txt` was run with an endless generator in each argument position
//! (`first(B(.. repeat(v) ..))`, `[limit(2; B(.. repeat(v1, v2) ..))]`) against jq 1.7.1 and the
//! unfixed build, with a 2-second limit. A position where jq answered and succinctly did not is
//! listed with those two rows, the position's error row (`B(.. (v1, error("E"), v2) ..)`: the
//! first value's output, then the error, exit 5) and the builtin's order row over a finite
//! cartesian product (`[B((a1, a2); (b1, b2))]`, which pins jq's per-builtin nesting order).
//! Every expected value is jq 1.7.1's (`/usr/bin/jq -nc`, `TZ=UTC`); jq 1.8.2 prints the same.

use std::fs;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use crate::succinctly_bin;

/// Long enough for a debug build on a loaded runner; an endless generator never ends at all.
const LIMIT: Duration = Duration::from_secs(20);

/// `succinctly <mode> <args>` with a wall-clock limit, so a regression to the eager argument
/// fails the test instead of hanging the suite and growing memory. `None` is a timeout.
fn run_bounded(mode: &str, args: &[&str], stdin: &str) -> Result<Option<(String, String, i32)>> {
    let mut child = Command::new(succinctly_bin())
        .arg(mode)
        .args(args)
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    {
        use std::io::Write;
        let mut input = child.stdin.take().expect("stdin is piped");
        input.write_all(stdin.as_bytes())?;
    }
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            let output = child.wait_with_output()?;
            return Ok(Some((
                String::from_utf8(output.stdout)?,
                String::from_utf8(output.stderr)?,
                output.status.code().unwrap_or(-1),
            )));
        }
        if start.elapsed() > LIMIT {
            child.kill()?;
            child.wait()?;
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_generator_argument_is_bound_one_value_at_a_time_as_jq_does_7474() -> Result<()> {
    let mut failures = Vec::new();
    let mut timeouts = 0;
    for &(program, expected, code) in CASES {
        match run_bounded("jq", &["-nc", program], "")? {
            Some((stdout, stderr, got)) => {
                if stdout != expected || got != code {
                    failures.push(format!(
                        "{program}\n  want {expected:?} exit {code}\n  got  {stdout:?} exit {got}, stderr {stderr:?}"
                    ));
                } else if code == 5 && !stderr.contains("E") {
                    failures.push(format!(
                        "{program}: stderr {stderr:?} does not name the error"
                    ));
                }
            }
            None => {
                failures.push(format!("{program}: still running after {LIMIT:?}"));
                timeouts += 1;
                // One timeout already proves the regression; do not spend 20 s on each row.
                if timeouts == 3 {
                    break;
                }
            }
        }
    }
    if !failures.is_empty() {
        bail!(
            "{} of {} rows differ from jq 1.7.1:\n{}",
            failures.len(),
            CASES.len(),
            failures.join("\n")
        );
    }
    Ok(())
}

/// The argument pulls one input at a time: the first call's output reaches `first` before the
/// second input is read, so `input_line_number` is still 1 (jq 1.7.1 and 1.8.2 both print it).
#[test]
fn inputs_in_an_argument_is_read_one_value_at_a_time_7474() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let six = dir.path().join("six.json");
    fs::write(&six, "1\n2\n3\n4\n5\n6\n")?;
    let six = six.to_str().expect("utf-8 temp path");
    for (program, expected) in [
        (
            r#""x" | first(ltrimstr(inputs | tostring)), input_line_number"#,
            "\"x\"\n1\n",
        ),
        (
            r#""a,b" | first(splits(inputs | tostring)), input_line_number"#,
            "\"a,b\"\n1\n",
        ),
    ] {
        let got = run_bounded("jq", &["-nc", program, six], "")?;
        let Some((stdout, stderr, code)) = got else {
            bail!("{program}: still running after {LIMIT:?}");
        };
        assert_eq!(
            (stdout.as_str(), code),
            (expected, 0),
            "{program}: stderr {stderr:?}"
        );
    }
    Ok(())
}

/// The binding is jq mode's: real yq takes the first value of `has`'s argument (v4.53.3 prints
/// `true` for this), and `succinctly yq` keeps doing so.
#[test]
fn yq_mode_still_takes_the_first_value_of_has_7474() -> Result<()> {
    let got = run_bounded("yq", &[r#"has(("a", "b"))"#], "a: 1\n")?;
    let Some((stdout, stderr, code)) = got else {
        bail!("succinctly yq: still running after {LIMIT:?}");
    };
    assert_eq!((stdout.as_str(), code), ("true\n", 0), "stderr {stderr:?}");
    Ok(())
}

/// `(program, jq 1.7.1's stdout, jq 1.7.1's exit status)`, generated as the module doc says.
const CASES: &[(&str, &str, i32)] = &[
    (
        r#"0 | first(strflocaltime(repeat("%Y")))"#,
        r#""1970"
"#,
        0,
    ),
    (
        r#"0 | [limit(2; strflocaltime(repeat("%Y", "%j")))]"#,
        r#"["1970","001"]
"#,
        0,
    ),
    (
        r#"0 | strflocaltime(("%Y", error("E"), "%j"))"#,
        r#""1970"
"#,
        5,
    ),
    (
        r#"0 | [strflocaltime(("%Y", "%j"))]"#,
        r#"["1970","001"]
"#,
        0,
    ),
    (
        r#"0 | first(strftime(repeat("%Y")))"#,
        r#""1970"
"#,
        0,
    ),
    (
        r#"0 | [limit(2; strftime(repeat("%Y", "%j")))]"#,
        r#"["1970","001"]
"#,
        0,
    ),
    (
        r#"0 | strftime(("%Y", error("E"), "%j"))"#,
        r#""1970"
"#,
        5,
    ),
    (
        r#"0 | [strftime(("%Y", "%j"))]"#,
        r#"["1970","001"]
"#,
        0,
    ),
    (
        r#""2015" | first(strptime(repeat("%Y")))"#,
        r#"[2015,0,0,0,0,0,3,-1]
"#,
        0,
    ),
    (
        r#""2015" | [limit(2; strptime(repeat("%Y", "%C%y")))]"#,
        r#"[[2015,0,0,0,0,0,3,-1],[2015,0,0,0,0,0,3,-1]]
"#,
        0,
    ),
    (
        r#""2015" | strptime(("%Y", error("E"), "%C%y"))"#,
        r#"[2015,0,0,0,0,0,3,-1]
"#,
        5,
    ),
    (
        r#""2015" | [strptime(("%Y", "%C%y"))]"#,
        r#"[[2015,0,0,0,0,0,3,-1],[2015,0,0,0,0,0,3,-1]]
"#,
        0,
    ),
    (
        r#"[1] | first(format(repeat("text")))"#,
        r#""[1]"
"#,
        0,
    ),
    (
        r#"[1] | [limit(2; format(repeat("text", "json")))]"#,
        r#"["[1]","[1]"]
"#,
        0,
    ),
    (
        r#"[1] | format(("text", error("E"), "json"))"#,
        r#""[1]"
"#,
        5,
    ),
    (
        r#"[1] | [format(("text", "json"))]"#,
        r#"["[1]","[1]"]
"#,
        0,
    ),
    (
        r#""abc" | first(contains(repeat("a")))"#,
        r#"true
"#,
        0,
    ),
    (
        r#""abc" | [limit(2; contains(repeat("a", "z")))]"#,
        r#"[true,false]
"#,
        0,
    ),
    (
        r#""abc" | contains(("a", error("E"), "z"))"#,
        r#"true
"#,
        5,
    ),
    (
        r#""abc" | [contains(("a", "z"))]"#,
        r#"[true,false]
"#,
        0,
    ),
    (
        r#"{"a":1} | first(has(repeat("a")))"#,
        r#"true
"#,
        0,
    ),
    (
        r#"{"a":1} | [limit(2; has(repeat("a", "b")))]"#,
        r#"[true,false]
"#,
        0,
    ),
    (
        r#"{"a":1} | has(("a", error("E"), "b"))"#,
        r#"true
"#,
        5,
    ),
    (
        r#"{"a":1} | [has(("a", "b"))]"#,
        r#"[true,false]
"#,
        0,
    ),
    (
        r#"{"a":1,"b":2} | first(delpaths(repeat([["a"]])))"#,
        r#"{"b":2}
"#,
        0,
    ),
    (
        r#"{"a":1,"b":2} | [limit(2; delpaths(repeat([["a"]], [["b"]])))]"#,
        r#"[{"b":2},{"a":1}]
"#,
        0,
    ),
    (
        r#"{"a":1,"b":2} | delpaths(([["a"]], error("E"), [["b"]]))"#,
        r#"{"b":2}
"#,
        5,
    ),
    (
        r#"{"a":1,"b":2} | [delpaths(([["a"]], [["b"]]))]"#,
        r#"[{"b":2},{"a":1}]
"#,
        0,
    ),
    (
        r#"{"a":1,"b":2} | first(getpath(repeat(["a"])))"#,
        r#"1
"#,
        0,
    ),
    (
        r#"{"a":1,"b":2} | [limit(2; getpath(repeat(["a"], ["b"])))]"#,
        r#"[1,2]
"#,
        0,
    ),
    (
        r#"{"a":1,"b":2} | getpath((["a"], error("E"), ["b"]))"#,
        r#"1
"#,
        5,
    ),
    (
        r#"{"a":1,"b":2} | [getpath((["a"], ["b"]))]"#,
        r#"[1,2]
"#,
        0,
    ),
    (
        r#"{"a":1} | first(setpath(repeat(["a"]); 1))"#,
        r#"{"a":1}
"#,
        0,
    ),
    (
        r#"{"a":1} | [limit(2; setpath(repeat(["a"], ["b"]); 1))]"#,
        r#"[{"a":1},{"a":1,"b":1}]
"#,
        0,
    ),
    (
        r#"{"a":1} | setpath((["a"], error("E"), ["b"]); 1)"#,
        r#"{"a":1}
"#,
        5,
    ),
    (
        r#"{"a":1} | first(setpath(["a"]; repeat(1)))"#,
        r#"{"a":1}
"#,
        0,
    ),
    (
        r#"{"a":1} | [limit(2; setpath(["a"]; repeat(1, 2)))]"#,
        r#"[{"a":1},{"a":2}]
"#,
        0,
    ),
    (
        r#"{"a":1} | setpath(["a"]; (1, error("E"), 2))"#,
        r#"{"a":1}
"#,
        5,
    ),
    (
        r#"{"a":1} | [setpath((["a"], ["b"]); (1, 2))]"#,
        r#"[{"a":1},{"a":1,"b":1},{"a":2},{"a":1,"b":2}]
"#,
        0,
    ),
    (
        r#""a,b c" | first(split(repeat(",")))"#,
        r#"["a","b c"]
"#,
        0,
    ),
    (
        r#""a,b c" | [limit(2; split(repeat(",", " ")))]"#,
        r#"[["a","b c"],["a,b","c"]]
"#,
        0,
    ),
    (
        r#""a,b c" | split((",", error("E"), " "))"#,
        r#"["a","b c"]
"#,
        5,
    ),
    (
        r#""a,b c" | [split((",", " "))]"#,
        r#"[["a","b c"],["a,b","c"]]
"#,
        0,
    ),
    (
        r#""xab" | first(rtrimstr(repeat("b")))"#,
        r#""xa"
"#,
        0,
    ),
    (
        r#""xab" | [limit(2; rtrimstr(repeat("b", "ab")))]"#,
        r#"["xa","x"]
"#,
        0,
    ),
    (
        r#""xab" | rtrimstr(("b", error("E"), "ab"))"#,
        r#""xa"
"#,
        5,
    ),
    (
        r#""xab" | [rtrimstr(("b", "ab"))]"#,
        r#"["xa","x"]
"#,
        0,
    ),
    (
        r#""xab" | first(ltrimstr(repeat("x")))"#,
        r#""ab"
"#,
        0,
    ),
    (
        r#""xab" | [limit(2; ltrimstr(repeat("x", "xa")))]"#,
        r#"["ab","b"]
"#,
        0,
    ),
    (
        r#""xab" | ltrimstr(("x", error("E"), "xa"))"#,
        r#""ab"
"#,
        5,
    ),
    (
        r#""xab" | [ltrimstr(("x", "xa"))]"#,
        r#"["ab","b"]
"#,
        0,
    ),
    (
        r#""xab" | first(endswith(repeat("b")))"#,
        r#"true
"#,
        0,
    ),
    (
        r#""xab" | [limit(2; endswith(repeat("b", "x")))]"#,
        r#"[true,false]
"#,
        0,
    ),
    (
        r#""xab" | endswith(("b", error("E"), "x"))"#,
        r#"true
"#,
        5,
    ),
    (
        r#""xab" | [endswith(("b", "x"))]"#,
        r#"[true,false]
"#,
        0,
    ),
    (
        r#""xab" | first(startswith(repeat("x")))"#,
        r#"true
"#,
        0,
    ),
    (
        r#""xab" | [limit(2; startswith(repeat("x", "b")))]"#,
        r#"[true,false]
"#,
        0,
    ),
    (
        r#""xab" | startswith(("x", error("E"), "b"))"#,
        r#"true
"#,
        5,
    ),
    (
        r#""xab" | [startswith(("x", "b"))]"#,
        r#"[true,false]
"#,
        0,
    ),
    (
        r#"null | first(ldexp(repeat(1); 3))"#,
        r#"8
"#,
        0,
    ),
    (
        r#"null | [limit(2; ldexp(repeat(1, 2); 3))]"#,
        r#"[8,16]
"#,
        0,
    ),
    (
        r#"null | ldexp((1, error("E"), 2); 3)"#,
        r#"8
"#,
        5,
    ),
    (
        r#"null | first(ldexp(1; repeat(3)))"#,
        r#"8
"#,
        0,
    ),
    (
        r#"null | [limit(2; ldexp(1; repeat(3, 4)))]"#,
        r#"[8,16]
"#,
        0,
    ),
    (
        r#"null | ldexp(1; (3, error("E"), 4))"#,
        r#"8
"#,
        5,
    ),
    (
        r#"null | [ldexp((1, 2); (3, 4))]"#,
        r#"[8,16,16,32]
"#,
        0,
    ),
    (
        r#"null | first(scalbln(repeat(1); 3))"#,
        r#"8
"#,
        0,
    ),
    (
        r#"null | [limit(2; scalbln(repeat(1, 2); 3))]"#,
        r#"[8,16]
"#,
        0,
    ),
    (
        r#"null | scalbln((1, error("E"), 2); 3)"#,
        r#"8
"#,
        5,
    ),
    (
        r#"null | first(scalbln(1; repeat(3)))"#,
        r#"8
"#,
        0,
    ),
    (
        r#"null | [limit(2; scalbln(1; repeat(3, 4)))]"#,
        r#"[8,16]
"#,
        0,
    ),
    (
        r#"null | scalbln(1; (3, error("E"), 4))"#,
        r#"8
"#,
        5,
    ),
    (
        r#"null | [scalbln((1, 2); (3, 4))]"#,
        r#"[8,16,16,32]
"#,
        0,
    ),
    (
        r#"null | first(scalb(repeat(1); 3))"#,
        r#"8
"#,
        0,
    ),
    (
        r#"null | [limit(2; scalb(repeat(1, 2); 3))]"#,
        r#"[8,16]
"#,
        0,
    ),
    (
        r#"null | scalb((1, error("E"), 2); 3)"#,
        r#"8
"#,
        5,
    ),
    (
        r#"null | first(scalb(1; repeat(3)))"#,
        r#"8
"#,
        0,
    ),
    (
        r#"null | [limit(2; scalb(1; repeat(3, 4)))]"#,
        r#"[8,16]
"#,
        0,
    ),
    (
        r#"null | scalb(1; (3, error("E"), 4))"#,
        r#"8
"#,
        5,
    ),
    (
        r#"null | [scalb((1, 2); (3, 4))]"#,
        r#"[8,16,16,32]
"#,
        0,
    ),
    (
        r#"null | first(nexttoward(repeat(1); 3))"#,
        r#"1.0000000000000002
"#,
        0,
    ),
    (
        r#"null | [limit(2; nexttoward(repeat(1, 2); 3))]"#,
        r#"[1.0000000000000002,2.0000000000000004]
"#,
        0,
    ),
    (
        r#"null | nexttoward((1, error("E"), 2); 3)"#,
        r#"1.0000000000000002
"#,
        5,
    ),
    (
        r#"null | first(nexttoward(1; repeat(3)))"#,
        r#"1.0000000000000002
"#,
        0,
    ),
    (
        r#"null | [limit(2; nexttoward(1; repeat(3, 4)))]"#,
        r#"[1.0000000000000002,1.0000000000000002]
"#,
        0,
    ),
    (
        r#"null | nexttoward(1; (3, error("E"), 4))"#,
        r#"1.0000000000000002
"#,
        5,
    ),
    (
        r#"null | [nexttoward((1, 2); (3, 4))]"#,
        r#"[1.0000000000000002,2.0000000000000004,1.0000000000000002,2.0000000000000004]
"#,
        0,
    ),
    (
        r#"null | first(nextafter(repeat(1); 3))"#,
        r#"1.0000000000000002
"#,
        0,
    ),
    (
        r#"null | [limit(2; nextafter(repeat(1, 2); 3))]"#,
        r#"[1.0000000000000002,2.0000000000000004]
"#,
        0,
    ),
    (
        r#"null | nextafter((1, error("E"), 2); 3)"#,
        r#"1.0000000000000002
"#,
        5,
    ),
    (
        r#"null | first(nextafter(1; repeat(3)))"#,
        r#"1.0000000000000002
"#,
        0,
    ),
    (
        r#"null | [limit(2; nextafter(1; repeat(3, 4)))]"#,
        r#"[1.0000000000000002,1.0000000000000002]
"#,
        0,
    ),
    (
        r#"null | nextafter(1; (3, error("E"), 4))"#,
        r#"1.0000000000000002
"#,
        5,
    ),
    (
        r#"null | [nextafter((1, 2); (3, 4))]"#,
        r#"[1.0000000000000002,2.0000000000000004,1.0000000000000002,2.0000000000000004]
"#,
        0,
    ),
    (
        r#"null | first(fmod(repeat(7); 3))"#,
        r#"1
"#,
        0,
    ),
    (
        r#"null | [limit(2; fmod(repeat(7, 9); 3))]"#,
        r#"[1,0]
"#,
        0,
    ),
    (
        r#"null | fmod((7, error("E"), 9); 3)"#,
        r#"1
"#,
        5,
    ),
    (
        r#"null | first(fmod(7; repeat(3)))"#,
        r#"1
"#,
        0,
    ),
    (
        r#"null | [limit(2; fmod(7; repeat(3, 4)))]"#,
        r#"[1,3]
"#,
        0,
    ),
    (
        r#"null | fmod(7; (3, error("E"), 4))"#,
        r#"1
"#,
        5,
    ),
    (
        r#"null | [fmod((7, 9); (3, 4))]"#,
        r#"[1,0,3,1]
"#,
        0,
    ),
    (
        r#"null | first(fmin(repeat(1); 3))"#,
        r#"1
"#,
        0,
    ),
    (
        r#"null | [limit(2; fmin(repeat(1, 2); 3))]"#,
        r#"[1,2]
"#,
        0,
    ),
    (
        r#"null | fmin((1, error("E"), 2); 3)"#,
        r#"1
"#,
        5,
    ),
    (
        r#"null | first(fmin(1; repeat(3)))"#,
        r#"1
"#,
        0,
    ),
    (
        r#"null | [limit(2; fmin(1; repeat(3, 4)))]"#,
        r#"[1,1]
"#,
        0,
    ),
    (
        r#"null | fmin(1; (3, error("E"), 4))"#,
        r#"1
"#,
        5,
    ),
    (
        r#"null | [fmin((1, 2); (3, 4))]"#,
        r#"[1,2,1,2]
"#,
        0,
    ),
    (
        r#"null | first(fmax(repeat(1); 3))"#,
        r#"3
"#,
        0,
    ),
    (
        r#"null | [limit(2; fmax(repeat(1, 2); 3))]"#,
        r#"[3,3]
"#,
        0,
    ),
    (
        r#"null | fmax((1, error("E"), 2); 3)"#,
        r#"3
"#,
        5,
    ),
    (
        r#"null | first(fmax(1; repeat(3)))"#,
        r#"3
"#,
        0,
    ),
    (
        r#"null | [limit(2; fmax(1; repeat(3, 4)))]"#,
        r#"[3,4]
"#,
        0,
    ),
    (
        r#"null | fmax(1; (3, error("E"), 4))"#,
        r#"3
"#,
        5,
    ),
    (
        r#"null | [fmax((1, 2); (3, 4))]"#,
        r#"[3,3,4,4]
"#,
        0,
    ),
    (
        r#"null | first(fma(repeat(1); 3; 5))"#,
        r#"8
"#,
        0,
    ),
    (
        r#"null | [limit(2; fma(repeat(1, 2); 3; 5))]"#,
        r#"[8,11]
"#,
        0,
    ),
    (
        r#"null | fma((1, error("E"), 2); 3; 5)"#,
        r#"8
"#,
        5,
    ),
    (
        r#"null | first(fma(1; repeat(3); 5))"#,
        r#"8
"#,
        0,
    ),
    (
        r#"null | [limit(2; fma(1; repeat(3, 4); 5))]"#,
        r#"[8,9]
"#,
        0,
    ),
    (
        r#"null | fma(1; (3, error("E"), 4); 5)"#,
        r#"8
"#,
        5,
    ),
    (
        r#"null | first(fma(1; 3; repeat(5)))"#,
        r#"8
"#,
        0,
    ),
    (
        r#"null | [limit(2; fma(1; 3; repeat(5, 6)))]"#,
        r#"[8,9]
"#,
        0,
    ),
    (
        r#"null | fma(1; 3; (5, error("E"), 6))"#,
        r#"8
"#,
        5,
    ),
    (
        r#"null | [fma((1, 2); (3, 4); (5, 6))]"#,
        r#"[8,11,9,13,9,12,10,14]
"#,
        0,
    ),
    (
        r#"null | first(fdim(repeat(7); 3))"#,
        r#"4
"#,
        0,
    ),
    (
        r#"null | [limit(2; fdim(repeat(7, 9); 3))]"#,
        r#"[4,6]
"#,
        0,
    ),
    (
        r#"null | fdim((7, error("E"), 9); 3)"#,
        r#"4
"#,
        5,
    ),
    (
        r#"null | first(fdim(7; repeat(3)))"#,
        r#"4
"#,
        0,
    ),
    (
        r#"null | [limit(2; fdim(7; repeat(3, 4)))]"#,
        r#"[4,3]
"#,
        0,
    ),
    (
        r#"null | fdim(7; (3, error("E"), 4))"#,
        r#"4
"#,
        5,
    ),
    (
        r#"null | [fdim((7, 9); (3, 4))]"#,
        r#"[4,6,3,5]
"#,
        0,
    ),
    (
        r#"null | first(drem(repeat(7); 3))"#,
        r#"1
"#,
        0,
    ),
    (
        r#"null | [limit(2; drem(repeat(7, 9); 3))]"#,
        r#"[1,0]
"#,
        0,
    ),
    (
        r#"null | drem((7, error("E"), 9); 3)"#,
        r#"1
"#,
        5,
    ),
    (
        r#"null | first(drem(7; repeat(3)))"#,
        r#"1
"#,
        0,
    ),
    (
        r#"null | [limit(2; drem(7; repeat(3, 4)))]"#,
        r#"[1,-1]
"#,
        0,
    ),
    (
        r#"null | drem(7; (3, error("E"), 4))"#,
        r#"1
"#,
        5,
    ),
    (
        r#"null | [drem((7, 9); (3, 4))]"#,
        r#"[1,0,-1,1]
"#,
        0,
    ),
    (
        r#"null | first(copysign(repeat(1); 3))"#,
        r#"1
"#,
        0,
    ),
    (
        r#"null | [limit(2; copysign(repeat(1, 2); 3))]"#,
        r#"[1,2]
"#,
        0,
    ),
    (
        r#"null | copysign((1, error("E"), 2); 3)"#,
        r#"1
"#,
        5,
    ),
    (
        r#"null | first(copysign(1; repeat(3)))"#,
        r#"1
"#,
        0,
    ),
    (
        r#"null | [limit(2; copysign(1; repeat(3, -4)))]"#,
        r#"[1,-1]
"#,
        0,
    ),
    (
        r#"null | copysign(1; (3, error("E"), -4))"#,
        r#"1
"#,
        5,
    ),
    (
        r#"null | [copysign((1, 2); (3, -4))]"#,
        r#"[1,2,-1,-2]
"#,
        0,
    ),
    (
        r#"null | first(yn(repeat(1); 3))"#,
        r#"0.3246744247918
"#,
        0,
    ),
    (
        r#"null | [limit(2; yn(repeat(1, 2); 3))]"#,
        r#"[0.3246744247918,-0.1604003934849237]
"#,
        0,
    ),
    (
        r#"null | yn((1, error("E"), 2); 3)"#,
        r#"0.3246744247918
"#,
        5,
    ),
    (
        r#"null | first(yn(1; repeat(3)))"#,
        r#"0.3246744247918
"#,
        0,
    ),
    (
        r#"null | [limit(2; yn(1; repeat(3, 4)))]"#,
        r#"[0.3246744247918,0.3979257105571001]
"#,
        0,
    ),
    (
        r#"null | yn(1; (3, error("E"), 4))"#,
        r#"0.3246744247918
"#,
        5,
    ),
    (
        r#"null | [yn((1, 2); (3, 4))]"#,
        r#"[0.3246744247918,-0.1604003934849237,0.3979257105571001,0.21590359460361502]
"#,
        0,
    ),
    (
        r#"null | first(jn(repeat(1); 3))"#,
        r#"0.3390589585259364
"#,
        0,
    ),
    (
        r#"null | [limit(2; jn(repeat(1, 2); 3))]"#,
        r#"[0.3390589585259364,0.4860912605858911]
"#,
        0,
    ),
    (
        r#"null | jn((1, error("E"), 2); 3)"#,
        r#"0.3390589585259364
"#,
        5,
    ),
    (
        r#"null | first(jn(1; repeat(3)))"#,
        r#"0.3390589585259364
"#,
        0,
    ),
    (
        r#"null | [limit(2; jn(1; repeat(3, 4)))]"#,
        r#"[0.3390589585259364,-0.06604332802354915]
"#,
        0,
    ),
    (
        r#"null | jn(1; (3, error("E"), 4))"#,
        r#"0.3390589585259364
"#,
        5,
    ),
    (
        r#"null | [jn((1, 2); (3, 4))]"#,
        r#"[0.3390589585259364,0.4860912605858911,-0.06604332802354915,0.3641281458520728]
"#,
        0,
    ),
    (
        r#"null | first(remainder(repeat(7); 3))"#,
        r#"1
"#,
        0,
    ),
    (
        r#"null | [limit(2; remainder(repeat(7, 9); 3))]"#,
        r#"[1,0]
"#,
        0,
    ),
    (
        r#"null | remainder((7, error("E"), 9); 3)"#,
        r#"1
"#,
        5,
    ),
    (
        r#"null | first(remainder(7; repeat(3)))"#,
        r#"1
"#,
        0,
    ),
    (
        r#"null | [limit(2; remainder(7; repeat(3, 4)))]"#,
        r#"[1,-1]
"#,
        0,
    ),
    (
        r#"null | remainder(7; (3, error("E"), 4))"#,
        r#"1
"#,
        5,
    ),
    (
        r#"null | [remainder((7, 9); (3, 4))]"#,
        r#"[1,0,-1,1]
"#,
        0,
    ),
    (
        r#"null | first(pow(repeat(2); 4))"#,
        r#"16
"#,
        0,
    ),
    (
        r#"null | [limit(2; pow(repeat(2, 3); 4))]"#,
        r#"[16,81]
"#,
        0,
    ),
    (
        r#"null | pow((2, error("E"), 3); 4)"#,
        r#"16
"#,
        5,
    ),
    (
        r#"null | first(pow(2; repeat(4)))"#,
        r#"16
"#,
        0,
    ),
    (
        r#"null | [limit(2; pow(2; repeat(4, 5)))]"#,
        r#"[16,32]
"#,
        0,
    ),
    (
        r#"null | pow(2; (4, error("E"), 5))"#,
        r#"16
"#,
        5,
    ),
    (
        r#"null | [pow((2, 3); (4, 5))]"#,
        r#"[16,81,32,243]
"#,
        0,
    ),
    (
        r#"null | first(hypot(repeat(3); 4))"#,
        r#"5
"#,
        0,
    ),
    (
        r#"null | [limit(2; hypot(repeat(3, 5); 4))]"#,
        r#"[5,6.4031242374328485]
"#,
        0,
    ),
    (
        r#"null | hypot((3, error("E"), 5); 4)"#,
        r#"5
"#,
        5,
    ),
    (
        r#"null | first(hypot(3; repeat(4)))"#,
        r#"5
"#,
        0,
    ),
    (
        r#"null | [limit(2; hypot(3; repeat(4, 12)))]"#,
        r#"[5,12.36931687685298]
"#,
        0,
    ),
    (
        r#"null | hypot(3; (4, error("E"), 12))"#,
        r#"5
"#,
        5,
    ),
    (
        r#"null | [hypot((3, 5); (4, 12))]"#,
        r#"[5,6.4031242374328485,12.36931687685298,13]
"#,
        0,
    ),
    (
        r#"null | first(atan2(repeat(1); 3))"#,
        r#"0.3217505543966422
"#,
        0,
    ),
    (
        r#"null | [limit(2; atan2(repeat(1, 2); 3))]"#,
        r#"[0.3217505543966422,0.5880026035475675]
"#,
        0,
    ),
    (
        r#"null | atan2((1, error("E"), 2); 3)"#,
        r#"0.3217505543966422
"#,
        5,
    ),
    (
        r#"null | first(atan2(1; repeat(3)))"#,
        r#"0.3217505543966422
"#,
        0,
    ),
    (
        r#"null | [limit(2; atan2(1; repeat(3, 4)))]"#,
        r#"[0.3217505543966422,0.24497866312686414]
"#,
        0,
    ),
    (
        r#"null | atan2(1; (3, error("E"), 4))"#,
        r#"0.3217505543966422
"#,
        5,
    ),
    (
        r#"null | [atan2((1, 2); (3, 4))]"#,
        r#"[0.3217505543966422,0.5880026035475675,0.24497866312686414,0.4636476090008061]
"#,
        0,
    ),
    (
        r#""a,b,a" | first(indices(repeat(",")))"#,
        r#"[1,3]
"#,
        0,
    ),
    (
        r#""a,b,a" | [limit(2; indices(repeat(",", "a")))]"#,
        r#"[[1,3],[0,4]]
"#,
        0,
    ),
    (
        r#""a,b,a" | indices((",", error("E"), "a"))"#,
        r#"[1,3]
"#,
        5,
    ),
    (
        r#""a,b,a" | [indices((",", "a"))]"#,
        r#"[[1,3],[0,4]]
"#,
        0,
    ),
    (
        r#""a,b,a" | first(index(repeat(",")))"#,
        r#"1
"#,
        0,
    ),
    (
        r#""a,b,a" | [limit(2; index(repeat(",", "a")))]"#,
        r#"[1,0]
"#,
        0,
    ),
    (
        r#""a,b,a" | index((",", error("E"), "a"))"#,
        r#"1
"#,
        5,
    ),
    (
        r#""a,b,a" | [index((",", "a"))]"#,
        r#"[1,0]
"#,
        0,
    ),
    (
        r#""a,b,a" | first(rindex(repeat(",")))"#,
        r#"3
"#,
        0,
    ),
    (
        r#""a,b,a" | [limit(2; rindex(repeat(",", "a")))]"#,
        r#"[3,4]
"#,
        0,
    ),
    (
        r#""a,b,a" | rindex((",", error("E"), "a"))"#,
        r#"3
"#,
        5,
    ),
    (
        r#""a,b,a" | [rindex((",", "a"))]"#,
        r#"[3,4]
"#,
        0,
    ),
    (
        r#"["a","b"] | first(join(repeat(",")))"#,
        r#""a,b"
"#,
        0,
    ),
    (
        r#"["a","b"] | [limit(2; join(repeat(",", "-")))]"#,
        r#"["a,b","a-b"]
"#,
        0,
    ),
    (
        r#"["a","b"] | join((",", error("E"), "-"))"#,
        r#""a,b"
"#,
        5,
    ),
    (
        r#"["a","b"] | [join((",", "-"))]"#,
        r#"["a,b","a-b"]
"#,
        0,
    ),
    (
        r#"[[1,[2]]] | first(flatten(repeat(1)))"#,
        r#"[1,[2]]
"#,
        0,
    ),
    (
        r#"[[1,[2]]] | [limit(2; flatten(repeat(1, 0)))]"#,
        r#"[[1,[2]],[[1,[2]]]]
"#,
        0,
    ),
    (
        r#"[[1,[2]]] | flatten((1, error("E"), 0))"#,
        r#"[1,[2]]
"#,
        5,
    ),
    (
        r#"[[1,[2]]] | [flatten((1, 0))]"#,
        r#"[[1,[2]],[[1,[2]]]]
"#,
        0,
    ),
    (
        r#""aAbB" | first(match(repeat("a"); ""))"#,
        r#"{"offset":0,"length":1,"string":"a","captures":[]}
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; match(repeat("a", "b"); ""))]"#,
        r#"[{"offset":0,"length":1,"string":"a","captures":[]},{"offset":2,"length":1,"string":"b","captures":[]}]
"#,
        0,
    ),
    (
        r#""aAbB" | match(("a", error("E"), "b"); "")"#,
        r#"{"offset":0,"length":1,"string":"a","captures":[]}
"#,
        5,
    ),
    (
        r#""aAbB" | first(match("a"; repeat("")))"#,
        r#"{"offset":0,"length":1,"string":"a","captures":[]}
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; match("a"; repeat("", "i")))]"#,
        r#"[{"offset":0,"length":1,"string":"a","captures":[]},{"offset":0,"length":1,"string":"a","captures":[]}]
"#,
        0,
    ),
    (
        r#""aAbB" | match("a"; ("", error("E"), "i"))"#,
        r#"{"offset":0,"length":1,"string":"a","captures":[]}
"#,
        5,
    ),
    (
        r#""aAbB" | [match(("a", "b"); ("", "i"))]"#,
        r#"[{"offset":0,"length":1,"string":"a","captures":[]},{"offset":2,"length":1,"string":"b","captures":[]},{"offset":0,"length":1,"string":"a","captures":[]},{"offset":2,"length":1,"string":"b","captures":[]}]
"#,
        0,
    ),
    (
        r#""aAbB" | first(match(repeat("a")))"#,
        r#"{"offset":0,"length":1,"string":"a","captures":[]}
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; match(repeat("a", "b")))]"#,
        r#"[{"offset":0,"length":1,"string":"a","captures":[]},{"offset":2,"length":1,"string":"b","captures":[]}]
"#,
        0,
    ),
    (
        r#""aAbB" | match(("a", error("E"), "b"))"#,
        r#"{"offset":0,"length":1,"string":"a","captures":[]}
"#,
        5,
    ),
    (
        r#""aAbB" | [match(("a", "b"))]"#,
        r#"[{"offset":0,"length":1,"string":"a","captures":[]},{"offset":2,"length":1,"string":"b","captures":[]}]
"#,
        0,
    ),
    (
        r#""aAbB" | first(test(repeat("a"); ""))"#,
        r#"true
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; test(repeat("a", "c"); ""))]"#,
        r#"[true,false]
"#,
        0,
    ),
    (
        r#""aAbB" | test(("a", error("E"), "c"); "")"#,
        r#"true
"#,
        5,
    ),
    (
        r#""aAbB" | first(test("a"; repeat("")))"#,
        r#"true
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; test("a"; repeat("", "i")))]"#,
        r#"[true,true]
"#,
        0,
    ),
    (
        r#""aAbB" | test("a"; ("", error("E"), "i"))"#,
        r#"true
"#,
        5,
    ),
    (
        r#""aAbB" | [test(("a", "c"); ("", "i"))]"#,
        r#"[true,false,true,false]
"#,
        0,
    ),
    (
        r#""aAbB" | first(test(repeat("a")))"#,
        r#"true
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; test(repeat("a", "c")))]"#,
        r#"[true,false]
"#,
        0,
    ),
    (
        r#""aAbB" | test(("a", error("E"), "c"))"#,
        r#"true
"#,
        5,
    ),
    (
        r#""aAbB" | [test(("a", "c"))]"#,
        r#"[true,false]
"#,
        0,
    ),
    (
        r#""aAbB" | first(capture(repeat("(?<x>a)"); ""))"#,
        r#"{"x":"a"}
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; capture(repeat("(?<x>a)", "(?<x>b)"); ""))]"#,
        r#"[{"x":"a"},{"x":"b"}]
"#,
        0,
    ),
    (
        r#""aAbB" | capture(("(?<x>a)", error("E"), "(?<x>b)"); "")"#,
        r#"{"x":"a"}
"#,
        5,
    ),
    (
        r#""aAbB" | first(capture("(?<x>a)"; repeat("")))"#,
        r#"{"x":"a"}
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; capture("(?<x>a)"; repeat("", "i")))]"#,
        r#"[{"x":"a"},{"x":"a"}]
"#,
        0,
    ),
    (
        r#""aAbB" | capture("(?<x>a)"; ("", error("E"), "i"))"#,
        r#"{"x":"a"}
"#,
        5,
    ),
    (
        r#""aAbB" | [capture(("(?<x>a)", "(?<x>b)"); ("", "i"))]"#,
        r#"[{"x":"a"},{"x":"b"},{"x":"a"},{"x":"b"}]
"#,
        0,
    ),
    (
        r#""aAbB" | first(capture(repeat("(?<x>a)")))"#,
        r#"{"x":"a"}
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; capture(repeat("(?<x>a)", "(?<x>b)")))]"#,
        r#"[{"x":"a"},{"x":"b"}]
"#,
        0,
    ),
    (
        r#""aAbB" | capture(("(?<x>a)", error("E"), "(?<x>b)"))"#,
        r#"{"x":"a"}
"#,
        5,
    ),
    (
        r#""aAbB" | [capture(("(?<x>a)", "(?<x>b)"))]"#,
        r#"[{"x":"a"},{"x":"b"}]
"#,
        0,
    ),
    (
        r#""aAbB" | first(scan(repeat("a"); ""))"#,
        r#""a"
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; scan(repeat("a", "b"); ""))]"#,
        r#"["a","b"]
"#,
        0,
    ),
    (
        r#""aAbB" | scan(("a", error("E"), "b"); "")"#,
        r#""a"
"#,
        5,
    ),
    (
        r#""aAbB" | first(scan("a"; repeat("")))"#,
        r#""a"
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; scan("a"; repeat("", "i")))]"#,
        r#"["a","a"]
"#,
        0,
    ),
    (
        r#""aAbB" | scan("a"; ("", error("E"), "i"))"#,
        r#""a"
"#,
        5,
    ),
    (
        r#""aAbB" | [scan(("a", "b"); ("", "i"))]"#,
        r#"["a","a","A","b","b","B"]
"#,
        0,
    ),
    (
        r#""aAbB" | first(scan(repeat("a")))"#,
        r#""a"
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; scan(repeat("a", "b")))]"#,
        r#"["a","b"]
"#,
        0,
    ),
    (
        r#""aAbB" | scan(("a", error("E"), "b"))"#,
        r#""a"
"#,
        5,
    ),
    (
        r#""aAbB" | [scan(("a", "b"))]"#,
        r#"["a","b"]
"#,
        0,
    ),
    (
        r#""aAbB" | first(splits(repeat("a"); ""))"#,
        r#"""
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; splits(repeat("a", "b"); ""))]"#,
        r#"["","AbB"]
"#,
        0,
    ),
    (
        r#""aAbB" | splits(("a", error("E"), "b"); "")"#,
        r#"""
"AbB"
"#,
        5,
    ),
    (
        r#""aAbB" | [splits(("a", "b"); ("", "i"))]"#,
        r#"["","","","bB","aA","","",""]
"#,
        0,
    ),
    (
        r#""aAbB" | first(splits(repeat("a")))"#,
        r#"""
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; splits(repeat("a", "b")))]"#,
        r#"["","AbB"]
"#,
        0,
    ),
    (
        r#""aAbB" | splits(("a", error("E"), "b"))"#,
        r#"""
"AbB"
"#,
        5,
    ),
    (
        r#""aAbB" | [splits(("a", "b"))]"#,
        r#"["","AbB","aA","B"]
"#,
        0,
    ),
    (
        r#""aAbB" | first(split(repeat("a"); ""))"#,
        r#"["","AbB"]
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; split(repeat("a", "b"); ""))]"#,
        r#"[["","AbB"],["aA","B"]]
"#,
        0,
    ),
    (
        r#""aAbB" | split(("a", error("E"), "b"); "")"#,
        r#"["","AbB"]
"#,
        5,
    ),
    (
        r#""aAbB" | [split(("a", "b"); ("", "i"))]"#,
        r#"[["","","","bB"],["aA","","",""]]
"#,
        0,
    ),
    (
        r#""aAbB" | first(sub(repeat("a"); "X"; ""))"#,
        r#""XAbB"
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; sub(repeat("a", "b"); "X"; ""))]"#,
        r#"["XAbB","aAXB"]
"#,
        0,
    ),
    (
        r#""aAbB" | sub(("a", error("E"), "b"); "X"; "")"#,
        r#""XAbB"
"#,
        5,
    ),
    (
        r#""aAbB" | first(sub("a"; "X"; repeat("")))"#,
        r#""XAbB"
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; sub("a"; "X"; repeat("", "gi")))]"#,
        r#"["XAbB","XXbB"]
"#,
        0,
    ),
    (
        r#""aAbB" | sub("a"; "X"; ("", error("E"), "gi"))"#,
        r#""XAbB"
"#,
        5,
    ),
    (
        r#""aAbB" | [sub(("a", "b"); ("X", "Y"); ("", "gi"))]"#,
        r#"["XAbB","YAbB","XXbB","YYbB","aAXB","aAYB","aAXX","aAYY"]
"#,
        0,
    ),
    (
        r#""aAbB" | first(sub(repeat("a"); "X"))"#,
        r#""XAbB"
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; sub(repeat("a", "b"); "X"))]"#,
        r#"["XAbB","aAXB"]
"#,
        0,
    ),
    (
        r#""aAbB" | sub(("a", error("E"), "b"); "X")"#,
        r#""XAbB"
"#,
        5,
    ),
    (
        r#""aAbB" | [sub(("a", "b"); ("X", "Y"))]"#,
        r#"["XAbB","YAbB","aAXB","aAYB"]
"#,
        0,
    ),
    (
        r#""aAbB" | first(gsub(repeat("a"); "X"; ""))"#,
        r#""XAbB"
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; gsub(repeat("a", "b"); "X"; ""))]"#,
        r#"["XAbB","aAXB"]
"#,
        0,
    ),
    (
        r#""aAbB" | gsub(("a", error("E"), "b"); "X"; "")"#,
        r#""XAbB"
"#,
        5,
    ),
    (
        r#""aAbB" | first(gsub("a"; "X"; repeat("")))"#,
        r#""XAbB"
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; gsub("a"; "X"; repeat("", "i")))]"#,
        r#"["XAbB","XXbB"]
"#,
        0,
    ),
    (
        r#""aAbB" | gsub("a"; "X"; ("", error("E"), "i"))"#,
        r#""XAbB"
"#,
        5,
    ),
    (
        r#""aAbB" | [gsub(("a", "b"); ("X", "Y"); ("", "i"))]"#,
        r#"["XAbB","YAbB","XXbB","YYbB","aAXB","aAYB","aAXX","aAYY"]
"#,
        0,
    ),
    (
        r#""aAbB" | first(gsub(repeat("a"); "X"))"#,
        r#""XAbB"
"#,
        0,
    ),
    (
        r#""aAbB" | [limit(2; gsub(repeat("a", "b"); "X"))]"#,
        r#"["XAbB","aAXB"]
"#,
        0,
    ),
    (
        r#""aAbB" | gsub(("a", error("E"), "b"); "X")"#,
        r#""XAbB"
"#,
        5,
    ),
    (
        r#""aAbB" | [gsub(("a", "b"); ("X", "Y"))]"#,
        r#"["XAbB","YAbB","aAXB","aAYB"]
"#,
        0,
    ),
    (
        r#"[5,6] | first(nth(repeat(0)))"#,
        r#"5
"#,
        0,
    ),
    (
        r#"[5,6] | [limit(2; nth(repeat(0, 1)))]"#,
        r#"[5,6]
"#,
        0,
    ),
    (
        r#"[5,6] | nth((0, error("E"), 1))"#,
        r#"5
"#,
        5,
    ),
    (
        r#"[5,6] | [nth((0, 1))]"#,
        r#"[5,6]
"#,
        0,
    ),
    (
        r#""a" | first(inside(repeat("abc")))"#,
        r#"true
"#,
        0,
    ),
    (
        r#""a" | [limit(2; inside(repeat("abc", "xyz")))]"#,
        r#"[true,false]
"#,
        0,
    ),
    (
        r#""a" | inside(("abc", error("E"), "xyz"))"#,
        r#"true
"#,
        5,
    ),
    (
        r#""a" | [inside(("abc", "xyz"))]"#,
        r#"[true,false]
"#,
        0,
    ),
    (
        r#"[1,2,3] | first(bsearch(repeat(2)))"#,
        r#"1
"#,
        0,
    ),
    (
        r#"[1,2,3] | [limit(2; bsearch(repeat(2, 5)))]"#,
        r#"[1,-4]
"#,
        0,
    ),
    (
        r#"[1,2,3] | bsearch((2, error("E"), 5))"#,
        r#"1
"#,
        5,
    ),
    (
        r#"[1,2,3] | [bsearch((2, 5))]"#,
        r#"[1,-4]
"#,
        0,
    ),
];
