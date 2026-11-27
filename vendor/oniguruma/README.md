# Oniguruma, as jq 1.7.1 bundles it

These are the C sources of [Oniguruma](https://github.com/kkos/oniguruma) at commit
`d2f1a14ced5d5d461acac0da0d477ab240a7ab5f` (2023-07-11, the merge of upstream PR #287).
That is the commit jq 1.7.1's `modules/oniguruma` submodule pins: 18 commits before the 6.9.9 release, with Unicode 15.0 tables, and `onig_version()` still says 6.9.8.
ADR-0031 (`docs/adrs/adr-0031.md`) records why jq mode compiles and matches with this exact commit (#7965).

- **Licence:** BSD-2-Clause, in [COPYING](COPYING).
- **Subset:** only what UTF-8 and the Perl_NG syntax need: the parser, compiler, matcher, error text, syntax tables, the UTF-8 and ASCII encodings, and the Unicode data that `unicode.c` includes.
  The other encodings, the POSIX and GNU APIs, the build system and the test suite are left out.
  `config.h` is written by `build.rs` on Unix-like targets; `config.h.win32` and `config.h.win64` are upstream's, for Windows.
- **Local changes:** none. `SHA256SUMS` lists every file; `shasum -a 256 -c SHA256SUMS` in this directory checks them.
- **Upstream** ended on 2025-04-24 (the repository is archived). No advisory feed will report a fix, so this copy is ours to keep.
  A change to it is a change to jq-mode answers, and has to pass the regex corpus gate (`scripts/jq-regex-corpus.py`, #7965).
- **Build:** `build.rs` compiles these files with the `cc` crate whenever the `regex` feature is on (so in every `cli` build), with `-std=gnu17` on non-MSVC targets.
  The pinned sources do not compile as C23, which GCC 15 uses by default.

To re-vendor (only with a decision to move off the pin):

```bash
gh api repos/kkos/oniguruma/tarball/<commit> > onig.tgz && tar xzf onig.tgz
# copy the files listed in SHA256SUMS from <dir>/src (and COPYING), then:
shasum -a 256 COPYING src/* > SHA256SUMS
```
