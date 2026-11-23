#!/usr/bin/env bash
# Filter-level DSP parity check: every filter of the audio chain, driven with
# the same input on both sides, compared bit for bit.
#
#   ./run_dsp_parity.sh [--keep]
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="$(cd "$HERE/../.." && pwd)"
REPO_ROOT="$(cd "$RUST_DIR/.." && pwd)"
WORK="${WORK:-/tmp/engine-sim-dsp-parity}"
REF_CORE="${REF_CORE:-/tmp/refcore}"
KEEP="${1:-}"

export CARGO_HOME="${CARGO_HOME:-/tmp/cargo-home}"
export PATH="/tmp/rust-toolchain/bin:$PATH"

if [ ! -f "$REF_CORE/librefcore.a" ]; then
    "$HERE/stage_reference.sh"
fi

mkdir -p "$WORK"

echo "== building reference (C++) =="

# `Synthesizer::renderAudio` calls `rand()`, so this TU must be compiled with the
# same substitution the core harness uses.
g++ -std=c++14 -O2 -fpermissive -D__forceinline=inline \
    -include cmath -include cstdlib -include cstring \
    -include "$HERE/parity_rand.h" \
    -I "$REF_CORE/include" \
    -I "$REF_CORE/dependencies/submodules/simple-2d-constraint-solver/include" \
    -I "$REF_CORE/dependencies/submodules/csv-io/include" \
    -c "$REF_CORE/src/synthesizer.cpp" -o "$WORK/synthesizer_parity.o"
g++ -std=c++14 -O2 -fpermissive -D__forceinline=inline \
    -include cmath -include cstdlib -include cstring \
    -include "$HERE/parity_rand.h" \
    -I "$REF_CORE/include" \
    -I "$REPO_ROOT/include" \
    -I "$REF_CORE/dependencies/submodules/simple-2d-constraint-solver/include" \
    -I "$REF_CORE/dependencies/submodules/csv-io/include" \
    -Wl,--allow-multiple-definition \
    "$HERE/dsp_reference.cpp" "$HERE/parity_rand.cpp" \
    "$WORK/synthesizer_parity.o" "$REF_CORE/librefcore.a" -o "$WORK/dsp_reference"

echo "== building rust =="
(cd "$RUST_DIR" && cargo build --release --quiet --example dsp_trace)

echo "== running =="
"$WORK/dsp_reference" > "$WORK/cpp.txt"
"$RUST_DIR/target/release/examples/dsp_trace" > "$WORK/rust.txt"

if diff -q "$WORK/cpp.txt" "$WORK/rust.txt" >/dev/null; then
    echo "PASS: $(wc -l < "$WORK/cpp.txt") filter outputs bit-identical"
    [ "$KEEP" = "--keep" ] || rm -rf "$WORK"
else
    echo "FAIL: $(diff "$WORK/cpp.txt" "$WORK/rust.txt" | grep -c '^<') mismatches"
    diff "$WORK/cpp.txt" "$WORK/rust.txt" | head -20
    echo "--- first mismatch per case:"
    python3 - "$WORK/cpp.txt" "$WORK/rust.txt" <<'PY'
import sys
def load(p):
    d = {}
    for i, line in enumerate(open(p)):
        name, bits = line.strip().split(',')
        d.setdefault(name, []).append(bits)
    return d
a, b = load(sys.argv[1]), load(sys.argv[2])
for name in a:
    if a[name] != b.get(name):
        idx = next(i for i in range(len(a[name])) if a[name][i] != b[name][i])
        print(f"  {name}: sample {idx}  cpp={a[name][idx]} rust={b[name][idx]}")
    else:
        print(f"  {name}: OK ({len(a[name])} samples)")
PY
    exit 1
fi
