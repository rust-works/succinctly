//! #5526: a value the program built more than 384 levels deep crashed `succinctly jq` with a raw
//! Rust panic (exit 101) at the sites that splice it back into the AST (`last(f)`, `+=`, `//=`,
//! a fold's `$x`) or render it for `error(...)`. It is now the same refusal `reduce range(385)
//! as $i (0; [.])` already gave: `nesting depth exceeds limit of 384` on stderr, exit 5, and no
//! panic line. jq itself answers every program below (`1`); whether 384 stays is #5497's call.

use anyhow::Result;

use crate::run_jq_full;

const DEF: &str = "def f(n): if n == 0 then 0 else [f(n-1)] end;";

fn assert_refused(stdout: &str, stderr: &str, code: i32, what: &str) {
    assert_eq!(code, 5, "{what}: stderr={stderr:?}");
    assert_eq!(stdout, "", "{what}");
    assert!(
        stderr.contains("jq: error") && stderr.contains("nesting depth exceeds limit of 384"),
        "{what}: stderr={stderr:?}"
    );
    assert!(
        !stderr.contains("panicked"),
        "{what}: the panic line leaked: {stderr:?}"
    );
}

#[test]
fn test_built_value_past_the_tree_ceiling_is_a_refusal_not_a_panic_5526() -> Result<()> {
    for filter in [
        "last(f(400)) | length",
        "{\"a\":1} | .a += f(400) | length",
        "null | .a //= f(400) | length",
        "try error(f(400)) catch 1",
        "last(foreach range(385) as $i (0;[.])) | length",
    ] {
        let program = format!("{DEF} {filter}");
        let (stdout, stderr, code) = run_jq_full(&["-nc", &program], None)?;
        assert_refused(&stdout, &stderr, code, filter);
    }
    Ok(())
}

#[test]
fn test_built_value_past_the_tree_ceiling_on_document_input_5526() -> Result<()> {
    // The lazy (document) route has its own nets; the same programs over an input.
    for filter in [
        "last(f(400)) | length",
        ".a += f(400) | length",
        ".a //= f(400) | length",
        "try error(f(400)) catch 1",
    ] {
        let program = format!("{DEF} {filter}");
        let (stdout, stderr, code) = run_jq_full(&["-c", &program], Some("{}"))?;
        assert_refused(&stdout, &stderr, code, filter);
    }
    Ok(())
}

#[test]
fn test_refusal_is_per_document_and_try_cannot_swallow_it_5526() -> Result<()> {
    // The second document is still processed. As in jq, the exit status follows the *last*
    // document (`echo '1 2' | jq 'if . == 1 then error("x") else . end'` exits 0), so a refusal on
    // the first one reports on stderr and the run ends 0; the refusal's own exit 5 is the
    // single-document case above.
    let program = format!("{DEF} if . == 1 then last(f(400)) | length else . end");
    let (stdout, stderr, code) = run_jq_full(&["-c", &program], Some("1 2"))?;
    assert_eq!(stdout, "2\n", "stderr={stderr:?}");
    assert_eq!(code, 0);
    assert!(stderr.contains("nesting depth exceeds limit of 384"));
    assert!(!stderr.contains("panicked"), "stderr={stderr:?}");

    // A resource limit, not a catchable error: `try`/`catch` and `?` do not absorb it.
    let program = format!("{DEF} try (last(f(400)) | length) catch 0");
    let (stdout, stderr, code) = run_jq_full(&["-nc", &program], None)?;
    assert_refused(&stdout, &stderr, code, "try/catch around last(f(400))");
    Ok(())
}
