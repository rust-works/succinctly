//! #7471: a chain of `E as $x |` bindings cost memory and time in the square of its length (2,000
//! bindings: 470 MB; 4,000: 1.8 GB and 13 s), where jq 1.7.1 runs 2,000 in a few MB. Each bind
//! substituted its value into a copy of the whole rest of the chain, read or not, and kept the copy
//! while the rest ran. A body that does not read the variable is now evaluated as it is, and a
//! chain answers that question for all of its links in one walk. Every row below was captured from
//! jq 1.7.1 (`/usr/bin/jq`), over the input given and then `10`. jq 1.8.2 agrees except where it
//! words a type error's value preview differently (an improvement, not a fix: 1.7.1 stays the
//! reference); past about 2,900 bindings both refuse with `memory exhausted`, a limit of jq's
//! parser that succinctly does not adopt.

use std::time::{Duration, Instant};

use anyhow::Result;

use crate::run_jq_full;

/// `n` links `{src} as ${name(i)} |` and then `tail`.
fn chain(n: usize, tail: &str, src: &str, repeated: bool) -> String {
    let mut program = String::new();
    for i in 0..n {
        if repeated {
            program.push_str(&format!("{i} as $x|"));
        } else {
            program.push_str(&format!("{src} as $x{i}|"));
        }
    }
    program + tail
}

/// The programs of [`EXPECTED`], in order, with their first input.
fn programs() -> Vec<(String, String)> {
    let mut programs = Vec::new();
    for n in [3, 50, 2000] {
        for (tail, src, input, repeated) in [
            ("$x0".to_string(), "1", "null", false),
            (format!("[$x0, $x{}]", n - 1), "1", "null", false),
            ("1".to_string(), "1", "null", false),
            ("$x".to_string(), "", "null", true),
            ("$x0".to_string(), ".", "5", false),
            ("[$x0, .]".to_string(), ".a", "{\"a\":7}", false),
            (format!("$x{}", n / 2), "(.+1)", "1", false),
        ] {
            programs.push((chain(n, &tail, src, repeated), input.to_string()));
        }
    }
    for &(program, input) in FIXED {
        programs.push((program.to_string(), input.to_string()));
    }
    programs
}

/// Short chains around every construct that binds, reads or shadows a variable.
const FIXED: &[(&str, &str)] = &[
    ("(1,2) as $x0 | (3,4) as $x1 | [$x0,$x1]", "null"),
    ("1 as $x | def f($x): $x; 2 as $y | [f(3), $x, $y]", "null"),
    ("1 as $x | 2 as $y | reduce (5,6) as $x (0; . + $x) | [., $x, $y]", "null"),
    ("path(1 as $x | 2 as $y | .a)", "{\"a\":1}"),
    ("path(1 as $x | 2 as $y | .a[$x])", "{\"a\":[1,2]}"),
    ("(1 as $x | 2 as $y | .a) |= 7", "{\"a\":1}"),
    ("(1 as $x | 2 as $y | .a[$x]) |= 7", "{\"a\":[1,2]}"),
    ("1 as $x | 2 as $y | {a: $x, b: $y}", "null"),
    ("1 as $x | 2 as $y | $__loc__", "null"),
    ("1 as $x | error(\"e\") as $y | 3", "null"),
    ("try (1 as $x | error(\"e\") as $y | 3) catch .", "null"),
    ("[empty as $x | 1 as $y | 2]", "null"),
    ("def f: 1 as $a | 2 as $b | $a + $b + .; [range(3) | f]", "null"),
    ("def f(n): n as $n | 1 as $one | 2 as $two | if $n > 0 then f($n - $one) else \"done\" end; f(30)", "null"),
    ("1 as $x | ($x + 1) as $y | 3 as $z | $y", "null"),
    ("1 as $x | ($x + 1) as $x | 5 as $z | $x", "null"),
    ("[.[] as $x | 1 as $y | 2 as $z | $x]", "[1,2,3]"),
    ("[label $out | 1 as $x | 2 as $y | ($x, break $out, $y)]", "null"),
    ("try (1 as $x | 2 as $y | error($x)) catch .", "null"),
    ("[nan] as $a | 1 as $b | 2 as $c | $a == $a", "null"),
    (". as $d | 1 as $b | 2 as $c | [$d | .. ]", "{\"a\":[1]}"),
    ("1 as $x | 2 as $y | [range($x; $y + 3)]", "null"),
    ("1 as $x | 2 as $y | def g: $x + $y; 10 as $x | g", "null"),
    ("1 as $x | 2 as $y | \"\\($x)-\\($y)\"", "null"),
    ("1 as $x | 2 as $y | . as [$a, $b] | [$a, $b, $x]", "[8,9]"),
    ("1 as $x | 2 as $y | .[$x:$y]", "[0,1,2]"),
    ("1 as $x | 2 as $y | getpath([\"a\"]) as $v | [$v, $x]", "{\"a\":3}"),
    ("[limit(3; 1 as $x | 2 as $y | repeat($x))]", "null"),
    ("1 as $x | 2 as $y | input as $i | [$i, $x]", "null"),
    ("del(1 as $x | 2 as $y | .a)", "{\"a\":1,\"b\":2}"),
    ("to_entries | map(1 as $x | 2 as $y | .value + $x)", "{\"a\":1,\"b\":2}"),
];

