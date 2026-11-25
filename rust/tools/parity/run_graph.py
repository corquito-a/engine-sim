#!/usr/bin/env python3
"""Records and checks the node-graph fixtures.

`script_reference --graph` builds the node graph the IR generates
(`IrCompilationUnit::build`) and dumps its structure. Those dumps are the target
the port's `Node`/`NodeOutput`/`NodeProgram` port has to reproduce, and this
script is what keeps them honest:

    ./run_graph.py --record      # (re)record every `graph/*.graph`
    ./run_graph.py               # compare the reference against the recordings

Until the Rust graph builder exists the comparison is reference-against-recorded,
which catches a change in the driver or in a fixture. When `graph_dump`
(`crates/engine-sim-script/examples`) lands it becomes the second side, and the
same recordings become a real port gate -- the format does not change.

Record layout, one record per line (see `GraphDumper` in `script_reference.cpp`):

    G,<containerId>,<name>,<parentId>,<nodeCount>,<childCount>
    CI,<containerId>,<index>,<name>,<modifies>,<enable>
    CO,<containerId>,<index>,<name>,<targetNode>,<targetPort>,<nodeOutput>
    N,<containerId>,<nodeId>,<name>,<builtinName>,<virtual>,<inputs>,<outputs>
    O,<nodeId>,<index>,<name>,<channelType>,<interfaceNode>
    I,<nodeId>,<index>,<name>,<connectionNode>,<connectionPort>,<dependencyNode>

`-` is a null pointer. Node ids follow traversal order: a container's own nodes
in insertion order, then its children, depth first.
"""

import argparse
import os
import pathlib
import subprocess
import sys

PARITY = pathlib.Path(__file__).resolve().parent
REPO = PARITY.parents[2]
RUST = REPO / "rust"
FIXTURES = PARITY / "graph"
WORK = pathlib.Path(os.environ.get("WORK", "/tmp/engine-sim-script-parity"))

IR_DUMP = RUST / "target/release/examples/ir_dump"
REFERENCE = WORK / "script_reference"


def run(command, **kwargs):
    return subprocess.run(command, capture_output=True, text=True, **kwargs)


def graph_for(fixture):
    dumped = run([str(IR_DUMP), str(fixture)])
    if dumped.returncode != 0:
        return None, dumped.stderr.strip().splitlines()[0] if dumped.stderr else "parse failed"

    dump = WORK / (fixture.stem + ".ir")
    dump.write_text(dumped.stdout)

    env = dict(os.environ, PARITY_RULES="test")
    reference = run([str(REFERENCE), "--graph", str(fixture), str(dump)], env=env)
    if reference.returncode != 0:
        return None, reference.stderr.strip().splitlines()[0] if reference.stderr else "graph failed"

    return reference.stdout, None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--record", action="store_true")
    parser.add_argument("fixtures", nargs="*")
    args = parser.parse_args()

    if not REFERENCE.exists():
        print(f"missing {REFERENCE}; run tools/parity/run_script_parity.sh first")
        return 2

    fixtures = sorted(FIXTURES.glob("*.mr"))
    if args.fixtures:
        wanted = {pathlib.Path(name).stem for name in args.fixtures}
        fixtures = [fixture for fixture in fixtures if fixture.stem in wanted]

    failures = 0
    for fixture in fixtures:
        text, error = graph_for(fixture)
        if text is None:
            print(f"FAIL {fixture.name}: {error}")
            failures += 1
            continue

        expected_path = fixture.with_suffix(".graph")
        if args.record:
            expected_path.write_text(text)
            print(f"recorded {expected_path.name} ({len(text.splitlines())} records)")
            continue

        expected = expected_path.read_text() if expected_path.exists() else ""
        if text != expected:
            print(f"FAIL {fixture.name}: graph differs from {expected_path.name}")
            for line in text.splitlines():
                if line not in expected.splitlines():
                    print(f"    + {line}")
            for line in expected.splitlines():
                if line not in text.splitlines():
                    print(f"    - {line}")
            failures += 1
        else:
            print(f"ok   {fixture.name} ({len(text.splitlines())} records)")

    if failures:
        print(f"\n{failures} graph fixture(s) differ")
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
