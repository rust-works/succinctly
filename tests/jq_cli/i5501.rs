//! #5501: jq refuses a *constant* non-string object key while it compiles the program
//! (`check_object_key`, exit 3), where a computed key fails only when the program runs (exit 5).
//! A constant is what jq's `constant_fold` can reduce: a literal, an operator over constants, an
//! all-constant array or object, and a pipe whose other stages are `.`. Every row was captured from
//! jq 1.7.1 (`/usr/bin/jq -n -c`): its stdout, the first line of its stderr and its exit status.

use anyhow::Result;

use crate::run_jq_full;

/// Rows jq rejects at compile time: nothing runs, so stdout is empty and the status is 3.
#[test]
fn test_constant_non_string_object_key_is_a_compile_error_5501() -> Result<()> {
    for (filter, kind) in [
        ("{(0):1}", "number (0)"),
        ("{non_const:.,(0):1}", "number (0)"),
        ("{a:1,(0):2}", "number (0)"),
        ("{(null):1}", "null (null)"),
        ("{(true):1}", "boolean (true)"),
        ("{(1>0):1}", "boolean (true)"),
        ("{([]):1}", "array ([])"),
        ("{([1]):1}", "array ([1])"),
        ("{({}):1}", "object ({})"),
        ("{({a:1}):1}", "object ({\"a\":1})"),
        ("{(1+1):1}", "number (2)"),
        ("{(5-10):1}", "number (-5)"),
        ("{(1.5):1}", "number (1.5)"),
        ("{(def g: 1; 1):1}", "number (1)"),
        // The pipe of a constant and `.` is still one constant, in either order.
        ("{(1|.):1}", "number (1)"),
        ("{(.|1):1}", "number (1)"),
        ("{(1|.|.):1}", "number (1)"),
        ("{((1|.)+1):1}", "number (2)"),
        ("{([1|.]):1}", "array ([1])"),
        // Compile-time, so neither a postfix `?`, a `try`, a skipped branch nor an unused
        // definition spares the program.
        ("{(0):1}?", "number (0)"),
        ("try {(0):1} catch .", "number (0)"),
        ("[{(0):1}]", "number (0)"),
        ("{\"a\":{(0):1}}", "number (0)"),
        ("if false then {(0):1} else 1 end", "number (0)"),
        ("def f: {(0):1}; 1", "number (0)"),
        ("{(0):1} | empty", "number (0)"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-n", "-c", filter], None)?;
        assert_eq!((stdout.as_str(), code), ("", 3), "{filter}");
        assert_eq!(
            stderr.lines().next(),
            Some(
                format!("jq: error: Cannot use {kind} as object key at <top-level>, line 1:")
                    .as_str()
            ),
            "{filter}"
        );
        assert!(
            stderr.trim_end().ends_with("jq: 1 compile error"),
            "{filter}"
        );
    }
    Ok(())
}

/// Rows jq compiles: a key it cannot fold is judged when the program runs.
#[test]
fn test_computed_object_key_is_judged_at_run_time_5501() -> Result<()> {
    for (filter, want_stdout, want_code) in [
        // A leading `-` is jq's always-computed negate call, never a constant.
        ("{(-1):1}", "", 5),
        ("{(1,2):3}", "", 5),
        ("{(0,\"a\"):1}", "", 5),
        ("{(.a):1}", "", 5),
        ("{(1|2):1}", "", 5),
        ("{(.|.):1}", "", 5),
        ("{(1 as $x|$x):1}", "", 5),
        // Strings, folded or computed, are fine.
        ("{(\"a\"):1}", "{\"a\":1}\n", 0),
        ("{(\"a\"+\"b\"):1}", "{\"ab\":1}\n", 0),
        ("{(\"a\",\"b\"):1}", "{\"a\":1}\n{\"b\":1}\n", 0),
        ("{(0|tostring):1}", "{\"0\":1}\n", 0),
        ("{(\"a\"|.):1}", "{\"a\":1}\n", 0),
        ("{(. | \"a\"):1}", "{\"a\":1}\n", 0),
        ("{(empty):1}", "", 0),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-n", "-c", filter], None)?;
        assert_eq!(
            (stdout.as_str(), code),
            (want_stdout, want_code),
            "{filter}"
        );
        if want_code == 5 {
            assert!(stderr.contains("as object key"), "{filter}: {stderr}");
            assert!(!stderr.contains("compile error"), "{filter}: {stderr}");
        }
    }
    Ok(())
}

/// A destructuring pattern folds the same pipe-of-a-constant (jq's `check_object_key` serves both).
#[test]
fn test_pattern_key_folds_a_pipe_of_a_constant_5501() -> Result<()> {
    for (filter, want_code) in [
        (". as {(1|.):$x} | $x", 3),
        (". as {((1|.)+1):$x} | $x", 3),
        (". as {(.|\"a\"):$x} | $x", 0),
    ] {
        let (_stdout, _stderr, code) = run_jq_full(&["-n", "-c", filter], None)?;
        assert_eq!(code, want_code, "{filter}");
    }
    Ok(())
}
