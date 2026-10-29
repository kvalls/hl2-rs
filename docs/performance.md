# Performance measurements - 2026-10-05

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

Physics previously rebuilt the query tree twice every fixed step and marked unchanged poses as moved. It now incrementally refreshes actual changes;400 swept collision comparisons match full rebuilds across moves, rotations and visibility changes. Animated surfaces share rig samples, and constant poses no longer dirty/upload their meshes every frame. Full diagnostic snapshots are sampled every60 frames and refreshed for scripted/bounded capture completion and UI quit.

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
