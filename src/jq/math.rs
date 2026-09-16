//! The libm behind jq's floating-point builtins (#3045).
//!
//! Real jq computes `sin`, `exp`, `pow`, `sqrt`, ... by calling the C library
//! it was linked against: Apple's libm for the pinned `/usr/bin/jq`
//! 1.7.1-apple, glibc for the `jq-linux-amd64` release binary the `jq-drift`
//! CI job checks the goldens with. Neither is correctly rounded, and they
//! disagree with each other -- and with the pure-Rust `libm` crate, a musl
//! port -- in the last bit on a large share of ordinary inputs: over 400
//! sampled inputs, the crate's `tan` differs from Apple's on 163 and from
//! glibc's on 19, while the two real jqs differ from each other on 161. The
//! only way to print jq's digits is to call the same library jq does, so
//! under `std` every function here is a direct `extern "C"` binding to the
//! platform libm; that is bit-exact against the pinned jq on macOS and
//! against the release jq on glibc, on every sampled input of every function.
//! Without `std` there is no platform libm to link, and the `libm` crate
//! stands in with a recorded last-bit divergence
//! (`docs/compliance/jq/limitations.md`).
//!
//! These are deliberately the C symbols, not the `f64` inherent methods.
//! `f64::atanh` is a formula (`0.5 * ((2x) / (1 - x)).ln_1p()`), not a libm
//! call, and differs from Apple's `atanh` on 130/400 inputs; `f64::sqrt` is
//! correctly rounded but the Newton iteration this module replaces was not
//! (161/400). Naming the symbol is the only spelling that pins which
//! implementation runs.

#![allow(unsafe_code)] // FFI to the platform libm: pure by-value f64 functions

macro_rules! libm_fns {
    ($( $name:ident ( $($arg:ident),+ ) => $sym:ident ; )+) => {
        #[cfg(feature = "std")]
        mod platform {
            extern "C" {
                $( pub fn $sym($($arg: f64),+) -> f64; )+
            }
        }

        $(
            #[inline]
            pub(crate) fn $name($($arg: f64),+) -> f64 {
                #[cfg(feature = "std")]
                {
                    // SAFETY: a C math function taking and returning `f64` by
                    // value, with no pointers, no global state and no
                    // preconditions -- every input pattern is a valid double.
                    unsafe { platform::$sym($($arg),+) }
                }
                #[cfg(not(feature = "std"))]
                {
                    libm::$sym($($arg),+)
                }
            }
        )+
    };
}

libm_fns! {
    sin(x) => sin;
    cos(x) => cos;
    tan(x) => tan;
    asin(x) => asin;
    acos(x) => acos;
    atan(x) => atan;
    atan2(y, x) => atan2;
    sinh(x) => sinh;
    cosh(x) => cosh;
    tanh(x) => tanh;
    asinh(x) => asinh;
    acosh(x) => acosh;
    atanh(x) => atanh;
    exp(x) => exp;
    exp2(x) => exp2;
    log(x) => log;
    log10(x) => log10;
    log2(x) => log2;
    pow(x, y) => pow;
    sqrt(x) => sqrt;
    trunc(x) => trunc;
    fabs(x) => fabs;
}

/// jq's `exp10` is whatever the platform calls it: `__exp10` on Apple
/// platforms (jq's `builtin.c` renames it), `exp10` on glibc and musl. On a
/// platform with neither, jq's own build defines `exp10/0` as an "not found
/// at build time" error; succinctly falls back to `pow(10, x)` there instead,
/// which is not bit-exact against any jq but is the closest value on offer.
#[inline]
pub(crate) fn exp10(x: f64) -> f64 {
    #[cfg(all(feature = "std", target_vendor = "apple"))]
    {
        extern "C" {
            #[link_name = "__exp10"]
            fn apple_exp10(x: f64) -> f64;
        }
        // SAFETY: as for the functions above.
        unsafe { apple_exp10(x) }
    }
    #[cfg(all(
        feature = "std",
        not(target_vendor = "apple"),
        any(target_os = "linux", target_os = "android")
    ))]
    {
        extern "C" {
            fn exp10(x: f64) -> f64;
        }
        // SAFETY: as for the functions above.
        unsafe { exp10(x) }
    }
    #[cfg(all(
        feature = "std",
        not(target_vendor = "apple"),
        not(any(target_os = "linux", target_os = "android"))
    ))]
    {
        pow(10.0, x)
    }
    #[cfg(not(feature = "std"))]
    {
        libm::exp10(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inputs are `i * 0.137 - 20` (or the other sweep sets, noted per row)
    /// as two IEEE operations, so they are the exact doubles jq computes for
    /// `range(1;401) | . * 0.137 - 20`. Every expected value was captured from
    /// *both* the pinned `/usr/bin/jq` 1.7.1-apple and the `jq-linux-amd64`
    /// 1.7.1 release binary, which agree on these rows, and the `libm` crate
    /// disagrees on each one (except `atanh`, where the crate matches glibc and
    /// the divergent implementation is Rust's own `f64::atanh` formula, off by
    /// several ulps on both platforms). A regression to either the crate or
    /// the inherent methods therefore fails here on both measured platforms.
    /// Gated to those platforms; the hermetic golden case
    /// `math_platform_libm_bits` covers the remaining CI legs.
    #[cfg(all(
        feature = "std",
        any(target_os = "macos", all(target_os = "linux", target_arch = "x86_64"))
    ))]
    #[test]
    fn platform_libm_matches_the_platform_jq_bit_for_bit() {
        let a = |i: u32| f64::from(i) * 0.137 - 20.0;
        let p = |i: u32| f64::from(i) * 0.137;
        let u = |i: u32| f64::from(i) * 0.005 - 1.0025;
        let rows: [(&str, f64, f64); 11] = [
            ("tan", tan(a(10)), 0.223_153_182_896_234_45),
            ("tan", tan(a(12)), 0.537_964_437_295_841_3),
            ("sin", sin(a(60)), 0.707_794_073_405_996_3),
            ("cos", cos(a(23)), -0.416_652_270_240_986_2),
            ("atan", atan(a(134)), -1.023_775_673_523_293),
            ("cosh", cosh(a(5)), 122_283_517.367_918_3),
            ("exp", exp(a(6)), 4.689_218_028_746_673e-9),
            ("exp10", exp10(a(18)), 2.924_152_377_843_342_6e-18),
            ("log", log(p(12)), 0.497_132_296_633_988_2),
            ("pow", pow(p(9), a(9) / 3.0), 0.269_752_397_845_759_5),
            ("atanh", atanh(u(1)), -3.341_680_472_883_125_8),
        ];
        for (name, got, want) in rows {
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "{name}: got {got:?}, jq 1.7.1 prints {want:?}"
            );
        }
    }

    /// The exact operations are the same everywhere; pin them so the shim
    /// cannot silently route them somewhere inexact again (the Newton `sqrt`
    /// this module replaced was 161/400 off, e.g. `0.137 | sqrt` printed
    /// `0.370135110466435` where jq prints `0.37013511046643494`).
    #[test]
    fn exact_operations_are_ieee() {
        assert_eq!(
            sqrt(0.137).to_bits(),
            0.370_135_110_466_434_94_f64.to_bits()
        );
        assert_eq!(sqrt(2.0), core::f64::consts::SQRT_2);
        assert_eq!(sqrt(0.0).to_bits(), 0.0_f64.to_bits());
        assert!(sqrt(-1.0).is_nan());
        assert_eq!(trunc(-2.7), -2.0);
        assert_eq!(fabs(-3.5), 3.5);
        assert_eq!(pow(2.0, 10.0), 1024.0);
    }
}
