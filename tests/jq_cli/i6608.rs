//! #6608: a constructor of static reads (`{id, name}`, `[.id]`, `{a: .b.c, k: "lit"}`) builds its
//! answer from the owned tree, where every node of a `recurse` walk used to be written out as text and
//! indexed again to read two fields. Every expected string was captured from jq 1.7.1
//! (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// Each constructor over inputs that read through, miss, hit a scalar (where jq's `.id` raises) and
/// carry number spellings; stdout, stderr and the exit status as jq answers them.
#[test]
fn constructors_answer_as_jq_6608() -> Result<()> {
    for (filter, input, want, want_err, want_code) in [
        ("{id, name}", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}", "{\"id\":1,\"name\":\"a\"}\n", "", 0),
        ("{id, name}", "[{\"id\":1,\"x\":{\"y\":2.50}},{\"id\":2}]", "", "jq: error (at <stdin>:0): Cannot index array with string \"id\"\n", 5),
        ("{id, name}", "null", "{\"id\":null,\"name\":null}\n", "", 0),
        ("{id, name}", "5", "", "jq: error (at <stdin>:0): Cannot index number with string \"id\"\n", 5),
        ("{id, name}", "{\"a\":[1,{\"b\":100000000000000000000}],\"c\":null}", "{\"id\":null,\"name\":null}\n", "", 0),
        ("{id, name}", "{\"id\":1.50,\"name\":\"n\"}", "{\"id\":1.50,\"name\":\"n\"}\n", "", 0),
        ("{id: .id}", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}", "{\"id\":1}\n", "", 0),
        ("{id: .id}", "[{\"id\":1,\"x\":{\"y\":2.50}},{\"id\":2}]", "", "jq: error (at <stdin>:0): Cannot index array with string \"id\"\n", 5),
        ("{id: .id}", "null", "{\"id\":null}\n", "", 0),
        ("{id: .id}", "5", "", "jq: error (at <stdin>:0): Cannot index number with string \"id\"\n", 5),
        ("{id: .id}", "{\"a\":[1,{\"b\":100000000000000000000}],\"c\":null}", "{\"id\":null}\n", "", 0),
        ("{id: .id}", "{\"id\":1.50,\"name\":\"n\"}", "{\"id\":1.50}\n", "", 0),
        ("{id: .id, id: .name}", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}", "{\"id\":\"a\"}\n", "", 0),
        ("{id: .id, id: .name}", "[{\"id\":1,\"x\":{\"y\":2.50}},{\"id\":2}]", "", "jq: error (at <stdin>:0): Cannot index array with string \"id\"\n", 5),
        ("{id: .id, id: .name}", "null", "{\"id\":null}\n", "", 0),
        ("{id: .id, id: .name}", "5", "", "jq: error (at <stdin>:0): Cannot index number with string \"id\"\n", 5),
        ("{id: .id, id: .name}", "{\"a\":[1,{\"b\":100000000000000000000}],\"c\":null}", "{\"id\":null}\n", "", 0),
        ("{id: .id, id: .name}", "{\"id\":1.50,\"name\":\"n\"}", "{\"id\":\"n\"}\n", "", 0),
        ("{a: .a[1].b, z: .zz, k: \"lit\", w: {id}}", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}", "{\"a\":null,\"z\":null,\"k\":\"lit\",\"w\":{\"id\":1}}\n", "", 0),
        ("{a: .a[1].b, z: .zz, k: \"lit\", w: {id}}", "[{\"id\":1,\"x\":{\"y\":2.50}},{\"id\":2}]", "", "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n", 5),
        ("{a: .a[1].b, z: .zz, k: \"lit\", w: {id}}", "null", "{\"a\":null,\"z\":null,\"k\":\"lit\",\"w\":{\"id\":null}}\n", "", 0),
        ("{a: .a[1].b, z: .zz, k: \"lit\", w: {id}}", "5", "", "jq: error (at <stdin>:0): Cannot index number with string \"a\"\n", 5),
        ("{a: .a[1].b, z: .zz, k: \"lit\", w: {id}}", "{\"a\":[1,{\"b\":100000000000000000000}],\"c\":null}", "{\"a\":100000000000000000000,\"z\":null,\"k\":\"lit\",\"w\":{\"id\":null}}\n", "", 0),
        ("{a: .a[1].b, z: .zz, k: \"lit\", w: {id}}", "{\"id\":1.50,\"name\":\"n\"}", "{\"a\":null,\"z\":null,\"k\":\"lit\",\"w\":{\"id\":1.50}}\n", "", 0),
        ("[.id]", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}", "[1]\n", "", 0),
        ("[.id]", "[{\"id\":1,\"x\":{\"y\":2.50}},{\"id\":2}]", "", "jq: error (at <stdin>:0): Cannot index array with string \"id\"\n", 5),
        ("[.id]", "null", "[null]\n", "", 0),
        ("[.id]", "5", "", "jq: error (at <stdin>:0): Cannot index number with string \"id\"\n", 5),
        ("[.id]", "{\"a\":[1,{\"b\":100000000000000000000}],\"c\":null}", "[null]\n", "", 0),
        ("[.id]", "{\"id\":1.50,\"name\":\"n\"}", "[1.50]\n", "", 0),
        ("[.[0]]", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}", "", "jq: error (at <stdin>:0): Cannot index object with number\n", 5),
        ("[.[0]]", "[{\"id\":1,\"x\":{\"y\":2.50}},{\"id\":2}]", "[{\"id\":1,\"x\":{\"y\":2.50}}]\n", "", 0),
        ("[.[0]]", "null", "[null]\n", "", 0),
        ("[.[0]]", "5", "", "jq: error (at <stdin>:0): Cannot index number with number\n", 5),
        ("[.[0]]", "{\"a\":[1,{\"b\":100000000000000000000}],\"c\":null}", "", "jq: error (at <stdin>:0): Cannot index object with number\n", 5),
        ("[.[0]]", "{\"id\":1.50,\"name\":\"n\"}", "", "jq: error (at <stdin>:0): Cannot index object with number\n", 5),
        ("[.a[-1]]", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}", "[null]\n", "", 0),
        ("[.a[-1]]", "[{\"id\":1,\"x\":{\"y\":2.50}},{\"id\":2}]", "", "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n", 5),
        ("[.a[-1]]", "null", "[null]\n", "", 0),
        ("[.a[-1]]", "5", "", "jq: error (at <stdin>:0): Cannot index number with string \"a\"\n", 5),
        ("[.a[-1]]", "{\"a\":[1,{\"b\":100000000000000000000}],\"c\":null}", "[{\"b\":100000000000000000000}]\n", "", 0),
        ("[.a[-1]]", "{\"id\":1.50,\"name\":\"n\"}", "[null]\n", "", 0),
        ("[.]", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}", "[{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}]\n", "", 0),
        ("[.]", "[{\"id\":1,\"x\":{\"y\":2.50}},{\"id\":2}]", "[[{\"id\":1,\"x\":{\"y\":2.50}},{\"id\":2}]]\n", "", 0),
        ("[.]", "null", "[null]\n", "", 0),
        ("[.]", "5", "[5]\n", "", 0),
        ("[.]", "{\"a\":[1,{\"b\":100000000000000000000}],\"c\":null}", "[{\"a\":[1,{\"b\":100000000000000000000}],\"c\":null}]\n", "", 0),
        ("[.]", "{\"id\":1.50,\"name\":\"n\"}", "[{\"id\":1.50,\"name\":\"n\"}]\n", "", 0),
        ("{x: .}", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}", "{\"x\":{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}}\n", "", 0),
        ("{x: .}", "[{\"id\":1,\"x\":{\"y\":2.50}},{\"id\":2}]", "{\"x\":[{\"id\":1,\"x\":{\"y\":2.50}},{\"id\":2}]}\n", "", 0),
        ("{x: .}", "null", "{\"x\":null}\n", "", 0),
        ("{x: .}", "5", "{\"x\":5}\n", "", 0),
        ("{x: .}", "{\"a\":[1,{\"b\":100000000000000000000}],\"c\":null}", "{\"x\":{\"a\":[1,{\"b\":100000000000000000000}],\"c\":null}}\n", "", 0),
        ("{x: .}", "{\"id\":1.50,\"name\":\"n\"}", "{\"x\":{\"id\":1.50,\"name\":\"n\"}}\n", "", 0),
        ("[{id}]", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}", "[{\"id\":1}]\n", "", 0),
        ("[{id}]", "[{\"id\":1,\"x\":{\"y\":2.50}},{\"id\":2}]", "", "jq: error (at <stdin>:0): Cannot index array with string \"id\"\n", 5),
        ("[{id}]", "null", "[{\"id\":null}]\n", "", 0),
        ("[{id}]", "5", "", "jq: error (at <stdin>:0): Cannot index number with string \"id\"\n", 5),
        ("[{id}]", "{\"a\":[1,{\"b\":100000000000000000000}],\"c\":null}", "[{\"id\":null}]\n", "", 0),
        ("[{id}]", "{\"id\":1.50,\"name\":\"n\"}", "[{\"id\":1.50}]\n", "", 0),
        (". | {id}", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}", "{\"id\":1}\n", "", 0),
        (". | {id}", "[{\"id\":1,\"x\":{\"y\":2.50}},{\"id\":2}]", "", "jq: error (at <stdin>:0): Cannot index array with string \"id\"\n", 5),
        (". | {id}", "null", "{\"id\":null}\n", "", 0),
        (". | {id}", "5", "", "jq: error (at <stdin>:0): Cannot index number with string \"id\"\n", 5),
        (". | {id}", "{\"a\":[1,{\"b\":100000000000000000000}],\"c\":null}", "{\"id\":null}\n", "", 0),
        (". | {id}", "{\"id\":1.50,\"name\":\"n\"}", "{\"id\":1.50}\n", "", 0),
        ("{(.id|tostring): 1}", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}", "{\"1\":1}\n", "", 0),
        ("{(.id|tostring): 1}", "[{\"id\":1,\"x\":{\"y\":2.50}},{\"id\":2}]", "", "jq: error (at <stdin>:0): Cannot index array with string \"id\"\n", 5),
        ("{(.id|tostring): 1}", "null", "{\"null\":1}\n", "", 0),
        ("{(.id|tostring): 1}", "5", "", "jq: error (at <stdin>:0): Cannot index number with string \"id\"\n", 5),
        ("{(.id|tostring): 1}", "{\"a\":[1,{\"b\":100000000000000000000}],\"c\":null}", "{\"null\":1}\n", "", 0),
        ("{(.id|tostring): 1}", "{\"id\":1.50,\"name\":\"n\"}", "{\"1.50\":1}\n", "", 0),
        ("[.id, .name]", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}", "[1,\"a\"]\n", "", 0),
        ("[.id, .name]", "[{\"id\":1,\"x\":{\"y\":2.50}},{\"id\":2}]", "", "jq: error (at <stdin>:0): Cannot index array with string \"id\"\n", 5),
        ("[.id, .name]", "null", "[null,null]\n", "", 0),
        ("[.id, .name]", "5", "", "jq: error (at <stdin>:0): Cannot index number with string \"id\"\n", 5),
        ("[.id, .name]", "{\"a\":[1,{\"b\":100000000000000000000}],\"c\":null}", "[null,null]\n", "", 0),
        ("[.id, .name]", "{\"id\":1.50,\"name\":\"n\"}", "[1.50,\"n\"]\n", "", 0),
        ("{id: .id?}", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}", "{\"id\":1}\n", "", 0),
        ("{id: .id?}", "[{\"id\":1,\"x\":{\"y\":2.50}},{\"id\":2}]", "", "", 0),
        ("{id: .id?}", "null", "{\"id\":null}\n", "", 0),
        ("{id: .id?}", "5", "", "", 0),
        ("{id: .id?}", "{\"a\":[1,{\"b\":100000000000000000000}],\"c\":null}", "{\"id\":null}\n", "", 0),
        ("{id: .id?}", "{\"id\":1.50,\"name\":\"n\"}", "{\"id\":1.50}\n", "", 0),
        ("{id: .a.b.c.d}", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}", "{\"id\":null}\n", "", 0),
        ("{id: .a.b.c.d}", "[{\"id\":1,\"x\":{\"y\":2.50}},{\"id\":2}]", "", "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n", 5),
        ("{id: .a.b.c.d}", "null", "{\"id\":null}\n", "", 0),
        ("{id: .a.b.c.d}", "5", "", "jq: error (at <stdin>:0): Cannot index number with string \"a\"\n", 5),
        ("{id: .a.b.c.d}", "{\"a\":[1,{\"b\":100000000000000000000}],\"c\":null}", "", "jq: error (at <stdin>:0): Cannot index array with string \"b\"\n", 5),
        ("{id: .a.b.c.d}", "{\"id\":1.50,\"name\":\"n\"}", "{\"id\":null}\n", "", 0),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (want, want_err, want_code),
            "{filter} on {input}"
        );
    }
    Ok(())
}

