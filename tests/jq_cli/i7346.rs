//! #7346: an object the canonical scan declines only because its keys repeat is written from its
//! text: the runs of members jq's rule leaves alone are copied (`-c`) or re-indented, and each
//! repeated key is written in its first place with its last value. The repeats sit past the scan's
//! pairwise tier (its first 17 keys): `k05` takes a nested object, `k\u0001` (an escaped key)
//! repeats a key of its own tier, and `k00`'s last occurrence, a nested array, is the object's last
//! field, which is dropped. Every expected output was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_stdin;

const INPUT: &str = r#"{"k00":0,"k01":1,"k02":2,"k03":3,"k04":4,"k05":5,"k06":6,"k07":7,"k08":8,"k09":9,"k10":10,"k11":11,"k12":12,"k13":13,"k14":14,"k15":15,"k16":16,"k05":{"a":[1,{"b":[]}],"c":{}},"x":[[],[1,[2]]],"k\u0001":"s","k\u0001":-0.0,"k00":[{"z":null}]}"#;

/// `(extra args, stdout)`.
const ROWS: &[(&[&str], &str)] = &[
    (
        &["-c"],
        r#"{"k00":[{"z":null}],"k01":1,"k02":2,"k03":3,"k04":4,"k05":{"a":[1,{"b":[]}],"c":{}},"k06":6,"k07":7,"k08":8,"k09":9,"k10":10,"k11":11,"k12":12,"k13":13,"k14":14,"k15":15,"k16":16,"x":[[],[1,[2]]],"k\u0001":-0.0}
"#,
    ),
    (
        &[],
        r#"{
  "k00": [
    {
      "z": null
    }
  ],
  "k01": 1,
  "k02": 2,
  "k03": 3,
  "k04": 4,
  "k05": {
    "a": [
      1,
      {
        "b": []
      }
    ],
    "c": {}
  },
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
  "x": [
    [],
    [
      1,
      [
        2
      ]
    ]
  ],
  "k\u0001": -0.0
}
"#,
    ),
    (
        &["--tab"],
        r#"{
	"k00": [
		{
			"z": null
		}
	],
	"k01": 1,
	"k02": 2,
	"k03": 3,
	"k04": 4,
	"k05": {
		"a": [
			1,
			{
				"b": []
			}
		],
		"c": {}
	},
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
	"x": [
		[],
		[
			1,
			[
				2
			]
		]
	],
	"k\u0001": -0.0
}
"#,
    ),
    (
        &["--indent", "3"],
        r#"{
   "k00": [
      {
         "z": null
      }
   ],
   "k01": 1,
   "k02": 2,
   "k03": 3,
   "k04": 4,
   "k05": {
      "a": [
         1,
         {
            "b": []
         }
      ],
      "c": {}
   },
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
   "x": [
      [],
      [
         1,
         [
            2
         ]
      ]
   ],
   "k\u0001": -0.0
}
"#,
    ),
    (
        &["--indent", "1"],
        r#"{
 "k00": [
  {
   "z": null
  }
 ],
 "k01": 1,
 "k02": 2,
 "k03": 3,
 "k04": 4,
 "k05": {
  "a": [
   1,
   {
    "b": []
   }
  ],
  "c": {}
 },
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
 "x": [
  [],
  [
   1,
   [
    2
   ]
  ]
 ],
 "k\u0001": -0.0
}
"#,
    ),
];

#[test]
fn repeated_keys_print_from_their_text_as_jq_does_7346() -> Result<()> {
    for &(args, expected) in ROWS {
        let (out, code) = run_jq_stdin(".", INPUT, args)?;
        assert_eq!((out.as_str(), code), (expected, 0), "{args:?}");
    }
    Ok(())
}
