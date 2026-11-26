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
#   RUN    `assets/main.mr` is built and simulated twice: through the original
#          resolve/validate/build/execute/generate pipeline, and through the
#          port's own front end into the port's own `engine-sim-core` engine.
#          Both print the 98-line engine header and 16 166 per-frame checkpoints
#          of physics, gas dynamics, synthesizer input and rendered audio, and
#          all of it has to match -- the reference against the recorded fixture,
#          and the port against the reference.
#
#   DRIVEN the same script with the keys pressed: `PARITY_IGNITION=1`, the
#          starter held for one second and released, the throttle wide open, and
#          20 000 frames instead of 400 -- two seconds of an engine that is
#          actually running. The RUN gate only ever turns the engine over, so it
#          cannot see when a cylinder fires; this one can, because every firing
#          has to land on the same crank angle on both sides.
#
#   DRIVE  two scripts in `tools/parity/engines/`, driven in first gear with the
#          clutch engaged: the Honda wrapper, which publishes the bike it came
#          from, and an engine built straight from an asset's engine node, which
#          publishes neither. The engine's torque has to move the script's
#          vehicle through the script's transmission in the first case and the
#          application's pair in the second, so `loadScript`'s fallback -- the
#          gear ratios, the clutch's limit, the car's mass and drag -- reaches
#          the trace either way.
#
#   OPTIMIZE `NodeProgram::optimize` -- constant folding plus dead-node pruning --
#          which the application never calls and the port therefore does not
#          have. Every script that builds an engine, and every execute fixture,
#          has to run identically with and without it, which is what makes
#          leaving it out safe rather than merely convenient.
#
#   OPTIMIZE `NodeProgram::optimize` -- constant folding plus dead-node pruning --
#          which the application never calls and the port therefore does not
#          have. Every script that builds an engine, and every execute fixture,
#          has to run identically with and without it, which is what makes
#          leaving it out safe rather than merely convenient.
#
#   RUNNING every engine asset with the keys pressed: ignition on, the starter
#          released after 0.3 s, wide open, free-revving, 8 000 frames -- all
#          20 engines rather than the three `DRIVE` takes to two seconds.
#
#   GRAPH  every script in `tools/parity/graph/` is built into a node graph by
#          the original (`IrCompilationUnit::build`) and compared against the
#          recorded structure. The Rust node graph is what has to reproduce it.
#
#   VALUES the same fixtures in `tools/parity/values/`, one layer up: every
#          port's value read through `fullCompute` after the original has built,
#          initialized and executed the graph.
#
#   EXECUTE `tools/parity/execute/` runs the graph through `NodeProgram` and
#          compares what the nodes that write actually wrote, and whether the
#          run failed.
#
#   OBJECTS `--objects` and `--settings` on every script in `assets/`: the objects
#          the data nodes generate and the application settings the script
#          writes, byte for byte. This is the gate that reaches M4.
#
#   WAVE   every `.wav` in `es/sound-library/`: the port's RIFF decoder against
#          `ysWindowsAudioWaveFile`'s parse -- channels, rate, bits, frame count,
#          and an FNV-1a hash over every decoded sample. This is what the audio in
#          the traces below actually runs on.
#
#   ENGINES every engine under `assets/engines/` that builds one, through
#          `tools/parity/engines/` wrappers: the port's engine against the
#          original's, 400 steps of the same per-frame checkpoints, byte for
#          byte. Twenty engines -- inline fours to a V12 to a nine-cylinder
#          radial -- so the parts `buildEngine` wires together (banks, rods and
#          journals, camshafts, valvetrains, exhausts, intake, ignition, fuel)
#          are exercised far beyond `assets/main.mr`'s one configuration.
#
#   ENGINE `--engine` on `assets/main.mr`: the engine the script configures --
#          its name and counts, its curves, and every part's parameters, 96
#          lines -- byte for byte, against both the reference and the recorded
#          `tools/parity/engine/main.engine`. This is what `EngineNode::buildEngine`
#          decides; the simulation trace below is what the engine then does.
#
# Requires the staged reference at $REF_SCRIPT (default /tmp/refscript).
#
# Not a gate: `measure_performance.sh` is the same tree asked how *fast* it
# renders (real-time factor, port against original). Nothing here fails on a
# clock; see `docs/mr-interpreter-plan.md` §8.7.
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

