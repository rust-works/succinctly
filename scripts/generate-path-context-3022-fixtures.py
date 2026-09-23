#!/usr/bin/env python3
"""Generate the deterministic fixtures used by issue #3022.

The record corpus is a top-level sequence of ``{"k":{"x":n}}`` values,
encoded as compact JSON and block YAML twins. Size-labelled pairs contain the
same number of records; the JSON member is the first one at least as large as
the requested size (``mb`` is decimal; ``mib`` is binary).

The deep corpus is one chain of ``depth`` nested objects ending in an array of
string leaves. The array index makes each leaf's absolute path one level deeper
than the named object depth, matching the seed-allocation sweep.
"""

from __future__ import annotations

import argparse
from pathlib import Path


MB = 1_000_000
MIB = 1024 * 1024


def parse_size(value: str) -> int:
    text = value.strip().lower()
    if text.endswith("mb"):
        return int(text[:-2]) * MB
    if text.endswith("mib"):
        return int(text[:-3]) * MIB
    raise argparse.ArgumentTypeError(f"size must end in mb or mib: {value!r}")


def json_record(index: int) -> str:
    return f'{{"k":{{"x":{index}}}}}'


def record_count_for_size(target_bytes: int) -> int:
    # Opening bracket, closing bracket and final newline.
    size = 3
    count = 0
    while size < target_bytes:
        if count:
            size += 1  # comma
        size += len(json_record(count))
        count += 1
    return count


def write_record_pair(output_dir: Path, label: str, count: int) -> None:
    json_path = output_dir / f"records-{label}.json"
    yaml_path = output_dir / f"records-{label}.yaml"

    with json_path.open("w", encoding="utf-8", newline="\n") as handle:
        handle.write("[")
        for index in range(count):
            if index:
                handle.write(",")
            handle.write(json_record(index))
        handle.write("]\n")

    with yaml_path.open("w", encoding="utf-8", newline="\n") as handle:
        for index in range(count):
            handle.write(f"- k:\n    x: {index}\n")

    print(
        f"{label}: records={count} "
        f"json_bytes={json_path.stat().st_size} yaml_bytes={yaml_path.stat().st_size}"
    )


def deep_document(leaves: int, depth: int) -> str:
    value = "[" + ",".join(f'"leaf-{index}"' for index in range(leaves)) + "]"
    for _ in range(depth):
        value = f'{{"k":{value}}}'
    return value


def write_deep_fixture(output_dir: Path, leaves: int, depth: int) -> None:
    path = output_dir / f"deep-d{depth}-n{leaves}.json"
    with path.open("w", encoding="utf-8", newline="\n") as handle:
        handle.write(deep_document(leaves, depth))
        handle.write("\n")
    print(f"deep-d{depth}-n{leaves}: bytes={path.stat().st_size}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output_dir", type=Path)
    parser.add_argument(
        "--sizes",
        nargs="+",
        default=["1mb", "6mb", "20mb"],
        help="record-corpus JSON sizes; mb is decimal, mib is binary "
        "(default: 1mb 6mb 20mb)",
    )
    args = parser.parse_args()

    args.output_dir.mkdir(parents=True, exist_ok=True)
    write_record_pair(args.output_dir, "n1000", 1000)
    for size_text in args.sizes:
        size_bytes = parse_size(size_text)
        write_record_pair(
            args.output_dir,
            size_text.lower(),
            record_count_for_size(size_bytes),
        )
    for leaves, depth in [(1000, 64), (1000, 32), (1000, 16), (250, 64)]:
        write_deep_fixture(args.output_dir, leaves, depth)


if __name__ == "__main__":
    main()
