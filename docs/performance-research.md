# Performance research and implementation results

## Indexed bullet decals in a mixed crowd (2026-11-20)

The station02 mixed-crowd fight spends fixed-step time finding the surface under a shot
and clipping its mark. Static surfaces are batched by material across the map. Indexing
their triangles narrows the search while preserving the original triangle order for
receiver selection, UVs, and lightmap samples. Brush models and posed actors keep their
local-space search.

M1 Pro/Metal, release, 1920×1080, `profiles/test.json`, 65 NPCs. Three alternating pairs
compared the package at `a88dadd0` with the final index. Frame time is elapsed time divided
by frames in the last eight one-second windows. The fixed-step column is the profiler's
whole-run stage mean.

| Pair | Fight frame, original → indexed (ms) | Frame-time reduction | Fixed step, original → indexed (ms) |
| --- | ---: | ---: | ---: |
| 1 | 20.774 → 18.672 | 10.1% | 6.575 → 5.349 |
| 2 (reverse launch order) | 20.638 → 18.508 | 10.3% | 6.871 → 5.398 |
| 3 | 20.683 → 18.274 | 11.6% | 6.482 → 5.449 |

The medians were 48.3 → 54.0 FPS and 6.575 → 5.398 ms for fixed simulation. An earlier
three-pair run on the first index implementation measured 21.034 → 18.799 ms. All six
final runs reached the script endpoint with 319 world marks created, 256 active, 17
unclippable hits, 368 studio marks, observed GPU completions and zero reported pose or
GPU completion errors.

Station02 indexed 148,720 triangles. Median construction took 20.5 ms (20.2–21.0).
The triangle records, surface counts, tree nodes and proxies occupied 15.16 MiB of live
data, excluding spare allocation capacity and construction scratch. Gameplay preparation
rose from 251.9 to 277.3 ms. Total request-to-installed time was 1844.1 → 1854.3 ms.
Median peak process RSS was 5.87 → 5.79 GB across the full launches. That process-wide
measurement varies independently of the index's own storage. Map reports expose the
triangle count, payload size and build time under `decal_index`.

Reproduce with the same release package flags `--map d1_trainstation_02 --background
--no-setup --mute --width 1920 --height 1080 --uncapped --profile --movement-script
test-inputs/performance-research/mixed-crowd.json` and a separate report for each run.
Frozen captures of a pistol mark on the station door and an adjacent stone wall showed
the same visible placement and shape at 1080p.

## GPU shadow silhouettes become the default

2026-11-19, owner decision. Visual fidelity is judged as a player sees normal play at native
resolution. In full-screen 1080p frozen captures of 30 citizens, G-Men and dropships, the CPU
and GPU frames look the same. Only a pixel count finds the difference: at most 9 levels on 9
pixels for G-Men, 3 levels for dropships, none above 2 for citizens. That difference does not
justify leaving the measured 21–55% crowd frame-time gain (below) switched off. The GPU path
is used whenever a renderer exists; `HL2RS_GPU_SILHOUETTES=0` keeps the CPU rasterizer as the
reference and fallback. Headless runs without a render sub-app keep the CPU path.

## Mixed-NPC crowd benchmark (2026-11-20)

`test-inputs/performance-research/mixed-crowd.json` spawns 65 NPCs on station02's plaza with
`ent_create`, the camera above each cell like `plaza-crowd.json`. Combine: Metrocops with
stunsticks and pistols, soldiers, shotgunners, elites, a prison guard, manhacks, both scanners,
rollermines and a floor turret. Rebels and allies: four kinds of citizen, Barney, Alyx,
vortigaunts, Dog and the monk. Xen: four kinds of zombie and of headcrab, antlions, an antlion
guard and a stalker. Bystanders: Kleiner, Eli, Mossman, Breen, the G-Man and birds. Armed
classes get their retail weapon (`additionalequipment`), and `notarget` keeps the player out of
the fight. The view holds from tick 141 to 1200. Antlions, the antlion guard, vortigaunts, Dog,
the monk, the stalker, the claw scanner, rollermines, the floor turret and the story characters
have no HL2RS AI yet: they cost drawing and animation only.

M1 Pro, 1920×1080, `profiles/test.json`, `--uncapped --profile`, muted. At the end 43 actors
fight another NPC. The last eight one-second windows average 23.9 ms (41.9 FPS), p95 ≤ 44 ms.
A 10 ms stack sample of 20 s of the fight:

| Work | ms per frame |
| --- | ---: |
| `fixed_step` | 11.0 |
| — Rapier step (broad phase `SAPLayer::update_regions` 1.7) | 3.4 |
| — shot impacts (`Impacts::add_along`, decal placement 1.6) | 1.8 |
| — AI think (line-of-sight rays 0.5) | 1.1 |
| — `Entity::class` lookups | 0.8 |
| `allocate_and_free_meshes` (render world) | 5.7 |
| `present_entities` | 3.2 |

Each frame about 190 effect-quad meshes, 27 eye meshes, 13 shadow meshes and 9 animated meshes
(52,000 vertices) change and are uploaded again. Since `effects::quads` draws alpha sprites as runs (consecutive
quads of one material share a mesh, in input order), the fight frame fell from 23.8 to 21.8 ms
(median of three A/B pairs). The audio output thread is busy about one core
mixing about 70 live sinks, muted or not. About one run in three hung at exit: closing the
CoreAudio output waited on the device lock while the mixing callback kept it busy. Runs now end
from their exit frame without dropping the app (`launch::exit_process`).