"$HERE/build_script_reference.sh"

echo "== building the rust front end =="
(cd "$RUST_DIR" && cargo build --release --example ir_dump --example corpus_check --example graph_dump --example script_trace >/dev/null)

IR_DUMP="$RUST_DIR/target/release/examples/ir_dump"
GRAPH_DUMP="$RUST_DIR/target/release/examples/graph_dump"
SCRIPT_TRACE="$RUST_DIR/target/release/examples/script_trace"

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

# The engine header `assets/main.mr` configures, recorded from
# `script_reference --engine`. This is M4's first target: the port has to build
# the same engine -- the same name, counts, fuel and parameter values -- before
# any of the simulation trace below can match.
if [ "${RECORD_ENGINE:-0}" = "1" ]; then
    echo "== recording the engine header =="
    "$IR_DUMP" assets/main.mr > "$WORK/main.ir"
    "$WORK/script_reference" --engine assets/main.mr "$WORK/main.ir" \
        > "$HERE/engine/main.engine" 2> "$WORK/engine.err" \
        || { echo "FAIL: the oracle could not configure the engine"; head -20 "$WORK/engine.err"; exit 1; }
    echo "  assets/main.mr -> $(wc -l < "$HERE/engine/main.engine") lines"
    exit 0
fi

# The error catalogue's expectations: one `.errors` file per broken script in
# `negative/`, recorded from `script_reference --errors`.
if [ "${RECORD_ERRORS:-0}" = "1" ]; then
    echo "== recording the error fixtures =="
    for script in "$HERE"/negative/*.mr "$HERE"/negative/*/root.mr; do
        [ -f "$script" ] || continue
        (cd "$REPO_ROOT" && "$IR_DUMP" "$script" > "$WORK/negative.ir")
        "$WORK/script_reference" --errors "$script" "$WORK/negative.ir" 2>/dev/null \
            > "${script%.mr}.errors" || true
        echo "  $(basename "$(dirname "$script")")/$(basename "$script"): $(tr '\n' ' ' < "${script%.mr}.errors")"
    done
    exit 0
fi

# How much of M2 is still open, measured against the recorded reference dump.
# Informational: it cannot pass until resolution and type checking exist.
if [ "${RESOLVED_GAP:-0}" = "1" ]; then
    echo "== resolved-type gap (informational) =="
    (cd "$RUST_DIR" && cargo build --release --example resolved_dump >/dev/null)
    for entry in es/engine_sim.mr assets/main.mr; do
        name=$(echo "$entry" | tr '/' '_' | sed 's/\.mr$//')
        "$RUST_DIR/target/release/examples/resolved_dump" "$entry" > "$WORK/rust.resolved"
        echo "-- $entry"
        python3 "$HERE/compare_resolved.py" "$WORK/rust.resolved" \
            "$RUST_DIR/crates/engine-sim-script/fixtures/${name}.resolved" || true
    done
    exit 0
fi

echo "== gate IR: parse every script =="
# `tools/parity/engines/` is in here too: the fixtures the DRIVE gate runs are
# scripts like any other, and the original's resolve/validate stages are what
# caught them writing a vehicle the original cannot resolve.
"$RUST_DIR/target/release/examples/corpus_check" "$REPO_ROOT/es" "$REPO_ROOT/assets" \
    "$REPO_ROOT/rust/tools/parity/engines"

FILES=$(find "$REPO_ROOT/es" "$REPO_ROOT/assets" "$REPO_ROOT/rust/tools/parity/engines" -name '*.mr' | sort)
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

echo "== gate RUN: build and simulate assets/main.mr, both sides =="
"$IR_DUMP" assets/main.mr > "$WORK/main.ir"

