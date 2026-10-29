//! #5501: the compile-time rejection of a constant non-string object key is jq's. `succinctly yq`
//! keeps yq's own rule, which stringifies the key. Captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_yq_stringifies_a_constant_number_object_key_5501() -> Result<()> {
    let (stdout, code) = run_yq_stdin("{(1): 2}", "a: 1\n", &["-o=json", "-I=0"])?;
    assert_eq!((stdout.as_str(), code), ("{\"1\":2}\n", 0));
    Ok(())
}
