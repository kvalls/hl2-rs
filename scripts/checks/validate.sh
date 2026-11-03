#!/usr/bin/env bash
# Architecture guardrails, tests, strict clippy (CI flags) and fmt for the workspace.
set -euo pipefail
cd "$(dirname "$0")/../.."

echo "=== Checking architecture guardrails ==="
python3 scripts/checks/architecture.py

echo "=== Checking Python helpers ==="
python3 -m unittest discover -s scripts/checks -p 'test_*.py'

echo "=== Checking application plugin selections ==="
python3 scripts/checks/check_plugin_selection.py

echo "=== Running workspace tests ==="
cargo test --workspace --locked --no-fail-fast

echo "=== Running cargo clippy ==="
cargo clippy --workspace --all-targets --locked -- -D warnings

echo "=== Running cargo fmt check ==="
cargo fmt --check

echo "=== All validation checks passed ==="
