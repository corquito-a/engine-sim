#!/usr/bin/env python3
"""Records and checks the node-graph fixtures.

`IrCompilationUnit::build` turns resolved IR into the `Node` graph the runtime
executes. Two programs produce it: `script_reference --graph` (the original) and
`graph_dump` (`crates/engine-sim-script/examples`, the port). This script keeps
them honest:

    ./run_graph.py --record      # (re)record every `graph/*.graph` from the reference
    ./run_graph.py               # compare the port against the recordings

`--record` also checks the port against what it just recorded, so a recording
cannot quietly bless a divergence. A divergence reported here is a difference in
the graph -- which node was generated where, which port a connection points at,
which container holds what -- not a formatting difference.

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
GRAPH_DUMP = RUST / "target/release/examples/graph_dump"
REFERENCE = WORK / "script_reference"


def run(command, **kwargs):
    return subprocess.run(command, capture_output=True, text=True, **kwargs)


def graph_from_reference(fixture):
    """The original's graph: its own `build`, fed the Rust parser's IR."""
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


def graph_from_port(fixture):
    """The port's own graph, built from its own IR."""
    port = run([str(GRAPH_DUMP), "--rules=piranha", str(fixture)])
    if port.returncode != 0:
        return None, port.stderr.strip().splitlines()[0] if port.stderr else "graph failed"

    return port.stdout, None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--record", action="store_true")
    parser.add_argument("fixtures", nargs="*")
    args = parser.parse_args()

    if not GRAPH_DUMP.exists() or (args.record and not REFERENCE.exists()):
        print("run tools/parity/run_script_parity.sh first (it builds both sides)")
        return 2

    fixtures = sorted(FIXTURES.glob("*.mr"))
    if args.fixtures:
        wanted = {pathlib.Path(name).stem for name in args.fixtures}
        fixtures = [fixture for fixture in fixtures if fixture.stem in wanted]

    failures = 0
    for fixture in fixtures:
        expected_path = fixture.with_suffix(".graph")

        if args.record:
            reference, error = graph_from_reference(fixture)
            if reference is None:
                print(f"FAIL {fixture.name}: {error}")
                failures += 1
                continue

            expected_path.write_text(reference)
            print(f"recorded {expected_path.name} ({len(reference.splitlines())} records)")

        expected = expected_path.read_text() if expected_path.exists() else ""
        text, error = graph_from_port(fixture)

        if text is None:
            print(f"FAIL {fixture.name}: {error}")
            failures += 1
            continue

        if text != expected:
            print(f"FAIL {fixture.name}: the port's graph differs from {expected_path.name}")
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
