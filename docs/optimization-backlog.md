# Optimization backlog

Goal (owner, 2026-11-21): maxed settings at native Retina (3456x2168, 4x MSAA, full HDR, motion
blur, anisotropic 8x, VSync) on an Apple M1 Pro at a steady 120 FPS: the slowest 1% of frames under
8.33 ms. Method: capture, add findings here, fix the largest or most frequent cost, re-capture the
same path. Tooling and capture layers: docs/DESIGN.md "Profiling toward a steady 120 FPS";
measurements: docs/performance.md.

Each item: evidence (capture, numbers), suspected cause, next step. Status: open, measuring,
fixing, done (with the commit and the before/after numbers), rejected (with the reason).
Numbers from trace builds are inflated by the trace itself; compare them only with each other.

## Captures

| # | Date | Build and settings | Path | Data |
|---|---|---|---|---|
| 0a | 2026-11-21 | release, maxed Retina, console open, macOS `sample` 60 s | d1_trainstation_02, running | CPU stacks |
| 0b | 2026-11-21 | release `--profile`, maxed Retina | same | stage times, 4,096 frames |
| 0c | 2026-11-21 | `trace-chrome`, `--profile`, process sampler | same, 2 min | system spans, GPU/CPU/RAM samples |
| 1 | 2026-11-21 | `trace-chrome` with game spans and state timeline, `--profile`, process sampler; idle GPU 24% before launch | same, about 2 min, moving, shooting the SMG, near the station crowd | 7,058 traced frames, game spans per actor, NPC state |
| 2 | 2026-11-21 | capture 1 plus `trace-alloc` (allocations per frame, about 0.6 ms of its own) and a 45 s Metal System Trace attached; idle GPU 45% before launch | same, about 2.5 min | 8,327 traced frames, 151,194 GPU intervals, displayed-surface timing. The three recorders together slow the game to about 51 FPS, so absolute times are inflated |

## Items