# The point of the port: the script's engine, built by the port's own front end
# and simulated by the port's own `engine-sim-core`. It prints the same 98-line
# header and the same per-frame fields as the reference -- physics, gas dynamics,
# synthesizer input and rendered audio -- so the whole 16 265-line trace is one
# diff.
"$SCRIPT_TRACE" assets/main.mr > "$WORK/main.port.trace" 2> "$WORK/main.port.err" \
    || { echo "FAIL: the port could not run assets/main.mr"; head -20 "$WORK/main.port.err"; exit 1; }

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

    if diff -q "$WORK/main.trace" "$WORK/main.port.trace" >/dev/null; then
        echo "PASS: the port's own engine reproduces all $(( $(wc -l < "$WORK/main.port.trace") )) lines of it"
    else
        echo "FAIL: the port's own engine does not match the reference"
        diff "$WORK/main.trace" "$WORK/main.port.trace" | head -20
        exit 1
    fi
else
    mkdir -p "$REPO_ROOT/rust/crates/engine-sim-script/fixtures"
    "$WORK/script_reference" --run assets/main.mr "$WORK/main.ir" 400 \
        > "$REPO_ROOT/rust/crates/engine-sim-script/fixtures/main_engine.trace" 2> "$WORK/main.err" \
        || { echo "FAIL: the oracle could not run assets/main.mr"; head -30 "$WORK/main.err"; exit 1; }
    echo "recorded rust/crates/engine-sim-script/fixtures/main_engine.trace"
fi

# The RUN gate cranks: the starter is held for all 400 frames and the ignition is
# off, which is the application before `S` and `A` are pressed. Nothing in it
# fires, so nothing in it can catch a difference in combustion. This gate presses
# the keys -- ignition on, starter held for a second and released, throttle wide
# open -- and runs twenty thousand frames, two seconds of a running engine, where
# every firing has to land on the same crank angle on both sides. `PARITY_THROTTLE`
# is the engine's *speed control*, the channel the application's keys drive, and
# one is wide open.
echo "== gate DRIVEN: a running engine, both sides =="
DRIVEN_STEPS=20000

PARITY_STEPS="$DRIVEN_STEPS" PARITY_THROTTLE=1 PARITY_STARTER_SECONDS=1 PARITY_IGNITION=1 \
    "$SCRIPT_TRACE" assets/main.mr > "$WORK/driven.port.trace" 2> "$WORK/driven.port.err" \
    || { echo "FAIL: the port could not drive assets/main.mr"; head -20 "$WORK/driven.port.err"; exit 1; }

PARITY_STEPS="$DRIVEN_STEPS" PARITY_THROTTLE=1 PARITY_STARTER_SECONDS=1 PARITY_IGNITION=1 \
    "$WORK/script_reference" --run assets/main.mr "$WORK/main.ir" "$DRIVEN_STEPS" \
        > "$WORK/driven.trace" 2> "$WORK/driven.err" \
    || { echo "FAIL: the oracle could not drive assets/main.mr"; head -20 "$WORK/driven.err"; exit 1; }

if diff -q "$WORK/driven.trace" "$WORK/driven.port.trace" >/dev/null; then
    echo "PASS: $(( $(wc -l < "$WORK/driven.trace") )) checkpoints of a running engine are identical"
else
    echo "FAIL: a running engine diverges from the original"
    diff "$WORK/driven.trace" "$WORK/driven.port.trace" | head -20
    exit 1
fi

