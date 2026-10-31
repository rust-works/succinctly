//! #5614 is a jq-mode literal fix: yq rejects a trailing-dot number without `--jq-extensions` and
//! reads it as the plain float with it, as before.

use crate::run_yq_stdin;

#[test]
fn test_yq_trailing_dot_number_unchanged_5614() {
    let (_, code) = run_yq_stdin("1.e1", "a: 1\n", &[]).unwrap();
    assert_eq!(code, 1);
    let (out, code) = run_yq_stdin("1.e1", "a: 1\n", &["--jq-extensions"]).unwrap();
    assert_eq!((out.as_str(), code), ("10\n", 0));
}
