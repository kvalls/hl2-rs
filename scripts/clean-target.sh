#!/usr/bin/env bash
# clean-target.sh - Maintain target/ directory and clean build remnants for HL2RS.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

CLEANER_PY="$PROJECT_ROOT/scripts/checks/clean_target.py"

if [[ ! -f "$CLEANER_PY" ]]; then
    echo "Error: Cleaner script not found at $CLEANER_PY" >&2
    exit 1
fi

exec python3 "$CLEANER_PY" "$@"
