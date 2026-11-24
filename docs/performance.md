# Performance measurements - 2026-10-05

## `O42` — `LightingIndex`, `32 x 32 x 128` light-cache cell sharing, and `MeshTag` lighting deduplication (2026-11-24)

Measured on Apple M1 Pro / Metal (`advanced-options,debug-tools`, everyday release baseline `sha256 fa4db9f9` vs `O42` `sha256 f8f85a8d`):

- **Changes:**
  - `crates/modkit-core/src/lighting/{accel.rs,brushes.rs}`, `crates/modkit-core/src/lighting.rs`, `crates/source-assets/src/model_lighting.rs`: added `LightingIndex` preserving exact `state_at` output (`active_lights` pre-filters baked `DWL_FLAGS_INAMBIENTCUBE` and `SkyAmbient` lights and tightens conservative `max_dist_sq`, keeping `51` of `1,941` lights on `d1_trainstation_02`), replaced per-call `kept: Vec` heap allocations with fixed stack storage, moved `brushes::entry` to a single pass, added conservative brush AABBs + exact axial `entry_aabb`, and built the index inside `model_lighting::read_with`.
  - `crates/hl2-bevy/src/rendering/lighting.rs`: added `StaticLightCache` caching map-scoped `32 x 32 x 128` `cache_cell` centers (`4,096`-entry cap with retail center-trace occlusion fallback), preserving static post-boost `LightState` across transient dynamic `elights`, caching `ModelIllumination` on `ModelLit`, skipping non-reflective `envmap` clones/lookups, and deduplicating shared `MeshTag` draws across both updated and skipped paths.
- **Owned-map bit-exact parity & microbenchmark (`cargo test --release -p source-assets --locked owned_station02_indexed_lighting_matches_unindexed_and_benchmarks -- --ignored --nocapture`):**
  - `d1_trainstation_02` (`1,941` world lights: `1,907` surface, `10` point, `22` spot, `1` sky, `1` sky-ambient, `51` active; `1,903` brushes): `200/200` sample points matched `unindexed.state_at(p) == indexed.state_at(p)` bit-for-bit; `500` calls `20.90 -> 15.13 us/call` (`1.38x`).
- **Headless `--uncapped --profile` A/B (1 discarded warm-up + 3 alternating `A-B-B-A-A-B` pairs per scene, `fa4db9f9` vs `f8f85a8d`, medians & spreads in ms):**
  - **`d1_trainstation_02` `test-inputs/performance-research/mixed-crowd.json` (maxed Retina `3456x2168`):**
    - `model lighting` mean (median): **`0.419 -> 0.082 ms` (`-80.49%`, `-0.337 ms`)** (`spread 0.022` / `0.005`), `p95` **`0.851 -> 0.163 ms` (`-80.84%`)**.
    - `main_schedule.PostUpdate` mean: `6.236 -> 6.202 ms` (`-0.54%`, `p95 7.793 -> 7.733 ms`); `frame_total` mean (GPU-bound): `12.772 -> 12.942 ms` (`+1.33%`, `p95 15.122 -> 15.260 ms`, `p99 18.0 -> 21.0 ms`).
    - Stamped `--item O42` `capture.py` run (`1,388` frames): `frame_total 13.528 ms`, `PostUpdate 6.347 ms`.
  - **`d1_trainstation_02` `test-inputs/performance-research/mixed-crowd.json` (`320x240` Potato CPU-isolated):**
    - `model lighting` mean (median): **`1.000 -> 0.086 ms` (`-91.38%`, `-0.914 ms`)** (`spread 0.044` / `0.005`), `p95` **`2.526 -> 0.327 ms` (`-87.06%`)**.
    - `main_schedule.PostUpdate` mean: **`3.937 -> 3.175 ms` (`-19.35%`)**, `p95` **`6.462 -> 4.636 ms` (`-28.26%`)**.
    - `frame_total` mean (median): **`7.668 -> 6.757 ms` (`-11.88%`, `-0.911 ms`)**, `p95` **`11.966 -> 10.367 ms` (`-13.36%`)**, `p99` **`16.0 -> 14.0 ms` (`-12.50%`)**.
  - **`d1_trainstation_02` `test-inputs/performance-research/animated-fire.json` (maxed Retina):**
    - `model lighting` mean (median): **`0.054 -> 0.018 ms` (`-66.45%`)** (`spread 0.003` / `0.001`), `p95` **`0.128 -> 0.064 ms` (`-50.20%`)**; `frame_total`: `12.491 -> 12.366 ms` (`-1.00%`, `p95 16.949 -> 16.783 ms`).
  - **`d1_trainstation_02` `test-inputs/performance-research/quiet.json` (maxed Retina):**
    - `model lighting`: `0.007 -> 0.007 ms` (`-3.00%`, `p95 0.010 -> 0.009 ms`); `frame_total`: `6.435 -> 6.438 ms` (`+0.05%`, `p95 7.633 -> 7.610 ms`).
- **Verification & replays:** `hl2-bevy` (`10` lighting tests), `modkit-core` (`15` lighting tests), `source-assets` (`5` + `2` owned `#[ignore]` tests), `scripts/checks/validate.sh`, `scripts/checks/check_windows.sh`, and **`7/7`** targeted lighting/effect packaged replays (`lamp_emission`, `flashlight_shadows`, `flashlight_viewmodel`, `pallet-gibs-fade`, `entities-weapons`, `face-security-1805`, `particles-smoke`, `33 s`) passed; Windows runtime/linking **not tested**.

## `O39` — GPU storage-buffer quads (`QUAD_ROWS`) and pooled shadow vertex batches (`SHADOW_VERTS`) (2026-11-24)

Measured on Apple M1 Pro / Metal (`advanced-options,debug-tools`, everyday release `O16` `sha256 20136538` vs `O39` `sha256 0560458f`; `--mode trace` `sha256 e3032d13` vs `79573cd7`):

- **Changes:**
  - `crates/hl2-bevy/src/effects/quads/gpu.rs`, `effects/quads{.rs,/batch.rs}`, `effects/material.rs`, `effects{.rs,/initial.rs}`, `lib.rs`, `assets/shaders/{effects,effects_depth}.wgsl`: allocate `QUAD_ROWS` (`4,096 x 6` `Vec4` rows) once at `Startup` (`install_quad_buffer`), cache immutable `RENDER_WORLD` bucket template meshes (`[1, 2, 4, 8, 16, 32, 64, 128, 256, 512]` quads with `[quad, corner, 0.0]` in `ATTRIBUTE_POSITION` and `NoAutoAabb` on `QuadDraw`), pack quad geometry/UVs/color once per frame into `QUAD_ROWS` via `write_rows_range` on dirty ranges, split runs at `512` quads, index slices in `@vertex` via `MeshTag` (`(start << 16) | count`, preserving depth/motion prepass in `effects_depth.wgsl`), keep `QuadDraws` disjoint from `GrenadeDraw` (`Without<quads::QuadDraw>`), and fall back to `MeshTag(0)` CPU meshes on overflow (releasing `cpu_mesh` on GPU recovery or empty batch).
  - `crates/hl2-bevy/src/shadows/draw.rs`, `shadows{.rs,/caster.rs,/material.rs,/texture.rs}`, `assets/shaders/shadow.wgsl`: allocate `SHADOW_VERTS` (`8,192` triangles x `3` verts x `2` `Vec4` rows) once at `Startup` (`install_shadow_buffer`), consolidate visible shadow decals onto two pooled `ShadowDraw` entities (`atlas` and `blob`) with immutable `[64..=8192]` bucket templates and `NoAutoAabb`, re-sync only when `shown`/`rebuilt`/`use_atlas`/`rect` changes, and fall back to `MeshTag(0)` CPU meshes above `8,192` triangles or if `SHADOW_VERTS` is absent.
- **Headless `--uncapped --profile` A/B (1 discarded warm-up + 3 alternating `A-B-B-A-A-B` pairs per scene, `20136538` vs `0560458f`, medians & spreads in ms):**
  - **`d1_trainstation_02` `test-inputs/performance-research/mixed-crowd.json`:**
    - `render_assets` mean (median): **`1.466 -> 0.539 ms` (`-63.25%`, `-0.927 ms`)** (`A: 1.466 / 1.472 / 1.459`, `B: 0.541 / 0.535 / 0.539`; spread `0.013` / `0.005`), `p95` **`2.583 -> 0.849 ms` (`-67.14%`)**.
    - `frame_total` mean (median): **`13.906 -> 13.706 ms` (`-1.44%`)** (`A: 13.906 / 13.909 / 13.829`, `B: 13.686 / 13.766 / 13.706`), `p95` `16.855 -> 16.744 ms` (`-0.66%`), `p99` `18.0 -> 18.0 ms`.
    - `main_schedule.PostUpdate` mean: **`6.396 -> 6.242 ms` (`-2.41%`)**, `p95` **`8.028 -> 7.641 ms` (`-4.82%`)**; `render_prepare`: **`1.062 -> 1.001 ms` (`-5.79%`)**; `shadows`: `0.832 -> 0.845 ms` (`+1.50%`); `render_graph`: `9.344 -> 10.370 ms` (`+10.98%`, pipelined wait shift while `frame_total` improved).
    - Official stamped `--item O39` `capture.py` run (`1,237` frames): `render_assets` `0.567 ms` mean (`p95 0.876 ms`), `frame_total` `15.128 ms` mean (`p95 18.173 ms`).
  - **`d1_trainstation_02` `test-inputs/performance-research/animated-fire.json`:**
    - `frame_total` mean (median): **`14.144 -> 13.487 ms` (`-4.64%`, `-0.657 ms`)** (`A: 14.144 / 14.386 / 14.081`, `B: 13.421 / 13.487 / 13.488`), `p95` **`19.254 -> 18.625 ms` (`-3.27%`)**, `render_graph` **`11.996 -> 11.191 ms` (`-6.71%`)**.
    - `render_assets` mean (median): `0.314 -> 0.385 ms` (`+0.071 ms`, `+22.56%`; `p95 0.423 -> 0.518 ms`) due to whole-buffer `ShaderBuffer` updates (`QUAD_ROWS` `384 KiB` / `SHADOW_VERTS` `768 KiB`) when active emitters/casters tick; `shadows` `0.183 -> 0.206 ms`.
  - **`d1_trainstation_02` `test-inputs/performance-research/shadow-plaza.json`:**
    - `frame_total` mean (median): `7.238 -> 7.206 ms` (`-0.44%`), `p95` `9.031 -> 8.922 ms` (`-1.21%`); `render_assets` `0.341 -> 0.347 ms` (`+1.83%`, `p95 0.232 -> 0.228`); `shadows` `0.044 -> 0.043 ms` (`-2.62%`).
  - **`d1_trainstation_02` `test-inputs/performance-research/quiet.json`:**
    - `frame_total` mean (median): **`7.161 -> 6.957 ms` (`-2.86%`)**, `p95` **`8.954 -> 8.658 ms` (`-3.31%`)**, `render_graph` `5.153 -> 5.044 ms` (`-2.11%`), `PostUpdate` `1.919 -> 1.863 ms` (`-2.91%`); `render_assets` `0.191 -> 0.232 ms` (`+0.041 ms` from `ShaderBuffer` updates).
- **Headless `--mode trace` A/B (`allocate_and_free_meshes` + `render_assets`, full script, `e3032d13` vs `79573cd7`):**
  - **`mixed-crowd.json` (`867` vs `921` observed trace frames):** `allocate_and_free_meshes` mean **`1.454 -> 0.310 ms` (`-78.71%`)**, `p99` **`3.365 -> 2.332 ms` (`-30.70%`)**, total **`1261.72 -> 285.39 ms` (`-77.38%`)** (`max 4.670 -> 13.899 ms` from initial bucket-template allocation at spawn); `render_assets` mean **`1.658 -> 0.633 ms` (`-61.81%`, `p95 2.678 -> 0.945 ms`)**; `frame_total` mean **`22.157 -> 21.309 ms` (`-3.83%`)**.
  - **`animated-fire.json`:** `allocate_and_free_meshes` mean **`0.282 -> 0.239 ms` (`-15.07%`)**, `p99` **`0.740 -> 0.639 ms` (`-13.64%`)**, total **`177.01 -> 158.95 ms` (`-10.21%`)** (`max 1.276 -> 4.447 ms`); `render_assets` `0.333 -> 0.454 ms`; `frame_total` mean **`14.463 -> 13.720 ms` (`-5.14%`, `p95 20.448 -> 19.017 ms`)**.
- **Visual parity & replays:**
  - Frame-matched `--tonemap-scale 1.0` `1280x720` captures (`20136538` vs `0560458f`): `particles-smoke` `mean_abs_rgb=0.1951 (0.0765%)`, `shadow-plaza` `mean_abs_rgb=0.0173 (0.0068%)` (`67` changed pixels >2 from the F1 overlay frame timing text; visual inspection confirmed identical smoke/embers and Metrocops sidewalk shadow decal).
  - Restored everyday release build (`0560458f`) passed `hl2-bevy` unit tests (`265 passed; 0 failed`), `validate.sh`, Windows-target Clippy, `10/10` targeted effect/shadow replays (`22 s`), and **`109/109`** shared packaged replays (`211 s`, `0` failed). Windows runtime **not tested**.

## `O16` — Split static/dynamic `SplitBroadPhase`, player-shadow rest guard, and zero-alloc hitbox posing (2026-11-24)

Measured on Apple M1 Pro / Metal (`advanced-options,debug-tools` release build, `7a3a53e4` `sha256 ad387905` vs `O16` `sha256 20136538`):

- **Changes:**
  - `crates/hl2-simulation/src/physics/broad_phase.rs`, `crates/hl2-simulation/src/physics/queries.rs`, `crates/hl2-simulation/src/physics.rs`: implemented `SplitBroadPhase` (`impl BroadPhase`) wired into `RecordingBroadPhase`, partitioning fixed colliders without `FIXED_FIXED` into a lazy `static_tree: Qbvh<ColliderHandle>` and keeping only dynamic/kinematic/promoting colliders in `moving_tree: Qbvh<ColliderHandle>`. Active pairs are maintained in sorted per-collider lists for exact two-pointer `AddPair`/`DeletePair` parity (including seamless `Static -> Moving` promotion when a door/brush moves), prediction-distance growth rebuilds, non-finite AABB guards, and zero steady-state per-tick allocations; covered by 7 randomized parity and `PhysicsPipeline::step` tests (`broad_phase/tests.rs`).
  - `crates/hl2-simulation/src/physics/shadow_push.rs`, `crates/hl2-simulation/src/physics.rs`, `crates/hl2-simulation/src/physics/tests/shadow.rs`: extracted `clip_shadow_sweep_velocity` (early-returning without cloning `world_sides.shadow` or running `cast_shape` when `velocity` is zero) and `sync_player_shadow_body` (skipping mutable body borrows and `wake_up = true` calls when translation and zero velocity are unchanged), shrinking `physics.rs` (`3148 -> 3106` lines) and `update_player_shadow` (`238 -> 195` lines) and covered by 2 shadow sleep/wake tests.
  - `crates/hl2-simulation/src/physics/hitboxes.rs`, `crates/hl2-simulation/src/gameplay.rs`: replaced per-tick `studio_hitbox_entities()` `Vec<(usize, String)>` + `String` clones in `pose_npc_hitboxes` with `Physics::pose_studio_hitboxes` sharing `apply_npc_hitboxes`.
- **macOS `sample` (`test-inputs/performance-research/mixed-crowd.json`, 12s @ 1ms, call-graph counts under `simulation::fixed_step`):**
  - `simulation::fixed_step` denominator: `2,219` (A) vs `2,221` (B).
  - `PhysicsPipeline::step`: **`568` (`25.60%`) -> `196` (`8.82%`, `-16.78 pp`)**.
  - `broad_phase` (`BroadPhaseMultiSap` -> `SplitBroadPhase`): **`304` (`13.70%`) -> `24` (`1.08%`, `-12.62 pp`, 12.7x lower)**.
  - `narrow_phase`: **`133` (`5.99%`) -> `43` (`1.94%`, `-4.05 pp`)**.
  - `island_solver`: `99` (`4.46%`) -> `100` (`4.50%`, `+0.04 pp`).
  - `pose_npc_hitboxes`: **`223` (`10.05%`) -> `176` (`7.92%`, `-2.13 pp`)**.
- **Headless `--uncapped --profile` A/B (1 discarded warm-up + 3 alternating `AB/BA/AB` pairs per scene, `ad387905` vs `20136538`):**
  - **`d1_trainstation_02` `test-inputs/performance-research/mixed-crowd.json`:**
    - `fixed_simulation` mean (median): **`5.199 -> 4.347 ms` (`-16.39%`, `-0.852 ms`)** (`A: 5.151 / 5.220 / 5.199`, `B: 4.469 / 4.347 / 4.318`), `p95` **`7.113 -> 6.175 ms` (`-13.19%`)**.
    - `main_schedule.RunFixedMainLoop` mean (median): **`4.875 -> 4.048 ms` (`-16.96%`)**, `p95` **`7.434 -> 6.382 ms` (`-14.15%`)**.
    - `frame_total` mean (median): `13.792 -> 13.729 ms` (`-0.46%`, `A: 12.962 / 13.945 / 13.792`, `B: 12.873 / 13.767 / 13.729`), `p95` **`16.813 -> 16.278 ms` (`-3.18%`)**, `p99` **`19.0 -> 18.0 ms` (`-5.26%`)**; official `--item O16` `capture.py` run: `fixed_simulation` `4.257 ms` (`p95 5.973 ms`).
  - **`d1_trainstation_02` `test-inputs/performance-research/quiet.json` (idle static spawn):**
    - `fixed_simulation` mean (median): **`1.927 -> 1.088 ms` (`-43.55%`, `-0.839 ms`)** (`A: 1.931 / 1.927 / 1.924`, `B: 1.088 / 1.082 / 1.092`), `p95` **`2.291 -> 1.423 ms` (`-37.88%`)**.
    - `main_schedule.RunFixedMainLoop` mean (median): **`0.965 -> 0.567 ms` (`-41.26%`)**; headless GPU-bound `frame_total` remained flat (`7.069 -> 7.085 ms`), shifting freed main-thread time into `main_schedule.outside` (`3.802 -> 4.259 ms`).
- **Correctness & replays:** `hl2-simulation` unit suite (`899 passed; 0 failed; 40 ignored`), `scripts/checks/validate.sh`, Windows-target Clippy, `15/15` targeted physics packaged replays, and all **`109/109`** shared packaged replays (`206 s`, `0` failed) passed; Windows runtime **not tested**.

## O30 GPU facial flexes (2026-11-23)

The packaged O41 release (`c8c336b4`, A) and the revised GPU morph release (`db6ecd38`, B) ran on the Apple M1 Pro with the Metal backend. Both used the advanced-options and debug-tools plugins. The 1280×720 uncapped stage runs used the same default settings and tick scripts. Each scene had one discarded A warm-up followed by three pairs in AB, BA, AB order. Frame and stage times are median run means in milliseconds.

| Scene | Frame total | Animation | Render assets | Flex mesh rewrites/frame | Mesh allocator ms/frame |
| --- | ---: | ---: | ---: | ---: | ---: |
| Mixed crowd, station 02 | 11.203 → 10.037 | 1.409 → 1.177 | 2.540 → 1.004 | 51.015 → 0.000 | 4.565 → 2.096 |
| Security face, station 01 | 3.733 → 3.582 | 0.563 → 0.529 | 0.296 → 0.159 | 13.385 → 0.000 | 0.613 → 0.181 |

Release frame means per pair (A/B, ms) were crowd 10.990/10.043, 11.866/10.037, 11.203/9.986, face 3.733/3.582, 3.716/3.587, 3.734/3.564.

The last two columns use separate trace builds of the same sources (`c181954b` / `97c8e5fa`). Each ran the full script at 2×. The middle half of complete gameplay frames excludes startup and exit. Crowd contributed 65/72 frames and face 756/761. A temporary span counted each flex-driven position-buffer rewrite, with separate spans for CPU reskinning and GPU weight updates. Neither revised scene took the CPU fallback. B updated weights 76.35 times/frame in the crowd and 12.75 in the face replay. The allocator column sums Bevy’s `allocate_and_free_meshes` duration per frame across all meshes.

