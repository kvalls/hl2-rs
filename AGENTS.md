# AGENTS.md

## Project

HL2-RS is an experimental Rust rewrite of Half-Life 2 and relevant Source behavior, using content from an owned installation. It is not a passthrough mod and does not load Valve game/engine DLLs. Complete campaign and 1:1 fidelity have not been achieved.

## Hard rules, never break these

1. Keep game assets, native binaries, decompiled code, databases and research tools outside public source checkouts. Read installed content in place; binary analysis uses private copies. Generated screenshots/reports belong in ignored artifacts/.
2. Keep .gitignore as a source-only whitelist. Add narrow exceptions for reviewed source, documentation, fixtures or explicitly licensed project resources. Never force-add extracted game content or generated executables.
3. Commit/push only authorized work and stage only the task's files. The owner already authorized reviewed source commits/pushes to kvalls/hl2-rs, including milestones and checkpoints near 98% of the five-hour quota. Do not repeatedly ask. Work on main (Bevy-only since 2026-10-07); keep experimental or unverified work on separate branches until it is validated.
4. Keep the installed game read-only. Approved private working directories are workspace work/hl2-decompiled, work/references and work/publishing. Other unrelated files remain outside scope unless named by the owner.
5. Do not put credentials, tokens or passwords in project files, logs or commits.
6. No FAL, replacement assets, crossovers or unsolicited Discord messages. This phase reconstructs HL2. Mods and crossovers with other Rust game rewrites are the long-term goal after 1:1 fidelity (owner, 2026-10-07); keep `modkit-core::ModPlugin` and shared crates mod-friendly. There is no single-player/offline-only project restriction; multiplayer/netcode fidelity is not implemented or tested.
7. Fonts come only from the owned installation's VFS (`resource/*.ttf`, `platform/resource/linux_fonts/*.ttf`) or the MIT `ProggyClean.ttf`; never read, bundle or depend on host OS fonts (`C:/Windows/Fonts`, `/System/Library/Fonts`, `/Library/Fonts`).
8. User-visible text comes only from installed localization files through `Localization::text("#Token")`; no English literals in UI or HUD code. In-repo text only where no installed token exists: an `HL2RS_` placeholder, or `FALLBACK_STRINGS` for screens shown before resources are mounted (setup, abort dialogs). Details: docs/gameui.md section 0.
9. Never patch third-party crates (Bevy, winit, Rapier...). Read their code only after their docs and this codebase fail to explain something; adapt on the HL2-RS side.
10. This repository is self-contained: its files and commit messages never name anything outside it (no workspace folders, scripts, notes, `../` or machine paths); a check that lives elsewhere is described by what it does. The installed game is described by its layout (`hl2/resource/...`), the Source SDK by SDK-relative paths (Source SDK `game/client/...`).
11. Never slow the game through careless code or a poor design (owner, 2026-10-24): a requested feature or fidelity change may cost frame time, waste may not (work done while its option is off, per-frame allocation or file access, repeated work that could be cached, a heavier solution than needed); measure hot paths and new render passes, record frame time before and after in MODLOG, and reduce a cost larger than the design needs instead of dropping the feature.

12. Keep the runtime cross-platform and document every separately installed runtime prerequisite in README.md before introducing it; never silently depend on external applications or research tools.

## How to work

