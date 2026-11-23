#!/usr/bin/env bash
# Stages a buildable copy of the original C++ simulation core and archives it
# as `librefcore.a` for the parity harnesses.
#
#   ./stage_reference.sh            # -> $REF_CORE (default /tmp/refcore)
#
# The upstream tree is MSVC-only as shipped, so this script applies
# `reference.patch` (five mechanical edits, none of which change arithmetic)
# and compiles every core translation unit, skipping the Win32/D3D app shell.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../../.." && pwd)"
REF_CORE="${REF_CORE:-/tmp/refcore}"
SUBMODULES="$REPO_ROOT/dependencies/submodules"

SCS_SRC="$(ls -d "$SUBMODULES"/simple-2d-constraint-solver 2>/dev/null || true)"
CSV_SRC="$(ls -d "$SUBMODULES"/csv-io 2>/dev/null || true)"

if [ -z "$SCS_SRC" ] || [ ! -d "$SCS_SRC/include" ] || [ ! -d "$CSV_SRC/include" ]; then
    echo "==> fetching submodules (simple-2d-constraint-solver, csv-io)"
    ( cd "$REPO_ROOT" && git submodule update --init --depth 1 \
        dependencies/submodules/simple-2d-constraint-solver \
        dependencies/submodules/csv-io )
    SCS_SRC="$SUBMODULES/simple-2d-constraint-solver"
    CSV_SRC="$SUBMODULES/csv-io"
fi

echo "==> staging reference tree in $REF_CORE"
rm -rf "$REF_CORE"
mkdir -p "$REF_CORE"
cp -r "$REPO_ROOT/include" "$REPO_ROOT/src" "$REF_CORE/"
patch -p1 -s -d "$REF_CORE" < "$HERE/reference.patch"

# The app shell (window, D3D renderer, UI clusters, gauges, .ysce loaders) is
# not needed to run the physics; everything else is.
EXCLUDE_RE='(_object|_cluster|_gauge|_display)$'
EXCLUDE_RE="$EXCLUDE_RE|^(main|gauge|engine_sim_application|engine_view|shaders"
EXCLUDE_RE="$EXCLUDE_RE|geometry_generator|oscilloscope|simulation_object"
EXCLUDE_RE="$EXCLUDE_RE|load_simulation_cluster|ui_manager|ui_element|ui_button"
EXCLUDE_RE="$EXCLUDE_RE|ui_math|ui_utilities|throttle_display)$"

CFLAGS=(-std=c++14 -O2 -fpermissive -D__forceinline=inline
        -include cmath -include cstdlib -include cstring
        -I "$REF_CORE/include"
        -I "$SCS_SRC/include"
        -I "$CSV_SRC/include")

# `include/scs.h` reaches the solver through a repo-relative path, so the staged
# tree needs the submodule layout (symlinked, not copied).
mkdir -p "$REF_CORE/dependencies/submodules"
ln -sfn "$SCS_SRC" "$REF_CORE/dependencies/submodules/simple-2d-constraint-solver"
ln -sfn "$CSV_SRC" "$REF_CORE/dependencies/submodules/csv-io"

mkdir -p "$REF_CORE/obj"

echo "==> compiling core (engine-sim)"
for src in "$REF_CORE"/src/*.cpp; do
    base="$(basename "$src" .cpp)"
    if echo "$base" | grep -qE "$EXCLUDE_RE"; then
        continue
    fi
    g++ "${CFLAGS[@]}" -c "$src" -o "$REF_CORE/obj/$base.o"
done

echo "==> compiling core (atg_scs)"
for src in "$SCS_SRC"/src/*.cpp; do
    base="scs_$(basename "$src" .cpp)"
    g++ "${CFLAGS[@]}" -c "$src" -o "$REF_CORE/obj/$base.o"
done

echo "==> archiving"
rm -f "$REF_CORE/librefcore.a"
ar rcs "$REF_CORE/librefcore.a" "$REF_CORE"/obj/*.o

echo "reference core ready: $REF_CORE/librefcore.a"