/// `(exit status, stdout, stderr)` per program, from jq 1.7.1.
const EXPECTED: &[(i32, &str, &str)] = &[
    (0, "1\n1\n", ""),
    (0, "[1,1]\n[1,1]\n", ""),
    (0, "1\n1\n", ""),
    (0, "2\n2\n", ""),
    (0, "5\n10\n", ""),
    (
        5,
        "[7,{\"a\":7}]\n",
        "jq: error (at <stdin>:2): Cannot index number with string \"a\"\n",
    ),
    (0, "2\n11\n", ""),
    (0, "1\n1\n", ""),
    (0, "[1,1]\n[1,1]\n", ""),
    (0, "1\n1\n", ""),
    (0, "49\n49\n", ""),
    (0, "5\n10\n", ""),
    (
        5,
        "[7,{\"a\":7}]\n",
        "jq: error (at <stdin>:2): Cannot index number with string \"a\"\n",
    ),
    (0, "2\n11\n", ""),
    (0, "1\n1\n", ""),
    (0, "[1,1]\n[1,1]\n", ""),
    (0, "1\n1\n", ""),
    (0, "1999\n1999\n", ""),
    (0, "5\n10\n", ""),
    (
        5,
        "[7,{\"a\":7}]\n",
        "jq: error (at <stdin>:2): Cannot index number with string \"a\"\n",
    ),
    (0, "2\n11\n", ""),
    (
        0,
        "[1,3]\n[1,4]\n[2,3]\n[2,4]\n[1,3]\n[1,4]\n[2,3]\n[2,4]\n",
        "",
    ),
    (0, "[3,1,2]\n[3,1,2]\n", ""),
    (0, "[11,1,2]\n[11,1,2]\n", ""),
    (
        5,
        "[\"a\"]\n",
        "jq: error (at <stdin>:2): Cannot index number with string \"a\"\n",
    ),
    (
        5,
        "[\"a\",1]\n",
        "jq: error (at <stdin>:2): Cannot index number with string \"a\"\n",
    ),
    (
        5,
        "{\"a\":7}\n",
        "jq: error (at <stdin>:2): Cannot index number with string \"a\"\n",
    ),
    (
        5,
        "{\"a\":[1,7]}\n",
        "jq: error (at <stdin>:2): Cannot index number with string \"a\"\n",
    ),
    (0, "{\"a\":1,\"b\":2}\n{\"a\":1,\"b\":2}\n", ""),
    (
        0,
        "{\"file\":\"<top-level>\",\"line\":1}\n{\"file\":\"<top-level>\",\"line\":1}\n",
        "",
    ),
    (
        5,
        "",
        "jq: error (at <stdin>:1): e\njq: error (at <stdin>:2): e\n",
    ),
    (0, "\"e\"\n\"e\"\n", ""),
    (0, "[]\n[]\n", ""),
    (0, "[3,4,5]\n[3,4,5]\n", ""),
    (0, "\"done\"\n\"done\"\n", ""),
    (0, "2\n2\n", ""),
    (0, "2\n2\n", ""),
    (
        5,
        "[1,2,3]\n",
        "jq: error (at <stdin>:2): Cannot iterate over number (10)\n",
    ),
    (0, "[1]\n[1]\n", ""),
    (0, "1\n1\n", ""),
    (0, "true\ntrue\n", ""),
    (0, "[{\"a\":[1]},[1],1]\n[10]\n", ""),
    (0, "[1,2,3,4]\n[1,2,3,4]\n", ""),
    (0, "3\n3\n", ""),
    (0, "\"1-2\"\n\"1-2\"\n", ""),
    (
        5,
        "[8,9,1]\n",
        "jq: error (at <stdin>:2): Cannot index number with number\n",
    ),
    (
        5,
        "[1]\n",
        "jq: error (at <stdin>:2): Cannot index number with object\n",
    ),
    (
        5,
        "[3,1]\n",
        "jq: error (at <stdin>:2): Cannot index number with string \"a\"\n",
    ),
    (0, "[1,1,1]\n[1,1,1]\n", ""),
    (0, "[10,1]\n", ""),
    (
        5,
        "{\"b\":2}\n",
        "jq: error (at <stdin>:2): Cannot index number with string \"a\"\n",
    ),
    (
        5,
        "[2,3]\n",
        "jq: error (at <stdin>:2): number (10) has no keys\n",
    ),
];

#[test]
fn test_as_binding_chains_match_jq_7471() -> Result<()> {
    let programs = programs();
    assert_eq!(programs.len(), EXPECTED.len());
    for ((program, input), &(code, stdout, stderr)) in programs.iter().zip(EXPECTED) {
        let got = run_jq_full(&["-c", program], Some(&format!("{input}\n10\n")))?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (stdout, stderr, code),
            "{}",
            &program[..program.len().min(120)]
        );
    }
    Ok(())
}

/// 6,000 bindings end to end, three ways. Before the fix this held a copy of the rest of the chain
/// per link, about 4 GB at this length; the count of copies itself, which is what makes the cost
/// quadratic, is pinned deterministically by the unit test
/// `as_chain_copies_only_the_bodies_that_read_their_variable_7471` in `src/jq/eval.rs`. The time
/// bound only catches a gross regression.
#[test]
fn test_a_long_as_binding_chain_is_not_quadratic_7471() -> Result<()> {
    for (tail, want) in [("$x0", "1\n"), ("1", "1\n")] {
        let program = chain(6000, tail, "1", false);
        let started = Instant::now();
        let (stdout, stderr, code) = run_jq_full(&["-n", "-c", &program], None)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{stderr:?}");
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{tail}: took {:?}",
            started.elapsed()
        );
    }
    let program = chain(6000, "$x", "", true);
    let (stdout, stderr, code) = run_jq_full(&["-n", "-c", &program], None)?;
    assert_eq!((stdout.as_str(), code), ("5999\n", 0), "{stderr:?}");
    Ok(())
}