- Read first only what the task needs: STATUS.md's first section (current state), the docs/DESIGN.md section of the plan at hand and, for UI work, docs/gameui.md section 0. README.md, docs/research.md and docs/bevy-migration.md are references: search them instead of reading them whole. Machine-local detail is in workspace work/publishing/continuation.md; its newest checkpoint supersedes history.
- Current priority (owner decision 2026-10-06): the Rust rewrite resumes with authored NPC gestures. Whole-corpus coverage/discovery passes are paused after pass14. Decompile on demand: for each feature, retrieve the relevant native functions from the private pass14 index by path/hash/address, review them privately and record findings before implementing. Preserve the baseline catalogue; indexing or relaxed-metadata exports do not establish semantic parity.
- Shared work: STATUS.md "Claims" lists items owned by other contributors (GitHub issue assignments and open PRs). Before starting an item, check open issues/PRs and that list; skip claimed items and avoid editing the files they touch. Ask the owner before claiming new items publicly.
- For substantial changes, record a bounded design/validation plan in docs/DESIGN.md before coding. Keep steps independently reviewable and reversible.
- Use instrumentation and visual evidence together. The owner explicitly authorizes testing, screenshots and computer use. Record positions, timings, state transitions and image comparisons. Test packaged executables. Do not close user-launched games or send input to unrelated apps.
- Reuse shared implementations: see "Architecture rules" below. No blocking asset reads/font rasterization in frame systems; runtime loads (late precache for spawned entities, map changes) go through asynchronous workers and are inserted when ready. Entities can be created at runtime (owner, 2026-10-12): Source creates them freely (CreateEntityByName, CC_NPC_Create, npc_maker), and mods/sandbox play need it.
- HL2-RS is a Bevy Rust rewrite. Owner decision 2026-10-07: bevy-migration merged into main and the former Macroquad host (hl2-runtime) was removed; continue on main. Ask before materially different large refactors. Preserve the macroquad-prototype (historical host), bevy-migration (frozen at the merge) and unverified wip/scripted-scenes branches.
- Preserve 15 ms order: sample input first; scene -> weapons -> entity collider poses/query refresh -> NPC/projectiles -> rigid physics -> player. Publish presentation before transform propagation and current animation bounds before visibility. Preserve held input, pause and selection semantics.
- Build with scripts/build-bevy.ps1 and run launch-bevy.cmd (or the 1080p/borderless variants). Keep generated executable/shader metadata current after runtime changes.
- Local checks (no arguments): `scripts/checks/validate.sh` (architecture check, tests, strict Clippy, fmt) and `scripts/checks/check_windows.sh` (strict Clippy for the Windows target from macOS/Linux). Packaged checks: `scripts/checks/replays.py` (in the background and muted, from a copy of the game with default settings; `ONLY=`, `JOBS=`, `WINDOWED=1`). Run the replays before and after merging main; a changed gameplay outcome is a regression to fix, or an intended change recorded with `UPDATE_EXPECTED=1` after review. New replays go into `test-inputs/` with assertions on the report. Details: docs/validation.md.
- Use Cargo.lock. Run relevant tests, strict Clippy and formatting; verify parsers against owned files and rendering through packaged captures. Give exact test commands and expected results. Documentation-only edits require document/whitelist checks, not gameplay rebuilds.
- Owned-install tests (`#[ignore]`, `cargo test -- --ignored`) find the game with `source_assets::install::discover()` like the runtime; never require HL2_ROOT.
- Windows is the primary tested platform, macOS a contributor platform. Gate platform-specific retail restrictions (`[$OSX]` `.res` conditionals) per platform, never globally; never rewrite stored settings because a platform lacks them. macOS display modes: docs/gameui.md section 0.
- On sandbox/offline barriers or fundamental design choices, ask the owner instead of looping.
- Keep window presentation size and the internal 3D render-target resolution distinct (render lower, upscale).
- Retail behavior of the owned game wins unless the owner asks for a deviation. Use SDK references within their license and private binary analysis where legal; implement independently in Rust, never copy or mechanically translate Valve code, never use leaked source.
- Research database/native exports are evidence, not automatic parity. Distinguish inferred names/signatures, SDK behavior and verified retail behavior. Seeded developer actors do not establish ordinary campaign completion.
- Explain changes plainly: actual behavior, verification, limitations and next bounded step.

## Architecture rules

