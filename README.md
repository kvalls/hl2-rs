# HL2-RS

A standalone, partial Rust reconstruction of Half-Life 2 that reads maps, models, textures, animations and sounds from an installed copy. It does not load Valve's game or engine DLLs. The full campaign is not playable yet.

## Setup steps

1. **Install Half-Life 2 through Steam.** HL2-RS needs the files from your owned Steam PC copy (app 220). It reads those files in place; the rewrite does not include or download them.
2. **Build HL2-RS.** This repository contains source, not a ready-to-play executable. On Windows, install [Rust](https://rustup.rs/) using the MSVC toolchain, [Visual Studio Build Tools](https://visualstudio.microsoft.com/downloads/) with **Desktop development with C++** and a Windows SDK, and [Git](https://git-scm.com/downloads). Open PowerShell in the folder where you want the project and run:

   ```powershell
   git clone https://github.com/kvalls/hl2-rs.git
   cd hl2-rs
   .\scripts\build-bevy.ps1
   .\launch-bevy.cmd
   ```

   If PowerShell blocks the build script, use `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-bevy.ps1` for that command. On macOS, install Rust and the Xcode command-line tools (`xcode-select --install`), then follow the [macOS build instructions](#macos). See [Platforms](#platforms) for tested systems.
3. **Let HL2-RS find the game.** It searches Steam and additional Steam libraries automatically. If the installation is ready, the game starts. If it cannot find usable files, a **Locate Half-Life 2** window opens: use **Browse...**, type or paste the folder path, or choose **Retry detection** after installing/moving the game.
4. **Choose the folder and Continue.** Select `Half-Life 2` or its `hl2` subfolder. To locate it yourself, open Steam > Half-Life 2 > Properties > Installed Files > Browse. HL2-RS checks the requested map (if any) and representative required content, explains missing files, and remembers the selected location. For damaged/incomplete content, use Steam's [Verify integrity of game files](https://help.steampowered.com/en/faqs/view/0C48-FCBD-DA71-93EB). This startup check is not a complete integrity scan.
5. **Launch again whenever you want to playtest.** Double-click `launch-bevy.cmd` on Windows, or run `./launch-bevy.sh` on macOS. Rust is only needed to build/rebuild. It opens the main menu; start a game from it (or pass `--map d1_trainstation_02`), click the game window to capture the mouse, use WASD to move, and Escape to pause/release the mouse. Campaign completion remains unfinished.

To change the remembered folder, run `.\launch-bevy.cmd --setup` (macOS: `./launch-bevy.sh --setup`). If a remembered folder stops working, HL2-RS retries Steam discovery and opens setup if necessary. `--game PATH` takes priority over `HL2_ROOT`, then the remembered folder, then Steam discovery; explicit overrides apply to that launch and are not automatically saved. Correct or unset an outdated `HL2_ROOT` so it does not keep overriding your saved choice.

The folder preference is stored in `%LOCALAPPDATA%\hl2-rs\installation.json` on Windows, `~/Library/Application Support/hl2-rs/installation.json` on macOS, or `${XDG_CONFIG_HOME:-~/.config}/hl2-rs/installation.json` on Linux. `HL2_RS_CONFIG_DIR` selects an alternative settings directory. To reset setup, remove that file or use `--setup`. Automated runs can use `--no-setup` to return a text error instead of opening a window; bounded captures, movement scripts and `--no-focus` runs already do this unless `--setup` is explicit. The new setup screen has not yet been tested on macOS/Linux.

## Platforms

| Operating system | Status | Evidence |
| --- | --- | --- |
| Windows 11 64-bit (x86_64) | Primary tested platform | Owner builds, unit and owned-file tests, packaged replays/captures and native-game comparisons; CI runs on `windows-latest`. |
| macOS 15+ on Apple Silicon (aarch64, Metal) | Tested by a contributor | On an Apple M1 Pro: build, tests, and the packaged build (`./scripts/build-bevy.sh`, `./launch-bevy.sh`) running `d1_trainstation_02`, including the scripted dispenser, Metrocop sequence and queue, with exit code 0 ([PR #1](https://github.com/kvalls/hl2-rs/pull/1)). Background packaged replays and controlled captures pass; windowed checks and native comparisons are **not tested** (see [validation](docs/validation.md)). |
| Linux | **Not tested** | Install discovery looks in `~/.steam/steam` and `~/.local/share/Steam`, but neither a build nor the POSIX scripts have been run on Linux. No Linux test environment is available. |
| macOS on Intel, other systems | **Not tested** | |

## Runtime and development requirements

The packaged game needs its executable, the matching `bin/bevy-assets` shader folder,
your owned installed HL2 content, and the operating system's graphics/audio support.
It reads fonts from the owned content, with the bundled MIT ProggyClean fallback.
Windows uses its MSVC/UCRT runtime and graphics drivers; macOS uses Apple system
frameworks and Metal. Linux needs a working X11/Wayland desktop, graphics drivers
with a wgpu-supported backend (normally Vulkan), and the ALSA/udev system libraries.
Linux runtime behavior is **not tested** yet.

The game does not require FFmpeg, Python, Node.js, a mod loader, Valve engine DLLs,
or research tools. Movie playback and commentary implementation are deferred; no
movie decoder dependency has been added. Rust/Cargo and compiler toolchains are
build requirements, not requirements for running an already built package. The
POSIX launcher can rebuild stale packages; use `./launch-bevy.sh --no-build` or run
`bin/hl2-bevy` directly when no build tools are installed.

On Linux, setup's optional **Browse...** folder picker uses the `rfd` native-dialog
backend. Its documented runtime requirements are libdbus and a supported
GTK/GNOME/KDE XDG Desktop Portal backend, with Zenity used for dialogs/fallback.
These are desktop packages, not bundled game files. Typing the installation path
in setup, passing `--game`, or using automatic discovery avoids invoking Browse.
See the pinned [rfd backend requirements](https://docs.rs/rfd/0.17.2/rfd/#linux--bsd-backends).

Source builds need Git, Rust stable/Cargo, and the platform compiler tools:
Visual Studio C++ tools/Windows SDK on Windows, Xcode command-line tools on macOS,
and a C++ compiler plus pkg-config and development headers on Linux. Python 3,
rustfmt and Clippy are needed for the repository's validation scripts; they are
not game runtime dependencies. Cargo library versions are locked in `Cargo.lock`.

For an Ubuntu/Debian source build, the initial dependency setup is:

```sh
sudo apt-get update
sudo apt-get install g++ pkg-config libx11-dev libasound2-dev libudev-dev libxkbcommon-x11-0 libwayland-dev libxkbcommon-dev libdbus-1-dev
./scripts/build-bevy.sh
./launch-bevy.sh --no-build
```

Install the graphics driver and desktop portal backend appropriate to the system;
package names differ between distributions. The library list follows the pinned
[Bevy Linux build requirements](https://github.com/bevyengine/bevy/blob/v0.19.1/docs/linux_dependencies.md).
The Linux commands are documented prerequisites, **not a locally verified build**.
CI defines native checks on Windows and Apple Silicon macOS; owned-content/rendering checks remain
separate from source-only CI.

## Branches and contributing

HL2-RS is a Bevy 0.19 / wgpu Rust rewrite. Owned asset readers, shared simulation and rendering are separated following [iw4L](https://github.com/vladtrc/iw4L). On 2026-10-07 the Bevy work was merged into `main` and the original Macroquad/OpenGL host was removed from it.

| Branch | Purpose | Pull requests |
| --- | --- | --- |
| `main` | The Bevy/wgpu rewrite: shared movement, entity/weapon simulation, choreography, animated presentation, model lighting, owned HUD/audio, sky and monitors. | Target this branch. |
| [macroquad-prototype](https://github.com/kvalls/hl2-rs/tree/macroquad-prototype) | Preserved snapshot of the original Macroquad prototype. | Historical reference only. |
| [bevy-migration](https://github.com/kvalls/hl2-rs/tree/bevy-migration) | The migration branch, frozen at the point it merged into `main`. | Historical reference only. |

`source-assets` reads Source formats, `modkit-core` owns shared contracts, player and pose math, `hl2-simulation` owns gameplay, physics and choreography, `source-ui` is the game-agnostic UI toolkit (canvas, fonts, widgets, dialog frames, window focus, `.res` layouts, the localization engine; what the Source SDK's `vgui_controls` is to every Source game), `hl2-ui` is Half-Life 2's own UI over it and over `hl2-simulation`: the HUD, menus and console, and `hl2-bevy` is the host. Bevy's default PBR materials do not recreate Source shaders or physics; those have their own implementations and comparisons. No speed or fidelity improvement is assumed solely from the engine. See [CONTRIBUTING](CONTRIBUTING.md) before opening a PR.

Build with `scripts/build-bevy.ps1` and run `launch-bevy.cmd`. Use `launch-bevy-1080p.cmd` for a 1920x1080 window or `launch-bevy-borderless.cmd` for the primary desktop resolution. The executable defaults to walking with the shared 15 ms `Player` controller and `Physics` collision queries; `--fly` or F2 enables flight. Escape opens the pause menu and releases the cursor; select Resume to continue. Tilde opens the developer console. Doors and physics props move with their colliders, entity clips and weapon viewmodels animate, and scene/weapon/projectile logic runs in the fixed step. F3 gives the developer loadout; E uses doors, slots/mouse wheel select weapons and mouse buttons fire/confirm. Owned ambient, scene, weapon and HUD cues play through Bevy audio. Owned sky backgrounds (HDR faces when present), miniature scenery, model-driven iris projection, facial flexes, lip sync and live `_rt_Camera` monitor feeds are rendered. Authored changelevel triggers carry landmark-relative position and inventory; console `map` starts fresh. Save/global entity state and complete player transfer remain unfinished. Tick-based `--movement-script` fixtures record player/gameplay state and capture the final frame. For unattended runs, `--volume 0.01` sets the master volume and `--no-focus` opens the window without taking focus. See [Bevy instructions and milestones](docs/bevy-migration.md).

GPU skinning and conservative BSP PVS culling (alongside Bevy frustum culling) keep the tested station02 spawn at about 157-175 FPS uncapped at 1080p. Other views, native performance parity and sustained 144 FPS remain unverified; see [performance measurements](docs/performance.md). `launch-bevy.cmd --uncapped` presents without VSync for testing; `--profile` records bounded CPU stage measurements.

![HL2-RS Rust runtime rendering the trainstation with pistol, HUD and development diagnostics](docs/images/hl2-rs-trainstation.png)

Capture from the packaged Rust build on `d1_trainstation_02`, with F1 diagnostics and a developer weapon loadout. This is a rendering preview, not evidence of completed campaign gameplay. The screenshot depicts owned HL2 content; distributable game assets are not included.

Model-authored activity lookup now repairs unsupported implicit NPC idle labels, including Kleiner, while preserving explicit defaults and existing loaded poses. Scene GESTURE events now play as layered animations (Source delta/post layers, bone masks, autolayers, Faceposer tag timing) for actors such as Barney and Kleiner. A side-by-side comparison with the original game is in progress: Barney now spawns from his point_template and opens doors on his way to the desk, but NPC-touch triggers are still missing. Native AI scheduling, head/facial animation, lip sync and first-level staging remain incomplete.

## Build a fresh checkout

You need an owned, installed Steam PC copy of Half-Life 2, Rust stable, and the compiler tools for your platform listed in [Runtime and development requirements](#runtime-and-development-requirements). On Windows use the MSVC toolchain and Visual Studio C++ build tools with a Windows SDK. The renderer uses Bevy/wgpu (tested on Windows with the default backend); GPU and RAM minimums have not been measured. On macOS use `./scripts/build-bevy.sh` and `./launch-bevy.sh`; see [Platforms](#platforms) for what each system has been tested with.

The tested installation is Steam app 220, build `19307283`, patch `9912070`. Other game builds are unverified. Rust `1.99.0` was used for the recorded Windows checks. Cargo resolves the library versions in `Cargo.lock`; no external mod, loader or Source engine runtime is needed.

Private research now includes 42 selected native modules, 122,721 saved analysis entries and 10,752 separate temporary discovery entries. The audited consolidated database holds 133,473 distinct observed addresses with some pseudocode and preserves exact bytes, original failures and export variants. Reviewed names/types, Rust implementation and native verification remain separate; this research sets no completion flags. Full runtime scope, discovery, ABIs and behavior parity remain unverified. All native binaries, code, tools and databases remain outside this repository. See [research evidence](docs/research.md).

```powershell
git clone https://github.com/kvalls/hl2-rs.git
cd hl2-rs
.\scripts\build-bevy.ps1
.\launch-bevy.cmd
```

The repository contains source, not a prebuilt executable. Building creates `bin/hl2-bevy.exe` and its shader folder `bin/bevy-assets`. If PowerShell blocks the local build script, invoke `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-bevy.ps1` for that process. To uninstall, remove the checkout and optionally the folder preference listed in [Setup steps](#setup-steps); the game installation is read-only.

### macOS

On macOS (Apple Silicon / POSIX) with the Rust toolchain installed, use `./launch-bevy.sh` to build and launch the game. Steam installations in `~/Library/Application Support/Steam` are automatically discovered.

```bash
git clone https://github.com/kvalls/hl2-rs.git
cd hl2-rs
./launch-bevy.sh
```

`launch-bevy.sh` is an all-in-one launcher that manages builds automatically:
- **Automatic build & freshness check**: It checks whether `bin/hl2-bevy` exists and is up to date with respect to source files (`crates/`), `Cargo.toml`, shader assets (`crates/hl2-bevy/assets`), git branch switches, and git commits.
- **Automatic target cleanup**: Before a required build, the launcher prunes stale workspace artifacts while preserving third-party caches. It holds Cargo-compatible debug/release locks throughout cleanup, then releases them before building. Busy targets and unsupported/network filesystems skip pruning. Use `--no-clean` to disable cleanup; `--no-build` and up-to-date launches do not clean. Manual preview: `./scripts/clean-target.sh --stale --dry-run`.
- **Direct launch**: If the build is already up to date, it launches immediately without rebuilding.

Convenience launcher scripts:
- `./launch-bevy.sh` — launches the game with stored resolution/display settings or opens the main menu.
- `./launch-bevy-1080p.sh` — launches in a 1920x1080 window (`--windowed --width 1920 --height 1080`).
- `./launch-bevy-borderless.sh` — launches in borderless window mode (`--borderless`).

Common `launch-bevy.sh` arguments:
- `--rebuild` (or `--force-build`) — force a build before launching, even if build artifacts appear current.
- `--debug` (or `-d`) — build and run the debug profile instead of release.
- `--no-build` — launch the existing executable directly without checking for updates, cleaning or compiling.
- `--no-clean` — skip automatic stale-target cleanup when a build is needed.
- Any game arguments (such as `--map d1_trainstation_01`, `--setup`, `--game <path>`, `--fullscreen`, `--windowed`) are forwarded directly to the game binary.

To compile and package `bin/hl2-bevy` and the `bin/bevy-assets` shader folder without immediately launching the game, run `./scripts/build-bevy.sh`. The executable and shader folder must remain together.

## Play the current build

Double-click `launch-bevy.cmd` in this folder. It launches `bin/hl2-bevy.exe`, the packaged release build. Rust is needed only for rebuilding. Without `--map` (or when the requested map cannot be loaded) the game opens the main menu over the chapter-1 background map `background01`; a loadable `--map` starts gameplay directly. Options that need an immediately loaded map (`--position`, `--yaw`, `--pitch`, `--fly`, `--movement-script`, `--capture-live`, `--capture-burst`, `--capture-monitor`) make the launch fail with an error when no loadable `--map` is given. The main menu uses the installed `gamemenu.res` categories; Load Game, Achievements and Workshop are disabled placeholders (see [docs/gameui.md](docs/gameui.md)); EXTRAS opens the Extras submenu (Lost Coast and Workshop placeholder), and New Game opens a resource-driven chapter browser for Half-Life 2, Episode One, Episode Two, and Lost Coast. Select a chapter and use Start New Game (or Enter) to load its first map; all installed chapters are selectable. Chapter progress locking remains unfinished. Developer commentary and startup/movie playback are implemented; see docs/gameui.md for their current limitations. OPTIONS opens the Options dialog (Gameplay, Keyboard with its Advanced dialog, Mouse, Gamepad, Audio, Video with its Advanced dialog) laid out from the installed resources; unfinished or partial controls carry cyan development status hints, and unavailable controls are hidden per platform; settings are saved to `hl2-config.json` next to the executable. Dialog buttons follow the installed scheme colors, plus a slight hover fill that retail lacks on desktop, and keep a press while the pointer is dragged off and back; an open drop-down list takes all clicks until it closes (see docs/gameui.md sections 6a and 6b). On macOS the green full-screen button is recognized as the Full screen display mode (and leaving it as Windowed); applying settings that do not change resolution or display mode never touches the window (docs/gameui.md section 10; not tested by hand). Key and mouse-button bindings, mouse sensitivity/invert/filter/acceleration, difficulty, master and music volume, resolution and display mode take effect; the window starts in the stored display mode unless `--windowed`, `--fullscreen` or `--borderless` is given (runs with `--frames`, `--capture` or `--movement-script` start windowed). Brightness (gamma) applies in every display mode with a live preview; The Closed captions setting shows subtitles or closed captions from the installed caption dictionaries for scene speech and sound scripts, also behind the main menu (VOX sentences such as HEV lines have none yet). HDR selects LDR without bloom, LDR with bloom, or full HDR. Model/texture/shadow detail, supported antialiasing, filtering, color correction and motion blur affect rendering; Advanced motion blur adds viewmodel animation blur to retail camera blur; Ultra adds denser camera/world and opaque/masked moving-model reconstruction, including projectiles. Ultra GPU performance tuning is in progress; transparent particle motion is not reconstructed. Speaker setup, sound quality and spoken language remain stored-only; focus-loss muting is partial (see the cyan hints and docs/gameui.md).

```powershell
.\launch-bevy.cmd --map d1_trainstation_01
.\launch-bevy.cmd --game 'C:\Program Files (x86)\Steam\steamapps\common\Half-Life 2'
.\launch-bevy.cmd --borderless
.\launch-bevy.cmd --width 1920 --height 1080
```

Steam libraries are discovered automatically; `HL2_ROOT` can override discovery. Original assets are read in place. There are no bundled game assets, decompiler tools or native game DLLs in the runtime. No FAL account or credits are needed.

| Control | Action |
| --- | --- |
| WASD / mouse | Move / look; click the window to capture the mouse |
| Escape / tilde | Cancel weapon selection first; otherwise pause / open developer console |
| Space / Ctrl | Jump / crouch |
| Shift / Alt | Sprint / walk slowly |
| E | Use a door or button within reach |
| Left / right click, R | Primary / implemented secondary attack, reload |
| 1-6 / mouse wheel | Open/cycle owned weapons; either attack button confirms |
| Q | Switch to the previous weapon |
| F3 | Developer loadout: crowbar, pistol, .357, SMG1, AR2, shotgun, reserves and suit |
| F1 | Toggle the development overlay; hidden by default |
| G | Apply a test impulse to a nearby prop |
| F2 | Toggle flight (Space/Ctrl rise/fall) |
| F10 | Quit |

The starting map normally has no player weapon. F3 is a test aid. It does not demonstrate campaign weapon acquisition. Some campaign scripts still block progression.

The pause menu has working Resume Game, Developer Console and Quit controls. The console completes commands, cvars and their arguments (installed maps, the loaded map's entity names and inputs) as you type, in a list under the line: Tab/Down and Shift+Tab/Up step through it, an empty line lists the command history (kept across runs), Ctrl+R searches it, and a help line shows the usage of the command being typed. Ctrl+W/U/K/A/E/L, Ctrl+arrows and paste (Cmd+V on macOS) edit the line; `con_filter_enable`, `con_filter_text` and `con_timestamp` filter and stamp the output, and engine warnings appear in it. `help` lists the implemented commands. Enter `sv_cheats 1; impulse 101` to get the implemented weapons, ammunition and suit. Other supported commands include `noclip`, `getpos`, `setpos`, `setang`, `map`, `ent_fire`, `find`, `echo`, `clear`, `toggleconsole` and `quit`. `ent_fire <target> [input] [parameter] [delay]` sends normal entity I/O to named targets; its console delay uses whole seconds. Classname fallback and the full Source command registry are unfinished. Unsupported commands report an error. Save/load is unfinished; the Options dialog is described above. Player, weapon, entity and prop simulation pauses while the menu or console is open; ambient audio does not yet pause. The UI uses installed GameUI labels/colors and fonts read from the owned VFS, with the MIT ProggyClean fallback, but its layout is not an exact VGUI reconstruction.

## Implemented so far

In short: owned VPK/BSP/VMT/VTF/MDL/PHY/audio readers; world, displacement, prop and model rendering with baked lightmaps, Source model lighting, HDR auto exposure, bloom, skies and live monitors; fixed-step player movement and collision; entity I/O, doors, triggers and choreographed scenes with layered gestures, facial flexes and lip sync; the HL2 arsenal through the gravity gun, frag, crossbow, RPG and bug bait (crowbar, pistol, .357, SMG1, AR2, shotgun first) with their HUD; VPhysics-like props with breakables, gibs, explosive barrels (they ignite, burn with the installed burning_character particles and explode) and basic constraints; NPC AI ported from the SDK for metrocops, Combine soldiers (including shotgunner, elite and prison guard), citizens/rebels and the Barney/Alyx allies (senses, relationships, schedules, cover and reload, NPC weapons, grenades, kicks, speech and footsteps; few native comparisons), classic and poison zombies (chase, claws/bites, headcrab bodygroup, burning), squads (membership, shared enemies, squad speech), NPC death ragdolls from the models' .phy ragdolls (jointed bodies with authored limits; the player walks through them; shots, explosions and crossbow pins act on corpses; retired beyond eight by fading; death forces checked against native pistol/shotgun kills); friendly NPCs immune to the player (F4 debug option to allow it); runtime entity creation (F4 spawn overlay with NPC and physics-prop tabs, npc_create, ent_create, prop_physics_create, npc_template_maker copies); and map transitions. The full item-by-item list is in [docs/features.md](docs/features.md).

## Fidelity work remaining

This is not a completed rewrite of Source. Major gaps include squad strategy slots, AI gesture layers (move-and-shoot recoil, flinches), most other NPC classes and native-verified NPC navigation/combat, arsenal and prop-physics leftovers (STATUS "Deferred physics/arsenal leftovers": ropes, fire, ragdoll gibs, several native comparisons), recoil/muzzle flashes/tracers/shell ejection, exact prediction spread and shotgun pellet hull traces, drowning and the rest of the player damage pipeline (knockback, punch angles, time-based damage), vehicles, save/load, campaign state transfer, complete choreography movement/loops/subscenes/dialogue/lip sync, non-sound animation events/blends/layered gestures/IK/flexes, ragdoll fidelity (death animation velocity, prop_ragdoll map bodies, headcrab release, burning ragdolls), dynamic/animated PHY collision, moving-platform behavior and blocked-door handling. Scripted movement and parenting/attachments are incomplete. History/aux-power panels, zoom/convar gates and additional VGUI animation commands remain unfinished. Projectile collision/material response, relationship and damage filters, water rejection, guided targeting, dissolve visuals and explosion particles do not reproduce the full native behavior.

Water/ladders, surface movement modifiers, exact crouch transitions and prediction need further work. The renderer lacks Source's full lighting/material system: bumped diffuse lighting, envmaps on models/brush entities (world LightmappedGeneric `$envmap` reflections with masks and bump normals are implemented), light probes, complete shadow parity, sky polygon masking, sky fog, fog, water/refraction, dynamic lightstyles and many material proxies. Symbolic speech now uses the owned actor-model gender registry and registered wave alternatives. Audio has Source distance gain for ambient_generic loops and env_soundscape backgrounds (SDK selection and 3 s crossfades, tested on d1_trainstation_02), player footsteps (SDK step timing and surface sounds) and surface-dependent impact sounds for bullets and the crowbar, but still lacks streaming, stereo panning, DSP room effects and other compressed formats. Player damage follows the SDK order (skill 2 scale, integer armor/health, trigger_hurt for the player and NPCs, fall damage) and is refused while the map's `gordon_invulnerable` env_global is on (d1_trainstation_01-03); env_global states carry across level changes. Damage shows the owned HudDamageIndicator sequences, DamageEffect/env_fade screen fades and, with the suit only, HEV voice sentences (VOX words from sentences.txt, HEV_DEAD at death); without the suit only Player.Death / Player.FallGib play. The player respawns by restarting the map; drowning and saves are unfinished. An MP3 is decoded once into a bounded in-memory PCM cache; its first request can stall while decoding. Content mounting follows each game's `gameinfo.txt` search path; arbitrary mod/engine search-path behavior remains unverified. Supported map loading does not prove that a map's gameplay works.

## Rebuild and inspect

With Rust stable, rustfmt, Clippy and Visual Studio C++ tooling installed, rebuild using:

```powershell
.\scripts\build-bevy.ps1
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all --check
.\launch-bevy.cmd --movement-script test-inputs/bevy-movement.json --capture artifacts/check-movement.png --report artifacts/check-movement.json
```

`build-bevy.ps1` replaces the executable used by the launchers; changing source or committing it does not rebuild that executable. Use `-DebugBuild` only when deliberately packaging a debug build. `bin/build-bevy-info.json` records the packaged executable and shader fingerprints. Runtime captures/reports are written in ignored `artifacts/`.

## Research boundary

The separate native research folder for this checkout is `../../work/hl2-decompiled`. Its private binary copies, Ghidra databases, C pseudocode, indices and research scripts are outside this project. It is a reference for implementing behavior; none of its native code is loaded or compiled into the Rust runtime. Native exports are approximate pseudocode, not recovered original source.

Development used the universal-modder Codex plugin and ten skills, version 0.2.0. They are not runtime or build dependencies. Its FAL capabilities are unused. The requested three reference repositories were read and are recorded in [research](docs/research.md). See [validation](docs/validation.md) for concrete checks and [MODLOG](MODLOG.md) for implementation notes.

This is an AI-assisted project developed with OpenAI Codex (GPT-6) and, since 2026-10-06, Anthropic Claude Code (Claude Opus 5.5), with original-game testing and manual review of selected native behavior. It remains an experimental reconstruction. Contributions are welcome; read [CONTRIBUTING](CONTRIBUTING.md) for the code boundaries, checks and useful evidence to include in a PR.

Original project code is MIT licensed. Installed assets and private analysis retain their own rights and are excluded from Git. Dependencies retain their own licenses.
