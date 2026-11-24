#!/usr/bin/env python3
"""Compare the Rust resolution dump against the recorded reference dump.

The row format is the one `script_reference --resolved` writes and
`examples/resolved_dump.rs` reproduces:

    U,<unit>,<nodes>,<definitions>
    P,<unit>,<def>,<defName>,<port>,<portName>,<direction>,<alias>,<typeDef>,<channelType>
    A,<unit>,<node>,<symbol>,<attr>,<attrName>,<resolvedPort>,<direction>

Fields the Rust side cannot compute yet are `-`, so a difference on `typeDef`
(field 8), `channelType` (field 9) or `resolvedPort` (field 6 of an `A` row) is
the open part of M2 -- everything else differing is a bug.

    compare_resolved.py <rust.txt> <reference.txt> [--quiet]

Exit status: 0 if the files are identical, 1 otherwise. `--quiet` suppresses
the summary and prints only the first few differences.
"""

import sys
from collections import Counter

# Field names, indexed by position after the leading tag.
FIELDS = {
    "U": ["unit", "nodes", "definitions"],
    "P": ["unit", "def", "defName", "port", "portName", "direction", "alias", "typeDef", "channelType"],
    "A": ["unit", "node", "symbol", "attr", "attrName", "resolvedPort", "direction"],
}

# The fields M2 is still allowed to be missing.
OPEN = {
    ("P", "typeDef"),
    ("P", "channelType"),
    ("A", "resolvedPort"),
    ("A", "direction"),
}


def load(path):
    with open(path, "r", encoding="utf-8") as stream:
        return [line.rstrip("\n") for line in stream if line.strip()]


def main(argv):
    if len(argv) < 3:
        print(__doc__.strip().splitlines()[0], file=sys.stderr)
        return 2

    rust = load(argv[1])
    reference = load(argv[2])
    quiet = "--quiet" in argv[3:]

    if len(rust) != len(reference):
        print(
            f"row count differs: rust={len(rust)} reference={len(reference)}",
            file=sys.stderr,
        )

    differences = []
    open_misses = Counter()
    unexplained = Counter()
    identical = 0

    for index, (left, right) in enumerate(zip(rust, reference)):
        if left == right:
            identical += 1
            continue

        left_fields = left.split(",")
        right_fields = right.split(",")
        tag = right_fields[0]
        names = FIELDS.get(tag, [])

        for position, (a, b) in enumerate(zip(left_fields, right_fields)):
            if a == b:
                continue

            # `position` counts the tag, which is not in FIELDS.
            name = names[position - 1] if 0 < position <= len(names) else f"#{position}"
            if (tag, name) in OPEN:
                open_misses[(tag, name)] += 1
            else:
                unexplained[(tag, name)] += 1

        differences.append((index, left, right))

    total = max(len(reference), 1)
    if not quiet:
        print(f"rows: rust={len(rust)} reference={len(reference)}")
        print(f"identical: {identical}/{total} ({100.0 * identical / total:.1f}%)")

        if open_misses:
            print("still open (expected, M2):")
            for (tag, name), count in sorted(open_misses.items()):
                print(f"  {tag}.{name}: {count}")

        if unexplained:
            print("unexplained (a bug):")
            for (tag, name), count in sorted(unexplained.items()):
                print(f"  {tag}.{name}: {count}")

    for index, left, right in differences[:3]:
        print(f"line {index + 1}\n  rust: {left}\n  ref:  {right}", file=sys.stderr)

    return 0 if not differences else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
