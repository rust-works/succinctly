#!/usr/bin/env python3
"""Randomised differential sweep for `--seq`'s parse-error diagnostics (#1723).

Generates RFC 7464 streams, runs each through both the pinned jq oracle and
the built succinctly binary, and compares **stderr, stdout and exit code
together**. Comparing stderr alone would miss the failure mode that actually
matters: `seq_record_scan` dropping a record the warning model considers
clean, which loses a value on stdout with no diagnostic to explain it.

This is a verification tool, not a CI gate -- the pinned rows in
`tests/jq_cli_tests.rs` and the unit tests in
`src/bin/succinctly/jq_seq_reader.rs` are what CI enforces. Run it after
touching `--seq` parsing or the warning model.

**Why the corpus is shaped the way it is.** Three earlier attempts at this
area shipped bugs their own verification missed, every time because the
corpus was biased in exactly the dimension under test (all whitespace-
separated, so adjacency went untested; single-record only, so record
boundaries went untested). So the generator is random rather than
hand-picked, and it is structured to make jq's *line-reader* artifacts
unreachable rather than filtering them out afterwards:

* newline-terminated multi-record streams -- what real RFC 7464 looks like;
* single-record streams with no trailing newline -- these reach real EOF
  mid-value, which is what exercises the `at EOF` templates, and a lone
  record has no interior RS byte for the stream to end early on.

Both artifacts are properties of jq's 4096-byte `fgets` buffer, not of its
parser, and neither survives into a newline-terminated stream:

1. A trailing unterminated RS-record after an earlier newline is dropped
   unread below the buffer boundary and parsed above it.
2. A record yielding no value makes `jv_parser_next` return
   invalid-with-no-message, which jq's input loop reads as end-of-input --
   but only when it lands in the buffer that hit EOF.
3. `jq_util_input_read_more` measures the chunk with `strlen`, so a NUL
   byte truncates the input -- but only in a chunk holding no newline.

`--expect-artifacts` re-enables the shapes that trip them, for confirming
they are still the only divergences.

Expected result: `0 stderr mismatches, 0 stdout supersets`; exits non-zero
otherwise.

Usage:
  cargo build --release --features cli
  ./scripts/jq-seq-oracle-sweep.py [--seed N] [--cases N] [path-to-binary]
"""

import argparse
import os
import pathlib
import random
import re
import subprocess
import sys
import tempfile

RS = b"\x1e"
REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent

# Structural bytes, both quote-ish characters, digits, the letters that start
# jq's keywords and decNumber's special forms, and the number punctuation --
# small enough that random strings collide into real JSON shapes often.
ALPHABET = b'\x1e{}[]",:0-9abfilnrstuxe \n\\.-+'

# BOM prefixes are their own dimension: jq consumes the matching prefix of
# one *without counting it*, and a prefix that starts a BOM and then
# contradicts it costs jq a parser_reset. A review found both, precisely
# because an earlier version of this generator had no 0xef in its alphabet.
BOM_PREFIXES = [b"", b"", b"", b"", b"\xef\xbb\xbf", b"\xef\xbb", b"\xef"]