Where code goes:
- source-assets reads file formats. modkit-core holds contracts, the player and pose math. hl2-simulation holds gameplay, physics, entities and NPCs, with no engine. source-ui is the game-agnostic UI toolkit (like the SDK's `vgui_controls`) and knows nothing of HL2. hl2-ui is HL2's UI (GameUI, HUD, captions, `Config`, console commands) over source-ui and hl2-simulation. hl2-bevy is the Bevy host only.
- Depend only downward: modkit-core <- source-assets <- source-ui; modkit-core, source-assets <- hl2-simulation; source-ui, hl2-simulation <- hl2-ui <- hl2-bevy. Bevy and rfd only in hl2-bevy; rapier3d, symphonia and hound only in hl2-simulation. A new crate, a new edge between crates or a new engine dependency needs the owner.
- modkit-core, hl2-simulation and source-ui never touch the file system: bytes come through source-assets' `Vfs` or from the host. source-ui takes HL2's text, settings and looks through traits or values.

Keep it small:
- Functions up to 100 lines; a file's production code up to 1,000 lines, its tests up to 2,000. When you touch code already over a limit, do not make it longer: first move what you add (or a piece of what is there) into a new function or module.
- One concern per module. When a file grows, split it by concern into child modules; large test modules go into a `tests.rs` beside the code.
- Group related parameters into a struct instead of allowing `too_many_arguments`; keep state in an owned struct or a Bevy resource, never a new mutable static; no new glob imports except preludes; delete dead code instead of allowing it.

Say it once:
- Before writing a helper, search for one and reuse or extend it. Never copy a block: extract a shared function or a table.
- These are described once and dispatched; add to them, never put a special case beside them: console commands and cvars (`console::commands::COMMANDS`, `setting` for stored cvars), entity inputs (`entities::inputs::HANDLERS`), widgets, dialog frames and placement (`source_ui::widgets`, `Painter::frame_spec`, `placement::Placement`), keyboard focus (`widgets::focus_step`, `KeyPress`), key repeat (`input::KeyRepeat`), scroll bars (`widgets::ScrollBarState`).
- New behavior gets a test in the crate that owns it; source-ui tests itself with small fake apps (`shell::tests`).

`scripts/checks/architecture.py` (in validate.sh, CI and the pre-commit hook) fails on new breaks of the size, layer, file-system, duplication and allowance rules. Existing ones are listed in `scripts/checks/architecture-baseline.json` and may only shrink: after an improvement run `UPDATE_BASELINE=1 python3 scripts/checks/architecture.py`; `ACCEPT_GROWTH=1` only after merging main or with the owner's approval, noted in MODLOG. Fix the code, never the limit.

## GameUI, HUD and localization

Read docs/gameui.md section 0 before UI work; in one line each:
- Text from installed tokens; reuse an installed token before adding an `HL2RS_` placeholder; escapes only at the parser boundary (`Localization::add_resource` / `insert`); decode VFS text with `source_assets::keyvalues::decode_text`; keep `audit_no_hardcoded_user_visible_strings_in_ui_code_outside_fallback_constants` passing.
- Scale UI through `proportional::design_height` (`dialog_scale`, `scale`), never the raw canvas height.
- A game's search path is its own `gameinfo.txt`; a map is `(game, map)`, never a name alone; no map tables or name-prefix guessing.
- Widgets: the rollover sound only on menu buttons (`WidgetKind::has_rollover_sound`); button looks through `Theme::button_style`; dialog controls use `ButtonTracker::capturing()`; popup lists close or select on release; occluded layers call `ButtonTracker::occlude`.
- Dialogs: frames through `Painter::frame_spec` (rounded, clipped, no outer 3D border); move and resize only through `Placement`; size fixed-content dialogs to their content, never clip or scroll them.
- Background maps come from game files (`gamemenu.res`, `chapterbackgrounds.txt`, `cfg/chapter<n>.cfg`); a failed load only logs; Back and Escape keep the expansion's background; the pre-blurred image, backdrop and plaque show until the transition finishes or fails.
- Options and the console are non-modal windows (`source_ui::windows`); only the quit/return box and game loading are modal; `Console::set_mode` is the only way to change the focused layer.
- Options use `settings::support` for platform availability and cyan development status; remove the status when a feature is finished, preserve saved values and existing orange hints.
- Menu titles and buttons draw `TextOutline::DEFAULT`; subtitles use the installed title fonts.

## Honesty

- Write **not tested** when applicable. Distinguish unit tests, owned-file checks, controlled replays, native comparisons and ordinary playthroughs.
- After two unsuccessful real attempts, stop repeating that approach and record evidence in STATUS.md. Choose a materially different evidence-based approach; continue independent work rather than retrying random variations.
- Record failures/rejected captures alongside successes. Never claim complete decompilation, 1:1 fidelity or FPS guarantees without verification.

## Keep these files updated

- MODLOG.md: newest first; Agent/model / Changed / Why / Tested how / Result / Still broken or not tested / Next. Record the agent application and exact model identifier when available; distinguish contributor work and materially involved subagents. Never guess a model from the date or the current chat. Use "not recorded" for historical entries without evidence, and state when only a model family is known.
- STATUS.md: verified revision/builds, current problems, evidence, rejected approaches, reproduction commands and next work.
- docs/DESIGN.md: architecture and substantial plans. README.md: tested capabilities/limitations. docs/validation.md and docs/performance.md: detailed evidence.
- Private work/publishing/continuation.md: precise local checkpoint before quota exhaustion or handoff. When the owner requests a fresh chat, finish the handoff and await further instructions here instead of continuing implementation in this chat.
- This file: rules only, one line each, in the section they belong to; details, findings and history go to docs/ (UI: docs/gameui.md section 0) and MODLOG. It stays under 20 KB (the architecture check enforces it).

## Environment

- Windows 11 Pro 64-bit, observed version 10.0.26200, is the primary tested platform. macOS Apple Silicon (aarch64, Metal) was tested by a contributor (@OrbitalMan, PR #1: build, tests and the packaged build running d1_trainstation_02 with its scripted dispenser, Metrocop and queue sequences on an M1 Pro); other platforms **not tested**.
- Owned Steam Half-Life 2 app220, tested build19307283/patch9912070; other versions **not tested**. Default installation: C:/Program Files (x86)/Steam/steamapps/common/Half-Life 2; discover another library or use HL2_ROOT.
- Rust1.99.0 stable MSVC, Visual Studio C++ tools/Windows SDK. Bevy0.19.1/wgpu and direct Rapier0.26.1. Switching to Avian was not requested; comparative physics performance is not tested.
- Loader: none. Game owning the player: not applicable; Rust simulation owns state and the installation supplies content.
- Agent: Claude Code desktop (2026-10-06/07 sessions, Claude Opus 5.5); earlier runs used Codex Desktop with GPT-6.1 Extra High/Normal Speed. Select new-chat settings explicitly.
- Active checkout: workspace outputs/hl2-rs-bevy on main. Sibling outputs/hl2-rs is a detached historical checkout of the former Macroquad main (b4b1731).

## Runtime invariants to preserve

- Walking uses shared Player/Physics; F2/--fly enables flight. Escape cancels selection before pause; tilde opens console. Consume transition input and pause simulation/audio. Map workers freeze fixed/script clocks; replace map-owned resources only after success. Missing maps preserve the world; saved/global state transfer is unfinished.
- Startup: a loadable `--map` starts gameplay; otherwise the main menu runs over the live chapter-1 background map (player input ignored, seen through its `point_viewcontrol`); map-dependent arguments without a loadable `--map` fail the launch. Escape never turns a background level into gameplay. Installation: `--game`/HL2_ROOT, then the remembered folder, then Steam discovery; headless tests, scripted runs and binaries outside `bin/` pass `--no-setup`. Details: docs/gameui.md section 0.
- HUD uses owned resources and an ordered CPU canvas. GDI font rasterization is setup-only.
- Closed captions: text only from the installed `resource/closecaption_<language>.dat` dictionaries, never inline literals (docs/gameui.md section 0).
- Sky background layer3/order-2, miniature sky layer4/order-1, world layer0: independent depth/color clears and BSP 2D/3D eligibility. Current animation bounds and all active player/monitor views drive visibility; missing data fails open, script-hidden/killed entities remain hidden. Spatial splitting is opt-in.
- Eyes use owned MDL records, animated eyes attachment origin/forward and iris projection before entity transform/scale. Preserve authored LOOKAT ramps, cancellation tails, pause refresh, cycler meshes and one target for both eyes. EyeRefract uses explicit Eyes_dx8 fallback; native head/facial/random/tactical attention is unfinished.
- Door use carries opener origin, opens linked leaves away and honors explicit direction. Blocked reversal/arbitrary spawn/native linkage remain incomplete.
- Native HL2 intentionally skips d2_coast_02: the campaign goes from d2_coast_01 to d2_coast_03 (owner confirmed 2026-10-16; native console loading d2_coast_02 also errors). The local zero-byte d2_coast_02.bsp is an expected rejected stub, not a missing campaign level or rewrite regression; exclude it from campaign-completion denominators and do not reconstruct it. Keep station03's known materials distinct from new regressions. Research references are not integrated iw4L crossovers.

Template basis: [AGENTS starter](https://github.com/trevaintdead/ai-game-modding-guides/blob/main/templates/AGENTS-starter.md), adapted to existing owner authorization and this rewrite.
