#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

SMOKE_ROOT="${KONGODB_SMOKE_ROOT:-./.smoke}"

echo "Cleaning smoke artifacts in: $SMOKE_ROOT"
rm -rf "$SMOKE_ROOT/data" "$SMOKE_ROOT/logs" "$SMOKE_ROOT/auth" "$SMOKE_ROOT/lookup"
mkdir -p "$SMOKE_ROOT/data" "$SMOKE_ROOT/logs"

echo "Done."
