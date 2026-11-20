//! #7260 (startup on an Apple M4 Pro): the two cuts to what `jq -n 1` pays before it reads its
//! program must not move a result. The order of named flags now comes from clap's own matches
//! instead of a second scan of argv, and on Apple platforms every libm function behind a math builtin
//! is looked up on its first call instead of being bound while the process starts. These rows call
//! each such function once in one process (and some twice, through the kept address) with inputs whose
//! results are exact on every libm (checked against jq 1.7.1 on glibc and on Apple's libm: `27|cbrt` is
//! `3.0000000000000004` on glibc), and put the named flags in the orders jq applies them in. Every
//! expected text was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// Every function `src/jq/math.rs` calls in the platform libm, in one program.
#[test]
fn test_each_libm_builtin_answers_on_its_first_and_later_calls_7260() -> Result<()> {
    let program = r#"[(0|sin,cos,tan,asin,atan,sinh,cosh,tanh,asinh,atanh,exp,erf,erfc,expm1,log1p,j0,j1), (1|acos,acosh,log,lgamma), (3|exp2,logb), (1|cbrt), (8|log2,significand,frexp), (1000|log10), (16|sqrt), (5|tgamma), (2|exp10), (3.5|modf), (1|lgamma_r), (0|atan2(.;1)), pow(2;10), hypot(3;4), fmod(7;3), fdim(5;3), remainder(7;4), nextafter(1;1), fma(2;3;4), ldexp(3;2), scalb(3;2), scalbln(3;2), jn(2;0), (0|y0,y1), yn(2;0)|tostring]|join(" ")"#;
    let expected = "0 1 0 0 0 0 1 0 0 0 1 0 1 0 0 1 0 0 0 0 0 8 1 1 3 1 [0.5,4] 3 4 24 100 [0.5,3] [0,1] 0 1024 5 1 2 -1 1 10 12 12 12 0 -1.7976931348623157e+308 -1.7976931348623157e+308 -1.7976931348623157e+308\n";
    assert_eq!(
        run_jq_full(&["-nr", program], None)?,
        (expected.to_string(), String::new(), 0)
    );
    // A second call of each in the same process goes through the address the first one kept.
    assert_eq!(
        run_jq_full(&["-nc", "[range(3) | (0|cos), (16|sqrt), pow(2;10)]"], None)?,
        (
            "[1,4,1024,1,4,1024,1,4,1024]\n".to_string(),
            String::new(),
            0
        )
    );
    Ok(())
}

/// The named flags apply in the order given (a name keeps its first value) whichever kinds are mixed,
/// with options after `--args` hoisted, and with the perf suite's `args_many` row.
#[test]
fn test_named_flags_apply_in_argv_order_from_clap_matches_7260() -> Result<()> {
    let cases: [(&[&str], &str); 4] = [
        (
            &[
                "-nc",
                "--argjson",
                "a",
                "1",
                "--arg",
                "a",
                "2",
                "--arg",
                "b",
                "x",
                "--argjson",
                "b",
                "3",
                "[$a,$b]",
            ],
            "[1,\"x\"]\n",
        ),
        (
            &[
                "-nc",
                "--arg",
                "a",
                "1",
                "--argjson",
                "b",
                "{\"x\":2}",
                "--arg",
                "c",
                "3",
                "[$a,$b,$c,$ARGS.named]",
            ],
            "[\"1\",{\"x\":2},\"3\",{\"a\":\"1\",\"b\":{\"x\":2},\"c\":\"3\"}]\n",
        ),
        (
            &[
                "-nc",
                "--arg",
                "a",
                "1",
                "--args",
                "[$a, $ARGS.positional]",
                "x",
                "--argjson",
                "a",
                "2",
                "y",
            ],
            "[\"1\",[\"x\",\"y\"]]\n",
        ),
        (
            &[
                "-n",
                "--arg",
                "a",
                "1",
                "--arg",
                "b",
                "two",
                "--arg",
                "c",
                "3",
                "--arg",
                "d",
                "four",
                "--arg",
                "e",
                "5",
                "--arg",
                "f",
                "six",
                "--arg",
                "g",
                "7",
                "--arg",
                "h",
                "eight",
                "--argjson",
                "i",
                "9",
                "--argjson",
                "j",
                "[10,11]",
                "--argjson",
                "k",
                "{\"l\":12}",
                "[$a,$b,$c,$d,$e,$f,$g,$h,$i,$j,$k]|length",
            ],
            "11\n",
        ),
    ];
    for (args, expected) in cases {
        assert_eq!(
            run_jq_full(args, None)?,
            (expected.to_string(), String::new(), 0),
            "{args:?}"
        );
    }
    Ok(())
}