| ID | Area | Status | Finding | Next |
|---|---|---|---|---|
| O1 | GPU | open | GPU busy 97% median (0c, 1) at maxed Retina: the first limit. A headless Metal System Trace at 3456x2168 (one view): GPU busy 5.7 ms mean, p95 12.0, p99 16.1 per frame, 75 of 461 frames over 8.33 ms; transparent 3D 1.11 ms, opaque 3D 1.09 + 0.34 ms vertex, bloom 0.89 ms (2.6 passes), MSAA writeback 0.71 ms, early prepass 0.50 ms per frame. Windowed (2), GPU ms per frame: opaque 3D 3.06 fragment + 0.79 vertex (3.2 passes), transparent 3D 2.36 (3.6 passes), weapon reduce 1.42 + 0.28, early prepass 1.02, world motion blur 0.77 + composite 0.36, bloom 0.74 (4.6 passes), transparent 2D 0.62, MSAA writeback 0.33, opaque 2D 0.28 | See O20 for the viewmodel blur; per-camera split of the 3D passes |
| O20 | GPU spikes | open | `viewmodel_animation_blur` (Motion Blur: viewmodel) averages 3.15 ms GPU per frame but runs in 70% of frames at 4.6 ms each, p99 18.2 ms, max 22.4 ms (2): the largest GPU spike source and alone over twice the 8.33 ms budget at its p99 | Why its cost varies by 5x (sample count, blur length, resolution); bound its worst case |
| O2 | GPU / render | open | Five 3D camera passes and two 2D per frame (0c), each at full Retina with 4x MSAA | Per-camera GPU cost; which cameras need full resolution and MSAA |
| O3 | Render thread | open | Swapchain wait `prepare_windows` 4.5 ms mean, p99 21.6, max 193 ms (0c): the GPU limit reaching the CPU | Follows O1 |
| O4 | Render thread | open | The render thread paces the frame: 16.5 ms mean, p99 32.8 (1); inside it render graph encoding 7.3 ms, `camera_driver` 5.2 ms, swapchain wait 3.4 ms (p99 20.0) per frame; `Core3d` 4.7 times per frame (Camera 0 1.66 ms, Cameras 1 and 2 0.88 and 0.95 ms, Cameras -1/-2/-3 about 0.5 ms each) | Per-node CPU cost; draw counts per camera |
| O5 | Render prepare | open | `PrepareViews` (`render_views`) 4.6 ms mean, p95 13.9 (0b/0c); `prepare_clusters_for_cpu_clustering` in the CPU stacks (0a) | Which lights and clusters; per-system spans inside `PrepareViews` |
| O6 | Animation | partly done | Fixed 2026-11-21 for the renderer and the eyes: a shared `ActorPoses` cache (one skeleton per actor, rig and pose stamp, reused across frames while the pose is unchanged). Headless trace, 600 frames at d1_trainstation_02: pose compositions 35.7 -> 22.0 per frame (0.326 -> 0.202 ms), frames with a repeat 411/783 -> 100/834, extra calls 7,589 -> 1,786, `eyes` stage 0.257 -> 0.059 ms mean, `animation` 0.661 -> 0.702 ms (the cache's own lookups). Remaining repeats come from the shadow caster, the view attachment and impacts, which do not use the cache yet. Before the fix: the same skeleton composed more than once per frame: in capture 1, 289,888 extra `actor_pose` calls in 6,575 of 7,058 frames (2.1 s of the run); capture 2: 438,784 extra calls in 8,171 of 8,327 frames (3.8 s); 85 pose calls per frame, 0.78 ms per frame. The eyes pass recomputes full skeletons the renderer already posed (452 of 551 pose samples, 0a) | Share one pose per actor per frame; the largest confirmed waste so far |
| O7 | UI | open | Open console: whole history redrawn every frame, a font mutex and hash per glyph, canvas quad list regrown (memmove) every frame (0a) | Cache console rows; keep quad capacity |
| O8 | 2D | open | HUD/UI `Transparent2d` draws and `mesh2d` specialization checks every frame (0a) | Count 2D draws; specialize only on change |
| O9 | Stutter | open | One-off spikes: fixed step 32 ms, `allocate_and_free_meshes` up to 598 ms while loading, `prepare_erased_assets<SourceMaterial>` max 27 ms (0c) | Attribute with the game spans and state timeline |
| O10 | Memory | open | RSS 1.6 GB median, 4.7 GB peak, about 70 MB free on the 16 GB machine (0c); an unexplained drop from 2.4 GB to 413 MB (0a) | Per-category memory; allocation counts |
| O11 | Tooling | open | `--profile` keeps only the first 4,096 frames after the warm-up (35-50 s of play) | A rolling window or a larger buffer for long runs |
| O12 | Measurement | open | The GPU read 90% busy before the game started (0c sampler); 24% before capture 1, 45% before capture 2 | Close other GPU users before captures; record idle GPU load |
| O13 | Render assets | open | `allocate_and_free_meshes` 2.3 ms per frame mean, p99 7.9, max 626 (1): meshes are created or freed every frame in play, not only at loads | Which systems add and drop mesh assets per frame (canvas, effects, decals) |
| O14 | Render assets | open | `prepare_erased_assets<SourceMaterial>` 0.97 ms per frame mean, p99 2.0, max 26 (1): materials are re-prepared every frame | Which materials change each frame (animated proxies); update uniforms instead of assets |
| O15 | Simulation | open | Fixed-step catch-up: after a slow frame the next runs two to four 15 ms ticks (npc_ai twice in 1,429 frames, the weapons stage four times in one); `FixedMain` 1.2 per frame, 3.25 ms each (1) | Slow frames cost more simulation: fixing render cost also cuts this |
| O16 | Physics | open | Rapier `update_layers_and_find_pairs` 0.74 ms, 1.9 times per frame (1) | Count bodies and colliders; static versus moving |
| O17 | Presentation | open | Main-thread presentation per frame (1): `present_entities` 1.02 ms, GPU shadow `draw` 0.90 (p99 2.6), HUD `present` 0.53, `eyes::present` 0.33, `effects::present` 0.29, `shadows::update` 0.28; Bevy's `check_entities_needing_specialization` runs every frame for four material types, about 0.6 ms together | Skip specialization checks without changes; cheaper presentation per entity |
| O19 | Stutter | done | Every 60 frames, in all play (not only `--report` or `--profile` runs), `monitor::monitor` rebuilt the full diagnostic report: mean 0.13 ms but p99 7.45 ms, max 11.8 ms (1). At 120 FPS that was a hitch every half second | Fixed 2026-11-21: the report is built on the first frame, captures, script end, the frame limit and quit; F1's map metadata is copied on a map change. Headless trace build, 600 frames at d1_trainstation_02: monitor p99 4.02 -> 0.020 ms (the 5 ms first-frame snapshot remains), `diagnostics` stage mean 0.068 -> 0.002 ms, max 4.68 -> 0.01 ms |
| O21 | CPU / memory | open | Heap churn: 161,344 allocations and 23.9 MB requested per frame on average, p50 148,066, p99 321,442 allocations (2); loading frames reach 3.7 million allocations and 3.1 GB | Attribute allocations to systems (per-span counts in the next capture); reuse buffers in the per-frame paths (poses, canvas, presentation) |
| O22 | Pacing | open | Visible pacing under capture 2's instrumentation (about 51 FPS): of 2,341 frames shown, 30% stayed one refresh, 23% two, 31% three, 15% four; 3,196 repeated refreshes. Uneven judder, not a steady lower rate | Capture 3: the normal build with only a Metal trace, for real pacing numbers |
| O23 | Memory | open | Swap grew from 884 to 1,170 MB during capture 2 (trace writing, Metal recording and the game on the 16 GB machine) | Keep captures short; check swap growth in play without recorders |
| O18 | NPC | open | NPC AI 0.23 ms per frame (p99 0.64, max 2.3) for 118-122 actors; by class citizen 0.044, metropolice 0.039, scanner 0.017 ms per frame; the costliest schedules are idle and alert standing, actbusy leave and scanner patrol; route planning 0.88 ms when it runs (1) | Not a bottleneck now; watch it with larger crowds |
