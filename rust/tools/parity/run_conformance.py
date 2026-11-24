#!/usr/bin/env python3
"""Runs the original's own conformance suite against the port.

For every `.mr` fixture under `piranha/test/sdl/`:

  1. dump it with line numbers (`ir_dump --lines`) -- if the Rust parser rejects
     the file, the case is reported as PARSE and nothing else can run;
  2. ask the reference for its `code,line` list
     (`PARITY_RULES=test PARITY_LINES=1 script_reference --errors`);
  3. ask the port for its own (`error_dump --rules=piranha`);
  4. compare the two multisets.

The expected lists come from the reference, not from the gtest literals: the
`findError(list, code, line)` assertions in `piranha/test/*.cpp` cover only a
subset of each case, and the reference is the same code those tests run.

A non-zero exit means at least one fixture disagrees -- which is the intended
state while phases are still unported. Read it as a to-do list, not a verdict:
`--summary` groups the differences by error code so the report says *which*
phase each one belongs to.
"""

import argparse
import collections
import os
import pathlib
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parents[3]
RUST = REPO / "rust"
FIXTURES = REPO / "dependencies/submodules/piranha/test/sdl"
WORK = pathlib.Path(os.environ.get("WORK", "/tmp/engine-sim-script-parity"))

IR_DUMP = RUST / "target/release/examples/ir_dump"
ERROR_DUMP = RUST / "target/release/examples/error_dump"
REFERENCE = WORK / "script_reference"


def run(command, **kwargs):
    return subprocess.run(command, capture_output=True, text=True, **kwargs)


def oracle_codes(fixture, dump):
    env = dict(os.environ, PARITY_RULES="test", PARITY_LINES="1")
    proc = run([str(REFERENCE), "--errors", str(fixture), str(dump)], env=env)
    if "unknown value form" in proc.stderr or "cannot" in proc.stderr:
        return None, proc.stderr.strip().splitlines()[0]

    return proc.stdout.split(), None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--summary", action="store_true", help="group differences by code")
    parser.add_argument("--verbose", action="store_true")
    parser.add_argument("fixtures", nargs="*")
    args = parser.parse_args()

    if not REFERENCE.exists():
        sys.exit(f"missing {REFERENCE}; run tools/parity/run_script_parity.sh first")

    fixtures = sorted(FIXTURES.rglob("*.mr"))
    if args.fixtures:
        wanted = set(args.fixtures)
        fixtures = [f for f in fixtures if f.name in wanted or str(f).endswith(tuple(wanted))]

    status = collections.Counter()
    missing = collections.Counter()
    rows = []

    for index, fixture in enumerate(fixtures):
        relative = fixture.relative_to(FIXTURES)
        dump = pathlib.Path(f"/tmp/conformance-{index}.dump")

        dumped = run([str(IR_DUMP), "--lines", str(fixture)])
        if dumped.returncode != 0:
            first = (dumped.stderr.strip().splitlines() or ["parse failed"])[0]
            status["PARSE"] += 1
            rows.append((relative, "PARSE", first, "", ""))
            continue

        dump.write_text(dumped.stdout)
        expected, error = oracle_codes(fixture, dump)
        if expected is None:
            status["EXCLUDED"] += 1
            rows.append((relative, "EXCLUDED", error, "", ""))
            continue

        actual = run([str(ERROR_DUMP), "--rules=piranha", str(fixture)]).stdout.split()

        if collections.Counter(expected) == collections.Counter(actual):
            status["MATCH"] += 1
            continue

        status["DIFF"] += 1
        for code in collections.Counter(expected) - collections.Counter(actual):
            missing[code.split(",")[0]] += 1

        rows.append((relative, "DIFF", " ".join(expected), " ".join(actual), ""))

    for row in rows:
        if row[1] == "DIFF":
            print(f"{row[0]}\n    reference: {row[2]}\n    port:      {row[3] or '<none>'}")
        elif args.verbose:
            print(f"{row[0]}: {row[1]} {row[2]}")

    print("\n" + ", ".join(f"{name} {count}" for name, count in sorted(status.items())))

    if args.summary and missing:
        print("\nmissing from the port, by code:")
        for code, count in missing.most_common():
            print(f"  {code:6} {count}")

    return 0 if status["DIFF"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
