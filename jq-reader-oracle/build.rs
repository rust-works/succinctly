//! Compiles jq's own input reader twice, once from each vendored release (`vendor/jq-1.7.1/`,
//! `vendor/jq-1.8.2/`), and links both into the oracle.
//!
//! Each build is the subset of jq's sources that `jq_util_input_*` and `jv_parse.c` need: the
//! `jv` value library, the JSON parser and printer, `util.c` and decNumber. It has no bytecode
//! interpreter, no lexer or parser of the jq language and no oniguruma, so neither bison, flex nor
//! autoreconf is needed. The pointer-only shim (`src/shim.c`) is compiled into each build.
//!
//! Both releases define the same C symbols (`jv_parse`, `jq_util_input_next_input`, ...). Every
//! translation unit of a build is therefore compiled with `-include vendor/rename-<release>.h`,
//! which `#define`s each global symbol of that release to a `sjq171_` or `sjq182_` name.
//! `scripts/gen-rename.py` writes those headers, and the `every_global_symbol_is_renamed`
//! test fails when a symbol is missing from one.

use std::path::{Path, PathBuf};

/// The jq sources the reader needs, relative to the release tree's `src/`.
const SOURCES: &[&str] = &[
    "jv.c",
    "jv_alloc.c",
    "jv_aux.c",
    "jv_dtoa.c",
    "jv_dtoa_tsd.c",
    "jv_parse.c",
    "jv_print.c",
    "jv_unicode.c",
    "util.c",
];

const DECNUMBER: &[&str] = &["decContext.c", "decNumber.c"];

struct Release {
    /// The directory under `vendor/`.
    dir: &'static str,
    /// The symbol prefix the rename header and the shim use.
    prefix: &'static str,
    /// The directory holding `decNumber/`, relative to the release tree: jq 1.7.1 keeps it in
    /// `src/`, jq 1.8.2 in `vendor/`.
    decnumber_parent: &'static str,
    /// Whether the vendored `jv_parse.c` carries the sync-point patch (1.8.2 only).
    sync_points: bool,
}

const RELEASES: &[Release] = &[
    Release {
        dir: "jq-1.7.1",
        prefix: "sjq171_",
        decnumber_parent: "src",
        sync_points: false,
    },
    Release {
        dir: "jq-1.8.2",
        prefix: "sjq182_",
        decnumber_parent: "vendor",
        sync_points: true,
    },
];

fn main() {
    let manifest =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let unix = std::env::var("CARGO_CFG_TARGET_FAMILY")
        .unwrap_or_default()
        .split(',')
        .any(|f| f == "unix");
    if !unix {
        // jv.c includes <unistd.h> and <pthread.h>, util.h's MIN/MAX are GNU statement
        // expressions, and jv.c's `_MSC_VER` thread-local block does not compile. The oracle is a
        // development tool for the Linux and macOS boxes; it is not built for Windows.
        panic!(
            "jq-reader-oracle builds only for a unix target (jq's C sources need POSIX and GNU C)"
        );
    }
    if std::env::var("CARGO_CFG_TARGET_ENDIAN").as_deref() != Ok("little") {
        // `IEEE_8087` below tells jv_dtoa.c the target stores doubles little-endian.
        panic!("jq-reader-oracle builds only for a little-endian target (jv_dtoa.c is built with IEEE_8087)");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/shim.c");
    // Every vendored file, including the headers the sources include.
    println!("cargo:rerun-if-changed=vendor");
    for release in RELEASES {
        build(&manifest, &target_os, release);
    }
    println!("cargo:rustc-link-lib=m");
}

fn build(manifest: &Path, target_os: &str, release: &Release) {
    let root = manifest.join("vendor").join(release.dir);
    let src = root.join("src");
    let dec_parent = root.join(release.decnumber_parent);
    let rename = manifest.join(format!("vendor/rename-{}.h", &release.dir[3..]));
    println!("cargo:rerun-if-changed={}", rename.display());

    let mut b = cc::Build::new();
    // A directory per release: both compile src/shim.c, whose object would otherwise be shared.
    let out =
        PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join(release.dir);
    b.out_dir(&out);
    b.include(&src)
        .include(&dec_parent)
        .warnings(false)
        .opt_level(2)
        .flag("-include")
        .flag(rename.to_str().expect("a UTF-8 path"))
        // What jq's configure defines on a little-endian POSIX system.
        .define("IEEE_8087", "1")
        .define("HAVE_MEMMEM", "1")
        .define("HAVE_STRPTIME", "1")
        .define("HAVE_PTHREAD", "1")
        .define("HAVE_PTHREAD_KEY_CREATE", "1")
        .define("HAVE_PTHREAD_ONCE", "1")
        .define("HAVE_ATEXIT", "1")
        .define("HAVE___THREAD", "1")
        .define("USE_DECNUM", "1")
        .define("SJQ_PREFIX", release.prefix);
    if !release.sync_points {
        b.define("SJQ_NO_SYNC", "1");
    }
    if target_os == "linux" {
        // glibc hides memmem/strptime/realpath without it, as jq's configure's
        // AC_USE_SYSTEM_EXTENSIONS does.
        b.define("_GNU_SOURCE", "1");
    }
    for s in SOURCES {
        let p = src.join(s);
        println!("cargo:rerun-if-changed={}", p.display());
        b.file(p);
    }
    for s in DECNUMBER {
        let p = dec_parent.join("decNumber").join(s);
        println!("cargo:rerun-if-changed={}", p.display());
        b.file(p);
    }
    b.file(manifest.join("src/shim.c"));
    b.compile(&format!("jqreader{}", &release.prefix[3..6]));
}