# The drivetrain. Both branches of `EngineSimApplication::loadScript`'s rule have
# to be gated, because each one hides the other: a script that publishes a vehicle
# and a transmission has to move *that* car, and a script that publishes neither
# has to move the application's. Either comparison is also the only place the gear
# ratios can be seen at all -- `Transmission` keeps them protected -- and they only
# reach the trace when the engine is in gear, so both runs engage first and let the
# clutch out at wide-open throttle.
echo "== gate DRIVE: the script's own vehicle and transmission =="
drive_one() {
    local name="$1" script="$2" ir="$3"
    local steps="${4:-$DRIVEN_STEPS}"

    PARITY_STEPS="$steps" PARITY_THROTTLE=1 PARITY_IGNITION=1 \
        PARITY_STARTER_SECONDS="${DRIVE_STARTER:-1}" \
        ${DRIVE_GEAR:+PARITY_GEAR=$DRIVE_GEAR} ${DRIVE_CLUTCH:+PARITY_CLUTCH=$DRIVE_CLUTCH} \
        "$SCRIPT_TRACE" "$script" > "$WORK/$name.port.trace" 2> "$WORK/$name.port.err" \
        || { echo "FAIL: the port could not drive $script"; head -20 "$WORK/$name.port.err"; exit 1; }

    PARITY_STEPS="$steps" PARITY_THROTTLE=1 PARITY_IGNITION=1 \
        PARITY_STARTER_SECONDS="${DRIVE_STARTER:-1}" \
        ${DRIVE_GEAR:+PARITY_GEAR=$DRIVE_GEAR} ${DRIVE_CLUTCH:+PARITY_CLUTCH=$DRIVE_CLUTCH} \
        "$WORK/script_reference" --run "$script" "$ir" "$steps" \
            > "$WORK/$name.trace" 2> "$WORK/$name.err" \
        || { echo "FAIL: the oracle could not drive $script"; head -20 "$WORK/$name.err"; exit 1; }

    if diff -q "$WORK/$name.trace" "$WORK/$name.port.trace" >/dev/null; then
        echo "PASS: $(wc -l < "$WORK/$name.trace") checkpoints, $script"
    else
        echo "FAIL: $script diverges in the drivetrain"
        diff "$WORK/$name.trace" "$WORK/$name.port.trace" | head -20
        exit 1
    fi
}

# From the script: the bike's 500 kg, five ratios and 50 ft-lb clutch.
DRIVE_SCRIPT="$HERE/engines/engines_atg-video-1_01_honda_trx520.mr"
"$IR_DUMP" "$DRIVE_SCRIPT" > "$WORK/drive_from_script.ir"
drive_one drive_from_script "$DRIVE_SCRIPT" "$WORK/drive_from_script.ir"

# From the script as far as it goes: a 900 kg car of its own, the application's
# gearbox behind it. `loadScript` decides per object, so this run is what catches a
# port that swapped both or neither.
DRIVE_MIXED="$HERE/engines/vehicle_without_transmission.mr"
"$IR_DUMP" "$DRIVE_MIXED" > "$WORK/drive_mixed.ir"
drive_one drive_mixed "$DRIVE_MIXED" "$WORK/drive_mixed.ir"

# From the application: the same engine with no vehicle and no transmission.
DRIVE_FALLBACK="$HERE/engines/drivetrain_from_application.mr"
"$IR_DUMP" "$DRIVE_FALLBACK" > "$WORK/drive_fallback.ir"
drive_one drive_fallback "$DRIVE_FALLBACK" "$WORK/drive_fallback.ir"

# Every engine asset, running. `ENGINES` runs all 22 with the starter held and the
# ignition off -- one firing cycle short of an engine -- and `DRIVE` runs three of
# them for two seconds. This one gives every engine a spark and a wide-open
# throttle for 6 000 frames (the starter lets go after a second), so a wrong
# firing order, a wrong advance curve or a `rand()` consumed out of order has to
# show up on the engine it belongs to rather than only on the three in `DRIVE`.
# Free-revving, because in gear an engine that starts from rest spends the whole
# window being dragged round at cranking speed: `chamber_lit` stays zero and the
# gate would be measuring the clutch. Eleven of these fire inside 0.8 s of audio
# and nine do not -- the Merlin's starter cannot turn it at all in that time -- and
# both states are worth comparing, so the count is reported rather than required.
echo "== gate RUNNING: every engine asset, cranking to running =="
RUNNING_STEPS=8000
DRIVE_STARTER=0.3
DRIVE_GEAR=
DRIVE_CLUTCH=
running_count=0
running_fired=0

