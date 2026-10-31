//! #5680: `pow` and `atan2` of a number whose operands are `.` or a literal are answered from the
//! number, like the one-operand libm builtins since #5669, instead of writing it out as JSON,
//! indexing it and evaluating the call over that document. The answer is the builtin's own libm
//! call, so the tests pin what is the same on every platform: exact results, the domain edges
//! (an infinity, a NaN printed as `null`), signed zeros and results rounded to nine places.
//! Expected values were captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_pow_and_atan2_of_a_number_match_jq_5680() -> Result<()> {
    for (program, input, expected) in [
        (
            r"map(pow(.;2))",
            r"[0,1,2,3,-2,0.5,10]",
            r"[0,1,4,9,4,0.25,100]",
        ),
        (
            r"map(pow(2;.))",
            r"[0,1,10,-1,0.5]",
            r"[1,2,1024,0.5,1.4142135623730951]",
        ),
        (r"map(pow(.;.))", r"[1,2,3]", r"[1,4,27]"),
        (r"map(pow(.;0))", r"[0,5,-5]", r"[1,1,1]"),
        (
            r"map(pow(.;-1))",
            r"[0,1,2,4]",
            r"[1.7976931348623157e+308,1,0.5,0.25]",
        ),
        (r"map(pow(.;0.5))", r"[-1,4,9]", r"[null,2,3]"),
        (r"map(pow(-8;.))", r"[1,2,3]", r"[-8,64,-512]"),
        (r"map(pow(.;1000))", r"[10]", r"[1.7976931348623157e+308]"),
        (
            r"map(pow(10;.))",
            r"[400,-400]",
            r"[1.7976931348623157e+308,0]",
        ),
        (r"map(. + 1 | pow(.;2))", r"[0,1,2]", r"[1,4,9]"),
        (r"map(. * 2 | pow(2;.))", r"[1,2,3]", r"[4,16,64]"),
        (r"map(. + 0 | pow(.;.))", r"[2,3]", r"[4,27]"),
        (r"map(atan2(0;.))", r"[1,-1,0]", r"[0,3.141592653589793,0]"),
        (
            r"map(atan2(.;0))",
            r"[1,-1]",
            r"[1.5707963267948966,-1.5707963267948966]",
        ),
        (
            r"map(atan2(.;.))",
            r"[1,-1,0]",
            r"[0.7853981633974483,-2.356194490192345,0]",
        ),
        (r"map([pow(.;(1,2))])", r"[2,3]", r"[[2,4],[3,9]]"),
        (r"map([pow((2,3);.)])", r"[2]", r"[[4,9]]"),
        (r"map(try pow(.;1) catch 0)", r"[1,2]", r"[1,2]"),
        (
            r"map(pow(-0;.))",
            r"[1,-1]",
            r"[-0,-1.7976931348623157e+308]",
        ),
        (r"map(atan2(-0;-1))", r"[0]", r"[-3.141592653589793]"),
        (
            r"map(atan2(.;1) | . * 1e9 | round / 1e9)",
            r"[0.1,0.25,0.5,0.75,0.9,1,1.5,2]",
            r"[0.099668652,0.244978663,0.463647609,0.643501109,0.732815102,0.785398163,0.982793723,1.107148718]",
        ),
        (
            r"map(. + 0 | atan2(.;1) | . * 1e9 | round / 1e9)",
            r"[0.1,0.25,0.5,0.75,0.9,1,1.5,2]",
            r"[0.099668652,0.244978663,0.463647609,0.643501109,0.732815102,0.785398163,0.982793723,1.107148718]",
        ),
        (
            r"map(atan2(1;.) | . * 1e9 | round / 1e9)",
            r"[0.1,0.25,0.5,0.75,0.9,1,1.5,2]",
            r"[1.471127674,1.325817664,1.107148718,0.927295218,0.837981225,0.785398163,0.588002604,0.463647609]",
        ),
        (
            r"map(. + 0 | atan2(1;.) | . * 1e9 | round / 1e9)",
            r"[0.1,0.25,0.5,0.75,0.9,1,1.5,2]",
            r"[1.471127674,1.325817664,1.107148718,0.927295218,0.837981225,0.785398163,0.588002604,0.463647609]",
        ),
        (
            r"map(pow(.;1.5) | . * 1e9 | round / 1e9)",
            r"[0.1,0.25,0.5,0.75,0.9,1,1.5,2]",
            r"[0.031622777,0.125,0.353553391,0.649519053,0.853814968,1,1.837117307,2.828427125]",
        ),
        (
            r"map(. + 0 | pow(.;1.5) | . * 1e9 | round / 1e9)",
            r"[0.1,0.25,0.5,0.75,0.9,1,1.5,2]",
            r"[0.031622777,0.125,0.353553391,0.649519053,0.853814968,1,1.837117307,2.828427125]",
        ),
        (
            r"map(pow(1.5;.) | . * 1e9 | round / 1e9)",
            r"[0.1,0.25,0.5,0.75,0.9,1,1.5,2]",
            r"[1.041379744,1.10668192,1.224744871,1.355403005,1.440396751,1.5,1.837117307,2.25]",
        ),
        (
            r"map(. + 0 | pow(1.5;.) | . * 1e9 | round / 1e9)",
            r"[0.1,0.25,0.5,0.75,0.9,1,1.5,2]",
            r"[1.041379744,1.10668192,1.224744871,1.355403005,1.440396751,1.5,1.837117307,2.25]",
        ),
        (
            r"map(atan2(.;.) | . * 1e9 | round / 1e9)",
            r"[0.1,0.25,0.5,0.75,0.9,1,1.5,2]",
            r"[0.785398163,0.785398163,0.785398163,0.785398163,0.785398163,0.785398163,0.785398163,0.785398163]",
        ),
        (
            r"map(. + 0 | atan2(.;.) | . * 1e9 | round / 1e9)",
            r"[0.1,0.25,0.5,0.75,0.9,1,1.5,2]",
            r"[0.785398163,0.785398163,0.785398163,0.785398163,0.785398163,0.785398163,0.785398163,0.785398163]",
        ),
        (r"map(pow(.;1))", r"[-0]", r"[-0]"),
        (r"map(pow(.;1))", r"[-0.0]", r"[-0]"),
        (r"map(atan2(.;-1))", r"[-0]", r"[-3.141592653589793]"),
        (r"map(atan2(-0;.))", r"[-1,1]", r"[-3.141592653589793,-0]"),
        (
            r"map(pow(.;1))",
            r"[869389897822472004]",
            r"[869389897822472000]",
        ),
        (
            r"map(pow(.;2))",
            r"[9007199254740993,1e19]",
            r"[8.112963841460668e+31,1e+38]",
        ),
        (
            r"map(pow(.;2))",
            r"[1.0,1E2,0.1]",
            r"[1,10000,0.010000000000000002]",
        ),
        (
            r"map(pow(.;1))",
            r"[1e500,-1e500]",
            r"[1.7976931348623157e+308,-1.7976931348623157e+308]",
        ),
        (r"map(atan2(.;1))", r"[1e500]", r"[1.5707963267948966]"),
        (r"map(. + 0 | pow(.;1))", r"[-0,-0.0]", r"[0,0]"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(input))?;
        assert_eq!(code, 0, "{program}: {stderr:?}");
        assert_eq!(stdout.trim_end(), expected, "{program} over {input}");
    }
    Ok(())
}

/// A value that is not a number keeps the evaluator's message and exit status.
#[test]
fn test_pow_and_atan2_of_a_non_number_report_jq_s_error_5680() -> Result<()> {
    for (program, input, message) in [
        (
            r"map(pow(.;2))",
            r#"["a"]"#,
            r#"jq: error (at <stdin>:0): string ("a") number required"#,
        ),
        (
            r"map(atan2(.;1))",
            r"[null]",
            r"jq: error (at <stdin>:0): null (null) number required",
        ),
        (
            r"map(pow(2;.))",
            r"[[1]]",
            r"jq: error (at <stdin>:0): array ([1]) number required",
        ),
        (
            r"map(pow(.;2))",
            r#"[{"a":1}]"#,
            r#"jq: error (at <stdin>:0): object ({"a":1}) number required"#,
        ),
        (
            r"map(atan2(1;.))",
            r"[true]",
            r"jq: error (at <stdin>:0): boolean (true) number required",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(input))?;
        assert_eq!(code, 5, "{program}");
        assert_eq!(stdout.trim_end(), "", "{program}");
        assert_eq!(stderr.trim_end(), message, "{program}");
    }
    Ok(())
}
