//! #3027: yq's integer arithmetic keeps the base of a hex or octal `!!int` operand. `parseInt64`
//! hands back a format string with the value (`0x%X`, `0o%o`, else `%v`) and `+`, `-`, `*` and `%`
//! write an integer result with the **left** operand's, so `0x10 + 1` is `0x11` (upper-case digits,
//! the sign after the prefix: `0x-10`), `0o17 + 1` is `0o20` and `1 + 0x10` is `17`. `/`, and any
//! float operand, go through `strconv.ParseFloat` on the operand's text and fail naming the first
//! hex or octal one. `-o=json` prints the number either way. Every row is a live capture from yq
//! v4.53.3 (YAML output and `-o=json -I0` of the same filter), over one document.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

const DOC: &str = "a: 0x10\nb: 0xff\nc: 0o17\ne: 017\nf: 2\ng: 0.5\nh: +5\n";

/// (filter, YAML output, JSON output) captured from yq v4.53.3.
const ROWS: &[(&str, &str, &str)] = &[
    (".a + 1", "0x11\n", "17\n"),
    ("1 + .a", "17\n", "17\n"),
    (".a - 1", "0xF\n", "15\n"),
    (".a - 32", "0x-10\n", "-16\n"),
    (".a * 2", "0x20\n", "32\n"),
    (".a % 3", "0x1\n", "1\n"),
    (".b + 1", "0x100\n", "256\n"),
    (".c + 1", "0o20\n", "16\n"),
    (".a + .c", "0x1F\n", "31\n"),
    (".c + .a", "0o37\n", "31\n"),
    (".e + 1", "18\n", "18\n"),
    (".a + 1 | . + 1", "0x12\n", "18\n"),
    (".a + 1 | tostring", "0x11\n", "\"0x11\"\n"),
    ("[.a + 1]", "- 0x11\n", "[17]\n"),
    (
        ".a |= . + 1",
        "a: 0x11\nb: 0xff\nc: 0o17\ne: 017\nf: 2\ng: 0.5\nh: +5\n",
        "{\"a\":17,\"b\":255,\"c\":15,\"e\":17,\"f\":2,\"g\":0.5,\"h\":5}\n",
    ),
    (".f + .a", "18\n", "18\n"),
    (".f * .a", "32\n", "32\n"),
    (".a * .a", "0x100\n", "256\n"),
    (".a - .a", "0x0\n", "0\n"),
    (".e + .a", "33\n", "33\n"),
    (".e * 2", "34\n", "34\n"),
    (".h + 1", "6\n", "6\n"),
    (".h - 10", "-5\n", "-5\n"),
    (".a + .h", "0x15\n", "21\n"),
    (".h + .a", "21\n", "21\n"),
    (".f % .a", "2\n", "2\n"),
    (".a % .f", "0x0\n", "0\n"),
    ("(.a + 1) == 17", "false\n", "false\n"),
    ("(.a + 1) == \"0x11\"", "true\n", "true\n"),
    (".a * 0", "0x0\n", "0\n"),
    (".a - 100", "0x-54\n", "-84\n"),
    (".c - 100", "0o-125\n", "-85\n"),
    (".c * -1", "0o-17\n", "-15\n"),
    (
        ".b * 1000000000000",
        "0xE7EBD06AF000\n",
        "255000000000000\n",
    ),
    ("[.a, .c] | map(. + 1)", "- 0x11\n- 0o20\n", "[17,16]\n"),
    (
        "map_values(. + 1)",
        "a: 0x11\nb: 0x100\nc: 0o20\ne: 18\nf: 3\ng: 1.5\nh: 6\n",
        "{\"a\":17,\"b\":256,\"c\":16,\"e\":18,\"f\":3,\"g\":1.5,\"h\":6}\n",
    ),
    (
        ".a += 1",
        "a: 0x11\nb: 0xff\nc: 0o17\ne: 017\nf: 2\ng: 0.5\nh: +5\n",
        "{\"a\":17,\"b\":255,\"c\":15,\"e\":17,\"f\":2,\"g\":0.5,\"h\":5}\n",
    ),
    (
        ".a -= 1",
        "a: 0xF\nb: 0xff\nc: 0o17\ne: 017\nf: 2\ng: 0.5\nh: +5\n",
        "{\"a\":15,\"b\":255,\"c\":15,\"e\":17,\"f\":2,\"g\":0.5,\"h\":5}\n",
    ),
    (
        ".a *= 2",
        "a: 0x20\nb: 0xff\nc: 0o17\ne: 017\nf: 2\ng: 0.5\nh: +5\n",
        "{\"a\":32,\"b\":255,\"c\":15,\"e\":17,\"f\":2,\"g\":0.5,\"h\":5}\n",
    ),
    (
        ".b += 1",
        "a: 0x10\nb: 0x100\nc: 0o17\ne: 017\nf: 2\ng: 0.5\nh: +5\n",
        "{\"a\":16,\"b\":256,\"c\":15,\"e\":17,\"f\":2,\"g\":0.5,\"h\":5}\n",
    ),
    (".a + .a", "0x20\n", "32\n"),
    (".a % .c", "0x1\n", "1\n"),
    ("null + .a", "0x10\n", "16\n"),
    (".a + \"x\"", "0x10x\n", "\"0x10x\"\n"),
    ("\"x\" + .a", "x0x10\n", "\"x0x10\"\n"),
    (".a * \"x\"", "xxxxxxxxxxxxxxxx\n", "\"xxxxxxxxxxxxxxxx\"\n"),
    (".a | . + 1 | . * 2", "0x22\n", "34\n"),
    ("(.a, .c) + 1", "0x11\n0o20\n", "17\n16\n"),
    (".a + (1,2)", "0x11\n0x12\n", "17\n18\n"),
    ("[.a] | .[0] + 1", "0x11\n", "17\n"),
    (".a as $x | $x + 1", "0x11\n", "17\n"),
    ("[.a, .c] | .[] + 1", "0x11\n0o20\n", "17\n16\n"),
    (".a * 9223372036854775807", "0x-10\n", "-16\n"),
    (
        ".a + 9223372036854775807",
        "0x-7FFFFFFFFFFFFFF1\n",
        "-9223372036854775793\n",
    ),
    (".a + -17", "0x-1\n", "-1\n"),
    (".a + 1 | tag", "!!int\n", "\"!!int\"\n"),
    (".a + 1 | type", "!!int\n", "\"!!int\"\n"),
    (".a + 1 | length", "4\n", "4\n"),
    ("(.a + 1) | @json", "17\n", "\"17\"\n"),
    ("[.a + 1] | sort", "- 0x11\n", "[17]\n"),
    ("[.a + 1, .a] | sort", "- 0x10\n- 0x11\n", "[16,17]\n"),
    ("[.a + 1, 17] | unique", "- 0x11\n- 17\n", "[17,17]\n"),
    ("{\"k\": (.a + 1)}", "k: 0x11\n", "{\"k\":17}\n"),
    (
        ".k = (.a + 1)",
        "a: 0x10\nb: 0xff\nc: 0o17\ne: 017\nf: 2\ng: 0.5\nh: +5\nk: 0x11\n",
        "{\"a\":16,\"b\":255,\"c\":15,\"e\":17,\"f\":2,\"g\":0.5,\"h\":5,\"k\":17}\n",
    ),
    (
        ".a |= . * 2",
        "a: 0x20\nb: 0xff\nc: 0o17\ne: 017\nf: 2\ng: 0.5\nh: +5\n",
        "{\"a\":32,\"b\":255,\"c\":15,\"e\":17,\"f\":2,\"g\":0.5,\"h\":5}\n",
    ),
    (
        ".a |= . - 100",
        "a: !!int 0x-54\nb: 0xff\nc: 0o17\ne: 017\nf: 2\ng: 0.5\nh: +5\n",
        "{\"a\":-84,\"b\":255,\"c\":15,\"e\":17,\"f\":2,\"g\":0.5,\"h\":5}\n",
    ),
    (
        ".b |= . % 7",
        "a: 0x10\nb: 0x3\nc: 0o17\ne: 017\nf: 2\ng: 0.5\nh: +5\n",
        "{\"a\":16,\"b\":3,\"c\":15,\"e\":17,\"f\":2,\"g\":0.5,\"h\":5}\n",
    ),
    (
        ".c *= 3",
        "a: 0x10\nb: 0xff\nc: 0o55\ne: 017\nf: 2\ng: 0.5\nh: +5\n",
        "{\"a\":16,\"b\":255,\"c\":45,\"e\":17,\"f\":2,\"g\":0.5,\"h\":5}\n",
    ),
    (
        "[.a, .b, .c] | map(. * 2)",
        "- 0x20\n- 0x1FE\n- 0o36\n",
        "[32,510,30]\n",
    ),
    (
        "[.a, .b, .c] | map(. + .)",
        "- 0x20\n- 0x1FE\n- 0o36\n",
        "[32,510,30]\n",
    ),
    (
        "to_entries | map(select(.key == \"a\") | .value + 1)",
        "- 0x11\n",
        "[17]\n",
    ),
    (".a + 1 | . == \"0x11\"", "true\n", "true\n"),
    ("\"\\(.a + 1)\"", "0x11\n", "\"0x11\"\n"),
    (".a + 1 | @yaml", "0x11\n\n", "\"0x11\\n\"\n"),
    ("0 - .a", "-16\n", "-16\n"),
    ("0 + .a", "16\n", "16\n"),
    ("1 * .a", "16\n", "16\n"),
    (".a * 1", "0x10\n", "16\n"),
    (".a + 0", "0x10\n", "16\n"),
    (".a - 0", "0x10\n", "16\n"),
    (".a % 100", "0x10\n", "16\n"),
];