# Shapes worth keeping regardless of what the generator happens to produce:
# every template the issue named, plus the ones earlier sessions got wrong.
HAND = [
    b'\x1e"abc', b'\x1e{"a":1', b"\x1e[1,2\n", b"\x1etru", b"\x1e-", b"\x1e1.",
    b"\x1e1e", b"\x1exyz\n", b"\x1e{}extra", b"\x1e[1,2]extra", b"\x1e[1,]",
    b"\x1e}", b"\x1e]", b'\x1e"unterminated\x1e"ok"\n', b'\x1e[1,2\x1e"ok"\n',
    b'\x1exyz\x1e"ok"\n', b'\x1etru\x1e"ok"\n', b'\x1e1.\x1e"ok"\n',
    b'\x1e{}extra\x1e"ok"\n', b'\x1e[1,]\x1e"ok"\n', b'\x1e}\x1e"ok"\n',
    b"\x1e{,}\n", b"\x1ea:b\n", b"\x1enot valid json\n", b"\x1e{1:2}\n",
    b"\x1e} 5\n", b"\x1e1 } 2\n", b'\x1e} {"a":1}\n', b"\x1e1,2\n",
    b"\x1e1 {invalid\n", b"\x1e1 2 xyz\n", b"\x1e\xc3\xa9]", b"\x1e\xff]",
    b"\x1e1\nJUNK", b'\x1e"a" 2', b"\x1etrue\x1e3", b"\x1e1 2", b"\x1e1 2 ",
    b'\x1e"\\q"', b'\x1e"\\u00"', b'\x1e"\\uZZZZ"', b'\x1e"\\ud800"',
    b'\x1e"\\ud800\\u0041"', b'\x1e"a\x01b"', b'\x1e{"a", "b"}', b"\x1e[1 2]",
    b"\x1e:", b"\x1e,", b"\x1enan", b"\x1einf", b"\x1esnan", b"\x1enul",
    b"\x1en", b"\x1e+1", b"\x1e[[1],]", b'\x1e{"a":1,}', b"\xef\xbb\xbf\x1e}", b"\xef\xbb\x1e[1,]\n", b"\xef\xbb1 2",
    b"\xef1 2", b"\xef", b"\xef\xbb",
    b'\x1e"a"\x1e"b"', b"\x1e[1,2]}\n", b"\x1e{}}\n", b"\x1etrue,[1]",
    b"\x1e5-3 7", b"\x1e1-2\n",
]


def resolve_oracle():
    pin = (REPO_ROOT / "tests/data/jq-golden/JQ_VERSION").read_text().strip()
    # Prefer the pinned oracle directly (macOS ships jq-1.7.1 at /usr/bin/jq;
    # a PATH jq, e.g. Homebrew's, is often newer) -- same reasoning as
    # tests/jq_cli_tests.rs's own pinned-oracle comments.
    for candidate in ("/usr/bin/jq", None):
        path = candidate or shutil_which_jq()
        if path and os.access(path, os.X_OK):
            version = subprocess.run(
                [path, "--version"], capture_output=True, text=True
            ).stdout.strip()
            if version.startswith(pin):
                return path, version
    sys.exit(f"error: no jq matching pin {pin} found at /usr/bin/jq or on PATH")


def shutil_which_jq():
    import shutil

    return shutil.which("jq")


