#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$SCRIPT_DIR"
cd "$PROJECT_ROOT"

show_help() {
    cat << 'EOF'
Usage: ./launch-bevy.sh [OPTIONS]

Options:
      --audio-trace <path.jsonl>      Record audio cue and soundscape events to a JSONL trace file
      --background                    Run headlessly without opening an OS window (for background replay / test runs)
      --background-map <name>         Override the 3D background map loaded behind the main menu (default: background01)
      --borderless                    Start in borderless windowed mode (native full-screen Space on macOS)
      --capture <path.png>            Save screenshot of the final rendered frame to a PNG file
      --capture-burst <count>         Capture a sequence of N frames to PNG files (requires --map)
      --capture-live                  Capture frames live during interactive execution (requires --map)
      --capture-monitor <path.png>    Capture an in-game camera monitor texture to a PNG file (requires --map)
      --capture-window                Capture the presented OS window buffer rather than the internal game viewport
      --cpu-skinning                  Use CPU vertex skinning instead of GPU compute skinning
  -d, --debug                         Build and run the debug profile instead of release
      --fake-monitor <spec>           Simulate a monitor with specified modes (e.g. 1920x1080,1280x720)
      --fly                           Start with flight/noclip mode enabled (requires --map)
      --frag-swept-box                Use swept-box physics query for frag grenade collision
      --frames <count>                Exit automatically after rendering N frames (30..=36000)
      --fullscreen                    Start in exclusive fullscreen display mode
      --game <path>                   Path to Half-Life 2 installation (overrides saved preference and HL2_ROOT)
      --height <pixels>               Window height in pixels (240..=8192, default from video config)
  -h, --help                          Show this help message and exit
      --main-menu                     Open the main menu (overlaid on top of --map if specified)
      --map <name>                    Map to load directly into gameplay (e.g. d1_trainstation_01)
      --movement-script <path.json>   Replay recorded player inputs from a movement script fixture (requires --map)
      --mute                          Silence all audio output for this run (Audio settings unchanged)
      --mute-ambient                  Mute looping ambient soundscapes and background environment audio
      --no-build                     Launch the existing binary without checking freshness or rebuilding
      --no-clean                     Skip automatic stale target cleanup before rebuilding
      --no-focus                      Launch window without requesting input focus (for unattended test runs)
      --no-pvs                        Disable BSP potential visible set (PVS) occlusion culling
      --no-setup                      Fail with an error instead of opening setup if installation is missing
      --pitch <degrees>               Set initial camera pitch angle in degrees (requires --map)
      --position <x> <y> <z>          Set initial camera spawn coordinates (requires --map)
      --profile                       Record CPU stage execution timings in the run report
      --rebuild, --force-build        Force rebuilding the binary even if build artifacts are up to date
      --report <path.json>            Path for simulation report output (default: artifacts/bevy-report.json)
      --setup                         Open the setup dialog to choose/remember the Half-Life 2 folder
      --setup-preview <path.png>      Capture a screenshot of the setup dialog and exit without loading a map
      --tonemap-scale <scale>         Override HDR tonemapping exposure multiplier (positive float)
      --uncapped                      Disable VSync to render and present frames unconstrained
      --volume <0.0..1.0>             Set master audio volume level (default from audio config)
      --width <pixels>                Window width in pixels (320..=8192, default from video config)
      --windowed                      Start in windowed display mode
      --world-partition               Enable spatial world partition culling
      --yaw <degrees>                 Set initial camera yaw angle in degrees (requires --map)
EOF
}

# If --help or -h is present, display help immediately and exit without running any build check
for arg in "$@"; do
    if [[ "$arg" == "--help" || "$arg" == "-h" ]]; then
        show_help
        exit 0
    fi
done

FORCE_REBUILD=0
DEBUG_BUILD=0
NO_BUILD=0
NO_CLEAN=0
GAME_ARGS=()

for arg in "$@"; do
    case "$arg" in
        --rebuild|--force-build)
            FORCE_REBUILD=1
            ;;
        --debug|-d)
            DEBUG_BUILD=1
            ;;
        --no-clean)
            NO_CLEAN=1
            ;;
        --no-build)
            NO_BUILD=1
            ;;
        *)
            GAME_ARGS+=("$arg")
            ;;
    esac
done

TARGET_BIN="$PROJECT_ROOT/bin/hl2-bevy"
INFO_JSON="$PROJECT_ROOT/bin/build-bevy-info.json"
BEVY_ASSETS="$PROJECT_ROOT/bin/bevy-assets"
SHADER_SOURCE="$PROJECT_ROOT/crates/hl2-bevy/assets"
CLEANER_SCRIPT="$PROJECT_ROOT/scripts/clean-target.sh"

DESIRED_PROFILE="release"
if [[ "$DEBUG_BUILD" -eq 1 ]]; then
    DESIRED_PROFILE="debug"
fi

# Clean remnants from old/aborted builds in bin/
rm -f "$PROJECT_ROOT/bin/".*.tmp "$PROJECT_ROOT/bin/"*.tmp "$PROJECT_ROOT/bin/"*.old "$PROJECT_ROOT/bin/"*.bak 2>/dev/null || true
find "$PROJECT_ROOT/bin" -name ".DS_Store" -delete 2>/dev/null || true

NEEDS_BUILD=0
BUILD_REASON=""

CURRENT_BRANCH="$(git symbolic-ref --short -q HEAD 2>/dev/null || git rev-parse --short HEAD 2>/dev/null || echo "unknown")"
CURRENT_COMMIT="$(git rev-parse HEAD 2>/dev/null || echo "unknown")"