for wrapper in "$HERE"/engines/engines_*.mr; do
    "$IR_DUMP" "$wrapper" > "$WORK/running.ir" 2>/dev/null || continue
    drive_one "running_$(basename "$wrapper" .mr)" "$wrapper" "$WORK/running.ir" "$RUNNING_STEPS"
    running_count=$((running_count + 1))
    trace="$WORK/running_$(basename "$wrapper" .mr).trace"
    # one process, not a pipe: under `pipefail` a satisfied `grep -q` is a SIGPIPE
    # for the first grep and the pipeline reports failure
    if awk -F, '$2 == "chamber_lit" && $3 != "0000000000000000" { found = 1 } END { exit !found }' "$trace"; then
        running_fired=$((running_fired + 1))
    fi
done

unset DRIVE_STARTER DRIVE_GEAR DRIVE_CLUTCH

echo "-- $running_count engines, every checkpoint identical; $running_fired of them fire inside the window"

# The node graph the IR generates: every fixture, plus every script under
# `assets/`, against a live reference (`--record` rewrites the fixtures).
echo "== gate GRAPH: the assembled node graph of every graph fixture =="
python3 "$HERE/run_graph.py" || exit 1

# And the layer above it: the values that graph computes.
echo "== gate VALUES: what every port computes =="
python3 "$HERE/run_graph.py" --values || exit 1

# And the objects the data nodes generate -- curves (`function`, `fuel`,
# `piston`, `connecting_rod`, `crankshaft`, `vehicle`) and the structure the
# actions build (`add_cylinder`, `set_cylinder_head`, `add_rod_journal`).
echo "== gate OBJECTS and SETTINGS: the objects and settings a script makes =="
objects_failures=0
objects_scripts=0
while IFS= read -r script; do
    "$IR_DUMP" "$script" > "$WORK/objects.ir" 2>/dev/null || continue
    objects_scripts=$((objects_scripts + 1))

    "$WORK/script_reference" --objects "$script" "$WORK/objects.ir" > "$WORK/objects.ref" 2>"$WORK/objects.err" \
        || { echo "FAIL: the reference could not run $script"; head -5 "$WORK/objects.err"; exit 1; }

    "$GRAPH_DUMP" --objects "$script" > "$WORK/objects.port" 2>/dev/null

    if ! diff -q "$WORK/objects.ref" "$WORK/objects.port" > /dev/null; then
        echo "FAIL: $script generates different objects"
        diff "$WORK/objects.ref" "$WORK/objects.port" | head -10
        objects_failures=$((objects_failures + 1))
    fi

    # The application settings are the one piece of script state no object holds.
    "$WORK/script_reference" --settings "$script" "$WORK/objects.ir" > "$WORK/settings.ref" 2>/dev/null \
        || { echo "FAIL: the reference could not run $script (settings)"; exit 1; }

    "$GRAPH_DUMP" --settings "$script" > "$WORK/settings.port" 2>/dev/null

    if ! diff -q "$WORK/settings.ref" "$WORK/settings.port" > /dev/null; then
        echo "FAIL: $script writes different settings"
        diff "$WORK/settings.ref" "$WORK/settings.port" | head -10
        objects_failures=$((objects_failures + 1))
    fi
done < <(find assets -name '*.mr' | sort)

if [ "$objects_failures" -ne 0 ]; then
    echo "FAIL: $objects_failures of $objects_scripts scripts differ"
    exit 1
fi
echo "-- $objects_scripts scripts, every generated object and every settings line identical"

# And above those: the engine header. `assets/main.mr` is the only script that
# builds one, and the header is what `buildEngine` decided -- the name, the
# counts, the curves and every part's parameters -- before anything is simulated.
echo "== gate ENGINE: the engine a script configures =="
"$IR_DUMP" assets/main.mr > "$WORK/main.ir"
"$WORK/script_reference" --engine assets/main.mr "$WORK/main.ir" > "$WORK/engine.ref" 2> "$WORK/engine.err" \
    || { echo "FAIL: the reference could not configure the engine"; head -20 "$WORK/engine.err"; exit 1; }
