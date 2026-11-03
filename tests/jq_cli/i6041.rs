//! #6041: `isfinite` is `type == "number" and (isinfinite | not)` in jq, so a NaN is finite.
//! Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`, macOS).

use anyhow::Result;

use crate::{run_jq_null, run_jq_stdin};

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

#[test]
fn a_nan_read_from_a_document_is_finite() -> Result<()> {
    for (filter, input, want) in [
        ("isfinite", "nan", "true"),
        (
            "map(isfinite)",
            "[nan,1,-Infinity,Infinity]",
            "[true,true,false,false]",
        ),
        ("[.[]|select(isfinite)]", "[nan,1]", "[null,1]"),
    ] {
        let (out, code) = run_jq_stdin(filter, input, &["-c"])?;
        assert_eq!(code, 0, "{input} | {filter}");
        assert_eq!(out.trim_end(), want, "{input} | {filter}");
    }
    Ok(())
}

#[test]
fn path_mode_keeps_a_nan_that_isfinite_selects() -> Result<()> {
    check("[nan,1,infinite]|[path(.[]|select(isfinite))]", "[[0],[1]]")?;
    check("[nan,1,infinite]|[paths(isfinite)]", "[[0],[1]]")
}
