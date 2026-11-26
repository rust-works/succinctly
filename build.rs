//! Builds the vendored Oniguruma (`vendor/oniguruma`, the commit jq 1.7.1 bundles) when the
//! `onig-engine` feature is on (#7965, ADR-0031). Every other build runs this script and does
//! nothing.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(feature = "onig-engine")]
    oniguruma::build();
}

#[cfg(feature = "onig-engine")]
mod oniguruma {
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    /// The translation units UTF-8 and the Perl_NG syntax need. `unicode.c` includes the
    /// `unicode_*_data.c` tables itself.
    const SOURCES: &[&str] = &[
        "regparse.c",
        "regcomp.c",
        "regexec.c",
        "regenc.c",
        "regerror.c",
        "regsyntax.c",
        "regtrav.c",
        "regversion.c",
        "st.c",
        "utf8.c",
        "ascii.c",
        "unicode.c",
        "unicode_unfold_key.c",
        "unicode_fold1_key.c",
        "unicode_fold2_key.c",
        "unicode_fold3_key.c",
    ];

    pub fn build() {
        let root =
            PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
        let src = root.join("vendor/oniguruma/src");
        let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
        let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
        let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
        let bits = env::var("CARGO_CFG_TARGET_POINTER_WIDTH").unwrap_or_else(|_| "64".into());
        println!("cargo:rerun-if-changed=vendor/oniguruma/src");

        let mut cc = cc::Build::new();
        let config_h = out.join("config.h");
        if os == "windows" {
            fs::copy(src.join(format!("config.h.win{bits}")), &config_h)
                .expect("copy config.h.win{32,64}");
        } else {
            // What `configure` would find on a Unix-like target. `alloca.h` is missing on musl
            // and the BSDs, where the compiler provides `alloca` itself.
            cc.define("HAVE_UNISTD_H", "1")
                .define("HAVE_SYS_TYPES_H", "1")
                .define("HAVE_SYS_TIME_H", "1");
            let bsd = matches!(os.as_str(), "freebsd" | "openbsd" | "netbsd" | "dragonfly");
            if target_env != "musl" && !bsd {
                cc.define("HAVE_ALLOCA_H", "1");
            }
            let long = if bits == "64" { "8" } else { "4" };
            let ptr = if bits == "64" { "8" } else { "4" };
            fs::write(
                &config_h,
                format!(
                    "#define HAVE_PROTOTYPES 1\n#define STDC_HEADERS 1\n#define HAVE_STRING_H 1\n\
                     #define HAVE_STDARG_H 1\n#define HAVE_STDLIB_H 1\n#define HAVE_LIMITS_H 1\n\
                     #define HAVE_INTTYPES_H 1\n#define SIZEOF_INT 4\n#define SIZEOF_SHORT 2\n\
                     #define SIZEOF_LONG {long}\n#define SIZEOF_VOIDP {ptr}\n#define SIZEOF_LONG_LONG 8\n"
                ),
            )
            .expect("write config.h");
        }
        // wasm32 cannot take the address of a label, which direct threading needs.
        if env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
            cc.define("ONIG_DISABLE_DIRECT_THREADING", "1");
        }
        // A static library: `ONIG_EXTERN` is a plain `extern`, not `__declspec(dllimport)`.
        cc.define("ONIG_STATIC", "1");
        // The pinned sources declare functions without prototypes, which C23 (GCC 15's default)
        // rejects; gnu17 compiles them unchanged.
        if !cc.get_compiler().is_like_msvc() {
            cc.flag("-std=gnu17");
        }
        cc.include(&out).include(&src).warnings(false);
        for file in SOURCES {
            cc.file(src.join(file));
        }
        cc.compile("onig");
    }
}
