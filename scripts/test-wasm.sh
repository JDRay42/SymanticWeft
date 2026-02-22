#!/usr/bin/env bash
# Run the SemanticWeft WASM binding tests in Node.js via wasm-pack.
#
# Usage:
#   scripts/test-wasm.sh
#
# Requirements:
#   wasm-pack is installed automatically if not already on PATH.
#   Node.js >= 18 must be available.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WASM_CRATE="$REPO_ROOT/packages/wasm"

# ── Install wasm-pack if missing ─────────────────────────────────────────────

if ! command -v wasm-pack &>/dev/null; then
    echo "wasm-pack not found — installing…"
    curl --proto '=https' --tlsv1.2 -sSf https://rustwasm.github.io/wasm-pack/installer/init.sh | sh
fi

echo "wasm-pack $(wasm-pack --version)"

# ── Test ─────────────────────────────────────────────────────────────────────

echo ""
echo "▶ Running WASM tests in Node.js…"
wasm-pack test "$WASM_CRATE" --node

echo ""
echo "✓ WASM tests passed."
