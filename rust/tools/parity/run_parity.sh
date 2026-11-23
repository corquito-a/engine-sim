#!/usr/bin/env bash
# Bit-exact parity check: original C++ gas system vs. the Rust port.
#
#   1. compiles the repo's own src/gas_system.cpp + src/utilities.cpp with g++
#   2. runs the Rust trace example
#   3. diffs the raw f64 bit patterns
#
# Two build-only concessions are needed to compile the original sources with
# anything other than MSVC (they are the reason this project only builds on
# Windows today):
#
#   * `const Mix &mix = {}` default arguments inside the class definition are
#     rejected by GCC/Clang, so the reference build strips those defaults and
#     every call site passes the mixture explicitly (`Mix{0,1,0}`, which is what
#     `Mix{}` means).
#   * `extern constexpr double` in units.h/constants.h is a definition with
#     external linkage, so it is multiply-defined across TUs; the linker is told
#     to accept the duplicates (all definitions are identical constants).
#
# Neither concession changes any arithmetic.
#
# Usage: ./run_parity.sh [--keep]
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../../.." && pwd)"
WORK="${PARITY_WORK:-/tmp/engine-sim-parity}"
KEEP=0
[ "${1:-}" = "--keep" ] && KEEP=1

mkdir -p "$WORK"

if ! command -v cargo >/dev/null 2>&1; then
    if [ -x /tmp/rust-toolchain/bin/cargo ]; then
        export PATH="/tmp/rust-toolchain/bin:$PATH"
    else
        echo "error: cargo not found; run rust/tools/bootstrap-toolchain.sh first" >&2
        exit 2
    fi
fi
export CARGO_HOME="${CARGO_HOME:-/tmp/cargo-home}"

echo "==> preparing reference build tree"
rm -rf "$WORK/ref"
mkdir -p "$WORK/ref"
cp -r "$REPO_ROOT/include" "$REPO_ROOT/src" "$WORK/ref/"
sed -i -E 's/const Mix &mix = \{\}/const Mix \&mix/g; s/const Mix &mix = Mix\(\)/const Mix \&mix/g' \
    "$WORK/ref/include/gas_system.h"

echo "==> building C++ reference (original sources)"
g++ -std=c++17 -O2 -Wl,--allow-multiple-definition \
    -I "$WORK/ref/include" \
    "$HERE/gas_reference.cpp" \
    "$WORK/ref/src/gas_system.cpp" \
    "$WORK/ref/src/utilities.cpp" \
    -o "$WORK/gas_reference"

echo "==> building Rust trace"
( cd "$REPO_ROOT/rust" && cargo build --release --quiet --example gas_trace )

CPP_OUT="$WORK/cpp.txt"
RS_OUT="$WORK/rust.txt"

"$WORK/gas_reference" > "$CPP_OUT"
"$REPO_ROOT/rust/target/release/examples/gas_trace" > "$RS_OUT"

echo "==> comparing $(wc -l < "$CPP_OUT") checkpoints"
if diff -q "$CPP_OUT" "$RS_OUT" >/dev/null; then
    echo "PASS: Rust and C++ produce identical f64 bit patterns on every checkpoint"
    [ "$KEEP" -eq 1 ] && echo "outputs kept in $WORK"
    exit 0
else
    echo "FAIL: divergences found" >&2
    diff "$CPP_OUT" "$RS_OUT" | head -40 >&2
    echo "(full outputs in $WORK)" >&2
    exit 1
fi
