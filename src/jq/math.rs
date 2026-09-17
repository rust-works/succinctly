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
//! against the release jq on glibc, on every sampled input of every function
//! -- with one version caveat: that release binary is static and embeds glibc
//! 2.35, and glibc 2.39 rewrote `exp10` (137/400 differ from 2.35; 2.39's
//! agrees with `pow(10, x)` on all 400 and with Apple's on 399). Every other
//! function is identical across 2.35 and 2.39. Without `std` there is no
//! platform libm to link, and the `libm` crate stands in with a recorded
//! last-bit divergence (`docs/compliance/jq/limitations.md`).
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
    // #3042
    erf(x) => erf;
    erfc(x) => erfc;
    expm1(x) => expm1;
    log1p(x) => log1p;
    tgamma(x) => tgamma;
    lgamma(x) => lgamma;
    rint(x) => rint;
    hypot(x, y) => hypot;
    fmod(x, y) => fmod;
    fdim(x, y) => fdim;
    copysign(x, y) => copysign;
    remainder(x, y) => remainder;
    nextafter(x, y) => nextafter;
    fma(x, y, z) => fma;
}

/// The Bessel functions, `lgamma_r` and `scalb` are POSIX rather than C99,
/// and MSVC's UCRT exports them under `_j0`-style names or (for `lgamma_r`)
/// not at all; the release workflow builds `x86_64-pc-windows-msvc`. There
/// is no Windows jq to match bit for bit, so those targets use the `libm`
/// crate for this group, as a `no_std` build does.
macro_rules! posix_libm_fns {
    ($( $name:ident ( $($arg:ident),+ ) ; )+) => {
        $(
            #[inline]
            pub(crate) fn $name($($arg: f64),+) -> f64 {
                #[cfg(all(feature = "std", not(target_os = "windows")))]
                {
                    extern "C" {
                        fn $name($($arg: f64),+) -> f64;
                    }
                    // SAFETY: as for `libm_fns!`.
                    unsafe { $name($($arg),+) }
                }
                #[cfg(any(not(feature = "std"), target_os = "windows"))]
                {
                    libm::$name($($arg),+)
                }
            }
        )+
    };
}

posix_libm_fns! {
    j0(x);
    j1(x);
    y0(x);
    y1(x);
}

/// The three functions a plain `extern "C"` declaration does **not** reach
/// the platform libm for on `linux-gnu`. `compiler_builtins` exports its own
/// weak `cbrt`, `fmax` and `fmin` (alongside `sqrt`/`floor`/`fma`/... which
/// are IEEE-exact and so indistinguishable), and the linker binds the
/// declaration to those before glibc's: `cbrt` came out 192/400 off the
/// `jq-linux-amd64` binary while every other symbol resolved to glibc, and
/// `fmax(-0.0; 0)` printed `-0` where glibc's `maxsd`-based `fmax` returns
/// its second operand on a zero tie (`0`). `dlsym(RTLD_NEXT, name)` skips
/// the executable's own copy and finds `libm.so`'s; each lookup runs once.
/// Apple's linker has no such shadow, and a musl build *wants* musl's
/// algorithms, which is what `compiler_builtins` carries.
#[cfg(all(feature = "std", target_os = "linux", target_env = "gnu"))]
mod glibc_next {
    use core::ffi::{c_char, c_void};

    extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    /// `dlsym(RTLD_NEXT, name)`; `name` must be NUL-terminated.
    pub(super) fn lookup(name: &[u8]) -> *mut c_void {
        debug_assert_eq!(name.last(), Some(&0));
        // RTLD_NEXT is `(void *) -1` in glibc's <dlfcn.h>.
        let next = usize::MAX as *mut c_void;
        // SAFETY: `dlsym` with a NUL-terminated name and glibc's own
        // pseudo-handle; it only reads the string.
        unsafe { dlsym(next, name.as_ptr().cast::<c_char>()) }
    }
}

