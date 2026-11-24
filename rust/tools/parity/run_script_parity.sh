#!/usr/bin/env bash
# `.mr` front-end parity: the Rust parser against the original piranha IR.
#
#   ./run_script_parity.sh [--keep]
#
# Two gates:
#
#   IR     every script in `es/` and `assets/` is parsed by the Rust front end,
#          the dump is read back by the C++ oracle into real piranha objects,
#          and the oracle re-emits it. The two trees have to match exactly
#          (`compare_ir.py`), which pins the Rust parser to the original IR.
#
#   RUN    `assets/main.mr` is compiled and executed through the original
#          resolve/validate/build/execute pipeline from the Rust dump, and the
#          resulting engine is dumped together with a deterministic trace. This
#          is the end-to-end target the Rust port has to reproduce once the
#          node bindings land.
#
# Requires the staged reference at $REF_SCRIPT (default /tmp/refscript).
set -euo pipefail

KEEP="${1:-}"
REF_SCRIPT="${REF_SCRIPT:-/tmp/refscript}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="$(cd "$HERE/../.." && pwd)"
REPO_ROOT="$(cd "$RUST_DIR/.." && pwd)"
WORK="${WORK:-/tmp/engine-sim-script-parity}"

export CARGO_HOME="${CARGO_HOME:-/tmp/cargo-home}"
export PATH="/tmp/rust-toolchain/bin:$PATH"

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
    -I "$REF_SCRIPT/dependencies/submodules/simple-2d-constraint-solver/include" \
    -I "$REF_SCRIPT/dependencies/submodules/csv-io/include" \
    -Wl,--allow-multiple-definition \
    "$HERE/script_reference.cpp" "$HERE/parity_rand.cpp" \
    "$WORK/combustion_chamber_parity.o" "$WORK/synthesizer_parity.o" \
    -Wl,--start-group "$REF_SCRIPT/librefscript.a" "${REF_CORE:-/tmp/refcore}/librefcore.a" -Wl,--end-group \
    -o "$WORK/script_reference"

echo "== building the rust front end =="
(cd "$RUST_DIR" && cargo build --release --example ir_dump --example corpus_check >/dev/null)

IR_DUMP="$RUST_DIR/target/release/examples/ir_dump"

# The M2 target: what the original's resolve + validate decided, per port and
# per attribute. Recorded rather than gated -- the Rust side cannot reproduce it
# yet, and a target nothing compares against is still what M2 has to hit.
if [ "${RECORD_RESOLVED:-0}" = "1" ]; then
    echo "== recording the resolved-type fixtures =="
    for entry in es/engine_sim.mr assets/main.mr; do
        name=$(echo "$entry" | tr '/' '_' | sed 's/\.mr$//')
        (cd "$REPO_ROOT" && "$IR_DUMP" "$entry" > "$WORK/record.ir")
        "$WORK/script_reference" --resolved "$entry" "$WORK/record.ir" \
            | sed "s|$REPO_ROOT/||g" \
            > "$RUST_DIR/crates/engine-sim-script/fixtures/${name}.resolved"
        echo "  $entry -> $(wc -l < "$RUST_DIR/crates/engine-sim-script/fixtures/${name}.resolved") rows"
    done
    exit 0
fi

echo "== gate IR: parse every script =="
"$RUST_DIR/target/release/examples/corpus_check" "$REPO_ROOT/es" "$REPO_ROOT/assets"

FILES=$(find "$REPO_ROOT/es" "$REPO_ROOT/assets" -name '*.mr' | sort)
COUNT=0
MISMATCHES=0

for file in $FILES; do
    relative="${file#$REPO_ROOT/}"
    (
        cd "$REPO_ROOT"
        "$IR_DUMP" "$relative" > "$WORK/rust.ir" 2> "$WORK/rust.err"
    ) || { echo "FAIL $relative (rust): $(cat "$WORK/rust.err")"; MISMATCHES=$((MISMATCHES + 1)); continue; }

    "$WORK/script_reference" --ir "$WORK/rust.ir" > "$WORK/cpp.ir" 2> "$WORK/cpp.err" \
        || { echo "FAIL $relative (oracle): $(head -3 "$WORK/cpp.err")"; MISMATCHES=$((MISMATCHES + 1)); continue; }

    if ! python3 "$HERE/compare_ir.py" "$WORK/rust.ir" "$WORK/cpp.ir" --quiet > "$WORK/diff.txt" 2>&1; then
        echo "FAIL $relative: $(head -3 "$WORK/diff.txt" | tr '\n' ' ')"
        MISMATCHES=$((MISMATCHES + 1))
        continue
    fi

    # The oracle also runs the original resolve/validate stages on the graph the
    # Rust parser produced: types, ports, aliases and defaults all have to hold
    # up under the original rules.
    if ! "$WORK/script_reference" --check "$relative" "$WORK/rust.ir" > "$WORK/check.txt" 2>&1; then
        echo "FAIL $relative (validate): $(head -3 "$WORK/check.txt" | tr '\n' ' ')"
        MISMATCHES=$((MISMATCHES + 1))
        continue
    fi

    COUNT=$((COUNT + 1))
done

echo "-- $COUNT files round-tripped and validated, $MISMATCHES mismatches"

echo "== gate RUN: execute assets/main.mr through the original pipeline =="
"$IR_DUMP" assets/main.mr > "$WORK/main.ir"

if [ -f "$REPO_ROOT/rust/crates/engine-sim-script/fixtures/main_engine.trace" ] && \
   [ "${UPDATE_FIXTURE:-}" != "1" ]; then
    "$WORK/script_reference" --run assets/main.mr "$WORK/main.ir" 400 > "$WORK/main.trace" 2> "$WORK/main.err" \
        || { echo "FAIL: the oracle could not run assets/main.mr"; head -20 "$WORK/main.err"; exit 1; }

    if diff -q "$REPO_ROOT/rust/crates/engine-sim-script/fixtures/main_engine.trace" "$WORK/main.trace" >/dev/null; then
        echo "PASS: $(( $(wc -l < "$WORK/main.trace") )) engine/trace checkpoints match the recorded fixture"
    else
        echo "FAIL: engine trace differs from the recorded fixture"
        diff "$REPO_ROOT/rust/crates/engine-sim-script/fixtures/main_engine.trace" "$WORK/main.trace" | head -20
        exit 1
    fi
else
    mkdir -p "$REPO_ROOT/rust/crates/engine-sim-script/fixtures"
    "$WORK/script_reference" --run assets/main.mr "$WORK/main.ir" 400 \
        > "$REPO_ROOT/rust/crates/engine-sim-script/fixtures/main_engine.trace" 2> "$WORK/main.err" \
        || { echo "FAIL: the oracle could not run assets/main.mr"; head -30 "$WORK/main.err"; exit 1; }
    echo "recorded rust/crates/engine-sim-script/fixtures/main_engine.trace"
fi

grep -E '^(E|0),' "$REPO_ROOT/rust/crates/engine-sim-script/fixtures/main_engine.trace" | head -8

[ "$KEEP" = "--keep" ] || rm -rf "$WORK"

if [ "$MISMATCHES" -ne 0 ]; then
    echo "FAIL: $MISMATCHES script(s) did not round-trip"
    exit 1
fi

echo "PASS"