All four security-face, three gesture and two Breen monitor replay checks passed on B. At normal speed, two captures per build used camera `camera_tv_breen`, froze at tick 1674 and ended at tick 1704. Mean absolute RGB differences (0–255) for the matched AB and BA pairs were 0.048 / 0.232 in the player view and 0.000 / 0.083 in the monitor texture. The first pair also matched render frame 1339 and differed by 0.00005 per monitor RGB channel. The within-build A/A and B/B controls measured 0.004 / 0.180 and 0.000 / 0.083, respectively. Accelerated player-view captures had greater exposure variation (A/A 14.970).

The initial GPU release (`8cad383f`) measured 11.94 → 11.05 ms in the crowd and 4.25 → 4.17 ms on the face replay before the CPU fallback, runtime-template and bodygroup fixes. An initial sandboxed O41 visual capture failed before gameplay because it had no GPU. Windows runtime and ordinary campaign play are **not tested**.

## `O41` — Share non-reflective lit `SourceMaterial`s via `MeshTag` and split `LIGHT_ROWS` storage buffer (2026-11-23)

Measured on Apple M1 Pro across 3 alternating `--profile` release pairs per scene (`28078e30` `sha256 5a7f0038` vs `O41` `sha256 c8c336b4`):

- **Changes:**
  - `crates/hl2-bevy/assets/shaders/source.wgsl`, `crates/hl2-bevy/src/material_slots/lighting.rs`, `crates/hl2-bevy/src/rendering/lighting.rs`, `crates/hl2-bevy/src/rendering.rs`, `crates/hl2-bevy/src/rendering/runtime.rs`, `crates/hl2-bevy/src/map_install.rs`: read per-draw `MeshTag` via `mesh_functions::get_tag(instance_index)` in `@fragment` (`tag > 0` -> `slot = tag - 1`), assign `MeshTag` at spawn time (`1024` slots, initialized with `ModelLighting::fallback`, cleared on map install via `slots.clear_lights()`, released on `RemovedComponents<ModelLit>`) so non-reflective, non-two-texture, non-commentary lit models share `SourceMaterial` handles across entities at both map load and runtime spawns while keeping `present_lighting`, `present_ragdoll_fades`, and `present_render_colors` read-only on `MeshMaterial3d` and `MeshTag` (with `detach_tagged_material` copy-on-write `dynamic.w = 1` on fade/rendercolor, and `copy_asset` fallback for reflective models or slot overflow).
  - `crates/hl2-bevy/src/material_slots.rs`, `crates/hl2-bevy/src/rendering/material.rs`, `crates/hl2-bevy/src/sky.rs`: move lighting slots into a dedicated `@binding(22)` `LIGHT_ROWS` buffer (`1024 x 31` rows = `496 KiB`) separate from animated `MATERIAL_ROWS` (`@binding(21)`, `256 x 4` rows = `16 KiB`), upload `material_slots::upload` `.after(rendering::present_lighting)`, and replace full-buffer `encase` serialization with in-place dirty-range `write_rows_range`.
  - Earlier iterations measured on the way here: an initial prototype using `&mut MeshMaterial3d` / `&mut MeshTag` in `present_lighting` with `2048` slots in the combined buffer regressed `PostUpdate` parallel scheduling (`5.44 -> 6.31 ms` crowd), and putting `1024` lighting slots in the combined buffer (`560 KiB`) regressed `quiet.json` `render_assets` (`0.125 -> 0.136 ms`, `+8.3%`) because animated monitors triggered a `560 KiB` buffer copy every frame; splitting `LIGHT_ROWS` and writing dirty slices eliminated that overhead.
- **`d1_trainstation_02` `test-inputs/performance-research/mixed-crowd.json` (`O25` `5a7f0038` vs `O41` `c8c336b4`, 3 pairs):**
  - Map-load `render.materials`: **`1,886 -> 1,267` (`-619`, `-32.8%`)**.
  - `render_graph` mean (median): **`4.7654 -> 4.5998 ms` (`-3.5%`, `-0.166 ms`)** (`A: 4.7447 / 4.8287 / 4.7654`, `B: 4.6141 / 4.5884 / 4.5998`; `p95 5.7434 -> 5.5496`).
  - `frame_total` mean (median): **`12.2437 -> 12.1551 ms` (`-0.7%`)** (`A: 12.2437 / 12.2796 / 12.1088`, `B: 12.1809 / 12.0420 / 12.1551`).
  - `fight_ms` median: **`11.77 -> 11.66 ms` (`-0.9%`)**; `run_ms` median: **`12.14 -> 12.08 ms` (`-0.5%`)**.
  - `main_schedule.PostUpdate` mean (median): **`5.4131 -> 5.3878 ms` (`-0.5%`)**; `render_prepare`: `0.9561 -> 0.9571 ms`.
- **`d1_trainstation_02` `test-inputs/performance-research/quiet.json` (static spawn with lit props, 3 pairs, `0.03–0.07 ms` spread):**
  - `frame_total` mean (median): **`4.3820 -> 4.3430 ms` (`-0.9%`)** (`A: 4.3835 / 4.3820 / 4.3579`, `B: 4.3911 / 4.3195 / 4.3430`).
  - `fight_ms` median: **`4.37 -> 4.33 ms` (`-0.9%`)**; `run_ms` median: **`4.38 -> 4.35 ms` (`-0.7%`)**.
  - `render_graph` mean (median): **`1.9639 -> 1.9358 ms` (`-1.4%`)**.
  - `render_assets` mean (median): **`0.1239 -> 0.1155 ms` (`-6.8%`**, `p95 0.2368 -> 0.2057 ms` [`-13.1%`]).
  - `render_prepare` mean (median): **`0.6985 -> 0.6944 ms` (`-0.6%`)**.
  - `main_schedule.PostUpdate` mean (median): `1.9824 -> 1.9994 ms` (`+0.017 ms`, offset by `-28 us` `render_graph` + `-8 us` `render_assets`).
- **Fidelity:** `flashlight_shadows`, `flashlight_viewmodel`, `lamp_emission`, `campaign`, `commentary`, `pallet-gibs-fade`, and `entities-weapons` packaged replay checks all pass (`7/7`, 0 failed); `lighting` (`7/7`) and `material_slots` (`8/8`) unit tests pass.

## `O25` — Replace `update_mesh_previous_global_transforms` in `PreUpdate` to skip inactive motion views and static world meshes (2026-11-23)

Measured on Apple M1 Pro (400-frame headless `--mode trace` A/B capture on `d1_trainstation_02` with `test-inputs/performance-research/quiet.json` and Ultra motion vectors active, baseline `1177db81` vs `O25`):

- **Root cause fixed:** Bevy's default `bevy_pbr::prepass::update_mesh_previous_global_transforms` ran whenever any `Camera3d` was active (even when `MotionVectorPrepass` was disabled), inserted `PreviousGlobalTransform` via `Commands` onto every `With<Mesh3d>` entity (including ~1,000–2,000 static world geometry batches, sky faces, effect quads, and shadow decals), and ran `meshes.par_iter_mut()` across the full `Mesh3d` table every frame.
- **Change:** `rendering::drawing::install_previous_transforms` (`crates/hl2-bevy/src/rendering/drawing.rs`, wired in `launch::add_presentation_systems`) removes Bevy's system from `PreUpdate` via `Schedules::remove_systems_in_set` (`ScheduleCleanupPolicy::RemoveSystemsOnly`) and registers `hl2_bevy::rendering::drawing::update_mesh_previous_global_transforms`, which returns immediately unless an active `Camera3d` carries `MotionVectorPrepass`, filters to movable entity and weapon meshes (`With<Mesh3d>, Or<(With<SourceEntity>, With<WeaponMesh>)>`) so static world meshes keep `HAS_PREVIOUS_TRANSFORM = false` in `mesh_preprocess.wgsl`, and writes `PreviousGlobalTransform` only when `previous.0 != transform.affine()` (`previous_transforms_track_only_movable_meshes_when_motion_vectors_are_active`).
- **400-frame `--mode trace` A/B capture (`quiet.json`, walking the static spawn with motion vectors on, `o25-trace-a` vs `o25-trace-b`):**
  - `update_mesh_previous_global_transforms` (`398` calls):
    - `mean`: **`0.0796 -> 0.0079 ms` (`-90.0%`, 10.0x faster)**.
    - `p99`: **`0.4770 -> 0.0210 ms` (`-95.6%`, 22.8x lower p99)**.
    - `max`: **`0.5453 -> 0.0425 ms` (`-92.2%`, 12.8x lower max)**.
    - `total`: **`31.67 -> 3.16 ms` (`-90.0%`)**.
  - `main_schedule.PreUpdate` (`--profile` stage, `249` post-warmup samples):
    - `mean`: **`0.452 -> 0.412 ms` (`-8.8%`)**.
    - `p95`: **`0.632 -> 0.537 ms` (`-15.0%`)**.
    - `max`: **`0.89 -> 0.70 ms` (`-21.3%`)**.
  - `schedule: name=PreUpdate` (trace): `mean` `0.4419 -> 0.4080 ms` (`-7.7%`), `p99` **`0.8010 -> 0.6505 ms` (`-18.8%`)**.
- **Fidelity:** `motion_blur_options` and `viewmodel_motion_blur` replay checks pass with 0 failures.

## `O21` — Eliminate pure-waste per-frame heap allocations in `eyes`, `material_slots`, `Canvas`, and `Rig` (2026-11-23)

Measured on Apple M1 Pro (`--mode trace-alloc` A/B captures on `d1_trainstation_02`, baseline `e6563cd7` vs `O21`):

- **Root causes fixed:**
  1. `crates/hl2-bevy/src/eyes.rs`: `eyes::present` built `serde_json::json!` maps and formatted target-label `String`s (`"player"`, `"view"`, `format!("#{id}")`) inside per-actor and per-eye-draw loops on every frame even when no report was generated, and cloned `eyes.rows` (`32 KiB`) twice on dirty frames (`buffer.set_data(eyes.rows.clone())` and `eyes.uploaded = eyes.rows.clone()`). Replaced with typed `OriginRecord`, `ProjectionRecord`, and `TargetLabel` structs that reuse `Vec` capacity across frames and serialize only inside `Eyes::report`, plus a `dirty: bool` flag in `Eyes::set_rows`.
  2. `crates/hl2-bevy/src/material_slots.rs`: `material_slots::upload` called `buffer.set_data(slots.rows.clone())`, cloning `19,968 Vec4` (`312 KiB`) and allocating a new `312 KiB` `Vec<u8>` inside `ShaderBuffer::set_data` on every dirty frame. Added `material_slots::write_rows` (reused by `eyes::present`), which writes directly into `buffer.data`'s existing `Vec<u8>` buffer via `encase::StorageBuffer::new(&mut bytes)` (`write_rows_reuses_the_existing_shader_buffer_allocation`).
  3. `crates/source-ui/src/canvas.rs` and `crates/hl2-bevy/src/canvas_renderer.rs`: `Canvas::drain` took `state.commands` with `std::mem::take`, dropping capacity every frame so the next frame's HUD drawing reallocated `0 -> 4 -> ... -> 512` `Quad`s. Added `Canvas::drain_into` and `Hud::scratch_quads` to swap buffers and retain capacity.
  4. `crates/modkit-core/src/animation.rs` and `crates/hl2-simulation/src/physics/hitboxes.rs`: added `Rig::clip` to check already-lowercase clip keys before allocating `to_lowercase()`, and deduplicated changed collider pushes in `Physics::set_npc_hitboxes`.
- **`--mode trace-alloc` (`state_frame`) A/B results (`d1_trainstation_02`):**
  - **Static spawn (`300` frames, `o21-alloc-static-a` vs `o21-alloc-static-b`):**
    - Heap allocations per frame: **`108,983 -> 108,056` mean (`-927/frame`)**, `p99` **`192,301 -> 181,004` (`-11,297/frame`, `-5.9%`)**.
    - Bytes requested per frame: **`11,754 -> 11,160 KB` mean (`-594 KB/frame`, `-5.1%`)**.
    - `RenderExtractApp`: **`4.237 -> 3.884 ms` mean (`-8.3%`**, `p99 5.372 -> 4.828 ms`); `main app` `7.089 -> 6.690 ms` mean (`-5.6%`).
  - **Crowd (`400` frames on `test-inputs/performance-research/mixed-crowd.json`, `o21-alloc-crowd-a` vs `o21-alloc-crowd-b`):**
    - Heap allocations per frame: **`418,338 -> 412,528` mean (`-5,810/frame`)**, `p99` **`1,413,298 -> 1,392,403` (`-20,895/frame`)**.
    - Bytes requested per frame: **`47,965 -> 46,560 KB` mean (`-1,405 KB/frame`, `-2.9%`)**.
    - **`eyes` stage**: **`0.781 -> 0.609 ms` mean (`-22.0%`**, `p95` **`1.104 -> 0.841 ms`, `-23.8%`**, `max 1.25 -> 0.99 ms`).
    - `main_schedule.PostUpdate`: `9.209 -> 9.006 ms` mean (`p95 11.715 -> 11.456 ms`, `max 21.94 -> 18.56 ms`).

## `O29` — Share immutable LOD flex vertex data via `Arc<lod::Vertices>` (2026-11-23)

Measured on Apple M1 Pro (`d1_trainstation_02`, `test-inputs/performance-research/mixed-crowd.json`, baseline `808989b5` from `6fe185eb` vs `1a9830af`):

- **Root cause fixed:** `AnimatedMesh` and `lod::Level` stored separate heap `Vec` copies of immutable per-vertex `bind`, `normals`, `flex`, and `flex_base` tables. Every `AnimatedMesh::copy_for` (runtime NPC spawn from a template) and every facial-flex LOD switch in `hl2_bevy::rendering::lod::switch_levels` cloned all three/four `Vec`s (bursting on spawn frames when distant NPCs step off `LOD0`).
- **Change:** `AnimatedMesh` and `lod::Level` share `Arc<lod::Vertices>` (`bind`, `normals`, `flex`), and `AnimatedMesh` replaces the duplicate `flex_base` table with `deformed: Option<Vec<glam::Vec3>>` populated only for the CPU skinning fallback when facial flexes are active. `switch_levels` and `AnimatedMesh::copy_for` clone the `Arc` pointer in $O(1)$ without heap allocation (`level_switches_and_template_copies_share_immutable_vertex_tables`).
- **400-frame `--mode trace` A/B capture (`mixed-crowd.json`, covering spawn burst and movement across LOD switch distances):**
  - `hl2_bevy::rendering::lod::switch_levels`:
    - `max`: **`0.6614 -> 0.0536 ms` (`-91.9%`, 12.3x lower max spike)**.
    - `mean`: **`0.0219 -> 0.0183 ms` (`-16.5%`)**.
    - `p99`: `0.0431 -> 0.0400 ms` (`total 8.74 -> 7.30 ms` over 399 calls).
- **Three alternating `--profile` stage pairs on `mixed-crowd.json` (`808989b5` vs `1a9830af`):**
  - `animation`: **`1.685 -> 1.637 ms` median (`-2.9%`**, A `1.721 / 1.685 / 1.684`, B `1.665 / 1.637 / 1.626`; `p95` **`2.178 -> 2.096 ms`, `-3.8%`**).
  - `main_schedule.PostUpdate`: **`5.641 -> 5.565 ms` median (`-1.4%`**, A `5.766 / 5.569 / 5.641`, B `5.694 / 5.565 / 5.550`; `p95` `6.998 -> 6.910 ms`).
  - `frame_total`: **`13.02 -> 12.89 ms` median (`-1.0%`**, A `13.27 / 12.95 / 13.02`, B `13.31 / 12.89 / 12.84`).

## `O5` — `ClusterConfig::Single` on non-lit 3D cameras and dynamic `FlyCamera` clustering (2026-11-23)

Measured on Apple M1 Pro (everyday `stages` release build, `--uncapped --profile`, three alternating A/B pairs after one discarded warm-up run, baseline `0e02e9b1` from `5d7950c5` vs `808989b5`):

- **Root cause fixed:** all `Camera3d` entities (`FlyCamera`, `ViewmodelCamera`, `SkyCamera`, `MonitorCamera`, `IntroCamera`) defaulted to `ClusterConfig::FixedZ` (4,096 light clusters per view), running `assign_objects_to_clusters` (`main_schedule.PostUpdate`) and `prepare_clusters_for_cpu_clustering` (`render_prepare`) across thousands of clusters on views that never use clustered dynamic lights, and on `FlyCamera` even while the player's flashlight `SpotLight` is off.
- **Change:** `ViewmodelCamera`, `SkyCamera`, `MonitorCamera`, and `IntroCamera` spawn with `ClusterConfig::Single`; `FlyCamera` spawns with `ClusterConfig::Single` and `flashlight::update` (`sync_camera_clusters`) toggles `FlyCamera` to `ClusterConfig::FixedZ` only while `host.light.is_some()`.
- **Three alternating `--profile` stage pairs (`0e02e9b1` vs `808989b5`):**
  - **`quiet.json` (flashlight off):**
    - `render_prepare`: **`0.908 -> 0.681 ms` median (`-25.0%`**, A `0.903 / 0.908 / 0.908`, B `0.681 / 0.678 / 0.682`; `p95` **`1.141 -> 0.873 ms`, `-23.5%`**).
    - `main_schedule.PostUpdate`: **`2.065 -> 1.958 ms` median (`-5.2%`**, A `2.065 / 2.058 / 2.075`, B `1.953 / 1.958 / 1.961`; `p95` `2.401 -> 2.298 ms`).
    - `frame_total`: **`4.54 -> 4.32 ms` median (`-4.8%`**, A spread `0.04 ms`, B spread `0.03 ms`).
  - **`test-inputs/bevy-flashlight-depth-actor.json` (`--capture-live --frames 450`, flashlight on):**
    - `render_prepare`: **`1.526 -> 1.464 ms` median (`-4.1%`**, A `1.536 / 1.526 / 1.505`, B `1.431 / 1.464 / 1.496`), while `FlyCamera` uses `ClusterConfig::FixedZ` and the non-lit cameras stay on `ClusterConfig::Single`.
    - `frame_total`: `6.01 -> 6.01 ms` median.
- **Fidelity:** `flashlight_shadows` packaged replay check and `fly_camera_uses_single_cluster_only_while_flashlight_is_off` unit test pass with 0 failures.

## `O1` / `O20` — Moving-mesh `MotionRegion` scissoring across world and weapon motion blur and 25-tap jittered viewmodel gather (2026-11-23)

Measured on Apple M1 Pro at maxed Retina (`3456x2168`, 4x MSAA, full HDR, Ultra motion blur, anisotropic 8x; everyday `stages` release build, baseline `c8f8e9ea` from `d8208f77` vs `365827f5` / `a14a1f8a`):

- **Root causes fixed:**
  1. `crates/hl2-bevy/assets/shaders/object_motion.wgsl` (`if all(current == previous) { return vec2(0.0); }`) exports zero object velocity for any mesh whose world vertex positions did not change between frames, even while the camera turns. However, `plugins/hl2-advanced-options/src/bevy/weapon_blur/region.rs` only computed `MotionRegion` for `WeaponCamera` (`!blur.world`) and only scissored `weapon_reduce_4` on `!blur.world`, leaving `FlyCamera` (`blur.world`) to run full-screen `weapon_reduce_4` (`0.700 ms/frame`), `weapon_reduce_32` (`0.116 ms/frame`), `weapon_motion_dilate` (`0.139 ms/frame`), and `world_object_gather` (`0.112 ms/frame`) across `3456x2168` every frame.
  2. Using Bevy's raw `Changed<GlobalTransform>` boolean on `SkinnedMesh` `Joint` entities triggered on every tick of sub-pixel idle breathing animations (`"idle01"` on the crowbar and `"idle_subtle"` on plaza citizens). `region.rs` (`mesh_world_displacement`) now multiplies changed joint `GlobalTransform`s by `SkinnedMeshInverseBindposes` to cancel bone-origin lever arms, bounds the maximum world-space displacement across the mesh's `Aabb`, projects it to physical screen pixels at the mesh's camera distance, and filters at `MIN_PIXEL_MOTION = 0.8` pixels (below the shader's hard `1.0` pixel early-out where `smoothstep(1.0, 3.0, pixel_motion) == 0.0`).
  3. When `MotionRegion` is empty and the camera is stationary, `snapshot` (full-screen `3456x2168` `Rgba16Float` copy on `WeaponCamera`) and `reconstruct` skip all passes. Otherwise, `weapon_reduce_4`, `weapon_reduce_32`, `weapon_motion_dilate`, and `world_object_gather` (`world.rs`) are all scissored to the moving region (`pass_scissor` / `tile_scissor` / `gather_scissor`) while `LoadOp::Clear` clears the full intermediate targets.
  4. `plugins/hl2-advanced-options/assets/shaders/weapon_blur.wgsl` (`viewmodel_animation_blur`) caps viewmodel trail taps at `15..25` odd taps (`i32(clamp(ceil(length(vector * data.params.zw) * 0.5), 15.0, 25.0)) | 1`), keeping `phase = 0.5` on `center_tap = count / 2` so thin surfaces never lose coverage while jittering non-center taps with interleaved gradient noise.
