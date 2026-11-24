#!/usr/bin/env python3
"""Gate sensitivity, measured rather than assumed.

Applies one deliberate fault at a time to the ported circularity phase and
reports which gate notices. The point is not coverage of the code -- it is to
find out which *failure modes* the gates are blind to, because a hybrid of
gates only buys confidence where their blind spots differ.

Usage: python3 rust/tools/mutation_check.py [--commit SHA]

Mutates a tracked file in place, runs `cargo test`, then restores the file from
git. Refuses to run on a dirty file so nothing is ever lost.
"""

import argparse
import pathlib
import re
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
TARGET = REPO / "rust/crates/engine-sim-script/src/circular.rs"
CRATE = REPO / "rust"

MUTATIONS = [
    (
        "main_context_flag",
        "contexts.main_child(context, Some(structure))",
        "contexts.child(context, Some(structure))",
        "a main context used where a plain one belongs",
    ),
    (
        "drop_loop_guard",
        """        // Loop guard for a cycle inside a definition that is not the focus.
        if contexts.find_context(context, structure).is_some() {
            return;
        }

""",
        "",
        "no guard against re-entering a node already in the context chain",
    ),
    (
        "no_root_error",
        "if definition == Some(root) {",
        "if false {",
        "the cycle is never reported",
    ),
    (
        "invert_check_references",
        "if self.entry(child).check_references {",
        "if !self.entry(child).check_references {",
        "components gated on the wrong flag value",
    ),
    (
        "frames_unsorted",
        "    frames.reverse();\n",
        "",
        "instantiation frames in newest-first order",
    ),
    (
        "no_definition_descend",
        "if let Some(definition) = definition {",
        "if let Some(definition) = None::<StructureId> {",
        "the walk never follows a node into its definition",
    ),
]


def git(*args):
    return subprocess.run(
        ["git", *args], cwd=REPO, capture_output=True, text=True, check=False
    )


def run_tests():
    """Returns (ok, failing test names, compile failure?)."""
    proc = subprocess.run(
        ["cargo", "test", "-p", "engine-sim-script", "--quiet"],
        cwd=CRATE,
        capture_output=True,
        text=True,
    )
    out = proc.stdout + proc.stderr

    if "error[E" in out or "error: could not compile" in out:
        return False, ["<compile error>"], True

    failing = sorted(set(re.findall(r"^failures:\n\n((?:    \S+\n)+)", out, re.M)[0].split())
                 if re.search(r"^failures:\n\n((?:    \S+\n)+)", out, re.M) else [])

    # A test binary that aborts (stack overflow) still fails the run.
    for name in re.findall(r"^---- (\S+) stdout ----", out, re.M):
        failing.append(name)

    return proc.returncode == 0, sorted(set(failing)), False


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--commit", default="HEAD")
    args = parser.parse_args()

    if git("status", "--porcelain", "--", str(TARGET)).stdout.strip():
        sys.exit(f"{TARGET} is dirty; commit or stash it first")

    original = TARGET.read_text()
    results = []

    try:
        ok, failing, _ = run_tests()
        results.append(("(unmutated)", ok, failing, "-"))
        if not ok:
            sys.exit("baseline is red; nothing to measure")

        for name, old, new, description in MUTATIONS:
            if old not in original:
                sys.exit(f"mutation {name}: pattern not found in {TARGET.name}")

            TARGET.write_text(original.replace(old, new, 1))

            try:
                ok, failing, compiled = run_tests()
            finally:
                TARGET.write_text(original)

            results.append((name, ok, failing, description))
            status = "caught" if not ok else "MISSED"
            print(f"{status:7} {name:24} {description}", flush=True)
    finally:
        TARGET.write_text(original)

    print("\n| mutation | gate | caught by |")
    print("|---|---|---|")
    for name, ok, failing, description in results[1:]:
        verdict = "no" if ok else "yes"
        caught = ", ".join(failing) if failing else "—"
        print(f"| `{name}` | {description} | {verdict}: {caught} |")

    missed = [r[0] for r in results[1:] if r[1]]
    print(f"\n{len(results) - 1 - len(missed)}/{len(results) - 1} mutations caught")
    if missed:
        print("blind spots: " + ", ".join(missed))


if __name__ == "__main__":
    main()