/// (filter, the error yq prints) captured from yq v4.53.3.
const ERRORS: &[(&str, &str)] = &[
    (
        ".a + 0.5",
        "strconv.ParseFloat: parsing \"0x10\": invalid syntax",
    ),
    (
        ".a / 2",
        "strconv.ParseFloat: parsing \"0x10\": invalid syntax",
    ),
    (
        ".a + .g",
        "strconv.ParseFloat: parsing \"0x10\": invalid syntax",
    ),
    (
        ".f / .a",
        "strconv.ParseFloat: parsing \"0x10\": invalid syntax",
    ),
    (
        ".a / 1",
        "strconv.ParseFloat: parsing \"0x10\": invalid syntax",
    ),
    (
        ".g + .a",
        "strconv.ParseFloat: parsing \"0x10\": invalid syntax",
    ),
    (
        ".g * .a",
        "strconv.ParseFloat: parsing \"0x10\": invalid syntax",
    ),
    (
        ".g - .a",
        "strconv.ParseFloat: parsing \"0x10\": invalid syntax",
    ),
    (
        ".g / .a",
        "strconv.ParseFloat: parsing \"0x10\": invalid syntax",
    ),
    (
        ".a * 1.5",
        "strconv.ParseFloat: parsing \"0x10\": invalid syntax",
    ),
    (
        ".a - 0.5",
        "strconv.ParseFloat: parsing \"0x10\": invalid syntax",
    ),
    (
        ".a % 0.5",
        "strconv.ParseFloat: parsing \"0x10\": invalid syntax",
    ),
    (
        ".a / .c",
        "strconv.ParseFloat: parsing \"0x10\": invalid syntax",
    ),
    (
        ".a / .a",
        "strconv.ParseFloat: parsing \"0x10\": invalid syntax",
    ),
    (
        ".c / .a",
        "strconv.ParseFloat: parsing \"0o17\": invalid syntax",
    ),
    (
        ".a * .g",
        "strconv.ParseFloat: parsing \"0x10\": invalid syntax",
    ),
    (".a % 0", "cannot modulo by 0"),
    (
        ".a / 0",
        "strconv.ParseFloat: parsing \"0x10\": invalid syntax",
    ),
];

