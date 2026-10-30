//! #5600 is a jq-mode parser fix: `succinctly yq` keeps reading `a::b` as before, where the
//! unquoted key is the namespaced call of a module that is not loaded.

use crate::run_yq_stdin;

#[test]
fn test_yq_unquoted_key_with_double_colon_unchanged_5600() {
    let (_, code) = run_yq_stdin("{a::b: 1}", "a: 1\n", &[]).unwrap();
    assert_eq!(code, 1);
}
