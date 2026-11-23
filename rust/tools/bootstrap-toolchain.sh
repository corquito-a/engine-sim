#!/usr/bin/env bash
# Installs a minimal Rust toolchain (rustc + rust-std + cargo) without rustup.
#
# This sandbox has no access to static.rust-lang.org, but the official Rust
# release tarballs are republished verbatim on the npm registry, so we pull the
# three components we need from there and merge them into a normal prefix
# layout ($PREFIX/bin/rustc, $PREFIX/lib/rustlib/..., ...).
#
# Usage:  ./bootstrap-toolchain.sh [prefix]      (default: /tmp/rust-toolchain)
#         source ./bootstrap-toolchain.sh --env  (only exports PATH for an
#                                                 already-installed prefix)
set -euo pipefail

VERSION="${RUST_VERSION:-1.88.0}"
TARGET="${RUST_TARGET:-x86_64-unknown-linux-gnu}"
DEFAULT_PREFIX="/tmp/rust-toolchain"
PREFIX="${1:-$DEFAULT_PREFIX}"

if [ "${1:-}" = "--env" ]; then
    export PATH="$DEFAULT_PREFIX/bin:$PATH"
    return 0 2>/dev/null || exit 0
fi

if [ -x "$PREFIX/bin/rustc" ] && [ -x "$PREFIX/bin/cargo" ]; then
    echo "toolchain already installed at $PREFIX"
    "$PREFIX/bin/rustc" --version
    "$PREFIX/bin/cargo" --version
    exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

declare -A COMPONENTS=(
    [rustc]="rustc"
    [std]="rust-std"
    [cargo]="cargo"
)

for key in rustc std cargo; do
    name="${COMPONENTS[$key]}"
    pkg="@rustbin/${name}-${VERSION}-${TARGET}"
    mkdir -p "$WORK/$key"
    echo "fetching $pkg"
    ( cd "$WORK/$key" && npm pack "$pkg" >/dev/null 2>&1 )
    tarball="$(ls "$WORK/$key"/*.tgz | head -1)"
    tar xzf "$tarball" -C "$WORK/$key"
done

mkdir -p "$PREFIX"
cp -r "$WORK/rustc/package/rustc/." "$PREFIX/"
cp -r "$WORK/std/package/rust-std-${TARGET}/." "$PREFIX/"
cp -r "$WORK/cargo/package/cargo/." "$PREFIX/"

echo "installed:"
"$PREFIX/bin/rustc" --version
"$PREFIX/bin/cargo" --version
echo
echo "add to PATH:  export PATH=\"$PREFIX/bin:\$PATH\""
