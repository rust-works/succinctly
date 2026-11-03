//! #6092 changed how `succinctly yq` interpolates a mapping or sequence; jq mode keeps `tostring`'s
//! JSON text, as jq 1.7.1 does.

use anyhow::Result;

use crate::run_jq_null;

#[test]
fn a_container_still_interpolates_as_json_text() -> Result<()> {
    let (out, code) = run_jq_null(r#""I \([1,{"a":2}]) \({"b":[]})""#, &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim_end(), r#""I [1,{\"a\":2}] {\"b\":[]}""#);
    Ok(())
}
