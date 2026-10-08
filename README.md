# HL2-RS

A standalone, partial Rust reconstruction of Half-Life 2 that reads maps, models, textures, animations and sounds from an installed copy. It does not load Valve's game or engine DLLs. The full campaign is not playable yet.

## Platforms

| Operating system | Status | Evidence |
| --- | --- | --- |
| Windows 11 64-bit (x86_64) | Primary tested platform | Owner builds, unit and owned-file tests, packaged replays/captures and native-game comparisons; CI runs on `windows-latest`. |
| macOS 15+ on Apple Silicon (aarch64, Metal) | Tested by a contributor | On an Apple M1 Pro: build, tests, and the packaged build (`./scripts/build-bevy.sh`, `./launch-bevy.sh`) running `d1_trainstation_02`, including the scripted dispenser, Metrocop sequence and queue, with exit code 0 ([PR #1](https://github.com/kvalls/hl2-rs/pull/1)). Replays, captures and native comparisons are **not tested**. |
| Linux | **Not tested** | Install discovery looks in `~/.steam/steam` and `~/.local/share/Steam`, but neither a build nor the POSIX scripts have been run on Linux. Reports are welcome. |
| macOS on Intel, other systems | **Not tested** | |

## Branches and contributing

HL2-RS is a Bevy 0.19 / wgpu Rust rewrite. Owned asset readers, shared simulation and rendering are separated following [iw4L](https://github.com/vladtrc/iw4L). On 2026-10-07 the Bevy work was merged into `main` and the original Macroquad/OpenGL host was removed from it.

| Branch | Purpose | Pull requests |
| --- | --- | --- |
| `main` | The Bevy/wgpu rewrite: shared movement, entity/weapon simulation, choreography, animated presentation, model lighting, owned HUD/audio, sky and monitors. | Target this branch. |
| [macroquad-prototype](https://github.com/kvalls/hl2-rs/tree/macroquad-prototype) | Preserved snapshot of the original Macroquad prototype. | Historical reference only. |
| [bevy-migration](https://github.com/kvalls/hl2-rs/tree/bevy-migration) | The migration branch, frozen at the point it merged into `main`. | Historical reference only. |

`source-assets` reads Source formats, `modkit-core` owns shared contracts, player and pose math, `hl2-simulation` owns gameplay, physics and choreography, `hl2-ui` owns the HUD, menus and console, and `hl2-bevy` is the host. Bevy's default PBR materials do not recreate Source shaders or physics; those have their own implementations and comparisons. No speed or fidelity improvement is assumed solely from the engine. See [CONTRIBUTING](CONTRIBUTING.md) before opening a PR.

Build with `scripts/build-bevy.ps1` and run `launch-bevy.cmd`. Use `launch-bevy-1080p.cmd` for a 1920x1080 window or `launch-bevy-borderless.cmd` for the primary desktop resolution. The executable defaults to walking with the shared 15 ms `Player` controller and `Physics` collision queries; `--fly` or F2 enables flight. Escape opens the pause menu and releases the cursor; select Resume to continue. Tilde opens the developer console. Doors and physics props move with their colliders, entity clips and weapon viewmodels animate, and scene/weapon/projectile logic runs in the fixed step. F3 gives the developer loadout; E uses doors, slots/mouse wheel select weapons and mouse buttons fire/confirm. Owned ambient, scene, weapon and HUD cues play through Bevy audio. Owned sky backgrounds (HDR faces when present), miniature scenery, model-driven iris projection, facial flexes, lip sync and live `_rt_Camera` monitor feeds are rendered. Authored changelevel triggers carry landmark-relative position and inventory; console `map` starts fresh. Save/global entity state and complete player transfer remain unfinished. Tick-based `--movement-script` fixtures record player/gameplay state and capture the final frame. For unattended runs, `--volume 0.01` sets the master volume and `--no-focus` opens the window without taking focus. See [Bevy instructions and milestones](docs/bevy-migration.md).

GPU skinning and conservative BSP PVS culling (alongside Bevy frustum culling) keep the tested station02 spawn at about 157-175 FPS uncapped at 1080p. Other views, native performance parity and sustained 144 FPS remain unverified; see [performance measurements](docs/performance.md). `launch-bevy.cmd --uncapped` presents without VSync for testing; `--profile` records bounded CPU stage measurements.

![HL2-RS Rust runtime rendering the trainstation with pistol, HUD and development diagnostics](docs/images/hl2-rs-trainstation.png)

Capture from the packaged Rust build on `d1_trainstation_02`, with F1 diagnostics and a developer weapon loadout. This is a rendering preview, not evidence of completed campaign gameplay. The screenshot depicts owned HL2 content; distributable game assets are not included.

Model-authored activity lookup now repairs unsupported implicit NPC idle labels, including Kleiner, while preserving explicit defaults and existing loaded poses. Scene GESTURE events now play as layered animations (Source delta/post layers, bone masks, autolayers, Faceposer tag timing) for actors such as Barney and Kleiner. A side-by-side comparison with the original game is in progress: Barney now spawns from his point_template and opens doors on his way to the desk, but NPC-touch triggers are still missing. Native AI scheduling, head/facial animation, lip sync and first-level staging remain incomplete.

## Build a fresh checkout

You need an owned, installed Steam PC copy of Half-Life 2, Windows 64-bit, Rust stable with the MSVC toolchain, and Visual Studio C++ build tools with a Windows SDK. The renderer uses Bevy/wgpu (tested on Windows with the default backend); GPU and RAM minimums have not been measured. On macOS use `./scripts/build-bevy.sh` and `./launch-bevy.sh`; see [Platforms](#platforms) for what each system has been tested with.

The tested installation is Steam app 220, build `19307283`, patch `9912070`. Other game builds are unverified. Rust `1.99.0` was used for the recorded Windows checks. Cargo resolves the library versions in `Cargo.lock`; no external mod, loader or Source engine runtime is needed.

Private research now includes 42 selected native modules, 122,721 saved analysis entries and 10,752 separate temporary discovery entries. The audited consolidated database holds 133,473 distinct observed addresses with some pseudocode and preserves exact bytes, original failures and export variants. Reviewed names/types, Rust implementation and native verification remain separate; this research sets no completion flags. Full runtime scope, discovery, ABIs and behavior parity remain unverified. All native binaries, code, tools and databases remain outside this repository. See [research evidence](docs/research.md).

```powershell
git clone https://github.com/kvalls/hl2-rs.git
cd hl2-rs
.\scripts\build-bevy.ps1
.\launch-bevy.cmd
```

The repository contains source, not a prebuilt executable. Building creates `bin/hl2-bevy.exe` and its shader folder `bin/bevy-assets`. If PowerShell blocks the local build script, invoke `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-bevy.ps1` for that process. To uninstall, remove the checkout; the game installation is read-only.

## Play the current build

Double-click `launch-bevy.cmd` in this folder. It launches `bin/hl2-bevy.exe`, the packaged release build. Rust is needed only for rebuilding. The default map is `d1_trainstation_02`.

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

The pause menu has working Resume Game, Developer Console and Quit controls. The console supports history, editing and command-name completion with Tab. `help` lists the implemented commands. Enter `sv_cheats 1; impulse 101` to get the six implemented weapons, ammunition and suit. Other supported commands include `noclip`, `getpos`, `setpos`, `setang`, `map`, `ent_fire`, `find`, `echo`, `clear`, `toggleconsole` and `quit`. `ent_fire <target> [input] [parameter] [delay]` sends normal entity I/O to named targets; its console delay uses whole seconds. Classname fallback and the full Source command registry are unfinished. Unsupported commands report an error. Save/load, options and bindings are unfinished. Player, weapon, entity and prop simulation pauses while the menu or console is open; ambient audio does not yet pause. The UI uses installed GameUI labels/colors and system fonts, but its layout is not an exact VGUI reconstruction.

## Implemented so far

In short: owned VPK/BSP/VMT/VTF/MDL/PHY/audio readers; world, displacement, prop and model rendering with baked lightmaps, Source model lighting, HDR auto exposure, bloom, skies and live monitors; fixed-step player movement and collision; entity I/O, doors, triggers and choreographed scenes with layered gestures, facial flexes and lip sync; six weapons (crowbar, pistol, .357, SMG1, AR2, shotgun) with their HUD; and map transitions. The full item-by-item list is in [docs/features.md](docs/features.md).

## Fidelity work remaining

This is not a completed rewrite of Source. Major gaps include general NPC navigation/schedules/combat, the remaining arsenal, recoil/muzzle flashes/tracers/shell ejection, exact prediction spread and shotgun pellet hull traces, the full player damage/death pipeline, vehicles, save/load, campaign state transfer, complete choreography movement/loops/subscenes/dialogue/lip sync, non-sound animation events/blends/layered gestures/IK/flexes, ragdolls, dynamic/animated PHY collision, moving-platform behavior and blocked-door handling. Scripted movement and parenting/attachments are incomplete. HUD damage-message dispatch, history/aux-power panels, zoom/convar gates and additional VGUI animation commands remain unfinished. Projectile collision/material response, relationship and damage filters, water rejection, guided targeting, dissolve visuals and explosion particles do not reproduce the full native behavior.

Water/ladders, surface movement modifiers, exact crouch transitions and prediction need further work. The renderer lacks Source's full lighting/material system: an LDR mode, bumped diffuse lighting and $selfillum, envmaps on models/brush entities (world LightmappedGeneric `$envmap` reflections with masks and bump normals are implemented), light probes, shadows, sky polygon masking, sky fog, fog, water/refraction, dynamic lightstyles and many material proxies. Symbolic speech now uses the owned actor-model gender registry and registered wave alternatives. Audio still lacks streaming, spatial mixing, soundscapes, DSP, HEV sentence scheduling and other compressed formats. An MP3 is decoded once into a bounded in-memory PCM cache; its first request can stall while decoding. Content mounting is not a general `gameinfo.txt` implementation. Supported map loading does not prove that a map's gameplay works.

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