/// The walk the issue measured: a constructor after `recurse(.children[]?)`, over a tree and over one
/// whose scalar child makes jq's `.id` raise after the nodes before it were built.
#[test]
fn a_constructor_after_recurse_answers_as_jq_6608() -> Result<()> {
    for (filter, input, want, want_err, want_code) in [
        ("[recurse(.children[]?) | {id, name}]", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[{\"id\":3,\"name\":\"c\"}]},{\"id\":4,\"name\":\"d\",\"children\":null},{\"id\":5,\"name\":\"e\",\"children\":{\"k\":{\"id\":6,\"name\":\"f\"}}}]}", "[{\"id\":1,\"name\":\"a\"},{\"id\":2,\"name\":\"b\"},{\"id\":3,\"name\":\"c\"},{\"id\":4,\"name\":\"d\"},{\"id\":5,\"name\":\"e\"},{\"id\":6,\"name\":\"f\"}]\n", "", 0),
        ("[recurse(.children[]?) | {id, name}]", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}", "", "jq: error (at <stdin>:0): Cannot index number with string \"id\"\n", 5),
        ("[recurse(.children[]?) | [.id]]", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[{\"id\":3,\"name\":\"c\"}]},{\"id\":4,\"name\":\"d\",\"children\":null},{\"id\":5,\"name\":\"e\",\"children\":{\"k\":{\"id\":6,\"name\":\"f\"}}}]}", "[[1],[2],[3],[4],[5],[6]]\n", "", 0),
        ("[recurse(.children[]?) | [.id]]", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}", "", "jq: error (at <stdin>:0): Cannot index number with string \"id\"\n", 5),
        ("[recurse(.children[]?) | {id, c: .children}] | length", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[{\"id\":3,\"name\":\"c\"}]},{\"id\":4,\"name\":\"d\",\"children\":null},{\"id\":5,\"name\":\"e\",\"children\":{\"k\":{\"id\":6,\"name\":\"f\"}}}]}", "6\n", "", 0),
        ("[recurse(.children[]?) | {id, c: .children}] | length", "{\"id\":1,\"name\":\"a\",\"children\":[{\"id\":2,\"name\":\"b\",\"children\":[]},5,null]}", "", "jq: error (at <stdin>:0): Cannot index number with string \"id\"\n", 5),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (want, want_err, want_code),
            "{filter} on {input}"
        );
    }
    Ok(())
}

/// The shapes the door must leave alone or build exactly (generators and `//` and `as` inside `[...]`,
/// interpolated and computed keys, a repeated key, number literals as the program spells them), each
/// run after a `recurse` so the constructor is reached as a stage of the walk.
#[test]
fn constructors_after_recurse_keep_jq_cardinality_and_spelling_6608() -> Result<()> {
    let input =
        "[{\"a\":1,\"b\":null,\"id\":\"x\",\"children\":[]},{\"a\":false,\"b\":2,\"id\":\"y\"}]";
    for (filter, want, want_err, want_code) in [
        (
            "[.[] | recurse(.children[]?) | [(.a,.b)]]",
            "[[1,null],[false,2]]\n",
            "",
            0,
        ),
        (
            "[.[] | recurse(.children[]?) | [.a // 1]]",
            "[[1],[1]]\n",
            "",
            0,
        ),
        (
            "[.[] | recurse(.children[]?) | [.a as $x | $x]]",
            "[[1],[false]]\n",
            "",
            0,
        ),
        (
            "[.[] | recurse(.children[]?) | [.a?]]",
            "[[1],[false]]\n",
            "",
            0,
        ),
        (
            "[.[] | recurse(.children[]?) | [.a,.b]]",
            "[[1,null],[false,2]]\n",
            "",
            0,
        ),
        (
            "[.[] | recurse(.children[]?) | [empty]]",
            "[[],[]]\n",
            "",
            0,
        ),
        (
            "[.[] | recurse(.children[]?) | {n: 1.0}]",
            "[{\"n\":1.0},{\"n\":1.0}]\n",
            "",
            0,
        ),
        (
            "[.[] | recurse(.children[]?) | {n: 1e2}]",
            "[{\"n\":1E+2},{\"n\":1E+2}]\n",
            "",
            0,
        ),
        (
            "[.[] | recurse(.children[]?) | [1.10]]",
            "[[1.10],[1.10]]\n",
            "",
            0,
        ),
        (
            "[.[] | recurse(.children[]?) | {n: 100000000000000000000}]",
            "[{\"n\":100000000000000000000},{\"n\":100000000000000000000}]\n",
            "",
            0,
        ),
        (
            "[.[] | recurse(.children[]?) | [-0]]",
            "[[-0],[-0]]\n",
            "",
            0,
        ),
        (
            "[.[] | recurse(.children[]?) | {\"a\\(1)\": .id}]",
            "[{\"a1\":\"x\"},{\"a1\":\"y\"}]\n",
            "",
            0,
        ),
        (
            "[.[] | recurse(.children[]?) | {(.id): 1}]",
            "[{\"x\":1},{\"y\":1}]\n",
            "",
            0,
        ),
        (
            "[.[] | recurse(.children[]?) | {id: (.a,.b)}]",
            "[{\"id\":1},{\"id\":null},{\"id\":false},{\"id\":2}]\n",
            "",
            0,
        ),
        (
            "[.[] | recurse(.children[]?) | {id: .a?}]",
            "[{\"id\":1},{\"id\":false}]\n",
            "",
            0,
        ),
        (
            "[.[] | recurse(.children[]?) | {\"x\": .id, \"x\": .a}]",
            "[{\"x\":1},{\"x\":false}]\n",
            "",
            0,
        ),
        (
            "[.[] | recurse(.children[]?) | {a: [.id, {b: .a}]}]",
            "[{\"a\":[\"x\",{\"b\":1}]},{\"a\":[\"y\",{\"b\":false}]}]\n",
            "",
            0,
        ),
        (
            "[.[] | recurse(.children[]?) | [[.a]]]",
            "[[[1]],[[false]]]\n",
            "",
            0,
        ),
        (
            "[.[] | recurse(.children[]?) | {id, a, zz}]",
            "[{\"id\":\"x\",\"a\":1,\"zz\":null},{\"id\":\"y\",\"a\":false,\"zz\":null}]\n",
            "",
            0,
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (want, want_err, want_code),
            "{filter}"
        );
    }
    Ok(())
}
