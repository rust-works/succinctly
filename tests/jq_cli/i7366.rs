//! #7366: a document that is canonical but for one object inside it that repeats a key (a member,
//! an element, deeper, an object of the pairwise tier and of the wide one) is printed from its text
//! around that object, and only the object by its keys. Two such objects keep the walk. Every row
//! is pinned to jq 1.7.1's own output, compact, indented and with tabs.

use anyhow::Result;

use crate::run_jq_full;

/// `member`
const MEMBER: &str = "{\"data\":{\"a\":1,\"a\":2,\"b\":[1,{\"c\":2}]},\"z\":[true,{}]}";

/// `element`
const ELEMENT: &str = "[1,{\"k\":\"v\",\"k\":{\"x\":[]}},{\"q\":1}]";

/// `deep`
const DEEP: &str = "{\"o\":{\"p\":[{\"a\":1,\"b\":2,\"a\":3}]},\"e\":[]}";

/// `two`
const TWO: &str = "[{\"a\":1,\"a\":2},{\"b\":1,\"b\":2}]";

/// `wide_early`
const WIDE_EARLY: &str = "{\"data\":{\"k00\":0,\"k00\":1,\"k02\":2,\"k03\":3,\"k04\":4,\"k05\":5,\"k06\":6,\"k07\":7,\"k08\":8,\"k09\":9,\"k10\":10,\"k11\":11,\"k12\":12,\"k13\":13,\"k14\":14,\"k15\":15,\"k16\":16,\"k17\":17,\"k18\":18,\"k19\":19},\"n\":1.5}";

/// `wide_late`
const WIDE_LATE: &str = "[{\"k00\":0,\"k01\":1,\"k02\":2,\"k03\":3,\"k04\":4,\"k05\":5,\"k06\":6,\"k07\":7,\"k08\":8,\"k09\":9,\"k10\":10,\"k11\":11,\"k12\":12,\"k13\":13,\"k14\":14,\"k15\":15,\"k16\":16,\"k17\":17,\"k03\":18,\"k19\":19},\"tail\"]";