- **Headless Metal System Trace (`d1_trainstation_02`, maxed Retina `3456x2168`, `c8f8e9ea` vs `365827f5` / `a14a1f8a`):**
  - **Static spawn (no script, `20261123-o2-after-static-1` vs `20261123-o20-after-static-1`):**
    - GPU busy per frame: **`8.07 -> 6.97 ms` mean** (`p50 8.47 -> 6.50`, `p95 9.01 -> 7.99`, `p99 10.08 -> 9.16`, `max 10.42 -> 9.58`; frames > 8.33 ms: `410/763` [`53.7%`] -> `11/874` [`1.3%`]).
    - `weapon_reduce_4` Fragment: **`0.707 -> 0.015 ms/frame`** (`-97.9%`).
    - `weapon_reduce_32` Fragment: **`0.117 -> 0.017 ms/frame`** (`-85.5%`).
    - `weapon_motion_dilate` Fragment: **`0.142 -> 0.041 ms/frame`** (`-71.1%`).
    - `world_object_gather` Fragment: **`0.116 -> 0.018 ms/frame`** (`-84.5%`).
    - `--profile` stages (`2,849` post-warmup samples): `frame_total` **`8.693 -> 7.111 ms` mean** (`p95 10.336 -> 8.936 ms`), `render_graph` **`5.874 -> 4.310 ms` mean** (`p95 7.752 -> 6.246 ms`).
  - **`quiet.json` (`20261123-o2-after-quiet-1` vs `20261123-o20-after-quiet-1` and `20261123-o20-after-quiet-2`):**
    - GPU busy per frame: **`7.88 -> 6.73 / 7.31 ms` mean** (`p50 7.40 -> 6.27 / 7.74`, `p95 8.91 -> 8.01 / 8.20`, `p99 10.11 -> 8.15 / 9.34`; frames > 8.33 ms: `176/435` [`40.5%`] -> `2/509` [`0.4%`] / `11/378` [`2.9%`]).
    - `weapon_reduce_4` Fragment: **`0.700 -> 0.014 / 0.013 ms/frame`** (`-98.1%`).
    - `weapon_reduce_32` Fragment: **`0.116 -> 0.015 / 0.017 ms/frame`** (`-86.2%`).
    - `weapon_motion_dilate` Fragment: **`0.139 -> 0.034 / 0.045 ms/frame`** (`-71.6%`).
    - `world_object_gather` Fragment: **`0.112 -> 0.017 / 0.015 ms/frame`** (`-85.7%`).
    - `--profile` stages: `frame_total` **`8.419 -> 7.001 / 7.822 ms` mean** (`p95 10.498 -> 9.002 / 9.712 ms`), `render_graph` **`5.879 -> 4.448 / 5.137 ms` mean** (`p95 7.866 -> 6.425 / 7.154 ms`).
- **`check_blur_performance.py` (`3456x2168`, 4x MSAA, 3 alternating pairs per scene):**
  - `still`: `raw_weapon` processed texels `37.2M -> 18.2M` (`24.6% -> 11.8%`), `raw_world` processed texels `150.8M -> 78.2M` (`100% -> 51.4%`, bounding the walking citizens and flying scanners across the plaza), `still` Advanced/Ultra mean frame time `12.08 -> 11.51 ms`.
  - `swing`: `raw_weapon` processed texels `31.9M -> 20.2–21.2M`, `swing` Advanced/Ultra mean frame time `12.78 -> 12.32 ms`, `p95` upper bound `18.0 -> 16.0 ms` (`p95` performance guard passes), `max` frame time `20.20 -> 16.39–17.20 ms` (retail Enabled `9.18 ms` median, which omits the full-scene 4x MSAA `MotionVectorPrepass` + `DepthPrepass` and post-process composites).
- **Fidelity:** `motion_blur_options` and `viewmodel_motion_blur` replay checks (including `viewmodel_blur_swing`, `viewmodel_blur_sharp`, `viewmodel_vectors_none`, `viewmodel_vectors_x4`, `viewmodel_silhouette_trail`, `still`, `paused`, `switch`, `off`, `retail`) pass with 0 failures.

## `O2` / `O1` — Single sky camera merge, direct 1x `Hud2d` pass (`Msaa::Off`), `OwnTarget` MSAA exclusion, and `metal_gpu.py` `depth == 0` fix (2026-11-23)

Measured on Apple M1 Pro at maxed Retina (`3456x2168`, 4x MSAA, full HDR, Ultra motion blur, anisotropic 8x; everyday `stages` release build, baseline `45fa0eaf` vs `c8f8e9ea`):

- **Root causes fixed:**
  1. `.agents/skills/hl2-performance-trace/scripts/metal_gpu.py` previously filtered `if row["depth"] >= 1:`, dropping non-overlapping `depth == 0` Metal GPU intervals (including `upscaling` frame markers and `early prepass` intervals), which had made the earlier 2026-11-22 single-sky-camera test appear to regress on GPU.
  2. `crates/hl2-bevy/src/sky.rs` spawned two full-Retina 4x MSAA `Camera3d` entities (`order: -2` for six 2D sky quads on layer 3 and `order: -1` for miniature 3D sky on layer 4). A single `SkyCamera` (`order: -1`) now draws both layer 3 and layer 4, keeping the six `SkyFace` quads centred on the active sky camera at `FACE_DISTANCE = 16,000` units (`far: 32,000`, `32,000 * sqrt(3) < 65,504` FP16 limit; verified across all 15 owned HL2 campaign maps by `source_assets::sky::owned_sky_depth`, max miniature sky reach `14,320` units in `d1_canals_05`).
  3. Because Bevy 0.19 keys `MainTargetTextures` on `(target, usage, format, msaa)`, `HudCamera` (`Camera2d`, `order: 2`) previously either had to use `Msaa::Sample4` (paying a full-Retina 4x `msaa_writeback` blit of `~0.50 ms/frame` plus a 4x MSAA `main_transparent_pass_2d` of `~0.60 ms/frame`) or allocate a separate target texture that discarded the 3D scene. `crates/hl2-bevy/src/canvas_renderer/hud_pass.rs` (`Hud2d` schedule) now renders `Transparent2d` directly onto `target.out_texture_color_attachment(clear)` (`GameFrame`) at `Msaa::Off` and `MsaaWriteback::Off` after the last active 3D camera resolves into `GameFrame` (`video/outputs.rs` `skip_covered_outputs`).
  4. `crates/hl2-bevy/src/video/msaa.rs` (`sync_antialiasing`) now excludes `Without<OwnTarget>` alongside `Without<PresentCamera>` and `Without<HudCamera>` so `MonitorCamera` (`256x256`) and `IntroCamera` keep `Msaa::Off` on their standalone 1x render targets.
- **Headless Metal System Trace (`d1_trainstation_02`, maxed Retina `3456x2168`):**
  - **Static spawn (no script, `20261123-o2-head-static-1` vs `20261123-o2-after-static-1`):**
    - GPU busy per frame: **`9.15 -> 8.07 ms` mean** (`p50 8.95 -> 8.47`, `p95 10.22 -> 9.01`, `p99 11.57 -> 10.08`, `max 17.41 -> 10.42`; frames > 8.33 ms: `649/652` [`99.5%`] -> `410/763` [`53.7%`]).
    - `main_opaque_pass_3d` Fragment: **`3.202 -> 2.712 ms/frame`** (`3.0 -> 2.0` passes/frame).
    - `msaa_writeback` Fragment: **`0.485 -> 0.000 ms/frame`** (`1.0 -> 0` passes/frame).
    - 2D HUD pass: `main_transparent_pass_2d` **`0.600 ms/frame` (4x MSAA) -> `hud_pass_2d` `0.185 ms/frame` Fragment + `0.035 ms/frame` Vertex (1x)**.
  - **`quiet.json` (`20261123-o2-head-quiet-1` vs `20261123-o2-after-quiet-1` and `audit-hud2d-1`):**
    - GPU busy per frame: **`9.82 -> 7.88 / 7.82 ms` mean** (`p50 9.99 -> 7.40 / 7.30`, `p95 11.45 -> 8.91 / 8.94`, `p99 11.80 -> 10.11 / 10.22`; frames > 8.33 ms: `352/352` [`100%`] -> `176/435` [`40.5%`] / `271/724` [`37.4%`]).
    - `main_opaque_pass_3d` Fragment: **`3.370 -> 2.640 / 2.625 ms/frame`** (`3.0 -> 2.0` passes/frame).
    - `msaa_writeback` Fragment: **`0.509 -> 0.000 ms/frame`** (`1.0 -> 0` passes/frame).
    - 2D HUD pass: `main_transparent_pass_2d` **`0.635 ms/frame` -> `hud_pass_2d` `0.178 / 0.168 ms/frame` Fragment + `0.038 / 0.040 ms/frame` Vertex**.
- **Three alternating `--profile` stage pairs at maxed Retina (`3456x2168`, `A-B-B-A-A-B` after warm-up, `45fa0eaf` vs `c8f8e9ea`):**
  - **`quiet.json`:** `frame_total` median **`10.16 -> 8.37 ms` (`-17.6%`**, A `10.38 / 10.16 / 9.94`, B `8.42 / 8.21 / 8.37`, `p95 13.21 -> 10.87 ms`), `render_graph` **`7.77 -> 6.07 ms` (`-21.9%`**, `p95 10.53 -> 8.55 ms`), `render_prepare` **`1.06 -> 0.95 ms` (`-10.4%`)**, `PostUpdate` `2.60 -> 2.52 ms`.
  - **`bevy-monitors-breen-screen.json` (`--frames 600`):** `frame_total` median **`17.53 -> 15.93 ms` (`-9.1%`**, A `17.58 / 17.51 / 17.53`, B `15.94 / 15.75 / 15.93`, `p95 22.19 -> 18.97 ms`), `render_graph` **`15.17 -> 13.63 ms` (`-10.2%`**, `p95 19.67 -> 16.66 ms`), `render_prepare` **`1.02 -> 0.94 ms` (`-7.8%`)**.
- **Fidelity:** `monitors-breen-screen`, `hud`, and `dev_overlay` packaged replay checks and `owned_sky_depth` (15 campaign maps) pass with 0 failures.

## A/A noise floor and stashed lockstep control (2026-11-22)

Measured at `6c680b59` (`bin/hl2-bevy`, everyday `stages` release build, `--uncapped --profile`,
three alternating A/A pairs of the same binary after one discarded cold warm-up run) on
`d1_trainstation_02` with `test-inputs/performance-research/quiet.json`:

- **Wall clock (`1,923–1,942` frames/run):** `frame_total` median A `4.57` vs B `4.58 ms` (`+0.2%`,
  A spread `0.06 ms` over `4.56–4.62`, B spread `0.03 ms` over `4.57–4.59`), `fixed_simulation`
  median `1.98` vs `1.98 ms` (`-0.2%`), `fight_ms` median `4.58` vs `4.60 ms`.
- **Stashed `--lockstep` experiment (one 15 ms fixed tick per rendered frame and synchronous
  runtime loads; `601` frames/run):** `frame_total` median A `5.11` vs B `5.14 ms` (`+0.5%`, A
  spread `0.07 ms` over `5.07–5.13`, B spread `0.15 ms` over `5.09–5.24`), `fixed_simulation` median
  `1.88` vs `1.90 ms` (`+0.7%`), `fight_ms` median `5.12` vs `5.17 ms`.
- **Crowd and physics determinism notes:** on `mixed-crowd.json`, two `--lockstep` runs matched
  workload totals (`1,201` frames, `73/73` spawns, `43` Combat / `4` Alert / `0` dead) while
  differing in `1,585` of `88,249` simulated float values (`frame_total` `13.93 ms` cold with a
  `140 ms` hitch vs `13.16 ms` warm). In-process `Physics::new` stepped twice on
  `d1_trainstation_02` diverged on tick 0 because `parry3d` keeps composite-shape contact manifolds
  in a randomly seeded `hashbrown` map unless Rapier's `enhanced-determinism` feature is on.
- **Outcome:** `--lockstep` did not lower run-to-run timing noise on the quiet scene and altered the
  simulation-to-render ratio (`~0.31` ticks/frame on wall clock vs `1.0` under lockstep). The
  `--lockstep` code and its two failing determinism tests remain stashed (not committed); the
  retained tooling stamps `executable_sha256`, `args`, `backlog_item`, and `backlog_measure` into
  `capture_meta.json` (`capture.py` / `session.py`).

## Batch 1 CPU optimizations (`O6`, headless `trace` on `mixed-crowd.json`, M1 Pro, 2026-11-22)

Matched headless `trace` captures on `d1_trainstation_02` (`400` static spawn frames at `4278c1ee`, `300` frames running `test-inputs/performance-research/mixed-crowd.json` at `4278c1ee` vs each change; absolute trace times include `trace-chrome` instrumentation, compare like with like):

- **Baseline at `4278c1ee`:**
  - Everyday `stages` (`d1_trainstation_02` static, 400 frames, 249 post-warmup samples): `frame_total` `9.826 ms` mean (`p95 13.169`, `max 17.22`), `render_graph` `5.641 ms`, `render_assets` `1.506 ms`, `PostUpdate` `2.832 ms`, `PreUpdate` `0.598 ms`, `fixed_simulation` `2.131 ms`, `animation` `0.662 ms`, `hud` `0.225 ms`.
  - Static `trace` (`d1_trainstation_02`, 400 frames): `allocate_and_free_meshes` `1.795 ms` mean, `prepare_erased_assets<SourceMaterial>` `0.824 ms` mean; `0` `present_entities + update` duplicate `actor_pose` groups because static spawn holds no animating rigged casters continuously in view (`silhouettes: 34` total).
  - Crowd `trace` (`mixed-crowd.json`, 300 frames, `65` spawned NPCs in the plaza): `frame_total` `58.136 ms` mean (`main app` `43.111 ms`), `shadows::update` `1.048 ms` mean (`p99 2.439`, `max 4.21`), `--profile` `shadows` stage `1.244 ms` mean (`p95 1.504`, `max 3.82`), `actor_pose` `597.7` calls/frame (`4.460 ms/frame`), **`7,308` duplicate `actor_pose` groups by `present_entities + update`**, `visibility::present` `0.407 ms` (`p99 1.834`, `max 2.26`), `rendering::lod::switch_levels` `0.052 ms` (`max 8.77`).
- **`O6` — Share `ActorPoses` in `shadows::update` (`shadows/caster.rs`):**
  - Crowd `trace` (`mixed-crowd.json`, 300 frames): **`present_entities + update` duplicate `actor_pose` groups `7,308 -> 0`**, `shadows::update` **`1.048 -> 0.617 ms`** mean (`p99 2.439 -> 1.119`, `max 4.21 -> 1.38`), `--profile` `shadows` stage **`1.244 -> 0.811 ms`** mean (`p95 1.504 -> 1.004`, `max 3.82 -> 1.37`), total `actor_pose` **`597.7 -> 463.4`** calls/frame (**`4.460 -> 2.729 ms/frame`**), `frame_total` `58.136 -> 43.030 ms` mean.

- **`O28` — PVS bitset rows and per-draw cluster lists (`hl2-bevy/src/visibility.rs`, `source-assets/src/visibility.rs`):** everyday `stages` build, headless `d1_trainstation_02` with the whole `mixed-crowd.json` run (`--frames 2000`, the script ends first; about 740-770 post-warmup samples), three runs each side, one at a time, at `3283ac47` against the change:
  - `source_pvs` stage mean `0.180 / 0.181 / 0.180 -> 0.156 / 0.160 / 0.157 ms` (about `-12%`), `p95 0.216 / 0.215 / 0.214 -> 0.189 / 0.190 / 0.187`, `max 0.263 / 0.261 / 0.265 -> 0.256 / 0.239 / 0.234`.
  - Same culling: `checked_draws 2396`, `culled_draws 468` in every run on both sides; `reused_bounds_tests` 886-894 both sides (the remaining ~1,500 per frame are crowd draws whose animated bounds move every frame and keep the early-exit tree walk).
  - `frame_total` unchanged within noise (`21.13-21.37 -> 20.72-21.72 ms` mean); this headless path never shows capture 3's live `1.8 ms` p99 spike, so the transition gain (whole-scene re-walks on a view-cluster change now cost bitset lookups for draws whose bounds stay put) needs the next live capture to confirm.

- **`O13` / `O8` — HUD canvas atlas (`hl2-bevy/src/canvas_renderer/atlas.rs`):** static `d1_trainstation_02` spawn, 1,000 frames, everyday `stages` build with the F1 overlay shown (saved setting), three runs each side at `6c97a6c5` against the change:
  - HUD: `446` quads in `432 -> 4` pooled batch meshes (2 atlas pages); before, about 52 batch meshes were rewritten per frame because the overlay's frame-time line shifted every later glyph batch.
  - `render_assets` mean `1.160 / 1.127 / 1.137 -> 0.555 / 0.568 / 0.570 ms` (p95 `2.19 / 2.16 / 2.13 -> 0.91 / 0.92 / 0.93`), `PostUpdate` `2.60 -> 2.51 ms`; `frame_total` `8.57 -> 8.58 ms` (unchanged: headless at maxed settings is GPU-bound here, also with `--uncapped`), `render_graph` `4.93 -> 5.61 ms` (the freed render-thread time becomes swapchain wait).
  - Headless `trace` (400 frames, same spawn) per frame: `allocate_and_free_meshes` `1.71 -> 0.23 ms`, `main_transparent_pass_2d` encode `0.34 + 0.23 -> 0.05 + 0.02 ms`, `submit_pending_command_buffers` `0.82 -> 0.31 ms`, `render_system` `6.18 -> 5.56 ms`, `specialize_material2d_meshes<HudMaterial>` `0.032 -> 0.008 ms`.
  - Fidelity: full-frame captures differ in 65 pixels, the overlay's elapsed-time digits; text and panels match.

- **`O13` crowd — particles drawn per effect and material (`hl2-simulation/src/explosion_particles.rs`):** headless `d1_trainstation_02` running `mixed-crowd.json` to the end (a firefight: about 970 effect quads, 325 debris and 112 smoke particles live), everyday `stages` build, three runs per side alternating, at `4a0d9e35` against the change:
  - Effect-quad run meshes rewritten per frame `207 -> 88` (temporary mesh-owner probe, frames 300-700); other rewritten meshes unchanged (22 eye meshes, 20 shadow decals, 11 animated models).
  - `frame_total` mean `19.58 / 19.37 / 19.99 -> 17.63 / 17.92 / 17.76 ms` (p95 `27.98 / 26.96 / 26.49 -> 24.49 / 24.99 / 24.83`), `render_assets` `5.58 / 5.37 / 5.81 -> 4.12 / 4.21 / 4.06 ms`, `render_graph` `7.32 / 7.21 / 7.24 -> 6.76 / 6.74 / 6.77 ms`; about 10% more frames rendered over the same script.
  - Fidelity: the end-of-script frames show the same scene (ragdoll poses differ between any two runs of this script); the order change is per effect, as Source sorts a particle effect as one translucent object and draws it per material.

- **`O14` step 1 — per-frame material values in a shared buffer (`hl2-bevy/src/material_slots.rs`):** everyday `stages` build, three alternating runs per side at `a3f9a390` against the change; static spawn 1,000 frames, crowd `mixed-crowd.json` to the end:
  - Static spawn: `render_assets` mean `0.558 / 0.561 / 0.560 -> 0.192 / 0.192 / 0.191 ms` (p95 `0.91 -> 0.25`), `render_prepare` `1.11 / 1.10 / 1.10 -> 0.96 / 0.96 / 0.96 ms`; `frame_total` unchanged (8.56 -> 8.60 ms, GPU-bound headless).
  - Crowd: `frame_total` `17.26 / 17.52 / 17.23 -> 16.72 / 16.87 / 16.57 ms` (p95 `23.5 / 24.8 / 23.7 -> 23.3 / 23.4 / 22.5`), `render_assets` `4.08 / 4.08 / 4.02 -> 3.65 / 3.71 / 3.59 ms`, `render_prepare` `1.58 / 1.57 / 1.58 -> 1.36 / 1.35 / 1.36 ms`.
  - Before (temporary material probe): about 25 `SourceMaterial` re-prepares per frame at the spawn and 43 + 2 in the crowd from texture-scroll and monitor tint; these now update slot rows only.
  - Fidelity: the frame-matched `bevy-monitors-breen-screen` replay capture differs from the old build by 0.31 mean per channel, less than two runs of the old build differ from each other (0.59).

