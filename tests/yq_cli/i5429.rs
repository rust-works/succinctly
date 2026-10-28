//! #5429: a filter that builds a sequence out of document nodes keeps what each node carries -- its
//! explicit tag, trailing comment, anchor and quoting -- as `map(.)` and `.[0]` already did. A
//! CloudFormation `!Ref Bucket` came out as the plain string `Bucket` through `[.Outputs[]]`, an array
//! slice, `flatten`, `map_values` and `+` of two constructions. Every output is captured from yq
//! v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

/// Run every `(filter, want)` row over `input`; the row names the filter in its failure message.
fn check(input: &str, rows: &[(&str, &str)]) -> Result<()> {
    for (filter, want) in rows {
        let (stdout, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!((stdout.as_str(), code), (*want, 0), "`{filter}`");
    }
    Ok(())
}

const CFN: &str = "Outputs:\n  BucketName:\n    Value: !Ref Bucket # the bucket\n  BucketArn:\n    Value: !GetAtt Bucket.Arn\n";

const REFS: &str = "- !Ref A\n- !Ref B\n- !Ref C\n";

const MIX: &str = "- \"x\" # c1\n- &an 2\n- [1, 2]\n";

const NEST: &str = "- [!Ref A, !Ref B]\n- [!Ref C]\n- !Ref D # d\n";

const FLOW: &str = "[!Ref A, !Ref B, \"x\"]\n";

const HEAD: &str = "# h\n- !Ref A # a\n# mid\n- [x, y] # n\n# f\n";

#[test]
fn test_cfn_template_keeps_presentation_5429() -> Result<()> {
    check(
        CFN,
        &[
            (
                "[.Outputs[]]",
                "- Value: !Ref Bucket # the bucket\n- Value: !GetAtt Bucket.Arn\n",
            ),
            (
                "[.Outputs[] | .Value]",
                "- !Ref Bucket # the bucket\n- !GetAtt Bucket.Arn\n",
            ),
            (
                ".Outputs | map_values(.Value)",
                "BucketName: !Ref Bucket # the bucket\nBucketArn: !GetAtt Bucket.Arn\n",
            ),
            (
                "[.Outputs.BucketName.Value] + [.Outputs.BucketArn.Value]",
                "- !Ref Bucket # the bucket\n- !GetAtt Bucket.Arn\n",
            ),
            (
                "[(.Outputs.BucketName.Value, .Outputs.BucketArn.Value)]",
                "- !Ref Bucket # the bucket\n- !GetAtt Bucket.Arn\n",
            ),
        ],
    )
}

#[test]
fn test_tagged_sequence_keeps_presentation_5429() -> Result<()> {
    check(
        REFS,
        &[
            ("[.[]]", "- !Ref A\n- !Ref B\n- !Ref C\n"),
            ("[.[] | select(true)]", "- !Ref A\n- !Ref B\n- !Ref C\n"),
            (".[1:]", "- !Ref B\n- !Ref C\n"),
            (".[:2]", "- !Ref A\n- !Ref B\n"),
            (".[-1:]", "- !Ref C\n"),
            (".[5:]", "[]\n"),
            (".[0:0]", "[]\n"),
            ("flatten", "- !Ref A\n- !Ref B\n- !Ref C\n"),
            ("flatten(0)", "- !Ref A\n- !Ref B\n- !Ref C\n"),
            ("[(.[0], .[1])]", "- !Ref A\n- !Ref B\n"),
            ("[.[0]] + [.[1]]", "- !Ref A\n- !Ref B\n"),
            ("map_values(.)", "- !Ref A\n- !Ref B\n- !Ref C\n"),
        ],
    )
}

#[test]
fn test_quoted_anchored_and_nested_keeps_presentation_5429() -> Result<()> {
    check(
        MIX,
        &[
            ("[.[]]", "- \"x\" # c1\n- &an 2\n- [1, 2]\n"),
            (".[0:3]", "- \"x\" # c1\n- &an 2\n- [1, 2]\n"),
            (".[1:]", "- &an 2\n- [1, 2]\n"),
        ],
    )
}

#[test]
fn test_nested_sequences_keeps_presentation_5429() -> Result<()> {
    check(
        NEST,
        &[
            ("flatten", "- !Ref A\n- !Ref B\n- !Ref C\n- !Ref D # d\n"),
            ("flatten(1)", "- !Ref A\n- !Ref B\n- !Ref C\n- !Ref D # d\n"),
            (
                "flatten(0)",
                "- [!Ref A, !Ref B]\n- [!Ref C]\n- !Ref D # d\n",
            ),
            (".[1:]", "- [!Ref C]\n- !Ref D # d\n"),
        ],
    )
}

#[test]
fn test_flow_sequence_keeps_presentation_5429() -> Result<()> {
    check(
        FLOW,
        &[
            ("[.[]]", "- !Ref A\n- !Ref B\n- \"x\"\n"),
            (".[1:]", "- !Ref B\n- \"x\"\n"),
            ("flatten", "[!Ref A, !Ref B, \"x\"]\n"),
            (".[:2]", "- !Ref A\n- !Ref B\n"),
        ],
    )
}

#[test]
fn test_head_and_foot_comments_keeps_presentation_5429() -> Result<()> {
    check(
        HEAD,
        &[
            ("flatten", "# h\n- !Ref A # a\n- x\n- y\n"),
            ("[.[]]", "- !Ref A # a\n# mid\n- [x, y] # n\n# f\n"),
            (".[0:2]", "- !Ref A # a\n# mid\n- [x, y] # n\n# f\n"),
        ],
    )
}

/// `-o=json` puts no tag, comment or quoting in its output, so the same filters answer as before.
#[test]
fn test_json_output_is_unchanged_5429() -> Result<()> {
    for (filter, want) in [
        ("[.[]]", "[\"A\",\"B\",\"C\"]\n"),
        (".[1:]", "[\"B\",\"C\"]\n"),
        ("flatten", "[\"A\",\"B\",\"C\"]\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, REFS, &["-o=json", "-I0"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "`{filter}`");
    }
    Ok(())
}
