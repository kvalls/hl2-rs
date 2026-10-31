# Plugins

HL2-RS plugins are ordinary source-built Rust crates selected when the executable is built. There is no native library loading, hot unloading, scripting engine or package manager.

## Structure

Dependencies point one way: application (`apps/hl2rs`, package `hl2-app`) -> plugins (`plugins/*`) -> core (`crates/*`).

- Core never depends on, imports, feature-gates or registers a concrete plugin; `scripts/checks/architecture.py` enforces those names. It cannot detect feature knowledge under other names, so AGENTS.md's plugin rules and review of `git diff -- crates/` cover that.
- The application owns the features `advanced-options` and `debug-tools`, both on by default, and injects what they select in `apps/hl2rs/src/main.rs` and `plugins.rs`.
- A plugin's portable code stays free of Bevy. Its optional `bevy` feature holds the host adapter, which may use the public `hl2-bevy` API.
- `hl2-simulation` stays independent of any engine.

| Plugin | Owns |
| --- | --- |
| `hl2-advanced-options` | Advanced and Ultra motion blur, Fit Display and the extra aspect choices, the 15-second keep-or-revert dialog (on an aspect-choice change), vertical and viewmodel FOV |
| `hl2-debug` | `viewmodel_debug_mode`, `background_debug_mode`, F1 statistics, F2 flight shortcut, F3 loadout shortcut, F4 NPC and prop panel, their tuning and presentation |

Ordinary motion blur, flight, spawning, camera behavior and simulation mechanisms stay in core.

## Contracts

| Contract | Location | Contributes |
| --- | --- | --- |
| `hl2_ui::extensions::Extensions` | per application instance | Console commands (a duplicate name is an error), layout addenda, localization placeholders, default bindings, after-apply handlers |
| `hl2_ui::extensions::VideoExtension` | through `VideoPolicy` | Frame and window sizing, projection, aspect and mode lists, blur selection and sliders (`CustomValue`), aspect choices (`AspectChoice`) |
| `hl2_ui::extensions::FramingProfiles` | `VideoExtension::framing_profiles` | Viewmodel framing profile ids and values; without one, every aspect keeps the retail view |
| `hl2_ui::config::StoredExtensions` | `video.extensions`, `video.advanced.extensions` | Plugin settings saved beside core fields; unknown keys survive, so old files keep their format |
| `hl2_bevy::extensions::RuntimeExtension` | `Application::runtime` | Input, UI, presentation, loading, map and config lifecycle, replay events, reports |
| `Application::install` | `fn(&mut App)` | Bevy systems, render nodes and resources |

Runtime extensions are in place before startup settings are applied, so `startup` and the first `level_target` reach them. Their state belongs to the instance; do not add mutable globals.

Installed localization wins over a plugin's placeholder. Saved preferences stay in `Config` when a plugin is omitted, and video policy is never serialized.

Opaque events (`Effect::Extension`, the replay `extension` field, UI `Action::Extension`) must be claimed by returning `true`. An event no selected contributor handles is listed in `extension_rejected` in the report and fails the run.

## Known gaps

- `RuntimeExtension` is broader than the project's target: `Context` gives mutable access to `Simulation`, `Gameplay` and the console. Treat it as unstable.
- One video policy can be selected; `Extensions::select_video` rejects a second. Selecting among several video contributors is not implemented, and `framing` and `projection` take the first contributor's answer.
- There is no declared-dependency or capability validation between plugins, because the two current plugins are independent.
- `cargo run` without packaging has no `weapon_blur.wgsl` in its asset root, so run the packaged build for Advanced or Ultra blur.
- Plugin developer text in the F4 panel and debug overlays still has English literals.
- The window-following frame, the projection parameter and the background-selection services stay in core as generic mechanisms.
- `modkit_core::ModPlugin` is an older, unused simulation contract, not this mechanism.

## Workflows

**Add a plugin.** Create `plugins/<name>` with a portable `register(&mut Extensions)` and, if needed, a `bevy` feature with its runtime extension or `install` function. Add a workspace member and an entry in `LAYERS`. Add an optional dependency and feature to `apps/hl2rs/Cargo.toml`, and inject it in `main.rs` and `plugins.rs`. Add it to `scripts/checks/check_plugin_selection.py`. Extend the `.gitignore` whitelist narrowly for any new resource type. Core should not need an edit unless a contract is genuinely missing; change the contract generically in that case.

**Change or fix a plugin.** Edit the owning plugin and put the test beside the code. Portable behavior is tested without Bevy; adapter tests need the plugin's `bevy` feature, for example `cargo test -p hl2-debug --features bevy`.

**Enable or disable.** Choose at build time:

```bash
scripts/build-bevy.sh --plugins none
```

```bash
scripts/build-bevy.sh --plugins advanced-options
```

The default is both. Windows uses `scripts/build-bevy.ps1 -Plugins <list>`. A plugin shader is packaged only when its plugin is selected.

**Replace a plugin.** Select another crate that provides the same contracts in the application's manifest and composition files. Core does not change. `crates/hl2-bevy/src/extensions.rs` tests an independent contributor this way.

## Checks

`scripts/checks/check_plugin_selection.py` builds core-only, each plugin alone and both, and fails if an unselected plugin appears in a dependency tree. It runs in `scripts/checks/validate.sh`.
