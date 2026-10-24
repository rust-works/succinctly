//! Which of jq 1.7.1's C builtins hand back their input (#5223, D4).
//!
//! In path mode a value passes `PATH_END` or the next `INDEX` only if it *is*
//! the register's node (`jv_identical`), so whether a C builtin returns its
//! input `jv` or builds a new one is observable: `path(.a | ltrimstr("z"))` is
//! `["a"]` in jq 1.7.1, and `path(.a | ltrimstr("x"))` on `{"a":"x"}` raises.
//! This module records, for every function in 1.7.1's `function_list`
//! (`src/builtin.c`, read with `jv.c` and `jv_aux.c`), when it does.
//!
//! | C function (jq name)                            | identity of the result                                                |
//! |-------------------------------------------------|-----------------------------------------------------------------------|
//! | `f_tostring` (`tostring`)                       | the input itself when it is a string                                  |
//! | `f_tonumber` (`tonumber`)                       | the input itself when it is a number                                  |
//! | `f_format` (`format("text")`, `@text`)          | `f_tostring`'s: the input itself when it is a string                  |
//! | `f_ltrimstr` (`ltrimstr`)                       | the input itself unless input and prefix are strings and it matches   |
//! | `f_rtrimstr` (`rtrimstr`)                       | the input itself unless input and suffix are strings and it matches   |
//! | `f_debug` (`debug`), `f_stderr` (`stderr`)      | the input itself, always                                              |
//! | `f_setpath` (`setpath`)                         | the value argument itself for an empty path                           |
//! | `f_delpaths` (`delpaths`)                       | the input itself when nothing is deleted (see [`delpaths_keeps`])     |
//! | `binop_plus` (`+`)                              | `b` for `null + b`, `a` for `a + null`, `a + []` and `a + {}`         |
//! | `binop_multiply` (`*`)                          | `a` for `a * {}` over two objects                                     |
//! | `f_min`/`f_max`, `f_min_by_impl`/`f_max_by_impl` | one of the input's elements (`jv_copy`), or `null` for `[]`         |
//! | `f_sort`, `f_sort_by_impl`, `f_group_by_impl`   | a new array (or arrays) of the input's own elements                   |
//! | `f_getpath` (`getpath`)                         | register-aware (`_jq_path_append`), modelled by the interpreter itself |
//! | `f_error`, `f_halt`, `f_halt_error`             | no result: they raise or halt                                         |
//! | every other function                            | fresh                                                                 |
//!
//! "Every other function" is: the libm family (`floor`, `sqrt`, `pow`, `fma`,
//! ... -- computed numbers, which compare by value anyway), `frexp`, `modf`,
//! `lgamma_r`, `_negate`, `binop_minus`/`binop_divide`/`binop_mod` and the
//! comparisons (numbers, booleans, new arrays and strings), `tojson`,
//! `fromjson`, `keys`, `keys_unsorted`, `startswith`, `endswith`, `split`,
//! `explode`, `implode`, `_strindices`, `has`, `contains`, `length`,
//! `utf8bytelength`, `type`, `isinfinite`, `isnan`, `isnormal`, `infinite`,
//! `nan`, `format` for every format but `text`, `env`, `get_search_list`,
//! `get_prog_origin`, `get_jq_origin`, `_match_impl`, `modulemeta`, `input`,
//! `strptime`, `strftime`, `strflocaltime`, `mktime`, `gmtime`, `localtime`,
//! `now`, `input_filename` and `input_line_number`. A string built by
//! appending (`a + b` on strings, `"x" * 1`) is fresh too: `jvp_string_append`
//! reuses its buffer only when unshared, and a string the register can be
//! compared with is always shared.
//!
//! Each "itself" row is pinned by a `/usr/bin/jq` 1.7.1 probe in this
//! module's tests. The element-keeping builtins (`min`, `max`, `sort`, the
//! `_by_impl` helpers) are answered natively by the interpreter, which keeps
//! the elements as `jv_copy` does; this module answers only for a result that
//! is the input or an argument as a whole. succinctly's own builtins that jq
//! 1.7.1 does not define (`trim`, `ltrim`, `rtrim`, `@base32d`, ...) follow the
//! "fresh" row.