"$GRAPH_DUMP" --engine assets/main.mr > "$WORK/engine.port" 2> "$WORK/engine.port.err" \
    || { echo "FAIL: the port could not configure the engine"; head -20 "$WORK/engine.port.err"; exit 1; }

if ! diff -q "$WORK/engine.ref" "$WORK/engine.port" > /dev/null; then
    echo "FAIL: the port's engine header differs from the reference"
    diff "$WORK/engine.ref" "$WORK/engine.port" | head -20
    exit 1
fi

if ! diff -q "$WORK/engine.ref" "$HERE/engine/main.engine" > /dev/null; then
    echo "FAIL: the recorded engine header is stale"
    diff "$HERE/engine/main.engine" "$WORK/engine.ref" | head -20
    exit 1
fi
echo "PASS: $(( $(wc -l < "$WORK/engine.ref") )) engine header lines identical, and the recorded fixture still holds"

# The optimizer. `NodeProgram::optimize` folds constant operands into literals and
# prunes actionless nodes nothing reads; the application never calls it, so the
# port does not have it. That is only safe if it changes nothing the application
# can see, and this measures the claim instead of assuming it: every script that
# builds an engine has to produce the same 98-line header with and without it, and
# `assets/main.mr` the same trace.
echo "== gate OPTIMIZE: the optimizer the application never runs =="
optimize_failures=0
optimize_runs=0

# The execute fixtures are piranha's own, so they need piranha's rules; a fixture
# with a `.stdin` beside it (`reads.mr`) is fed it on both runs.
compare_optimized() {
    local script="$1" mode="$2" ir="$3" name="$4" rules="${5:-engine-sim}"
    local stdin_path="${script%.*}.stdin"
    [ -f "$stdin_path" ] || stdin_path=/dev/null

    # A run that is *meant* to fail (`throws.mr`) is a result, not an error, so the
    # exit status is compared rather than required to be zero.
    local plain_status=0 folded_status=0

    env "PARITY_RULES=$rules" "$WORK/script_reference" "$mode" "$script" "$ir" \
        > "$WORK/opt.plain" 2> "$WORK/opt.err" < "$stdin_path" || plain_status=$?

    env "PARITY_RULES=$rules" PARITY_OPTIMIZE=1 "$WORK/script_reference" "$mode" "$script" "$ir" \
        > "$WORK/opt.folded" 2> "$WORK/opt.err" < "$stdin_path" || folded_status=$?

    if [ "$plain_status" -ne "$folded_status" ]; then
        echo "FAIL: $name exits $plain_status without the optimizer and $folded_status with it"
        optimize_failures=$((optimize_failures + 1))
        return
    fi

    if ! diff -q "$WORK/opt.plain" "$WORK/opt.folded" > /dev/null; then
        echo "FAIL: $name changes when the optimizer runs"
        diff "$WORK/opt.plain" "$WORK/opt.folded" | head -10
        optimize_failures=$((optimize_failures + 1))
        return
    fi

    optimize_runs=$((optimize_runs + 1))
}

compare_optimized assets/main.mr --engine "$WORK/main.ir" "assets/main.mr"
compare_optimized assets/main.mr --run "$WORK/main.ir" "assets/main.mr's trace"

while IFS= read -r wrapper; do
    "$IR_DUMP" "$wrapper" > "$WORK/opt.ir" 2>/dev/null || continue
    compare_optimized "$wrapper" --engine "$WORK/opt.ir" "$(basename "$wrapper")"
done < <(find "$HERE/engines" -name '*.mr' | sort)

