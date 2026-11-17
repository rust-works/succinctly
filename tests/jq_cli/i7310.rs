//! #7310: an object the canonical echo scan declines only because its keys repeat is printed by
//! collapsing it straight away, without the key census that would hash and sort every key a second
//! time. The repeats sit past the scan's pairwise tier (its first 16 keys), where it settles the
//! hashes at the closing brace: `k02` repeats a pairwise-tier key and `a\nb`, an escaped key,
//! repeats one of its own tier. jq keeps each repeated key's first position and takes its last
//! value. Every expected output was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_stdin;

const INPUT: &str = r#"{"k00":0,"k01":1,"k02":2,"k03":3,"k04":4,"k05":5,"k06":6,"k07":7,"k08":8,"k09":9,"k10":10,"k11":11,"k12":12,"k13":13,"k14":14,"k15":15,"k16":16,"a\nb":{"x":[1,2]},"k02":"two","a\nb":true,"é":1.50}"#;

/// `(extra args, stdout)`.
const ROWS: &[(&[&str], &str)] = &[
    (
        &["-c"],
        r#"{"k00":0,"k01":1,"k02":"two","k03":3,"k04":4,"k05":5,"k06":6,"k07":7,"k08":8,"k09":9,"k10":10,"k11":11,"k12":12,"k13":13,"k14":14,"k15":15,"k16":16,"a\nb":true,"é":1.50}
"#,
    ),
    (
        &[],
        r#"{
  "k00": 0,
  "k01": 1,
  "k02": "two",
  "k03": 3,
  "k04": 4,
  "k05": 5,
  "k06": 6,
  "k07": 7,
  "k08": 8,
  "k09": 9,
  "k10": 10,
  "k11": 11,
  "k12": 12,
  "k13": 13,
  "k14": 14,
  "k15": 15,
  "k16": 16,
  "a\nb": true,
  "é": 1.50
}
"#,
    ),
    (
        &["--tab"],
        r#"{
	"k00": 0,
	"k01": 1,
	"k02": "two",
	"k03": 3,
	"k04": 4,
	"k05": 5,
	"k06": 6,
	"k07": 7,
	"k08": 8,
	"k09": 9,
	"k10": 10,
	"k11": 11,
	"k12": 12,
	"k13": 13,
	"k14": 14,
	"k15": 15,
	"k16": 16,
	"a\nb": true,
	"é": 1.50
}
"#,
    ),
];

#[test]
fn repeated_keys_past_the_pairwise_tier_collapse_as_jq_does_7310() -> Result<()> {
    for &(args, expected) in ROWS {
        let (out, code) = run_jq_stdin(".", INPUT, args)?;
        assert_eq!((out.as_str(), code), (expected, 0), "{args:?}");
    }
    Ok(())
}