use super::expr::{ArithOp, Builtin, FormatType};
use super::value::OwnedValue;

/// Which value a C builtin hands back unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kept {
    /// The input itself.
    Input,
    /// The argument at this position (in the builtin's own argument order).
    Arg(usize),
}

/// What `builtin`, called on `input` with argument values `args` (in its own
/// argument order), hands back unchanged, or `None` when its result is fresh.
pub(crate) fn kept(builtin: &Builtin, input: &OwnedValue, args: &[OwnedValue]) -> Option<Kept> {
    match (builtin, args) {
        (Builtin::ToString, []) if matches!(input, OwnedValue::String(_)) => Some(Kept::Input),
        (Builtin::ToNumber, []) if input.is_number() => Some(Kept::Input),
        (Builtin::FormatNamed(_), [OwnedValue::String(name)])
            if name.as_str() == "text" && matches!(input, OwnedValue::String(_)) =>
        {
            Some(Kept::Input)
        }
        (Builtin::Debug | Builtin::Stderr, []) => Some(Kept::Input),
        (Builtin::Ltrimstr(_), [prefix]) => trim_keeps(input, prefix, |s, a| s.starts_with(a)),
        (Builtin::Rtrimstr(_), [suffix]) => trim_keeps(input, suffix, |s, a| s.ends_with(a)),
        (Builtin::SetPath(..), [OwnedValue::Array(keys), _]) if keys.is_empty() => {
            Some(Kept::Arg(1))
        }
        (Builtin::DelPaths(_), [paths]) if delpaths_keeps(input, paths) => Some(Kept::Input),
        _ => None,
    }
}

/// `@text` (`f_format`'s `text` arm, which is `f_tostring`): the input itself
/// when it is a string.
pub(crate) fn format_keeps_input(format: &FormatType, input: &OwnedValue) -> bool {
    matches!(format, FormatType::Text) && matches!(input, OwnedValue::String(_))
}

/// The operand `binop_plus`/`binop_multiply` hand back unchanged, keeping its
/// identity: `null + b` is `b`, `a + null` is `a`, and an empty right-hand
/// array or object (`a + []`, `a + {}`, `a * {}`) leaves `a` itself
/// (`jv_array_concat`/`jv_object_merge`/`jv_object_merge_recursive` append
/// nothing). Every other result is a fresh `jv`.
pub(crate) fn binop_keeps<'v>(
    op: ArithOp,
    l: &'v OwnedValue,
    r: &'v OwnedValue,
) -> Option<&'v OwnedValue> {
    match (op, l, r) {
        (ArithOp::Add, OwnedValue::Null, _) => Some(r),
        (ArithOp::Add, _, OwnedValue::Null) => Some(l),
        (ArithOp::Add, OwnedValue::Array(_), OwnedValue::Array(items)) if items.is_empty() => {
            Some(l)
        }
        (ArithOp::Add | ArithOp::Mul(_), OwnedValue::Object(_), OwnedValue::Object(map))
            if map.is_empty() =>
        {
            Some(l)
        }
        _ => None,
    }
}

/// `f_ltrimstr`/`f_rtrimstr`: the input comes back unless both are strings
/// and the affix matches, when a new string is built -- even for an empty
/// affix (`jv_string_sized`).
fn trim_keeps(
    input: &OwnedValue,
    affix: &OwnedValue,
    matches: fn(&str, &str) -> bool,
) -> Option<Kept> {
    match (input, affix) {
        (OwnedValue::String(s), OwnedValue::String(a)) if matches(s, a) => None,
        _ => Some(Kept::Input),
    }
}

