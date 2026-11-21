//! #7470: refusing a deeply nested document for jq's parse-depth limit was quadratic in its depth on
//! the routes that read on after a parse fault (`-n input`, `inputs`, `input_line_number`): after
//! each fault jq drops the rest of its read and parses on from the next one, and every such read
//! that opened another refused container was parsed, indexed and split to the end of the text
//! again, where jq stops at the refused bracket. A 2.4 MB document cost 69 G instructions; jq 1.7.1
//! takes 19 M. Every row below was captured from jq 1.7.1 (`/usr/bin/jq`) over stdin. jq 1.8.2
//! differs only where its raised depth limit (10,000, jqlang/jq#3328) moves the refusal, a
//! deliberate limit change that leaves 1.7.1 the reference.

use std::time::{Duration, Instant};

use anyhow::Result;

use crate::run_jq_full;

/// The programs of the rows, by index: the routes that read on after a fault, one that reads every
/// fault in turn (`try input catch .`), and the plain route.
const PROGRAMS: [&[&str]; 5] = [
    &["-n", "input"],
    &["-c", "input_line_number"],
    &["-n", "-c", "[inputs]"],
    &["-n", "-c", "[limit(4; repeat(try input catch .))]"],
    &["-c", "."],
];

/// The documents of the rows: `n` nested objects, arrays, or both in turn, a deep array between
/// values, and deep documents over several lines.
fn shape(name: &str, n: usize) -> String {
    let arr = format!("{}{}", "[".repeat(n), "]".repeat(n));
    let obj = format!("{}1{}", "{\"a\":".repeat(n), "}".repeat(n));
    match name {
        "obj" => obj,
        "arr" => arr,
        "mixed" => format!("{}0{}", "[{\"a\":".repeat(n), "}]".repeat(n)),
        "between" => format!("1 2 {arr} 3"),
        "lines" => format!("{arr}\n\"x\"\n{obj}\n4\n"),
        _ => unreachable!("{name}"),
    }
}