if [[ "$NO_BUILD" -eq 1 ]]; then
    if [[ ! -x "$TARGET_BIN" ]]; then
        echo "Error: --no-build was specified, but executable does not exist: $TARGET_BIN" >&2
        exit 1
    fi
elif [[ "$FORCE_REBUILD" -eq 1 ]]; then
    NEEDS_BUILD=1
    BUILD_REASON="force rebuild requested (--rebuild)"
elif [[ ! -f "$TARGET_BIN" || ! -x "$TARGET_BIN" ]]; then
    NEEDS_BUILD=1
    BUILD_REASON="executable missing or not executable ($TARGET_BIN)"
elif [[ ! -d "$BEVY_ASSETS" ]]; then
    NEEDS_BUILD=1
    BUILD_REASON="shader assets directory missing ($BEVY_ASSETS)"
elif [[ ! -f "$INFO_JSON" ]]; then
    NEEDS_BUILD=1
    BUILD_REASON="build metadata missing ($INFO_JSON)"
else
    # 1. Check profile mismatch (release vs debug)
    RECORDED_PROFILE="$(grep -E '"profile"' "$INFO_JSON" 2>/dev/null | head -n1 | sed -E 's/.*"profile"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/' || true)"
    if [[ -n "$RECORDED_PROFILE" && "$RECORDED_PROFILE" != "$DESIRED_PROFILE" ]]; then
        NEEDS_BUILD=1
        BUILD_REASON="build profile changed (recorded: '$RECORDED_PROFILE', desired: '$DESIRED_PROFILE')"
    fi

    # 2. Check git branch changes
    if [[ "$NEEDS_BUILD" -eq 0 && "$CURRENT_BRANCH" != "unknown" ]]; then
        RECORDED_BRANCH="$(grep -E '"git_branch"' "$INFO_JSON" 2>/dev/null | head -n1 | sed -E 's/.*"git_branch"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/' || true)"
        if [[ -z "$RECORDED_BRANCH" ]]; then
            NEEDS_BUILD=1
            BUILD_REASON="metadata missing git branch information"
        elif [[ "$CURRENT_BRANCH" != "$RECORDED_BRANCH" ]]; then
            NEEDS_BUILD=1
            BUILD_REASON="git branch changed ('$RECORDED_BRANCH' -> '$CURRENT_BRANCH')"
        fi
    fi

    # 3. Check git commit changes
    if [[ "$NEEDS_BUILD" -eq 0 && "$CURRENT_COMMIT" != "unknown" ]]; then
        RECORDED_COMMIT="$(grep -E '"git_commit"' "$INFO_JSON" 2>/dev/null | head -n1 | sed -E 's/.*"git_commit"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/' || true)"
        if [[ -z "$RECORDED_COMMIT" ]]; then
            NEEDS_BUILD=1
            BUILD_REASON="metadata missing git commit information"
        elif [[ "$CURRENT_COMMIT" != "$RECORDED_COMMIT" ]]; then
            NEEDS_BUILD=1
            BUILD_REASON="git commit changed ('${RECORDED_COMMIT:0:8}' -> '${CURRENT_COMMIT:0:8}')"
        fi
    fi

    # 4. Check modified source files (newer than current binary)
    if [[ "$NEEDS_BUILD" -eq 0 ]]; then
        NEWER_SOURCE="$(find "$PROJECT_ROOT/crates" "$PROJECT_ROOT/Cargo.toml" "$PROJECT_ROOT/Cargo.lock" -newer "$TARGET_BIN" -type f ! -name ".*" 2>/dev/null | head -n 1 || true)"
        if [[ -n "$NEWER_SOURCE" ]]; then
            NEEDS_BUILD=1
            BUILD_REASON="source file modified (${NEWER_SOURCE#$PROJECT_ROOT/})"
        fi
    fi

    # 5. Check modified shader assets
    if [[ "$NEEDS_BUILD" -eq 0 && -d "$SHADER_SOURCE" ]]; then
        NEWER_SHADER="$(find "$SHADER_SOURCE" -newer "$TARGET_BIN" -type f ! -name ".*" 2>/dev/null | head -n 1 || true)"
        if [[ -n "$NEWER_SHADER" ]]; then
            NEEDS_BUILD=1
            BUILD_REASON="shader asset modified (${NEWER_SHADER#$PROJECT_ROOT/})"
        fi
    fi
fi

if [[ "$NEEDS_BUILD" -eq 1 ]]; then
    echo "==> Build is out of date: $BUILD_REASON"
    if [[ "$NO_CLEAN" -eq 0 && -x "$CLEANER_SCRIPT" ]]; then
        echo "==> Cleaning stale target directory remnants before build..."
        "$CLEANER_SCRIPT" --stale || true
    fi
    echo "==> Running scripts/build-bevy.sh..."
    BUILD_ARGS=()
    if [[ "$DEBUG_BUILD" -eq 1 ]]; then
        BUILD_ARGS+=("--debug")
    fi
    if [[ ${#BUILD_ARGS[@]} -gt 0 ]]; then
        "$PROJECT_ROOT/scripts/build-bevy.sh" "${BUILD_ARGS[@]}"
    else
        "$PROJECT_ROOT/scripts/build-bevy.sh"
    fi
else
    echo "==> Build is up to date (${DESIRED_PROFILE} on branch '$CURRENT_BRANCH', commit '${CURRENT_COMMIT:0:8}')."
fi

if [[ ! -x "$TARGET_BIN" ]]; then
    echo "Error: executable not found or not executable: $TARGET_BIN" >&2
    exit 1
fi

if [[ ${#GAME_ARGS[@]} -gt 0 ]]; then
    exec "$TARGET_BIN" "${GAME_ARGS[@]}"
else
    exec "$TARGET_BIN"
fi
