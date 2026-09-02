#!/usr/bin/env python3
"""Diff succinctly yq's streaming output against its DOM output, per document.

    scripts/streaming-dom-diff.py --binary target/release/succinctly \\
        --corpus tests/data/streaming-dom-corpus

`docs/compliance/yq/limitations.md`'s routing-divergences section states the
lesson this mechanizes: "when auditing what a construct loses, the question to
ask is which output route it takes." For each document in the corpus, this
runs the same query twice — once forcing the streaming (M2) output path, once
forcing the DOM (`OwnedValue`) path via `--arg unused 1` (which makes
`context.named` non-empty without changing anything else about the output
config) — and diffs the two. Any difference is a real streaming/DOM
divergence: a construct one path preserves and the other silently loses or
reshapes (duplicate keys, anchors/aliases, comments, tag fidelity, ...).

Run once with `--flags` empty (the default) to see the *pre-existing*
divergence set for this corpus. Pass `--flags` again with the specific flag(s)
you're auditing (e.g. `--flags='--ascii-output'`) and this reports which
per-document divergences are already covered by the empty-flags baseline
("pre-existing") versus only appear once the flag is added ("introduced") —
the second category is what a change under review should not be adding.
`--control-flags` lets that baseline itself be a non-empty set instead of
empty, for auditing one flag against another rather than against nothing.

Found #1982 (two real DOM-path divergences) the first time an ancestor of this
script ran, while investigating #1700.

Standard library only; no third-party dependencies.
"""

import argparse
import glob
import os
import shlex
import subprocess
import sys


def parse_args(argv=None):
    p = argparse.ArgumentParser(
        description="Diff succinctly yq's streaming output against its DOM output.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__.split("\n\n", 1)[1],
    )
    p.add_argument("--binary", default="target/release/succinctly",
                    help="path to the succinctly binary (default: %(default)s)")
    p.add_argument("--corpus", default="tests/data/streaming-dom-corpus",
                    help="directory searched (non-recursively) for .yaml/.yml documents "
                         "(default: %(default)s)")
    p.add_argument("--query", default=".", help="query to run (default: %(default)s)")
    p.add_argument("--output", default="json", help="-o format (default: %(default)s)")
    p.add_argument("--flags", default="",
                    help="extra yq flags under test, as one shell-quoted string -- use "
                         "the '=' form (--flags='--ascii-output'), since argparse "
                         "otherwise mistakes a leading '-' for a new option")
    p.add_argument("--control-flags", default="",
                    help="extra yq flags for the baseline that distinguishes pre-existing "
                         "divergences from ones --flags introduces (default: none), same "
                         "'=' shell-quoted-string form as --flags")
    p.add_argument("--verbose", action="store_true",
                    help="print both outputs in full for every diverging document, not "
                         "just a one-line summary")
    args = p.parse_args(argv)
    args.flags = shlex.split(args.flags)
    args.control_flags = shlex.split(args.control_flags)
    return args


def collect_corpus(corpus_dir):
    paths = sorted(
        p for pattern in ("*.yaml", "*.yml")
        for p in glob.glob(os.path.join(corpus_dir, pattern))
    )
    if not paths:
        sys.exit(f"no .yaml/.yml documents found under {corpus_dir!r}")
    return paths


def run(binary, doc_path, query, output, extra_flags, force_dom):
    cmd = [binary, "yq", "-o", output, "-I0", *extra_flags]
    if force_dom:
        cmd += ["--arg", "unused", "1"]
    cmd += [query, doc_path]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
        return proc.returncode, proc.stdout, proc.stderr.strip()
    except subprocess.TimeoutExpired:
        return -9, "", "timeout"


def diverges(binary, doc_path, query, output, extra_flags):
    """Run streaming vs. DOM for one document/flag-set; return (differs, streaming, dom)."""
    s_code, s_out, s_err = run(binary, doc_path, query, output, extra_flags, force_dom=False)
    d_code, d_out, d_err = run(binary, doc_path, query, output, extra_flags, force_dom=True)
    differs = (s_code, s_out) != (d_code, d_out)
    return differs, (s_code, s_out, s_err), (d_code, d_out, d_err)


def main(argv=None):
    args = parse_args(argv)
    if not (os.path.isfile(args.binary) and os.access(args.binary, os.X_OK)):
        sys.exit(f"--binary {args.binary!r} is not an executable file "
                  "(build it first: cargo build --release --features cli)")
    docs = collect_corpus(args.corpus)

    print(f"=== {len(docs)} documents, query={args.query!r}, "
          f"flags={args.flags or '(none)'}, control={args.control_flags or '(none)'} ===")

    pre_existing, introduced, fixed_by_flag, clean = [], [], [], 0
    for doc in docs:
        name = os.path.basename(doc)
        base_differs, _, _ = diverges(args.binary, doc, args.query, args.output,
                                       args.control_flags)
        test_differs, s, d = diverges(args.binary, doc, args.query, args.output, args.flags)

        if base_differs and test_differs:
            pre_existing.append(name)
        elif test_differs and not base_differs:
            introduced.append((name, s, d))
        elif base_differs and not test_differs:
            fixed_by_flag.append(name)
        else:
            clean += 1

    print(f"\n  {clean:4}  no divergence in either mode")
    print(f"  {len(pre_existing):4}  pre-existing (baseline also differs): "
          f"{', '.join(pre_existing) if pre_existing else '-'}")
    print(f"  {len(fixed_by_flag):4}  flag fixes a pre-existing divergence: "
          f"{', '.join(fixed_by_flag) if fixed_by_flag else '-'}")
    print(f"  {len(introduced):4}  INTRODUCED (only diverges with --flags): "
          f"{', '.join(n for n, _, _ in introduced) if introduced else '-'}")

    if introduced:
        print("\n=== introduced divergences ===")
        for name, (s_code, s_out, s_err), (d_code, d_out, d_err) in introduced:
            print(f"\n[{name}]")
            if args.verbose:
                print(f"  streaming (exit {s_code}): {s_out!r}" + (f"  err={s_err!r}" if s_err else ""))
                print(f"  dom       (exit {d_code}): {d_out!r}" + (f"  err={d_err!r}" if d_err else ""))
            else:
                print(f"  streaming exit={s_code} dom exit={d_code}  "
                      f"({'output differs' if s_out != d_out else 'exit code differs'})")

    return 1 if introduced else 0


if __name__ == "__main__":
    sys.exit(main())
