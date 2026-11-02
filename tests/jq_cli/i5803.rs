//! #5803: a `reduce`/`foreach` body that stores a value read through a container named argument
//! (`.[$k] = $d[$i].age`) shares the value on the value side of the store, as #5774 does for a
//! read; the target of the store is a path and keeps the literal. The rows pin that a store
//! answers as the literal did, that a stored container is never written through the argument
//! (`$d[0].age` after `.k0.age = 99`), and that the target still resolves. Outputs are captured
//! from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

const D: &str = "[{\"id\":1,\"age\":30,\"t\":[1,2]},{\"id\":2,\"age\":41,\"t\":[3]},{\"id\":3,\"age\":25,\"t\":[]}]";

/// `(filter, stdout)` over `--argjson d D`, every one exiting 0 under `-nc`.
const ROWS: &[(&str, &str)] = &[
    ("reduce range(3) as $i ({}; .[\"k\\($i)\"] = $d[$i].age)", "{\"k0\":30,\"k1\":41,\"k2\":25}\n"),
    ("reduce range(3) as $i ({}; .[\"k\\($i)\"] = $d[$i])", "{\"k0\":{\"id\":1,\"age\":30,\"t\":[1,2]},\"k1\":{\"id\":2,\"age\":41,\"t\":[3]},\"k2\":{\"id\":3,\"age\":25,\"t\":[]}}\n"),
    ("reduce range(3) as $i ({}; .[\"k\\($i)\"] = $d[$i]) | .k0.age = 99 | [.k0.age, $d[0].age]", "[99,30]\n"),
    ("reduce range(3) as $i ([]; .[$i] = $d[$i].t) | .[0] += [9] | [., $d[0].t]", "[[[1,2,9],[3],[]],[1,2]]\n"),
    ("reduce range(3) as $i (0; . += $d[$i].age)", "96\n"),
    ("reduce range(3) as $i ({}; .[$d[$i].id|tostring] = $d[$i].age)", "{\"1\":30,\"2\":41,\"3\":25}\n"),
    ("reduce range(3) as $i ($d[0]; .age += $d[$i].age) | [., $d[0].age]", "[{\"id\":1,\"age\":126,\"t\":[1,2]},30]\n"),
    ("reduce range(3) as $i ({}; .a |= ($d[$i].t | length))", "{\"a\":0}\n"),
    ("reduce range(3) as $i ({}; .x //= $d[$i].age)", "{\"x\":30}\n"),
    ("[foreach range(3) as $i ({}; .[$i|tostring] = $d[$i].id; .)] | length", "3\n"),
    ("reduce range(3) as $i ({}; .[\"k\\($i)\"] = $d[$i]) | [.k1 == $d[1], (.k1 | .id)]", "[true,2]\n"),
    ("$d | .[0].age = $d[1].age | .[0].age", "41\n"),
    ("{} | .a = $d[0].id", "{\"a\":1}\n"),
    ("[$d[] | .age] | .[0] = $d[2].age | .[0]", "25\n"),
    ("$d | map(.age = $d[0].age) | map(.age)", "[30,30,30]\n"),
    ("reduce range(3) as $i ({}; .l = $d[$i].t) | .l += [7] | [., $d[2].t]", "[{\"l\":[7]},[]]\n"),
    ("reduce range(3) as $i ([]; .[$i] = $d) | map(length)", "[3,3,3]\n"),
    ("reduce range(3) as $i ({}; .a = $d[$i] | .b = $d[$i].t) | [.a.id, .b]", "[3,[]]\n"),
    ("reduce range(2) as $i ({}; .a = $d[0] | .a.id = $i) | [.a.id, $d[0].id]", "[1,1]\n"),
];

#[test]
fn a_store_that_reads_a_container_named_argument_answers_as_jq_5803() -> Result<()> {
    for (filter, expected) in ROWS {
        let (stdout, stderr, code) = run_jq_full(&["-nc", "--argjson", "d", D, filter], None)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{filter}: {stderr}"
        );
    }
    Ok(())
}