- **Crowd ranking by macOS `sample` (2026-11-22, everyday build at `f3d2b5a1`, 20 s sampled at 10 ms once the crowd fights, fight frame 10.4 ms):** `fixed_step` 3.36 ms/frame (Rapier step 32% of it, AI think 10%, hitbox poses 6%, attachment poses 5%), `allocate_and_free_meshes` 2.15, `present_entities` 1.83 (of which `apply_facial_flexes` about 63%), `camera_driver` 1.53, `submit_pending_command_buffers` 0.91, HUD `present` 0.84 ms. The trace build had inflated the partial-pose cost (1.86 ms/frame from about 355 small spans).
- **`O30` CPU side — dense facial flex deltas (`source-assets/src/flexes.rs`, `hl2-bevy/src/rendering/face_flex.rs`):** crowd `stages` A/B, three alternating runs per side at `f3d2b5a1` against the change: `animation` stage mean `3.50 / 3.41 / 3.51 -> 2.11 / 2.13 / 2.10 ms` (p95 `4.77 / 4.64 / 4.78 -> 2.62 / 2.61 / 2.59`), `PostUpdate` `8.02 / 7.93 / 8.08 -> 6.74 / 6.62 / 6.67 ms`, `frame_total` `16.92 / 16.55 / 17.19 -> 16.04 / 15.92 / 15.76 ms` (p95 `23.4 / 23.7 / 23.9 -> 21.5 / 20.4 / 20.9`). Frame-matched `face-security-1805` replay captures: old vs new 0.47-1.63 mean per channel, two old-build runs 1.75.

- **`O6` — per-tick memo of partial actor poses (`hl2-simulation/src/entities/choreography/pose.rs`, owner-approved):** crowd `stages` A/B at `382fd37e`, three alternating runs per side:
  - First version also memoized whole skeletons: `fixed_simulation` `5.53 -> 5.44 ms` mean (p95 `8.17 -> 7.17`) but host `animation` `2.12 -> 2.20 ms` (hashing inputs and copying every presented skeleton into a memo cleared before reuse), so whole skeletons now skip it.
  - Partial skeletons only: `fixed_simulation` `5.66 / 5.62 / 5.63 -> 5.42 / 5.48 / 5.43 ms` (p95 `8.38 / 8.25 / 8.27 -> 7.23 / 7.15 / 7.25`, max `11.7 / 11.8 / 11.7 -> 10.9 / 9.7 / 9.8`), `animation` `2.10 / 2.09 / 2.12 -> 2.10 / 2.09 / 2.09 ms`, `frame_total` `16.26 / 16.31 / 16.39 -> 16.09 / 15.99 / 16.18 ms`, p99 bound `30 / 29 / 27 -> 27 / 26 / 26 ms`.

- **`O14` step 2 — model lighting slots (`hl2-bevy/src/material_slots/lighting.rs`):** at `be5e4927` against the change, three alternating runs per side each:
  - Crowd `stages`: `render_prepare` `1.414 / 1.423 / 1.398 -> 1.289 / 1.293 / 1.296 ms`, `render_assets` `3.85 / 3.75 / 3.80 -> 3.67 / 3.80 / 3.68 ms`, `frame_total` `16.10 / 15.96 / 16.20 -> 15.78 / 16.00 / 15.84 ms`; p99 bounds `26 / 25 / 26 -> 25 / 26 / 27`.
  - Crowd GPU (headless Metal System Trace, 8 s attached 3 s after gameplay starts, 3456x2234): opaque fragment `1.70 / 1.69 / 1.67 -> 1.68 / 1.71 / 1.82 ms` per frame, transparent fragment `1.42 / 1.24 / 1.43 -> 1.47 / 1.41 / 1.32`, GPU busy mean `8.15 / 7.61 / 8.12 -> 8.43 / 8.08 / 7.99 ms`; the opaque vertex pass, whose shader did not change, moved `2.02 / 1.92 / 2.11 -> 2.19 / 2.12 / 2.00`, so differences of a few percent are this scene's run-to-run noise.
  - Fidelity: frame-matched replay captures (`face-security-1805`, `gestures-security-1805`, the flashlight-lit pistol and SMG viewmodels) differ from the old build no more than two runs of one build differ; the SMG pair's only steady difference is the HUD ammo digits' glow.