/// `jv_delpaths`: the input comes back when the path list is empty, or when
/// no path names a top-level key to delete (`delkeys` stays empty, so
/// `jv_dels` changes nothing) and every longer path starts at a key whose
/// value is `null` (`delpaths_sorted` skips those without writing). Any other
/// deletion writes through the shared input and copies it. A malformed list
/// is an error, and an empty path deletes everything (`null`), so neither
/// keeps the input.
pub(crate) fn delpaths_keeps(input: &OwnedValue, paths: &OwnedValue) -> bool {
    let OwnedValue::Array(paths) = paths else {
        return false;
    };
    paths.iter().all(|path| match path {
        OwnedValue::Array(keys) if keys.len() >= 2 => match (input, &keys[0]) {
            (OwnedValue::Null, _) => true,
            (OwnedValue::Object(map), OwnedValue::String(k)) => {
                matches!(map.get(k.as_str()), None | Some(OwnedValue::Null))
            }
            (OwnedValue::Array(items), k) if k.is_number() => matches!(
                array_index(items.len(), k).and_then(|i| items.get(i)),
                None | Some(OwnedValue::Null)
            ),
            _ => false,
        },
        _ => false,
    })
}

/// `jv_get`'s element for number key `key` of an array of `len` elements: the
/// double truncated to an `int` (clamped), counted from the end when negative.
/// `None` -- jq's `null` -- for NaN and out of range.
pub(crate) fn array_index(len: usize, key: &OwnedValue) -> Option<usize> {
    let d = key.as_f64().unwrap_or(f64::NAN);
    if d.is_nan() {
        return None;
    }
    let mut idx = i64::from(d.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32);
    if idx < 0 {
        idx += len as i64;
    }
    usize::try_from(idx).ok().filter(|&i| i < len)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(feature = "unshared-containers"))]
    use crate::jq::path_mode::run;

    fn doc(text: &str) -> OwnedValue {
        let index = crate::json::JsonIndex::build(text.as_bytes());
        crate::jq::eval_generic::to_owned::<crate::jq::JqSemantics, _>(
            &index.root(text.as_bytes()).value(),
        )
        .expect("a valid document")
    }

    /// `path(.a | B)` for each row of the module table that keeps the input,
    /// and for the rows next to them that build a new value. Every expected
    /// answer was captured from `/usr/bin/jq` 1.7.1: `Ok` is the path printed,
    /// `Err` the `Invalid path expression` message.
    #[test]
    #[cfg(not(feature = "unshared-containers"))]
    fn each_keep_input_row_matches_jq_5223() {
        let rows: &'static [(&str, &str, Result<&str, &str>)] = &[
            (r#"{"a":"x"}"#, "path(.a | tostring)", Ok(r#"["a"]"#)),
            (r#"{"a":1}"#, "path(.a | tonumber)", Ok(r#"["a"]"#)),
            (r#"{"a":"x"}"#, "path(.a | @text)", Ok(r#"["a"]"#)),
            (
                r#"{"a":"x"}"#,
                r#"path(.a | format("text"))"#,
                Ok(r#"["a"]"#),
            ),
            (
                r#"{"a":"x"}"#,
                r#"path(.a | format("json"))"#,
                Err(r#"Invalid path expression with result "\"x\"""#),
            ),
            (
                r#"{"a":"x"}"#,
                r#"path(.a | ltrimstr("z"))"#,
                Ok(r#"["a"]"#),
            ),
            (r#"{"a":1}"#, r#"path(.a | ltrimstr("x"))"#, Ok(r#"["a"]"#)),
            (r#"{"a":"x"}"#, "path(.a | ltrimstr(1))", Ok(r#"["a"]"#)),
            (
                r#"{"a":"x"}"#,
                r#"path(.a | ltrimstr("x"))"#,
                Err(r#"Invalid path expression with result """#),
            ),
            (
                r#"{"a":"x"}"#,
                r#"path(.a | ltrimstr(""))"#,
                Err(r#"Invalid path expression with result "x""#),
            ),
            (
                r#"{"a":"x"}"#,
                r#"path(.a | rtrimstr("y"))"#,
                Ok(r#"["a"]"#),
            ),
            (
                r#"{"a":"x"}"#,
                r#"path(.a | rtrimstr(""))"#,
                Err(r#"Invalid path expression with result "x""#),
            ),
            (
                r#"{"a":{"b":1}}"#,
                "path(.a | setpath([]; .))",
                Ok(r#"["a"]"#),
            ),
            (
                r#"{"a":{"b":1}}"#,
                "path(.a | delpaths([]))",
                Ok(r#"["a"]"#),
            ),
            (
                r#"{"a":{"b":null}}"#,
                r#"path(.a | delpaths([["b","c"]]))"#,
                Ok(r#"["a"]"#),
            ),
            (
                r#"{"a":{"b":1}}"#,
                r#"path(.a | delpaths([["c"]]))"#,
                Err(r#"Invalid path expression with result {"b":1}"#),
            ),
            (r#"{"a":{"b":1}}"#, "path(.a | . + null)", Ok(r#"["a"]"#)),
            (r#"{"a":{"b":1}}"#, "path(.a | null + .)", Ok(r#"["a"]"#)),
            (r#"{"a":[1]}"#, "path(.a | . + [])", Ok(r#"["a"]"#)),
            (
                r#"{"a":[1]}"#,
                "path(.a | [] + .)",
                Err("Invalid path expression with result [1]"),
            ),
            (r#"{"a":{"b":1}}"#, "path(.a | . + {})", Ok(r#"["a"]"#)),
            (r#"{"a":{"b":1}}"#, "path(.a | . * {})", Ok(r#"["a"]"#)),
            (
                r#"{"a":"x"}"#,
                r#"path(.a | . + "")"#,
                Err(r#"Invalid path expression with result "x""#),
            ),
            (
                r#"{"a":[[1],[2]]}"#,
                "path(.a | min as $m | .[0] | $m)",
                Ok(r#"["a",0]"#),
            ),
            (
                r#"{"a":[[1],[2]]}"#,
                "path(.a | max as $m | .[1] | $m)",
                Ok(r#"["a",1]"#),
            ),
            (
                r#"{"a":[[2],[1]]}"#,
                "path(.a | sort[0] as $s | .[1] | $s)",
                Ok(r#"["a",1]"#),
            ),
        ];
        // The interpreter guards no stack depth yet; give it room.
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(move || check_rows(rows))
            .expect("spawn")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
    }

    #[cfg(not(feature = "unshared-containers"))]
    fn check_rows(rows: &[(&str, &str, Result<&str, &str>)]) {
        for (input, filter, want) in rows {
            let run = run(filter, doc(input)).expect(filter);
            let got = match run.outcome {
                crate::jq::path_mode::Outcome::Done => {
                    assert_eq!(run.outputs.len(), 1, "{filter}");
                    Ok(run.outputs[0].to_json_jq_preserve())
                }
                crate::jq::path_mode::Outcome::Error(e) => Err(e.message),
                other => panic!("{filter}: {other:?}"),
            };
            let want = want.map(String::from).map_err(String::from);
            assert_eq!(got, want, "{input} | {filter}");
        }
    }

    /// `debug` and `stderr` keep their input whatever it is; `delpaths` keeps
    /// it only while nothing is deleted.
    #[test]
    fn kept_reads_each_condition_5223() {
        let empty = OwnedValue::Array(Vec::new().into());
        assert_eq!(kept(&Builtin::Debug, &empty, &[]), Some(Kept::Input));
        assert_eq!(kept(&Builtin::ToString, &empty, &[]), None);
        assert!(delpaths_keeps(&OwnedValue::Null, &doc(r#"[["a","b"]]"#)));
        assert!(delpaths_keeps(&doc("[null]"), &doc("[[0,1],[-1,2],[9,0]]")));
        assert!(!delpaths_keeps(&doc("[[1]]"), &doc("[[0,1]]")));
        assert!(!delpaths_keeps(&doc(r#"{"a":1}"#), &doc("[[]]")));
        assert!(!delpaths_keeps(&doc(r#"{"a":1}"#), &doc(r#""a""#)));
        assert!(!delpaths_keeps(&doc(r#"{"a":1}"#), &doc("[[1,2]]")));
        assert!(delpaths_keeps(&doc("[1]"), &doc("[[nan,1]]")));
    }
}
