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

A NUL byte is in the alphabet (#7340): jq's `jq_util_input_read_more` measures
an `fgets` chunk that holds no newline with `strlen`, so the chunk ends at its
first NUL, and `check_literal` hands a number token to its number parser as a C
string, so the number ends there too. The reader models both, and `--long`
generates the records that put a NUL in a full 4095-byte chunk, with a value
open across the bytes jq drops.

One artifact is not the line reader's but jq's own, and no implementation
can match it (#7256): `check_literal` reads `tokenbuf[1]` for a bare `n` without
consulting `tokenpos`, and when no earlier token of the stream has written index 1
that byte is uninitialised heap (`parser_init` leaves `tokenbuf` null and `token()`
grows it with `jv_mem_realloc`). Apple's jq and x86_64 glibc's happen to hold
something that is not `u` there, Amazon Linux 2023 on aarch64 holds `u`, so
`\x1en` reports "Invalid numeric literal" on the first two and "Invalid literal" on
the third. `artifact()` skips a stream whose first bare `n` is classified before
any token has written index 1, so the sweep keeps checking the case it can check,
a byte an earlier token left (#1723, #6513). `--expect-artifacts` includes it.

**History.** Two more artifacts of that same `fgets` buffer used to live
here -- a trailing unterminated RS-record after an earlier newline being
dropped unread below the buffer boundary and parsed above it, and a record
yielding no value silently ending the *whole* stream when it was the first
thing scanned from jq's own final buffer. Both are real jq's own
non-slurp end-of-stream rule (`jq_util_input_next_input`, `src/util.c`),
not two separate corner cases, and #2998 modeled it: the corpus no longer
excludes either shape.

Expected result: `0 stderr mismatches, 0 stdout supersets`; exits non-zero
otherwise. Any superset is a real failure. The one this corpus used to hit
(#3195, a substituted U+FFFD read as a malformed BOM when invalid UTF-8 opened
the document) is gone: values now come from the same raw-byte walk as the
diagnostics (#3247).

**`--slurp-location` mode** runs the same corpus through `--seq -s -c
'error("x")'` instead, so the `jq: error (at ...)` line carries jq's
EOF-location answer (#2947/#3003: `file:N` or `<unknown>`) and is compared
along with the warnings. stderr is compared as a *sorted* multiset of lines
there: on the #3003 shapes real jq prints the runtime error before its EOF
warning (the warning belongs to the `next_input` call *after* the one that
dispatched the slurped array), while succinctly prints every warning before it
evaluates -- a recorded divergence, not what this mode tests. Random cases are
1-24 bytes, so the one shape the reader still models approximately (a
malformed BOM whose refill lands on jq's 4095-byte `fgets` boundary rather
than on a newline) is unreachable here.

Usage:
  cargo build --release --features cli
  ./scripts/jq-seq-oracle-sweep.py [--seed N] [--cases N] [--slurp-location] [--oracle JQ] [path-to-binary]
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
ALPHABET = b'\x1e{}[]",:0-9abfilnrstuxe \n\\.-+\x00'

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
    # #3003: a value completed by the final buffer's last byte keeps `-s`'s
    # position (the first row of each pair) where the same shape completed
    # one byte earlier, or preceded by an error in the buffer, loses it.
    b"\x1e1{", b"\x1e1[", b"\x1etrue{", b"\x1e1 2{", b'\x1etrue"', b'\x1e1"',
    b'\x1e"a"{', b"\x1e{}{", b'\x1e"a"[', b"\x1e[0,]\x1e1{", b"\x1e1\x1e{",
    b"\x1e1\n\x1e2{", b"\x1e1}\n\x1e2{", b"\x1e1{ ", b"\x1e1{\n",
    # #7340: a NUL in a chunk with a newline ends a number; in one without, it ends the chunk.
    b'\x1e"ab"\n\x1e1\x00\n\x1e7 \n', b"\x1e[1\x00]\n", b"\x1e2.50\x00x\n", b"\x1enan\x00\n",
    b"\x1etrue\x00\n", b"\x1e\x00\n", b'\x1e"a\x00b"\n', b"\x1e12\x00\x1e3\n", b"\x1e12\x00\x1e3",
    b"ab\x00\x1e1 2", b"\x1e1 \x00 2", b"\xef\xbb\xbfab\x00cd", b"x\nab\x00cd\x1e1\nzz",
    b'\x1e"ab" \x00 7 [8] ' + b" " * 5000 + b"9 10\n",
    b"\x1e[1," + b" " * 4000 + b"\x00garbage" + b" " * 200 + b"2]\n",
    b'\x1e"x' + b"y" * 4093 + b"\x00zz" + b"w" * 10 + b'"\n',
    # A NUL jq never sees does not make a BOM malformed, and a BOM can complete across it.
    b"\xef\xbb\x00" + b" " * 4100 + b"\x1e1\n", b"\xef\xbb\x00" + b" " * 4092 + b"\xbf\x1e1\n",
    b"\xef\xbb\x00x\n\x1e1\n",
]


def resolve_oracle(given=None):
    pin = (REPO_ROOT / "tests/data/jq-golden/JQ_VERSION").read_text().strip()
    # Prefer the pinned oracle directly (macOS ships jq-1.7.1 at /usr/bin/jq;
    # a PATH jq, e.g. Homebrew's, is often newer) -- same reasoning as
    # tests/jq_cli_tests.rs's own pinned-oracle comments. `--oracle` (the
    # compare harness's pinned build, #5552) is the only one tried when given.
    if given:
        version = subprocess.run([given, "--version"], capture_output=True, text=True).stdout.strip()
        if not version.startswith(pin):
            print(f"error: --oracle {given} reports {version!r}, not the pinned {pin}", file=sys.stderr)
            sys.exit(2)
        return given, version
    for candidate in ("/usr/bin/jq", None):
        path = candidate or shutil_which_jq()
        if path and os.access(path, os.X_OK):
            version = subprocess.run(
                [path, "--version"], capture_output=True, text=True
            ).stdout.strip()
            if version.startswith(pin):
                return path, version
    print(f"error: no jq matching pin {pin} found at /usr/bin/jq or on PATH", file=sys.stderr)
    sys.exit(2)


def shutil_which_jq():
    import shutil

    return shutil.which("jq")


WARNINGS_ARGS = ["--seq", "-c", "."]
SLURP_LOCATION_ARGS = ["--seq", "-s", "-c", 'error("x")']


def run(argv, data, mode_args):
    # stdin from a file, never a pipe: with `pipefail` a SIGPIPE'd writer
    # fabricates exit-code divergences (scripts/jq-fanout-oracle-sweep.sh
    # learned this the hard way).
    with tempfile.NamedTemporaryFile(delete=False) as handle:
        handle.write(data)
        path = handle.name
    try:
        with open(path, "rb") as stdin:
            done = subprocess.run(
                argv + mode_args,
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


# Bytes that end a token outside a string: jq's `token()` is called for each of these.
_TOKEN_ENDS = b" \t\r\n\x1e[]{},:\""


def reads_unwritten_tokenbuf(data):
    """Whether jq 1.7.1 classifies a bare one-byte `n` token (`check_literal`'s
    `tokenbuf[1] == 'u'`) before anything in `data` has written `tokenbuf[1]`
    (#7256), so what it compares is uninitialised heap.

    Index 1 is written by a token of two or more bytes (`tokenadd`), by a one-byte
    token on the number path (`t`, `f` and `n` are the three that are not: jq
    NUL-terminates the token in place, at index 1), and by a string of two or more
    bytes, which `tokenadd`s its raw bytes whatever it decodes to (`"\\u0041"` leaves
    the `u`, #6513) and whether or not it is ever closed. An RS ends a token or a
    string wherever it stands: jq tests it before its parser state. Bytes before the
    first RS are not tokenised (jq is waiting for one) unless a malformed BOM has
    already put the parser back in `Normal`. A parse error does not make jq skip
    ahead: it resets the parser and reads the next byte (limitations.md, "need RS to
    resync" never resyncs).

    Where this cannot tell it says "not written", which skips more streams than the
    exposure needs and never fewer."""
    if malformed_bom(data):
        # jq consumed the part of the BOM that matched without tokenising it.
        start = 0
        while start < 3 and data[start : start + 1] == b"\xef\xbb\xbf"[start : start + 1]:
            start += 1
    else:
        start = data.find(RS)
        if start < 0:
            return False
    written = False
    i = start
    n = len(data)
    while i < n:
        byte = data[i]
        if byte == 0x22:  # a string: up to its closing quote, an RS, or the end
            j = i + 1
            while j < n and data[j] not in (0x22, RS[0]):
                j += 2 if data[j] == 0x5C and data[j + 1 : j + 2] != RS else 1
            if j - i - 1 >= 2:
                written = True
            i = j + 1 if data[j : j + 1] == b'"' else j
        elif byte in _TOKEN_ENDS:
            i += 1
        else:
            j = i
            while j < n and data[j] not in _TOKEN_ENDS:
                j += 1
            word = data[i:j]
            if len(word) >= 2:
                written = True
            elif word == b"n":
                if not written:
                    return True
                # written already; a bare `n` leaves its own NUL at index 1
            elif word not in (b"t", b"f"):
                written = True
            i = j
    return False


def visible(data):
    """The bytes of `data` jq's parser sees (#7340): each 4095-byte `fgets` chunk that holds no
    newline ends at its first NUL."""
    out, at = [], 0
    while at < len(data):
        newline = data.find(b"\n", at, at + 4095)
        end = newline + 1 if newline >= 0 else min(at + 4095, len(data))
        chunk = data[at:end]
        if newline < 0 and b"\x00" in chunk:
            chunk = chunk[: chunk.index(b"\x00")]
        out.append(chunk)
        at = end
    return b"".join(out)


def artifact(data):
    """Whether `data` trips an artifact of jq itself: the one `fgets`-line-reader
    artifact #2998 left unmodeled, or the uninitialised `tokenbuf` byte (#7256). jq's own end-of-stream rule (an RS closing a record
    that yields no value ends the *entire* stream, silently, iff that
    record is the first thing jq's own `fgets`-chunked final buffer
    scans) is matched now, so the two shapes that used to trip it --
    both properties of that same buffer boundary, not of the parser --
    are no longer here; see the module docstring's history note."""
    if reads_unwritten_tokenbuf(visible(data)):
        return True  # jq reads uninitialised memory -- #7256, matched by no implementation
    if RS not in visible(data):
        # #1525's separate template -- except after a malformed BOM, which
        # makes jq read the stream rather than abandon it, so those stay in.
        # Both are judged on the bytes jq's parser sees (#7340).
        return not malformed_bom(visible(data))
    return False


def long_case(rng, plain):
    """A stream whose lines run past jq's 4095-byte `fgets` chunk, with NULs in it (#7340): a
    random record padded with white space, string bytes or array items so that a value is open
    across the chunk's end, and a NUL placed near that end often enough to cut something."""
    out = bytearray(rng.choice(BOM_PREFIXES))
    for _ in range(rng.randint(1, 3)):
        out += RS + bytes(rng.choice(ALPHABET) for _ in range(rng.randint(0, 6)))
        filler = rng.choice([b" ", b"y", b"1,", b"[", b'"a",'])
        out += filler * (rng.randint(3900, 4300) // len(filler))
        tail = bytearray(rng.choice(plain) for _ in range(rng.randint(1, 24)))
        if rng.random() < 0.8:
            tail.insert(rng.randint(0, len(tail)), 0)
        at = len(out) - rng.randint(0, 120)
        out[at:at] = tail
        if rng.random() < 0.7:
            out += b"\n"
    return bytes(out)


def random_cases(count, seed, allow_artifacts, long_lines=False):
    rng = random.Random(seed)
    plain = bytes(b for b in ALPHABET if b not in (RS[0], 0x0A))
    cases = []
    for _ in range(count):
        if long_lines:
            cases.append(long_case(rng, plain))
        elif allow_artifacts:
            body = bytes(rng.choice(ALPHABET) for _ in range(rng.randint(1, 24)))
            cases.append(body)
        elif rng.random() < 0.7:
            body = bytes(rng.choice(ALPHABET) for _ in range(rng.randint(1, 24)))
            # Now and then no RS after the BOM prefix, so a NUL can follow it (#7340).
            cases.append(rng.choice(BOM_PREFIXES) + rng.choice([RS, RS, RS, b""]) + body + b"\n")
        else:
            body = bytes(rng.choice(plain) for _ in range(rng.randint(1, 24)))
            cases.append(rng.choice(BOM_PREFIXES) + RS + body)
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", nargs="?", default=str(REPO_ROOT / "target/release/succinctly"))
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--cases", type=int, default=4000)
    parser.add_argument("--oracle", help="the jq to compare against (default: the pinned one at /usr/bin/jq or on PATH)")
    parser.add_argument(
        "--expect-artifacts",
        action="store_true",
        help="include the line-reader-artifact shapes, to confirm they are still the only divergences",
    )
    parser.add_argument(
        "--long",
        action="store_true",
        help="generate streams whose lines run past jq's 4095-byte fgets chunk, with NULs (#7340)",
    )
    parser.add_argument(
        "--slurp-location",
        action="store_true",
        help="run `--seq -s -c 'error(\"x\")'` instead, comparing the EOF location (#2947/#3003); "
        "stderr is then compared order-insensitively",
    )
    args = parser.parse_args()
    mode_args = SLURP_LOCATION_ARGS if args.slurp_location else WARNINGS_ARGS

    if not os.access(args.binary, os.X_OK):
        # A setup error exits 2, not 1: the compare harness reads 1 as "diverges" (comparison/jq/manifest.toml).
        print(
            f"error: succinctly binary not found at {args.binary} — "
            "run: cargo build --release --features cli",
            file=sys.stderr,
        )
        sys.exit(2)
    jq, version = resolve_oracle(args.oracle)
    print(f"oracle: {jq} ({version}), succinctly: {args.binary}", file=sys.stderr)

    cases = HAND + random_cases(args.cases, args.seed, args.expect_artifacts, args.long)
    stderr_bad, stdout_super, skipped = [], [], 0
    for data in cases:
        if not args.expect_artifacts and artifact(data):
            skipped += 1
            continue
        jq_out, jq_err, jq_code = run([jq], data, mode_args)
        our_out, our_err, our_code = run([args.binary, "jq"], data, mode_args)
        if args.slurp_location:
            # Order-insensitive: see the module docstring.
            same_err = sorted(jq_err.splitlines()) == sorted(our_err.splitlines())
        else:
            same_err = jq_err == our_err
        if not same_err or jq_code != our_code:
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