/// `(depth, shape, program, exit status, stdout, stderr)`, from jq 1.7.1.
const ROWS: &[(usize, &str, usize, i32, &str, &str)] = &[
    (300, "obj", 0, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 641\n"),
    (300, "obj", 1, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 641\n"),
    (300, "obj", 2, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 641\n"),
    (300, "obj", 3, 0, "[\"Exceeds depth limit for parsing at line 1, column 641\",\"break\",\"break\",\"break\"]\n", ""),
    (300, "obj", 4, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 641\n"),
    (300, "arr", 0, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 257\n"),
    (300, "arr", 1, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 257\n"),
    (300, "arr", 2, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 257\n"),
    (300, "arr", 3, 0, "[\"Exceeds depth limit for parsing at line 1, column 257\",\"break\",\"break\",\"break\"]\n", ""),
    (300, "arr", 4, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 257\n"),
    (300, "mixed", 0, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 512\n"),
    (300, "mixed", 1, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 512\n"),
    (300, "mixed", 2, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 512\n"),
    (300, "mixed", 3, 0, "[\"Exceeds depth limit for parsing at line 1, column 512\",\"break\",\"break\",\"break\"]\n", ""),
    (300, "mixed", 4, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 512\n"),
    (300, "between", 0, 0, "1\n", ""),
    (300, "between", 1, 5, "0\n0\n", "jq: parse error: Exceeds depth limit for parsing at line 1, column 261\n"),
    (300, "between", 2, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 261\n"),
    (300, "between", 3, 0, "[1,2,\"Exceeds depth limit for parsing at line 1, column 261\",\"break\"]\n", ""),
    (300, "between", 4, 5, "1\n2\n", "jq: parse error: Exceeds depth limit for parsing at line 1, column 261\n"),
    (300, "lines", 0, 5, "", "jq: error (at <stdin>:1): Exceeds depth limit for parsing at line 1, column 257\n"),
    (300, "lines", 1, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 257\n"),
    (300, "lines", 2, 5, "", "jq: error (at <stdin>:1): Exceeds depth limit for parsing at line 1, column 257\n"),
    (300, "lines", 3, 0, "[\"Exceeds depth limit for parsing at line 1, column 257\",\"x\",\"Exceeds depth limit for parsing at line 2, column 641\",4]\n", ""),
    (300, "lines", 4, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 257\n"),
    (3000, "obj", 0, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 641\n"),
    (3000, "obj", 1, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 641\n"),
    (3000, "obj", 2, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 641\n"),
    (3000, "obj", 3, 0, "[\"Exceeds depth limit for parsing at line 1, column 641\",\"Exceeds depth limit for parsing at line 1, column 1282\",\"Exceeds depth limit for parsing at line 1, column 1923\",\"Exceeds depth limit for parsing at line 1, column 2564\"]\n", ""),
    (3000, "obj", 4, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 641\n"),
    (3000, "arr", 0, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 257\n"),
    (3000, "arr", 1, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 257\n"),
    (3000, "arr", 2, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 257\n"),
    (3000, "arr", 3, 0, "[\"Exceeds depth limit for parsing at line 1, column 257\",\"Unmatched ']' at line 1, column 258\",\"break\",\"break\"]\n", ""),
    (3000, "arr", 4, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 257\n"),
    (3000, "mixed", 0, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 512\n"),
    (3000, "mixed", 1, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 512\n"),
    (3000, "mixed", 2, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 512\n"),
    (3000, "mixed", 3, 0, "[\"Exceeds depth limit for parsing at line 1, column 512\",\"Invalid numeric literal at line 1, column 514\",\"Exceeds depth limit for parsing at line 1, column 1026\",\"Invalid numeric literal at line 1, column 1028\"]\n", ""),
    (3000, "mixed", 4, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 512\n"),
    (3000, "between", 0, 0, "1\n", ""),
    (3000, "between", 1, 5, "0\n0\n", "jq: parse error: Exceeds depth limit for parsing at line 1, column 261\n"),
    (3000, "between", 2, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 261\n"),
    (3000, "between", 3, 0, "[1,2,\"Exceeds depth limit for parsing at line 1, column 261\",\"Unmatched ']' at line 1, column 262\"]\n", ""),
    (3000, "between", 4, 5, "1\n2\n", "jq: parse error: Exceeds depth limit for parsing at line 1, column 261\n"),
    (3000, "lines", 0, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 257\n"),
    (3000, "lines", 1, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 257\n"),
    (3000, "lines", 2, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 257\n"),
    (3000, "lines", 3, 0, "[\"Exceeds depth limit for parsing at line 1, column 257\",\"Unmatched ']' at line 1, column 258\",\"x\",\"Exceeds depth limit for parsing at line 2, column 641\"]\n", ""),
    (3000, "lines", 4, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 257\n"),
    (30000, "obj", 0, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 641\n"),
    (30000, "obj", 1, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 641\n"),
    (30000, "obj", 2, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 641\n"),
    (30000, "obj", 3, 0, "[\"Exceeds depth limit for parsing at line 1, column 641\",\"Exceeds depth limit for parsing at line 1, column 1282\",\"Exceeds depth limit for parsing at line 1, column 1923\",\"Exceeds depth limit for parsing at line 1, column 2564\"]\n", ""),
    (30000, "obj", 4, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 641\n"),
    (30000, "arr", 0, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 257\n"),
    (30000, "arr", 1, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 257\n"),
    (30000, "arr", 2, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 257\n"),
    (30000, "arr", 3, 0, "[\"Exceeds depth limit for parsing at line 1, column 257\",\"Exceeds depth limit for parsing at line 1, column 514\",\"Exceeds depth limit for parsing at line 1, column 771\",\"Exceeds depth limit for parsing at line 1, column 1028\"]\n", ""),
    (30000, "arr", 4, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 257\n"),
    (30000, "mixed", 0, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 512\n"),
    (30000, "mixed", 1, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 512\n"),
    (30000, "mixed", 2, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 512\n"),
    (30000, "mixed", 3, 0, "[\"Exceeds depth limit for parsing at line 1, column 512\",\"Invalid numeric literal at line 1, column 514\",\"Exceeds depth limit for parsing at line 1, column 1026\",\"Invalid numeric literal at line 1, column 1028\"]\n", ""),
    (30000, "mixed", 4, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 512\n"),
    (30000, "between", 0, 0, "1\n", ""),
    (30000, "between", 1, 5, "0\n0\n", "jq: parse error: Exceeds depth limit for parsing at line 1, column 261\n"),
    (30000, "between", 2, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 261\n"),
    (30000, "between", 3, 0, "[1,2,\"Exceeds depth limit for parsing at line 1, column 261\",\"Exceeds depth limit for parsing at line 1, column 518\"]\n", ""),
    (30000, "between", 4, 5, "1\n2\n", "jq: parse error: Exceeds depth limit for parsing at line 1, column 261\n"),
    (30000, "lines", 0, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 257\n"),
    (30000, "lines", 1, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 257\n"),
    (30000, "lines", 2, 5, "", "jq: error (at <stdin>:0): Exceeds depth limit for parsing at line 1, column 257\n"),
    (30000, "lines", 3, 0, "[\"Exceeds depth limit for parsing at line 1, column 257\",\"Exceeds depth limit for parsing at line 1, column 514\",\"Exceeds depth limit for parsing at line 1, column 771\",\"Exceeds depth limit for parsing at line 1, column 1028\"]\n", ""),
    (30000, "lines", 4, 5, "", "jq: parse error: Exceeds depth limit for parsing at line 1, column 257\n"),
];

#[test]
fn test_deep_document_refusal_matches_jq_on_every_input_route_7470() -> Result<()> {
    for &(n, name, program, code, stdout, stderr) in ROWS {
        let text = shape(name, n);
        let got = run_jq_full(PROGRAMS[program], Some(&text))?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (stdout, stderr, code),
            "{name} x {n}, {:?}",
            PROGRAMS[program]
        );
    }
    Ok(())
}

/// 400,000 nested objects (2.4 MB): each read after the first fault is parsed only through the
/// bracket jq refuses, so the refusal takes a fraction of a second in a debug build, where parsing
/// every read to the end of the text took minutes. The bound sits well between the two.
#[test]
fn test_deep_document_refusal_is_not_quadratic_on_the_input_routes_7470() -> Result<()> {
    let text = shape("obj", 400_000);
    for program in [&["-n", "input"][..], &["-c", "input_line_number"]] {
        let started = Instant::now();
        let (stdout, stderr, code) = run_jq_full(program, Some(&text))?;
        assert_eq!(code, 5, "{program:?}: {stdout:?} {stderr:?}");
        assert!(
            stderr.contains("Exceeds depth limit for parsing at line 1, column 641"),
            "{program:?}: {stderr:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{program:?} took {:?}",
            started.elapsed()
        );
    }
    Ok(())
}
