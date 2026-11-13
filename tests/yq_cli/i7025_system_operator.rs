//! #7025: `system(command)` and `system(command; args)` run a program only under
//! `--security-enable-system-operator`, as yq's `system` operator does: for each node the command (the first
//! output of its expression, a non-empty string) gets the arguments (the first output of that expression: a
//! non-null scalar or a sequence of them) and the node as YAML on standard input, and the program's standard
//! output with one trailing newline dropped is the node's replacement, a string. Without the flag the operator
//! is the error `system operations are disabled, ...`. Each row is `yq [--security-enable-system-operator]
//! FILTER` over `a: 1` / `b: [x, y]`, captured from yq v4.53.3 (stdout, the first stderr line, the exit
//! status). Differences, recorded in limitations.md: the standard input is the node written by this build's
//! YAML writer (a flow sequence is a block sequence), and the parse errors of an empty or an over-long
//! argument list.

#![cfg(unix)]

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const DOC: &str = "a: 1\nb: [x, y]\n";

/// (enabled, filter, yq's stdout, first stderr line, exit status).
#[rustfmt::skip]
const ROWS: &[(bool, &str, &str, &str, i32)] = &[
    (false, "system(\"echo\"; \"hi\")", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"echo\"; \"hi\")", "hi\n", "", 0),
    (false, "system(\"echo\"; [\"a\", \"b\"])", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"echo\"; [\"a\", \"b\"])", "a b\n", "", 0),
    (false, ".a | system(\"cat\")", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, ".a | system(\"cat\")", "1\n", "", 0),
    (false, "system(\"false\")", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"false\")", "", "Error: system command 'false' failed: exit status 1", 1),
    (false, "system(\"sh\"; [\"-c\", \"echo err >&2; exit 3\"])", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"sh\"; [\"-c\", \"echo err >&2; exit 3\"])", "", "Error: system command 'sh' failed: exit status 3", 1),
    (false, "system(\"\")", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"\")", "", "Error: system operator: command must be a non-empty string", 1),
    (false, "system(1)", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(1)", "", "Error: system operator: command must be a string scalar", 1),
    (false, "system(.zz)", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(.zz)", "", "Error: system operator: command expression returned no results", 1),
    (false, "system(\"echo\"; null)", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"echo\"; null)", "\n", "", 0),
    (false, "system(\"echo\"; [null])", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"echo\"; [null])", "", "Error: system operator: argument must be a non-null scalar; got kind=4 tag=!!null", 1),
    (false, "system(\"echo\"; {\"a\":1})", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"echo\"; {\"a\":1})", "", "Error: system operator: args must be a non-null scalar or sequence of non-null scalars; got kind=2 tag=!!map", 1),
    (false, "system(\"echo\"; 5)", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"echo\"; 5)", "5\n", "", 0),
    (false, "system(\"echo\"; [1, true, 2.5])", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"echo\"; [1, true, 2.5])", "1 true 2.5\n", "", 0),
    (false, "system(\"/nonexistent/x\")", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"/nonexistent/x\")", "", "Error: system command '/nonexistent/x' failed: fork/exec /nonexistent/x: no such file or directory", 1),
    (false, "system(\"nonexistent_zzz\")", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"nonexistent_zzz\")", "", "Error: system command 'nonexistent_zzz' failed: exec: \"nonexistent_zzz\": executable file not found in $PATH", 1),
    (false, "system(\"printf\"; \"a\\n\\n\")", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"printf\"; \"a\\n\\n\")", "a\n\n", "", 0),
    (false, "system(\"printf\"; \"a\\r\\n\")", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"printf\"; \"a\\r\\n\")", "a\n", "", 0),
    (false, ".b[] | system(\"cat\")", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, ".b[] | system(\"cat\")", "x\ny\n", "", 0),
    (false, "system(\"echo\"; .b)", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"echo\"; .b)", "x y\n", "", 0),
    (false, "system(\"echo\"; \"a\", \"b\")", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"echo\"; \"a\", \"b\")", "a\n", "", 0),
    (false, "(system(\"echo\"; \"x\") | tag)", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "(system(\"echo\"; \"x\") | tag)", "!!str\n", "", 0),
    (false, "system(\"sh\"; [\"-c\",\"kill -9 $$\"])", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"sh\"; [\"-c\",\"kill -9 $$\"])", "", "Error: system command 'sh' failed: signal: killed", 1),
    (false, ".a = system(\"echo\"; \"z\")", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, ".a = system(\"echo\"; \"z\")", "a: z\nb: [x, y]\n", "", 0),
    (false, "system(\"sh\"; [\"-c\",\"cat >/dev/null; echo ok\"])", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"sh\"; [\"-c\",\"cat >/dev/null; echo ok\"])", "ok\n", "", 0),
    (false, ".c = (.a | system(\"cat\"))", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, ".c = (.a | system(\"cat\"))", "a: 1\nb: [x, y]\nc: \"1\"\n", "", 0),
    (false, "system(\"echo\"; \"x\") | length", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"echo\"; \"x\") | length", "1\n", "", 0),
    (false, "[.b[] | system(\"echo\"; .)]", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "[.b[] | system(\"echo\"; .)]", "- x\n- y\n", "", 0),
    (false, "system(\"echo\"; true)", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"echo\"; true)", "true\n", "", 0),
    (false, "system(\"echo\"; 1.50)", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"echo\"; 1.50)", "1.50\n", "", 0),
    (false, "system(\"echo\"; \"é\")", "", "Error: system operations are disabled, use --security-enable-system-operator to enable", 1),
    (true, "system(\"echo\"; \"é\")", "é\n", "", 0),
];

#[test]
fn system_operator_runs_a_program_as_yq_does_7025() -> Result<()> {
    for (enabled, filter, stdout, first_error, status) in ROWS {
        let flags: &[&str] = if *enabled {
            &["--security-enable-system-operator"]
        } else {
            &[]
        };
        let (out, err, code) = run_yq_stdin_with_stderr(filter, DOC, flags)?;
        assert_eq!(
            (out.as_str(), err.lines().next().unwrap_or(""), code),
            (*stdout, *first_error, *status),
            "{filter} (enabled: {enabled})"
        );
    }
    Ok(())
}
