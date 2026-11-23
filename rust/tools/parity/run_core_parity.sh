#!/usr/bin/env bash
# Whole-core parity check: builds the original C++ simulation core (the
# reference tree staged by `run_parity.sh`), runs the same single-cylinder
# engine in both implementations, and compares every traced `f64` bit for bit.
#
#   ./run_core_parity.sh [steps] [--keep]
#
# Requires the reference core archive at $REF_CORE (default /tmp/refcore).
set -euo pipefail

STEPS="${1:-2000}"
KEEP="${2:-}"
REF_CORE="${REF_CORE:-/tmp/refcore}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="$(cd "$HERE/../.." && pwd)"
WORK="${WORK:-/tmp/engine-sim-core-parity}"

CARGO="${CARGO:-cargo}"
export CARGO_HOME="${CARGO_HOME:-/tmp/cargo-home}"
export PATH="/tmp/rust-toolchain/bin:$PATH"

if [ ! -f "$REF_CORE/librefcore.a" ]; then
    echo "== building the reference core (first run only) =="
    "$HERE/stage_reference.sh"
fi

mkdir -p "$WORK"

echo "== building reference (C++) =="
g++ -std=c++14 -O2 -fpermissive -D__forceinline=inline \
    -include cmath -include cstdlib -include cstring \
    -include "$HERE/parity_rand.h" \
    -I "$REF_CORE/include" \
    -I "$REF_CORE/dependencies/submodules/simple-2d-constraint-solver/include" \
    -I "$REF_CORE/dependencies/submodules/csv-io/include" \
    -c "$REF_CORE/src/combustion_chamber.cpp" -o "$WORK/combustion_chamber_parity.o"

g++ -std=c++14 -O2 -fpermissive -D__forceinline=inline \
    -include cmath -include cstdlib -include cstring \
    -I "$REF_CORE/include" \
    -I "$REF_CORE/dependencies/submodules/simple-2d-constraint-solver/include" \
    -I "$REF_CORE/dependencies/submodules/csv-io/include" \
    -Wl,--allow-multiple-definition \
    "$HERE/core_reference.cpp" "$HERE/parity_rand.cpp" \
    "$WORK/combustion_chamber_parity.o" "$REF_CORE/librefcore.a" \
    -o "$WORK/core_reference"

echo "== building rust =="
(cd "$RUST_DIR" && "$CARGO" build --release --example core_trace >/dev/null)

echo "== running =="
"$WORK/core_reference" "$STEPS" > "$WORK/cpp.txt"
"$RUST_DIR/target/release/examples/core_trace" "$STEPS" > "$WORK/rust.txt"

if diff -q "$WORK/cpp.txt" "$WORK/rust.txt" >/dev/null; then
    echo "PASS: $(wc -l < "$WORK/cpp.txt") checkpoints bit-identical ($STEPS steps)"
    [ "$KEEP" = "--keep" ] || rm -rf "$WORK"
else
    echo "FAIL: $(diff "$WORK/cpp.txt" "$WORK/rust.txt" | grep -c '^<') mismatches"
    diff "$WORK/cpp.txt" "$WORK/rust.txt" | head -20
    exit 1
fi