#[test]
fn a_nested_repeat_prints_around_its_object_as_jq_does_7366() -> Result<()> {
    let rows: &[(&str, &[&str], &str)] = &[
        (MEMBER, &["-c", "."], "{\"data\":{\"a\":2,\"b\":[1,{\"c\":2}]},\"z\":[true,{}]}\n"),
        (MEMBER, &["."], "{\n  \"data\": {\n    \"a\": 2,\n    \"b\": [\n      1,\n      {\n        \"c\": 2\n      }\n    ]\n  },\n  \"z\": [\n    true,\n    {}\n  ]\n}\n"),
        (MEMBER, &["--tab", "."], "{\n\t\"data\": {\n\t\t\"a\": 2,\n\t\t\"b\": [\n\t\t\t1,\n\t\t\t{\n\t\t\t\t\"c\": 2\n\t\t\t}\n\t\t]\n\t},\n\t\"z\": [\n\t\ttrue,\n\t\t{}\n\t]\n}\n"),
        (ELEMENT, &["-c", "."], "[1,{\"k\":{\"x\":[]}},{\"q\":1}]\n"),
        (ELEMENT, &["."], "[\n  1,\n  {\n    \"k\": {\n      \"x\": []\n    }\n  },\n  {\n    \"q\": 1\n  }\n]\n"),
        (ELEMENT, &["--tab", "."], "[\n\t1,\n\t{\n\t\t\"k\": {\n\t\t\t\"x\": []\n\t\t}\n\t},\n\t{\n\t\t\"q\": 1\n\t}\n]\n"),
        (DEEP, &["-c", "."], "{\"o\":{\"p\":[{\"a\":3,\"b\":2}]},\"e\":[]}\n"),
        (DEEP, &["."], "{\n  \"o\": {\n    \"p\": [\n      {\n        \"a\": 3,\n        \"b\": 2\n      }\n    ]\n  },\n  \"e\": []\n}\n"),
        (DEEP, &["--tab", "."], "{\n\t\"o\": {\n\t\t\"p\": [\n\t\t\t{\n\t\t\t\t\"a\": 3,\n\t\t\t\t\"b\": 2\n\t\t\t}\n\t\t]\n\t},\n\t\"e\": []\n}\n"),
        (TWO, &["-c", "."], "[{\"a\":2},{\"b\":2}]\n"),
        (TWO, &["."], "[\n  {\n    \"a\": 2\n  },\n  {\n    \"b\": 2\n  }\n]\n"),
        (TWO, &["--tab", "."], "[\n\t{\n\t\t\"a\": 2\n\t},\n\t{\n\t\t\"b\": 2\n\t}\n]\n"),
        (WIDE_EARLY, &["-c", "."], "{\"data\":{\"k00\":1,\"k02\":2,\"k03\":3,\"k04\":4,\"k05\":5,\"k06\":6,\"k07\":7,\"k08\":8,\"k09\":9,\"k10\":10,\"k11\":11,\"k12\":12,\"k13\":13,\"k14\":14,\"k15\":15,\"k16\":16,\"k17\":17,\"k18\":18,\"k19\":19},\"n\":1.5}\n"),
        (WIDE_EARLY, &["."], "{\n  \"data\": {\n    \"k00\": 1,\n    \"k02\": 2,\n    \"k03\": 3,\n    \"k04\": 4,\n    \"k05\": 5,\n    \"k06\": 6,\n    \"k07\": 7,\n    \"k08\": 8,\n    \"k09\": 9,\n    \"k10\": 10,\n    \"k11\": 11,\n    \"k12\": 12,\n    \"k13\": 13,\n    \"k14\": 14,\n    \"k15\": 15,\n    \"k16\": 16,\n    \"k17\": 17,\n    \"k18\": 18,\n    \"k19\": 19\n  },\n  \"n\": 1.5\n}\n"),
        (WIDE_EARLY, &["--tab", "."], "{\n\t\"data\": {\n\t\t\"k00\": 1,\n\t\t\"k02\": 2,\n\t\t\"k03\": 3,\n\t\t\"k04\": 4,\n\t\t\"k05\": 5,\n\t\t\"k06\": 6,\n\t\t\"k07\": 7,\n\t\t\"k08\": 8,\n\t\t\"k09\": 9,\n\t\t\"k10\": 10,\n\t\t\"k11\": 11,\n\t\t\"k12\": 12,\n\t\t\"k13\": 13,\n\t\t\"k14\": 14,\n\t\t\"k15\": 15,\n\t\t\"k16\": 16,\n\t\t\"k17\": 17,\n\t\t\"k18\": 18,\n\t\t\"k19\": 19\n\t},\n\t\"n\": 1.5\n}\n"),
        (WIDE_LATE, &["-c", "."], "[{\"k00\":0,\"k01\":1,\"k02\":2,\"k03\":18,\"k04\":4,\"k05\":5,\"k06\":6,\"k07\":7,\"k08\":8,\"k09\":9,\"k10\":10,\"k11\":11,\"k12\":12,\"k13\":13,\"k14\":14,\"k15\":15,\"k16\":16,\"k17\":17,\"k19\":19},\"tail\"]\n"),
        (WIDE_LATE, &["."], "[\n  {\n    \"k00\": 0,\n    \"k01\": 1,\n    \"k02\": 2,\n    \"k03\": 18,\n    \"k04\": 4,\n    \"k05\": 5,\n    \"k06\": 6,\n    \"k07\": 7,\n    \"k08\": 8,\n    \"k09\": 9,\n    \"k10\": 10,\n    \"k11\": 11,\n    \"k12\": 12,\n    \"k13\": 13,\n    \"k14\": 14,\n    \"k15\": 15,\n    \"k16\": 16,\n    \"k17\": 17,\n    \"k19\": 19\n  },\n  \"tail\"\n]\n"),
        (WIDE_LATE, &["--tab", "."], "[\n\t{\n\t\t\"k00\": 0,\n\t\t\"k01\": 1,\n\t\t\"k02\": 2,\n\t\t\"k03\": 18,\n\t\t\"k04\": 4,\n\t\t\"k05\": 5,\n\t\t\"k06\": 6,\n\t\t\"k07\": 7,\n\t\t\"k08\": 8,\n\t\t\"k09\": 9,\n\t\t\"k10\": 10,\n\t\t\"k11\": 11,\n\t\t\"k12\": 12,\n\t\t\"k13\": 13,\n\t\t\"k14\": 14,\n\t\t\"k15\": 15,\n\t\t\"k16\": 16,\n\t\t\"k17\": 17,\n\t\t\"k19\": 19\n\t},\n\t\"tail\"\n]\n"),
    ];
    for &(doc, args, want) in rows {
        let (stdout, stderr, code) = run_jq_full(args, Some(doc))?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (want, "", 0),
            "{args:?} {doc}"
        );
    }
    Ok(())
}
