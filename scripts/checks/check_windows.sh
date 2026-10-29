#!/usr/bin/env bash
# Type-checks and lints HL2RS for Windows from macOS or Linux (no linking), so macOS-only code and
# dependencies are proven to be gated. Installs the Windows std target once if missing.
set -euo pipefail
cd "$(dirname "$0")/../.."
TARGET=x86_64-pc-windows-msvc
rustup target list --installed | grep -qx "$TARGET" || rustup target add "$TARGET"
# blake3 would compile C for MSVC; its pure-Rust path is enough for a check (no linking).
CARGO_FEATURE_PURE=1 cargo clippy --locked --target "$TARGET" -p source-ui -p hl2-ui -p hl2-bevy --all-targets -- -D warnings
echo "=== Windows check passed ==="
