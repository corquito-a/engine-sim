#!/usr/bin/env bash
# Real-time factor: how much audio the port renders per second of wall clock,
# next to the original asking the same question.
#
#   ./measure_performance.sh [--seconds 2]
#
# The parity gates say the two render the same samples; this says whether either
# can render them as fast as they are played. Per asset it runs
#
#   · the port through `engine-sim-cli`, which is the application's frame loop
#     (`start_frame`/`simulate_step`/`end_frame`, then a synthesizer block and
#     `read_audio_output`), quiet, with the WAV going to /dev/null;
#   · the original through `script_reference --bench`, which is the same loop with
#     every `emit` taken out of `traceFrames` -- no formatting, no output, so the
#     number is the simulation and the synthesizer, not `printf`.
#
# Both stop by the same rule (the CLI's: once `seconds * 44100` samples exist, or
# four times the frames), so they render the same audio. Real-time factor is
# audio seconds over wall seconds; 1.0 means exactly as fast as it plays.
#
# This is a measurement, not a benchmark: it fails only below `PERF_FLOOR` (a
# three-or-more-times regression), because a shared machine's wall clock is not
# something a gate should be strict about.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="$(cd "$HERE/../.." && pwd)"
REPO_ROOT="$(cd "$RUST_DIR/.." && pwd)"
WORK="${WORK:-/tmp/engine-sim-performance}"

SECONDS_TO_RENDER="${PERF_SECONDS:-2}"
PERF_FLOOR="${PERF_FLOOR:-0.25}"

while [ $# -gt 0 ]; do
    case "$1" in
        --seconds) SECONDS_TO_RENDER="$2"; shift 2 ;;
        *) echo "unknown argument '$1'" >&2; exit 2 ;;
    esac
done

FRAMES=$(( SECONDS_TO_RENDER * 60 ))

echo "== building =="
cargo build --release --manifest-path "$RUST_DIR/Cargo.toml" -p engine-sim-cli -p engine-sim-script \
    --examples --quiet
CLI="$RUST_DIR/target/release/engine-sim-cli"
IR_DUMP="$RUST_DIR/target/release/examples/ir_dump"

WORK="$WORK" bash "$HERE/build_script_reference.sh" > /dev/null
REFERENCE="$WORK/script_reference"

mkdir -p "$WORK"

now_ms() { date +%s%N; }

# Both sides report their own loop, and the shell times the whole process: the
# difference is the setup (parsing the script, building the engine, recording the
# impulse responses), which is not rendering and should not be charged to the
# real-time factor. Prints "<loop ms> <setup ms>".
measure_port() {
    local script="$1"
    local start end total output loop_ms
    start=$(now_ms)
    output=$("$CLI" "$script" --seconds "$SECONDS_TO_RENDER" --fps 60 \
        --wav /dev/null --quiet --timing 2>&1 >/dev/null)
    end=$(now_ms)
    total=$(( (end - start) / 1000000 ))
    loop_ms=$(echo "$output" | awk -F, '/^timing,/ { printf "%d", $4 * 1000 }')
    if [ -z "$loop_ms" ]; then loop_ms=0; fi
    echo "$loop_ms $(( total - loop_ms ))"
}

# `bench,<frames>,<samples>,<wall seconds>`: the same numbers, from the original.
measure_reference() {
    local script="$1" ir="$2"
    local start end total output loop_ms
    "$IR_DUMP" "$script" > "$ir"
    start=$(now_ms)
    output=$("$REFERENCE" --bench "$script" "$ir" "$FRAMES")
    end=$(now_ms)
    total=$(( (end - start) / 1000000 ))
    loop_ms=$(echo "$output" | awk -F, '/^bench,/ { printf "%d", $4 * 1000 }')
    if [ -z "$loop_ms" ]; then loop_ms=0; fi
    echo "$loop_ms $(( total - loop_ms ))"
}

printf '%-44s %10s %10s %10s %10s %8s %8s\n' \
    "asset" "port loop" "orig loop" "port setup" "orig setup" "port RTF" "orig RTF"

worst_asset=""
worst_rtf=99
scripts=("$REPO_ROOT"/assets/main.mr)
while IFS= read -r wrapper; do
    scripts+=("$wrapper")
done < <(find "$HERE/engines" -name 'engines_*.mr' | sort)

total_port=0
total_reference=0
count=0

for script in "${scripts[@]}"; do
    read -r port_ms port_setup_ms <<< "$(measure_port "$script")"
    read -r reference_ms reference_setup_ms <<< "$(measure_reference "$script" "$WORK/bench.ir")"

    if [ "${port_ms:-0}" -eq 0 ]; then port_ms=1; fi
    if [ "${reference_ms:-0}" -eq 0 ]; then reference_ms=1; fi

    port_rtf=$(awk -v s="$SECONDS_TO_RENDER" -v ms="$port_ms" 'BEGIN { printf "%.2f", s * 1000.0 / ms }')
    reference_rtf=$(awk -v s="$SECONDS_TO_RENDER" -v ms="$reference_ms" 'BEGIN { printf "%.2f", s * 1000.0 / ms }')

    printf '%-44s %9sms %9sms %9sms %9sms %8s %8s\n' \
        "$(basename "$script")" "$port_ms" "$reference_ms" \
        "$port_setup_ms" "$reference_setup_ms" "$port_rtf" "$reference_rtf"

    total_port=$((total_port + port_ms))
    total_reference=$((total_reference + reference_ms))
    count=$((count + 1))

    if awk -v a="$port_rtf" -v b="$worst_rtf" 'BEGIN { exit !(a < b) }'; then
        worst_rtf="$port_rtf"
        worst_asset="$(basename "$script")"
    fi
done

port_rtf=$(awk -v s="$SECONDS_TO_RENDER" -v c="$count" -v ms="$total_port" \
    'BEGIN { printf "%.2f", (s * c) * 1000.0 / ms }')
reference_rtf=$(awk -v s="$SECONDS_TO_RENDER" -v c="$count" -v ms="$total_reference" \
    'BEGIN { printf "%.2f", (s * c) * 1000.0 / ms }')

echo
echo "-- $count assets, $SECONDS_TO_RENDER s of audio each"
echo "-- port $port_rtf x real time, original $reference_rtf x real time"
echo "-- slowest asset: $worst_asset at $worst_rtf x real time"

if awk -v a="$worst_rtf" -v f="$PERF_FLOOR" 'BEGIN { exit !(a < f) }'; then
    echo "FAIL: $worst_asset renders at $worst_rtf x real time, below the $PERF_FLOOR floor"
    exit 1
fi

echo "PASS: every asset renders at or above $PERF_FLOOR x real time"
