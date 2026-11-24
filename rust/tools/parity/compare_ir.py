#!/usr/bin/env python3
"""Compares two `.mr` IR dumps structurally.

The Rust front end writes the dump produced by `engine-sim-script`; the C++
oracle reads it back into real piranha objects and prints its own. The two
texts cannot be compared byte for byte -- the writers use different float
formatters -- so this script parses both and compares the trees, treating
atoms that hold numbers as numbers and everything else as opaque text.

    ./compare_ir.py <rust.txt> <cpp.txt> [--quiet]

Exit status is 0 when the trees are identical and 1 otherwise.
"""

import sys


class Atom:
    def __init__(self, text):
        self.text = text

    def __eq__(self, other):
        if not isinstance(other, Atom):
            return NotImplemented
        if self.text == other.text:
            return True
        # Numbers may be spelled differently by the two writers.
        try:
            return float(self.text) == float(other.text)
        except ValueError:
            return False

    def __repr__(self):
        return self.text


class List:
    def __init__(self, items):
        self.items = items

    def head(self):
        if self.items and isinstance(self.items[0], Atom):
            return self.items[0].text
        return None

    def __repr__(self):
        return "(" + " ".join(repr(item) for item in self.items) + ")"


def parse(text):
    position = 0
    length = len(text)

    def skip():
        nonlocal position
        while position < length:
            char = text[position]
            if char.isspace():
                position += 1
            elif char == ";":
                while position < length and text[position] != "\n":
                    position += 1
            else:
                break

    def read():
        nonlocal position
        skip()
        if position >= length:
            raise ValueError("unexpected end of input")

        char = text[position]
        if char == "(":
            position += 1
            items = []
            while True:
                skip()
                if position >= length:
                    raise ValueError("unterminated list")
                if text[position] == ")":
                    position += 1
                    return List(items)
                items.append(read())

        if char == '"':
            position += 1
            out = []
            while position < length:
                char = text[position]
                position += 1
                if char == '"':
                    return Atom("".join(out))
                if char == "\\":
                    escaped = text[position]
                    position += 1
                    out.append({"n": "\n", "t": "\t", "r": "\r"}.get(escaped, escaped))
                else:
                    out.append(char)
            raise ValueError("unterminated string")

        start = position
        while position < length:
            char = text[position]
            if char.isspace() or char in '()"':
                break
            position += 1
        return Atom(text[start:position])

    forms = []
    while True:
        skip()
        if position >= length:
            return forms
        forms.append(read())


def compare(left, right, path, differences):
    if isinstance(left, List) or isinstance(right, List):
        if not (isinstance(left, List) and isinstance(right, List)):
            differences.append("%s: list/atom mismatch (%r vs %r)" % (path, left, right))
            return
        if len(left.items) != len(right.items):
            heads = (left.head(), right.head())
            differences.append(
                "%s: %d vs %d items (%s vs %s)"
                % (path, len(left.items), len(right.items), heads[0], heads[1])
            )
            return
        for index, (left_item, right_item) in enumerate(zip(left.items, right.items)):
            head = right_item.head() if isinstance(right_item, List) else None
            label = "%s/%s[%d]" % (path, head or "?", index) if head else "%s[%d]" % (path, index)
            compare(left_item, right_item, label, differences)
        return

    if left != right:
        differences.append("%s: %r != %r" % (path, left.text, right.text))


def main():
    arguments = [entry for entry in sys.argv[1:] if not entry.startswith("--")]
    quiet = "--quiet" in sys.argv[1:]

    if len(arguments) != 2:
        print("usage: compare_ir.py <rust.txt> <cpp.txt> [--quiet]", file=sys.stderr)
        return 2

    with open(arguments[0], encoding="utf-8") as stream:
        left = parse(stream.read())
    with open(arguments[1], encoding="utf-8") as stream:
        right = parse(stream.read())

    differences = []
    compare(List(left), List(right), "", differences)

    if differences:
        print("MISMATCH %s vs %s: %d differences" % (arguments[0], arguments[1], len(differences)))
        for difference in differences[:10]:
            print("  " + difference)
        return 1

    if not quiet:
        print("OK %s" % arguments[0])
    return 0


if __name__ == "__main__":
    sys.exit(main())
