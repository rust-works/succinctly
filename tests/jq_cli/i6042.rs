//! #6042: the literal `-0` is negative zero in path mode too. Path mode read the parser's
//! `NumberLiteral(Int(0), "-0")` as the integer 0, so an error raised under `path(...)`, `pick(...)`
//! or `del(...)` carried `0` where jq carries `-0`. Every expectation was captured from jq 1.7.1
//! (`/usr/bin/jq`, macOS).

use anyhow::Result;

use crate::run_jq_null;

fn check(filter: &str, want: &str) -> Result<()> {
    let (out, code) = run_jq_null(filter, &["-c"])?;
    assert_eq!(code, 0, "{filter}");
    assert_eq!(out.trim_end(), want, "{filter}");
    Ok(())
}

#[test]
fn an_error_raised_under_path_carries_negative_zero() -> Result<()> {
    check("[-0|try path(error) catch .]", "[-0]")?;
    check("[-0|try path(.|error) catch .]", "[-0]")?;
    check("[-0|try pick(error) catch .]", "[-0]")?;
    check(r#"[{"a":-0}|try path(.a|error) catch .]"#, "[-0]")?;
    check("[[-0]|try path(.[0]|error) catch .]", "[-0]")?;
    check("[-0,0]|map(try path(error) catch .)", "[-0,0]")
}

#[test]
fn negative_zero_survives_del_of_nothing() -> Result<()> {
    check("-0|del(empty)", "-0")
}

#[test]
fn the_neighbours_that_already_kept_the_sign_still_do() -> Result<()> {
    check("[-0|try error catch .]", "[-0]")?;
    check("[0|-.|try path(error) catch .]", "[-0]")?;
    check("[-0|try paths(error) catch .]", "[-0]")?;
    check(
        "[-1,-0.0,-5e-1]|map(try path(error) catch .)",
        "[-1,-0,-0.5]",
    )
}
