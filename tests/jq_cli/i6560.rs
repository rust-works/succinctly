//! #6560: `$ENV` and `env` built the whole environment object again on every evaluation, so
//! `select(.x == $ENV.X)` over 85,000 records with an 87-variable environment took 806 ms against
//! jq's 132 ms. The CLI reads the environment once per thread now (`snapshot_environment`); what
//! it answers must not change, so each row runs in a child whose environment holds one variable of
//! each awkward kind (non-ASCII, a quote, a backslash) and is the answer jq 1.7.1 gives for the
//! same environment, including over several inputs, in a loop and under `$ENV` shadowing (#3029).

use anyhow::Result;

use crate::spawn_jq_with_env;

const VALUE: &str = "héllo wörld 日本 \"q\" \\ x";

#[test]
fn env_and_dollar_env_answer_what_jq_answers_6560() -> Result<()> {
    for (filter, stdin, expected) in [
        ("$ENV.DEREK_6560", None, "\"héllo wörld 日本 \\\"q\\\" \\\\ x\"\n"),
        ("env.DEREK_6560", None, "\"héllo wörld 日本 \\\"q\\\" \\\\ x\"\n"),
        ("$ENV | has(\"DEREK_6560\")", None, "true\n"),
        ("$ENV.DEREK_6560_ABSENT", None, "null\n"),
        ("[$ENV, $ENV] | .[0] == .[1]", None, "true\n"),
        ("[range(3) | $ENV.DEREK_6560]", None, "[\"héllo wörld 日本 \\\"q\\\" \\\\ x\",\"héllo wörld 日本 \\\"q\\\" \\\\ x\",\"héllo wörld 日本 \\\"q\\\" \\\\ x\"]\n"),
        ("$ENV | .DEREK_6560 | length", None, "22\n"),
        ("env | has(\"DEREK_6560\")", None, "true\n"),
        ("($ENV | keys | index(\"DEREK_6560\")) != null", None, "true\n"),
        ("$ENV | to_entries | map(select(.key == \"DEREK_6560\")) | length", None, "1\n"),
        ("(env | length) == ($ENV | length)", None, "true\n"),
        ("$ENV.DEREK_6560 as $h | [$h, $h]", None, "[\"héllo wörld 日本 \\\"q\\\" \\\\ x\",\"héllo wörld 日本 \\\"q\\\" \\\\ x\"]\n"),
        ("1 as $ENV | $ENV", None, "1\n"),
        ("def f($ENV): $ENV; f(2)", None, "2\n"),
        ("[limit(4; repeat($ENV.DEREK_6560_ABSENT))]", None, "[null,null,null,null]\n"),
        ("$ENV | type", None, "\"object\"\n"),
        ("$ENV | has(\"DEREK_6560_ABSENT\")", None, "false\n"),
        ("[., $ENV.DEREK_6560]", Some("\"a\"\n\"b\"\n\"c\""), "[\"a\",\"héllo wörld 日本 \\\"q\\\" \\\\ x\"]\n[\"b\",\"héllo wörld 日本 \\\"q\\\" \\\\ x\"]\n[\"c\",\"héllo wörld 日本 \\\"q\\\" \\\\ x\"]\n"),
        ("select(. == \"b\") | [., $ENV.DEREK_6560_ABSENT]", Some("\"a\"\n\"b\"\n\"c\""), "[\"b\",null]\n"),
        ("select(. == $ENV.DEREK_6560)", Some("\"a\"\n\"héllo wörld 日本 \\\"q\\\" \\\\ x\"\n\"c\""), "\"héllo wörld 日本 \\\"q\\\" \\\\ x\"\n"),
    ] {
        let mut args = vec!["-c"];
        if stdin.is_none() {
            args.push("-n");
        }
        args.push(filter);
        let (output, code) = spawn_jq_with_env(
            &args,
            "DEREK_6560",
            VALUE,
            stdin.map(str::as_bytes),
        )?;
        assert_eq!(String::from_utf8(output.stdout)?, expected, "{filter}");
        assert_eq!(String::from_utf8(output.stderr)?, "", "{filter}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}

/// An empty value is a set variable (`""`), not an absent one (`null`), through the snapshot too.
#[test]
fn an_empty_variable_is_set_not_absent_6560() -> Result<()> {
    for (filter, expected) in [
        ("$ENV.DEREK_6560", "\"\"\n"),
        ("env.DEREK_6560", "\"\"\n"),
        ("$ENV | has(\"DEREK_6560\")", "true\n"),
        ("[range(2) | $ENV.DEREK_6560]", "[\"\",\"\"]\n"),
    ] {
        let (output, code) = spawn_jq_with_env(&["-n", "-c", filter], "DEREK_6560", "", None)?;
        assert_eq!(String::from_utf8(output.stdout)?, expected, "{filter}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}