#[test]
fn test_hex_and_octal_arithmetic_keeps_the_left_operands_base_3027() -> Result<()> {
    for (filter, yaml, json) in ROWS {
        let (out, code) = run_yq_stdin(filter, DOC, &[])?;
        assert_eq!((out.as_str(), code), (*yaml, 0), "{filter}");
        let (out, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I0"])?;
        assert_eq!((out.as_str(), code), (*json, 0), "{filter} -o=json");
    }
    Ok(())
}

#[test]
fn test_hex_and_octal_operands_meet_a_float_or_a_division_with_parsefloat_3027() -> Result<()> {
    for (filter, message) in ERRORS {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, DOC, &[])?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{filter}");
        assert!(stderr.contains(message), "{filter}: {stderr}");
    }
    Ok(())
}

/// An explicit tag and a comment on the written node, and a hex right-hand side of a compound
/// assignment. Captured from yq v4.53.3. (A `null` left operand or a collection built around a
/// computed value also takes the node's tag and comment in yq, which is the general node-copy gap,
/// not this issue.)
const TAGGED_DOC: &str = "a: !!int 0x10 # note\nn: ~\nk: 0xFF\nm: 0o17\n";

const TAGGED_ROWS: &[(&str, &str, &str)] = &[
    (
        ".a -= 100",
        "a: !!int 0x-54 # note\nn: ~\nk: 0xFF\nm: 0o17\n",
        "{\"a\":-84,\"n\":null,\"k\":255,\"m\":15}\n",
    ),
    (
        ".a = .a - 100",
        "a: !!int 0x-54 # note\nn: ~\nk: 0xFF\nm: 0o17\n",
        "{\"a\":-84,\"n\":null,\"k\":255,\"m\":15}\n",
    ),
    (
        ".a += .k",
        "a: !!int 0x10F # note\nn: ~\nk: 0xFF\nm: 0o17\n",
        "{\"a\":271,\"n\":null,\"k\":255,\"m\":15}\n",
    ),
    (
        ".k += .a",
        "a: !!int 0x10 # note\nn: ~\nk: 0x10F\nm: 0o17\n",
        "{\"a\":16,\"n\":null,\"k\":271,\"m\":15}\n",
    ),
    (
        ".m -= 100",
        "a: !!int 0x10 # note\nn: ~\nk: 0xFF\nm: !!int 0o-125\n",
        "{\"a\":16,\"n\":null,\"k\":255,\"m\":-85}\n",
    ),
    (
        ".a *= -1",
        "a: !!int 0x-10 # note\nn: ~\nk: 0xFF\nm: 0o17\n",
        "{\"a\":-16,\"n\":null,\"k\":255,\"m\":15}\n",
    ),
    (
        "[.a - 100, .m - 100]",
        "- !!int 0x-54 # note\n- !!int 0o-125\n",
        "[-84,-85]\n",
    ),
    ("{\"x\": (.m - 100)}", "x: !!int 0o-125\n", "{\"x\":-85}\n"),
    (
        "(.a, .m) -= 100",
        "a: !!int 0x-54 # note\nn: ~\nk: 0xFF\nm: !!int 0o-125\n",
        "{\"a\":-84,\"n\":null,\"k\":255,\"m\":-85}\n",
    ),
    (
        "with(.a; . -= 100)",
        "a: !!int 0x-54 # note\nn: ~\nk: 0xFF\nm: 0o17\n",
        "{\"a\":-84,\"n\":null,\"k\":255,\"m\":15}\n",
    ),
    (".a - 100 | tag", "!!int\n", "\"!!int\"\n"),
    (".a - 100 | type", "!!int\n", "\"!!int\"\n"),
    (".a - 100 | length", "5\n", "5\n"),
    (".a - 100 | . == \"0x-54\"", "true\n", "true\n"),
];