def run(argv, data):
    # stdin from a file, never a pipe: with `pipefail` a SIGPIPE'd writer
    # fabricates exit-code divergences (scripts/jq-fanout-oracle-sweep.sh
    # learned this the hard way).
    with tempfile.NamedTemporaryFile(delete=False) as handle:
        handle.write(data)
        path = handle.name
    try:
        with open(path, "rb") as stdin:
            done = subprocess.run(
                argv + ["--seq", "-c", "."],
                stdin=stdin,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
        return done.stdout, done.stderr, done.returncode
    finally:
        os.unlink(path)


def malformed_bom(data):
    """A prefix that starts a BOM and then contradicts it. jq consumes the
    matched bytes and thereafter re-runs `parser_reset` at the top of every
    `jv_parser_next`, which leaves the parser reading rather than waiting
    for an RS byte."""
    return data[:1] == b"\xef" and not data.startswith(b"\xef\xbb\xbf")


def stops_early(data):
    """Whether jq's input loop ends the stream before reading it all.

    An RS closing a record that yields no value makes `jv_parser_next`
    return invalid-with-no-message, which the loop reads as end-of-input --
    but only in the call that performed the final read. With no newline the
    whole input is one buffer, so that reduces to "the first record yields
    nothing".

    After a malformed BOM the first record is the content *before* the
    first RS, since the parser is no longer waiting for one -- which is why
    `\xef\x1e...` stops immediately where `\x1e...` would not."""
    if b"\n" in data or RS not in data:
        return False
    if malformed_bom(data):
        consumed = 2 if data.startswith(b"\xef\xbb") else 1
        first = data[consumed:].split(RS)[0]
    elif data.count(RS) < 2:
        return False
    else:
        first = data.split(RS)[1]
    return first.strip(b" \t\r") in (b"", b'"')


def artifact(data):
    if b"\x00" in data and b"\n" not in data.split(b"\x00", 1)[0]:
        return True  # strlen truncation, artifact 3
    if RS not in data:
        # #1525's separate template -- except after a malformed BOM, which
        # makes jq read the stream rather than abandon it, so those stay in.
        return not malformed_bom(data)
    if stops_early(data):
        return True
    if b"\n" not in data:
        return False
    return not data.rsplit(RS, 1)[1].endswith(b"\n") and len(data) < 4096


def random_cases(count, seed, allow_artifacts):
    rng = random.Random(seed)
    plain = bytes(b for b in ALPHABET if b not in (RS[0], 0x0A))
    cases = []
    for _ in range(count):
        if allow_artifacts:
            body = bytes(rng.choice(ALPHABET) for _ in range(rng.randint(1, 24)))
            cases.append(body)
        elif rng.random() < 0.7:
            body = bytes(rng.choice(ALPHABET) for _ in range(rng.randint(1, 24)))
            cases.append(rng.choice(BOM_PREFIXES) + RS + body + b"\n")
        else:
            body = bytes(rng.choice(plain) for _ in range(rng.randint(1, 24)))
            cases.append(rng.choice(BOM_PREFIXES) + RS + body)
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", nargs="?", default=str(REPO_ROOT / "target/release/succinctly"))
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--cases", type=int, default=4000)
    parser.add_argument(
        "--expect-artifacts",
        action="store_true",
        help="include the line-reader-artifact shapes, to confirm they are still the only divergences",
    )
    args = parser.parse_args()

    if not os.access(args.binary, os.X_OK):
        sys.exit(
            f"error: succinctly binary not found at {args.binary} — "
            "run: cargo build --release --features cli"
        )
    jq, version = resolve_oracle()
    print(f"oracle: {jq} ({version}), succinctly: {args.binary}", file=sys.stderr)

    cases = HAND + random_cases(args.cases, args.seed, args.expect_artifacts)
    stderr_bad, stdout_super, skipped = [], [], 0
    for data in cases:
        if not args.expect_artifacts and artifact(data):
            skipped += 1
            continue
        jq_out, jq_err, jq_code = run([jq], data)
        our_out, our_err, our_code = run([args.binary, "jq"], data)
        if (jq_err, jq_code) != (our_err, our_code):
            stderr_bad.append((data, jq_err, our_err))
        # stdout may be a strict subset of jq's -- succinctly deliberately
        # drops a record jq can partially read -- but never a superset.
        jq_values = jq_out.split(RS)
        if any(v and v not in jq_values for v in our_out.split(RS)):
            stdout_super.append((data, jq_out, our_out))

    checked = len(cases) - skipped
    print(
        f"checked={checked} artifact-shapes-skipped={skipped} "
        f"stderr_mismatch={len(stderr_bad)} stdout_superset={len(stdout_super)}"
    )
    for data, expected, got in stderr_bad[:20]:
        print(f"\n  input : {data!r}")
        print(f"  jq    : {expected.decode(errors='replace').strip()!r}")
        print(f"  succ  : {got.decode(errors='replace').strip()!r}")
    for data, expected, got in stdout_super[:10]:
        print(f"\n  SUPERSET input: {data!r}\n    jq  : {expected!r}\n    succ: {got!r}")
    return 1 if stderr_bad or stdout_super else 0


if __name__ == "__main__":
    sys.exit(main())
