#!/usr/bin/env bash
# Builds `$WORK/script_reference`, the oracle the `.mr` gates drive.
#
#   ./build_script_reference.sh
#
# Shared by `run_script_parity.sh` (the IR, resolved and error gates) and
# `record_builtin_ports.sh` (which only needs the language rules), so the two
# cannot drift apart in how the reference is compiled.
set -euo pipefail

REF_SCRIPT="${REF_SCRIPT:-/tmp/refscript}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="$(cd "$HERE/../.." && pwd)"
REPO_ROOT="$(cd "$RUST_DIR/.." && pwd)"
WORK="${WORK:-/tmp/engine-sim-script-parity}"

if [ ! -f "$REF_SCRIPT/librefscript.a" ]; then
    echo "== building the scripting reference (first run only) =="
    "$HERE/stage_script_reference.sh"
fi

mkdir -p "$WORK"
cd "$REPO_ROOT"

echo "== building the oracle =="

# `Synthesizer::renderAudio` and the combustion chamber draw from `rand()`; the
# core parity harness replaces the generator so the stream is reproducible, and
# the oracle needs the same substitution.
g++ -std=c++17 -O2 -fpermissive -D__forceinline=inline \
    -include cmath -include cstdlib -include cstring \
    -include "$HERE/parity_rand.h" \
    -I "$REF_SCRIPT/include" \
    -I "$REF_SCRIPT/dependencies/submodules/simple-2d-constraint-solver/include" \
    -I "$REF_SCRIPT/dependencies/submodules/csv-io/include" \
    -c "${REF_CORE:-/tmp/refcore}/src/combustion_chamber.cpp" -o "$WORK/combustion_chamber_parity.o"

g++ -std=c++17 -O2 -fpermissive -D__forceinline=inline \
    -include cmath -include cstdlib -include cstring \
    -include "$HERE/parity_rand.h" \
    -I "$REF_SCRIPT/include" \
    -I "$REF_SCRIPT/dependencies/submodules/simple-2d-constraint-solver/include" \
    -I "$REF_SCRIPT/dependencies/submodules/csv-io/include" \
    -c "${REF_CORE:-/tmp/refcore}/src/synthesizer.cpp" -o "$WORK/synthesizer_parity.o"

g++ -std=c++17 -O2 -fpermissive -D__forceinline=inline \
    -include cmath -include cstdlib -include cstring \
    -I "$REF_SCRIPT/include" \
    -I "$REF_SCRIPT/scripting/include" \
    -I "$REF_SCRIPT/dependencies/submodules" \
    -I "$REF_SCRIPT/dependencies/submodules/piranha/test" \
    -I "$REF_SCRIPT/gtest_shim" \
    -I "$REF_SCRIPT/dependencies/submodules/simple-2d-constraint-solver/include" \
    -I "$REF_SCRIPT/dependencies/submodules/csv-io/include" \
    -Wl,--allow-multiple-definition \
    "$HERE/script_reference.cpp" "$HERE/parity_rand.cpp" \
    "$WORK/combustion_chamber_parity.o" "$WORK/synthesizer_parity.o" \
    -Wl,--start-group "$REF_SCRIPT/librefscript.a" "${REF_CORE:-/tmp/refcore}/librefcore.a" -Wl,--end-group \
    -o "$WORK/script_reference"