Across tiers (2026-11-20, revision `2d7d4b75`, 1920×1080, one run each; Low and Max as the
tier tool defines them, Default the settings' defaults; steady = last eight one-second windows):

| Scene | Low | Default | Max |
| --- | ---: | ---: | ---: |
| Quiet plaza | 4.77 ms (210 FPS) | 5.30 ms (189 FPS) | 5.42 ms (185 FPS) |
| Mixed crowd, fight | 18.32 ms (55 FPS) | 21.34 ms (47 FPS) | 22.10 ms (45 FPS) |

Every tier is CPU-bound on the M1 Pro: Max costs little over Default. In the Default crowd the
main thread sets the frame (fixed loop 11.1 ms, about 1.6 ticks of 6.9 ms; `PostUpdate` 9.2 ms:
animation 3.9, eyes 1.0, shadows 0.9, model lighting 0.8, the rest Bevy's transform propagation
and visibility); the render thread's 17 ms (render assets 7.9, render graph 5.7) run beside it.
NPC hitboxes no longer reach Rapier's broad phase (2026-11-20): they are trace colliders only,
yet its sort handled about 1,000 of them every tick and woke the bodies they overlapped. Three
alternating A/B pairs: fight frame 21.87 -> 20.67 ms, fixed simulation 7.11 -> 6.45 ms per tick
(medians). Open candidates: posing every NPC's hitboxes every tick is about 6% of the fixed step
(Source sets up bones only for entities a trace tests); a per-pair sight cache like Source's
90 ms one (`CBaseCombatCharacter::FVisible`) would save about 0.25 ms; the audio thread mixes
about one core in fights.

## Shared actor bone matrices: rejected

2026-11-19, revision `f2b0323f`, M1 Pro/Metal, release package, the 30-NPC station02 plaza
script at 1920×1080 with CPU silhouettes. A temporary probe timed `Scene::actor_matrices` at its
three render consumers (whole-run averages, one launch per class):

| Crowd | Animation stage / matrices (ms) | Eyes stage / matrices (ms) | Shadows stage / matrices (ms) |
| --- | ---: | ---: | ---: |
| 30 vortigaunts | 2.49 / 0.25 | 1.81 / 0.29 | 1.80 / 0.15 |
| 30 G-Men | 1.47 / 0.24 | 1.78 / 0.29 | 4.27 / 0.14 |
| 30 citizens | 2.51 / 0.30 | 0.50 / 0.30 | 2.21 / 0.18 |

Sharing the skinning pass's matrices with eyes and shadows could save at most the eyes and
shadows matrix time, 0.43–0.48 ms per frame against 18–21 ms frames. It would also make eyes and
shadows follow the drawn mesh's shared pose bucket instead of each actor's exact time. Not
implemented. The stage cost lies elsewhere: eye work other than matrices (about 1.5 ms for
vortigaunts and G-Men) and silhouette skinning and rasterization (see GPU silhouettes below).

## Thirty-NPC plaza shadows after the station merge

2026-11-19, revision `51646bbc`, M1 Pro/Metal, release package, 1920×1080, uncapped, High shadows.
Three alternating pairs compared default CPU silhouettes with `HL2RS_GPU_SILHOUETTES=1` on the
same executable. Each launch ran `d1_trainstation_02` to host tick 660. After warm-up it spawned
30 actors of one class in three rows and viewed them from (-2020, -2680, 158), yaw 90, pitch -14.
The control followed the same camera script with zero added actors. Each run's frame time is
elapsed wall time divided by frames over its last three complete one-second windows. Pairs
ran CPU/GPU, GPU/CPU, CPU/GPU. Paused captures were excluded from timing.

| Added NPCs | Whole frame, CPU → GPU (ms, median) | FPS, CPU → GPU | Frame-time change in each pair | CPU shadow stage, CPU → GPU (ms) | p95 frame upper bound, CPU → GPU (ms) |
| --- | ---: | ---: | --- | ---: | ---: |
| 0 | 6.88 → 6.75 | 145.4 → 148.2 | −5.0%, −1.9%, +1.3% | 0.47 → 0.15 | 9 → 9 |
| 30 citizens | 18.31 → 14.31 | 54.6 → 69.9 | −22.1%, −23.0%, −21.3% | 2.06 → 0.56 | 25 → 21 |
| 30 G-Men | 21.26 → 13.64 | 47.0 → 73.3 | −36.1%, −37.6%, −32.3% | 3.99 → 1.10 | 25 → 17 |
| 30 Combine dropships | 24.00 → 11.21 | 41.7 → 89.2 | −55.5%, −53.1%, −54.1% | 6.56 → 2.75 | 27 → 14 |

Subtracting the control medians, the extra frame time for 30 actors fell from 11.43 to
7.56 ms for citizens, 14.38 to 6.89 ms for G-Men, and 17.13 to 4.46 ms for dropships.
The control varied in both directions. Median GPU busy rose from 32% to 53% for citizens,
29% to 58% for G-Men and 27% to 66% for dropships. The game footprint was 6.4–6.8 GB in
those crowds. The aligned samples recorded zero competing builds or other game processes.
All timed runs observed GPU completions, at most two pending fences, zero completion or
reported pose errors, and the expected number of successful spawns. Final visible triangle
counts matched in every pair. AI, animations, spawn records, goals, health, projectiles,
inventory, choreography and entity I/O matched. Prop transforms, head or random look
states and path profiling counters could differ between launches.

Each crowd also supplied frozen High CPU, High GPU, repeat High CPU and Low CPU captures.
The comparison selected pixels where the two High CPU captures agreed within two RGB
levels and High versus Low differed by more than eight on at least one channel.

| Crowd | Selected pixels | CPU/GPU differences above 2 levels | Largest channel difference |
| --- | ---: | ---: | ---: |
| Citizens | 21,632 | 0 | 2 |
| G-Men | 18,431 | 9 | 9 |
| Dropships | 132,851 | 6 | 3 |

A direct Metal atlas readback found one differing pixel at a moving opaque silhouette edge and reproduced the faint-layer
precision difference (16 overlapping contributions gave CPU 4, GPU 0). CPU silhouettes remained
the default until 2026-11-19, when the GPU path became the default (see below). The results cover this camera, hardware and three crowd types. The citizen
crowd still costs 7.56 ms over the control with GPU silhouettes. With that path enabled,
the citizen fixed-simulation stage averaged 4.12 ms versus 2.36 ms in the control,
and animation averaged 2.14 versus 0.72 ms. For G-Men, eye updates averaged 2.04
versus 0.25 ms. These stage means cover the profiled run, including the pre-spawn
period.

## Station plaza ground crowd

2026-11-18. The ground workload places 30 zombies on the same station02 grid as the
30-manhack benchmark. Use the packaged flags below and the movement script
`test-inputs/performance-research/plaza-ground-crowd.json`. Each of 36 runs reached
tick 600, with the expected spawn count, 1920×1080 HUD, zero GPU completion errors
and zero pose mismatches. A weapon dropped by a map NPC also appears in the ground
spawn log. It is excluded from the count.

M1 Pro, Metal, CPU shadow silhouettes, uncapped and muted. Baseline `8208887c` and
candidate ran in alternating pairs. Each cell is the median of three runs' means in
milliseconds. The repeat's one-minute load averages are in the per-run records.

| Scene | First series, before / after frame | Repeat, before / after frame | Repeat, before / after fixed simulation | Repeat, before / after frame p95 upper bound |
| --- | ---: | ---: | ---: | ---: |
| Quiet | 8.364 / 7.823 | 5.448 / 5.231 | 2.567 / 2.457 | 8 / 8 |
| 30 zombies | 13.527 / 13.332 | 11.238 / 10.928 | 4.052 / 4.013 | 15 / 16 |
| 30 manhacks | 13.680 / 13.427 | 9.436 / 9.340 | 3.170 / 3.191 | 14 / 14 |

Ground mean frame time fell in five of six pairs. Two repeat pairs differed by only
0.04 ms. Host load and the simulated fight varied across runs, limiting attribution.
All 30 manhacks moved over 32 units; typically 15–16 zombies did. Ground actors at the
sample included both melee attackers and actors in failed chase or wander schedules.
This workload measures combat in a crowd, not every actor's ability to reach a goal.

## GPU shadow silhouettes on station02

2026-11-15. Three alternating pairs on the final packaged build reduced the median crowd
frame from **9.575 to 8.693 ms** (104.4 to 115.0 FPS). Quiet remained **5.33 ms**. This is
an opt-in experiment, `HL2RS_GPU_SILHOUETTES=1`. A measured difference on very faint
overlapping surfaces keeps the CPU renderer as the default.

The earlier experiment which omitted coverage altogether measured 9.69 to 9.04 ms in a
different run. Here the candidate skins and projects the same caster surfaces, clears dirty
cells and draws two-sided triangles with additive texture-alpha coverage into the sampled
R8 page before the cameras run. It also avoids replacing the CPU-backed page and refreshing
the material after each batch. The geometry and index upload buffers persist across frames.
Only 35 silhouettes were requested in an entire quiet run. On the crowd workload animated
casters needed roughly 17,000–19,000 redraws over 600 ticks.

M1 Pro, Metal, 16 GiB unified memory, release packages, 1920×1080 uncapped, High shadows,
muted background rendering with observed GPU completion and at most two outstanding frames.
Each row is an independent pair, A is the production CPU renderer and B the GPU prototype.
Times are post-warm-up frame means. Tail values are 1 ms histogram upper bounds.

| Scene / pair | Order | A / B frame (ms) | B change | A / B p95 (ms) | A / B p99 (ms) |
| --- | --- | --- | ---: | --- | --- |
| Quiet 1 | AB | 5.325 / 5.349 | +0.46% | 8 / 8 | 9 / 12 |
| Quiet 2 | BA | 5.343 / 5.302 | −0.77% | 8 / 8 | 12 / 10 |
| Quiet 3 | AB | 5.327 / 5.328 | +0.03% | 8 / 8 | 10 / 9 |
| 30 manhacks 1 | AB | 9.614 / 8.649 | −10.03% | 14 / 13 | 19 / 15 |
| 30 manhacks 2 | BA | 9.524 / 8.693 | −8.72% | 14 / 13 | 18 / 17 |
| 30 manhacks 3 | AB | 9.575 / 8.752 | −8.60% | 14 / 13 | 19 / 17 |

Crowd CPU `shadows` stage means fell from 1.442–1.450 to 0.434–0.477 ms.
The median fixed tick moved from 3.22 to 3.20 ms. The GPU submitted 19,023 / 18,876 /
18,837 cells in the three crowd runs. At report sampling each had 16 more main-world
requests than submissions. Quiet submitted 35/35 in each. The matched pair digests of
AI, inventory, projectiles and the workload validator's player/outcomes agreed. All 30
spawned manhacks hunted the player. All twelve runs finished tick 600 with zero GPU
completion errors and pose mismatches. Verification allowed only the exact previously
recorded station02 graffiti decal with no receiver. Preliminary runs on the first
prototype also favoured the crowd path in all three pairs (median 9.338 to 8.690 ms).

Load averages spanned 3.7–5.3 on ten logical CPUs. The first crowd baseline had a competing
build flag; both other crowd pairs improved by over 8%. Across crowd runs, system GPU busy
rose from 55–57% to 63–64% and process CPU from 235–237% to 243–245% of one core. Peak
sampled game RSS ranged from 1.78–2.21 GiB on A and 2.03–2.24 GiB on B. Swap was already
in use; the sampled swap-out counter stayed flat in the final runs. These are system GPU
occupancy samples, with no per-pass GPU timings or separate VRAM measurement.

**Image evidence.** Two fixed 1280×720 views placed a Metrocop on the plaza, then 48 units
to the side. The initial indoor view faced a wall and was discarded. In the outdoor views
the repeat CPU captures had zero pixels differing by more than two RGB levels. CPU High
versus Low changed 5,909 / 4,484 pixels by that threshold. Within a fixed rectangle over
the receiver, 2,989 / 1,939 pixels responded to the control by more than eight levels.
CPU and GPU differed by at most one level at all those pixels. Over each whole frame,
11 / 17 of 921,600 pixels differed by more than two. Direct atlas readback independently
found one covered edge sample 0.001287 texel from the CPU triangle outline. Unmodified
neighbouring cells were byte-identical through every update.

**Decision.** The fixed-view accuracy and crowd gain justify retaining the experiment.
Rendering directly into R8 rounds every alpha contribution. Sixteen layers each contributing
one quarter of an R8 level measured 0 on Metal versus 4 after the CPU's final quantization.
Before changing the default, test higher-precision accumulation on translucent cells with
that fixture and remeasure the crowd. The reported Windows/native gap remains unmeasured by
this experiment. To reproduce the workloads use the packaged arguments above with
`--profile --map d1_trainstation_02 --width 1920 --height 1080 --frames 12000`,
`--movement-script test-inputs/performance-research/{quiet,plaza-crowd}.json` (one fixture
per launch) and a separate `--report` per run. Settings and fixtures must match within each
pair. The installed-game camera procedure is in [validation](validation.md#gpu-shadow-silhouette-experiment-2026-11-15).

Checkpoint: 2026-11-11. Two presentation-side CPU optimizations are implemented and tested,
but the 30-run paired matrix establishes no end-to-end FPS gain. Helper cost falls in isolated
experiments; quiet/active/crowd frame medians remain within variation and 1080p crowd tails worsen.
The following result supersedes the earlier research-only disposition. The 18-run sensitivity
study below remains preliminary context and is not an implementation A/B result.

## Second-pass diagnosis: where the frame goes, and the next cycle (2026-11-11)

This section supersedes the ranking further down. It re-reads the retained 30-run reports and
1 ms stack samples and adds two 10 ms stack-sampled runs of the same candidate package (quiet
and 30-manhack plaza, 2400 host ticks, M1 Pro / Metal, 1080p). Sampled runs are attribution
only: both samplers suspend the game for roughly a third of wall time, so their frame times
are not throughput figures. No runtime code changed in this pass.

### Why the first cycle gained nothing

1. **The targets were chosen by analogy, not from a budget.** The web survey suggested sprite
   batching and repeated-lookup removal; nobody first asked how many milliseconds those paths own
   here. In the retained crowd samples the two optimized helpers hold 10 and 2 of 3,454 busy
   observations. At most about 0.05 ms of a 13.4 ms frame was ever available to them.
2. **A microbenchmark ratio is not a frame budget.** 28x on a 0.24 microsecond helper called a
   few hundred times a frame cannot exceed 0.1 ms. The ratio was reported before the call count.
3. **The paired test could not see the predicted gain.** Run-to-run spread was about 1 to 3%
   and the predicted saving under 0.5%, so the 30 runs could only return "no difference".
4. **No instrument covered the whole frame.** The CPU scopes overlap, average over loading
   frames (1,278 scope calls for 648 gameplay frames in the crowd run, so per-frame means read
   about half their gameplay value) and leave part of the main schedule unmeasured (below).
5. **The 1 ms sampler distorted what it measured.** Suspended wall time still accumulates
   fixed ticks, so the simulation's share was inflated and counts could not be scaled to ms.
6. **The baseline was stale.** Upstream had already removed repeated flight sweeps and sight
   rays (`4805c93`, `7f0e9ff`), the largest item this profile still shows.

### What bounds the frame

- The main world bounds it, not the renderer or the GPU. In the samples the main thread is
  inside the main schedule in 2,080 of 2,129 snapshots, the pipelined render thread is parked
  in more than half of its snapshots, and 3.6 times the pixels changes the crowd frame by under 2%.
- The work is serial. Four compute workers are each about 30% busy and their busy time sums
  to about one core: one system at a time hops between workers. More threads will not help.
- **The fixed step is amplified by the frame time.** One 15 ms tick costs `S` ms and runs
  `T / 15` times per frame, so `T = N + S * T / 15`, or `T = N / (1 - S / 15)`, with `N` the
  rest of the main schedule (valid up to the 50 ms clamp, three ticks per frame).

| Case | Frame `T` | Tick `S` | Fixed per frame | `N` (rest of main schedule) |
| --- | ---: | ---: | ---: | ---: |
| M1 Pro quiet (measured) | 5.6 ms | 3.86 ms | 1.4 ms | 4.2 ms |
| M1 Pro 30 manhacks (measured) | 13.4 ms | 5.64 ms | 5.0 ms | 8.4 ms |
| Windows plaza, empty (retained profile) | 13.6 ms | 5.3 ms | 4.8 ms | 8.8 ms |
| Windows plaza, 30 manhacks (retained profile) | 42.1 ms | 10.5 ms | 29.5 ms | 12.6 ms |

`N` is derived from the model, not timed. On the Windows crowd every millisecond removed from
a tick returns about 2.8 ms of frame, and every millisecond removed from `N` about 3.3 ms.
A free simulation would still leave 8.8 to 12.6 ms (80 to 113 FPS) there: the reported 276 FPS
needs `N` cut about threefold as well, not simulation work alone.

### Fixed step: exclusive shares (10 ms samples, busy observations)

| Path | 30 manhacks (914) | Quiet (633) |
| --- | ---: | ---: |
| NPC hull traces through the linear `trace_brushes` scan (flight carry and sweep, manhack steer, `tick_npcs`) | about 48% | about 19% |
| Rapier step: SAP `update_regions`, query-tree rebuild, narrow phase | 25% | 41% |
| `pose_npc_hitboxes` (`actor_matrices`) | 3.4% | 3.8% |
| NPC think including sight rays | 3.6% | 1.4% |
| `Scene::tick` (`Entity::class` lookups, attachments, filters) | about 5% | about 7% |
| Ground-surface decal projection (`Impacts::locate`) | 1.5% | 2.7% |

Scaled by the measured tick cost this is roughly 2.4 ms per crowd frame in brush traces and
1.25 ms in Rapier; with four awake bodies and no contacts, Rapier's share is tree and
broad-phase maintenance over all colliders, not solving.

### Rest of the main schedule

Of the crowd's roughly 8.4 ms, the scoped presentation systems hold about 4 ms once loading
frames are excluded: `present_entities` about 1.6 ms (60% of its samples are CPU facial flexes,
`apply_facial_flexes` and `vertex_deltas`; 13% pose composition), the shadow update about 1.6 ms
(two thirds in CPU silhouette rasterization, `silhouette::draw`; 12,457 silhouettes in the run),
eyes about 0.4 ms and model lighting about 0.3 ms. `actor_matrices` is composed again in
hitboxes, animation, eyes, shadows and effects for the same actors.

**About 4 ms per crowd frame (about 3 ms quiet) is in no scope and is not busy system time
in the samples.** Candidates are executor hand-off between workers, Bevy's own schedules and
extraction; this is the largest unexplained block and is **not measured**.

On the render thread the sampled leaders are `allocate_and_free_meshes` (7 to 8% of wall
while sampled) and `prepare_erased_assets` (5%): meshes re-uploaded each frame, consistent
with CPU flexes and per-frame silhouette and effect meshes. It is not the bound today.

### After merging upstream: measured, and the ranking holds

Upstream's single flight sweep and filtered sight ray are merged (`6c64694`). Three interleaved
pairs per case on the M1 Pro, 600 host ticks, same settings and package layout as the first
cycle; before is the first cycle's candidate package, after the merged release build. Tick cost
drops its three slowest calls per run.

| Case | Tick before | Tick after | Frame before | Frame after |
| --- | ---: | ---: | ---: | ---: |
| 30 manhacks | 5.59 ms (5.58 to 5.60) | 5.03 ms (5.01 to 5.03) | 13.62 ms (13.55 to 13.75) | 12.58 ms (12.45 to 12.83) |
| Quiet | 3.38 ms (3.27 to 3.42) | 3.34 ms (3.34 to 4.06) | 5.99 ms (5.75 to 6.12) | 5.75 ms (5.64 to 6.76) |

The crowd tick is 10% cheaper and its frame 7.6% shorter, with no overlap between the trial
ranges; quiet is unchanged within its spread (one after-run had a 711 ms load hitch). The frame
model predicted 12.85 ms from the new tick cost against 12.58 ms measured. This is the first
frame-time gain of the work, and it came from the profile's top item, not from the survey.

Machine state was recorded before every run: load average 4.3 to 8.8 on 10 logical CPUs and
18 to 195% of one core used by the five busiest other processes, after the owner closed the
heavy applications. Not idle; the pairs are interleaved and the crowd ranges are tight, so the
difference stands, but the absolute values belong to that load.

A 10 ms sampled crowd run of the merged build (attribution only) gives the same order:
hull traces through the linear brush scan 42% of the fixed step (310 of 733 busy observations,
from 48%), Rapier step 27%, `tick_npcs` hull traces within the first figure, hitbox posing 5%.
The ranked list below is unchanged, with item 1 done.

### The whole frame, timed: it closes, and the earlier stage means were diluted

`--profile` now records `frame_total` and one `main_schedule.<name>` stage per main schedule
(marker schedules between them, installed only with `--profile`); `main_schedule.outside` is
the time between the last main schedule and the next frame's first one: extraction, the wait
on the render thread and the window event loop. The stages add up to the frame. Two
interleaved pairs of the same package with and without the markers: crowd 12.58 against
12.61 ms, quiet 5.53 against 5.53 ms, so the markers cost nothing measurable.

The first timed runs exposed a measurement fault: every CPU stage mean covered the whole
process, including about 600 menu and loading frames and the menu background map's cheap
ticks, while the frame statistics start after the map's 120 warm-up frames. Stage samples now
restart when that warm-up ends, so both cover the same frames. Earlier stage figures in this
document are therefore too low for per-frame stages (shadows 0.82 against 1.56 ms, animation
0.79 against 1.50 ms) and for the tick (5.03 against 5.91 ms post-merge crowd). Comparisons
between builds made with the same fault (the merge result above) keep their direction; their
absolute tick values do not stand. The "4 ms in no scope" was mostly this dilution.

Four runs per case of the merged build with the markers, 600 host ticks, M1 Pro / Metal,
1080p, medians of per-run means; load average 4.4 to 7.3 on 10 logical CPUs, recorded per run:

| Stage, ms per frame | Quiet | 30 manhacks |
| --- | ---: | ---: |
| Frame (`frame_total`; overlay 5.53 and 12.45) | 5.54 | 12.35 |
| `PostUpdate` | 2.10 | 5.17 |
| of it: shadow silhouettes | 0.03 | 1.56 |
| of it: entity presentation (animation, flexes) | 0.65 | 1.50 |
| of it: eyes, model lighting, PVS, HUD | 0.28 | 0.72 |
| of it: in no scope | 1.14 | 1.39 |
| `RunFixedMainLoop` (tick 3.85 and 5.91 ms, 0.37 and 0.83 per frame) | 1.47 | 4.96 |
| `PreUpdate` (in no scope) | 0.33 | 0.92 |
| Outside the main schedule | 1.35 | 0.97 |
| `Last`, `First`, `Update`, `SpawnScene` | 0.27 | 0.31 |
| Render thread, sum of its fenced stages (parallel) | 3.6 | 7.1 |

- Crowd: the main schedule is 11.4 of 12.35 ms and waits under 1 ms for the renderer, whose
  fenced stages fill 58% of the frame. The serial main world bounds the frame, now by direct
  measurement. `PostUpdate` is as large as the fixed loop, which the diluted means hid.
- Quiet: the main schedule is 4.2 of 5.54 ms and spends 1.35 ms outside it (extraction and
  the wait on the renderer, whose fenced stages fill 65% of the frame). The main world still
  bounds this scene, with the renderer the nearer second bound.
- Frame model with the corrected tick: crowd `N` = 12.35 - 4.96 = 7.39 ms, `T` = 7.39 /
  (1 - 5.91/15) = 12.19 ms against 12.35 ms measured. A millisecond off any per-frame stage
  returns about 1.65 ms of crowd frame, because fewer ticks then fall in each frame.
- Remaining unnamed: 1.39 ms in `PostUpdate` and 0.92 ms in `PreUpdate` per crowd frame are
  bounded by schedule but not by system. **Not measured** per system.

### Exact world-brush candidate rejection: implementation and correctness

The next implementation starts from a fresh upstream merge, `a49ce84` (upstream main
`9561759`), rather than the older timed package above. Its baseline includes the newer lighting,
viewmodel and manhack rendering fixes. `BrushSet` builds a static median-split bounds tree once
from each world's brush list. Player and NPC hull traces gather nearby candidates into a
128-entry stack buffer, sort their original indices, and use the original plane accumulator.
Equal-hit normals and the normal retained before an all-solid result keep their list order.
There is no per-query allocation, no cadence reduction and no change to the hull or contents mask.

Only a brush's own axial planes justify rejection. Missing sides remain unbounded; a brush
without a complete box is checked individually. Non-finite data, unordered hull bounds,
coordinates/extents beyond magnitude 65,536, or candidate/stack overflow use conservative
fallbacks. The coordinate bound keeps axial arithmetic rounding below the 0.125-unit padding.
The tree does not depend on converted Rapier shapes, so missing conversion cannot remove collision.

Six synthetic tests cover 24,000 generated sweeps, face/epsilon boundaries, partial boxes,
overflow, contents masks, equal-hit normal ownership, all-solid accumulation, coordinate limits,
non-finite inputs and malformed planes. An owned-install test in `source-assets`, using normal
installation discovery, compares another 18,000 sweeps bit for bit with the full scan:

| Map | Brushes | Incomplete boxes | Mean candidates, all 6,000 sweeps | Full-scan fallbacks | Mean candidates, 4,000 short sweeps | Short fallbacks |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| d1_trainstation_02 | 1,913 | 0 | 47.41 | 137 | 2.11 | 0 |
| d1_canals_01 | 2,039 | 0 | 60.88 | 165 | 2.68 | 0 |
| d2_coast_01 | 536 | 0 | 9.47 | 45 | 3.74 | 0 |

Candidate means count every brush when the buffer falls back. The broad workload includes
deliberate 32,000-unit sweeps; short sweeps use component displacement scales up to 30 units.
These are generated correctness probes, **not live-game candidate counts or FPS gains**.
All three maps exercise hundreds of hits and start-solid cases. The existing source whitelist
admits the new Rust modules without exceptions. Native layout/assets remain read-only.

### Exact world-brush candidate rejection: measured

Three interleaved pairs per case of two packages from the same merge (`a49ce84`), without and
with `BrushSet`; 600 host ticks, M1 Pro / Metal, 1080p, all 12 runs exit 0 at tick 600. Load
average 3.6 to 4.4 on 10 logical CPUs, recorded before every run. Medians, with ranges:

| Case | Tick before | Tick after | Frame before | Frame after |
| --- | ---: | ---: | ---: | ---: |
| 30 manhacks | 6.03 ms (6.00 to 6.05) | 3.84 ms (3.84 to 3.85) | 13.85 ms (13.70 to 14.02) | 11.18 ms (11.17 to 11.26) |
| Quiet | 3.89 ms (3.86 to 3.94) | 3.32 ms (3.32 to 3.34) | 6.04 ms (6.04 to 6.06) | 5.79 ms (5.76 to 5.81) |

No range overlaps. The crowd tick is 36% cheaper and its frame 19% shorter (72 to 89 frames per
second); the quiet tick is 15% cheaper and its frame 4% shorter. Per crowd frame the fixed loop
falls from 5.65 to 2.94 ms, at 0.93 and then 0.75 ticks per frame; the 2.7 ms saved is above
the 2.1 ms ceiling, which was a sampled share of a slightly cheaper tick. Tick p95 falls from
7.26 to 4.38 ms in the crowd. `PostUpdate` (5.51 ms) is now half of the 11.1 ms crowd frame.

Packaged replays of the candidate against the merged baseline: 107 of 107 checks pass (1,839 s). The merged baseline had 105 of 107: its
`flashlight_viewmodel` off-light image control and `monitors-kleiner-scene` gib count did
not fail again on the candidate. The index returns the full scan's result bit for bit, so
those two are treated as unstable checks, not as something this change fixed.

### Facial flex deltas once per actor: measured; silhouettes already redraw only on change

Both presentation items in the ranked list assumed work was repeated while nothing changed. The
code already skips it: a silhouette is redrawn only when its caster moved, animates or got a new
page fragment, and a face is redeformed only when its quantized weights changed. Their cost is
paid by casters and faces that are changing, so each was looked at by call path (10 ms sample of
the crowd, attribution only).

Facial flexes were about 73% of entity presentation. Two wastes inside a changing face: the
vertex deltas of the whole face model were rebuilt for every surface of the actor in the frame,
and every vertex key was hashed with the default keyed hash, both when building and when
reading. Now the frame's per-actor entry holds the weights, their signature and the deltas,
built at most once, and the delta map uses a multiplicative hash of the key. Accumulation
order and values are unchanged; a unit test compares the map with a default-hashed one bit for
bit. Three interleaved pairs against the brush-set build, 600 ticks, M1 Pro / Metal, 1080p, load
average 4.4 to 12.4 recorded per run (the first pair followed a build):

| Case | `animation` stage before | after | Frame before | Frame after |
| --- | ---: | ---: | ---: | ---: |
| 30 manhacks | 1.46 ms | 1.02 ms | 11.36 ms (11.25 to 11.40) | 11.04 ms (11.02 to 11.14) |
| Quiet | 0.72 ms | 0.55 ms | 5.87 ms (5.86 to 5.91) | 5.86 ms (5.83 to 5.87) |

The crowd frame is 2.8% shorter with no range overlap. The quiet stage is 0.17 ms cheaper but
its frame does not move: `PostUpdate` falls from 2.41 to 2.27 ms and the time outside the main
schedule rises by about as much, the renderer being the nearer bound there. The tick reads 0.03
ms higher in both cases (3.80 to 3.83, 3.33 to 3.37 ms); the change does not touch the fixed
step, so this is taken as build-to-build variation, **not investigated**. Packaged replays were
**not rerun** for this step.

Shadow silhouettes (1.57 ms per crowd frame): 71% of the shadow system's samples are the CPU
triangle rasterizer and its caller, 8% posing and skinning the silhouette mesh, 7% decal
clipping. Thirty manhacks move and animate every frame, so thirty silhouettes are legitimately
redrawn, and each redraw also re-uploads the 1024 by 1024 coverage page. No repeated work was
found to remove. What remains is a design choice: draw silhouettes on the GPU into the page, as
the original engine does, or keep the CPU rasterizer and tune its per-triangle setup for a
fraction of the 1.1 ms. **Not started; owner decision.**

### Rapier maintenance: both costs are fixed per step, and neither is reachable by a small change

Read, not changed (rapier3d 0.26.1, parry3d 0.21.1) and attributed by call path in the 10 ms
samples. Inside `PhysicsPipeline::step`, with about four bodies awake:

| Per tick, M1 Pro | Share of the fixed step when sampled | About |
| --- | ---: | ---: |
| Broad phase, `SAPLayer::update_regions` | 13% crowd, 17% quiet | 0.65 to 0.75 ms |
| Query tree update inside the step | 10% crowd, 15% quiet | 0.6 ms |
| `refresh_entity_queries` (our own update of the same tree) | 2 to 3% | 0.1 ms |

Together about 1.3 ms of a 3.3 ms quiet tick and of a 3.8 ms crowd tick: roughly 0.5 ms per
quiet frame and 1.0 ms per crowd frame here, more where frames hold more ticks.

- Broad phase: every update of a region marks all of its subregions to be updated, so the
  sweep runs over every proxy on every step whether or not anything moved. The cost follows the
  collider count (5,659 on this map), not motion.
- Query tree: the step ends with `update_incremental(.., refit_and_rebalance = true)`, and
  `Qbvh::rebalance` takes apart and rebuilds every node above depth 5 on each call, changed or
  not (up to 1,024 subtrees re-split). Our own `refresh_entity_queries` pays the same again when
  an entity moved. The public call ties refitting to rebalancing; the tree itself is private to
  `QueryPipeline`, so "refit every tick, rebalance every N ticks" (plan 14s, step 2) cannot be
  done through it.

What would remove them, each a design choice rather than a tweak:

1. Own the query tree: hold parry's `Qbvh` directly, refit every tick, rebalance every N ticks
   or after large changes, and provide the handful of queries the game uses over it. Removes
   most of the 0.7 ms per tick. Query contents stay the same; equal-distance ties may resolve in
   a different order, which the replays must show harmless.
2. Own the broad phase through Rapier's public `BroadPhase` trait, testing only colliders of
   awake bodies and colliders that changed against a tree. Removes most of the other 0.7 ms.
   It changes the order in which contact pairs are created, which can change solver results
   slightly; the replays decide.

Neither is started. **Owner decision.**

### Correction: most of the Rapier maintenance was caused by our own collider access

The section above calls both costs fixed inside the physics library. A probe of what each step
is told shows otherwise. `ColliderSet::get_mut` marks a collider modified, and the step treats a
modified collider that is disabled as removed. `set_entity` and `set_npc_hitboxes` took that
mutable reference before checking whether pose or enabled state differed, so on
d1_trainstation_02 every step was told of about 2,175 modified colliders, 1,855 of them
disabled: removed from and re-added to the broad phase and the query tree on every tick. With
the reference taken only for a real change the crowd run reports about 334 modified and 6
removed per step. Those six were disabled NPC hulls whose entities move while hidden:
`set_entity` repositioned them every tick. A collider that stays disabled is now left where
it is and placed when it is enabled (the same for an entity's other colliders).

Checkpoint, **not measured**: that fix, and the query tree owned by the game (DESIGN plan E:
the same tree type, refit on every update, rebalance on insert, removal or after 64 changing
refits; the step gets no query pipeline and a recording wrapper around the unchanged broad
phase passes on the colliders it reports). Unit tests compare all six queries with the
library's pipeline through 300 rounds of moves, inserts and removals. Two things depend on the
tree's shape and are stated in the tests: a shape cast that starts overlapping several
colliders reports the first one visited, and the box query answers from stored boxes that may
be looser than the collider (its callers test each candidate exactly). The one packaged pair
taken before the access fix showed the owned tree alone at about 0.13 ms off the tick.

### Unchanged colliders not reported, query tree owned: measured

Three interleaved pairs on the same pre-merge base (the flex build against the checkpoint
commit), 600 ticks, M1 Pro / Metal, 1080p, all runs exit 0 at tick 600. The machine was busy:
load average 6.8 to 15.3 on 10 logical CPUs, recorded per run.

| Case | Tick before | Tick after | Fixed loop per frame | Frame before | Frame after |
| --- | ---: | ---: | ---: | ---: | ---: |
| Quiet | 3.34 ms (3.28 to 3.39) | 2.48 ms (2.44 to 2.51) | 1.30 to 0.97 ms | 5.56 ms (5.56 to 5.91) | 5.45 ms (5.11 to 5.54) |
| 30 manhacks | 3.80 ms (3.79 to 3.83) | 3.20 ms (3.19 to 3.52) | 2.80 to 2.34 ms | 10.68 ms (10.66 to 10.85) | 10.64 ms (10.24 to 11.28) |

The tick is 26% cheaper in the quiet scene and 16% cheaper in the crowd, with no overlap
between the tick ranges. Frame time is **not established** as improved at this load: the quiet
ranges barely separate and the crowd ranges overlap (one after-run at load 10.9 took 11.28 ms).
The saving sits in the fixed loop, which is now 18 to 22% of the frame; presentation
(`PostUpdate`, about 5 ms in the crowd) is the larger part. Where frames hold more ticks, as on
the retained Windows crowd profile, the tick saving counts more than once per frame. The
timing was not repeated on the merged tree.

Packaged replays on the merged tree: 106 of 107 pass. The one failure is
`flashlight_viewmodel`'s off-light image control, which also failed on an earlier merged
baseline without these changes and passed on the run after it: an unstable check, **not
investigated**. A first suite run that showed 70 failures is discarded: it had packaged a
stale pre-merge binary, because a temporary second checkout had shared the build directory.

### Presentation after the merge: no single unscoped system; mesh rewrites load the render thread

A 10 ms sample of the merged build in the crowd (attribution only; load average about 1.5).
The `PostUpdate` time in no scope is spread over several small systems, none above about
0.3 ms per frame: the effect, eye, visibility, monitor and material-proxy `present` systems
together about 0.9 ms (effects 0.3, of which half is posing actors for muzzle attachments),
visibility and bounds about 0.4 ms. Nothing there repeats unchanged work.

The larger cost is on the render thread: `allocate_and_free_meshes` about 2.7 ms per frame and
material preparation about 1.7 ms (bind groups rebuilt for materials whose lighting changed).
The time the main schedule spends outside itself has grown with every main-thread saving
(0.97, then 1.36, now about 1.7 ms per crowd frame): the render thread is becoming the bound.

A counter at each mesh write, per frame in the crowd: effect sprite quads 115 to 180, shadow
decals about 23, eye meshes about 13, face meshes about 8. Every one is a real change (each
site compares before writing); each is one mesh asset rewritten and uploaded again. The sprite
quads are one mesh and one draw per quad by design, to keep alpha ordering and per-view
visibility, and a quad that faces the camera changes every frame.

Ceiling, a discarded experiment (sprite quads not rewritten, so drawn wrongly): two
interleaved pairs, 600 ticks, M1 Pro / Metal, 1080p, load average 18 to 24 (very busy, recorded):
frame 10.84 ms (10.73, 10.96) to 9.86 ms (9.85, 9.87); the render thread's asset stage 2.52 to
1.71 ms. So at most about 1 ms, 9% of the crowd frame, is available from the sprite quads.

Reaching it is a design choice, **not started**: draw the quads of one material as one mesh
(one upload per material instead of one per quad). Additive sprites do not depend on draw
order among themselves; alpha-blended ones do, and a merged mesh is sorted as one object
against other translucent surfaces, so images can change where sprites overlap them. The eye
meshes (the iris projection is written into a vertex attribute of the head mesh) and the
per-entity lighting carried in materials are the same kind of cost and the same kind of choice.

Also found: the report check used by the paired timing driver
(`scripts/checks/check_effect_pool_performance.py`) rejects every run of the merged tree,
because the map now reports one infodecal without a surface as a texture error. **Not fixed.**

### Additive sprite quads as one mesh per material: measured

DESIGN plan F. Quads whose material blends by pure addition share one mesh per material in the
world layer; every other quad, and the viewmodel's, keeps its own draw. Three interleaved
pairs against the merged build, 600 ticks, M1 Pro / Metal, 1080p, load average 4.1 to 9.5
recorded per run:

| Case | Frame before | Frame after | Render asset stage |
| --- | ---: | ---: | ---: |
| 30 manhacks | 10.65 ms (10.63 to 10.92) | 9.84 ms (9.67 to 9.96) | 2.51 to 1.77 ms |
| Quiet | 5.42 ms (5.38 to 5.52) | 5.42 ms (5.41 to 5.43) | 0.53 to 0.54 ms |

The crowd frame is 7.6% shorter with no range overlap, which is the whole ceiling the discarded
experiment gave (9.86 ms); the quiet scene has few sprites and does not move. Time outside the
main schedule falls from 1.65 to 1.36 ms. Packaged replays: 106 of 107 on the first run; the
failure was `effect-ball`, whose check reads `pooled_quads` as the most quads drawn at once,
a meaning the first version of the change had dropped. The counter keeps that meaning now
(`batched_quads` is new); the full suite on the final state, with the silhouette change below,
passes 107 of 107. Limitation, as planned: a
batch is sorted against other translucent draws as one object; no image check changed, but no
check places a glow against glass. **Not tested** beyond the replays' images.

### Shadow silhouettes: each vertex placed once

The silhouette rasterizer skinned and projected every vertex once per triangle corner, about
six times per vertex. A surface's vertices are now placed once into a reused buffer and the
triangles index them; a unit test compares the coverage bit for bit with the per-triangle
path. Three interleaved pairs against the batching build, same settings, load average 4.4 to
5.7: the `shadows` stage 1.56 to 1.46 ms per crowd frame, frame 9.80 ms (9.71 to 9.81) to
9.70 ms (9.68 to 9.71). The remaining 1.46 ms is the triangle rasterizer itself and the decal
clipping for casters that moved; drawing silhouettes on the GPU stays an owner decision.

Model lighting, examined and **not changed**: 0.39 ms per crowd frame on the main thread, 96%
of it the light visibility traces of entities that moved more than a unit, each computed once
per entity per frame. No repeated work was found. Its larger cost is on the render thread: a
moved entity's lighting lives in its materials, so each is prepared again (about 1.7 ms per
crowd frame in the sample). Moving per-entity lighting out of the material is a rendering
design change, **not started**.

### Per-entity lighting outside the material: ceiling measured, not worth the change

Before designing it, a discarded experiment froze every entity's lighting after its first
state (wrong lighting, never kept): two interleaved pairs against the current build, 600
ticks, M1 Pro / Metal, 1080p, load average 4.8 to 6.7 recorded per run.

| Per crowd frame | Current | Lighting frozen |
| --- | ---: | ---: |
| Frame | 9.46 ms (9.2 to 9.70) | 9.04 ms (8.96, 9.12) |
| `model lighting` (main thread) | 0.38 ms | 0.02 ms |
| Render asset stage | 1.70 ms | 1.27 ms |
| Outside the main schedule | 1.40 ms | 1.17 ms |

Quiet is unchanged (5.54 against 5.57 ms). The 0.42 ms the experiment frees is mostly the
main-thread light traces (0.36 ms), which a change of storage would keep: moving the lighting
out of the material would only remove the material preparation, about 0.2 ms of a 9.5 ms
crowd frame, 2%. The earlier estimate of 1.7 ms was the whole material preparation stage in
a sample, most of which is not lighting. A workable design exists (a fixed-size storage buffer
of light states written in place, indexed by a per-draw mesh tag, with the material's own
lighting as the fallback), but it changes the model shader for every lit draw for that 2%.
**Not done; recommended against.**

### Machine load during these runs

The two new sampled runs were taken on a loaded machine: load average about 4 on 10 logical
CPUs, two unrelated processes at a full core each (an archive extraction among them) and
4.6 GiB of swap in use, read shortly after the runs rather than during them. Their frame
times (8.1 and 21.8 ms) are therefore not throughput evidence. The shares inside one run are
less sensitive, and the sample-to-scope agreement holds within the same run, but they should
be repeated on an idle machine. The first cycle's 30 runs recorded swap growth 706 to 2,339 MiB
and no CPU load, so their absolute values carry the same doubt.

### Rules for the next cycle

0. Record available compute with every run (other processes' CPU, free memory, swap, thermal
   state) and judge results per available compute; do not time on a busy machine.
1. Write the budget first: owning system, ms per frame, call count, expected saving. Do not
   start a change whose ceiling is under about 0.3 ms or under the measured run-to-run spread.
2. Close the frame before optimizing: `frame_total` must equal the sum of the
   `main_schedule.*` stages, over gameplay frames only; check it in every report.
3. Judge simulation changes by tick cost at a fixed tick count, not by frame time; judge frame
   changes with ticks per frame recorded. Keep stack sampling for attribution only, at 10 ms.
4. Rebase on current upstream before measuring anything.

### Ranked work, with ceilings from the timed frame

Ceilings are milliseconds per crowd frame from the whole-frame table (fixed-step shares from
the post-merge sample applied to 4.96 ms); each returns about 1.65 times itself in frame time.

1. Merge upstream (flight single sweep, filtered sight ray), re-profile. Done.
2. Whole-frame timing. Done; it corrected the stage means and this ranking.
3. Candidate rejection for world brush traces. Done: crowd tick 6.03 to 3.84 ms, frame
   13.85 to 11.18 ms; quiet tick 3.89 to 3.32 ms. The ceilings below predate it.
4. Shadow silhouettes: tested on the GPU; the final crowd pairs and precision limits
   are recorded under [GPU shadow silhouettes on station02](#gpu-shadow-silhouettes-on-station02).
5. Facial flex deltas once per actor, cheap key hash. Done: `animation` 1.46 to 1.02 ms
   crowd, 0.72 to 0.55 ms quiet. Flexes on the GPU would remove the rest and the mesh
   re-uploads (`render_assets` about 2.5 ms on the render thread); not planned.
6. Physics maintenance. Done: unchanged colliders are no longer reported to the step and the
   query tree is owned; tick 3.34 to 2.48 ms quiet, 3.80 to 3.20 ms crowd. What the broad
   phase still costs per step is **not re-measured**.
7. Sprite quads: additive ones are one mesh per material. Done: crowd frame 10.65 to 9.84 ms.
   Silhouette vertices placed once: `shadows` 1.56 to 1.46 ms. Next on the render thread:
   per-entity lighting carried in materials (about 1.7 ms of material preparation), eye and
   face mesh rewrites. `PreUpdate` 0.92 ms: not attributed.
8. One pose composition per actor per tick shared by its consumers. Ceiling about 0.5 ms.
9. Design choice for the owner: run the fixed step off the frame's critical path (its own
   thread, or overlapped with rendering), which removes the amplification itself.

Evidence: `artifacts/performance-cycle2/` (reports, 10 ms samples, logs) and the retained
`artifacts/performance-implementation-2026-11-11/`. A per-system and a per-call-path reducer of
the sampler's call graph produced the tables; they attribute each non-waiting self sample
once, so rows are exclusive. Windows, a matched native run and upstream's tip are **not tested**.

## Implemented changes: helper gains, no established frame-rate gain

Two bounded changes were implemented after the research phase. Effect sprites reuse existing
position/UV/color vectors, initialize new slots once and avoid unchanged material/visibility
writes. Each quad retains its draw, ordering, layers and bounds; material-handle remaps still
update. Manhack glows resolve first matching model instances once for multiple eligible actors,
with zero/singleton fast paths; shared current attachment math, eligibility, actor order and
first-instance behavior are unchanged. No shader, graphics quality, effect lifetime/count,
AI cadence, dependency or collision behavior changed.

Four effect-pool tests and three glow tests pass. An isolated release experiment measures
geometry construction/patching at about 0.242→0.0086 microseconds/update; this excludes Assets,
ECS, upload and GPU work. A 24-case equivalent-output glow experiment includes 30 actors behind
1,000/10,000/50,000 unrelated instances: 28.590→17.789, 279.063→46.276 and 1379.215→175.059
microseconds/collection. Small/front-loaded 30-actor cases add 0.43–0.59 microseconds and one-actor
front-loaded cases about 0.027–0.033 microseconds. These are helper results, not AI or game FPS.

### Thirty packaged runs do not establish end-to-end improvement

M1 Pro / Metal, 10 logical CPUs / 16 GiB, isolated release baseline 2208bcb plus the candidate runtime
changes; advanced-options/debug-tools plugins. Explicit High/HDR Full/no-AA settings, uncapped,
muted headless profiling. Five 600 host-tick cases, three paired rounds AB/BA/AB, reversed workload
order in round 2, no capture/build/test during timing. All 30 runs complete; all 15 paired final
inventory/AI/projectile digests and reduced outcomes match exactly.

| Workload | Baseline trials, ms | Current trials, ms | Median before → after, ms | Frame-time reduction |
| --- | --- | --- | --- | ---: |
| quiet | 5.568 / 5.536 / 5.521 | 5.632 / 5.549 / 5.530 | 5.5359 → 5.5493 | -0.24% |
| frozen-fire | 6.049 / 5.972 / 5.995 | 5.957 / 5.847 / 6.040 | 5.9949 → 5.9566 | +0.64% |
| animated-fire | 7.383 / 7.400 / 7.410 | 7.334 / 7.586 / 7.397 | 7.4000 → 7.3968 | +0.04% |
| plaza-crowd | 13.903 / 13.567 / 13.503 | 13.400 / 13.642 / 13.914 | 13.5671 → 13.6421 | -0.55% |
| plaza-crowd-retina | 14.131 / 13.877 / 13.847 | 13.871 / 13.927 / 13.845 | 13.8773 → 13.8715 | +0.04% |

| Workload | Baseline p95 / p99 upper bounds, ms | Current p95 / p99 upper bounds, ms |
| --- | --- | --- |
| quiet | 9.000 / 9.000 / 9.000 / 13.000 / 12.000 / 12.000 | 9.000 / 9.000 / 9.000 / 12.000 / 11.000 / 11.000 |
| frozen-fire | 7.000 / 7.000 / 7.000 / 13.000 / 13.000 / 13.000 | 7.000 / 7.000 / 8.000 / 13.000 / 13.000 / 13.000 |
| animated-fire | 11.000 / 11.000 / 11.000 / 14.000 / 14.000 / 14.000 | 11.000 / 11.000 / 11.000 / 14.000 / 14.000 / 13.000 |
| plaza-crowd | 21.000 / 19.000 / 19.000 / 23.000 / 23.000 / 23.000 | 21.000 / 21.000 / 22.000 / 24.000 / 25.000 / 26.000 |
| plaza-crowd-retina | 23.000 / 21.000 / 21.000 / 27.000 / 25.000 / 24.000 | 21.000 / 23.000 / 20.000 / 25.000 / 26.000 / 24.000 |

Positive reduction means faster. Trial ranges overlap; crowd 1080p p99 buckets worsen 23/23/23
→24/25/26ms. No sustained game-FPS gain is established. Frozen-fire's secondary contained-window
median 5.9182→5.7931ms (~2.1%) is not a robust general improvement: candidate window trials span
5.738–6.010ms and include a slower pair. Quiet, animated and crowd controls remain neutral.

Quiet has 0 sprites; frozen fire pauses at tick 86 with 142 quads / 130 particles; animated fire has
four actual detonations from six input requests, 62 final / 110 peak pooled quads. Crowd cases have
30 Combat/player-enemy manhacks, 1785 emitted sparks and 60 final eye quads; peak pools 597–612 vary
with render cadence, not dropped particles. Retina uses 3456x2168 internal pixels with the same
explicit settings, not the contributor's saved maximum settings. Actual dimensions are checked.

The 120 render-frame warmup covers unequal initial simulation intervals. Secondary windows are
weighted complete wall-clock windows with retained endpoints, not exact simulation alignment.
P95/p99 are 1 ms histogram upper bounds; p50 is unavailable. CPU scopes overlap/include setup;
render-graph CPU wall time is not GPU time. Occupied swap 706.00→2338.62MiB and about 1.66 GiB
system swapout traffic qualify the matrix; this is not attribution of paging to these changes.
Power/thermal status and complete custom-pass GPU timing are unavailable. Do not compare
these absolute values with the earlier 18-run sensitivity session as an optimization gain;
only the baseline/current pairs within this matrix test the implementation.

### Visual and validation disposition

All six original particle replays pass their assertions, but ordinary-HDR images are not
byte-identical: fire 1,332,005 changed pixels/max 5 RGB, smoke 53,266 / max 5, ball 929,126 / max 4.
Same-baseline repeated captures also differ (max 5/7/8), with different reported exposure.
Those failed exact-image checks remain retained; they do not prove a geometry regression.

A materially different untimed control uses existing `--tonemap-scale 1.0` and a 400 host-tick
prewarm. All six control assertions pass, forced exposure 1 / readbacks 0 and equal converged bloom
are verified. Fire and smoke are pixel-identical. Ball differs at 61 pixels, maximum 1 RGB, within
[1257,841,1286,854] over the viewmodel glow; exact ball pixel parity remains **not established**.
The capture approach stopped there; no further flags/tolerances turn that failure into a pass.
Controlled particle counts/collisions/visibility/candidates agree, but these bounded views do
not establish full campaign/all-camera/native parity. Evidence: `regression-summary.json`,
`capture-control-summary.json` and `controlled-capture-summary.json` in the ignored experiment.
The original 30 HDR timing runs were not rerun or replaced by fixed-exposure timing.

`scripts/checks/validate.sh` and `scripts/checks/check_windows.sh` pass. The latter is cross-target
strict Clippy, not Windows runtime testing. New source-only measurement tooling lives in
`scripts/checks/check_effect_pool_performance.py` with authored tests and five JSON fixtures in
`test-inputs/performance-research/`; the whitelist addition admits only those fixture sources.
The driver requires isolated artifacts packages, restores regular config files, rejects config
symlinks and changed full gameplay state, checks actual dimensions/outcomes and retains tails.
No game content or executable is committed.

Baseline executable SHA256:
`51cf256188b5e0446165cc2dc55ad90729768143683151f9c4e5182c41895fc5`.
Candidate SHA256:
`51002f989e04e2f8d72dc96bfa0fc55d1ce1c38ce17b7f776b7939f310cd07ab`.
The candidate build records a22f6a0 plus the uncommitted runtime source hashes pinned in
`artifacts/performance-implementation-2026-11-11/package-source-identity.json`; do not identify
it as an unchanged a22f6a0 build. Shaders match. Both builds omit four published upstream-main
commits including flight/sight improvements; repeat fetch found main 6d0b2f6 / session53 1473203.

Reproduction after preparing two isolated packages (use Python and `.exe` names appropriate
to the platform):

```text
python3 scripts/checks/check_effect_pool_performance.py --baseline artifacts/performance-implementation-2026-11-11/baseline-game/hl2-bevy --current artifacts/performance-implementation-2026-11-11/current-game/hl2-bevy --output artifacts/performance-paired-new
```

The driver builds nothing. The ignored experiment retains all 30 reports/logs/configs,
`paired-summary.json`, `paired-measurements.json`, environment snapshots, package/source identity,
CPU experiment logs and rejected attempts in `rejections.md`. Full validation/release logs are
retained there. Windows/macOS interactive performance, complete campaign regression coverage,
matched native 276 FPS reproduction and latest-upstream runtime are **not tested**. The next
high-value work remains existing 14s hitbox/query maintenance and render preparation attribution,
not scaling these helper ratios into a promised FPS. Plugin rules read: yes; exceptions: none.


## Why the helper wins did not become a frame-rate win

The implementation removed real repeated work, but selected kernels whose likely frame budget
was too small to explain the observed gap. The large isolated ratios were weak prioritization
signals without invocation counts and critical-path cost. A useful next cycle must budget the
absolute recoverable milliseconds before implementing a change, then measure the owning stage.
This conclusion is based on the actual presentation path and local workload, not a transfer
of an external demo's FPS or the historical Windows profile.

## The sprite experiment speeds a fraction of a fraction

The presentation system runs once in PostUpdate. It builds the effects once, then calls the
world and viewmodel quad pools once each. For a slot that changes, the geometry microbenchmark
saves approximately 0.233 microseconds of CPU construction. Multiplying that helper difference
by 142 hypothetically changed quads gives 0.033 ms; at a 6 ms frame that is about 0.55%, before
considering overlap and additional cost not covered by the microbenchmark. With every one of
the largest observed 612 world-pool slots changing together, the estimate is 0.143 ms, about 1.05%
of a 13.6 ms crowd frame. Those are optimistic scaling estimates for the measured helper,
not timings of the complete effects system, ECS or uploads, and not a proven frame-speed limit.
The peak pool is historical capacity; it does not mean 612 changing quads exist every frame.
Final crowd snapshots have 60 eye quads, for which the same calculation is 0.014 ms (~0.10%).

The frozen-fire workload is even more revealing: its 142 quads do not change after pausing.
The original implementation already compared the previous quad and skipped geometry rebuilding
when unchanged. Therefore the approximately 28x geometry helper improvement does not apply to
that steady workload at all. Its new benefit is avoiding repeated unchanged material/visibility
ECS commands. Command/asset extraction costs were not isolated in the geometry microbenchmark.
The tiny whole-run frozen gain and mixed secondary windows are consistent with that distinction;
they do not demonstrate a large buffer-rebuilding bottleneck.

## The repeated model lookup has the same scale mismatch

shot_effects calls manhack_glows once per presentation, not once per camera, attachment or
collider trace. For 30 actors after 1,000 unrelated instances, the measured saving is 10.801 microseconds
per complete collection, or 0.0108 ms. Even that artificial-prefix case is less than 0.08% of the
13.6 ms frame. The much larger 10,000/50,000-prefix wins describe data arrangements not established
in this map. The loader report records 570 successful model instances with no model errors;
30 spawned manhacks append 30 model instances through Gameplay::insert_entity. Mesh/submesh counts
must not be substituted for World::model_instances length. An exact live-vector observation
is still needed before claiming a precise map-specific lookup saving.

The microbenchmark's small/front-loaded 30-actor cases are 0.43–0.59 microseconds slower and the
single-actor path still has around 0.03 microseconds of overhead. That retained evidence refutes
a blanket claim that the new lookup is faster for all populations/orderings. It is a bounded
large-prefix repeated-search improvement with neutral whole-frame evidence in this workload.

## Millisecond-scale work remains outside both changes

Across the three current 1080p crowd runs, median scope means are fixed simulation 5.648 ms per
scope call, render graph 2.582 ms, render assets 1.951 ms, render prepare 1.035 ms, animation 0.786 ms
and shadows 0.799 ms. Their p95 medians are 7.895, 4.021, 5.285, 1.422, 2.104 and 1.821 ms respectively.
These are overlapping, setup-inclusive CPU wall scopes with different counts/denominators,
not exclusive stages to sum. The render-assets mean includes a roughly 501 ms setup outlier;
it is not a measured 1.95 ms steady per-frame upload bottleneck. Nonetheless they identify
millisecond-scale ownership boundaries, while the chosen helper budgets are microseconds.
A separate current-only sampled CPU diagnostic identifies the shared static world-brush
trace as a concrete remaining candidate; its limitations and prerequisites follow below.

The crowd's 3456x2168 image has about 3.61 times the pixels of 1080p, but its median frame changes
only 13.642→13.871 ms in the candidate. This is consistent with a substantial resolution-insensitive
cost, not proof that every GPU/custom pass is cheap. A faster GPU or an additional GPU culling
setup cannot remove CPU physics, pose or preparation work automatically. The next intervention
must follow measured critical-path ownership rather than adopt a GPU feature by headline.

## Measurement uncertainty is larger than the predicted saving

The paired frame ranges overlap; 1080p crowd p99 buckets worsen 23→24/25/26 ms. Whole-machine
occupied swap increases 706.00→2338.62 MiB with about 1.66 GiB additional swapout traffic. Neither
fact proves these edits caused a regression, but both prevent assigning small differences to
the edits as clean throughput wins. The original HDR captures also use different exposure;
unchanged particle clocks/counts do not make their framebuffer bytes comparable. Fixed-exposure
controls must be reported separately from the original timing matrix.

The next cycle first needs an up-to-date shared baseline including published 4805c93 flight
single-sweep and 7f0e9ff sight-filter fixes (fetched main 6d0b2f6), then a new matched profile and
unprofiled throughput check. This cycle's package/profile omits those four newer commits;
repeated old flight sweeps can amplify the observed static scan. Do not implement a new index
against a stale target. Coordinate the existing 14s lazy-hitbox/query-maintenance ownership.

If the shared static-brush kernel remains dominant, investigate that ONE kernel's candidate
rejection, with actual query/brush/plane/candidate counts and an absolute recoverable budget.
Reuse existing Rapier swept-AABB candidate traversal where conservative mapping can be proven;
retain exact fallback brushes for failed or unproven collider conversions. Existing BSP leaf-
brush membership is flattened by loading, and PVS is not a collision index. Feed candidates
in original stable brush order through one shared exact plane accumulator. Preserve contents,
1/32 epsilon, start_solid, all_solid, fraction and normal; per-brush minimum merging is not
identical when an all-solid brush follows an earlier normal. Cover tangent/stationary overlap,
escaping/all-solid cases, equal-fraction corner contacts, clip masks, large/asymmetric hulls,
failed conversion fallback and moving doors/props. Include candidate collection/sorting and
query maintenance in the measured cost; do not shift waste into another hot stage.

These task-specific lessons are recorded in this report and project design, not persistent
memory. No next-cycle kernel implementation or projected FPS is claimed.


## Current worker-thread stacks identify a stronger conditional target

Separate current-only quiet, crowded and frozen workloads run to host tick 1800 with completed
scripts; the crowd retains 30 Combat actors and the frozen workload 142 sprites. Five-second
built-in sampled stacks are parsed by conserving parent/child observations per thread.
These diagnostic runs perturb execution and are excluded from the paired throughput table.
The parser accounts for 30 threads in quiet/crowd and 29 in frozen fire. Counts below are stack
observations with keyword ancestry, not calls, milliseconds or exclusive frame percentages.

| Diagnostic | Quiet | 30-manhack crowd | Frozen fire |
| --- | ---: | ---: | ---: |
| Total thread-state observations | 46,948 | 55,354 | 36,010 |
| Known wait leaves | 43,888 | 51,900 | 33,060 |
| Other observations, including unresolved states | 3,060 | 3,454 | 2,950 |
| Fixed-system non-wait ancestry | 1,123 | 1,527 | 0 observed |
| Static brush trace | 295 | 710 | 0 observed |
| PhysicsPipeline step | 454 | 379 | 0 observed |
| Qbvh rebalance, any captured context | 195 | 162 | 0 observed |
| Broadphase | 196 | 161 | 0 observed |
| Optimized glow lookup | 0 observed | 10 | 1 |
| Optimized quad pool | 0 observed | 0 observed | 2 |

These categories overlap: in the crowd 634 of 710 static-trace observations occur beneath flight
traces, and 141 rebalance / 161 broadphase observations occur within the 379 physics-step group.
Never add those parents and children. 710/1527 is about 46.5% of the tagged non-wait fixed-system
observations, NOT 46.5% of total CPU or frame time. The main thread waits in 2101 of 2129 snapshots
while compute workers contain the active simulation work; a main-thread-only trace would
misidentify the problem. The residual non-wait denominator also contains unresolved thread
entries, so it is not an exact active CPU-time denominator.

Source inspection confirms modkit-core::trace_brushes visits the entire supplied static-brush
slice, testing relevant planes. Physics uses the misleadingly named player_world_brushes for
BOTH player and NPC hull queries; the crowd profile chiefly reaches it through NPC flight/
sweep paths. 523 observations use a common Air::sweep helper and 95 a direct manhack-steer path;
its module name does not mean the population is all birds. Render submissions and other
stages remain substantial, but the samples support investigating this shared simulation
kernel ahead of further micro-optimizing quad/glow helpers, after the latest-baseline check.
No exact brush-count denominator is exposed by the reviewed reports.

Evidence: `diagnostics/stack-summary.json`, its raw sampled stacks/parser and diagnostic reports;
source audit: `artifacts/performance-research-2026-11-11/next-cycle-static-brush-audit.md`.

## Scope and the reported gap

Subsequent direct clarification of the original observation identifies an RTX 3070: native
`d1_trainstation_02` plaza, many manhacks colliding and sparking, 276 FPS; the rewrite barely
60 FPS on average and much lower with many manhacks. The observer describes a 16-thread CPU
at 4 GHz and 64 GB RAM at 3200; its exact CPU model is not established. The earlier user-supplied
RTX 5070 identification conflicts with that original account and remains provisional; use
RTX 3070 as the reported hardware for this comparison. These are observations, not a matched
benchmark recorded here. Native/rewrite resolution, quality, exact view, actor count and scene
time remain unestablished. A separate contemporaneous contributor observation reports M1 Pro,
maximum graphics at 3456x2168, idle plaza 65–80 FPS at `2208bcb`; that interactive observation
is not the controlled indoor test below. At exactly 60 and 250 FPS, frame budgets are 16.67 and
4.00 ms: the difference is 12.67 ms of critical-path time. A small shader improvement
cannot by itself remove that gap if simulation and render preparation dominate.

The investigation followed successful particle, crowd and renderer demonstrations through
available author explanations, source code and comments. It covers simulation, animation,
physics, submission, geometry, visibility, effects, lighting and shaders. It is a bounded
survey, not an exhaustive search of the internet. External throughput figures belong to
their authors and are not independently reproduced HL2-RS improvements.

## What the existing evidence already establishes

The November 10 Windows profile in [performance.md](performance.md) used an RTX 3070,
Ryzen 7 3700X, release package `078d8d1`, 1080p owner settings and temporary reverted probes.
It remains a separately instrumented session, not a matched reproduction of the original
276 FPS observation or the fresh contributor measurements.

| Retained Windows scenario | Mean frame | Relevant measured work |
| --- | ---: | --- |
| Station02 plaza, empty | 13.6 ms | About 5.3 ms per fixed tick |
| Same plaza, 30 hunting manhacks | 42.1 ms | About 10.5 ms per fixed tick; AI 3.9–6.1 ms |
| Rapier in that crowd | 2.39–2.41 ms per tick | About 1 ms broad phase and 1 ms query maintenance |
| Crowd with hitbox posing disabled, diagnostic only | 24.4 ms | Fixed cost 6.49 ms, versus 8.50 ms in its paired control |

The hitbox ablation barely changed Rapier's own cost. It attributes expensive work elsewhere;
it is not an acceptable optimization because accurate hits still require current hitboxes.
The tree measurements used 5,659 colliders, four awake bodies and no contacts. This makes
unconditional query/tree work a stronger lead than reducing solver iterations.

Recorded render CPU scopes include graph 3.4–3.8 ms, preparation 1.7–2.2 ms and writes
0.5–0.9 ms per frame. Crowd shadows rose from roughly 0.04 to 1.5–1.7 ms. Instrumented
Bevy GPU passes omitted custom passes: their small total does not establish complete GPU
idle time. Fixed-tick and frame scopes overlap and have different denominators; do not sum
them as an exclusive frame decomposition. Slow frames also run additional fixed ticks,
which can amplify a simulation bottleneck.

GPU skinning, cached PVS with active player/monitor views, shared material handles, CCD
gating, unchanged-pose checks, a shadow atlas and cached shadow cases already exist.
They are not new discoveries or unimplemented recommendations. The existing on-demand
hitbox/query-maintenance plan is in [DESIGN.md](DESIGN.md), section 14s.

## Revision scope: already implemented upstream

Read-only fetched-ref inspection on 2026-11-11 found main at `6d0b2f6`, four commits after the
local measured baseline. Two changes already address the repeated-trace opportunity:

- `4805c93` replaces repeated flight sweeps past NPCs with one sweep excluding
  `CONTENTS_MONSTER`. Upstream records fixed-tick samples 8.50 → 7.95/7.75 ms;
  frame differences were explicitly within noise. [Flight change](https://github.com/kvalls/hl2-rs/commit/4805c93).
- `7f0e9ff` replaces repeated sight rays and exclusion-vector growth with one filtered ray.
  Upstream records crowd frames 37.1–39.8 → 27.9/28.4 ms, and fixed ticks
  7.75–7.95 → 7.22/7.29 ms. These are retained upstream Windows results, not fresh local
  measurements or a matched RTX 3070 comparison. [Sight change](https://github.com/kvalls/hl2-rs/commit/7f0e9ff),
  [measurement scope](https://github.com/kvalls/hl2-rs/blob/6d0b2f6/docs/performance.md).

The newest fetched published session53 tip is `1473203`, with additional manhack appearance,
asset/fidelity and lamp-beam/PVS work. It still poses NPC hitboxes each tick and passes the
query pipeline into the physics step; the existing 14s redesign remains planned.
[Published work](https://github.com/kvalls/hl2-rs/compare/6d0b2f6...1473203),
[latest plan](https://github.com/kvalls/hl2-rs/blob/1473203/docs/DESIGN.md).
Do not duplicate the completed trace changes or describe baseline timings as latest upstream.
Published branch evidence cannot establish the status of unpublished work.

## Successful cases: what actually made them fast

### Millions of agents: reduce neighbor work before increasing parallelism

The BabylonJS/WebGPU boids author reports approximately 16 million 2D or four million 3D
boids before falling below 60 FPS on an M1 Max. An accessible author reply explicitly says
that the acceleration structure is a plain uniform grid; the linked repository repeats the
claim. This supplies a useful chain from headline to author comment to implementation,
not an independently verified performance result. [Author post and replies](https://www.reddit.com/r/GraphicsProgramming/comments/13cemv5/webgpu_millions_of_flocking_boids_with_babylonjs/),
[project repository](https://github.com/jtsorlinis/BoidsWebGPU).

NVIDIA's particle implementation explains the transferable mechanism: bin or sort particles
by spatial cell, inspect neighboring cells, and improve memory coherence. Variable occupancy
requires correct overflow handling, such as count/scan/scatter or sorted cell ranges.
A grid is not automatically linear under arbitrarily dense clustering. [NVIDIA implementation paper](https://developer.download.nvidia.com/assets/cuda/files/particles.pdf).

Sebastian Lague's tutorial shader provides a useful counterexample: it moves neighbor
calculation to a compute shader but still loops over every boid for every invocation.
Parallel throughput does not eliminate quadratic work. It also uses squared distances for
radius comparisons. [Author shader](https://github.com/SebLague/Boids/blob/master/Assets/Scripts/BoidCompute.compute).

These boids do not reproduce HL2 schedules, speech, squads, weapons, hitgroups, scripts or
dynamic prop interactions. The transferable experiment is exact candidate reduction, not
replacing campaign actors with simplified GPU creatures.

### Factorio: stop repeating work and share results

Factorio developers measured dormant roboport updates falling from 1 to 0.025 ms per tick
in one save. A separate shared registration/bucket system for overlapping radar coverage
improved overall performance by 3.6% in another save. The subsystem reduction is not a
40-fold whole-game speedup. [Developer measurements](https://www.factorio.com/blog/post/fff-421).

Their path-cache work reuses suitable internal portions of previously computed paths instead
of requiring closely matching endpoints. Their parallel update experiment also demonstrates
that adding threads can become slower through cache contention, whereas read-only work can
scale better. [Path reuse](https://www.factorio.com/blog/post/fff-121),
[parallel update investigation](https://www.factorio.com/blog/post/fff-215).

For HL2-RS this motivates dirty flags, reusable pose results, exact active-work lists and
immutable job inputs. Every wakeup path must remain correct; offscreen scripts and NPC
obligations cannot be dropped because an actor is outside the current view.

### Millions of particles: representation and blend mode matter

Mike Turitzin's shipped PARTICULATE renders over two million particles to two VR eye buffers
at 90 Hz. His tested compute-rasterization variants outperform conventional rendering,
but specifically for single-pixel additive particles. Avoiding fragment-quad waste and
primitive-order requirements is central; cloud distribution affects the outcome.
This suggests a specialist spark path, not a general replacement for smoke billboards.
[Author's implementation analysis](https://miketuritzin.com/post/rendering-particles-with-compute-shaders/).

Xiao Jiang reports over one million particles at 60 FPS on GTX 1080/i7-8700K, using two
compute dispatches and one draw. The implementation keeps state in ping-pong structured
buffers, compacts live particles, and produces indirect arguments. It avoids individual CPU
particle objects and submissions. Full HL2 scenes, sorted transparency and equivalent gameplay
collision are not demonstrated. [Author's implementation](https://xiaojiangbrian.com/work/gpu-particles/).

The immediate HL2-RS opportunity is compact compatible sprite batches and reduced mesh
updates. Compute simulation is a later question if simulation itself remains expensive.
Keep alpha order, material behavior, simulation clocks and random/event sequences intact.

### GPU-driven rendering: the work before the draw call still matters

Ubisoft's Trials Rising presentation reports an initial construction-site CPU figure falling
from 51.43 to 24.09 ms and GPU from 22.17 to 17.95 ms on Xbox One. That iteration still
missed its target because gathering instance data and batching remained expensive.
Persistent pooled geometry/instance data and filtering address more than command submission.
[Primary GDC presentation](https://media.gdcvault.com/gdc2019/presentations/Drazhevskyi_Oleksandr_GPU_Driven_Rendering.pdf).

The Last General's developer reports tripled FPS and around 90% fewer batches. Author replies
identify unexpected fragmentation caused by objects instantiated at different times in Unity's
renderer, alongside occupancy, hierarchy and dependency changes. This is a reminder to inspect
actual batches; Unity component-reset remedies are not Bevy APIs.
[Author post and technical replies](https://www.reddit.com/r/Unity3D/comments/1nc1pt3/tripled_performance_on_my_dots_game_in_a_month/).

Niagara's culling shader combines bounds checks, LOD selection, depth-pyramid tests and a late
recovery pass. Its author measured a tradeoff: larger meshlets reduced culling effectiveness in
an artificial scene. Fewer batches and finer culling can oppose each other.
[Culling implementation](https://github.com/zeux/niagara/blob/master/src/shaders/drawcull.comp.glsl),
[meshlet measurements](https://zeux.io/2023/01/16/meshlet-size-tradeoffs/).

### Animation budgets and shader tuning have limits

Epic's animation budgeting lowers update frequency or interpolates according to significance.
Coconut Lizard's experiment reports 256 humanoids improving from 12 to 27 FPS, but explicitly
warns of immersion-breaking degradation around 128–255 actors. This trades temporal quality
for time; it does not justify silently changing HL2 animation or AI cadence.
[Official mechanism](https://dev.epicgames.com/documentation/en-us/unreal-engine/animation-budget-allocator-in-unreal-engine),
[measured experiment](https://www.coconutlizard.co.uk/blog/animation-budget-allocator/).

AMD explains why occupancy alone is not a shader performance target: available work,
registers, memory and latency determine the useful next change. Apple's tile-local rendering
guidance motivates attachment/bandwidth inspection on Metal, without establishing a portable
renderer rewrite. Meshoptimizer documents indexing/cache/fetch ordering and cautions that
overdraw reordering can be unhelpful on tile GPUs. [AMD analysis](https://gpuopen.com/learn/occupancy-explained/),
[Apple documentation](https://developer.apple.com/documentation/Metal/rendering-a-scene-with-deferred-lighting-in-swift),
[mesh layout guidance](https://github.com/zeux/meshoptimizer).

## Geometry headlines: distinguish source scenes from shaded frames

The Nyx author advertises roughly 1.6 billion unique / 18.9 billion instanced triangles and
links the implementation; author comments point to Epic's technical presentations. The
repository clarifies that these are source-scene totals, with actual per-frame work selected
by visibility and LOD. Its current example uses Ryzen 9950X / 96 GiB / RTX 5070, DX12 mesh
shaders, two-pass hierarchical-depth culling, compressed asynchronous pages and cached
shadows. It defaults to DLSS Quality with fallback, so output 4K is not proof of native 4K
shading. Current repository counts differ from the older post. This is a renderer prototype,
not a complete campaign comparison or a drop-in Bevy feature.
[Author post and comments](https://www.reddit.com/r/GraphicsProgramming/comments/1rm9mf0/i_built_a_nanitestyle_virtualized_geometry/),
[implementation and architecture](https://github.com/moonlovelj/Nyx).

The legacy Vulkan Guide sample reports 125,000 objects, 40 million triangles across two mesh
passes and 290 FPS on RTX 2080, with CPU processing under 0.5 ms. GPU-resident state and
indirect command construction explain its low submission cost; the chapter explicitly labels
itself outdated. These are sample results, not equivalent HL2 gameplay or predicted savings.
[Implementation guide](https://vkguide.dev/docs/gpudriven/gpu_driven_engines/).

## Current code: concrete opportunities and existing safeguards

| Area | Inspected implementation | Implication to test |
| --- | --- | --- |
| Hitboxes | `hl2-simulation/src/gameplay.rs`, `pose_npc_hitboxes` | Composes every enabled actor's skeleton each fixed tick; existing on-demand plan comes first |
| Query trees | `hl2-simulation/src/physics.rs`, `refresh_entity_queries` and `tick_inner` | Incremental scripted refresh exists; Rapier step also receives the query pipeline |
| Sensing | `hl2-simulation/src/ai/runtime.rs`, NPC sight gather | Cached ID list is cloned and scanned per think; exact spatial candidates may reduce work |
| Sight/flight | Baseline `hl2-simulation/src/ai/host.rs`; upstream commits above | Repeat traversal already removed upstream; validate those changes rather than duplicate them |
| Pose consumers | `hl2-simulation/src/entities/choreography/pose.rs`, Bevy eyes/shadow/render adapters | Exact poses are composed in several consumers; renderer already caches samples |
| Effects | `hl2-bevy/src/effects.rs`, quad-pool update | Handles/entities are pooled, but changed quads create replacement mesh arrays |
| Shadows | `hl2-bevy/src/shadows.rs` and child modules | Ranking, atlas and unchanged cases already exist; separate remaining stages before redesign |
| Lighting | `hl2-bevy/src/rendering/lighting.rs` | Unchanged illumination and cubemap choices are cached; profile invalidation/work first |
| Submission | `hl2-bevy/src/rendering/drawing.rs` | M1 direct-draw/CPU-clustering policy already reflects earlier measurements |

All source paths in this table are relative to `crates/`. These are observed code patterns,
not fresh timing attribution. Exact actor pose reuse must distinguish fixed simulation time
from presentation time, retain gestures/ragdolls/pose parameters, and publish current bounds
before visibility. The sensing change must preserve candidate order, relationships, hearing,
LOS, spawn/kill/teleport handling and dense-cell correctness.

## Ranked, independently reversible experiments

1. **Finish the existing on-demand hitbox plan.** Use conservative bounds to identify candidate
   actors; sample their exact current pose before hitbox-capable narrow queries. Audit bullet,
   melee, projectile, decal, impact and NPC paths. Compare hit entity, hitgroup, bone and distance
   against the reference, including moving limbs and the first shot after spawn or teleport.
2. **Finish query-maintenance ownership.** Track every insert, remove, enable and pose change;
   refresh at the existing pre-query/post-physics boundaries. Refit current bounds and measure
   amortized rebalancing. Passing no query pipeline to the step without replacing maintenance
   is incorrect. Preserve doors, props, ragdolls and maker spawns.
3. **Validate upstream sight/flight improvements on matched fixtures.** Reuse the completed
   one-query changes, preserving their collision semantics. Count casts/candidates and compare
   trace and gameplay outcomes. Do not implement a second independent copy of this work.
4. **Count rendering work by view.** Attribute extraction, changed materials/meshes, upload bytes,
   batches and draws to world, miniature sky, monitors, weapon and shadows. Test existing direct
   versus indirect overrides on each adapter; do not extrapolate the Metal policy to RTX.
5. **Batch compatible effect sprites.** Compare compact instanced records or reusable contiguous
   geometry with mesh-per-quad updates. Include zero effects, normal combat and dense smoke/sparks.
   Retain alpha ordering; separately report effects CPU, render-asset cost and total frame time.
6. **Test exact spatial sensing candidates and shared pose results.** First count candidates and
   pose compositions. Preserve stable decision order and exact clocks; avoid lossy neighbor caps.
   Reuse scratch capacity where allocations are measured before attempting a broad data rewrite.
7. **Attribute crowd shadows.** Separate pose sampling, silhouette production, projection, mesh
   writes and atlas updates. Verify moving actors, stationary crowds, doors and camera motion.
8. **Test existing world partitioning before adding an occlusion pipeline.** Compare candidate
   triangles with actual submissions. Any later occlusion experiment needs conservative current
   bounds, newly visible recovery and every monitor view; visibility never suspends gameplay.
9. **Time custom GPU passes at high resolution.** Include blur, bloom, flashlight depth and
   composition. Inspect attachment/copy traffic and shader bottlenecks only after attribution.
   Validate quality and resource lifetime alongside timing; ordinary Bevy GPU totals omit these.
10. **Evaluate load-time mesh layout and immutable jobs.** Preserve lightmap seams, mesh-local
    flex IDs and alpha order. A new dependency or materially different architecture needs the
    existing owner decision process. Quantization, simplified LOD and reduced update cadence
    are separate fidelity-sensitive proposals, not assumed free optimizations.

The growing after-AI stage in the retained profile also deserves an active-count/lifetime
trace: it may be legitimate workload growth or a cleanup issue. A memory leak is not proven.
Every experiment needs its own before/after result; estimated savings from different scopes
must not be added into a promised FPS total.

## Measurement and acceptance plan

Use isolated packaged release builds, complete replay outcomes, declared settings and fixed
scene time/view/workload. Run alternating repeated A/B trials after warmup with no concurrent
build or GPU workload. Record hardware, backend, package hash, resolution, internal render
scale, quality, VSync, plugins and profile status. Separate cold setup and shader priming from
steady state. Record p50/p95/p99 frame time, ticks per frame, fixed cost per tick, render work,
allocation/upload counts and GPU timings where supported. Measure profiling overhead separately.

Cover station02 spawn/plaza, ordinary campaign actors, hunting manhacks, moving props,
script-heavy interactions, outdoor geometry, effects, monitor views and map transitions.
Each timing fixture must finish its intended workload; a fixed frame cap can sample unequal
simulation duration. Pair visual comparisons with gameplay/trace assertions. Diagnostic runs
that disable work establish attribution only and cannot qualify an optimization.

Preliminary local sensitivity tests: **18 valid completed runs**, detailed below; the subsequent
30-run implementation comparison is recorded above. Matched native RTX 3070 comparison:
**not tested**. Windows validation of any future runtime change: **not tested** here. No
performance gain, sustained FPS target or complete campaign fidelity is established by this report.

## Preliminary sensitivity study: indoor station02 spawn

The existing release package matches `2208bcb618adfd3072ea5fd00553babdcb9a7866`, SHA256
`51cf256188b5e0446165cc2dc55ad90729768143683151f9c4e5182c41895fc5`.
Apple M1 Pro / Metal, 10 logical CPUs, 16 GiB, advanced-options/debug-tools plugins;
muted headless uncapped profiling. Explicit baseline: 1920x1080, FOV 75, HDR Full,
High model/texture/shader/shadow, World water, no AA, trilinear, motion blur and color
correction on, multicore on, VSync off. These are controlled settings, not the observer's
unestablished RTX 3070 settings or the local saved configuration.

Six variants ran in three sequential interleaved orders. Each completed the same script at
host tick 600; the 12,000-frame limit is only a safety cap. All 18 valid launches exited 0,
with zero GPU completion errors, pose/visibility mismatches and texture errors. Two initial
Low trials used an invalid water enum and remain rejected; two asset warnings and existing
runtime warnings remain. No native game was launched and no user-launched application closed.

| Variant | Trial mean frame times, ms | Median, ms | FPS from median | Trial p95 upper bounds, ms | Trial p99 upper bounds, ms |
| --- | --- | ---: | ---: | --- | --- |
| Baseline | 5.879 / 5.811 / 5.751 | 5.811 | 172.1 | 10 / 9 / 9 | 13 / 11 / 11 |
| 720p | 5.781 / 5.735 / 5.982 | 5.781 | 173.0 | 9 / 9 / 10 | 12 / 11 / 13 |
| PVS disabled | 7.736 / 7.674 / 7.784 | 7.736 | 129.3 | 10 / 10 / 11 | 14 / 14 / 14 |
| World partition | 5.773 / 5.775 / 5.811 | 5.775 | 173.2 | 9 / 9 / 9 | 13 / 13 / 14 |
| CPU skinning | 12.568 / 12.640 / 12.476 | 12.568 | 79.6 | 20 / 21 / 21 | 23 / 22 / 23 |
| Low at 1080p | 5.680 / 5.796 / 5.650 | 5.680 | 176.1 | 9 / 9 / 9 | 11 / 13 / 11 |

PVS disabled costs 33.1% more median frame time; CPU skinning costs 116.3% more. These
measure already implemented mechanisms, not a newly optimized build. The 720p and partition
differences (0.51% and 0.61%) are inside trial variation. Low differs by 2.25%, overlaps baseline
variation and changes quality; no general gain is established.

At the final viewpoint partitioning reduces candidates 52,768 to 7,257 triangles (about 86%)
but raises candidate meshes 50 to 62 and loaded meshes 1,835 to 2,587. The tiny timing change
shows why candidate triangle savings alone do not establish a faster frame. These are CPU
candidates unioned across views, excluding sky/viewmodels, not actual GPU draws.

The scene is the indoor spawn facing the stairwell exit, without a spawned crowd, not the
plaza. Baseline final feet (-4304,-224,-63.96875), eye (-4304,-224,0.03125), grounded, tick 600
agree. A separate untimed 1920x1080 capture shows lit, textured stairs and walls; its readback
cost is excluded. Evidence: `artifacts/performance-research-2026-11-11/baseline-visual.png`.

Frame statistics discard 120 gameplay frames and use 1 ms histogram upper bounds for p95/p99;
p50 is unavailable. Different rendered-frame counts mean the discarded frames cover different
simulation intervals; this is not a cycle-isolated skinning test. CPU scopes include setup,
discard 30 calls, overlap, and are not GPU timing. Existing compressed memory and about
640 MiB swap qualify results; unchanged swap occupancy does not establish zero paging.
Power/thermal sensors, full custom-pass GPU timestamps, live windowed FPS, heavy crowds
locally and Windows RTX 3070/native comparison are **not tested**.

Reproduce from the repository root:

```text
python3 artifacts/performance-research-2026-11-11/benchmark.py
```

The ignored experiment directory retains the isolated package, exact per-run arguments and
configs, raw reports/logs, `measurements.json`, `summary.json`, environment snapshots and
rejected trials. A populated matrix is resumed rather than fresh evidence; independent reruns
need a fresh isolated package/output set. The full report is
`artifacts/performance-research-2026-11-11/reports/HL2RS performance research.md`.
This earlier sensitivity phase introduced no runtime dependency or production optimization;
the subsequent implementation and paired results are recorded at the top of this document.

## Research access limits

Accessible Reddit author replies materially explained both the boids grid and Unity batch
fragmentation. YouTube descriptions/source links were discovery leads; no claim is made that
videos, their audiovisual contents or their comments were exhaustively inspected. Some pages
failed to fetch, including an exact-version Rapier documentation route and pinned Bevy occlusion
API. Current unversioned documentation is not proof that a feature exists unchanged in the pinned
runtime. No third-party crate was patched or runtime prerequisite introduced.

A point-cloud fractal proof of concept and an invisible-particle simulation update were excluded
from full-game throughput evidence. Headline counts without workload, representation, hardware,
settings and implementation detail remain leads, not performance results.
