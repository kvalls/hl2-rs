#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

DEBUG_BUILD=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --debug|-d|-DebugBuild)
            DEBUG_BUILD=1
            shift
            ;;
        *)
            echo "Unknown option: $1" >&2
            echo "Usage: $0 [--debug]" >&2
            exit 1
            ;;
    esac
done

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo not found. Install Rust using rustup first." >&2
    exit 1
fi

SHADER_SOURCE="$PROJECT_ROOT/crates/hl2-bevy/assets"
if [[ ! -d "$SHADER_SOURCE" ]]; then
    echo "Bevy shader assets are missing from crates/hl2-bevy/assets." >&2
    exit 1
fi
if [[ -z "$(find "$SHADER_SOURCE" -type f ! -name ".*" 2>/dev/null)" ]]; then
    echo "Bevy shader assets are empty." >&2
    exit 1
fi

PROFILE_NAME="release"
CARGO_ARGS=("build" "--locked" "-p" "hl2-bevy" "--bin" "hl2-bevy")
if [[ "$DEBUG_BUILD" -eq 1 ]]; then
    PROFILE_NAME="debug"
else
    CARGO_ARGS+=("--release")
fi

echo "Building hl2-bevy ($PROFILE_NAME)..."
cargo "${CARGO_ARGS[@]}"

TARGET_ROOT="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}"
if [[ -n "${CARGO_BUILD_TARGET:-}" ]]; then
    TARGET_ROOT="$TARGET_ROOT/$CARGO_BUILD_TARGET"
fi

SOURCE_EXECUTABLE="$TARGET_ROOT/$PROFILE_NAME/hl2-bevy"
if [[ ! -f "$SOURCE_EXECUTABLE" ]]; then
    echo "Built executable not found: $SOURCE_EXECUTABLE" >&2
    exit 1
fi

PACKAGE_ROOT="$PROJECT_ROOT/bin"
SHADER_DEST="$PACKAGE_ROOT/bevy-assets"

WORKSPACE_PREFIX="${PROJECT_ROOT%/}/"
case "$PACKAGE_ROOT"/ in
    "$WORKSPACE_PREFIX"*) ;;
    *)
        echo "Bevy package destination is outside the project." >&2
        exit 1
        ;;
esac

if [[ "$SHADER_DEST" != "$PACKAGE_ROOT/bevy-assets" ]]; then
    echo "Bevy package destination is outside the project." >&2
    exit 1
fi

for package_path in "$PACKAGE_ROOT" "$SHADER_DEST"; do
    if [[ -L "$package_path" ]]; then
        echo "Bevy package destination must not be a link: $package_path" >&2
        exit 1
    fi
done

mkdir -p "$PACKAGE_ROOT"

# Only replace this verified generated directory, so removed shaders cannot linger.
if [[ -d "$SHADER_DEST" ]]; then
    rm -rf "$SHADER_DEST"
fi
cp -R "$SHADER_SOURCE" "$SHADER_DEST"
find "$SHADER_DEST" -name ".DS_Store" -delete 2>/dev/null || true

PACKAGE_EXECUTABLE="$PACKAGE_ROOT/hl2-bevy"
# Replace the file instead of overwriting it in place: macOS kills new launches of an executable
# whose signed pages were rewritten (exit 137), e.g. while a previous build is still running.
rm -f "$PACKAGE_EXECUTABLE"
cp "$SOURCE_EXECUTABLE" "$PACKAGE_EXECUTABLE"
chmod +x "$PACKAGE_EXECUTABLE"

compute_sha256() {
    local file="$1"
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$file" | awk '{print toupper($1)}'
    elif command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file" | awk '{print toupper($1)}'
    else
        openssl dgst -sha256 "$file" | awk '{print toupper($NF)}'
    fi
}

EXE_SHA256=$(compute_sha256 "$PACKAGE_EXECUTABLE")
BUILT_UTC=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
GIT_BRANCH="$(git symbolic-ref --short -q HEAD 2>/dev/null || git rev-parse --short HEAD 2>/dev/null || echo "unknown")"
GIT_COMMIT="$(git rev-parse HEAD 2>/dev/null || echo "unknown")"

# Build shader asset array in JSON
ASSETS_JSON="["
FIRST=1
while IFS= read -r SHADER_FILE; do
    if [[ -f "$SHADER_FILE" ]]; then
        REL_PATH="bin/bevy-assets/${SHADER_FILE#$SHADER_DEST/}"
        FILE_SHA=$(compute_sha256 "$SHADER_FILE")
        if [[ $FIRST -eq 0 ]]; then
            ASSETS_JSON+=","
        fi
        FIRST=0
        ASSETS_JSON+=$(cat <<ITEM_EOF

    {
      "path": "$REL_PATH",
      "sha256": "$FILE_SHA"
    }
ITEM_EOF
)
    fi
done < <(find "$SHADER_DEST" -type f ! -name ".*" | sort)
ASSETS_JSON+=$'\n  ]'

METADATA_FILE="$PACKAGE_ROOT/build-bevy-info.json"
TMP_METADATA="$PACKAGE_ROOT/.build-bevy-info-$$.tmp"

cat <<EOF > "$TMP_METADATA"
{
  "built_utc": "$BUILT_UTC",
  "profile": "$PROFILE_NAME",
  "executable": "bin/hl2-bevy",
  "sha256": "$EXE_SHA256",
  "git_branch": "$GIT_BRANCH",
  "git_commit": "$GIT_COMMIT",
  "assets": $ASSETS_JSON
}
EOF

mv -f "$TMP_METADATA" "$METADATA_FILE"

echo "Built bin/hl2-bevy with bin/bevy-assets. Run ./launch-bevy.sh."