macro_rules! glibc_next_fns {
    ($( $name:ident ( $($arg:ident),+ ) ; )+) => {
        $(
            #[inline]
            pub(crate) fn $name($($arg: f64),+) -> f64 {
                #[cfg(all(feature = "std", target_os = "linux", target_env = "gnu"))]
                {
                    use std::sync::OnceLock;
                    type F = unsafe extern "C" fn($($arg: f64),+) -> f64;
                    static NEXT: OnceLock<Option<F>> = OnceLock::new();
                    let f = NEXT.get_or_init(|| {
                        let p = glibc_next::lookup(concat!(stringify!($name), "\0").as_bytes());
                        if p.is_null() {
                            None
                        } else {
                            // SAFETY: the symbol of this name in a C libm has
                            // exactly this signature.
                            Some(unsafe { core::mem::transmute::<*mut core::ffi::c_void, F>(p) })
                        }
                    });
                    extern "C" {
                        fn $name($($arg: f64),+) -> f64;
                    }
                    // SAFETY: as for `libm_fns!`, whichever copy was found.
                    match f {
                        Some(f) => unsafe { f($($arg),+) },
                        None => unsafe { $name($($arg),+) },
                    }
                }
                #[cfg(all(feature = "std", not(all(target_os = "linux", target_env = "gnu"))))]
                {
                    extern "C" {
                        fn $name($($arg: f64),+) -> f64;
                    }
                    // SAFETY: as for `libm_fns!`.
                    unsafe { $name($($arg),+) }
                }
                #[cfg(not(feature = "std"))]
                {
                    libm::$name($($arg),+)
                }
            }
        )+
    };
}

glibc_next_fns! {
    cbrt(x);
    fmax(x, y);
    fmin(x, y);
}