#[test]
fn test_computed_prefixed_integers_keep_their_tag_and_comment_3027() -> Result<()> {
    for (filter, yaml, json) in TAGGED_ROWS {
        let (out, code) = run_yq_stdin(filter, TAGGED_DOC, &[])?;
        assert_eq!((out.as_str(), code), (*yaml, 0), "{filter}");
        let (out, code) = run_yq_stdin(filter, TAGGED_DOC, &["-o=json", "-I0"])?;
        assert_eq!((out.as_str(), code), (*json, 0), "{filter} -o=json");
    }
    Ok(())
}

/// A computed prefixed integer bound to a variable and used again, which rebuilds it from its
/// text. Captured from yq v4.53.3.
const BOUND_ROWS: &[(&str, &str, &str)] = &[
    ("(.a - 100) as $v | [$v]", "- !!int 0x-54\n", "[-84]\n"),
    (
        "(.a - 100) as $v | [$v, $v + 1]",
        "- !!int 0x-54\n- !!int 0x-53\n",
        "[-84,-83]\n",
    ),
    (
        "(.c - 100) as $v | {\"k\": $v}",
        "k: !!int 0o-125\n",
        "{\"k\":-85}\n",
    ),
    (
        ".a as $x | .b as $y | [$x + $y, $y + $x]",
        "- 0x10F\n- 0x10F\n",
        "[271,271]\n",
    ),
];

#[test]
fn test_a_computed_prefixed_integer_survives_a_binding_3027() -> Result<()> {
    for (filter, yaml, json) in BOUND_ROWS {
        let (out, code) = run_yq_stdin(filter, DOC, &[])?;
        assert_eq!((out.as_str(), code), (*yaml, 0), "{filter}");
        let (out, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I0"])?;
        assert_eq!((out.as_str(), code), (*json, 0), "{filter} -o=json");
    }
    Ok(())
}
