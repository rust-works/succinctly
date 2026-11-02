//! #5836: yq's string builtins refuse an input whose tag is not `!!str`, naming the tag in each
//! builtin's own words, and `tag` reads a plain date as `!!timestamp` and a plain `<<` as
//! `!!merge`. A custom tag reads as its value's own type (`!foo x` is a string, `!foo 5` is
//! refused as `!foo`). Every row is `printf 'a: DOC\n' | yq '.a | FILTER'` from yq v4.53.3:
//! stdout, and stderr for a refusal (exit 1). `@sh` on a string is pinned by `i5837.rs`.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

/// (document value, filter, stdout, stderr)
const ROWS: &[(&str, &str, &str, &str)] = &[
    ("5", "split(\",\")", "", "Error: cannot split !!int, can only split strings"),
    ("5", "sub(\"x\";\"y\")", "", "Error: cannot substitute with !!int, can only substitute strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("5", "test(\"x\")", "", "Error: cannot match with !!int, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("5", "match(\"x\")", "", "Error: cannot match with !!int, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("5", "capture(\"(?P<a>x)\")", "", "Error: cannot match with !!int, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("5", "upcase", "", "Error: cannot change case with !!int, can only operate on strings."),
    ("5", "downcase", "", "Error: cannot change case with !!int, can only operate on strings."),
    ("5", "trim", "", "Error: cannot trim !!int, can only operate on strings."),
    ("5", "@base64", "", "Error: cannot encode !!int as base64, can only operate on strings"),
    ("5", "@uri", "", "Error: cannot encode !!int as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("5", "@sh", "", "Error: cannot encode !!int as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("5", "tag", "!!int\n", ""),
    ("true", "split(\",\")", "", "Error: cannot split !!bool, can only split strings"),
    ("true", "sub(\"x\";\"y\")", "", "Error: cannot substitute with !!bool, can only substitute strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("true", "test(\"x\")", "", "Error: cannot match with !!bool, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("true", "match(\"x\")", "", "Error: cannot match with !!bool, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("true", "capture(\"(?P<a>x)\")", "", "Error: cannot match with !!bool, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("true", "upcase", "", "Error: cannot change case with !!bool, can only operate on strings."),
    ("true", "downcase", "", "Error: cannot change case with !!bool, can only operate on strings."),
    ("true", "trim", "", "Error: cannot trim !!bool, can only operate on strings."),
    ("true", "@base64", "", "Error: cannot encode !!bool as base64, can only operate on strings"),
    ("true", "@uri", "", "Error: cannot encode !!bool as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("true", "@sh", "", "Error: cannot encode !!bool as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("true", "tag", "!!bool\n", ""),
    ("null", "split(\",\")", "", ""),
    ("null", "sub(\"x\";\"y\")", "", "Error: cannot substitute with !!null, can only substitute strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("null", "test(\"x\")", "", "Error: cannot match with !!null, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("null", "match(\"x\")", "", "Error: cannot match with !!null, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("null", "capture(\"(?P<a>x)\")", "", "Error: cannot match with !!null, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("null", "upcase", "", "Error: cannot change case with !!null, can only operate on strings."),
    ("null", "downcase", "", "Error: cannot change case with !!null, can only operate on strings."),
    ("null", "trim", "", "Error: cannot trim !!null, can only operate on strings."),
    ("null", "@base64", "", "Error: cannot encode !!null as base64, can only operate on strings"),
    ("null", "@uri", "", "Error: cannot encode !!null as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("null", "@sh", "", "Error: cannot encode !!null as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("null", "tag", "!!null\n", ""),
    ("[1]", "split(\",\")", "", "Error: cannot split !!seq, can only split strings"),
    ("[1]", "sub(\"x\";\"y\")", "", "Error: cannot substitute with !!seq, can only substitute strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("[1]", "test(\"x\")", "", "Error: cannot match with !!seq, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("[1]", "match(\"x\")", "", "Error: cannot match with !!seq, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("[1]", "capture(\"(?P<a>x)\")", "", "Error: cannot match with !!seq, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("[1]", "upcase", "", "Error: cannot change case with !!seq, can only operate on strings."),
    ("[1]", "downcase", "", "Error: cannot change case with !!seq, can only operate on strings."),
    ("[1]", "trim", "", "Error: cannot trim !!seq, can only operate on strings."),
    ("[1]", "@base64", "", "Error: cannot encode !!seq as base64, can only operate on strings"),
    ("[1]", "@uri", "", "Error: cannot encode !!seq as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("[1]", "@sh", "", "Error: cannot encode !!seq as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("[1]", "tag", "!!seq\n", ""),
    ("{x: 1}", "split(\",\")", "", "Error: cannot split !!map, can only split strings"),
    ("{x: 1}", "sub(\"x\";\"y\")", "", "Error: cannot substitute with !!map, can only substitute strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("{x: 1}", "test(\"x\")", "", "Error: cannot match with !!map, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("{x: 1}", "match(\"x\")", "", "Error: cannot match with !!map, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("{x: 1}", "capture(\"(?P<a>x)\")", "", "Error: cannot match with !!map, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("{x: 1}", "upcase", "", "Error: cannot change case with !!map, can only operate on strings."),
    ("{x: 1}", "downcase", "", "Error: cannot change case with !!map, can only operate on strings."),
    ("{x: 1}", "trim", "", "Error: cannot trim !!map, can only operate on strings."),
    ("{x: 1}", "@base64", "", "Error: cannot encode !!map as base64, can only operate on strings"),
    ("{x: 1}", "@uri", "", "Error: cannot encode !!map as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("{x: 1}", "@sh", "", "Error: cannot encode !!map as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("{x: 1}", "tag", "!!map\n", ""),
    ("<<", "split(\",\")", "", "Error: cannot split !!merge, can only split strings"),
    ("<<", "sub(\"x\";\"y\")", "", "Error: cannot substitute with !!merge, can only substitute strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("<<", "test(\"x\")", "", "Error: cannot match with !!merge, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("<<", "match(\"x\")", "", "Error: cannot match with !!merge, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("<<", "capture(\"(?P<a>x)\")", "", "Error: cannot match with !!merge, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("<<", "upcase", "", "Error: cannot change case with !!merge, can only operate on strings."),
    ("<<", "downcase", "", "Error: cannot change case with !!merge, can only operate on strings."),
    ("<<", "trim", "", "Error: cannot trim !!merge, can only operate on strings."),
    ("<<", "@base64", "", "Error: cannot encode !!merge as base64, can only operate on strings"),
    ("<<", "@uri", "", "Error: cannot encode !!merge as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("<<", "@sh", "", "Error: cannot encode !!merge as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("<<", "tag", "!!merge\n", ""),
    ("!!binary aGk=", "split(\",\")", "", "Error: cannot split !!binary, can only split strings"),
    ("!!binary aGk=", "sub(\"x\";\"y\")", "", "Error: cannot substitute with !!binary, can only substitute strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("!!binary aGk=", "test(\"x\")", "", "Error: cannot match with !!binary, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("!!binary aGk=", "match(\"x\")", "", "Error: cannot match with !!binary, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("!!binary aGk=", "capture(\"(?P<a>x)\")", "", "Error: cannot match with !!binary, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("!!binary aGk=", "upcase", "", "Error: cannot change case with !!binary, can only operate on strings."),
    ("!!binary aGk=", "downcase", "", "Error: cannot change case with !!binary, can only operate on strings."),
    ("!!binary aGk=", "trim", "", "Error: cannot trim !!binary, can only operate on strings."),
    ("!!binary aGk=", "@base64", "", "Error: cannot encode !!binary as base64, can only operate on strings"),
    ("!!binary aGk=", "@uri", "", "Error: cannot encode !!binary as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("!!binary aGk=", "@sh", "", "Error: cannot encode !!binary as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("!!binary aGk=", "tag", "!!binary\n", ""),
    ("!foo x", "split(\",\")", "- x\n", ""),
    ("!foo x", "sub(\"x\";\"y\")", "y\n", ""),
    ("!foo x", "test(\"x\")", "true\n", ""),
    ("!foo x", "match(\"x\")", "string: x\noffset: 0\nlength: 1\ncaptures: []\n", ""),
    ("!foo x", "capture(\"(?P<a>x)\")", "a: x\n", ""),
    ("!foo x", "upcase", "X\n", ""),
    ("!foo x", "downcase", "x\n", ""),
    ("!foo x", "trim", "x\n", ""),
    ("!foo x", "@base64", "eA==\n", ""),
    ("!foo x", "@uri", "x\n", ""),
    ("!foo x", "tag", "!foo\n", ""),
    ("!foo 5", "split(\",\")", "", "Error: cannot split !foo, can only split strings"),
    ("!foo 5", "sub(\"x\";\"y\")", "", "Error: cannot substitute with !foo, can only substitute strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("!foo 5", "test(\"x\")", "", "Error: cannot match with !foo, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("!foo 5", "match(\"x\")", "", "Error: cannot match with !foo, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("!foo 5", "capture(\"(?P<a>x)\")", "", "Error: cannot match with !foo, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("!foo 5", "upcase", "", "Error: cannot change case with !foo, can only operate on strings."),
    ("!foo 5", "downcase", "", "Error: cannot change case with !foo, can only operate on strings."),
    ("!foo 5", "trim", "", "Error: cannot trim !foo, can only operate on strings."),
    ("!foo 5", "@base64", "", "Error: cannot encode !foo as base64, can only operate on strings"),
    ("!foo 5", "@uri", "", "Error: cannot encode !foo as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("!foo 5", "@sh", "", "Error: cannot encode !foo as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("!foo 5", "tag", "!foo\n", ""),
    ("x", "split(\",\")", "- x\n", ""),
    ("x", "sub(\"x\";\"y\")", "y\n", ""),
    ("x", "test(\"x\")", "true\n", ""),
    ("x", "match(\"x\")", "string: x\noffset: 0\nlength: 1\ncaptures: []\n", ""),
    ("x", "capture(\"(?P<a>x)\")", "a: x\n", ""),
    ("x", "upcase", "X\n", ""),
    ("x", "downcase", "x\n", ""),
    ("x", "trim", "x\n", ""),
    ("x", "@base64", "eA==\n", ""),
    ("x", "@uri", "x\n", ""),
    ("x", "tag", "!!str\n", ""),
    ("\"<<\"", "split(\",\")", "- \"<<\"\n", ""),
    ("\"<<\"", "sub(\"x\";\"y\")", "<<\n", ""),
    ("\"<<\"", "test(\"x\")", "false\n", ""),
    ("\"<<\"", "match(\"x\")", "", ""),
    ("\"<<\"", "capture(\"(?P<a>x)\")", "", ""),
    ("\"<<\"", "upcase", "<<\n", ""),
    ("\"<<\"", "downcase", "<<\n", ""),
    ("\"<<\"", "trim", "<<\n", ""),
    ("\"<<\"", "@base64", "PDw=\n", ""),
    ("\"<<\"", "@uri", "%3C%3C\n", ""),
    ("\"<<\"", "tag", "!!str\n", ""),
    ("2001-12-14", "split(\",\")", "", "Error: cannot split !!timestamp, can only split strings"),
    ("2001-12-14", "sub(\"x\";\"y\")", "", "Error: cannot substitute with !!timestamp, can only substitute strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("2001-12-14", "test(\"x\")", "", "Error: cannot match with !!timestamp, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("2001-12-14", "match(\"x\")", "", "Error: cannot match with !!timestamp, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("2001-12-14", "capture(\"(?P<a>x)\")", "", "Error: cannot match with !!timestamp, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("2001-12-14", "upcase", "", "Error: cannot change case with !!timestamp, can only operate on strings."),
    ("2001-12-14", "downcase", "", "Error: cannot change case with !!timestamp, can only operate on strings."),
    ("2001-12-14", "trim", "", "Error: cannot trim !!timestamp, can only operate on strings."),
    ("2001-12-14", "@base64", "", "Error: cannot encode !!timestamp as base64, can only operate on strings"),
    ("2001-12-14", "@uri", "", "Error: cannot encode !!timestamp as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("2001-12-14", "@sh", "", "Error: cannot encode !!timestamp as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("2001-12-14", "tag", "!!timestamp\n", ""),
    ("\"2001-12-14\"", "sub(\"x\";\"y\")", "2001-12-14\n", ""),
    ("\"2001-12-14\"", "test(\"x\")", "false\n", ""),
    ("\"2001-12-14\"", "match(\"x\")", "", ""),
    ("\"2001-12-14\"", "capture(\"(?P<a>x)\")", "", ""),
    ("\"2001-12-14\"", "upcase", "2001-12-14\n", ""),
    ("\"2001-12-14\"", "downcase", "2001-12-14\n", ""),
    ("\"2001-12-14\"", "trim", "2001-12-14\n", ""),
    ("\"2001-12-14\"", "@base64", "MjAwMS0xMi0xNA==\n", ""),
    ("\"2001-12-14\"", "@uri", "2001-12-14\n", ""),
    ("\"2001-12-14\"", "tag", "!!str\n", ""),
    ("!<tag:yaml.org,2002:binary> x", "split(\",\")", "", "Error: cannot split !!binary, can only split strings"),
    ("!<tag:yaml.org,2002:binary> x", "sub(\"x\";\"y\")", "", "Error: cannot substitute with !!binary, can only substitute strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("!<tag:yaml.org,2002:binary> x", "test(\"x\")", "", "Error: cannot match with !!binary, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("!<tag:yaml.org,2002:binary> x", "match(\"x\")", "", "Error: cannot match with !!binary, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("!<tag:yaml.org,2002:binary> x", "capture(\"(?P<a>x)\")", "", "Error: cannot match with !!binary, can only match strings. Hint: Most often you'll want to use '|=' over '=' for this operation"),
    ("!<tag:yaml.org,2002:binary> x", "upcase", "", "Error: cannot change case with !!binary, can only operate on strings."),
    ("!<tag:yaml.org,2002:binary> x", "downcase", "", "Error: cannot change case with !!binary, can only operate on strings."),
    ("!<tag:yaml.org,2002:binary> x", "trim", "", "Error: cannot trim !!binary, can only operate on strings."),
    ("!<tag:yaml.org,2002:binary> x", "@base64", "", "Error: cannot encode !!binary as base64, can only operate on strings"),
    ("!<tag:yaml.org,2002:binary> x", "@uri", "", "Error: cannot encode !!binary as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("!<tag:yaml.org,2002:binary> x", "@sh", "", "Error: cannot encode !!binary as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"),
    ("!<tag:yaml.org,2002:binary> x", "tag", "!!binary\n", ""),
];

#[test]
fn test_string_builtins_refuse_a_non_string_tag_as_yq_5836() -> Result<()> {
    for (doc, filter, want_out, want_err) in ROWS {
        let (stdout, stderr, code) =
            run_yq_stdin_with_stderr(&format!(".a | {filter}"), &format!("a: {doc}\n"), &[])?;
        let want_code = i32::from(!want_err.is_empty());
        assert_eq!(
            (stdout.as_str(), stderr.trim(), code),
            (*want_out, *want_err, want_code),
            "a: {doc} | {filter}"
        );
    }
    Ok(())
}

/// A bad pattern or flag is reported before the input's type, as yq checks it first; `split`
/// ignores its pattern and refuses the input.
#[test]
fn test_a_bad_pattern_is_reported_before_the_input_tag_5836() -> Result<()> {
    for (filter, want_err) in [
        ("test(\"(\")", "Error: error parsing regexp: missing closing ): `(`"),
        ("sub(\"(\";\"y\")", "Error: error parsing regexp: missing closing ): `(`"),
        (
            "test(\"x\";\"i\")",
            "Error: 'i' is not a valid option for match. To ignore case, use an expression like match(\"(?i)cat\")",
        ),
        ("split(\"(\")", "Error: cannot split !!int, can only split strings"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, "5\n", &[])?;
        assert_eq!((stdout.as_str(), stderr.trim(), code), ("", want_err, 1), "{filter}");
    }
    // `split` passes a null through.
    let (stdout, stderr, code) = run_yq_stdin_with_stderr("split(\",\")", "null\n", &[])?;
    assert_eq!((stdout.as_str(), code), ("", 0), "{stderr:?}");
    Ok(())
}

/// `tag` and `type` report a verbatim tag as what it spells, a core one in its `!!` form.
#[test]
fn test_tag_of_a_verbatim_tag_is_what_it_spells_5836() -> Result<()> {
    for (doc, want) in [
        ("a: !<tag:yaml.org,2002:str> x\n", "!!str\n"),
        ("a: !<tag:yaml.org,2002:int> \"5\"\n", "!!int\n"),
        (
            "a: !<tag:example.com,2000:x> y\n",
            "tag:example.com,2000:x\n",
        ),
        ("a: !<!foo> z\n", "!foo\n"),
    ] {
        for filter in [".a | tag", ".a | type"] {
            let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, doc, &[])?;
            assert_eq!(
                (stdout.as_str(), code),
                (want, 0),
                "{doc:?} {filter}: {stderr:?}"
            );
        }
    }
    Ok(())
}

/// The arguments yq reads are evaluated on the real input before the type check, so an
/// argument's error or value decides the answer first; `split(a; b)` reads neither argument and
/// `sub(re; r; flags)` only its pattern.
#[test]
fn test_arguments_are_read_from_the_real_input_before_the_refusal_5836() -> Result<()> {
    const HINT: &str = "Hint: Most often you'll want to use '|=' over '=' for this operation";
    let not_matched = format!("Error: cannot match with !!int, can only match strings. {HINT}");
    let not_substituted =
        format!("Error: cannot substitute with !!int, can only substitute strings. {HINT}");
    for (filter, want_err) in [
        (
            "test(\"x\";.)",
            "Error: unrecognised match params '5', please see docs at https://mikefarah.gitbook.io/yq/operators/string-operators",
        ),
        (
            "test(\"x\";tag)",
            "Error: 'i' is not a valid option for match. To ignore case, use an expression like match(\"(?i)cat\")",
        ),
        ("test(error(.))", "Error: 5"),
        ("test(error(tag))", "Error: !!int"),
        ("split(error(\"s\"))", "Error: s"),
        ("split(\"x\";error(\"s\"))", "Error: cannot split !!int, can only split strings"),
        ("sub(\"x\";\"y\";error(\"f\"))", not_substituted.as_str()),
        ("sub(error(\"p\");\"y\";\"g\")", "Error: p"),
        ("sub(\"x\";error(\"r\"))", "Error: r"),
        ("test(\"x\";error(\"f\"))", "Error: f"),
        ("test(\"a\",\"b\")", not_matched.as_str()),
        ("test(error(\"p\");\"g\")", "Error: p"),
        ("match(\"x\";\"g\")", not_matched.as_str()),
        ("capture(\"x\";\"g\")", not_matched.as_str()),
    ] {
        // A document node, which the refusal reads its tag from.
        let (stdout, stderr, code) =
            run_yq_stdin_with_stderr(&format!(".b | {filter}"), "b: 5\n", &[])?;
        assert_eq!((stdout.as_str(), stderr.trim(), code), ("", want_err, 1), "{filter}");
    }
    Ok(())
}

/// An alias node is left to the builtin, which reads the anchor's value: `FOO` where yq applies it
/// to the alias's name and prints `X` (recorded in `docs/compliance/yq/limitations.md`).
#[test]
fn test_an_alias_node_is_left_to_the_builtin_5836() -> Result<()> {
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr(".c | upcase", "b: &x foo\nc: *x\n", &[])?;
    assert_eq!((stdout.as_str(), code), ("FOO\n", 0), "{stderr:?}");
    Ok(())
}