/// The C `(int)` conversion of a double, as the platform's jq performs it
/// for `ldexp`/`jn`/`yn`'s integer argument. Out-of-range, infinite and NaN
/// values are undefined behaviour in C and the two architectures answer
/// differently: x86-64's `cvttsd2si` yields `INT_MIN` for all of them, while
/// AArch64's `fcvtzs` saturates and maps NaN to 0. Real jq inherits whichever
/// its host does (`ldexp(3; infinite)` is `1.7976931348623157e+308` from
/// `/usr/bin/jq` on Apple silicon and `0` from `jq-linux-amd64`), so this
/// reproduces each, keyed on the architecture rather than the OS.
#[inline]
fn c_int(x: f64) -> core::ffi::c_int {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if x.is_nan() || x >= 2_147_483_648.0 || x < -2_147_483_648.0 {
            core::ffi::c_int::MIN
        } else {
            x as core::ffi::c_int
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    {
        x as core::ffi::c_int
    }
}

/// [`c_int`]'s `(long)` sibling, for `scalbln`. `long` is 64 bits on every
/// LP64 target and 32 on Windows; `c_long` tracks that.
#[inline]
fn c_long(x: f64) -> core::ffi::c_long {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        // `c_long::MIN as f64` is exact (a power of two), and `-MIN` is
        // one past `MAX`, so the half-open range below is the exact set of
        // doubles that convert without overflow.
        let lo = core::ffi::c_long::MIN as f64;
        if x.is_nan() || x >= -lo || x < lo {
            core::ffi::c_long::MIN
        } else {
            x as core::ffi::c_long
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    {
        x as core::ffi::c_long
    }
}

#[inline]
pub(crate) fn ldexp(x: f64, n: f64) -> f64 {
    #[cfg(feature = "std")]
    {
        extern "C" {
            fn ldexp(x: f64, n: core::ffi::c_int) -> f64;
        }
        // SAFETY: as for `libm_fns!`.
        unsafe { ldexp(x, c_int(n)) }
    }
    #[cfg(not(feature = "std"))]
    {
        libm::ldexp(x, c_int(n))
    }
}

#[inline]
pub(crate) fn scalbln(x: f64, n: f64) -> f64 {
    #[cfg(feature = "std")]
    {
        extern "C" {
            fn scalbln(x: f64, n: core::ffi::c_long) -> f64;
        }
        // SAFETY: as for `libm_fns!`.
        unsafe { scalbln(x, c_long(n)) }
    }
    #[cfg(not(feature = "std"))]
    {
        // `scalbn` saturates internally past the exponent range, so
        // clamping the `long` into an `int` loses nothing.
        let n = c_long(n).clamp(i32::MIN as core::ffi::c_long, i32::MAX as core::ffi::c_long);
        libm::scalbn(x, n as i32)
    }
}

/// `scalb(x, y)` takes its exponent as a *double*, and the two platform
/// libms disagree on a non-integral one: Apple's truncates (`scalb(3; 2.9)`
/// is `12`), glibc's returns NaN, and real jq prints each. The C symbol is
/// bound directly so succinctly prints the same as the jq beside it.
#[inline]
pub(crate) fn scalb(x: f64, y: f64) -> f64 {
    #[cfg(all(feature = "std", not(target_os = "windows")))]
    {
        extern "C" {
            fn scalb(x: f64, y: f64) -> f64;
        }
        // SAFETY: as for `libm_fns!`.
        unsafe { scalb(x, y) }
    }
    #[cfg(any(not(feature = "std"), target_os = "windows"))]
    {
        if y.is_nan() {
            return f64::NAN;
        }
        if y.is_infinite() {
            return if y > 0.0 { x * f64::INFINITY } else { x * 0.0 };
        }
        libm::scalbn(x, c_int(y))
    }
}

#[inline]
pub(crate) fn jn(n: f64, x: f64) -> f64 {
    #[cfg(all(feature = "std", not(target_os = "windows")))]
    {
        extern "C" {
            fn jn(n: core::ffi::c_int, x: f64) -> f64;
        }
        // SAFETY: as for `libm_fns!`.
        unsafe { jn(c_int(n), x) }
    }
    #[cfg(any(not(feature = "std"), target_os = "windows"))]
    {
        libm::jn(c_int(n), x)
    }
}

#[inline]
pub(crate) fn yn(n: f64, x: f64) -> f64 {
    #[cfg(all(feature = "std", not(target_os = "windows")))]
    {
        extern "C" {
            fn yn(n: core::ffi::c_int, x: f64) -> f64;
        }
        // SAFETY: as for `libm_fns!`.
        unsafe { yn(c_int(n), x) }
    }
    #[cfg(any(not(feature = "std"), target_os = "windows"))]
    {
        libm::yn(c_int(n), x)
    }
}

/// `frexp`: the mantissa in `[0.5, 1)` and the binary exponent.
#[inline]
pub(crate) fn frexp(x: f64) -> (f64, i32) {
    #[cfg(feature = "std")]
    {
        extern "C" {
            fn frexp(x: f64, exp: *mut core::ffi::c_int) -> f64;
        }
        let mut e: core::ffi::c_int = 0;
        // SAFETY: `exp` points at a live, writable `c_int` for the call.
        let m = unsafe { frexp(x, &mut e) };
        (m, e)
    }
    #[cfg(not(feature = "std"))]
    {
        libm::frexp(x)
    }
}

/// `modf`: the fractional and integral parts, both carrying `x`'s sign.
#[inline]
pub(crate) fn modf(x: f64) -> (f64, f64) {
    #[cfg(feature = "std")]
    {
        extern "C" {
            fn modf(x: f64, iptr: *mut f64) -> f64;
        }
        let mut i = 0.0_f64;
        // SAFETY: `iptr` points at a live, writable `f64` for the call.
        let f = unsafe { modf(x, &mut i) };
        (f, i)
    }
    #[cfg(not(feature = "std"))]
    {
        libm::modf(x)
    }
}

/// `lgamma_r`: `lgamma` plus the sign of `gamma(x)`.
#[inline]
pub(crate) fn lgamma_r(x: f64) -> (f64, i32) {
    #[cfg(all(feature = "std", not(target_os = "windows")))]
    {
        extern "C" {
            fn lgamma_r(x: f64, sign: *mut core::ffi::c_int) -> f64;
        }
        let mut s: core::ffi::c_int = 0;
        // SAFETY: `sign` points at a live, writable `c_int` for the call.
        let v = unsafe { lgamma_r(x, &mut s) };
        (v, s)
    }
    #[cfg(any(not(feature = "std"), target_os = "windows"))]
    {
        libm::lgamma_r(x)
    }
}

/// jq's `gamma` is whatever the platform's `gamma` is, and the platforms
/// disagree: jq's `builtin.c` defines `gamma` as `tgamma` on Apple (whose
/// own `gamma` is deprecated), while glibc's `gamma` is `lgamma` -- so
/// `5 | gamma` is `24` from `/usr/bin/jq` and `3.178...` from
/// `jq-linux-amd64`, both captured live.
#[inline]
pub(crate) fn gamma(x: f64) -> f64 {
    #[cfg(target_vendor = "apple")]
    {
        tgamma(x)
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        lgamma(x)
    }
}

/// `logb`: the unbiased binary exponent as a double, with C99's special
/// cases (`±0` -> `-inf`, `±inf` -> `+inf`, NaN -> NaN; a subnormal reports
/// its true exponent, `1e-310 | logb` is `-1030`).
#[inline]
pub(crate) fn logb(x: f64) -> f64 {
    #[cfg(feature = "std")]
    {
        extern "C" {
            fn logb(x: f64) -> f64;
        }
        // SAFETY: as for `libm_fns!`.
        unsafe { logb(x) }
    }
    #[cfg(not(feature = "std"))]
    {
        if x == 0.0 {
            f64::NEG_INFINITY
        } else if x.is_infinite() {
            f64::INFINITY
        } else if x.is_nan() {
            x
        } else {
            f64::from(libm::ilogb(x))
        }
    }
}

/// `significand`: `x` scaled into `[1, 2)`, passing `±0`, `±inf` and NaN
/// through. jq's own `builtin.c` spells it `2 * frexp(x)` on Apple, whose
/// libm lacks the symbol, and that is what every platform's `significand`
/// computes too.
#[inline]
pub(crate) fn significand(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    2.0 * frexp(x).0
}

/// `nearbyint` rounds to the nearest integer in the current rounding mode
/// without raising the inexact flag; jq exposes it beside `rint`, and with
/// the default rounding mode and no flag observable through jq the two are
/// the same function.
#[inline]
pub(crate) fn nearbyint(x: f64) -> f64 {
    rint(x)
}

/// jq's `exp10` is whatever the platform calls it: `__exp10` on Apple
/// platforms (jq's `builtin.c` renames it), `exp10` on glibc and musl. On a
/// platform with neither -- Android's bionic exports none, nor do the BSDs
/// or MSVC -- jq's own build defines `exp10/0` as a "not found at build time"
/// error; succinctly falls back to `pow(10, x)` there instead, which is not
/// bit-exact against any jq but is the closest value on offer.
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
        target_os = "linux",
        any(target_env = "gnu", target_env = "musl")
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
        not(all(target_os = "linux", any(target_env = "gnu", target_env = "musl")))
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
    /// Gated to those platforms (a musl-libc build prints musl's digits,
    /// which are the crate's, and fails 5 of these rows by design); the
    /// hermetic golden case `math_platform_libm_bits` covers the remaining
    /// CI legs.
    #[cfg(all(
        feature = "std",
        any(
            target_os = "macos",
            all(target_os = "linux", target_env = "gnu", target_arch = "x86_64")
        )
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

    /// #3042's family where the platform libms disagree with each other, so
    /// the hermetic golden cannot pin them and only a per-platform row can:
    /// each value below was captured from the platform's own jq 1.7.1
    /// (`/usr/bin/jq` on macOS; `jq-linux-amd64` on glibc 2.35 and Ubuntu
    /// 24.04's `jq` on glibc 2.39 agree on every row here). The two
    /// `linux-gnu`-only rows are exactly the `compiler_builtins` shadow
    /// `glibc_next_fns!` exists for: `cbrt(27)` is `3` from the shadowing
    /// copy and `3.0000000000000004` from glibc.
    #[cfg(all(
        feature = "std",
        any(
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "linux", target_env = "gnu", target_arch = "x86_64")
        )
    ))]
    #[test]
    fn platform_libm_family_matches_the_platform_jq_bit_for_bit() {
        // The two `ldexp` rows are the AArch64 `fcvtzs` results; an Intel
        // Mac's jq would print the x86 column's `0`s, so the macOS gate is
        // narrowed to the architecture these were captured on.
        #[cfg(target_os = "macos")]
        let rows: &[(&str, f64, f64)] = &[
            ("cbrt", cbrt(27.0), 3.0),
            ("lgamma", lgamma(5.0), 3.178_053_830_347_945_3),
            ("gamma", gamma(5.0), 24.0),
            ("erf", erf(1.0), 0.842_700_792_949_714_8),
            ("y0", y0(1.0), 0.088_256_964_215_676_97),
            ("scalb", scalb(3.0, 2.9), 12.0),
            ("ldexp", ldexp(3.0, f64::INFINITY), f64::INFINITY),
            ("ldexp", ldexp(3.0, f64::NAN), 3.0),
            ("lgamma_r", lgamma_r(0.0).1.into(), 0.0),
        ];
        #[cfg(target_os = "linux")]
        let rows: &[(&str, f64, f64)] = &[
            ("cbrt", cbrt(27.0), 3.000_000_000_000_000_4),
            ("lgamma", lgamma(5.0), 3.178_053_830_347_945_8),
            ("gamma", gamma(5.0), 3.178_053_830_347_945_8),
            ("erf", erf(1.0), 0.842_700_792_949_714_9),
            ("y0", y0(1.0), 0.088_256_964_215_676_98),
            ("scalb", scalb(3.0, 2.9), f64::NAN),
            ("ldexp", ldexp(3.0, f64::INFINITY), 0.0),
            ("ldexp", ldexp(3.0, f64::NAN), 0.0),
            ("lgamma_r", lgamma_r(0.0).1.into(), 1.0),
        ];
        for (name, got, want) in rows {
            assert!(
                got.to_bits() == want.to_bits() || (got.is_nan() && want.is_nan()),
                "{name}: got {got:?}, the platform's jq 1.7.1 prints {want:?}"
            );
        }
    }

    /// The C `(int)` conversion the exponent arguments go through: the
    /// architectures differ on out-of-range input and real jq inherits it.
    #[test]
    fn c_int_conversion_matches_the_architecture() {
        assert_eq!(c_int(2.9), 2);
        assert_eq!(c_int(-2.9), -2);
        assert_eq!(c_int(2_147_483_647.0), i32::MAX);
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            assert_eq!(c_int(f64::NAN), i32::MIN);
            assert_eq!(c_int(f64::INFINITY), i32::MIN);
            assert_eq!(c_int(1e10), i32::MIN);
            assert_eq!(c_long(f64::NAN), core::ffi::c_long::MIN);
            assert_eq!(c_long(1e300), core::ffi::c_long::MIN);
        }
        #[cfg(target_arch = "aarch64")]
        {
            assert_eq!(c_int(f64::NAN), 0);
            assert_eq!(c_int(f64::INFINITY), i32::MAX);
            assert_eq!(c_int(1e10), i32::MAX);
            assert_eq!(c_long(f64::NAN), 0);
            assert_eq!(c_long(1e300), core::ffi::c_long::MAX);
        }
    }
}