- **Thread priority (`hl2-bevy/src/priority.rs`) and application metadata (`apps/hl2rs/Info.plist`):** frame threads (main and Bevy's compute pool) ask for user-interactive QoS, asset/IO pools for user-initiated; App Nap off; crowd `stages` A/B at `2b23906a`:
  - Under load (ten `yes` processes on the ten-core M1 Pro, two alternating runs per side): `frame_total` `41.21 / 37.89 -> 33.92 / 31.21 ms` mean (p95 `65.5 / 63.2 -> 60.6 / 49.8`, max `99.5 / 94.1 -> 75.3 / 70.4`, p99 bounds `85 / 81 -> 81 / 66`), `fixed_simulation` `7.42 / 7.17 -> 6.65 / 6.11 ms`.
  - Without load (six alternating runs per side): `frame_total` median `16.30 -> 16.28 ms` (`15.93-16.62` against `15.93-17.14`, one slow run on the new side), p99 bounds `27-32` against `25-38`: no change.
  - Game Mode: macOS labels the game and turns Game Mode on only when it runs full screen from an application bundle (O35); the embedded Info.plist alone is not enough.
  - Game Mode A/B (full screen, same binary bare and from a scratch bundle, maxed saved settings, crowd script, ten `yes` processes, six alternating pairs, Game Mode state confirmed per run in the system log): mean frame `20.03 -> 20.30 ms` (off -> on), p95 `26.1 -> 27.4`, p99 bound `33.0 -> 34.5`, simulation `5.47 -> 5.51`; first three pairs `20.62 -> 20.42` mean, second three `19.45 -> 20.18`. No measurable gain once the frame threads have user-interactive QoS.

## Uninflated Retina pacing and Metal GPU passes on M1 Pro (2026-11-22, Capture 4)

Everyday release build (`22332865`, `advanced-options,debug-tools`, no Chrome trace), windowed
`d1_trainstation_02` at native Retina `(3456, 2234)` / 120 Hz, 4x MSAA, full HDR, motion blur,
anisotropic 8x, VSync, with `--profile`, external once-a-second process sampler, and a 15 s macOS
`Metal System Trace` auto-attached when gameplay started (`58%` idle GPU, `879 MB` swap before
launch). Compare against Capture 3 (`48100633`, `trace-alloc`, 5,610 traced frames) in
`docs/optimization-backlog.md` for CPU/allocation culprits (`O6`, `O13`, `O25`–`O29`).

- **Visible display pacing (`displayed-surfaces-interval`, 962 frames at 120 Hz):** **62.0 FPS effective**
  (899 repeated refreshes): 1 refresh `186` frames (19.3%), 2 refreshes (60 Hz) `723` (75.2%),
  3 refreshes `47` (4.9%), 4–6 refreshes `5` (0.5%), and `1` frame at 64 refreshes (0.1%).
- **Metal GPU intervals (63,095 intervals over 817 frames):** GPU busy **6.00 ms mean**, p50 `5.30`,
  p95 `14.31`, p99 `20.10`, max `29.69` ms; `183/817` frames (22.4%) exceeded 8.33 ms. Top
  per-frame passes (ms/frame across all frames): `main_transparent_pass_3d` Fragment `1.520`
  (1.7/frame, p99 `3.618`, max `4.11`), `main_opaque_pass_3d` Fragment `1.099` + Vertex `0.542`,
  `source_bloom` Fragment `0.565` + Vertex `0.084` (2.3 Fragment passes/frame), `early prepass`
  Fragment `0.467`, `msaa_writeback` Fragment `0.406` + Vertex `0.040`, world motion blur
  (`world_motion_blur` `0.390` + `world_blur_composite` `0.208` + downsample/env `0.064` + vertex
  `0.049`, `world_object_gather` `0.049` avg / p99 `2.138`), 2D Fragment `0.147 + 0.098`,
  `upscaling` `0.143 + 0.050`. Weapon motion blur was mostly idle in this 15 s window
  (`weapon_reduce_4` `0.100` at 0.2/frame, `weapon_reduce_32` `0.098`, `weapon_motion_dilate` `0.068`).
- **`--profile` CPU stages (4,096 post-warmup of 10,518 frames):** `frame_total` **13.350 ms mean**
  (p95 `18.305`, max `94.46`), `render_graph` **7.091 ms** (p95 `9.995`, max `72.48`),
  `main_schedule.outside` **6.809 ms** (p95 `11.228`, max `86.39`), `main_schedule.PostUpdate`
  `3.966 ms`, `render_assets` `2.141 ms` (p95 `4.607`), `fixed_simulation` `1.939 ms`,
  `render_prepare` `1.513 ms`, `render_views` `1.047 ms` (p95 `5.316`), `animation` `0.924 ms`,
  `diagnostics` `0.003 ms`.
- **Process resources (132 samples, 131.5 s):** GPU `84.1%` mean (`max 99.0%`), `gpu_mem` `1941.1 MB`
  mean (`max 2377.0 MB`), CPU `165.5%` mean (`top_thread 34.3%`), RSS `1472.1 MB` mean (`max
  2622.2 MB`), `swap 878.6 -> 995.1 MB` (`+116.5 MB`).

## Maxed Retina on M1 Pro: GPU-bound, render thread second (2026-11-21)

Owner run, packaged release with `trace-chrome`, windowed d1_trainstation_02 at 3456x2168, 4x MSAA,
full HDR, motion blur, anisotropic 8x, VSync, the player running for about two minutes, with
`--profile`, a Chrome trace and an external once-a-second sampler. Writing the trace (17 GB) inflates
absolute times; use this run for structure, not budgets.

- GPU: Device/Renderer/Tiler utilization median 97%, mean 89%, max 100% (macOS driver statistics);
  driver memory in use median 1.99 GB. The GPU is the first limit at these settings.
- CPU: process 244% mean; the busiest thread 54% mean, 92% max; no thread saturated.
- Render thread (trace, 7,224 frames): `Render` 15.5 ms mean, p99 32.7 ms; inside it the render
  graph 6.5 ms, `camera_driver` 4.9 ms and `prepare_windows` 4.5 ms mean (p99 21.6 ms, max 193 ms):
  the wait for the next swapchain image, which is how the GPU limit reaches the CPU. Main app 8.3 ms
  mean, p99 18.5 ms. Five 3D camera passes per frame (`Core3d` 4.9 per frame) and two 2D.
- `--profile` stages: `frame_total` 16.3 ms mean, p95 26.2, max 138; `render_views` (`PrepareViews`)
  4.6 ms mean, p95 13.9; the new `render_create_views` 0.17 ms and `render_specialize` 0.15 ms mean,
  so the view-stage spikes are in `PrepareViews`.
- Spikes: a fixed step of 32 ms (frame 2074), a 29 ms `prepare_windows` (frame 2663), and
  `allocate_and_free_meshes` up to 598 ms during loading.
- Memory: RSS 1.6 GB median, 4.7 GB peak; the 16 GB machine kept about 70 MB of free pages, with
  swap unchanged at 932 MB.

Next: GPU time per pass and per camera with Instruments' Metal System Trace (`xcrun xctrace record
--template 'Metal System Trace'`), to see which of the five 3D passes and the post-processing at
Retina use the GPU.

## GPU shadow silhouettes (2026-11-15, default since 2026-11-19)

Dirty model silhouettes are drawn into the shadow atlas on the GPU whenever a renderer exists.
`HL2RS_GPU_SILHOUETTES=0` keeps the CPU rasterizer as the reference and fallback. The paired station02 tests,
GPU readback and receiver-pixel comparisons are in the
[performance research report](performance-research.md#gpu-shadow-silhouettes-on-station02).

## Dense report labels distort frame time (2026-11-12, session 55)

RTX 3070, Ryzen 7 3700X, Windows 11, packaged session 55 build (c20abc1), 1920x1080 borderless on the second
monitor, `--uncapped --profile`, d1_trainstation_02 plaza, static fly view (eye -2330 -2272 60, yaw 0, pitch -8).
Session 52's plaza fixtures (artifacts/s52/plaza.json, plaza-empty.json) label every 3 ticks from tick 150 for
the idle-wobble test; each label samples the report. With them, four runs measured 45.6, 56.7, 52.1 and 29.0 ms a
frame (17 to 35 fps) with 370-550 ms frames every few seconds, all in the `diagnostics` scope (mean 2.2 ms,
max 538 ms). The same fixture with labels only at ticks 1, 150 multiples and 1500 (artifacts/s55/plaza-sparse.json):
17.1 ms mean (58.5 fps), p95 24 ms, max 44 ms over 1195 measured frames; per-scope means fixed_simulation 5.2 ms
(max 38.8), render_graph 5.5 (max 108), render_prepare 2.3, animation 2.0, render_assets 1.4 (one 598 ms upload
at the start), shadows 0.8, render_bindgroups 0.8. Use sparse labels for any frame-time claim; the session 52
numbers above that came from the dense fixtures are inflated by the sampling.

## Owner's manhack plaza on Windows: where the frame goes (2026-11-10, session 52)

RTX 3070, Ryzen 7 3700X, Windows 11, packaged build 078d8d1 with a temporary lap probe in `Gameplay::tick`
(not committed), the owner's saved video settings (1920x1080 borderless, HDR Full, High details, motion
blur, no AA), `--uncapped --profile`, 1,500 ticks, labels every 150 ticks (dense labels add 2-3 ms of
report sampling to the `diagnostics` scope). Fixtures: artifacts/s52 make_walk.py, make_plaza.py,
make_crowd.py.

| Scenario | Mean frame | About |
| --- | ---: | ---: |
| d1_town_02, flying, no manhacks | 9.55 ms | 105 fps |
| d1_town_02, flying, 30 manhacks hovering (unsensed) | 18.85 ms | 53 fps |
| d1_trainstation_02 plaza, walking, no manhacks | 13.6 ms | 73 fps |
| plaza, walking, 30 manhacks hunting the player (no damage on this map) | 42.1 ms | 24 fps; F1 read 25 fps, 1,064,474 triangles, 1,329 entities |
| plaza, flying, 30 manhacks and a citizen every 0.9 s | 58.0 ms | 17 fps |

Native: the owner's screenshot of the plaza with a comparable crowd of manhacks shows 276 fps (3.6 ms). It was
not a matched fixture; no native timing was taken in this session.

One 15 ms simulation tick, ms, averages over 300 ticks (the lap probe):

| Stage | Plaza, empty | Plaza, 30 manhacks hunting |
| --- | ---: | ---: |
| scene and npc_makers | 0.69 | 0.67 |
| weapons and entity poses | 0.20 | 0.21 |
| NPC hitboxes and projectile spawns | 0.27 | 0.45 |
| query refresh | 0.13 | 0.13 |
| AI (`ai.tick`) | 0.68-0.88 | 3.9-6.1 |
| after the AI (results, NPC fire, effects, projectiles, zombies) | 0.20 rising to 1.14 | 0.29-0.94 |
| Rapier (`physics.tick`) | 2.55-2.85 | 2.43-2.79 |
| the rest (ragdolls, sounds) | 0.28-0.36 | 0.28-0.37 |
| total | about 5.3 | about 10.5 |

Per frame (`--profile` scopes): render_graph 3.4-3.8 ms, render_prepare 1.7-2.2, render_write 0.5-0.9,
render_assets 0.3-1.5, bind groups 0.5-0.7; shadows 0.04 empty and 1.5-1.7 with 30 casters; animation
0.4-1.2. Bevy's instrumented GPU passes add up to under 1 ms (our own passes are not instrumented).

Reading: the frame is CPU-bound and the GPU mostly idle. The fixed tick costs 5.3 ms on the empty plaza and
10.5 ms with the hunting crowd; once a frame takes over 15 ms the next one runs more than one tick, so the
frame rate falls faster than the work grows (about 2.8 ticks a frame at 42 ms). Rapier's 2.5 ms a tick is the
largest fixed cost and does not depend on the NPCs; the AI costs about 0.15 ms a tick per hunting manhack;
the render CPU is 8-9 ms a frame at these settings whatever the scene. The after-AI stage grows over an empty
run (0.2 to 1.1 ms in 18 s): something accumulates there. Not measured: Source's own server tick and render
CPU on this machine.

Step 1 findings, same scene (plaza, walking, 30 manhacks), temporary probes reverted afterwards:

| Rapier step, ms (averages over 300 ticks) | Hitboxes posed every tick | Hitbox posing switched off |
| --- | ---: | ---: |
| whole step | 2.39-2.41 | 2.31-2.32 |
| broad phase (BroadPhaseMultiSap) | 1.04 | 0.99 |
| query pipeline update | 0.99-1.00 | 1.03 |
| narrow phase | 0.30 | 0.22 |
| islands, solver, CCD | under 0.04 | under 0.04 |
| awake bodies / contact pairs | 4 / 0 | 4 / 0 |
| fixed_simulation per tick | 8.50 | 6.49 |
| mean frame | 37.8 ms | 24.4 ms |

Rapier's 2.4 ms is not the NPC hitboxes and not motion: with 4 awake bodies and no contacts, about 1 ms goes
to the broad phase over 5,659 colliders and about 1 ms to the query pipeline we pass into
`PhysicsPipeline::step`, whose last substep refits and rebalances the whole tree every tick (rapier3d 0.26.1
pipeline/physics_pipeline.rs, read, not changed; `update_incremental(.., remaining_substeps == 0)`). Posing the
NPCs' hitboxes every tick costs about 2 ms a tick elsewhere (the skinning matrices of every NPC, and possibly
manhack traces meeting moving hitboxes), and the frame falls from 37.8 to 24.4 ms without it. Rejected
hypothesis: hitbox moves as Rapier's cost (one A/B, recorded). Rapier timings came from its `profiler`
feature in a temporary build (Cargo.toml and Cargo.lock restored).

Flight traces in one sweep (committed after step 1): `ai::host::flight_trace`, which scanners, manhacks
and birds fly with, traced again past every NPC it met (up to eight), each sweep testing the NPCs'
hitbox colliders (CONTENTS_MONSTER). It now masks CONTENTS_MONSTER out of one sweep: the same result for
npc_* entities. Plaza crowd, two runs: fixed_simulation 7.95 and 7.75 ms a tick against 8.50; frames
39.8 and 37.1 ms against 37.8-42.1 (within run-to-run noise). AI sight has the same pattern: `PhysicsSight::
first_solid` casts a projectile ray again past every NPC, up to eight; Source's line of sight
(MASK_BLOCKLOS) never tests monsters. Then changed the same way: `first_solid` casts one
`filtered_ray` whose filter leaves npc_* colliders out. Plaza crowd, two runs: frames 27.9 and 28.4 ms
(35 fps) against 37.1-39.8 ms (25-26 fps); fixed_simulation 7.22 and 7.29 ms a tick against 7.75-7.95.
The frame gains more than the tick: fewer ticks a frame.

Next bounded steps (DESIGN 14s for the first two; then sight rays without NPC colliders): hitboxes posed on demand; our query pipeline updated by us instead of inside Rapier's step; laps inside
`ai.tick` for a manhack (senses, the flight trace that traces again for every NPC it passes, the swarm); the
growing after-AI stage; the cameras and passes per frame at the plaza; a matched native measurement on the
same spot.

## Low settings: what bounds the frame (2026-11-01)

Apple M1 Pro/Metal, packaged, headless, uncapped, `--profile`, spawn views, 1,500 frames, muted. Low is every Video > Advanced
option at its lowest, HDR None, no AA, bilinear, 1280x720. One option at a time was set back to maximum from Low (three runs each, median):

| Variant | station02 frame ms | coast01 frame ms | Sampled GPU busy % |
| --- | ---: | ---: | ---: |
| All Max, 3456x2168, 4x AA, HDR Full | 11.78 | 15.63 | 99-100 |
| All Low | 7.16 | 6.44 | 69-71 |
| 640x360 | 7.06 | 6.47 | 69-71 |
| 1920x1080 | 7.12 | 6.56 | 72-74 |
| 3456x2168 | 7.28 | 6.57 | 73-77 |
| 4x AA | 7.31 | 6.88 | 58-74 |
| HDR Full | 7.01 | 6.60 | 51-72 |
| Texture Very High | 7.05 | 6.45 | 58-77 |
| Model High | 6.81 | 6.61 | 49-72 |
| Shadow High | 6.69 | 6.58 | 49-58 |
| 16x anisotropic | 6.70 | 6.60 | 48-64 |
| Motion blur | 7.00 | 7.27 | 54-73 |
| Color correction | 6.68 | 6.54 | 48-70 |

At Low the frame is CPU-bound: uncapped frame time ignores a GPU-side option until the GPU becomes the slower side, and only the
combined Max reaches that. Captures confirm 640x360 and 3456x2168 frames are really that size. GPU busy is a noisy system-wide driver
counter sampled once a second, and it drifted during the batch, so it is not a pass timing; Metal pass timers in the report read 0 here.
Texture and Model detail do save memory: peak RSS 3.26 GB at Low, 4.36 and 4.70 GB with them raised on station02.

Waste removed, no visual change intended, alternating runs before then after:

| Change | station02 ms | coast01 ms |
| --- | ---: | ---: |
| CPU light clustering instead of GPU | 8.16, 7.23 -> 6.12, 6.10 | 6.52, 6.73 -> 5.49, 5.46 |
| Facial flex weights per actor | 5.82, 6.17 -> 5.43, 5.16 | 5.40, 5.38 -> 5.08, 4.98 |
| Flex and unchanged HUD meshes together | 5.49, 5.56 -> 4.96, 4.94 | 5.16, 5.04 -> 4.47, 4.42 |
| Only the last camera copies to the frame | within noise | 4.61, 4.51 -> 4.34, 4.35 |

Thin LTO with one codegen unit is the `dist` profile, selected by `--dist` and kept out of everyday builds. An early Low-only batch suggested 3-5% (station02 4.76, 4.77, 5.41 -> 4.65, 4.55, 4.63 ms; coast01 4.38, 4.41, 4.23
-> 4.41, 4.11, 4.09 ms) for a 4.5-minute build. The later comparison on the final code, three alternating
rounds, release then dist, shows less: Low station02 5.35, 4.82, 4.78 -> 4.70, 4.51, 4.71 ms and coast01 4.72, 4.45, 4.38 -> 4.43, 4.40, 4.06 ms; Max station02 11.03, 9.45, 9.38 -> 11.07,
9.36, 9.40 ms and coast01 14.13, 12.16, 12.17 -> 12.14, 12.14, 12.26 ms (no gain, GPU-limited). CPU per frame including loading is about 4% lower. A clean dist build took 5m 13s. The first
release run of each Max series is slower than the rest of its session, so compare within a table, not across them. Measured with a temporary switch, not
implemented: no sky cameras, station02 5.66, 4.88, 4.73 -> 4.09, 4.02, 4.43 ms and coast01 4.53, 4.19, 4.74 -> 3.69, 3.73, 3.95 ms. That
removes the sky, so it is not a Low change.

Base commit against these changes, same session, three alternating rounds, median (range):

| Tier, map | Before ms | After ms | Change |
| --- | ---: | ---: | ---: |
| Max, station02 | 12.14 (12.02-12.41) | 10.84 (10.25-10.99) | -11% |
| Max, coast01 | 14.88 (14.32-15.51) | 13.80 (13.34-13.93) | -7% |
| Low, station02 | 6.98 (6.93-7.13) | 5.24 (5.13-5.57) | -25% |
| Low, coast01 | 6.61 (6.41-6.69) | 4.57 (4.53-4.67) | -31% |

Max stays GPU-saturated (99-100% busy), and CPU time per frame including loading falls from about 28-32 ms to 22-26 ms.

The advanced options plugin lists a Potato item below retail's lowest in five combos. Each was measured alone from all-Low, three runs,
median, frame ms and peak RSS:

| Potato item | Gives up | station02 ms | coast01 ms | RSS GB (station02 / coast01) |
| --- | --- | ---: | ---: | ---: |
| None (all Low) | | 5.22 | 4.52 | 3.20 / 2.86 |
| Model | One further level of detail, except models with faces | 5.21 | 4.50 | 2.81 / 2.53 |
| Texture | One further mip | 5.07 | 4.72 | 3.17 / 2.85 |
| Shader | Environment maps, sky cameras (flat sky) | 4.46 | 4.15 | 3.22 / 2.86 |
| Shadow | All dynamic shadows | 5.01 | 4.58 | 3.20 / 2.89 |
| Filtering | Texture and frame filtering | 5.28 | 4.74 | 3.22 / 2.89 |
| All five | | 4.13 | 4.01 | 2.74 / 2.49 |

Only Shader moves frame time clearly; Model saves memory; Texture, Shadow and Filtering are within noise on frame time and set the look.
Those runs predate the fix that extends unfiltered sampling to world textures. Measured again after it, three runs, median: all Low 5.34 and 4.69 ms,
Filtering alone 5.27 and 4.73 ms, all five 4.45 and 4.03 ms (station02, coast01). Water Detail has no item because nothing renders it yet. The Model row and all-five rows were measured before models with faces were held at the Low root level, so their memory saving is now smaller; not measured again.

At saved settings of 3456x2168, 4x AA, HDR Full, Ultra blur and Shadow High, with the flashlight on at a fixed station02 view, one run each: base 16.07 ms,
these changes 13.31 ms. The two captures differ by a mean under 0.01 per channel and at most 3 of 255.

CPU stages at Low on coast01 after these changes: render graph 2.96 ms (per-camera encoding), fixed simulation 2.72 ms per tick (mostly
Rapier), render prepare 1.24 ms, animation 0.58 ms (1.24-1.90 before). These are overlapping wall scopes with profiling fences, and the
headless runs have no window present. Interactive frame time, Windows and power are **not tested**. Compare ratios, not absolutes.

## Ultra softening and brightness experiment (2026-10-30)

Apple M1 Pro/Metal, 3456x2168, 4x MSAA, HDR None. Three alternating triples,
reversed middle order; three priming runs retained separately. Original Ultra,
half-strength RG16F candidate and brightness-adaptive prototype respectively:

| Variant | Raw mean frame times (ms) | Median (ms) |
| --- | --- | --- |
| baseline | 16.808, 17.183, 16.529 | 16.808 |
| candidate | 16.665, 16.568, 16.661 | 16.661 |
| adaptive | 16.592, 16.636, 16.666 | 16.636 |

All pairs fail environment qualification due to memory pressure; one original/adaptive
pair also has baseline CPU/GPU drift. No speedup or energy saving is established.
GPU occupancy is 98.25-98.94%, host CPU busy 27.87-37.33%, and available-memory
estimates 3.14-3.43 GiB. Game CPU use is about 193-199% of one core. These are
system-wide occupancy proxies and observational associations, not isolated pass timers.
Dedicated VRAM and watts remain unavailable on unified memory. Full raw telemetry,
CPU/GPU work per frame, timestamp-aligned within-run Pearson correlations, priming,
paired ratios and rejected flags: artifacts/ultra-soft-efficient/timing.

RG16F retains XY half-float precision and reduces the two cameras' reduction payload
by 3,863,808 bytes at this resolution. It does not halve total GPU memory. Among 50
reference/candidate captures, 19 PNGs are byte-identical; projectile frames diverge
substantially, so these uncontrolled scene differences do not establish pixel equivalence.
Camera/lamp/projectile captures render without errors and preserve reported poses.
The brightness prototype gives vertical fluorescent trails more coverage; still images
do not establish reduced perceived afterimages. Keep it experimental pending controlled
GPU reduction readback equivalence, clean idle/turn/swing comparisons and display testing.


## Map install: shared LOD meshes (2026-11-20)

The load report times the main-thread install in steps (`install_monitors`, `install_draws`,
`install_effects`, `install_sky`, `install_rest`; `install_ms` stays the total). On
d1_trainstation_01/02, `install_draws` took 430-510 ms; about 200 ms of it built the coarser LOD
meshes of 400-530 entity draws, most of them repeats of the same model, material and scale.
Every studio vertex carries a reference to its source mesh, so every studio draw counted as
facially flexed and built private copies. Map install now builds each model, material, scale and
skinning combination's coarser levels once and shares them (`lod::SharedLevels`), as runtime
copies already did, unless the model's facial flexes move one of the draw's vertices (those keep
their own meshes and per-level vertex data). Three gameplay replays (M1 Pro, one game at a time):
`install_draws` 427-507 -> 326-330 ms, request to playable 1849-2015 -> 1758-1849 ms. Runtime
spawns are unchanged.

## Map loading: entity rigs on all cores (2026-11-20)

The map worker's models phase now reports `model_props` (prop meshes, physics and rigs),
`model_gibs` and `model_rigs` (weapon and player rigs) in `render.map_metadata.loading_ms`.
Timing inside `model_props` on d1_trainstation_01/02 showed the entity rigs (skeletons and
kept sequences of NPCs, props and ragdolls) took about 440-460 ms of 580-670 ms, loaded one model
at a time; model files (already read on all cores) took 25 ms. Rigs now load on all cores
before instances are placed, each stored at the instance that would have loaded it, so later
steps see the same rigs; the map's sequence names are collected once instead of per rig.
Three gameplay replays (M1 Pro, one game at a time): `model_props` 581-668 -> 268-315 ms,
request to playable 2163-2373 -> 1848-1932 ms. All 109 packaged checks kept their outcomes;
`monitors-kleiner-scene` missed one gib removal once with three games in parallel (also seen
before this change) and passed alone twice.

## Background/shared map loading: parallel preparation and content reuse (2026-10-30)

The serial timing baseline is `2bf2290`; the first complete pipeline checkpoint is
`8261813`. Compare retained packaged executables, not ordinary terminal wall time:
`render.map_metadata.loading_ms.request_to_installed_ms` measures request-to-host
installation CPU time, including worker polling, view/physics handoff and host
installation. It excludes completion of deferred GPU work. It is not a first-window
or GPU-present timestamp. Background visual readiness now precedes audio readiness;
authored clocks wait for audio while the menu is usable. Gameplay waits for all
sound/speech metadata.

The change removes throwaway startup gameplay/audio precache and hidden F4 spawn
preparation from backgrounds, overlaps materials with HUD/face/sky/flashlight work,
uses bounded wave/VOX decoders, and shares immutable audio across map loads. Gameplay
retains its full spawn templates and resource set. No render pass or frame-system
file read is added. Normal inactive polling is an optional resource check; caches
are accessed during loading and diagnostic snapshots, not playback decoding.

### Initial complete-pipeline comparison

Apple M1 Pro, 10 logical CPUs, 16 GiB unified memory, Metal, owned default-detail
content, HDR/AA off, 1024x768, muted isolated processes. Three pairs per scene,
ordered baseline/current, current/baseline, baseline/current; no concurrent builds
or other game processes reported. Each process starts with empty application
caches. OS file caches were not purged. Each run renders 240 profiled frames;
frame comparison uses the 120 measured frames after warm-up, with world_running
and complete audio verified. Resource sampling runs at 0.5 seconds with a discarded
primer. Retained executable SHA prefixes: baseline `fa0cd3483912`, first pipeline
`424ab1ca2d5b`. Raw reports, package copies, environment samples and paired summaries
remain in ignored artifacts/loading-performance.

| Scene | Baseline installed CPU ms (raw) | Pipeline ms (raw) | Median before → after | Paired after/before |
| --- | --- | --- | --- | --- |
| Base background01 | 2805.6, 2526.5, 2509.3 | 391.5, 375.0, 370.2 | 2526.5 → 375.0 | .140, .148, .148 |
| Episode Two background | 2644.1, 2500.1, 2483.5 | 287.3, 320.4, 295.4 | 2500.1 → 295.4 | .109, .128, .119 |
| d1_trainstation_02 | 3442.9, 3283.0, 3269.3 | 2141.3, 2186.4, 2123.1 | 3283.0 → 2141.3 | .622, .666, .649 |

The initial median reductions are 85%, 88% and 35%. Background worker medians fall
from 2119/2103 ms to 312/267 ms; host installation from 321/315 to 47/24 ms.
Gameplay worker time falls from 2766 to 1646 ms; installation remains substantial
(415→398 ms). This is a shared improvement, without omitting gameplay resources.

| Scene | Mean-frame ms median before → after | Per-run frame ranges, before / after |
| --- | --- | --- |
| Base background | 15.622 → 14.987 | 15.578–15.861 / 14.840–15.077 |
| Episode Two | 14.044 → 13.390 | 13.828–14.050 / 13.255–13.508 |
| Gameplay | 6.945 → 6.917 | 6.935–6.963 / 6.915–6.919 |

These short runs show no added steady frame cost; they do not establish sustained
FPS. Whole-process wall time also falls, but includes the rendered frames, GPU
initialization, capture/report work and shutdown, so it is not reported as load time.

The background active CPU/GPU occupancy medians are about 25.2/27.5% before versus
25.0/29.5% after for base HL2, and 26.4/40.3% versus 23.8/42.8% for Episode Two.
GPU percentages are occupancy proxies, not FLOPS or isolated pass cost. Dedicated
VRAM is unavailable on this unified-memory host. Sampled peak game RSS ranges are
1469–3856 versus 1349–1383 MiB (base background), 1330–3023 versus 1107–1116 MiB
(Episode Two), and 1325–1802 versus 1527–2201 MiB (gameplay). These are sampled
process peaks, not total CPU/GPU allocation accounting. Gameplay runs report memory
pressure level 2 and insufficient aligned CPU/GPU samples in their short measured
windows; backgrounds report level 1. Quiet-preflight and thermal-limit evidence is
incomplete. Do not interpret the small gameplay frame difference as causal or use
these samples as a hardware-independent guarantee.

### Final visual-first launch order

The remaining serial edge was resource kickoff: audio metadata planning (17–43 ms)
ran before HUD/sky preparation started. Launch the independent visual worker first,
then plan/start audio, checking parent cancellation again before spawning more work.
This leaves resource adoption, decode concurrency and gameplay completeness unchanged.
The owned cancellation test additionally checks that the old map-owned entity is gone.

Repeated all three alternating pairs per scene with the same retained baseline,
a separate final package and artifacts/loading-performance-final. Final executable
SHA prefix `d2475f02abd8`. CPU/GPU warm-up conditions and caveats above still apply.
All 18 final reports have complete audio, world_running=true and advancing scene
clocks. The final round reports memory pressure level 1, but gameplay's short aligned
CPU/GPU windows remain insufficient. Different memory pressure between rounds means
the rounds themselves are not a controlled isolated launch-order experiment.

| Scene | Baseline raw installed CPU ms | Final raw ms | Median before → final | Paired final/baseline |
| --- | --- | --- | --- | --- |
| Base background01 | 2625.5, 2517.3, 2517.6 | 368.8, 368.7, 375.1 | 2517.6 → 368.8 | .140, .146, .149 |
| Episode Two | 2508.4, 2467.1, 2510.1 | 263.8, 278.9, 269.9 | 2508.4 → 269.9 | .105, .113, .108 |
| Gameplay station02 | 3382.2, 3234.4, 3271.7 | 2160.8, 2182.9, 2141.0 | 3271.7 → 2160.8 | .639, .675, .654 |

Background medians improve about 85%/89%, gameplay about 34%, relative to the serial
baseline. Visual-auxiliary waiting is now 0.006–0.011 ms in these runs; its work
finishes during planning/material preparation. The prior Episode Two worker had
about 38 ms of auxiliary waiting in the recorded first pair. This establishes the
removed dependency; it does not isolate an overall speedup due solely to kickoff order.

Final mean-frame medians before→after: base 15.939→15.261 ms, Episode Two
13.965→13.689 ms, gameplay 6.945→7.005 ms. Gameplay's +0.060 ms lies within overlapping
run ranges (baseline 6.911–7.100, current 6.965–7.385); Episode Two also has an
isolated higher current run. Short, insufficiently aligned resource windows and
ordinary scene/host variation do not establish a causal frame regression or a
sustained frame guarantee. The loading improvement is much larger and consistent
across the separated per-run load ranges. No new recurring decode or render pass
was introduced. Remaining gameplay model preparation and host installation are
still visible costs; they were not replaced with feature/quality reductions.

Reproduce the separate final round with:

```sh
LOADING_BASELINE=artifacts/loading-performance/baseline-game LOADING_OUTPUT=artifacts/loading-performance-final python3 scripts/checks/check_loading_performance.py
```

### Reuse, cancellation and rejected trials

The 192 MiB wave/256 MiB VOX LRU limits account retained encoded identities and
decoded payloads; allocator/container overhead and other live resources are extra.
Decoded samples share Arc storage with current playback, so cache retention is not
a second decoded copy. Fresh cue libraries, entity bindings, RNG and soundscape
playback state remain per map. Exact resolved bytes plus loop/distance policy
protect game, loose-file and embedded-pak overrides.

A live four-background sequence (HL2→Episode One→HL2→Lost Coast) records 1,214 wave
hits, 1,441 VOX hits and no evictions, retaining about 317 MiB of accounted content.
A background→gameplay transition reuses 482 VOX words and 488 waves, retains all
785 gameplay waves and has no evictions. Its exploratory audio preparation is
214 ms; this run verifies reuse behavior and was not a controlled paired benchmark.
Five additional VOX entries across expansions are valid distinct content. A warm
test's initial assumption of completely identical word content failed; the recorded
counters and an independent fixture with alternate wave bytes verify correct
separation instead of masking those differences.

An initial 32 MiB VOX cache evicted 390 of 482 entries, defeating useful reuse.
Its package/report is retained as a rejected sizing trial; the final measured base
word working set is about 148 MiB including identities. The first paired harness
stopped before launch on a null CPU sample; the shared telemetry checker now treats
unavailable counters as unknown and has a regression test. Both rejected logs remain
in artifacts alongside accepted results.

Owned headless tests hold old late audio, start background/gameplay replacements,
verify cooperative cancellation before the new worker completes, check old scene
removal and real replacement installation. Unit tests verify owner tokens across
world copies, cancellation before adoption, channel/panic failures, ordered events,
budgeted eviction and exact content matches. A rapid return to a background whose
late audio was cancelled requires fresh loading, preventing permanent silent reuse.

Sky preparation was only 13–17 ms in the serial pilot and now overlaps materials.
Persistent CPU/GPU sky caching is not implemented; isolated sky upload benefit is
unmeasured. Final captures retain the pre-existing Lost Coast unsupported water
fallback, verified against the retained baseline. Native image/audio parity, audible
ordinary play, Windows GPU runtime and lower-core runtime are not tested.

Reproduce after retaining the instrumented prior package and building current:

```sh
LOADING_BASELINE=artifacts/loading-performance/baseline-game python3 scripts/checks/check_loading_performance.py
```

The harness defaults to three alternating pairs. LOADING_OUTPUT can select another
ignored artifact folder without overwriting a retained comparison. LOADING_RUNS
supports 1–9; source/package hashes, raw observations and environment flags are kept.

## SMG opacity repair: bounded timing smoke (2026-10-29)

The weapon gather retains opaque coverage when rejecting another weapon surface's motion,
using the current surface color for that rejected contribution. It adds arithmetic inside
existing taps; it adds no pass, attachment, texture fetch, tap count, worker or off-mode work.

Three alternating old/new packaged SMG runs at 1024x768, 4x AA, HDR None, Advanced,
uncapped headless Metal, after 120 warmup frames:

| Pair | Before mean ms | After mean ms | Before p95 upper bound ms | After p95 upper bound ms |
| --- | ---: | ---: | ---: | ---: |
| 1 | 8.839 | 8.966 | 12 | 12 |
| 2 | 10.571 | 9.815 | 24 | 18 |
| 3 | 10.788 | 9.480 | 27 | 12 |

Median means: 10.571 -> 9.480 ms. These are exploratory smoke measurements: compilation
was active during part of the batch, workloads are short, and resource/thermal controls
were not collected. They do not establish a speedup or sustained FPS guarantee. No
consistent extra frame-time cost is observed. Raw per-frame-window reports and the
rejected original shader control are retained in ignored artifacts.


## Final tier interpretation and native idle checkpoint (2026-10-27)

The owner split legacy Advanced into Advanced (retail camera + strong weapon animation)
and Ultra (adds dense world/camera and projectiles). Historical full-reconstruction series
labelled Advanced therefore describe today's Ultra. No existing save is promoted to Ultra.

| Current tier | Native idle median ms | FPS | p95 upper bound ms |
| --- | ---: | ---: | ---: |
| Enabled | 10.74 | 93.1 | 12 |
| Advanced | 12.86 | 77.8 | 14 |
| Ultra path measured before label split | 16.31 | 61.3 | 18 |

3456x2168, 4x AA, HDR None, uncapped completion-bounded headless M1 Pro/Metal. Three
alternating pairs per dataset, separately retained declared priming. Advanced's time guards
pass but two pairs flag pressure; Ultra's strict guards fail. These are provisional/cross-
batch observations, not interactive FPS guarantees or an isolated world-pass GPU timer.
Artifacts: advanced-only-native, region-native-still, tier-activation. Weapon raw reduction
processes about 24% of full texels in the idle probe; projection costs about 0.002 ms/call.

Supported direct drawing is automatic only on the measured M1 Pro/Metal adapter, with a
startup override. Other hardware defaults are unchanged. It cuts render buffer-write CPU
cost from several ms to about 0.02 ms. At 720p the earlier Ultra path improves about 58 to
110 FPS; CPU work/frame falls about 34% across batches. No watt/energy claim is made.
Both cooperative kernels retain correct GPU results but no Metal speedup; their code,
GPU comparator and timing are archived. Keep fragment reduction. Future work needs final
Ultra probes and matched-resource turns/swings; 120 FPS/native Ultra is not established.

## Uncapped main-depth baseline (2026-10-27)

The background profiler now runs with zero application-loop interval, keeping the two-GPU-
marker bound. Every run checks its reported interval, completed GPU work and script completion.
Ordinary replays retain their pacing; interactive FPS is not tested by these runs.

| Workload | Enabled median frame ms / FPS | Advanced median frame ms / FPS | Median p95 ms Enabled / Advanced |
| --- | ---: | ---: | ---: |
| Native 3456x2168, 4x AA, idle | 14.27 / 70.1 | 19.17 / 52.2 | 15 / 23 |
| Native, turning | 15.03 / 66.5 | 20.58 / 48.6 | 18 / 25 |
| Native, swinging | 14.28 / 70.0 | 21.49 / 46.5 | 15 / 30 |
| 1280x720, no AA, idle | 12.62 / 79.3 | 17.31 / 57.8 | 14 / 19 |
| 1920x1080, no AA, idle | 12.78 / 78.2 | 17.42 / 57.4 | 14 / 19 |

Thirty packaged runs total: three alternating pairs per row, middle order reversed. All native
turn pairs and all 720p pairs are resource-comparable. Native idle/swing and 1080p have some
memory-baseline/pressure flags, retained in their raw results. Every row fails the unchanged
performance budget; no optimisation is qualified. Native differs in AA and aspect ratio, so
these rows cannot isolate resolution bandwidth. The matching-aspect no-AA rows indicate a
substantial fixed cost. Earlier paced observations cannot be used as uncapped throughput.

In representative native/720p runs, render-phase buffer writes are 2.3-2.4 ms Enabled and
4.5-4.7 ms Advanced. These are CPU wall stage scopes, not direct GPU timers; scopes overlap.
Animation is about 2.5-2.7 ms per call, fixed simulation about 2.7-2.9 ms per call. These values
motivate testing supported direct drawing before adding more filter complexity. CPU work/frame
is tracked separately from maximum FPS; GPU busy-time/frame remains a system-wide proxy.
Power watts and joules/frame are null: local powermetrics requires privileged access, and
utilisation cannot establish energy. Future energy tests require matched FPS and power/thermal
conditions. Data: artifacts/blur-performance-regression/uncapped-main-depth-native,
uncapped-main-depth-720p-noaa and uncapped-main-depth-1080p-noaa. No interactive, Windows-runtime,
120 FPS or energy-efficiency success is claimed.


## Background frame pacing prevents an uncapped FPS claim (2026-10-27)

Source inspection finds main.rs configures every --background loop, including --profile,
with a 1/60 s ScheduleRunnerPlugin interval. GPU completion pacing does not remove that
application-loop interval. Existing measurements include this pacing: they show observed
regressions/resource use, but cannot establish maximum low-settings throughput or predict
120 FPS. No new uncapped or interactive run has verified the underlying work budget.
Before further optimisation qualification, provide an uncapped profiling-only loop, retain
ordinary replay pacing and the fixed two-marker GPU bound, then repeat matched measurements.
Earlier data stays archived with this interpretation; no frame-budget result is silently
rewritten as an uncapped GPU measurement.


## Main-depth follow-up: incomplete timing (2026-10-27)

Sampling main depth after drawing avoids its separate multisampled prepass export/copy.
Packaged world and weapon probes pass, but cost remains unqualified. A first native batch
overlaps another packaged replay and is interrupted, retaining its data. A second completes
three idle pairs: Enabled mean-frame median 19.43 ms (19.39-19.45), Advanced 20.26 ms
(20.11-20.30), median p95 24/24 ms. Both timing budgets pass. Two pairs are resource-comparable;
one has a material baseline-memory shift. The guard then detects a competing compiler and
stops before Advanced turns. No turning/swing qualification or direct-colour-versus-depth
speedup is established. Artifacts main-depth-repeated and main-depth-isolated remain separate,
including preflight evidence. Further timing requires an exclusive window; thresholds stand.

## Repeated direct-source experiment (2026-10-27; rejected for qualification)

Completed 18 packaged runs: three alternating Enabled/Advanced pairs for idle, continuous
turns and crowbar swings, reversing the middle pair. Apple M1 Pro / Metal, 16 GiB unified
memory, 3456x2168, 4x MSAA, HDR None, VSync off, completion-paced headless --profile.
The candidate reads resolved colour inputs directly and removes two native foreground
images/copies; weapon filtering math is retained. It is archived separately and unqualified.

| Scene | Enabled median mean (range), ms | Advanced median mean (range), ms | Median p95 Enabled / Advanced, ms | Resource comparability |
| --- | ---: | ---: | ---: | --- |
| Idle | 19.46 (18.27-23.61) | 30.76 (20.56-44.72) | 24 / 106 | All pairs flagged; builds and memory pressure |
| Turns | 18.01 (18.01-18.82) | 22.24 (22.06-22.37) | 23 / 27 | All pairs flagged; pressure/pageouts or baseline mismatch |
| Swing | 18.26 (18.23-18.30) | 23.60 (23.53-23.79) | 22 / 33 | Three comparable pairs; no quality flags |

The comparable swing pairs consistently fail the unchanged mean and p95 budgets. Their
Advanced/Enabled mean-frame ratios are 1.294, 1.303 and 1.286. Average host CPU busy is
43.2-45.7% Advanced versus 44.4-46.1% Enabled; GPU busy is 98.7-99.0% versus 73.8-74.5%.
Minimum available-memory estimates are 3.48-3.62 GiB versus 3.92-4.04 GiB, without measured
pressure/pageouts. This supports a GPU throughput bottleneck in this workload; precise
per-pass cost and causality remain unmeasured. It does not excuse the failed budget.

Swing output is 315-318 versus 409-411 Mpixel/s, or 22.0-22.1 versus 28.3-28.7 frames per
consumed game CPU-second. The system GPU-busy-time proxy is 319-322 versus 551-555 Mpixel/s
of GPU-busy time; it is not isolated GPU efficiency or available FLOPS. Dedicated VRAM and
GPU clocks remain unavailable. The loaded idle runs show positive CPU/frame-time associations
(r=0.60-0.75) and lower GPU occupancy during stalls; these small adjacent-window samples are
descriptive, not causal. All raw iterations, resource windows, correlations and ratios are
retained in artifacts/blur-performance-regression/direct-sources-repeated. No runs were
removed from the medians. No comparison here establishes interactive FPS or a copy-removal
speedup against the previous candidate. Visual qualification is recorded separately.

## Completion pacing changes measurement interpretation (2026-10-27)

Offscreen tests now keep at most two pending GPU completion markers during --profile.
The completion-paced swing reports about 2.23 GiB driver-managed memory, against 4.40 GiB
in the earlier unpaced resolved-world case; pressure flags disappear. This supports queue
growth as a contributor to memory pressure. The 23.14/25.44 ms frame figures are exploratory
and do not isolate pacing overhead or a rendering speedup. Interactive mode retains its
existing surface presentation and installs no new limiter.

Pre-pacing headless FPS is CPU-produced-frame cadence, not independent proof of completed
GPU throughput. New qualification requires observed completions and a maximum of two
pending markers, alongside the existing resource and frame-time checks. The baseline GPU
primer is excluded; stable moderate host inputs are compared instead of presuming full
idleness. Failed earlier flags/results remain in ignored evidence. Rendering candidates
are still unqualified; exact GPU-pass durations and isolated marker overhead are untested.

## Resource/output correlation batch (2026-10-27; not qualified)

Apple M1 Pro / Metal, 10 logical CPUs, 16 GiB unified memory, 3456x2168 / 4x MSAA. Driver
GPU utilisation is system-wide occupancy, not an isolated shader timer or available FLOPS.
All numbers below are observational. All pairs are rejected for optimisation qualification.

| Scene | Retail / experimental Advanced frame ms | Advanced / retail cost | Retail / Advanced GPU busy | Retail / Advanced min available memory |
| --- | ---: | ---: | ---: | ---: |
| Idle | 18.22 / 22.96 | 1.260 | 68.5 / 96.9% | 4.51 / 2.03 GiB |
| Turns | 18.25 / 27.97 | 1.533 | 81.9 / 94.9% | 4.50 / 1.96 GiB |
| Swing | 19.61 / 27.58 | 1.407 | 70.1 / 96.4% | 3.42 / 1.72 GiB |

Advanced runs show VM pressure/pageouts, and the swing pair includes competing build work.
In the idle pair, driver-reported in-use system memory averages about 1.69/4.45 GiB; these
are aggregate driver counters, not per-game VRAM or the renderer's exact allocation size.
Idle Advanced's available-memory/frame-time association has r=-0.85 over 15 windows. This
small autocorrelated sample does not establish cause: the renderer also changes resource
use itself. A later retail run records 21.29 ms / p95 49 ms with CPU busy 65.0% and competing
builds, compared with 18.22 ms / p95 22 ms / CPU busy 30.9% in the first run.

Sampler accounting observes 1.366 CPU-seconds across 17 active samples over a 16.758-second
retail window. This includes child commands and is not an isolated FPS-overhead measurement.
Raw samples, aligned windows, correlations and input/output ratios are retained in ignored
artifacts/blur-performance-regression/environment-loaded and environment-observer. Ratios
include pixels per second, frames per consumed game CPU-second and explicitly labelled
system GPU busy-time throughput proxies. Dedicated VRAM and precise GPU durations remain
unavailable; no causal normalisation is used to loosen the performance guard.

## Native-resolution Advanced regression: investigation open (2026-10-27)

The owner's 3456x2168 / 4x MSAA case supersedes the earlier 1024x768/no-AA qualification
below. Copied f7369cd packages reproduce 40.45 ms idle / 110.94 ms turning. Direct a8a050b
crowbar fixtures give 19.23 ms idle / 19.05 ms turning / 21.01 ms swinging. These earlier
fixtures are investigation evidence; they are not accepted timings for a final fix.

The native owned guard now records mean, p95/p99 upper bounds and maximum after 120 warm-up
frames, and includes the crowbar. Its thresholds are unchanged while investigating. A
quarter camera filter plus native object gather improves some scenes but still fails the
swing budget; no rendering optimisation is qualified or committed yet. Extra full-size
resolved-motion caches and an auxiliary single-sample camera fail the idle budget.

GPU durations returned by the encoder-boundary Metal timestamp probe are all zero, so
individual pass GPU times remain unavailable. Report CPU encoding and end-to-end frame
timing separately. Profile-only histogram collection adds one bounded update per measured
frame with no allocation; isolated overhead is not tested and no ordinary-play speedup is
claimed. See docs/validation.md for the opt-in guard and artifact evidence.

## Advanced world/projectile reconstruction (2026-10-27)

Packaged Metal / Apple M1 Pro, hl2/d1_trainstation_02, 1024x768, HDR None, MSAA off,
VSync off, muted background. Baseline a8a050b and final implementation use identical
900-tick fixtures: fixed plaza view, or repeated yaw sweeps of +/-70 degrees. Three
alternating pairs per scene, reversed order in the middle pair. All 12 scripts finish;
555-572 frames are measured after 120 warm-up frames over 10.900-11.076 seconds each.
No overlapping builds, game captures or image analysis in this final batch.

| View | Prior Advanced mean | Current Advanced mean | Prior range | Current range |
| --- | ---: | ---: | ---: | ---: |
| Still | 19.694 ms | 19.638 ms | 19.639-19.758 ms | 19.570-19.680 ms |
| Repeated turns | 19.510 ms | 19.258 ms | 19.465-19.550 ms | 19.225-19.309 ms |

No frame-time regression in this scene; the observed reductions do not establish a general
speedup, native equivalence or FPS guarantee. The new world prepass can also reduce main-pass
overdraw, so scene totals are not an isolated blur GPU cost. GPU timestamps are unavailable.
CPU encoding spans for still/turn (ms): prepass 0.17848/0.20674, 4x reduction
0.000501/0.000416, 32-pixel reduction 0.000330/0.000336, dilation 0.000377/0.000302,
world composite 0.000386/0.000350. These exclude GPU execution and color-copy cost.

Advanced adds world motion/depth prepasses and shares three reduced motion passes plus a
composite with the weapon backend. The world owns one full-size color copy and three small
fields (1/4, 1/32, 1/32 resolution); it skips the weapon background snapshot. Textures,
bind groups and pipelines are reused; there is no steady-frame allocation or asset access.
Camera-only regions avoid dynamic-object gathers; static regions return immediately.
World gathers use dense odd 17-257 taps, 1.25-frame exposure and 12% UV reach. Disabled/retail
Enabled do not add world prepass/reconstruction work; weapon taps remain 17-97.

The exploratory timing-summary.json batch overlapped architecture/fmt work and predates the
sky/history fixes; it is rejected for final timing claims. Accepted evidence:
artifacts/advanced-world-blur/benchmark-final.py, timing-final-summary.json,
timing-final-aggregate.json and final-*.json/log. Windows/Linux runtime and other scenes,
resolutions, frame rates or particle-heavy cases are not tested.


## Advanced surface-motion fix (2026-10-27)

Packaged Metal / Apple M1 Pro, 1024x768, HDR None, MSAA off, VSync off, muted background
hl2/d1_trainstation_02. Identical executable with original/fixed shader; fixed camera,
pistol/SMG/crowbar firing and switches, 1,100-tick fixture, 2,000-frame cap. Three isolated
runs per version, alternating and reversing the middle pair. Each measures 766-777 frames
after 120 warm-up frames over 13.959-14.157 seconds; all scripts finish.

| Shader | Mean frame | Range |
| --- | ---: | ---: |
| Original | 18.215 ms | 18.206-18.223 ms |
| Fixed | 18.213 ms | 18.181-18.238 ms |

Difference -0.002 ms, with overlapping ranges. No regression measured in this scene; this
is not a general speedup or FPS guarantee. CPU encoding spans (original/fixed ms): 4x
reduction 0.000676/0.000692, 32-pixel reduction 0.000332/0.000342, dilation
0.000369/0.000378, reconstruction 0.000265/0.000289. These measure encoding, not GPU execution;
GPU timestamps are unavailable. No extra passes, textures, buffers or off-mode work.
Gathers now use odd 17-97 taps instead of 16-96 so a center tap exists. Slow covered pixels
no longer inherit unrelated long rays, and onset blends over 1-3 pixels. Exposure/reach
remain 1.25 frames/12%. Evidence: artifacts/advanced-blur-flicker/timing-summary.json and
performance-summary.json, scripts and per-run logs/reports. Timing runs were separate from captures, image analysis and builds; no overlapping
capture/build timing round is included.

## Retail quick-info size and horizontal centering (2026-10-27)

Before/after/after/before packaged runs on Metal / Apple M1 Pro, default settings,
1280x720 d1_trainstation_02 HUD fixture, background/muted, profiling enabled.
No concurrent builds, replays or image analysis; only brief documentation writes.
120 warm-up frames, 148-151 measured frames per run. Mean frame times:
18.9647 ms before (range 18.6965-19.2329), 19.0175 ms after (18.7215-19.3136).
HUD CPU means: 0.08457 -> 0.07553 ms. The ranges overlap; +0.0528 ms overall
cannot establish a causal regression, and the lower HUD mean is not a speed claim.

The change preserves quad counts and arc spacing. Glyph bounds stay cached; no
new render pass, per-frame allocation or file access is introduced. Font-profile
selection parses the already-read scheme once more during load, never in a frame
system. Reproduce with artifacts/quickinfo-horizontal/benchmark.py; paired reports,
packages and timing-summary.json are retained. This bounded replay does not establish
sustained FPS or exact native renderer performance.

## Quick-info symbol placement (2026-10-27)

Metal / Apple M1 Pro, packaged before/after builds, default settings, 1280x720,
controlled d1_trainstation_02 HUD replay, profiling enabled, background and muted.
The final reverse-order pair measured 18.65495 ms/frame before and 18.66357 ms/frame
after (+0.00862 ms); HUD CPU means were 0.07757 and 0.07479 ms. Each run used
120 warm-up frames and 152/154 measured frames. This short pair does not establish
a speedup or isolate a causal cost; no material regression is evident in this sample.

An earlier pair overlapped documentation/architecture checks and is retained in
the aggregate report, excluded from the comparison above. The change queries
cached glyph geometry, adds no render pass or frame file access and does not
change glyph sizes or counts. Exact native rasterization and sustained FPS
are not established. Reproduce using artifacts/quickinfo-layout/benchmark.py;
individual reports, timing-summary.json and prior/final package copies are retained.

## Background-loading wait policy (2026-10-27)

Packages 6019af3 (prior) and fa4a000 (wait gate), Metal on Apple M1 Pro, default
settings, 1024x768 hl2/background01 with chapter browser open. Three isolated pairs
ordered prior/current, current/prior, prior/current. No concurrent builds, replays,
windowed test or image analysis. The benchmark waits for all validation processes
to finish. Same perf-browser.json fixture, host tick 1100, 2000-frame safety cap,
120 warm-up frames, 751–759 measured frames/run (about 14 seconds after warm-up).
All six runs complete in NewGame; the current package reports menu.visible=true.

| Package | Mean frame | Run range | HUD/UI CPU mean |
| --- | ---: | ---: | ---: |
| Prior | 18.693 ms | 18.672–18.709 ms | 0.1550 ms |
| Wait gate | 18.743 ms | 18.705–18.773 ms | 0.1581 ms |

The ranges overlap slightly. The +0.051 ms frame / +0.0031 ms HUD means do not
isolate a causal cost or establish a speedup. The gate adds simple state checks,
no render pass, allocation or asset access. During the wait, it omits menu/title
and window painting. This is a steady-state cost check; worker-load input latency,
wait-state GPU cost and final physics/GPU installation stalls are not measured.
The policy makes users wait instead of claiming responsive controls under load.

Reproduce with artifacts/chapter-loading-gate/benchmark.py; retained package metadata,
individual timing reports/logs, timing-summary.json and timing-aggregate.json identify
all samples. The prior copy is the retained 6019af3 package. Other native platforms,
retail frame/input latency and sustained FPS guarantees remain not tested.

## Chapter/loading bug fixes (2026-10-27)

Prior package `7171e3f`, fixed production source/package `6019af3`, Metal on Apple
M1 Pro, 1024x768 default settings, live hl2/background01 with the browser open.
Three alternating pairs (prior/current, current/prior, prior/current), no concurrent
builds, replays or image analysis. Use the same perf-browser.json fixture as the
chapter benchmark: finish host tick 1100, 2000-frame safety cap, profiling enabled,
120 warm-up frames. All six scripts finish with NewGame mode and 728–757 measured
frames per run (about 14 seconds after warm-up).

| Package | Mean frame | Run range | HUD/UI CPU mean |
| --- | ---: | ---: | ---: |
| Prior | 18.947 ms | 18.692–19.404 ms | 0.1670 ms |
| Fixed | 18.878 ms | 18.698–19.079 ms | 0.1615 ms |

Ranges overlap; the 0.069 ms difference does not establish a speedup. No steady
browser-frame regression is measured in this scene. No new render pass is added.
The runtime loader no longer reconstructs an unused GameUI: chapter thumbnails,
menu resources and fonts stay owned by the existing console. That avoids redundant
worker work, but this steady-state benchmark does not isolate loading latency,
input latency, GPU time or peak installation stalls. Physics construction and GPU
resource installation still run on the main thread and can hitch. Zero-hitch
loading and sustained FPS guarantees remain not tested.

Reproduction and evidence: artifacts/chapter-bugs/finish_validation.py waits for
packaged checks/build completion, repeats the affected committed-package replays,
then runs the six isolated processes against retained package copies. It resets
only each copy's own config. timing-summary.json, timing-aggregate.json, individual
timing reports/logs, package-hashes.json and prior/current copies are retained there.

## Chapter browser (2026-10-27)

Packaged Metal on Apple M1 Pro, 1024x768, default settings, live hl2/background01.
Prior package: d7dbc5b; final source package: 7171e3f. Three runs per build/state,
ordered prior idle/current idle/current browser, reversed for the middle round.
No concurrent builds, replays or image analysis. Each scripted run finishes at
host tick 1100 with a 2000-frame safety cap, profiling enabled and no capture.
All reports confirm completion, the intended UI mode and at least 684 measured
frames after 120 warm-up frames (ranges 684–761; roughly 14 measured seconds).

| Package/state | Mean frame | HUD/UI CPU mean |
| --- | ---: | ---: |
| Prior, browser closed | 20.187 ms | 0.2786 ms |
| New, browser closed | 20.138 ms | 0.2748 ms |
| New, browser open | 18.703 ms | 0.1562 ms |

Closed-browser frame ranges overlap: prior 20.011–20.429 ms, new
19.912–20.486 ms. No closed-browser regression is measured in this scene; the
0.048 ms difference does not establish a speedup. Open-browser frames range
18.671–18.757 ms. Opening the window also fades the underlying menu and suppresses
its outlines, so this is a different UI workload, not an isolated widget cost or
proof that extra UI makes every scene faster. HUD CPU includes the whole HUD/UI
scope; its profiler warm-up/history window differs from the frame benchmark.
GPU-only widget time and sustained FPS guarantees are not established.

The browser adds no render pass. Cards, textures, layout values and ellipsized
labels are prepared once; input/drawing do no asset reads or card-list allocation.
Closed windows do no card rendering. The toolkit extraction also avoids cloning
an extra decoded-thumbnail catalog while preparing textures.

Reproduce from the matching package copies with:

```sh
python3 artifacts/chapter-menu/benchmark.py
```

The script runs muted/background copies with their own default config, alternates
all nine runs, and asserts completion/mode/sample count. Scripts perf-idle.json
and perf-browser.json, timing-summary.json, timing-aggregate-toolkit.json and all
nine reports/logs are retained in artifacts/chapter-menu. The earlier pre-toolkit
measurements are retained in before-toolkit and excluded from this table. Native
visual comparison and Windows/Linux runtime remain not tested.


## Stronger Advanced weapon silhouette blur (2026-10-26)

Packaged Metal / Apple M1 Pro, 1024x768, HDR None, MSAA off, VSync off, muted background
`hl2/d1_trainstation_02`. Baseline cbe521b and current package, fixed player view, repeated
crowbar swings every 40 ticks, 900-tick fixture and 2,000-frame cap. Three isolated runs
per build/mode, reversed order for the middle round. Accepted runs measure 552-559 frames
after 120 warm-up frames over 10.922-11.052 seconds; all scripts complete.

| Mode | Prior mean frame | Current mean frame | Prior range | Current range |
| --- | ---: | ---: | ---: | ---: |
| Enabled (retail) | 19.770 ms | 19.797 ms | 19.721-19.808 ms | 19.736-19.882 ms |
| Advanced | 19.744 ms | 19.751 ms | 19.691-19.786 ms | 19.729-19.791 ms |

Advanced differs by +0.008 ms; the unchanged retail control differs by +0.027 ms. Ranges
overlap and these scene totals do not isolate GPU cost or establish an FPS guarantee.
CPU encoding spans average 0.000554 ms (4x reduction), 0.000347 ms (32-pixel tile reduction),
0.000249 ms (tile dilation), and 0.000320 ms (reconstruction). They exclude colour copies
and the prepass; GPU timestamps are unavailable. These are command encoding timings, not
GPU execution times.

Compared with the old filter, Advanced adds one pre-weapon colour snapshot, depth coverage,
and three small motion-field passes. Two full-size colour textures and 1/4 plus two 1/32
resolution motion textures are cached. Tile dilation avoids a full-resolution neighbourhood
search. The adaptive gather runs 16-96 samples only in moving neighbourhoods; stationary
hand detail returns unchanged. No new resources/passes are owned by Disabled/retail Enabled
or absent visible GPU-skinned weapon views. No steady-frame CPU allocation, file access or
vertex-history copies are introduced. Warm-up frames skip snapshots/reconstruction.

The initial third round overlapped CPU image analysis and is retained as rejected-overlap-*
reports/logs. All four third-round runs were replaced, with no concurrent builds, replays
or image analysis. Accepted aggregate contains the original first two rounds and isolated
third-round replacements only. Evidence: artifacts/viewmodel-blur-strength/benchmark.py,
benchmark-third-round.py, timing-summary.json, timing-aggregate.json and individual logs.

## Motion blur Advanced / weapon animation (2026-10-26)

Packaged Metal, Apple M1 Pro, 1024x768, HDR None, MSAA off, VSync off, muted background
`hl2/d1_trainstation_02`. Fixed player view, crowbar attacks every 40 ticks from tick 100,
900-tick script, 2,000-frame safety cap. Preserved parent package: 553ad19; current package:
this checkpoint atop that parent. Three runs per build/mode, reversed order for the middle
round; no concurrent build/replay during timing. All scripts finish, measuring 595-609
frames after 120 warm-up frames, 11.035-11.228 measured seconds each.

| Mode | Prior mean frame | Current mean frame | Prior range | Current range |
| --- | ---: | ---: | ---: | ---: |
| Disabled | 18.444 ms | 18.509 ms | 18.386-18.526 ms | 18.428-18.590 ms |
| Enabled (retail) | 18.403 ms | 18.430 ms | 18.391-18.411 ms | 18.402-18.456 ms |
| Advanced | unavailable | 18.481 ms | unavailable | 18.408-18.546 ms |

Advanced averages +0.051 ms versus current Enabled, +0.078 ms versus prior Enabled.
Ranges overlap; the Disabled difference is itself +0.066 ms, so these scene means do not
isolate GPU cost or establish an FPS guarantee. The new reconstruction draw's recorded
CPU encoding span averages 0.000588 ms; it excludes the frame copy and prepass. GPU
timestamp values are unavailable in these reports, so the CPU span is not a GPU timing.

Advanced adds the weapon-only motion-vector prepass, one cached full-size frame copy,
and a seven-tap half-frame reconstruction (moving pixels only, capped at 4% UV distance).
No weapon prepass, scratch, bind group or draw exists in Disabled or retail Enabled, or
without a visible GPU-skinned weapon. No asset reads, CPU skinning or vertex-history
allocation are introduced per frame. History strings change only on weapon/time resets;
buffers/bind groups are cached. Nine material uniform buffers become one shared buffer,
which also avoids Metal's vertex-binding limit with previous-pose skinning.

Evidence: artifacts/viewmodel-motion-blur/benchmark.py, benchmark.input.json,
timing-summary.json, timing-aggregate.json and all fifteen timing reports/logs. Runtime
shader diagnostic/capture checks and Windows/Linux limitations: docs/validation.md.

## Options platform filtering and development hints (2026-10-26)

Packaged Metal, Apple M1 Pro, 1024x768, default settings, live `hl2/background01` menu,
Mouse tab. Baseline is `2ca7c3d`; new package is this Options checkpoint atop it. Three
runs per build/state, alternating with reversed order for the middle pair, with no concurrent
builds or replays. Pointer idle outside the dialog versus hovering Aim-Assist: prior orange
help alone, new orange help plus cyan status. Platform-hidden controls also change the UI
workload. These numbers do not isolate a single cause or establish an FPS guarantee.

| State | Prior mean frame | New mean frame | Prior HUD CPU | New HUD CPU |
| --- | ---: | ---: | ---: | ---: |
| Idle | 25.145 ms | 22.351 ms | 0.3783 ms | 0.3300 ms |
| Hint hover | 25.496 ms | 25.032 ms | 0.3988 ms | 0.3919 ms |

Frame ranges: idle prior 24.007-27.250 / new 22.083-22.515 ms; hover prior 25.232-25.805 /
new 24.950-25.119 ms. No frame regression was measured in this scene. The first idle baseline
is noticeably slower; the lower new means do not prove a general speedup. The HUD column
is the existing `cpu_stages.hud` mean, including the whole HUD/UI preparation; its profiler
warm-up/history window differs from the frame benchmark. It is not an isolated hint CPU slice.

Scripts finish at tick 900 idle / 1,500 hover with a 2,000-frame safety cap. All accepted
reports confirm completion, Mouse tab and at least 300 measured frames after 120 warm-up
frames: idle 341-466 (9.29-10.49 measured seconds), hover 725-768 (18.29-19.29 seconds).
The first hover control still used the shorter 900-tick script while the hover window was
being extended. Its report/log is retained as `rejected-short-hover.*`; a matching 1,500-tick
control replaces it in the accepted aggregate. No short control is mixed into the hover mean.

No new render pass or asset-loading path. Development text and wrapping are prepared on
hover changes and reused while hovering the same control. Typed status lookup is constant
policy; platform filtering and settings binding normalization add no heap allocation.
Markers add one small quad per unfinished control, only while Options is drawn.

Evidence: `artifacts/options-status/timing-summary.json`, `timing-aggregate.json`, each
`timing-*.json`/log and retained package metadata. Reproduce with
`python3 artifacts/options-status/benchmark.py` (own package copies, muted, default settings).
Windows/Linux runtime and different UI sizes/scenes are **not tested**.

## Flashlight lifecycle (2026-10-26)

Packaged Metal, Apple M1 Pro, canal fixture view, 1920x1080, HDR None, Medium shadows.
Control is the lamp-fixed package (`70732bc` behavior); new package includes the reviewed
availability/lifetime implementation. Three runs per build per off/on state, alternating
and reversing order for the middle pair. No concurrent builds/replays. Tick 900 with a
2,000-frame safety cap; every report confirms script completion and the expected light
state. After 120 warm-up frames, runs measure 549-559 frames over about 10.88-11.09 seconds.

| State | Prior mean frame | New mean frame | Prior range | New range |
| --- | ---: | ---: | ---: | ---: |
| Off | 19.891 ms | 19.875 ms | 19.823-19.946 ms | 19.802-19.915 ms |
| On | 19.861 ms | 19.920 ms | 19.828-19.893 ms | 19.810-19.990 ms |

Ranges overlap; these results do not isolate a speedup or establish an FPS guarantee.
No new render pass/light, frame asset access or normal-frame allocation is introduced.
Permission requests allocate only when authored inputs arrive; their vector is retained.
The old per-frame game-name String allocation is removed. This measures total frame time,
not a separately instrumented controller CPU slice. Evidence: `artifacts/lighting-bugs/
lifecycle-timing-summary.json`, `lifecycle-aggregate.json`, per-run reports/logs and
`lifecycle-timings.py`. The script checks at least 300 measured frames and expected light
state. Reproduce from the retained packages with `python3 artifacts/lighting-bugs/lifecycle-timings.py`.

## Fluorescent self-illumination (2026-10-26)

Packaged Metal, Apple M1 Pro, 1920x1080 HDR None, canal fixture camera recorded in
`test-inputs/bevy-canals-lamp.json`. Prior package `e3713d0`, corrected shader atop
`02e6546`. Three alternating runs per package, no concurrent builds/replays; tick 900,
2,000-frame safety cap, completed scripts. Each measured 561-607 frames after 120 warm-up
frames (11.10-11.21 seconds). Prior/new means: **18.875/18.921 ms** (+0.045 ms).
Pairs: 18.420/18.473, 18.485/18.511, 19.721/19.778 ms. The third pair slowed in both
packages; this does not isolate GPU shader cost or establish an FPS guarantee.

No CPU loop, allocation, file read, light or render pass was added. The change preserves
unlit emissive albedo before baked modulation and uses it only in the selfillum branch.
Evidence: `artifacts/lighting-bugs/lamp-summary.json`, each package's `*-timing-*.json`
and logs. Reproduce each package three alternating times with `--background --no-setup
--mute --map d1_canals_01 --width 1920 --height 1080 --frames 2000 --movement-script
artifacts/lighting-bugs/lamp-benchmark.json --report artifacts/lighting-bugs/reproduce.json`.

## Flashlight on the weapon (2026-10-25)

Packaged Metal, Apple M1 Pro, `d1_trainstation_02`, 1920x1080, HDR Full, Medium shadows.
Clear close-wall view: eye -4324 -224 64, yaw 180, pitch 0; pistol selected at tick 20.
Baseline: `cdaf4be`; new package: this continuation atop it. Three alternating runs per
build/state, with rotated order, no concurrent builds or other agent replays. Scripts end
at tick 900 (13.5 simulated seconds), with a 2,000-frame safety cap. Each report confirms
completion and at least 300 measured frames. Actual reports contain 559-574 measured
frames after 120 warm-up frames (679-694 total), about 10.9-11.1 measured seconds per run.

| State | Prior mean frame | New mean frame | Prior model-light CPU | New model-light CPU |
| --- | ---: | ---: | ---: | ---: |
| Flashlight off | 19.484 ms | 19.490 ms | 0.02638 ms | 0.02710 ms |
| Flashlight on | 19.525 ms | 19.328 ms | 0.02726 ms | 0.02864 ms |

Frame ranges: off prior 19.463-19.497 / new 19.388-19.576 ms; on prior 19.491-19.575 /
new 19.190-19.435 ms. No frame regression was measured in this view. The lower new on
mean does not isolate a speedup or GPU cost. Model-light presentation adds about
0.0007 ms off and 0.0014 ms on. No extra light, render pass or shadow map was added;
the beam matrix is computed once per frame when lit, and changed uniforms are published
only to viewmodel materials. Off/dark returns disabled data without projection work.
No frame allocations, asset reads or font work were added. These results do not establish
a general FPS guarantee, costs for every weapon/view, or native timing parity.

Evidence: `artifacts/flashlight-viewmodel/timing-summary.json`,
`perf-{baseline,current}-{off,on}-{1,2,3}.json` and their logs; `benchmark.py` records the
rotated launch order and fails below 300 measured frames. Reproduce one launch from the
retained copy and its timing script (reset the copy's saved settings to the values above):

```sh
artifacts/flashlight-viewmodel/current/hl2-bevy --background --no-setup --mute --map d1_trainstation_02 --width 1920 --height 1080 --frames 2000 --profile --movement-script artifacts/flashlight-viewmodel/perf-on.json --report artifacts/flashlight-viewmodel/reproduce.json
```

Repeat off/on three times per package using `perf-off.json` for off. Capture fixtures pause
at tick 120 and are not timing substitutes. Earlier High and power-HUD descriptions below
have been corrected: their 600-frame cap was not reached because the script ended sooner.
Their numbers remain recorded with the actual short-window limitation.

## High flashlight depth shadows (2026-10-25)

Packaged Metal on Apple M1 Pro, `d1_trainstation_02`, 1920x1080, HDR Full,
a 600-frame safety cap, profiling on. Prior package: `f0d6945` (High still had no flashlight depth
map); new package: this continuation's constant-tap filter. Three runs per state,
without concurrent builds/replays. Cameras: plaza eye -3104 -2018 200, yaw 12, pitch -10;
citizen queue eye -4040 -2460 136, yaw 90, pitch -8. The queue capture visibly contains
three ordinary citizen NPCs. Scripts grant the suit/toggle the light at tick 1, finish
at tick 240 and end the launch early. These runs do not reach 600 frames.

| View and state | Mean frame | Median | Run range |
| --- | ---: | ---: | ---: |
| Plaza, prior High (no depth) | 20.044 ms | 19.943 ms | 19.066-21.124 ms |
| Plaza, new Medium | 20.600 ms | 20.758 ms | 20.274-20.768 ms |
| Plaza, new High | 21.979 ms | 22.052 ms | 21.377-22.507 ms |
| Three citizens, prior High (no depth) | 17.525 ms | 17.456 ms | 17.070-18.049 ms |
| Three citizens, new Medium | 17.576 ms | 17.266 ms | 17.135-18.326 ms |
| Three citizens, new High | 21.359 ms | 20.205 ms | 19.821-24.050 ms |

Correction recorded in the viewmodel continuation: these are short script-limited windows,
not 600-frame measurements. The nine `opt-plaza-*` reports contain 126-179 total frames
and only 6-59 measured frames after the 120-frame warm-up. Retain the numbers as observed,
noisy evidence; they need longer runs before precise cost attribution.

High minus Medium adds about 1.38 ms in the plaza and 3.78 ms mean in the citizen
view (2.94 ms by medians). The citizen High result includes a 24.05 ms outlier. These
are total frame costs of the requested depth pass and filter, not isolated GPU timings
or FPS guarantees. The plaza's shared lit path is 0.56 ms above the prior mean, within
the prior run range; that difference has not been isolated. In the citizen view the
Medium/prior means differ by 0.05 ms.

A later paired flashlight-off plaza set has prior/new medians 19.422/19.286 ms,
means 21.171/19.310 ms and ranges 19.289-24.803 / 19.185-19.458 ms. The prior set has a
large outlier; it does not demonstrate a speedup. No off regression was measured in these
runs. Off, Low/Medium and zero-brightness battery flicker create no depth pass. The final
battery-flicker guard was added after these timed runs; its normal lit-beam path is
unchanged and the packaged image checks were repeated afterward.

Cost controls: only High/lit enables the 1024-square map; cookie-zero, far-faded and
back-facing fragments skip its eight comparison samples. Existing Bevy visibility,
GPU skinning and opaque draws are reused; alpha-tested casters sample alpha in the
custom depth fragment. Inspection established that Bevy already omits the opaque fragment
stage, so a redundant override was removed. Moving the Poisson table to shader constants
preserved decoded capture pixels (fence identical, actor max one level); no measurable
speedup is claimed. A single depth-only control (maps rendered, filter bypassed) was too
noisy to isolate filter cost. Render-graph wall times include waits; GPU timestamps were
unavailable, so those times are not attributed to the shadow pass. Further batching or
filter tradeoffs need new evidence and fidelity review; no extra batching mode was enabled.

Evidence in `artifacts/flashlight-shadows`: `timing-summary.json`,
`perf-baseline-plaza-high-*`, `opt-plaza-{medium,high}-*`,
`final-crowd-{baseline-high,optimized-medium,optimized-high}-*`,
`final-plaza-{baseline,optimized}-off-*` reports and logs. Earlier `citizens`-named runs
showed mostly cops/benches; they are retained as a supplemental view, excluded from the
citizen table. Initial mismatched-resolution/HDR captures and two wall-facing fence views
are rejected, as recorded in STATUS and validation. No native timing comparison or
Windows runtime measurement was performed.

Reproduce a timed High run from the disposable package and its retained timing script:

```sh
artifacts/flashlight-shadows/optimized/hl2-bevy --background --no-setup --mute --map d1_trainstation_02 --width 1920 --height 1080 --frames 600 --profile --movement-script artifacts/flashlight-shadows/perf-crowd-high.json --report artifacts/flashlight-shadows/reproduce.json
```

Reset the copy's saved video settings to 1920x1080, HDR Full before each run; repeat
High/Medium three times using `perf-crowd-medium.json` for Medium. Use
`perf-plaza-{high,medium,off}.json` for the plaza. Public capture fixtures pause at tick
120 and are not timing substitutes. The optimized copy retains the timed package; the
final package adds only the zero-brightness gate described above.

## Flashlight power HUD (2026-10-25)

Packaged Metal, Apple M1 Pro, `d1_trainstation_02` spawn corridor, eye -4304 -224 64,
yaw -90, pitch -10, 1920x1080, a 600-frame safety cap, profiling on. Three alternating flashlight-off
and flashlight-on runs of each build, without a build or another replay running alongside.
Baseline package: `822e5f8`; new package: power HUD changes atop that revision. Copies use
the same saved settings; timing scripts give the suit and toggle the flashlight at tick 1,
then finish at tick 240, which ends the launch before its frame cap.

| State | Prior mean frame | New mean frame | Prior HUD CPU | New HUD CPU |
| --- | ---: | ---: | ---: | ---: |
| Off / fully recharged | 18.330 ms | 18.243 ms | 0.0365 ms | 0.0390 ms |
| On / auxiliary panel visible | 18.509 ms | 18.672 ms | 0.0383 ms | 0.0566 ms |

These power-HUD runs contain 182-189 total frames and 62-69 measured frames after warm-up;
the earlier 600-frame wording described a cap that was not reached.

The active HUD adds about 0.018 ms CPU to the prior on case, emitting 55 HUD quads instead
of 20. Off emits the same 20 quads. New on runs are 19.147, 18.470 and 18.399 ms, versus
18.536, 18.544 and 18.446 ms before. Frame time is noisy; the mean increase does not isolate
the GPU cost of this small panel or establish an FPS guarantee. No render pass was added.
Labels/icons are prepared on the map worker; drawing uses retained glyph quads.

Evidence: `artifacts/power-hud/timing-summary.json`, `base_bounded_{off,on}_{1,2,3}.json`,
`current_{off,on}_{1,2,3}.json` and their logs. With the completed 240-tick evidence script,
run the packaged copy as follows (substitute `off` for the other state):

```sh
artifacts/power-hud/current/hl2-bevy --background --no-setup --mute --map d1_trainstation_02 --width 1920 --height 1080 --frames 600 --profile --movement-script artifacts/power-hud/bounded_script_on.json --report artifacts/power-hud/reproduce.json
```

Those scripts preserve the camera above and exit when the script finishes, without pausing. Repeat off/on three
times per build. The replay fixtures also validate the HUD but pause for their final capture,
so they are not substitutes for the timing scripts.
Rejected sandbox launches could not access Metal. The first GPU launch rejected a 900-tick
script before completion at 600 frames; none of those rejected runs enter the table.


2026-10-08 owner-report milestone: prepared frame textures are shared and frame updates perform no asset I/O. Encoded/decoded budgets are bounded. Performance impact and FPS on the new material path are **not tested**; capture bursts verify visibility stability, not throughput. Existing measurements below retain their original scope.

Performance is a current limitation, not a completed migration milestone. The user reported about22 FPS versus about60 in the retained build. A local packaged station02 spawn run reproduced about30 FPS at1920x1080 on the RTX3070/Vulkan adapter. The hosts use different texture mip limits (Bevy2048, retained512); neither a matched native benchmark nor144 FPS is established.

Optional `--profile` measures bounded CPU scopes after the first30 calls. It does not measure GPU execution or the entire render-thread cost. The same600-frame default-VSync scene before/after changed-collider query refresh and sampled reports measured:

| CPU scope | Before mean ms | After mean ms |
| --- | ---: | ---: |
| Fixed simulation, per15ms tick | 4.02 | 2.61 |
| Full diagnostics, per presentation | 1.39 | 0.022 |
| CPU mesh animation, per presentation | 5.79 | 5.75 |

Frame rate remained about30.2 versus30.6 FPS. A subsequent constant-pose upload fix plus `--uncapped` measured about35.1 FPS with animation4.63ms. This last run also changes presentation mode and is not an isolated FPS comparison for that fix. The remaining render/animation cost needs investigation. Do not infer a144 FPS ceiling or promise that this build reaches it.

Physics previously rebuilt the query tree twice every fixed step and marked unchanged poses as moved. It now incrementally refreshes actual changes;400 swept collision comparisons match full rebuilds across moves, rotations and visibility changes. Animated surfaces share rig samples, and constant poses no longer dirty/upload their meshes every frame. Full diagnostic snapshots were sampled every 60 frames; since 2026-11-21 they are built only on the first frame and for capture requests, scripted/bounded completion and UI quit (the periodic snapshot was a 4-12 ms hitch every 60 frames in all play).

Reproduce with `launch-bevy.cmd --width 1920 --height 1080 --frames 600 --profile --report artifacts/performance.json`. Add `--uncapped` to request no-VSync presentation. Capture/JSON writes occur after shutdown. Current priorities are GPU upload/render-stage measurements, visible/PVS animation workload and a verified GPU skinning path. Preserve animation/eye/collision behavior when optimizing; do not lower fidelity silently to improve a benchmark.

## GPU skinning follow-up

The packaged binary uses Cargo's **release** profile. Development dependencies already have optimization overrides; changing those overrides does not change the release executable. The regression was reproduced in release mode.

Source still samples the same engine-independent animation clips. The Bevy adapter now sends joints and inverse bind poses to Bevy's GPU skinning pipeline instead of replacing each mesh's vertex positions every frame. Bone matrices include the Source-to-Bevy basis and model scale; weights preserve the CPU adapter's normalization and bind fallback. Surfaces of one actor share a skeleton. Models beyond Bevy's 256-joint budget retain CPU skinning. Eye UVs use the same sampled pose, and dynamically transformed joint bounds conservatively enclose animated vertices for frustum culling. Unchanged entity transforms are no longer marked dirty each frame.

`--cpu-skinning` selects the previous CPU path for controlled comparisons. `--profile` now records render-stage CPU wall times and Bevy's GPU/pass diagnostics; profiling adds timing fences and GPU queries, and per-pass GPU aggregates combine views with the same diagnostic name. They are not a complete GPU-frame measurement. The HUD report includes an average after 120 warm-up frames alongside its rolling one-second FPS.

The first controlled GPU run reached roughly 68 FPS uncapped at 1920x1080, versus roughly 35 with CPU skinning. CPU animation was about 0.25 ms and asset preparation about 0.56 ms, versus 4.77 and 11.60 ms. A subsequent matched-package 900-frame run averages 35.05 FPS with CPU skinning and 66.85 FPS with GPU skinning after 120 warm-up frames. See validation for scope and limitations. This is a substantial improvement, not verification of 144 FPS or Source performance parity.

Two controlled station01 close-ups compare Barney's monitor and player LOOKAT interests on both paths. Eye targets/projections agree exactly in reports, and the actor image region differs by less than 0.006 mean channel values on the 0-255 scale. Dynamic physics props elsewhere can differ between independent runs; whole-image identity is not claimed. This validates the migrated presentation against the previous Rust adapter, not the original engine's complete animation behavior.

## BSP PVS follow-up

Debug compilation was not the cause of this regression: the packaged executable is release-built, and development dependencies already have optimization overrides. Profiling instead showed substantial work preparing and binding geometry outside the current room. BSP PVS rejection now supplements Bevy's frustum culling. It uses a validated cached tree, padded current bounds, cached cluster rows and the union of active player/monitor cameras. It never suspends gameplay simulation or bases animation decisions on the previous frame's visibility.

Matched-package 900-frame uncapped/profiled station02 spawn runs at1920x1080, with120 warm-up frames, give:

| Measurement | PVS disabled | PVS enabled |
| --- | ---: | ---: |
| Average FPS | 66.69 | 156.94 |
| Average frame, ms | 14.995 | 6.372 |
| Render preparation CPU, ms | 5.857 | 1.518 |
| Render graph CPU wall interval, ms | 7.392 | 3.318 |
| Source visibility CPU, ms | 0.009 | 0.032 |

PVS rejects824 of886 tagged draws at this position. An inspected spawn comparison is pixel-identical.798 bounds checks were reusable at the final sample; the initial implementation retraversed every bound and cost about0.96ms per frame. The loaded mesh/material counts stay fixed across900 frames; they are not visible draw counts. Finer render preparation timers identify phase-buffer writes and bind-group preparation as major costs reduced by rejection.

Reproduce with `launch-bevy.cmd --width 1920 --height 1080 --frames 900 --uncapped --profile --report artifacts/pvs-on.json`; add `--no-pvs` for the reference run. Do not compile or run another host concurrently with a benchmark. The camera is stationary but autonomous scene time advances: faster runs cover fewer simulation seconds. These are room-specific measurements with profiling overhead, not a matched native campaign benchmark or proof of sustained144 FPS. Area portals/occluders, Source LODs and spatial partitioning of broad material batches still need work.

## Shared material handles

The map adapter now reuses a material handle for the same authored material and lightmap pair. Owner transforms, skeletons and eye UVs remain independent. Current implemented monitor proxies depend on the authored definition and shared scene clock; each shared handle is evaluated once, and unchanged proxy values no longer dirty the GPU asset. Future entity-specific material inputs will require separate handles or copy-on-write.

Station02 material assets fall from921 to743 with all921 draw meshes retained. Station01 uses695 material assets for1109 meshes. The reported material count now counts unique map material handles; it is not a visible draw or texture count. The station02 spawn image and Breen feed are pixel-identical to preceding captures. A matching Barney player-interest image crop is also exact, with unchanged per-actor eye origins/targets/projections. Global eye reports include the intervening Kleiner idle fix, so they are not claimed identical to pre-activity reports.

A sequential900-frame1080p uncapped/profiled spawn run averages174.72FPS/5.723ms after120 warm-up frames, compared with154.96FPS/6.453ms in the preceding activity package. Asset preparation averages0.159ms versus0.541ms; render preparation1.476ms versus1.521ms. This is an observed package comparison in one room, with autonomous scene time and normal run-to-run variation, not a whole-campaign144FPS guarantee. No simulation, texture resolution, model geometry or gaze updates were disabled.

## Optional spatial batches

`--world-partition` groups whole opaque/cutout world triangles into1024-unit cells during map setup. It preserves vertex attributes and winding, retains actual bounds for triangles spanning cells, and leaves entity, sky and translucent batches unchanged. Invalid data or more than64 cells falls back to the original batch. It is disabled by default pending broader benchmarks.

Station02 retains324429 triangles and743 materials, while loaded draw meshes rise921→1670. In the inspected plaza, union-of-view candidates fall69425→32011 triangles; its image is pixel-identical. These are candidates after PVS/frustum/layer checks, excluding sky/viewmodels, not GPU draw-call or fragment counts. Preliminary sequential900-frame1080p profiles measured159.74→165.91FPS at spawn and125.65→127.01FPS in the plaza; normal run variation and different autonomous scene durations prevent a robust general speed claim. A later frozen-scene diagnostic intentionally ends before its script completes and returns an incomplete-script error; it is excluded from these comparisons.

An initial hall camera intersected a pillar and was rejected as visibility evidence. Area portals, occluders, native LODs and campaign-wide culling validation remain unfinished.


## Bounded performance research: fixed-duration indoor sensitivity (2026-11-11)

An isolated release package at `2208bcb618adfd3072ea5fd00553babdcb9a7866` (SHA256
`51cf256188b5e0446165cc2dc55ad90729768143683151f9c4e5182c41895fc5`) ran on M1 Pro / Metal,
10 logical CPUs / 16 GiB, advanced-options/debug-tools plugins. Station02 indoor spawn,
fixed script ending tick 600, muted headless, uncapped/profiled, no capture while timing;
1920x1080, FOV75, HDR Full, High details/shadows, no AA, trilinear, motion blur/color correction
on. Three interleaved rounds: median of mean frame times, baseline 5.811 ms (5.751–5.879),
720p 5.781 (5.735–5.982), no-PVS 7.736 (7.674–7.784), partition 5.775 (5.773–5.811), CPU
skinning 12.568 (12.476–12.640), Low 5.680 (5.650–5.796). Baseline p95 bucket upper bounds
10/9/9 ms, p99 13/11/11; exact p50 is unavailable. Full trial and tail table:
[performance-research.md](performance-research.md).

All 18 valid runs exited 0, reached tick 600 and completed; two invalid-Low config attempts
remain rejected. Existing PVS/GPU skinning reduce median frame time substantially versus their
disabled/fallback controls; this is no new runtime optimization. Partition cuts candidate
triangles 52,768 to 7,257 but barely changes frame time; 720p and partition differences are within
variation. Low changes quality and overlaps baseline variation. CPU candidates are not GPU draws.
The untimed 1080p capture shows the indoor stairwell, not the crowd/plaza.

The 120-frame warmup covers different simulation intervals at different throughput. CPU scope
windows include setup and overlap; graph CPU wall intervals are not GPU timestamps. Existing
compression/about 640 MiB swap qualify timings; full GPU/power/thermal counters are unavailable.
The package omits four newer upstream-main commits, including flight/sight query improvements.
Windows RTX 3070 matched reproduction, live presentation, local crowds, native settings and general FPS
improvements are **not tested**. No source/build modification or installed-game write occurred.

Evidence and reproduction: `artifacts/performance-research-2026-11-11/summary.json`,
`measurements.json`, per-run reports/logs/configs, `baseline-visual.png`, environment snapshots;
`python3 artifacts/performance-research-2026-11-11/benchmark.py` from the repository root.


### Correction: original performance observation hardware (2026-11-11)

Subsequent direct clarification of the original observation identifies RTX 3070, not the
previously supplied RTX 5070: native station02 plaza with many colliding/sparking manhacks
read 276 FPS, while the rewrite reportedly averaged barely 60 FPS and less with many manhacks.
The observer reports a 16-thread CPU at 4 GHz and 64 GB RAM at 3200; exact CPU model, resolution,
quality, view, actor count and timing method are not established. The earlier RTX 5070 account
is conflicting/provisional. The historical Windows profile remains a separate instrumented
session; none of the fresh Mac measurements changed. A separate contemporaneous observation
reports M1 Pro / maximum graphics / 3456x2168 / idle plaza 65–80 FPS at `2208bcb`, not the controlled
1080p indoor headless workload. No new runtime test or speedup follows from this correction.


## Implemented effect/glow experiments: no established frame gain (2026-11-11)

M1 Pro / Metal, 10 logical CPUs / 16 GiB, identical explicit High/HDR Full/no-AA settings and plugins,
three paired rounds over five fixed 600-tick cases (30 launches), no capture/competing build during
timing. Baseline 2208bcb SHA256 51cf256188b5e0446165cc2dc55ad90729768143683151f9c4e5182c41895fc5;
candidate a22f6a0 plus source-manifest runtime edits, SHA256
51002f989e04e2f8d72dc96bfa0fc55d1ce1c38ce17b7f776b7939f310cd07ab. Shader hashes match.

Median frame ms before→after:quiet 5.5359→5.5493; frozenfire5.9949→5.9566; animated fire7.4000→7.3968;
crowd 1080p13.5671→13.6421; crowd3456x2168 13.8773→13.8715. All 15 full gameplay-state pairs match.
Ranges overlap, and 1080p crowd p99 bucket bounds worsen 23/23/23→24/25/26ms. Frozen contained-window
median 5.9182→5.7931ms is secondary/mixed, not an established whole-game win. Swap 706→2339 MiB and
~1.66 GiB system swapout traffic qualify interpretation. Complete trials/tails/methods:
[performance-research.md](performance-research.md).

The geometry helper improves ~0.242→0.0086 us, but 142 hypothetically changed quads save only ~0.033 ms;
the frozen baseline already skipped unchanged geometry. Glow 30 actors / 1k-prefix saves ~0.0108 ms.
These presentation helpers do not reduce NPC decisions, exact world sweeps, Rapier or draw count.
Current crowd scopes remain millisecond-scale (median means: fixed 5.648 ms/call, graph 2.582,
assets 1.951, prepare 1.035), overlap and include setup; the assets mean includes ~501 ms setup cost.

Separate 1800-tick current-only sampled diagnostics conserve thread observations:shared static
brush trace295quiet/710 crowd, optimized glow 10 crowd / quad pool 2 frozen. Crowd fixed-system 1527,
physics-step 379 includes rebalance 141 / broadphase 161; do not sum ancestors/descendants or treat
counts as milliseconds. Main 2101/2129 snapshots wait while workers run. The full brush-plane scan
is shared by NPC/player hull traces, mostly NPC flight in this crowd. First validate/profile
latest shared baseline including published flight/sight fixes before a new candidate-index
experiment; preserve ordered exact plane/epsilon/start/all-solid/normal behavior and fallback
brushes. This diagnosis is not a speedup measurement or a completed next-cycle change.

All 6 fixed-exposure particle checks pass; fire/smoke images exact, ball 61 pixels / max 1 RGB remains
unresolved. Ordinary HDR cross-build AND baseline-repeat byte checks fail; retained separately.
No original 30 timing run was replaced by fixed-exposure timing. Evidence:
`artifacts/performance-implementation-2026-11-11/paired-summary.json`,source/package manifest,
raw reports/logs,controlled/regression summaries,CPU experiments and diagnostics/stack-summary.json.
Windows/native matched timing and full-campaign/all-camera runtime parity remain **not tested**.

## O32 plaza population sweep and O12 pre-launch GPU load (2026-11-24)

On Apple M1 Pro / Metal, release `fa90617c` (executable SHA-256 prefix
`98ec717c`), the same 100-position camera path, final plaza view, 1920×1080 settings and
1,200 fixed ticks ran at each population. The single class was `npc_manhack`; the zero
case still includes map-authored NPCs. Only the first N positions issue a spawn command.
The benchmark checks unique successful placements and emits two labeled snapshots. Run
`python3 scripts/checks/crowd_scaling.py --out artifacts/o32-stages` using a fresh output
directory. Add `--mac-trace` with a second directory for Metal and sight-ray stack samples.

| Added NPCs | AI thinks | Fixed simulation | AI tick | Hitbox pose | Animation | Render assets | Render graph CPU | Frame mean | Frame p95 / p99 bounds |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 2,767 | 0.969 | 0.082 | 0.165 | 0.231 | 0.164 | 2.361 | 4.221 | 5 / 6 |
| 10 | 4,328 | 1.184 | 0.213 | 0.194 | 0.269 | 0.191 | 2.502 | 4.402 | 6 / 6 |
| 30 | 7,574 | 1.355 | 0.353 | 0.229 | 0.318 | 0.216 | 2.741 | 4.793 | 6 / 8 |
| 60 | 12,443 | 1.920 | 0.708 | 0.307 | 0.455 | 0.286 | 2.807 | 5.709 | 10 / 11 |
| 100 | 18,342 | 3.071 | 1.624 | 0.386 | 0.643 | 0.387 | 2.911 | 7.145 | 12 / 14 |

Times are milliseconds per stage invocation. AI and hitbox scopes run per simulation tick.
Frame tails are histogram upper bounds. Each row is one headless run including the placement
phase. The existing profiler caps each stage at 4,096 samples (O11). The 60-to-100 interval
added 0.916 ms per AI tick, compared with 0.355 ms from 30 to 60. This identifies a target
for profiling. The sweep alone cannot establish algorithmic complexity or live display pacing.

Four-second instrumented repeats used the single HUD fragment pass to delimit each GPU
frame. The default upscaling marker appeared twice per frame in this view. Sight counts
are inclusive CPU stack samples inside a fixed step (`PhysicsSight::visible` or
`first_solid`), with nested calls counted once. The zero case had no identifiable sight
samples in this window. The counts measure sampled time, not ray calls.

| Added NPCs | GPU frames | GPU busy mean / p95 / p99 (ms) | Sight / fixed-step samples |
|---:|---:|---:|---:|
| 0 | 699 | 3.92 / 5.54 / 6.29 | 0 / 103 |
| 10 | 528 | 4.09 / 5.99 / 6.45 | 10 / 119 |
| 30 | 471 | 4.58 / 6.28 / 6.92 | 19 / 130 |
| 60 | 473 | 4.38 / 6.60 / 7.08 | 34 / 185 |
| 100 | 350 | 5.07 / 7.46 / 8.61 | 118 / 340 |

A repeat of the final recorder path, attaching four seconds after gameplay started,
measured 5.006 ms mean GPU busy over 417 frames at 100 manhacks. Sight accounted for
146/370 fixed-step samples.

O12 verification: the status preflight showed 95% GPU device utilization before the sweep.
The capture metadata recorded 68%, 79%, 79%, 77% and 14% immediately before launching the
0, 10, 30, 60 and 100 cases respectively (the 100 case ran first). Both live and headless
entry points take this sample before spawning the game. Four earlier live metadata files
also contain readings of 17%, 56%, 56% and 58%. Each percentage is one driver reading.

The dedicated O12 preflight later read 34%, followed by 7% in the stamped capture.
That 100-manhack isolation run (`--no-animation`) finished all 1,200 ticks and retained
100 successful placements. The animation stage disappeared and AI measured 1.610 ms/tick,
compared with 1.624 ms/tick with presentation animation enabled. This was a single
isolation control. The graphics settings and idle load limit comparisons with other runs.

O24 changes only the capture fixture. At normal speed the 1024×768 pistol-fire bursts
recorded positive reconstruction with no borrowed motion or uncovered holes; see
docs/validation.md for the accelerated and unequal-bloom captures that were rejected.
