//! #6218: under the `n` flag (Oniguruma's `FIND_NOT_EMPTY`) the end of the pattern is where an empty
//! match fails, so a trailing possessive quantifier over an atom that can match nothing on its first
//! try (`(?:a??)*+`, `(?:|a)++`) is the atomic group of that atom, which takes its empty way and fails
//! (`"cabb" | [match("(?:a??)*+"; "gn")]` is `[]` in jq). The scan took a trailing `+` for the plain
//! quantifier, which backtracks into `a`. A lazy quantifier followed by `+` (`a??+`) is a repeat of
//! a repeat in jq and is written as the group it stands for. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -n`); the controls (`a?+`, `a*+`, `a++`, `(?:a?)?+`, `(?>a??)`) must not move.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_a_trailing_possessive_quantifier_over_a_lazy_atom_under_n_6218() -> Result<()> {
    for (pattern, input, flags, expected) in [
        (r"(?:a??)?+", "cabb", "gn", "[]"),
        (r"(?:a??)?+", "cabb", "n", "[]"),
        (r"(?:a??)?+", "aa", "gn", "[]"),
        (r"(?:a*?)?+", "cabb", "gn", "[]"),
        (r"(?:a*?)?+", "aa", "gn", "[]"),
        (r"(?:|a)?+", "cabb", "gn", "[]"),
        (r"(?:|a)?+", "aa", "gn", "[]"),
        (r"(?:a??|b)?+", "cabb", "gn", "[]"),
        (r"(?:a??|b)?+", "aa", "gn", "[]"),
        (r"(?:a??)*+", "cabb", "gn", "[]"),
        (r"(?:a??)*+", "cabb", "n", "[]"),
        (r"(?:a??)*+", "aa", "gn", "[]"),
        (r"(?:a*?)*+", "cabb", "gn", "[]"),
        (r"(?:a*?)*+", "aa", "gn", "[]"),
        (r"(?:|a)*+", "cabb", "gn", "[]"),
        (r"(?:|a)*+", "aa", "gn", "[]"),
        (r"(?:a??|b)*+", "cabb", "gn", "[]"),
        (r"(?:a??|b)*+", "aa", "gn", "[]"),
        (r"(?:a??)++", "cabb", "gn", "[]"),
        (r"(?:a??)++", "cabb", "n", "[]"),
        (r"(?:a??)++", "aa", "gn", "[]"),
        (r"(?:a*?)++", "cabb", "gn", "[]"),
        (r"(?:a*?)++", "aa", "gn", "[]"),
        (r"(?:|a)++", "cabb", "gn", "[]"),
        (r"(?:|a)++", "aa", "gn", "[]"),
        (r"(?:a??|b)++", "cabb", "gn", "[]"),
        (r"(?:a??|b)++", "aa", "gn", "[]"),
        (r"(?:a??){0,2}+", "cabb", "gn", "[]"),
        (r"(?:a??){0,2}+", "aa", "gn", "[]"),
        (r"(?:a*?){0,2}+", "cabb", "gn", "[]"),
        (r"(?:a*?){0,2}+", "aa", "gn", "[]"),
        (r"(?:|a){0,2}+", "cabb", "gn", "[]"),
        (r"(?:|a){0,2}+", "aa", "gn", "[]"),
        (r"(?:a??|b){0,2}+", "cabb", "gn", "[]"),
        (r"(?:a??|b){0,2}+", "aa", "gn", "[]"),
        (r"(?:a??){1,3}+", "cabb", "gn", "[]"),
        (r"(?:a??){1,3}+", "aa", "gn", "[]"),
        (r"(?:a*?){1,3}+", "cabb", "gn", "[]"),
        (r"(?:a*?){1,3}+", "aa", "gn", "[]"),
        (r"(?:|a){1,3}+", "cabb", "gn", "[]"),
        (r"(?:|a){1,3}+", "aa", "gn", "[]"),
        (r"(?:a??|b){1,3}+", "cabb", "gn", "[]"),
        (r"(?:a??|b){1,3}+", "aa", "gn", "[]"),
        (r"(?:a??){2,}+", "cabb", "gn", "[]"),
        (r"(?:a??){2,}+", "aa", "gn", "[]"),
        (r"(?:a*?){2,}+", "cabb", "gn", "[]"),
        (r"(?:a*?){2,}+", "aa", "gn", "[]"),
        (r"(?:|a){2,}+", "cabb", "gn", "[]"),
        (r"(?:|a){2,}+", "aa", "gn", "[]"),
        (r"(?:a??|b){2,}+", "cabb", "gn", "[]"),
        (r"(?:a??|b){2,}+", "aa", "gn", "[]"),
        (r"a??+", "cabb", "gn", "[[1,1]]"),
        (r"a??+", "cabb", "n", "[[1,1]]"),
        (r"a??+", "aa", "gn", "[[0,1],[1,1]]"),
        (r"a*?+", "cabb", "gn", "[[1,1]]"),
        (r"a*?+", "aa", "gn", "[[0,1],[1,1]]"),
        (r"a+?+", "cabb", "gn", "[[1,1]]"),
        (r"a+?+", "aa", "gn", "[[0,2]]"),
        (r"a??+$", "cabb", "gn", "[]"),
        (r"a??+$", "aa", "gn", "[[0,2]]"),
        (r"b??+", "cabb", "gn", "[[2,1],[3,1]]"),
        (r"b??+", "aa", "gn", "[]"),
        (r"b??+(?=b)", "cabb", "gn", "[[2,1]]"),
        (r"b??+(?=b)", "aa", "gn", "[]"),
        (r"a*?{0,2}+", "cabb", "gn", "[]"),
        (r"a*?{0,2}+", "aa", "gn", "[]"),
        (r"a?+", "cabb", "gn", "[[1,1]]"),
        (r"a?+", "cabb", "n", "[[1,1]]"),
        (r"a?+", "aa", "gn", "[[0,1],[1,1]]"),
        (r"a*+", "cabb", "gn", "[[1,1]]"),
        (r"a*+", "cabb", "n", "[[1,1]]"),
        (r"a*+", "aa", "gn", "[[0,2]]"),
        (r"a++", "cabb", "gn", "[[1,1]]"),
        (r"a++", "aa", "gn", "[[0,2]]"),
        (r"(?:a?)?+", "cabb", "gn", "[[1,1]]"),
        (r"(?:a?)?+", "cabb", "n", "[[1,1]]"),
        (r"(?:a?)?+", "aa", "gn", "[[0,1],[1,1]]"),
        (r"(?:a*)*+", "cabb", "gn", "[[1,1]]"),
        (r"(?:a*)*+", "aa", "gn", "[[0,2]]"),
        (r"(?>a??)", "cabb", "gn", "[]"),
        (r"(?>a??)", "cabb", "n", "[]"),
        (r"(?>a??)", "aa", "gn", "[]"),
        (r"(?>a*?)", "cabb", "gn", "[]"),
        (r"(?>a*?)", "aa", "gn", "[]"),
        (r"a{2}+", "cabb", "gn", "[]"),
        (r"a{2}+", "aa", "gn", "[[0,2]]"),
        (r"b?+a", "cabb", "gn", "[[1,1]]"),
        (r"b?+a", "aa", "gn", "[[0,1],[1,1]]"),
        (r"(?:a??)?+a", "cabb", "gn", "[[1,1]]"),
        (r"(?:a??)?+a", "aa", "gn", "[[0,1],[1,1]]"),
        (r"(?:a??)*+b", "cabb", "gn", "[[2,1],[3,1]]"),
        (r"(?:a??)*+b", "aa", "gn", "[]"),
        (r"(?:a|b)*+", "cabb", "gn", "[[1,3]]"),
        (r"(?:a|b)*+", "aa", "gn", "[[0,2]]"),
    ] {
        let program = format!(
            r#""{input}" | [match("{}";"{flags}") | [.offset, .length]]"#,
            pattern.replace('\\', "\\\\")
        );
        let (stdout, stderr, code) = run_jq_full(&["-nc", &program], None)?;
        assert_eq!(code, 0, "{pattern} on {input} ({flags}): stderr={stderr:?}");
        assert_eq!(
            stdout.trim_end(),
            expected,
            "{pattern} on {input} ({flags})"
        );
    }
    Ok(())
}