for fixture in "$HERE"/execute/*.mr; do
    "$IR_DUMP" "$fixture" > "$WORK/opt.ir" 2>/dev/null || continue
    compare_optimized "$fixture" --execute "$WORK/opt.ir" "$(basename "$fixture")" test
done

if [ "$optimize_failures" -ne 0 ]; then
    echo "FAIL: the optimizer changes $optimize_failures run(s)"
    exit 1
fi
echo "PASS: $optimize_runs runs are identical with and without the optimizer"

# The audio the traces run on: the impulse responses, decoded both sides.
echo "== gate WAVE: the impulse responses =="
if [ ! -x "$RUST_DIR/target/release/examples/wave_dump" ]; then
    (cd "$RUST_DIR" && cargo build --release --example wave_dump >/dev/null)
fi
WAVE_DUMP="$RUST_DIR/target/release/examples/wave_dump"

wave_failures=0
wave_files=0
while IFS= read -r wave; do
    wave_files=$((wave_files + 1))

    "$WORK/script_reference" --wave "$wave" > "$WORK/wave.ref" 2> "$WORK/wave.err" \
        || { echo "FAIL: the reference could not read $wave"; wave_failures=$((wave_failures + 1)); continue; }

    "$WAVE_DUMP" "$wave" > "$WORK/wave.port" 2> "$WORK/wave.port.err" \
        || { echo "FAIL: the port could not read $wave: $(head -2 "$WORK/wave.port.err")"; wave_failures=$((wave_failures + 1)); continue; }

    if ! diff -q "$WORK/wave.ref" "$WORK/wave.port" > /dev/null; then
        echo "FAIL: $wave decodes differently"
        diff "$WORK/wave.ref" "$WORK/wave.port" | head -5
        wave_failures=$((wave_failures + 1))
    fi
done < <(find "$REPO_ROOT/es/sound-library" -name '*.wav' | sort)

if [ "$wave_failures" -ne 0 ]; then
    echo "FAIL: $wave_failures of $wave_files wave files differ"
    exit 1
fi
echo "PASS: $wave_files impulse responses, every sample identical"

# And the same trace for every engine in `assets/`. `assets/main.mr` only
# configures one engine; these reach the parts it does not -- two exhaust
# systems, five-valve heads, a VTEC valvetrain, a master-and-slave radial, a V12.
echo "== gate ENGINES: every engine asset, simulated both sides =="
engine_failures=0
engine_count=0
for wrapper in "$HERE"/engines/*.mr; do
    [ -f "$wrapper" ] || continue
    name=$(basename "$wrapper" .mr)

    "$SCRIPT_TRACE" "$wrapper" > "$WORK/engines.port" 2> "$WORK/engines.port.err" \
        || { echo "FAIL: the port could not run $name: $(head -3 "$WORK/engines.port.err")"; engine_failures=$((engine_failures + 1)); continue; }

    "$IR_DUMP" "$wrapper" > "$WORK/engines.ir" 2>/dev/null

    "$WORK/script_reference" --run "$wrapper" "$WORK/engines.ir" 400 > "$WORK/engines.ref" 2> "$WORK/engines.ref.err" \
        || { echo "FAIL: the reference could not run $name: $(head -3 "$WORK/engines.ref.err")"; engine_failures=$((engine_failures + 1)); continue; }

    if ! diff -q "$WORK/engines.ref" "$WORK/engines.port" > /dev/null; then
        echo "FAIL: $name differs from the reference"
        diff "$WORK/engines.ref" "$WORK/engines.port" | head -10
        engine_failures=$((engine_failures + 1))
        continue
    fi

    engine_count=$((engine_count + 1))
done

if [ "$engine_failures" -ne 0 ]; then
    echo "FAIL: $engine_failures of $((engine_count + engine_failures)) engines differ"
    exit 1
fi

echo "PASS: $engine_count engines, every checkpoint identical to the original"

# And above that: what a run writes, and whether it fails where the original
# fails.
echo "== gate EXECUTE: what a run writes =="
python3 "$HERE/run_graph.py" --execute || exit 1

[ "$KEEP" = "--keep" ] || rm -rf "$WORK"

if [ "$MISMATCHES" -ne 0 ]; then
    echo "FAIL: $MISMATCHES script(s) did not round-trip"
    exit 1
fi

echo "PASS"
