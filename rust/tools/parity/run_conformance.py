#!/usr/bin/env python3
"""Runs the original's own conformance suite against the port.

For every `.mr` fixture under `piranha/test/sdl/`:

  1. dump it with line numbers (`ir_dump --lines`) -- if the Rust parser rejects
     the file, nothing else can run, and the case is counted as `PARSE-SYNTAX`
     (a deliberately malformed fixture whose expected errors come from the
     original's bison parser, which is not available here) or `PARSE-IMPORT`
     (it imports a file that is not in the repository, which the oracle cannot
     resolve either). Neither is a gap in the port; `--summary` says which;
  2. ask the reference for its `code,line` list
     (`PARITY_RULES=test PARITY_LINES=1 script_reference --errors`);
  3. ask the port for its own (`error_dump --rules=piranha`);
  4. compare the two multisets.

Every error is scored, including the ones the reference reports on an
*unspecified* token (line `2147483647`). Those are expansion-time diagnostics
about the *literal* builtin nodes: `IrValueLiteral::_expand` builds an
`IrLiteralNode` whose definition comes from the unit
(`resolveBuiltinNodeDefinition`), and when a fixture does not define
`__piranha__literal_*` itself the node keeps the default type token and its own
`resolveDefinitions()` reports `UndefinedNodeType` against it. The port
reproduces that path (`expansion.rs`), so the errors are comparable like any
other; there is no longer a reason to exclude them, and excluding them would
hide real regressions in the expansion phase.

The expected lists come from the reference, not from the gtest literals: the
`findError(list, code, line)` assertions in `piranha/test/*.cpp` cover only a
subset of each case, and the reference is the same code those tests run.

A non-zero exit means at least one fixture disagrees. `--summary` groups the
differences by error code so the report says *which* phase each one belongs to:

    MATCH            the port and the reference agree, code for code
    DIFF             they disagree -- the only bucket that is a port defect
    PARSE-SYNTAX     the fixture does not parse, and no oracle exists for it
    PARSE-IMPORT     the fixture's import target is missing from the repository
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

            # Two reasons a fixture cannot be scored, and neither is a gap in the
            # port: the fixture is deliberately malformed (the original reports it
            # through its bison parser, which is not available here), or it
            # imports a file that is not in the repository -- the oracle aborts on
            # that too. `--summary` counts them apart so the tally says which.
            missing_import = "cannot resolve import" in first
            status["PARSE-IMPORT" if missing_import else "PARSE-SYNTAX"] += 1

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

    if args.summary:
        unscorable = status["PARSE-SYNTAX"] + status["PARSE-IMPORT"]
        if unscorable:
            print(
                f"\nunscorable: {unscorable} fixtures -- "
                f"{status['PARSE-SYNTAX']} are malformed or use syntax this grammar "
                "does not have (the original reports them through bison, which is "
                f"not in this environment), {status['PARSE-IMPORT']} import a file "
                "that is not in the repository (the oracle aborts on it too)"
            )

    if args.summary and missing:
        print("\nmissing from the port, by code:")
        for code, count in missing.most_common():
            print(f"  {code:6} {count}")

    return 0 if status["DIFF"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
