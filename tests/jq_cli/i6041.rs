//! #6041: `isfinite` is `type == "number" and (isinfinite | not)` in jq, so a NaN is finite.
//! Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`, macOS).

use anyhow::Result;

use crate::run_jq_null;

fn check(filter: &str, want: &str) -> Result<()> {
    let (out, code) = run_jq_null(filter, &["-c"])?;
    assert_eq!(code, 0, "{filter}");
    assert_eq!(out.trim_end(), want, "{filter}");
    Ok(())
}

#[test]
fn a_nan_is_finite_and_an_infinity_is_not() -> Result<()> {
    check(
        r#"[nan,infinite,-infinite,1.5,0,-0,"a",null,[],{},true]|map(isfinite)"#,
        "[true,false,false,true,true,true,false,false,false,false,false]",
    )?;
    check("[1e1000,-1e1000,1e308]|map(isfinite)", "[false,false,true]")
}

#[test]
fn a_nan_agrees_with_its_siblings_and_with_select() -> Result<()> {
    check(
        "nan|[isfinite,isinfinite,isnan,isnormal]",
        "[true,false,true,false]",
    )?;
    check("[nan|select(isfinite)]", "[null]")?;
    check("[nan|isfinite|not]", "[false]")
}
