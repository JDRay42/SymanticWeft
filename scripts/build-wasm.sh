#!/usr/bin/env bash
# Build the SemanticWeft WASM package for all three distribution targets.
#
# Usage:
#   scripts/build-wasm.sh [--release]
#
# Outputs:
#   packages/wasm/pkg/bundler/  — for webpack / rollup / bundler pipelines
#   packages/wasm/pkg/nodejs/   — for Node.js (CommonJS)
#   packages/wasm/pkg/web/      — for browser <script type="module">
#
# Options:
#   --release   Build in release mode (enables wasm-opt if wasm-opt is in PATH).
#               Default is dev mode (faster, no optimisation).
#
# Requirements:
#   wasm-pack is installed automatically if not already on PATH.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WASM_CRATE="$REPO_ROOT/packages/wasm"
PROFILE="dev"

for arg in "$@"; do
    case "$arg" in
        --release) PROFILE="release" ;;
        *) echo "Unknown argument: $arg" >&2; exit 1 ;;
    esac
done

# ── Install wasm-pack if missing ─────────────────────────────────────────────

if ! command -v wasm-pack &>/dev/null; then
    echo "wasm-pack not found — installing…"
    curl --proto '=https' --tlsv1.2 -sSf https://rustwasm.github.io/wasm-pack/installer/init.sh | sh
fi

echo "wasm-pack $(wasm-pack --version)"

# ── Build ────────────────────────────────────────────────────────────────────

build() {
    local target="$1"
    local out="$WASM_CRATE/pkg/$target"
    echo ""
    echo "▶ Building target=$target  profile=$PROFILE  → $out"
    wasm-pack build "$WASM_CRATE" \
        --target "$target" \
        --out-dir "pkg/$target" \
        --"$PROFILE"
}

build bundler
build nodejs
build web

echo ""
echo "✓ WASM build complete."
echo "  bundler : $WASM_CRATE/pkg/bundler/"
echo "  nodejs  : $WASM_CRATE/pkg/nodejs/"
echo "  web     : $WASM_CRATE/pkg/web/"
