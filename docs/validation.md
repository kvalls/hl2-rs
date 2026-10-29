# Local validation — 2026-10-05

## Background projection independence and Fit Display (2026-10-28)

The previous follow-up initialized the saved player FOV, stabilizing an incorrect dependency.
Background projection now ignores gameplay FOV, vertical overrides and zoom, using the authored
75-degree 4:3 reference with the existing aspect adaptation. Gameplay remains adjustable.
The inspected a7f071e3/be113848 paths also consume player FOV; no Fit Display regression date
is established. The prior inventory workaround is removed.

New host tests cover FOV/zoom independence at portrait, square, 5:4, 4:3, 16:10, 16:9,
21:9 and 32:9; projection-ray equivalence with artwork crops at portrait/square/4:3/16:9/
ultrawide; and actual Fit Display frame.size_for/aspect changes across landscape, portrait,
ultrawide and 4:3 windows. The first-view/tick lifecycle test asserts the menu invariant.
These are geometry/component tests, not interactive window-drag or native-image comparisons.

validate.sh passes (1,190 Rust / 100 ignored, 27 Python, strict Clippy/fmt/architecture), as
does check_windows.sh and release packaging. NO_BUILD=1 ONLY='background_fov background_reveal
coverage' python3 scripts/checks/replays.py passes all three groups. Paired 75/110 player FOV
and 0/90 vertical overrides change only 521 pixels at 4:3 and 9 at 16:9, in animated details.
Logs/captures remain in artifacts/background-projection and artifacts/replays. Windows runtime,
interactive Fit Display resizing, every background artwork and native visual parity NOT TESTED.

## Virtual sequence lookup and raw loaded mapping (2026-10-28)

From this checkout, `C:/Python310/python.exe -B ../../work/hl2-decompiled/actor-virtual-sequences-20261028bs/verify_static.py` passes11 exact unchanged pass14 identities,7 dumpbin windows/723 instruction starts checked against PE bytes,11 selected anchors and2 direct edges. Windows do not establish complete function extents. The optional fresh Ghidra import times out at330 seconds; its diagnostics and zero-record export remain retained. REA closes with open:false/cleanup_pending:false; the temporary recovery setting is reset.

`C:/Python310/python.exe -B ../../work/hl2-decompiled/actor-sequence-content-20261028bt/verify.py` independently verifies5 owned MDLs/1392592 member bytes,1416 offline candidate entries,9 selected descriptors and9 rejection controls. Source directory/archive ranges, CRC/member hashes, includes and duplicate decision remain private. This alone is not a retail-loaded mapping.

`C:/Python310/python.exe -B ../../work/hl2-decompiled/actor-virtual-raw-20261028bw/verify.py` replays3 accepted raw graph samples/201 reads/169128 bytes through the actual collector, compares1416 group/local tuples per sample and admits9 selected tuples/8 cached descriptors against owned checksum/name/label sources. It rejects9 in-memory corruptions, including coherent-hash metadata changes, truncated table, forged output and an empty error key. Two actor acquisitions remain in one tick; the third is excluded from actor-state timing. Root group0 stays uncached; direct-root local0 `ragdoll` is separately read. Root owned44/loaded48 is observed, not a converter trace.

`C:/Python310/python.exe -B ../../work/publishing/seal-research-sequences-20261028.py verify BS BU BV BW` passes37/43/54/46 files and2/9/15/9 external bindings. `C:/Python310/python.exe -B ../../work/hl2-decompiled/actor-sequence-content-20261028bt/seal.py verify` passes8 files/7 external bindings/5 owned member ranges. BU/BV acceptance means intact rejected diagnostic inventories, not successful mapping:1/12 rejected snapshots respectively. All three native games exit0 with six checked installed settings unchanged. The raw title-card screenshot is visually inspected; no actor pose comparison. No Frida attachment, Rust tests/builds or parity test. Document diff, source-whitelist and exact staged-set checks apply. First_map_finished:false; original16 gaps/four missing bodies remain.

## Nondefault background FOV at reveal (2026-10-27)

Background preparation now copies configured FOV into inventory before the initial camera
presentation. A 75/87.3083/90-degree lifecycle test asserts the initial and ordinary fixed-tick
values agree without advancing clocks. Removing the assignment rejects the old 75-degree
initialization at configured 87.3083. Both old/new screenshot guards pass, so they are not
decisive evidence for this short transition; logs are retained in artifacts/background-fov.
validate.sh passes (1,187 Rust, 100 ignored; 27 Python; strict Clippy/fmt/architecture), as does
check_windows.sh and release packaging. NO_BUILD=1 ONLY='background_reveal coverage' python3
scripts/checks/replays.py passes at FOV 90. Interactive owner confirmation and Windows runtime
NOT TESTED. No frame-cost claim; the assignment runs only during background installation.

## Advanced weapon blur at viewport edges (2026-10-27)

All captures retain the cutout-alpha correction. Normal SMG/shotgun fire/reload captures
still show fading when their gather footprints leave the viewport. The shader skipped
outside taps but normalized both color and coverage by the requested count. It now counts
in-frame taps and uses that divisor for both; in-frame background is still included. No
exposure, reach cap, tap density, ownership rule, world/retail path or texture fetch changes.
Normal fixed SMG fire/shotgun reload bursts were inspected alongside the earlier output.

NO_BUILD=1 python3 scripts/checks/check_weapon_edges.py uses real animated geometry and
13-frame bursts. A private old-divisor control detects 6,843 pixels losing coverage. All
corrected cases report zero loss, with these peak counts proving real edge execution:

| Case | No AA | 4x AA |
| --- | ---: | ---: |
| SMG fire | 5,098 | 5,078 |
| SMG reload | 1,200 | 644 |
| Shotgun fire | 9,933 | 10,426 |
| Shotgun reload | 2,309 | 1,277 |

The private shader is restored on failure/success. Evidence: artifacts/weapon-edges and
artifacts/render-fixes. scripts/checks/validate.sh passes (1,186 Rust / 100 ignored,
27 Python, strict Clippy/fmt/architecture), as does scripts/checks/check_windows.sh;
release package rebuilt. NO_BUILD=1 ONLY='viewmodel_motion_blur coverage' python3
scripts/checks/replays.py passes both groups. NO_BUILD=1 python3 scripts/checks/check_weapon_blur.py passes all eight pistol/switch/
reload/SMG ownership cases at no AA/4x, with zero borrowed-motion and missing-coverage
pixels. Strong crowbar reconstruction and pause/still/switch/off states remain verified. No new pass/resource or CPU work; only a scalar
GPU tap count is added. Fresh FPS, Windows/other GPU runtime, every animation phase and
native comparison are NOT TESTED. Current-frame reconstruction cannot recover hidden or
offscreen source geometry/colors.

## Cutout alpha and sky-dependent fringes (2026-10-27)

The accepted Source alpha-test output retained fractional texture alpha. Setting surviving
masked fragments to opaque alpha removes background/sky bleed during later camera composition.
Flat-black probes ruled out uninitialized white MSAA samples. An isolated opaque-alpha color
comparison darkens 4,203 tree-edge pixels at 4x AA and 4,296 without AA, with no brightened
pixels. Ultra/4x shows the same static correction. The sample/cutoff/mip/filtering rules and
prepass are unchanged; original texture alpha still controls material masks.

Run NO_BUILD=1 python3 scripts/checks/check_cutout_alpha.py after release packaging. On M1 Pro/
Metal at 1024x768 its private shader probe reports 7,339 partial-alpha survivors when the
correction is removed. Corrected trees: no AA 111,920 opaque/0 partial; 4x and Ultra/4x
108,103 opaque/0 partial. Corridor fence/4x: 57,334 opaque/0 partial. Real diagnostic execution
and clean GPU logs are asserted; normal tree/fence captures are retained for visual review.
The private copy's shader is restored even on failure. Evidence: artifacts/cutout-alpha and
artifacts/render-fixes/cutout-*. No installation content or user settings are modified.

scripts/checks/validate.sh passes (1,186 Rust / 100 ignored; 27 Python; strict Clippy/fmt/
architecture), as does scripts/checks/check_windows.sh. Other GPU runtime, Windows runtime,
all resolutions/maps, native visual parity and new FPS measurements are NOT TESTED.

## Background camera handoff regression (2026-10-27)

The old install frame exposed the player spawn view; the next fixed tick enabled the
logic_auto-authored menu camera. A temporary install-time capture reproduced a 532,602-pixel
change at threshold 35 versus the settled 1024x768 frame. Ordinary post-tick bursts missed
this frame. The production diagnostic capture now saves -installed.png whenever --capture
is requested, with no extra capture work in ordinary play.

Due zero-time background map-spawn I/O and the shared scripted-view resolver now run before
loading is dismissed. Unit tests prove immediate camera setup, unchanged clocks and preserved
nonzero output delays. Packaged background_reveal measures 2,931 changed pixels (0.37%,
animated details), below its conservative 5% guard; inspect both captures alongside assertions.
The old captured output fails that guard (67.7%). Evidence: artifacts/render-fixes and
artifacts/replays/background_reveal*.png.

scripts/checks/validate.sh passes (1,186 Rust, 100 ignored; 27 Python; strict Clippy/fmt/
architecture), as does scripts/checks/check_windows.sh. Both owned campaign submenu tests
pass. NO_BUILD=1 ONLY='background_reveal submenus return_menu coverage' python3
scripts/checks/replays.py passes all four groups. Release package rebuilt. Initial test
compilation rejected mixed Bevy/portable vector assertions; corrected to coordinate arrays
and all final checks pass. Windows runtime, native comparison and full campaign NOT TESTED.

## Chapter/rendering branch integration (2026-10-27)

Merged chapter UI/loading, commentary/movie playback and HUD alignment with the four-tier
blur and M1 Pro/Metal rendering policy. scripts/checks/validate.sh passes: 1,184 Rust tests,
100 ignored; 27 Python tests; strict host Clippy, formatting and architecture. The Windows
target check, release packaging and owned startup stub test also pass.

Packaged command: NO_BUILD=1 ONLY='chapter_selection return_menu submenus commentary video
motion_blur_options viewmodel_motion_blur coverage' python3 scripts/checks/replays.py.
Seven groups pass initially; motion_blur_options fails because its stale Video-tab pixel
lands outside the tab after incoming font/layout changes. Replace that click with existing
Tab/Right navigation and assert Video, VideoAdvanced and Advanced-without-Ultra states.
The focused NO_BUILD=1 ONLY='motion_blur_options' rerun passes, completing all eight groups.
Retain the failed report/capture and run logs under artifacts/merge-ui-chapters. No runtime
behavior was changed to satisfy the fixture. Inspect the chapter dialog capture alongside
report assertions; captures do not prove transient loading frames or native parity.

Windows runtime, fresh interactive FPS, other GPU performance and ordinary full-campaign
playthrough are NOT TESTED in this integration. Prior performance figures remain historical;
this merge adds no measured performance claim.

Repeated native qualification defaults to three Enabled/Advanced pairs per scene, with
order reversed in the middle pair. Report every iteration, paired cost ratios and median/
minimum/maximum frame means and p95/p99 values. Medians smooth ordinary noise; known
interference remains flagged rather than averaged away. The raw evidence is never hidden.

## Wrap-up validation and tier split (2026-10-27)

validate.sh passes 1,143 Rust tests (92 ignored), 27 Python tests, strict Clippy, formatting
and architecture. check_windows.sh passes strict cross-target Clippy; Windows runtime is
not tested. Owned blur labels resolve. GPU counters verify Enabled has no reconstruction,
Advanced has weapon-only reconstruction and Ultra has both. Existing saves and console
master resets are tested. Eight normal weapon surface cases before the label split (same weapon algorithm) and final
ONLY="viewmodel_motion_blur coverage" replays pass. No user-launched app is closed.

Native idle frame budgets pass for new Advanced, but pressure flags leave qualification
provisional. Ultra still fails the strict budget. Priming, cold/failed and every measured
run are retained; no threshold is loosened. Historical full-blur Advanced series are now
Ultra. Final-tier Ultra projectile/world suite and interactive FPS remain untested.
The cooperative archive holds the GPU serial comparator/canary; both kernels were rejected
for Metal cost and are not active in the preferred build. The normal fragment path remains.

## Known-workload preflight and main-depth probes (2026-10-27)

The native guard saves each .preflight.json before launch. Known other games or compiler
processes in the current baseline abort the batch before invoking a game; moderate CPU/GPU
utilisation alone does not. Unknown counters do not establish idleness and still fail later
resource qualification. A workload beginning during a run remains flagged in its retained
samples. No automatic retry, data removal or threshold change is introduced. Twenty-five
Python tests include a busy-baseline case proving saved evidence and no game launch.

The main-depth candidate passes check_world_blur.py and all eight check_weapon_blur.py
cases (no AA/4x). Normal camera/lamp images were inspected. Full validate.sh passes 1,136
Rust tests (92 ignored), 23 Python tests at that point, strict Clippy, fmt and architecture;
the later harness checks raise the Python total to 25. The active-camera bookkeeping
refinement passes eleven bloom tests and host/Windows-target strict Clippy. These are
packaged Metal image and cross-compilation checks, not Windows runtime validation.

Native attempts main-depth-repeated and main-depth-isolated are retained separately. The
former overlaps another replay; only this task's harness/children are stopped. The latter
passes idle budgets with two comparable pairs and one memory-mismatched pair, then refuses
a compiler-contended preflight. Three pairs across all scenes, the broader crowbar replay
and interactive FPS are not qualified. After two interrupted attempts, further timing waits
for an exclusive window instead of launching more known-contended batches.

## Repeated direct-colour candidate (2026-10-27)

NO_BUILD=1 BLUR_LABEL=direct-sources-repeated BLUR_RUNS=3 python3
scripts/checks/check_blur_performance.py completes eighteen runs. All have valid GPU
completion bounds and zero rendering/wait errors; the overall performance guard fails.
Three swing pairs are resource-comparable and consistently fail both budgets; idle/turn
pairs retain their interference flags. All iterations remain in the named ignored evidence
directory, with source/package identities and resource-report.md. The direct-colour world
GPU probe passes; no interactive or Windows runtime qualification is claimed. Source is
preserved on codex/experiment/blur-direct-colour-sources. The main-depth follow-up is a
separate archived candidate, not implicitly qualified by these earlier captures.

## Headless GPU completion and settled baselines (2026-10-27)

Headless --profile runs use a two-marker ring in the host render graph. Completion markers
follow render submission; the oldest marker is awaited before reuse, bounding GPU render
submission without serialising every CPU frame. Reports expose submitted/observed-completed
frames, maximum pending fences, wait time and errors. The native guard requires a limit of
two, at least 100 observed completions and zero errors. No marker or wait is installed for
ordinary or interactive runs. Empty submissions add no render pass or callback allocation.

A native six-run batch exercises the limiter. Swing driver-managed memory averages about
2.23 GiB versus 4.40 GiB in the earlier unpaced resolved-world run; these are aggregate
counters. Marked waits are GPU backpressure, not render-pass timing. Pre-pacing headless
FPS reports measure produced-frame cadence and cannot alone establish GPU throughput.
The limiter retains CPU/GPU overlap; isolated overhead and native Windows/Linux are untested.

The sampler's first reading is retained as a primer, excluded from baseline means. It can
contain the previous game's GPU load while CPU counters have no initial delta. Subsequent
samples prefer a quiet baseline for up to five seconds; all baseline history is saved.
Resource comparison remains the committed matched-input protocol: stable moderate load is
comparable, while hardware differences, material shifts and known interference/pressure
remain flagged. A short-lived experimental requirement for total quietness is removed;
earlier files keep their original flags. No frame-time threshold is loosened.

## Hardware-aware benchmark evidence (2026-10-27)

The native blur guard writes per-run `.environment.json`, `resource-report.md`, package hashes
and source identity. `BLUR_LABEL` selects a simple directory label under the ignored evidence
root; `BLUR_TELEMETRY_INTERVAL` defaults to one second. Frame windows are epoch-aligned after
warm-up, retain 128 completed windows plus the partial tail, and report dropped windows.
The helper asserts aligned frame counts when no windows were dropped. Resource/output ratios
use that same retained window. No per-frame allocation or file access is added to the game.

Live host sampling is currently macOS-only: aggregate CPU ticks, process CPU time/RSS,
IOAccelerator utilisation and GPU-managed system-memory counters, VM pages and pressure.
CPU idle and driver GPU headroom are occupancy proxies, not available FLOPS. RAM availability
is free+inactive+speculative pages; purgeable pages are not added again. Apple Silicon uses
one unified pool, so dedicated VRAM availability is null. GPU counters are system-wide;
multiple ambiguous accelerators and unsupported platforms leave metrics unavailable.
Thermal limits are recorded where the OS supplies them; missing clocks/limits are unknown.

Qualification retains the existing mean/p95 thresholds. It also flags builds/other games,
pressure/pageout/swapout changes, unavailable samples, hardware mismatches, material baseline
memory differences and baseline CPU/GPU changes over 10 percentage points. These criteria
are declared before measuring; resource pressure caused by Advanced remains a regression
cost, not a reason to excuse the rendering. All cases are collected even when one fails.
Within-run Pearson correlations require six aligned windows; constant/insufficient inputs
produce null. No significance test, causal claim or cross-GPU compute-efficiency claim is
made. Sampler CPU accounting includes its child commands; a sample containing a game exit
is excluded from that accounting. Actual isolated FPS overhead is not tested.

The first six-run native batch and the sampler-accounting run complete without render errors,
but all scene pairs fail qualification. Evidence is in artifacts/blur-performance-regression/
environment-loaded and environment-observer. The latter detects competing builds instead of
silently treating a later retail slowdown as renderer cost. See docs/performance.md.

## Native-resolution blur performance guard (2026-10-27)

`NO_BUILD=1 python3 scripts/checks/check_blur_performance.py` uses a copied muted package,
owned d1_trainstation_02 content, 3456x2168 / 4x MSAA / HDR None / VSync off. It never edits
the user's package settings. Idle, continuous yaw turns and repeated crowbar swings are
paired between retail Enabled and Advanced. Each report must show the requested internal
viewport and at least 100 post-warm-up measured frames. Defaults can be changed with
BLUR_WIDTH, BLUR_HEIGHT, BLUR_AA and BLUR_RUNS; altered workloads do not qualify the owner's
native-resolution case. Do not overlap GPU runs with other games, builds or image analysis.

Acceptance thresholds remain mean <= retail * 1.20 + 0.25 ms and p95 <= retail * 1.25 +
1 ms. These are regression guards, not FPS guarantees; the current renderer fails them.
A failing crate does not prevent these opt-in owned checks from being run separately.

The --profile frame histogram starts after 120 warm-up frames. Percentiles are conservative
1 ms bin upper bounds; the overflow bin returns the exact maximum, retaining hitches above
255 ms. Maximum is exact. Ordinary-play percentile/max fields are null and collection is
skipped. Unit tests cover these boundaries and the ordinary-play/warm-up gates.

Encoder-boundary GPU timestamp experiments on Metal return zero durations for every blur
span. Those values are unavailable measurements, not evidence of zero GPU cost. The
experiment was removed; existing CPU command-encoding spans must not be called GPU timings.
The separate single-sample camera and resolved-motion cache experiments also failed the
native idle budget and were removed. Full investigation evidence remains in ignored
artifacts/blur-performance-regression and STATUS.md.

## Advanced world/projectile reconstruction (2026-10-27)

Baseline a8a050b uses camera-only retail world blur even in Advanced; a still camera gives
projectile models no independent blur. The new world path shares cached reconstruction
plumbing with the weapon path. Relative object motion comes from depth reprojection;
camera gathers use odd 17-257 densely spaced samples, and dynamic gathers extend moving
opaque/masked silhouettes while rejecting objects behind a nearer stationary surface.
Transparent smoke/glow/additive sprites do not supply independent motion vectors.

The owned isolated/muted GPU probe is `NO_BUILD=1 python3 scripts/checks/check_world_blur.py`.
Generated fixtures reuse existing grenade/RPG/bugbait inputs and recorded lamp coordinates.
It asserts live projectile state, current poses, no render errors, actual camera/object
motion at no AA/4x AA, expanded silhouettes, and absence of the Advanced world path in
Disabled/retail Enabled. Projectile error-color counts are restricted to the projected
projectile neighborhood, avoiding unrelated moving actors. Normal and diagnostic bursts
are retained in artifacts/advanced-world-blur/runs. Shader probes restore the copied shader
in finally; the owned installation and user config are untouched.

Initial probes find 1,431/1,492 moving grenade pixels, 17/13 missile pixels and 563/591
bugbait pixels at no AA/4x AA. Silhouette probes find 4,936/46/214 surrounding pixels for
grenade/missile/bugbait. These are GPU-path evidence, not native-image fidelity or proof
of every projectile/frame rate. Normal captures were inspected qualitatively; different
live bursts are not exact pose/history-aligned before/after pixel comparisons.

Rejected first composite: averaging static background along an object's ray smeared a
neighborhood of scenery. Static background now keeps its own color except for actual
foreground coverage; only moving center surfaces estimate hidden background from samples.
Rejected world-prepass depth hypothesis: plaza stippling also exists with blur/prepasses
off, so a speculative depth-bias experiment was removed. Initial Clippy type-complexity
failure was fixed with the shared view-query alias. A later turn probe found sky pixels
classified as relative object motion; zero-depth pixels now use camera-only motion and are
excluded from object tiles. MSAA samples subtract their own depth-dependent camera motion
before averaging; using the nearest sample depth for all samples falsely marked static
edges as object motion. The superseded probe batch was stopped before completion.
Reprojection now consumes the same PreviousViewData as Bevy's prepass. Persistent per-view
textures are owned directly: Bevy's one-frame texture pool cannot safely reserve retained
textures for independently activated cameras. No third-party changes were made.

Existing `viewmodel_motion_blur coverage` and standalone weapon-surface probes preserve
crowbar trails and slow pistol/switch ownership. Final local validation passes (1,127 Rust
tests, 92 ignored, 13 Python helpers,
architecture, strict Clippy/fmt), as does Windows-target Clippy. The full owned world batch
passes; the final affected MSAA, pause and normal lamp/turn cases are rerun separately
(final-extra.log), with at most 254 relative-motion pixels in the camera-only MSAA case
instead of the earlier sky-sized classification. The standalone weapon-surface suite passes
all eight animation/AA combinations with zero borrowed-motion or lost-center pixels.
Normal lamp and projectile captures inspected. Final package executable and all ten shader
hashes match metadata and the tested copies. Native Windows/Linux execution, native visual
comparison, all frame rates/resolutions, arbitrary transparent effects and ordinary campaign play are **not tested**.
Performance evidence is in docs/performance.md. The old detached worktree remains parked:
archival was rejected because the app identifies it as this chat's primary checkout;
attaching the original checkout was rejected because it is not app-managed. Work continued
in the original hl2-ui checkout without changing unrelated scratch files.

## Advanced weapon blur surface stability (2026-10-27)

The owner reports blinking/twitching on pistol and weapon-change animations in Advanced.
The original gather assigns dominant neighboring motion to covered pixels moving below one
pixel. An actual GPU diagnostic on the original shader finds up to 4,603 fully covered
pistol pixels borrowing unrelated motion. Only the postprocess diagnostic in a copied
package is changed; animation/model poses remain the owned ones. The fix always uses local
motion on covered pixels, includes the center via odd 17-97 taps, and fades reconstruction
from one to three pixels of motion. The fast trail footprint/exposure remains unchanged.

`NO_BUILD=1 python3 scripts/checks/check_weapon_blur.py` passes four animations (pistol fire,
pistol reload, switch-to-pistol draw, SMG fire) at no AA and 4x AA. Each nine-image live burst
asserts the selected weapon/animation, zero pose-bound mismatches, no rendering errors,
positive GPU diagnostic execution, zero borrowed-motion pixels and zero covered pixels
with no gathered coverage. The positive control prevents a missing/failed pass from passing
only because it drew no error color. Generated variations reuse the registered viewmodel
fixture and standard isolated/muted/background runner; no chapter runner/UI files changed.
Run this check separately from replays.py because both stage their own test package.

`NO_BUILD=1 ONLY='viewmodel_motion_blur coverage' python3 -u scripts/checks/replays.py`
passes both checks. The existing crowbar test still asserts real GPU motion at no AA/4x AA,
blur outside the current silhouette, unchanged upper world, and correct still/pause/switch/
Disabled/retail behavior. Its copied shader hooks are restored afterward.

`scripts/checks/validate.sh` passes (1,125 Rust tests, 92 ignored, 13 Python helpers,
architecture, strict Clippy/fmt); `scripts/checks/check_windows.sh` passes. Release builds
use scripts/build-bevy.sh and executable/shader hashes are checked against metadata and the
tested copy. No new runtime dependency, pass, texture or CPU animation history is introduced.
Performance evidence: docs/performance.md.

Normal live bursts/contact sheets were inspected qualitatively. Initial frame-index pixel
comparisons are rejected because request scene times differ. Matching request times also
does not fix the preceding-pose history, so these images are not exact visual-parity or
pixel-improvement measurements. One matched sharp prepass/retail sample differs by zero
pixels above threshold 2 (maximum 1); a separate unaligned frozen pair differs by 5,007 and
cannot establish/exclude a depth-prepass bug. The accepted regression instead examines the
GPU's own per-fragment motion choice. Initial contact-sheet output had a two-row header;
a correctly sized replacement supersedes it. Failure qualifications and original/fixed
captures/logs live under artifacts/advanced-blur-flicker and artifacts/replays.

Windows/Linux runtime, every weapon/animation, all frame rates/resolutions, ordinary campaign
play and native visual comparison are not tested. Advanced remains a screen-space animation
blur approximation and excludes transparent/diagnostic CPU-skinned draws.

## Retail quick-info size and horizontal alignment follow-up (2026-10-27)

The earlier vertical alignment checks did not constrain the horizontal midpoint
or native size. A new midpoint assertion fails on the previous revision: x319.274
instead of x320 at 640x480. Center the pair's bitmap bounds while preserving the
installed glyph spacing, and center the portable weapon glyph by ink width.

Retained accepted retail captures at 720p and 1080p were inspected beside the
packaged output. The 1080p capture's recorded hl2.exe, engine.dll, client.dll and
server.dll hashes all match the current owned installation. Its session closed
normally with settings unchanged. The first 1080p session had an empty module
manifest; use the second verified session for provenance. Rejected SMG-labelled
captures are excluded (they actually showed a pistol). The accepted 720p pistol
reference and verified 1080p pistol reference contain the same reticle type as
our SMG layout fixture; compare layout, not the captured weapon label.

Retail arcs are about 55x42 pixels at 720p and 85x63 at 1080p. The previous macOS
package drew 75-pixel-tall arcs at 720p: ClientScheme's macOS QuickInfo tall=50
was interpreted using Windows character-cell metrics. The local QuickInfo font
resolver now selects the installed desktop tall=28 for that raster contract.
No global platform condition or stored option is changed. The corrected emitted
bitmap bounds are approximately 57x42 and 86x63, with at most two pixels of fontdue
antialias fringe versus the visible native footprint. Tests constrain those sizes,
both axes, partial/warning geometry and all existing viewport/aspect cases.
A parser test verifies a consumer override cannot alter normal host conditions.

Reproduction uses the same commands in the section below. Workspace validation
passes (1,166 Rust tests, 100 ignored, 13 Python helper tests, strict Clippy/fmt/
architecture); Windows-target Clippy and five owned HUD tests pass. Packaged
HUD/aspect checks pass before (2/2); HUD/aspect/power pass after (3/3, 101 s).
Evidence, native module hashes, retained
failed midpoint assertion and the nearest-neighbor comparison image are in
artifacts/quickinfo-horizontal. The comparison uses retained retail captures;
no fresh native game run or exact cross-platform antialias/color equivalence is claimed.

## Quick-info and crosshair layout (2026-10-27)

The installed-resource regression fails on the prior geometry: at 640x480 the arc
bitmap center is y245.541 instead of y240. Portable SMG crosshair pixels were also
above view center; both symbol paths now center bitmap bounds independently of
text ascent. Native Windows GDI reticle placement is preserved.

```sh
scripts/checks/validate.sh
scripts/checks/check_windows.sh
cargo test -p hl2-ui hud:: -- --ignored --nocapture
NO_BUILD=1 ONLY='hud hud_aspect power_hud' python3 scripts/checks/replays.py
```

Workspace validation passes: 1,165 Rust tests, 100 ignored, 13 Python helper tests,
strict Clippy/fmt/architecture. Windows-target Clippy passes. Five owned HUD tests
pass, including health/ammo panel framing and power HUD behavior. The new layout
test checks actual emitted arc and weapon-crosshair quads across 640x480, 1280x720,
1920x1080, 1281x721, 3440x1440 and 720x1280 with all four HUD aspect settings.
Full, half-filled and warning arcs must center within pixel-rounding tolerance;
both progress pieces must meet without gaps/overlap, and both arcs stay additive.
Packaged HUD/aspect checks pass before the change (2 groups); HUD/aspect/power
checks pass afterward (3 groups, zero failures, 99 seconds). The 720p gameplay
capture was inspected: both arcs and the SMG reticle share view center. All eleven
executable/shader hashes match package metadata.
A portable font test deliberately changes ascent and checks full/split symbol
bounds at three sizes, with and without blur, including progress endpoints.

Initial failed alignment evidence is retained. The warning test initially sampled
the zero-time blink-off phase; it now uses a visible pulse phase. An unused import
after extraction was caught and fixed by strict Clippy. Evidence lives in
artifacts/quickinfo-layout. Native Windows execution, retail raster equivalence
and an ordinary campaign playthrough are not tested by these checks.

## Background-loading wait presentation (2026-10-27)

Production package `fa4a000` supersedes the interactive-during-background-load policy.
The owner still observed lag; earlier acceptance/completion tests were insufficient
latency evidence. On-demand native dispatch review now establishes menu hiding
(`InstantHideMainMenu`, menu/title alpha zero); details in gameui.md. Ordinary native
retail playback/input timing remains not tested.

`scripts/checks/validate.sh` passes: 1,145 Rust tests / 94 ignored, 13 Python helpers,
architecture, strict Clippy and fmt. `scripts/checks/check_windows.sh` passes.
Owned checks pass:

```sh
cargo test -p hl2-bevy --bin hl2-bevy native_menu_wait -- --ignored --nocapture
cargo test -p hl2-bevy --bin hl2-bevy headless_bevy_app_submenu_entry_switches_background_map_and_scene -- --ignored --nocapture
```

The native input-system check blocks arrows/Enter/Escape during the wait, then allows
opening, selecting and starting a chapter after completion. The actual asynchronous
worker test attempts chapter opening while a worker owns its consumed request and
asserts the menu is hidden, input is gated, the load remains alive, and scene replacement
completes (10.59 s). Unit tests cover success and failure restoring access, direct
activation, Escape, hidden-window canvas rendering, preserved chapter selection, and
shared card-browser occlusion cancelling held activation. Background request safeguards
remain independently tested. No English UI literal or new dependency was introduced.

Build with scripts/build-bevy.sh. Eleven executable/shader hashes match package metadata
for fa4a000. Packaged checks use a private copy and default config:

```sh
NO_BUILD=1 ONLY='return_menu submenus chapter_selection menu_options coverage' python3 scripts/checks/replays.py
```

Transient capture limitations are retained rather than counted as passing evidence.
The burst capture experiment skipped the loading interval because diagnostics/capture
counters stop while simulation is loading. It recorded the loading-request label with
menu.visible=false and subsequent restored controls, but its images are not screenshots
of the wait state. The controlled attached windowed launch loads an owned map through
map_background; its report also records false at request and true after completion.
Computer use could not bind the unbundled binary as an app (Invalid app; absent from
inventory), so no native wait-screen screenshot was obtained. No further capture retry
was made. Headless canvas-quad comparisons verify hidden rendering; packaged restored
browser captures provide visual restoration evidence. Final physics/GPU installation
can still hitch. Native Windows/Linux execution and end-to-end retail UI latency are
not tested here.

Evidence in artifacts/chapter-loading-gate: validate.log, windows.log, ui-tests-final.log,
toolkit-tests.log, native-wait.log, owned-wait.log, build.log, package-hashes.json,
replays.log, wait-burst-report.json and retained burst PNGs/log, window-wait-report.json
and log. Initial compile/test assumptions and their corrections are retained in ui-tests.log.
All five groups pass, zero failures, 270 s. Restored return-menu and Episode Two
browser captures were inspected and copied alongside their reports; both report
menu.visible=true and NewGame after loading. The burst and controlled windowed
reports record menu.visible=false at the request and true after successful completion.

Three isolated alternating prior/current browser pairs compare package 6019af3 with
fa4a000: 18.693/18.743 ms mean frame, HUD CPU 0.1550/0.1581 ms. Run ranges overlap
slightly; a 0.051 ms difference is not isolated causal evidence. No added render pass
or frame asset access. Details and reproduction in performance.md.

## Dialog occlusion and background loading (2026-10-27)

Runtime checkpoints: `5a70c90` (dialog/request lifetime), `6019af3` (resource reuse
and rapid switch back). Full `scripts/checks/validate.sh` passes: 1,143 Rust tests,
94 ignored, 13 Python helper tests, strict Clippy/fmt and architecture. The
architecture baseline shrinks for the asset loader/file and campaign poll.
`scripts/checks/check_windows.sh` passes against final source; native Windows/Linux
execution is not tested here.

Owned-content checks:

```sh
cargo test -p hl2-bevy --bin hl2-bevy native_chapter_input -- --ignored --nocapture
cargo test -p hl2-bevy --bin hl2-bevy headless_bevy_app_submenu_entry_switches_background_map_and_scene -- --ignored --nocapture
```

Both pass. The first drives the actual per-frame Bevy keyboard input system while
simulation is marked loading: arrow selection responds, Escape waits for release,
closing preserves background work, reopening and Enter release submit a chapter
with foreground loading. This is a controlled headless input-system check, not
an OS event-latency measurement. The second decodes and installs actual Episode
One/Lost Coast background worlds asynchronously and checks scene replacement.
It now opens chapter selection after the actual worker consumes the pending flag,
changes selection, closes on Escape release, verifies the load remains active, and
then checks successful completion and scene replacement. This strengthened test
passes in 11.30 s; final local/Windows strict Clippy and fmt also pass.
Unit regressions cover the return confirmation's lifetime, queued/active load
kinds, background result supersession/coalescing, foreground presentation, stale
failure and rapid return to the already installed background without reloading.

The old packaged build reproduces the reported rectangular hole after gameplay,
return confirmation, background completion and reopening chapter selection.
The revised return-menu fixture includes that sequence and asserts the final
browser/game identity and absence of an active confirmation. Captures and reports
are under artifacts/chapter-bugs: return-before and return-after (with suffixes).
The first baseline attempt selected no keyboard row and is retained as
return-before-rejected-no-selection; it is not visual evidence of the fix.
Initial native-input test failures incorrectly used press activation and expected
selection to survive reopening; corrected to release activation and reset-on-open.

The final release package is built with scripts/build-bevy.sh; executable and ten
shader hashes match build-bevy-info metadata for `6019af3`. Initial fix/resource-reuse
package: `NO_BUILD=1 ONLY='return_menu chapter_selection submenus menu_options coverage'
python3 scripts/checks/replays.py` passes five groups, zero failures, 290 s. This
includes all four campaign chapter previews/selected-map launches. After the final
rapid-switch reuse helper, committed package `6019af3` passes
`NO_BUILD=1 ONLY='return_menu submenus coverage' python3 scripts/checks/replays.py`:
three affected groups, zero failures, 111 s. Return capture is intact and inspected;
no dialog rectangle is cut out. Performance evidence is in performance.md.

Evidence: artifacts/chapter-bugs contains validate-resource-reuse.log,
windows-test-final.log, clippy-test-final.log, fmt-final.log, native-input-resource-reuse.log,
owned-browser-during-worker.log, replays.log, replays-final.log, package-hashes.json,
return-before/return-after captures/reports/logs and timing outputs. Retail findings
and verification limits are in gameui.md; static exports do not prove native loading
input responsiveness or establish zero-hitch loading in either implementation.

## Chapter selection (2026-10-27)

Source implementation: `56717d2`; single-chapter containment fix: `da289c2`;
reusable toolkit extraction: `7171e3f`.
Research/design: `7db0bbf`; findings and limitations: [chapter-selection.md](chapter-selection.md).

Commands (no installed files are changed):

```sh
scripts/checks/validate.sh
scripts/checks/check_windows.sh
cargo test --locked -p hl2-ui owned_chapter_catalogs -- --ignored
scripts/build-bevy.sh
NO_BUILD=1 ONLY='chapter_selection coverage menu_options submenus' python3 scripts/checks/replays.py
```

The final local run passes 1,135 Rust tests with 93 ignored, 13 Python tests,
architecture, strict Clippy and formatting. Strict Windows-target Clippy passes;
Windows native execution is **not tested**. The explicit owned-install test passes:
15/5/7/1 selectable entries for base/Episode One/Episode Two/Lost Coast, complete
localized titles and thumbnail coverage, correct first/last map and installed
Play controls. VFS fixtures verify own loose/custom/archive enumeration, exclusion
of inherited/embedded configs, and MOD-scoped reads even with embedded collisions.

Input/model tests cover numeric/suffix sorting, malformed/missing-map configs,
Episode Two recap syntax, pointer press/drag-off/back/release selection, release
activation with game identity, arrow paging/clamping, focus/Cancel, portrait frame
fit, Escape-on-release, preserved menu/background and input during background
loading. Canvas regressions assert native-size thumbnail sampling and containment
of the single-card layout. The toolkit owns five browser tests (121 total); hl2-ui has 302 passing tests/15 ignored.

The first packaged round passed submenus, chapter-return backgrounds, chapter
selection and non-modal Options (4 checks/305 s). All four selected chapters loaded,
but its stretched-thumbnail captures were rejected. SDK ImagePanel confirmed
native-size clipping; corrected images were inspected. A second round passed
base/Episode One/Episode Two pointer/cancel/load checks and coverage, but Lost
Coast's fixture clicked Cancel using wide-dialog coordinates. The failure and
images were retained, the pointer coordinates corrected to its own layout, and
visual inspection additionally found overwide decorative separators. The fix
centers a single card and sizes those rules using the authored frame inset.

Final committed package (`7171e3f`) passes submenus, chapter_selection, menu_options
and coverage: 4 check groups, zero failures, 199 s. The chapter group runs four
browser previews plus four actual chapter loads. It asserts opening does not start
gameplay; Cancel/reopen preserves the background; all titles/thumbnails are available;
9a sorts correctly; pointer selection and Start use the correct chapter and game;
load completion reaches Gameplay with no loader failures:

| Game | Selected chapter | Verified loaded map |
| --- | --- | --- |
| Half-Life 2 | 2 | d1_trainstation_05 |
| Episode One | 2 | ep1_citadel_03 |
| Episode Two | 2 | ep2_outland_02 |
| Lost Coast | 1 | d2_lostcoast |

All four final browser captures were inspected after the resource/containment fixes;
post-extraction comparison changes zero of 139,040 opaque thumbnail pixels across
ten sampled image interiors (all four games). Geometry/input behavior is covered
by the toolkit tests; transparent/background pixels are excluded from that comparison. Captures still
show unfinished world/material rendering; UI validation does not establish complete
background-map fidelity. Eleven package executable/shader hashes match the build
metadata. Replays use a muted/background copy with default settings, leaving the
normal package's settings and owned content untouched.

Retained rejected evidence: artifacts/chapter-menu/rejected-stretched,
rejected-pointer-coordinates and rejected-single-chapter. Final check logs:
validate-toolkit.log, windows-toolkit.log, owned-toolkit.log, replays-toolkit.log
package-hashes-toolkit.json and toolkit-thumbnail-comparison.json under artifacts/chapter-menu. Final performance
evidence: [performance.md](performance.md). Native desktop comparison, Windows/Linux
runtime, other UI languages and an ordinary campaign playthrough remain **not tested**.

## Selected LookAt registration, passive requests and animation fields (2026-10-27)

From this checkout, `C:/Python310/python.exe -B ../../work/publishing/seal-research-lookat-20261027.py verify BN BO BP BQ` passes closed inventories of46/122/93/18 files,7/15/13/4 external bindings and one bounded installed-content range for BP. It verifies only retained files/ranges; it does not rerun collectors. Historical BJ/BK/BL/BM seals still pass, including the failed BL diagnostic.

`C:/Python310/python.exe -B ../../work/publishing/verify-selected-lookat-static-20261027.py` checks14 BN +8 BQ exact baseline/original bodies against unchanged pass14/retail identities, six Ghidra body ranges totaling2480bytes/818 instruction starts,17 field anchors and21 actual-citizen ancestry entries. Six additional curve anchors verify member offsets and packed10-byte indexing. Two Ghidra pseudocode texts match after whitespace normalization; four do not. This is bounded static verification, not native execution or semantic parity.

Within private BO, `C:/Python310/python.exe -B verify.py` passes505 contiguous records/198 complete pairs/7483 raw reads/1750776 raw bytes, two JSON/two binary chunks, no probe errors/drops/open calls,26 observed prefixes and a closed finish witness. Fresh epoch begins at seq17. Selected callback seq289 registers the genuine citizen with low8 returns1 and count5 ->6;27 ProcessLookAt/AddLookTarget/request-helper chains update the player handle with duration float32(0.1), ramp0 and importance0.000181373..1. Eighty-five raw vector snapshots pass; the selected scene record is absent at the final snapshot. The actual observer source passes24 deterministic controls before native load. An earlier summary-variable shadow printed5 ->1 despite correctly checking5 ->6; its draft/report remain explicitly rejected.

Within private BP, `C:/Python310/python.exe -B verify.py` accepts5573/5687 samples, checks124721 raw-read occurrences/20272960 bytes and retains109 tick-crossing/five reload exclusions. Maximum measured acquisition gap0.5251283s; normal180-second deadline ends sampling. Actual loaded descriptor admission checks sequence/cycle/rate,24 nested pose floats and parent proxy. Eighty-seven rows retain the selected event;27 distinct coincident player requests corroborate duration/ramp/importance. Five packed curve points and default257 remain identical; all five loaded values match float32 reciprocal multiplication, with four one-step differences from rounded division. This is not a restore-instruction trace. `C:/Python310/python.exe -B check_controls.py` passes its positive control and rejects11 in-memory corruptions without changing retained files.

Private BR adds read-only `C:/Python310/python.exe -B verify.py verify`: 20 retained files/three external bindings, three exact bodies, seven byte anchors and direct PE checks of531 dumpbin instruction starts. The byte-bound reciprocal constant, event+curve caller, unsigned-byte conversion and MULSS explain all five BP values, including four one-step rounded-division differences. A no-match literal query and rejected first reference remain retained. Python310 lacked the proposed Capstone decoder; its pre-seal failure is preserved and no second-decoder agreement is claimed. This is static evidence, not an observed loader invocation.

Actor/controller generation, clocks and vector contents are bracketed/rechecked; single target-entry reads do not establish globally atomic target lifetimes. Source flags, callbacks, requests, numeric animation fields and final pose remain separate evidence levels. Both new owned native processes exit0/settings unchanged; Frida/Ghidra sessions close and owned process inventory is empty. Final gaze, included-model clip mapping, overlay/flex/root/final pose, audiovisual fidelity, ordinary first-map completion and Rust parity are not tested. No Rust tests/builds apply to this documentation-only milestone.

## File recorder, opening tuples and passive timing control (2026-10-27)

`C:/Python310/python.exe -B ../../work/publishing/seal-research-recorder-20261027.py verify BJ BK BL BM` passes four closed inventories:309/121/85/89 files and0/19/1/3 external bindings. BL is a sealed failed diagnostic, not a successful control. BJ's verify.py/verify_variants.py/check_controls.py check30,002 records,15,000 native fixture pairs,90,000 raw reads and16 negative controls. Fixture medians: baseline0.01036us, empty hook0.518us, hex recorder1579.996us, binary recorder198.167us per call; these are not native HL2 performance measurements.

BK/verify_trace.py checks388 records/134 pairs/5,030 raw reads/1,397,556 bytes, whole-file envelopes and a closed finish witness with no drops/open calls. Starting at seq83 checks the disclosed fresh-map epoch:306 records/108 pairs/4,017 reads/1,102,760 bytes. compare_authored.py matches39+11 exact event tuples, including the combined-disabled LookAt; check_controls.py rejects10 corruptions. Actual observer source passes19 offline scope/generation checks before native load. No callbacks or missing prefixes are manufactured from a provider summary.

BM/verify.py accepts4,138 of4,143 samples, checks62,710 reads/15,072,408 bytes and preserves five null-slot reload exclusions. Maximum measured acquisition gap0.348107s. G-Man playing rises within[10.2299995,10.2600002] and clears[70.4249954,70.4550018]; endline rises[93.1650009,93.1949997] and clears[98.3549957,98.3849945]. Three instrumented callback/state milestones fit those brackets. Endline output completion occurs while playing1; BK ends before that run clears playing. These are non-atomic raw-state observations, not a full timing-fidelity or hook-overhead result.

All three owned native processes close with exit0 and unchanged checked installed settings; the Frida session closes and process-specific audio finishes. BK's finalized129.97s stereo44100Hz waveform is nonzero; no dialogue listening/identity/mix fidelity was tested. No Rust tests/builds apply to this documentation milestone. Original16 gaps/four missing scene procedures and first_map_finished:false remain.

## Windows unit-test portability (2026-10-26)

Contributor kvalls reports three failures on Windows 11 at 4405064 using
`cargo test --workspace --locked --no-fail-fast`. The fixes gate the macOS-specific Retina
assertion, defer installation/setup/output effects until after flag parsing, and normalize
CRLF in the cvar golden comparison. Local validation and CI now use --no-fail-fast.

`scripts/checks/validate.sh` passes: 1,125 Rust tests, 92 ignored, 13 Python tests,
architecture, strict Clippy and fmt. `scripts/checks/check_windows.sh` passes. Parser tests
supply Config directly and assert unresolved installation requests, background override,
setup conflict validation and noninteractive flags. The initial compiler run exposed three
campaign test callers of the old API; they now share an explicit fixture-startup helper.

The actual cvars.golden was changed temporarily to 626 CRLF lines, then
`cargo test -p hl2-ui --locked every_cvar_matches_its_golden_transcript -- --nocapture`
passed. A finally block restored the original file bytes and verified its SHA-256; the
fixture has no source diff. Evidence: windows-crlf-golden.log/windows-crlf-fixture.json.

`scripts/build-bevy.sh` rebuilds the release package. Muted, background
`NO_BUILD=1 ONLY='bg_compare hud coverage' python3 -u scripts/checks/replays.py` passes:
default/menu startup across the four installed games and direct-map HUD startup. An explicit
missing installation with --no-setup --background --frames 30 exits 1 with an installation
error and no setup-window launch. Eleven executable/shader package hashes are verified.
Logs/reports are under artifacts/pr-readiness and artifacts/replays. The startup split adds
no per-frame work or render pass; no new frame-time comparison is needed.

Windows execution and the new Windows CI run are not tested locally. Cross-target Clippy,
macOS unit execution and CRLF simulation are distinct evidence and are not a Windows pass.

## PR readiness and automatic target maintenance (2026-10-26)

Runtime baseline 4405064: `NO_BUILD=1 JOBS=2 python3 -u scripts/checks/replays.py`
passes all 102 default background/muted checks (30 UI/rendering, 72 gameplay/coverage),
zero failures in 1,730 seconds on macOS/Metal. All eleven original-package and tested-copy
executable/shader hashes match afterward. Portrait Options and orange/cyan stacked tooltip
captures were inspected without clipping or incorrect panel spacing. An earlier interrupted
run stopped after two passing checks and is not counted as a full-suite pass.

`python3 -m unittest discover -s scripts/checks -p 'test_*.py'` passes 13 checks: three image
helpers, four cache/launcher cases and six locking cases. Cache fixtures cover workspace-only
incremental/dependency deletion, profile retention and dry-run bytes. Stubbed POSIX launches
cover default cleanup, --no-clean, rebuild/debug and --no-build, including game arguments.
The original regression fixture reproduced missing dependency-cache bytes; its initial
launcher marker leaked between subtests and was corrected before accepting results.

An isolated real Cargo 1.99 project proves both directions of exclusion. Cargo reports
waiting for a file lock while cleanup holds profile locks. After release its build script
signals readiness and waits for a test gate; stale cleanup skips without deleting candidate
bytes while Cargo holds its lock. After Cargo exits, cleanup removes only the old workspace
fixture. Other tests cover shared debug/release lock contention, partial-lock release,
exceptions during pruning, missing locking support and network/unknown filesystems.
No actual project Cargo cache is pruned. The POSIX-specific cases are skipped on Windows.

The lock contract was checked against [Cargo's profile layout](https://doc.rust-lang.org/stable/nightly-rustc/cargo/compiler/layout/index.html)
and [its compatibility-lock implementation](https://doc.rust-lang.org/stable/nightly-rustc/src/cargo/compiler/layout.rs.html).
Exclusive .cargo-lock guards coordinate with the installed Cargo build. Cargo can omit
locks on network mounts, so pruning is restricted to recognized local filesystems.
Rejected probes: BSD stat format strings returned file metadata; disk-management services
were unavailable; Python statvfs exposed no local-mount flag. Native Darwin statfs, using
its verified 64-bit record, identifies the tested filesystem as APFS without those services.
The first filesystem guard rejected APFS and failed four tests; the corrected native probe
passes the complete suite. Linux uses its filesystem-type probe but lock integration there
is not tested in this session.

The first full validation attempt failed `installed_maps_list_every_games_loose_maps_once`:
an unexpected `test` map from another fixture appeared. Timestamp-only directory names can
collide between parallel tests. Installation fixtures now reserve paths with exclusive
create_dir and retry suffixes. A deterministic identical-nonce test checks independent bytes
and drop lifetimes. This changes only test code; production asset loading is untouched.
The subsequent full suite passes; the failed log is retained alongside the final run.

`scripts/checks/validate.sh` passes: 1,123 Rust tests, 92 ignored, 13 Python helper tests,
architecture, strict Clippy and formatting. `scripts/checks/check_windows.sh` passes.
`bash -n launch-bevy.sh scripts/clean-target.sh scripts/checks/validate.sh` and
`git diff --check upstream/main` pass. CI runs the Python helper suite as well.
README describes default auto-clean, --no-clean and safe skipping; other capability claims
were corrected in place without a feature history. No game/shader payload changed, so no
additional gameplay rebuild/replay or frame-time comparison was run. Windows PR CI remains
a merge gate. Evidence: artifacts/pr-readiness (helper/validation/Windows logs, full runtime
replay log and pre/post package hashes).

## Stronger Advanced weapon blur (2026-10-26)

`scripts/checks/validate.sh`: 1,122 Rust tests pass, 92 ignored; three Python image-reader
checks, strict Clippy, formatting and architecture pass. `scripts/checks/check_windows.sh`
passes. `scripts/build-bevy.sh` refreshes the package; eleven executable/shader hashes are
verified. The gameplay-resource cleanup regression now also asserts depth prepass removal.

`NO_BUILD=1 ONLY='viewmodel_motion_blur coverage' python3 scripts/checks/replays.py` passes.
The updated sharp control bypasses only reconstruction. Accepted comparison: 19,226 changed
weapon-region pixels above threshold 2, zero upper-world changes. A new GPU diagnostic,
using actual weapon depth and nonzero integrated coverage, colours only blurred pixels
outside the current silhouette: 13,316 pixels. This prevents an interior-only filter from
passing. No-AA/4x-AA motion, settled/pause/switch/Disabled/retail Enabled and zero pose
mismatch assertions pass. Copied-package shader hooks are restored in finally blocks.

Prior cbe521b and current packages also run the same live fixture ending at tick 102 with
`--capture-live --capture-burst 12`. Their first four burst scene times match exactly:
1.545, 1.575, 1.590, 1.605 seconds. Captures show a continuous streak rather than a sharp
outline during the fast section, and clear as the weapon leaves view/settles. These are
controlled authored inputs, not an ordinary playthrough or native comparison.

Evidence: artifacts/viewmodel-blur-strength/live.py, live.input.json, live-times.json,
prior/current live PNGs/logs/reports, retained normal/sharp/coverage captures and timing data.
The first live burst had no info-level timestamp logs; timestamped reruns supersede it.
Two rejected image-analysis assumptions expected a stationary upper crop and at least six
changing weapon frames. The fast weapon can cross that crop and leave the view after four
visible transitions; those assumptions are invalid. Matched-time visual inspection and
controlled same-pose coverage diagnostics provide the accepted evidence instead. Details
are retained in rejected-live-assumptions.json. A timing round that overlapped that image
analysis is rejected and replaced; see docs/performance.md.

Initial private helper visibility and a 104-line function failed checks; the helper scope
and pipeline factory were corrected before validation. Windows/Linux runtime, every weapon/
reload, all frame rates/resolutions, native visual comparison, transparent effects and
diagnostic CPU-skinned blur remain **not tested**. Straight-line current-frame reconstruction
approximates curved movement and cannot reconstruct hidden/offscreen geometry.

## Motion blur Advanced / viewmodel animation (2026-10-26)

`scripts/checks/validate.sh`: 1,122 Rust tests pass, 92 ignored; three Python image-reader
checks, architecture, strict Clippy and formatting pass. `scripts/checks/check_windows.sh`
passes. Owning tests exercise history warming/reset, three dropdown states, invalid picks,
old boolean saves, Advanced round trips and retail console-command behavior. A minimal
Bevy app regression verifies existing blur/prepass components are removed when gameplay
resources disappear. The owned
`cargo test -p hl2-ui owned_motion_blur_advanced_token --locked -- --ignored --nocapture`
verifies installed Disabled/Enabled/Advanced text. The initial token search found that
GameUI_Advanced means Lock; the implementation uses verified GameUI_SettingAdvanced.

Packaged, muted checks: `NO_BUILD=1 ONLY='motion_blur_options viewmodel_motion_blur
flashlight_shadows flashlight_viewmodel options_status lamp_emission coverage' python3
scripts/checks/replays.py`. Checks run separately against copies of the release package;
see recorded checkpoint results. The dropdown is selected through real pointer/keyboard
input, visually inspected, applied and read back from saved settings. Its preview is
`artifacts/replays/motion_blur_selected.png`.

The crowbar fixture releases the selection lock before attacking, asserts a miss animation
and a fixed player view, then captures the swing and four following frames. A shader control
bypasses only weapon reconstruction; moving-weapon pixels change while the upper still
world is identical. A separate diagnostic colour after the one-pixel velocity threshold
proves actual GPU-skinned movement at no MSAA and 4x MSAA, restricted to the lower weapon
region. Settled poses, pause, switching, Disabled and retail Enabled have no stale vectors.
Diagnostic edits affect only the copied package and are restored in finally blocks.

Rejected evidence is retained under artifacts/viewmodel-motion-blur: separate material
uniforms exceeded Metal vertex bindings with previous-pose skinning; a single shared
buffer fixes it without third-party changes. The initial attack fixture retained the
selection lock; an initial report assertion used a nonexistent player.eye field. A
coverage run overlapped addition of a new registered fixture. Packing renamed two test
shader hooks; corrected hooks are asserted against the actual shader before rerunning.
An early recommendation initializer and profiling-span borrow also failed compilation;
subsequent complete checks pass. These are not accepted captures or measurements.

Windows/Linux runtime, every weapon/reload, native visual parity and transparent/diagnostic
CPU-skinned weapon blur are **not tested**. Screen-space reconstruction does not expand
silhouette coverage. Advanced is an owner-requested extension; SDK viewrender.cpp applies
retail camera blur before DrawViewModels.

## Options platform visibility and development hints (2026-10-26)

Owning checks: `cargo test -p hl2-ui --locked`, `cargo test -p source-ui --locked` and
`cargo test -p hl2-ui owned_options_support_resources --locked -- --ignored --nocapture`.
The owned-resource check inspects actual layout visibility, disabled controls, associations
and help tokens, and searches installed localization resources for development-report text.
No matching installed development text was found; new text uses HL2RS_ placeholders.

`scripts/checks/validate.sh`: 1,118 Rust tests passed, 91 ignored; 3 Python capture-reader
tests, architecture, strict Clippy/fmt pass. `scripts/checks/check_windows.sh` passes.
`scripts/build-bevy.sh` refreshes the release executable, shaders and metadata; all nine
package hashes match. Source-ui tests cover panel order/spacing and screen-edge clamping;
HL2 tests cover cyan-only, retail-only and stacked hints, cached hover text, disabled
subdialog controls, popups, tab changes and preservation of hidden stored values. Pure
platform tests explicitly exercise Windows/macOS/Linux/Other, independent of the host.

Packaged `NO_BUILD=1 ONLY='options_status options menu_options coverage' python3
scripts/checks/replays.py`: existing Options and menu/popup occlusion groups pass. The new
three captures assert unfinished status, completed controls unmarked, Mac-only hiding,
Aero hiding outside Windows and popup/click dismissal. Cyan pixel checks prevent a status
report alone from masquerading as visible rendering. Accepted final results: STATUS.

Rejected evidence under `artifacts/options-status`: initial fixture used unsupported
input-key `escape` (corrected to the UI Escape action); Advanced status then inherited the
inactive parent's fade. The latter capture/report is retained as `rejected-parent-alpha.*`.
The tooltip now takes the focused subdialog's alpha and restores the parent multiplier;
a regression checks both. Early compilation/test adaptation and strict iterator warnings
are retained with the subsequent passing checks. Native colour matching is not applicable
to this owner-requested development overlay; Windows/Linux runtime remain **not tested**.

## Flashlight availability and lifetime (2026-10-26)

`scripts/checks/validate.sh`: 1,111 Rust tests pass, 90 ignored; 3 Python capture-reader
tests, architecture, strict Clippy and fmt pass. `scripts/checks/check_windows.sh` passes.
An initial strict check rejected a helper below a test module; it was moved before tests
and the complete validation passes. Logs retained under `artifacts/lighting-bugs`.
`cargo test -p hl2-simulation flashlight --locked`: 11 pass. Registered player target test
and campaign state-carry test pass separately. The architecture ratchet shrinks two long
functions; existing idle/player-carry logic is factored without changing its behavior.

Release package: `scripts/build-bevy.sh`. Availability fixture begins away from the canal
suit, attempts impulse 100 without it, picks up the installed suit normally, then tests
activation and authored disable/enable. No cheat loadout is used. A lethal fall tests
death shutdown/dead impulses; a subsequent direct load tests fresh permission/power state.
The first death assertion was too early (still airborne at tick 300; dead by tick 320),
so labels were corrected from the recorded trajectory. Rejected evidence remains in
`artifacts/lighting-bugs/rejected-fall-availability.*`. This was a fixture timing failure,
not proof of a death-shutdown defect. All six packaged groups pass (`flashlight_availability`, `power_hud`, `flashlight_shadows`,
`lamp_emission`, `flashlight_viewmodel`, `coverage`). After final review tightened invalid
entity-id rejection, the rebuilt final package passes availability/coverage again. Its
executable and eight shader hashes match build metadata. Results are recorded in STATUS.

Campaign carry/rebased recharge and suit removal are unit-tested; an ordinary campaign,
actual suit-removal input, save/restore, all vehicle/control locks, player-proxy outputs,
Windows runtime and native image comparison are **not tested**. Exact installed on/off
bodies were reviewed privately; the additional Enable/Disable/death/impulse routing comes
from the SDK. The installed on body checks suit/disabled flags without a weapon gate.

## Fluorescent self-illumination (2026-10-26)

`scripts/checks/validate.sh` and `scripts/checks/check_windows.sh` pass. Package built with
`scripts/build-bevy.sh`. `NO_BUILD=1 ONLY='lamp_emission flashlight_viewmodel coverage'
python3 scripts/checks/replays.py`: all three groups pass. The new fixture uses installed
canal model/material data and fixed 1920x1080 HDR None. It requires over 10,000 brighter
tube pixels above threshold 8 versus the old emissive expression, fewer than 10 darker,
and fewer than 10 changed upper-wall pixels above threshold 2. The control runs in the
disposable package and restores its shader in `finally`. Both accepted captures inspected.
Rejected angled lamp capture retained in `artifacts/lighting-bugs/baseline-lamp.png`;
accepted front-on probe and packaged images are retained. Native comparison and Windows
runtime are **not tested**. Performance evidence: docs/performance.md.

## Canal paper receivers (2026-10-26)

`cargo test -p hl2-bevy canal_papers --locked -- --ignored --nocapture`: owned-map test passes.
The actual corridor paper model is static, and its baked world triangles keep that identity.
With ordinary reception disabled the isolated paper receives no triangles; NPC-style RTT
reception produces 17. This tests geometry/policy, not native image parity. Baseline floor
capture inspected at eye 765,-8180,-64, yaw151.4/pitch-25. Initial material diagnostic compile
failure and corrected output are retained. Full local and Windows-target checks pass for
the checkpoint. Native visual and packaged NPC-over-paper comparison are **not tested**.

## Lighting follow-up: direction report (2026-10-26)

`cargo test -p hl2-simulation shadows --locked`: all 15 pass. Exact installed-client
registrar/callback/setter reviewed with hash-checked provenance against SDK; the command
selects a fixed downward vector or stored global direction according to RTT mode.
The owned canal map's authored control is recorded in docs/lighting.md. Architecture and
repository isolation checks pass for the documentation checkpoint. Native visual play,
per-entity overrides and live control inputs are **not tested**; no runtime rebuild needed.

## Controlled first-map route and partial capture audits (2026-10-26)

`C:/Python310/python.exe -B ../../work/hl2-decompiled/firstmap-natural-progression-20261026bf/verify.py` passes 79 player snapshots,60 motion files/5976 samples,54574 raw reads/17597584 bytes,53 serialized requests, storage threshold(-3360.154,-29.031,-31.969), finalized demo/audio headers and native exit0/settings unchanged. It explicitly returns frida_trace_complete:false, fresh_never_attached_control:false, first_map_finished:false and rust_parity_tested:false. This validates bounded raw/route/cleanup integrity, not entire-map fidelity.

Independent BI checks the finite security suffix seq2558..4550:1993 events/930 closed pairs/53 complete sampled PauseThink groups,19 observed entry prefixes,28877 raw reads and2350 generation checks. Overall acceptance remains false for provider truncation/missing prefix. Two excluded boundary events and the failed broader captures remain unchanged. Three actual audio files are599.98/179.98/179.99s,stereo44100Hz; the third has one full-scale left sample. No dialogue identification, listening comparison or mix-fidelity claim follows. Demo806.8s/39542 gameframes is recording metadata, not an FPS guarantee.

`C:/Python310/python.exe -B ../../work/hl2-decompiled/scene-controller-consumption-20261026bh/verify_seal.py` passes73files/16new+15reused exact bodies/38anchors. `C:/Python310/python.exe -B ../../work/hl2-decompiled/firstmap-trace-audit-20261026bi/verify_inventory.py` passes23files/15dependencies without promoting partial traces. No Rust tests/build apply to these research documentation changes; document diff/whitelist/staged-set checks apply.

## Flashlight on the weapon (2026-10-25, macOS Apple Silicon)

Local checks: `scripts/checks/validate.sh` (1,103 Rust tests passed, 89 ignored; 3 Python
capture-reader tests; architecture, strict Clippy and formatting), and
`scripts/checks/check_windows.sh`. The new numerical tests cover translated/rotated
beam projection, narrower low-power FOV, near/far/attenuation and clearing off/dark data;
the weapon lighting basis preserves positions and normal lengths through view rotation.
The owned obstruction test rejects embedded camera positions and locates actual beam
placement that includes a weapon point. Owned checks:
`cargo test -p hl2-bevy flashlight --locked -- --include-ignored` (6 passed, 3 owned-install).

Reproduce packaged captures after `scripts/build-bevy.sh`:

```sh
NO_BUILD=1 ONLY='flashlight_viewmodel flashlight_shadows power_hud hud fov_settings coverage' python3 scripts/checks/replays.py
```

The weapon check compares normal pistol/SMG and portrait images with a disposable shader
control that suppresses only the viewmodel's flashlight contribution. It restores the
shader in `finally`. Comparisons exclude animated HUD pixels, require over 500 brighter
weapon pixels above threshold 8, and check the upper world region remains unchanged.
Off is compared with its control above the HUD. Final results: 18,157/11,021/35,187
brighter weapon pixels for pistol/SMG/portrait, zero changed upper-world pixels, zero off
weapon pixels above threshold 2. A separate capture of the preceding `cdaf4be` package
also matches the new world region exactly (1920x650, zero RGB difference), recorded in
`artifacts/flashlight-viewmodel/world-baseline-comparison.json`. Captures pin dimensions and HDR None;
reports assert selection, script completion and current posed bounds. Shader controls do
not modify the installation or package in `bin/`.

Rejected first comparison: changed pixels were in the ammo HUD, not on the weapon. A
coordinate diagnostic proved the open-corridor weapon was outside the actual cone. No
artificial brightness or altered beam was added. Two eye-height collision probes failed
their pull-back assertion: the beam starts lower and misses the bar the eye ray hit. A
beam-height probe then found a camera inside geometry and was also rejected. The final
probe checks camera clearance and actual projection before selecting the fixtures:
pistol eye -4324 -224 64, yaw 180/pitch 0; SMG eye -4304 -172 64, yaw 90/pitch 0.
Failures and the diagnostic image remain in `artifacts/flashlight-viewmodel`; the original
captures are retained under `rejected-open-view` instead of overwritten as successes.
Early compilation caught the new uniform missing from the sky material; the architecture
check rejected a new glob import. Both were fixed (explicit default/imports); failed
compilation remains in `tests-initial.log`, followed by passing `tests-fixed.log`.

Accepted groups: `flashlight_viewmodel`, `flashlight_shadows`, `power_hud`, `hud`,
`fov_settings` and `coverage`. The initial combined run failed the weapon proof; corrected
and final runs pass. Logs: `replays-initial.log`, `replays-corrected.log`, `replays-final.log`,
`validate-final.log`, `windows-final.log`, `owned-final.log` in `artifacts/flashlight-viewmodel`.
The final weapon run includes the world-region assertions and the shader with additive/unlit
exclusion guards; a dedicated firing/additive-effect capture was not performed.

Evidence: `artifacts/replays/flashlight_vm_*`, `artifacts/flashlight-viewmodel` logs and
reports. Native shader parity, weapon self-shadowing, world-depth-map sampling on the
weapon, Windows runtime and ordinary playthroughs are **not tested**. The exact installed
viewmodel draw wrapper was reviewed alongside Source SDK `game/client/viewrender.cpp`;
its projection/depth/list flow does not establish native illumination parity. Timing
windows and the correction to earlier 600-frame wording are in docs/performance.md.

## High flashlight shadows (2026-10-25, macOS Apple Silicon)

Passed: `scripts/checks/validate.sh` (1,101 Rust tests, 88 ignored; 3 Python image-reader
unit tests; architecture, strict Clippy and formatting), `scripts/checks/check_windows.sh`,
and `cargo test -p hl2-bevy flashlight --locked -- --include-ignored` (4 passed, including
2 owned-install checks). The owned cookie is black outside the cone; the map has real
cutouts with both transparent and opaque texels. Projection tests cover the narrowed
flicker frustum and pulled-back near plane; a dark battery-flicker sample skips the depth pass. Windows runtime rendering is **not tested**.

Packaged groups `hud`, `power_hud`, `flashlight_shadows` and `coverage` passed. Reproduce
with `NO_BUILD=1 ONLY='flashlight_shadows power_hud hud coverage' python3 scripts/checks/replays.py`
after `scripts/build-bevy.sh`; expected: all 4 groups pass. The shadow check pins 1920x1080
and HDR None, asserts High/on gating, checks Low/Medium/off, confirms current posed bounds,
and compares real captures with a dependency-free PNG decoder. That decoder is tested on
all five PNG filters and RGB/RGBA captures; it also matched an independent decoder byte
for byte on a game capture.

- High vs Medium darkens over 1,000 pixels above threshold 8 in both the posed-cop and
  fence views. Baked light is retained: the new shadow affects the added flashlight term.
- Negative control: the disposable copy's depth shader temporarily makes all cutouts solid;
  its colour shader stays unchanged. The normal shader lets light through the fence holes
  that the control blocks (171,663 brighter pixels above threshold 8 in the first accepted
  run). The depth shader is restored in `finally` before any following check.
- Final constant-tap shader: the fence capture has identical decoded RGB pixels to the accepted image;
  the posed-cop capture differs by at most one channel level (zero pixels above threshold 2).
- Source comparison at fixed exposure: the previous build's Medium fence view and the new
  Medium view differ by at most one level. The seeded cop comparison had 9 pixels above 2;
  no broad image-equivalence claim is made for animated/physics scenes.

Evidence: `artifacts/replays/flashlight_depth_*` and
`artifacts/flashlight-shadows/constant-tap-pixel-comparison.json`. The posed-cop fixture
seeds an actor for this test; it does not establish ordinary campaign playback. Native
side-by-side, exact generated noise/jitter, native slope bias, per-entity shadow-disable
inputs and viewmodel lighting are **not tested** here.

Rejected evidence is retained in `artifacts/flashlight-shadows`: initial captures restored
1280x720 settings when a video cvar was applied and had varying HDR exposure; two attempted
fence views hit solid walls. After those two attempts, an owned-mesh data check located
the real fence triangles before the next capture. The first combined replay invocation
also caught a fixture-registration edit accidentally added as call arguments (power HUD
TypeError and missing coverage); both affected checks passed after correction. Two Clippy
warnings in the new owned-data test were fixed with array chunks. The final shadow checks
were rerun after the shader change. Logs: `replays.log`, `replays-fixed.log`,
`replays-constant-taps.log`, `replays-complete.log`, `validate-complete.log`,
`windows-complete.log` and `owned-complete.log`. Frame-time evidence
and its limits are in `docs/performance.md`.

## Flashlight power HUD (2026-10-25, macOS Apple Silicon)

- `scripts/checks/validate.sh`: passes (1,099 tests passed, 87 owned-install/other ignored
  tests not run by this command), strict Clippy, formatting and the architecture check.
- `scripts/checks/check_windows.sh`: passes strict Windows-target Clippy; a Windows
  executable and Windows runtime are **not tested**.
- `cargo test -p hl2-ui hud::power --locked -- --include-ignored`: 5 passed, including
  2 owned-install tests discovered with `source_assets::install::discover()`. Coverage:
  chunk rounding, once-per-transition events, shared animation colour/fade/geometry,
  suit/death hiding, portrait resizing and Episode Two's battery separation.
- `NO_BUILD=1 ONLY='power_hud hud hud_aspect console options coverage' python3 scripts/checks/replays.py`:
  6 check groups passed, zero failed. The new `power_hud` check runs 3 packaged captures:
  legacy on (charge/device/colour and one-device geometry), off/recharged (100 percent,
  no device and fully faded), and Episode Two (battery in use, suit power still 100).
- Inspected `artifacts/replays/power_hud_on.png`, `power_hud_recharged.png` and
  `power_hud_ep2.png`: the localized auxiliary label, bar and device line appear above
  health, disappear after recharging, and Episode Two instead has the flashlight icon
  and its own bar. The Episode Two image validates the HUD only; magenta world surfaces
  are visible and their cause/regression status was not investigated in this step.
- The release package was built with `scripts/build-bevy.sh`. All games ran in the
  background and muted from copies with separate settings. No ordinary playthrough,
  native side-by-side or High flashlight depth shadow validation was performed.

Rejected performance launches remain in `artifacts/power-hud`: sandbox runs could not
find the Metal GPU; the first GPU baseline rejected an unfinished 900-tick script at its
600-frame bound. Accepted timing runs use a completed 240-tick script and GPU access.
Timings: `docs/performance.md` and MODLOG.

## Corrected probes and retained failures (2026-10-24)

Read-only verify_seal.py passes new private AT24, AU581, AV27, AW46, AX615, AY9, AZ24, BA26 and BB18 retained-file sets. AU acceptance means intact failed-diagnostic evidence: an owned crash before a window collision, two retained failed reader occurrences, unchanged settings and session closed after exit. It is not damage validation. AW checks a compiled x86 prefix, three sentinel controls and two instrumentation lifecycles. AY checks five loaded-script receipts/29 sites across 18 roots and identifies four incorrect interior call listeners; old roots remain unchanged.

AX checks 48 events/five actual calculator/lookup/selection/damage chains, 67 motion files/6804 samples, 518 trajectory samples, 90744 raw-read occurrences/27431410 bytes and native images. The 1769 legacy named-snapshot reads without per-read hashes remain file/seal-hash protected. Actual table/scale/lookup inputs match reconstruction and health stays 15. The original guard-snapshot overwrite is explicit; independent creation/module/prefix/storage identities remain. Both corrected observers unload/close before native/child exit 0 and unchanged settings.

BA checks 30 events/10 parser pairs/eight sites/192 raw reads/3400 bytes and ordered echo output. Its base-entity snapshots prove unchanged position but omit buttons. AZ checks three exact BLOBs/535 rooted bytes/137 instruction starts and 18 post-vector expression matches. BB checks two reused BLOBs/880 rooted server bytes/273 starts, one missing getter retained as four raw bytes, five caller-site witnesses and 10 entity joins. No full solver, complete ABI, ordinary campaign or Rust parity follows.

Private verify-research-index-20261024bb.py checks 27 roots/5411 file occurrences, 36 exact dependencies against immutable pass14 BLOBs and 19 metadata query cases, including wrong identities and missing-body/no-nearby fallbacks. The first-map audit preserves 16 gap statuses/four missing scene-index entries and first_map_finished:false. Documentation/whitelist checks apply; no Rust build, gameplay rebuild or Rust tests were run. Current checkpoint: work/publishing/research-followup-continuation-20261024bb.md.

## Aligned release and raw-entry acceptance (2026-10-23)

Read-only verify_seal.py passes private AL618, AN18, AO12, AP15 and AS31 retained-file sets. AL checks65 motion files/6431 samples plus517 trajectory samples,90804 reads/28246816 bytes, observed held/support identities, two unchanged-health throws, images and owned closure/settings. Rejected masked-summary/read-selection drafts remain retained; raw capture bytes are unchanged. AN verifies two retrieval failures, exact target prefix and owned worker closure. AO/AP independently traverse793 exact bytes/253 instruction starts rooted at the four missing entry targets, recheck descriptor/string/pointer stores and eight reused exact body occurrences; no indexed/Ghidra body recovery is asserted. AS compares three exact BLOBs, four missing identities, selected slots and seven rooted accessor graphs173 bytes/60 instructions.

AR verify_seal.py passes606 retained files:25 trace events/four collision pairs/four computed damage0 results/two application pairs,65 motion files/6928 samples and518 trajectory samples. Selected verification scope checks96258 raw-read occurrences/28424876 bytes, including repeated observations in receipts;1769 legacy named-snapshot reads lack per-read hashes and remain file/seal-hash protected. Actual preflight prefix reads, event/core fields, force vector, native images, script unload/detach and owned closure/settings are checked. No fresh crossing/exit, full damage equation, ordinary first-map completion, typed ABI or Rust parity is established. No Rust tests/builds apply. Private checkpoint: research-followup-continuation-20261023ar.md.

## Storage negative result, live models and queue-node acceptance (2026-10-23)

Read-only `python -B ../../work/hl2-decompiled/scene-storage-control-20261023ai/verify_seal.py`, `python -B ../../work/hl2-decompiled/scene-firstmap-live-models-20261023aj/verify_seal.py` and `python -B ../../work/hl2-decompiled/scene-command-queue-20261023am/verify_seal.py` pass closed file sets538/9/15. AI checks 6310 motion samples/71804 reads/22688712 bytes, ground and attached-handle generations, two preserved health15 impacts, native image identities and owned closure/settings. It accepts intact evidence of failure, not a successful crossing.

AJ checks 1721 raw reads/136020 bytes and independently maps two live RTTI/model witnesses to retail PE bytes; seven literal names are not observed and full scene/model gap closure is false. AM compares one exact pass14 BLOB, complete215-byte body and72 instruction starts against private PE bytes, retaining the rejected padding overrun. No runtime queue execution or fixed timing guarantee. AL/AN subsequently completed as bounded negative results above; no Rust build/tests or parity validation.

## Per-line reset and fresh ordering controls (2026-10-23)

Read-only python -B ../../work/hl2-decompiled/scene-command-buffer-20261023af/verify_seal.py and python -B ../../work/hl2-decompiled/scene-command-buffer-control-20261023ag/verify_seal.py pass closed file sets 99/93. AF independently checks 48 events/19 complete call pairs/eight wait sites, raw buffer/frame/clock/source fields, one exact 106-byte caller and its static global-buffer binding. Its 662 passive samples/6409 reads exclude 15 clock races. Prefix equality is attested by hashed observer source plus successful ready, with the missing separate prefix-read transcript explicitly limited.

AG checks 400 samples/4151 reads/1200920 source bytes/five captures; 12 clock-raced samples excluded. All three fresh controls show 0->2->6->0 buttons. First-observed tick deltas are 9/3/5 for wait12/wait6/wait12, bounded 7..10/2..4/4..7. This corroborates ordering, not fixed wait units. Both games closed/settings unchanged; AF script unloaded/session detached. Setup and ledger failures remain retained. Fresh route AI is not yet an accepted exit; no Rust tests/builds or Rust parity validation.

## Input acknowledgement and conditional wait acceptance (2026-10-23)

Run python -B ../../work/hl2-decompiled/scene-input-ack-20261023ac/verify_seal.py and python -B ../../work/hl2-decompiled/scene-command-wait-20261023ad/verify_seal.py. Both pass closed file sets of 94/25 files. AC independently checks raw descriptor m_nButtons/type5/+a80, 593 samples/5488 reads/1754824 bytes, process/module/entity-generation/clock brackets, inputs/markers, five PNG/TGA pixel identities and cleanup. Nine raced samples are excluded. Separate requested 50 ms input yields first observed 25 ticks/0.375 seconds; bounds 0.360..0.480. Two batches fail to expose ordered phases despite ordered markers and selected maximum acquisition gaps below 17 ms. Acceptance validates the evidence, not successful batching.

AD verifies one exact retail parser plus one comparison helper against immutable pass14 BLOBs and exact PE bytes: 514 body bytes/216 instruction starts/219 retained byte records. Helper padding is excluded; static wait recognition/flag/scalar dataflow is established. Runtime instance/flag/path, units/defaults and ABI remain unverified. Owned PID 31504 closed with six checked installed settings identities unchanged; REA session closed. No campaign route replay, Rust builds/tests or Rust parity evidence. AB remains stopped/unsealed and unchanged. Current continuation is private research-input-ack-continuation-20261023ac.md.

## Owner-requested research stop (2026-10-23)

python -B ../../work/publishing/verify-native-exit-index-20261023w.py passes five roots/1868 retained files/22 query checks with original 16 gaps preserved. Private AB experiment.py passes AST parsing only; phase trials did not run. session-d1_trainstation_01-20261023T105448Z/owner-stop.json binds PID70548 and creation/module identities; cleanup.json confirms closure and unchanged checked installed settings. driver-close.json records interrupted child exit1 at request6 with zero pulses. Stop-related reader/acknowledgement errors retained; AB is unsealed. No Rust tests/builds or Rust parity validation performed.

## Native interaction and exit acceptance (2026-10-23)

Private python -B scene-shadow-control-20261023w/verify_seal.py passes 28 files/11 external bindings, three new/three reused identities and selected return/call/table byte checks . scene-storage-interaction-20261023x/verify_seal.py passes 694 files/38 bindings, 21 exact identities, 20 hook entries/one interior site, 3197 main events/1543 closed pairs, seven collision pairs and 54 exit pairs.371 storage samples/5889 motion samples retain source transcripts; these bracketed reads are not atomic frames or a timing-fidelity proof.

Accepted native observations: window 15/15/profile strings, same-generation pickup/carry, three throws, computed damage10/75, health 15->5->-70, Break/Die/removal, crouched crossing and actual destination status/player/map-time identity. The failed first impact, intermediate nonthrow shutdown, stopped jamb approaches, watcher parent timeout and fixed-player transition rejection remain retained. The large watcher file independently parses120 complete samples despite its parent's failed status. Both installed module identities and checked configuration identities remain bounded to the recorded build/settings.

Fresh non-Frida Y acceptance passes 498 files, 58 motion captures/5142 samples, 11 storage captures and 62563 source reads. Z passes 631 files, 68 motion captures/6047 samples, 17 storage captures and 79294 source reads; sampled launch-to-crouch delay is0.80999755859375 game seconds, after the peak, with invalid initial lateral clearance. Both validate partial carry/window-removal evidence, **not** passage/transfer. Failed staged/placement approaches remain explicit. All owned games closed/settings matched.

AA seal passes 17 files/21 bindings, six reused exact identities, 126 byte records/four slots, 12 mass-set pairs and five damping phases. The five-root navigator/audit passes 1868 files/27 distinct exact dependencies/33 per-root occurrences and 22 CLI cases, preserving all 16 original gap statuses. Separate metadata verification confirms the exact engine identity/13824 completed exports/two negative patterns without retrieving a body or proving retail wait semantics. No Rust implementation/build/test/parity result follows; first-map full semantic/audiovisual completion remains false. Current private handoff and final publication receipt are research-native-exit-continuation-20261023w.md and research-native-exit-closeout-20261023w.json.

## Local check scripts (2026-10-17)

`scripts/checks/architecture.py` (2026-10-24; Python 3, standard library, about 4 s) enforces the
architecture guardrails described in AGENTS.md: function, file and test-file size limits with
`scripts/checks/architecture-baseline.json` holding the existing offenders (they may only shrink),
the crate layers and engine-crate owners, no file IO in modkit-core / hl2-simulation / source-ui,
no HL2 names in source-ui, no new duplicated 8-line blocks, and per-crate counts of mutable
statics, `too_many_arguments` and `dead_code` allowances and glob imports. validate.sh, CI and the
local pre-commit hook run it first.

`scripts/checks/validate.sh` (the architecture check, `cargo test --workspace --locked`, strict clippy
`--workspace --all-targets --locked -D warnings` as CI, `cargo fmt --check`) and
`scripts/checks/check_windows.sh` (strict clippy of hl2-ui/hl2-bevy for `x86_64-pc-windows-msvc`
without linking, from macOS/Linux) are bash.

The packaged checks are one cross-platform Python script, `scripts/checks/replays.py` (Python
3.9+, standard library only; `python3 scripts/checks/replays.py`, or `py` / `python` on Windows).
It builds once (`scripts/build-bevy.sh`, `scripts/build-bevy.ps1` on Windows), copies the packaged
game to `artifacts/replays/game/`, runs each check as bounded launches of that copy with their own
`--report`, and asserts on what the reports record. One ok/FAIL line per check; logs, reports and
captures in `artifacts/replays/`. The copy keeps its own settings next to it, so every check starts
from default settings and `bin/hl2-config.json` (and a game running from `bin/`) is never touched.
Environment instead of arguments: `ONLY="console movement"` runs those checks, `GROUP=ui` or
`GROUP=gameplay` one group, `JOBS=4` up to four gameplay replays at once (never more games than fit in
physical memory: `GAME_MEMORY_GB` per game, default 6, plus 4 GB for the system; a packaged game
measured 4.9-5.8 GB on the station maps on 2026-10-23, so a 16 GB machine runs two; the
`parallel_jobs` check covers the rule), `WINDOWED=1` adds the
windowed ones of this platform, `NO_BUILD=1` keeps `bin/` as it is, `UPDATE_EXPECTED=1` records the
gameplay outcomes (below).

UI group: the labelled UI states (console mode and output, the Options dialog's tab / focused
control / pending changes / sub-dialog), loaded maps, saved settings and captures.

Gameplay group: every other `test-inputs/*.json` fixture, one check per fixture (named after it
without `bevy-`), on its map (`d1_trainstation_01` for the security-room scenes,
`d1_trainstation_04` for player death and hurt, `d1_trainstation_02` otherwise), 1280x720, in
three layers:

1. Every fixture: every label sampled at its authored tick, the capture written at the reported
   size, drawn entities and effects agree with the simulation, entity I/O within its budget, every
   fixture action found its entity, models, collision models, effect sprites and sounds load. Known
   installation gaps are listed in the script: sounds `d1_trainstation_01` references that the
   installation lacks, collision models the loader does not support yet, and a body impact sound
   variant that is not preloaded (three pallet fixtures; main shows it too).
2. Specific assertions: `movement` (bench jumps, air duck, pause, held jump), `entities-weapons`
   (doors block and open, E use, pistol / SMG grenade / AR2 ball ammo), `door-swing` and
   `door-inputs` (both leaves swing away from the opener, `OpenAwayFrom`), the five `effect-*`
   (grenade, ball, decal, explosion, decal on a door), the three `particles-*` (emitter counts and
   lifetimes) and `attention` (Barney's LOOKAT interests through scene pause, UI pause, overlap,
   cancel and tail).
3. Expected outcomes: each labelled sample's I/O counters, inventory, active scenes, gesture and
   look targets, projectile, gib and impact counters, the player's feet (within 1 unit), state and
   clock, console mode and forward ray, compared with `test-inputs/expected/<fixture>.json`. Two
   fields that vary between runs of the same build are left out for two pallet fixtures, and
   `occluded_blast_targets` everywhere. A difference is either a regression or an intended change;
   after reviewing it, `UPDATE_EXPECTED=1` records the new outcomes (review the diff of the expected
   files). The outcomes were recorded on macOS; physics on another platform may differ slightly.

`coverage` fails when a fixture is neither used by a UI check nor listed with its map, so a new
fixture is never silently left out.

| Check | Category | What it asserts |
| --- | --- | --- |
| `submenus` | shared | `test-inputs/bevy-menu-submenus.json`: Episode One, back, Half-Life 2, back, Extras > Lost Coast load `hl2/background01`, `episodic/ep1_background01`, `hl2/background01`, `lostcoast/background01`, ending in the main menu |
| `return_menu` | shared | `bevy-menu-return.json`: Episode Two menu, `map d1_trainstation_02`, Return to main menu: back over `hl2/background01`, which is saved |
| `chapter_menu` | shared | `bevy-menu-chapter-return.json`: `d2_coast_01` returns to `hl2/background04`, `d3_c17_10a` to `hl2/background06` |
| `menu_options` | shared | `bevy-menu-options-nonmodal.json`: with Options open, a click on a menu row beside it runs the row (Episode One) and focuses the menu while Options stays open; Options dragged over the menu column; NEW GAME clicked under it does not start a game, Options keeps the focus (report `console.windows`); `bevy-menu-options-list.json`: with the Video > Resolution list open, a click on the menu only closes the list |
| `video_8k` | shared | `bevy-menu-video-8k.json` with `--fake-monitor 7680x4320,3840x2160,2560x1440,1920x1080,1280x720`: Options > Video lists the 16:9 resolutions of an 8K panel from 7680 x 4320 down (report `console.options.resolutions`); the capture shows the Resolution list open |
| `video_portrait` | shared | `bevy-menu-video-portrait.json` with `--fake-monitor 1080x1920` at 1080 x 1920: by keyboard (Tab, Right, Alt+Down) Video > Aspect Ratio opens with Portrait 9:16 selected; the Resolution choices are the 9:16 modes up to the panel; frame and capture are 1080 x 1920. | The Options dialog lies inside the frame (report `options.frame`).
| `video_keep` | shared | `bevy-menu-video-keep-{timeout,ok,escape}.json` with `--fake-monitor 1920x1080`: Normal (4:3) picked and applied by keyboard renders 1440 x 1080 at once and opens the keep box (report `console.confirm.kind` = `keep_video`); the timeout and Escape put back 1280 x 720 (rendered and saved); OK keeps 1440 x 1080 past the timeout |
| `narrow_layout` | shared | `bevy-console-open.json` at 1080 x 1920, 1080 x 1080 and 1920 x 1080 (report `console.menu`, `console.console_frame`): menu rows fit the width; on the narrow sizes the text is not shrunk with the width and the console spans the width inside the design area; on 1920 x 1080 the console frame is the retail one (half the screen wide, origin `(w/2 - 16, 64)` units) | It also runs 3440 x 1440 and 5120 x 1440: the menu grows relative to the height and its row stack stays inside the screen, the console is the retail one, and on every size (menu backgrounds) the world camera keeps the 4:3 vertical view (59.84 degrees at FOV 75).
| `fov_settings` | shared | `bevy-hud.json` on `d1_trainstation_02` with the settings file preset (report sample `camera`): default 1920 x 1080 gives 59.84 degrees vertical for the world and 41.83 for the viewmodel; `vertical_fov` 90 and `viewmodel_fov` 70 give 90 and 55.41; at 1080 x 1920 the automatic world view is 89.16 (aspect-tuned Vert+) and viewmodel 84.34 (its 4:3 width kept; the portrait profiles move it, `fov_offset` 0); at 5120 x 1440 the automatic world view is 50.22 (116.19 horizontal) and viewmodel 41.83 (vertical view kept) |
| `hud_aspect` | shared | `bevy-hud.json` on `d1_trainstation_02` at 3440 x 1440 with each `video.hud_aspect` (report `hud.frame`, `hud.panels`): Fit spans the screen (health near the left edge, ammo near the right); Centered 4:3, 16:9 and 16:10 give a centered frame of that aspect with the panels inside it at their frame-relative distances; at 1920 x 1080 Centered 16:9 equals Fit (frame, panels, quad count) | Captures `hud_aspect_*.png` inspected by eye; panel rectangles are the layout at rest, not the sliding ammo animation. |
| `resolution_stress` | shared | `bevy-menu-resolution-stress.json` with the faked 8K monitor: Options > Video picks 7680 x 4320 from the list and Apply renders it; `mat_setvideomode` to the smallest (320 x 240) and largest (8192 x 8192) sizes the console accepts, and four changes on consecutive ticks mixing full screen and windowed (the last, 1280 x 720, wins); each labelled sample's `frame` (size of the frame image being rendered) is checked, host ticks keep advancing, the saved settings and the capture end at 1024 x 768 |
| `chapter_quit` | shared | `--map d2_coast_01` quit mid-game: the saved background goes from `hl2/background01` to `hl2/background04` |
| `options` | shared | every `bevy-options-capture*.json`: Keyboard / Video Advanced sub-dialogs open, the dialog drags, the console states; the `-keyboard-*` replays reach their control by keyboard only (focused control, tab and drawn focus from the report; the text entry edits; the FOV slider in Video > Advanced; the 3rd party credits' Close holds the focus and Enter closes them on release) |
| `placement` | shared | a dragged Options dialog is saved and kept when it reopens in a second launch |
| `captions` | shared | `episodic/ep1_background02`: closed captions on process speech, off process none |
| `bg_compare` | shared | each game's chapter 1 background loads (FOV 75, 1024x768 captures for comparison with the installed pre-blurred `materials/console/*.vtf`) |
| `save_config` | shared | `bevy-save-config.json`: `volume`, `sensitivity`, `fov`, `mat_hdr_level` from the console are saved |
| `console` | shared | `bevy-console.json`: cheats refused then granted, quoted `;` kept, the console freezes the scene, `setpos` / queued `ent_fire`, Unicode input and editing, closing the console returns to the pause menu and Escape resumes, weapon selection cancelled by Escape before pausing, pause menu, outside click, console over the pause menu and back, unknown commands rejected, sounds pause and resume with the UI; the same replay at 1920x1080 (viewport and capture size) |
| `console_map` | shared | `bevy-console-map.json`: two `map` loads, a missing map reported once without losing the player or scene, console state kept across loads |
| `campaign` | shared | `bevy-campaign.json`: `d1_trainstation_01` > 02 > 03 through their triggers, inventory kept, the clock restarts per map |
| `dev_overlay` | shared | `bevy-dev-overlay.json`: F1 overlay visible with its five lines and live statistics |
| `hud` | shared | `bevy-hud.json`: unarmed start, loadout and weapon slots, HUD drawn |
| `coverage` | shared, gameplay | every `test-inputs` fixture is run by a check |
| one per gameplay fixture | shared, gameplay | the three layers above |
| `window` | windowed | `bevy-menu-resolution-change.json`: a console resolution change resizes the frame and Options still opens (window captured, `--capture-window`); `bevy-options-keyboard-apply-resolution.json`: Video > Resolution changed and Apply pressed by keyboard only changes the saved resolution and the window; `bevy-menu-resolution-stress-windowed.json`: windowed-only changes on a real window (1920 x 1080, 320 x 240, four on consecutive ticks, 8192 x 8192, 1024 x 768) each reach the rendered frame and the game keeps running (no exclusive full screen, so the display mode of the desktop is never changed) |
| `window_place` | macOS | windowed at 2560x1600 and 1024x768: sized and centred in the screen's visible frame, below the menu bar and above the Dock |
| `no_border` | Windows | `--borderless`: "Windowed (No Border)" is a borderless window over the monitor (on macOS the same mode is the native full-screen Space, which a replay cannot drive). Not run on this machine |

Writing replays: Escape clears the main menu selection, so Down then selects row 0; keyboard Down
skips spacers; scripted `pointer` positions are UI canvas pixels of the render resolution (pin
`--width 1024 --height 768`: Keyboard tab (389, 215), Advanced... (464, 537)); script ticks keep
running while Options is open; end a replay with a settle step so the capture follows a presented
frame. `bevy-console.json` has one step more than upstream's (tick 17 Escape, `resumed-from-pause`,
because closing the console returns to the pause menu here); its `menu-resume` step presses Enter
with no pause-menu row selected, which no longer resumes, so its assertions are left out.

Shared checks run in the background (`--background`: no window, nothing takes focus, captures
read the game frame); windowed ones open a window and take focus. On macOS a windowed run becomes
the frontmost app at launch (winit activates the app; Bevy offers no switch); on Windows `--no-focus`
shows the window without activating it (winit `SW_SHOWNOACTIVATE`; not run here).
Every check runs with `--mute` (since 2026-10-23, replacing `--volume 0.01`, which a settings
change such as the console `volume` reset to the saved value): sounds start and are reported, but
every sink stays at zero volume, whatever the Audio settings say, and nothing is saved. Each run's
report must show `audio.muted` true and `audio.peak_output` 0 (the loudest volume given to any
sink), or the check fails.

Verified 2026-10-17 on macOS (Apple Silicon): all 85 checks ok (14 UI, `coverage`, 70 gameplay
fixtures) in 616 s with `JOBS=4`; the outcomes were recorded with `JOBS=4` and reproduced by a
serial run (1227 s; one forward-ray difference on a tipping pallet, now left out) and a second
parallel run. `window` and `window_place` ok earlier the same day (not rerun after this change).
Not run on Windows or Linux.

## Closed captions (2026-10-17, macOS Apple Silicon)

`scripts/checks/validate.sh` (hl2-ui 233 passed / 7 ignored, hl2-bevy 86 / 8, strict clippy `--workspace --all-targets -D warnings`, fmt) and `scripts/checks/check_windows.sh` (strict clippy of hl2-ui/hl2-bevy for `x86_64-pc-windows-msvc`) pass; `cargo test --locked -p source-assets -p hl2-simulation`: 103 / 30 and 241 / 18. Owned-install tests (`--include-ignored`, installation discovery): the Episode One dictionaries load in search order (episodic 3153, hl2 3970 entries), every `kleinercast01..13.vcd` line resolves its caption, and the Episode One `HudCloseCaption` layout, `Panel.BgColor` and four caption faces load (DejaVu Sans substitutes for the uninstalled Tahoma). A dictionary check found every installed `.txt` token hashed in its `.dat` (ep2: 15 differing values). Packaged release (`scripts/checks/replays.py` (`captions`): `episodic/ep1_background02`, closed captions, 1024x768 windowed, 420 frames) exits 0; `artifacts/captions_capture.png` shows `ep_01.kl_fellowcitizens06` in Kleiner's `255,255,200` italics over the translucent rounded box behind the main menu; report `hud.captions`: dictionaries [3153, 3970], 1 processed, nothing missing, no attenuated scene captions. The same run with captions off (`captions_off.png`) shows no panel and 0 processed captions. Not compared with a retail capture; Subtitles mode, captions during gameplay and Windows **not tested** in a packaged run.

## Merge of main into the menu branch (2026-10-16, macOS Apple Silicon)

`cargo test --locked --workspace`: hl2-bevy 83 passed / 8 ignored, hl2-simulation 236 / 18, hl2-ui 193 / 5, modkit-core 58, source-assets 99 / 28 (including main's impact-table test). Owned installation (`--release -- --ignored`): hl2-bevy 8, hl2-simulation 18, hl2-ui 5 with installation discovery; source-assets 28 with `HL2_ROOT`. `cargo clippy --workspace --all-targets --locked -- -D warnings` (also for `x86_64-pc-windows-msvc`, hl2-ui/hl2-bevy) and `cargo fmt --check` clean. Packaged release: a bounded 300-frame windowed main-menu run exits 0; submenu replay (Episode One, back, Half-Life 2, back, Extras, Lost Coast) loads hl2/background01, episodic/ep1_background01, hl2/background01, lostcoast/background01 with no failures; Return to main menu from `d1_trainstation_02` comes back over hl2/background01. An intermittent Bevy "Entity despawned" command warning during the submenu replay (about one run in four) also occurs on the pre-merge build. The setup window was not opened.

## GameUI settings used by the game (2026-10-11, macOS Apple Silicon)

`cargo test --locked -p hl2-ui -p hl2-bevy -p hl2-simulation -p source-assets` (hl2-ui 85 passed / 3 ignored, hl2-bevy 61 / 2, hl2-simulation 169 / 14, source-assets 90 / 17), including binding parsing for every storable name and default, mouse/wheel/unbound bindings, mouse filter and acceleration, migration of missing defaults, skill rescaling, dry-mix music detection and the startup display mode. `cargo test --locked -p hl2-ui -- --ignored`: 3 owned-install tests pass. Strict Clippy (hl2-ui, hl2-bevy, source-assets, hl2-simulation) and `cargo fmt --all --check` clean. Packaged release: Keyboard-page capture (`artifacts/gameui/m6-options-keyboard.png`) with a config saved before the attack defaults shows MOUSE1/MOUSE2 added by migration and the stored Duck = CTRL kept; `--map d1_trainstation_01 --frames 120 --capture` starts windowed and completes. Live keyboard/mouse play in the packaged build (movement scripts bypass device input), audible music/mute checks, full-screen/No Border startup and Windows/Linux **not tested**. Details: [gameui.md](gameui.md) section 8.

## GameUI Options dialog (2026-10-11, macOS Apple Silicon)

`cargo test --locked -p hl2-ui -p hl2-bevy` (hl2-ui 83 passed / 3 ignored, hl2-bevy 56 passed / 2 ignored); `cargo test --locked -p hl2-ui -- --ignored` passes all 3 owned-install tests (the installation is discovered like the runtime does; the older HUD test no longer requires `HL2_ROOT`), including `gameui::options::tests::owned_install_pages_bind_every_expected_control`. Strict Clippy for hl2-ui, hl2-bevy and source-assets and `cargo fmt --all --check` clean. Packaged release with `--map background01 --main-menu --width 1280 --height 720 --no-focus --movement-script artifacts/gameui/options-<page>.json`: Gameplay, Keyboard (sections from `kb_act.lst`, MOUSE1/MOUSE2 on the attack rows with a fresh config), Mouse, Audio and Video (open display-mode list: Full screen / Windowed on macOS) captured and visually checked. Brightness dialog: unit tests only (its button is disabled in the windowed capture, as in retail). Windows/Linux, fullscreen switching and packaged key capture **not tested**. Details: [gameui.md](gameui.md) section 6.

## GameUI startup flow and main menu (2026-10-11, macOS Apple Silicon)

`cargo test --locked -p hl2-ui -p hl2-bevy` (hl2-ui 69 passed / 2 ignored, hl2-bevy 54 passed / 2 ignored); strict Clippy for hl2-ui, hl2-bevy and source-assets and `cargo fmt --all --check` clean. Packaged release (`scripts/build-bevy.sh`): no `--map` -> background01 + main menu (report mode MainMenu, background_level true); `--map no_such_map` -> failure message then the menu; `--map d1_trainstation_01` -> gameplay; `--fly --position 0 0 0` and `--map no_such_map --fly` -> exit status 1 with an error; scripted input on `--map background01 --main-menu` captured the QUIT GAME box and the HALF-LIFE 2 category. Captures are in ignored `artifacts/gameui/`. Windows/Linux **not tested**. Details: [gameui.md](gameui.md) section 5.

## Offline research final publication boundary (2026-10-22)

Final private closeout checks scoped documentation-only commits, unchanged whitelist, published HEAD against the actual branch remote, empty staged index/clean tree and private handoff/navigator/seal hashes. See research-pickup-dispatch-closeout-20261022j.json. Root and navigator acceptance commands above establish static provenance; publication checks establish Git hygiene only. No Rust tests/builds, native launches/captures, actual motion/break/exit or Rust parity were tested.

## Callback writer and navigator acceptance (2026-10-22)

Run private python -B scene-grab-error-writer-20261022v/verify_seal.py for24 retained files/21 externals, one new/four reused exact bodies,117 byte records and selected class/registration/field spans. Run publishing/verify-pickup-dispatch-index-20261022j.py for13 roots/57 distinct bodies/63 per-root identity occurrences/64 source paths/five navigation-only occurrences. Builder gated every selected root's seal; independent navigator acceptance rechecks closed sets, exact DB BLOBs, source hashes, full pagination, reused/resource-only filters and rejected arguments. Do not rerun producers into frozen roots.

These static/publication checks do not establish actual callback/input ordering, trace results, physics law/units, carry/throw, window break/exit, ordinary campaign completion or Rust parity. No game/process/Frida/UI/input actions or Rust builds/tests occurred. The owner's running game remained undisturbed.

## Target search and error-consumer static acceptance (2026-10-22)

Run private python -B verify_seal.py under scene-player-use-target-20261022t and scene-grab-error-20261022u. Expected32/21 and21/18 retained-file/external-binding counts. T rechecks selected slots/caps,1123 byte records, seven constants plus inline geometry, qword cone threshold, argument/occlusion sites, exact bodies and predecessor seals. U rechecks94 records, six constants, formula/reset/sentinel/null-path opcodes, both caller spans, exact bodies and source/provenance. All failures/rejected interpretations remain private with the successes.

These checks establish static paths and state mutation, not runtime call ordering, actual target/LOS/pickup, measured error/motion, validated units/ABI, ordinary play, break/exit or Rust parity. No gameplay build/tests/captures were run. Source-only documentation publication checks apply.

## Scale-key and grab-target static acceptance (2026-10-22)

Run private python -B verify_seal.py under scene-crate-scale-fields-20261022r and scene-grab-update-20261022s. Expected53/17 and24/16 retained-file/external-binding counts. R independently checks descriptor bytes, key flag/type dispatch/name comparison/float destination, exact immutable DB bodies, owned BSP spellings and source references. S checks806 records, result handling, target-field stores, branch/call edges, constants and full exact-body identities. Historical unsealed acceptance labels describe pre-parent review; the enclosing closed-set seal is authoritative for freeze status.

Actual native assignment/locale/conversion, both error readings, target movement/attachment, timing/units, carry/release/throw, window break/exit and Rust parity are **not tested**. No gameplay rebuild is needed for these research/documentation changes. Reviewed five-doc scope/whitelist checks and commit/push verification apply.

## Input/frame/resource/spawn acceptance (2026-10-22)

Run private verify_seal.py under scene-use-input-20261022n, scene-held-use-frame-20261022o, scene-crate-authored-physics-20261022p and scene-crate-physics-spawn-20261022q with C:/Python310/python.exe -B. Expected retained-file/binding counts22/13,33/22,20/16,22/15. Native raw checks retain selected slots, calls, arguments, constants, full DB identities and pinned references. Resource acceptance independently compares exact selected VPK/member/header ranges, CRC/SHA, ordered prefix/full-tail data and14 property witnesses. Whole carrier identity and full damaged-tail grammar agreement are explicitly unavailable.

Current native mass/settings/units, successful physics creation, input/frame cadence, actual attachment/motion/damage/break/exit and Rust parity are **not tested**. Zero authored drag-named fields does not mean zero drag. No gameplay rebuild or Rust tests for these offline/documentation changes; source-only scope and publication checks apply.

## List/damage/use static acceptance (2026-10-22)

Run each private verify_seal.py under scene-crate-physics-list-20261022k, scene-window-damage-dispatch-20261022l and scene-player-use-dispatch-20261022m with C:/Python310/python.exe -B. Expected retained-file/binding counts:18/21,43/41,32/18. Their independent checks retain full DB identities/body hashes, exact PE instructions/slots/call edges and pinned source/external hashes; M separately reconstructs six historical raw descriptors and distinguishes two selected methods from two navigation-only bodies.

Instruction counts25/1428/264 include bounded padding/neighbors and do not establish whole CFG/ABI coverage. Worker pre-seal reports remain historical; only closed-set seals attest current integrity. M's first verifier stopped on an invalid codec spelling and passed after correction without relaxed checks. No native launch/inspection/input/Frida or Rust tests/builds occurred. Actual list stability, damage/break/output execution, physical key/tick behavior, exit and Rust parity are **not tested**.

## Saved-state capture acceptance (2026-10-22)

Run C:/Python310/python.exe -B ../../work/hl2-decompiled/scene-crate-attach-20261022j/verify_seal.py. Expected passed:true,17 retained files/13 bindings,two exact bodies/646 bounded records. The verifier checks immutable complete identities/bodies, caller flags and callback-before-attach order, exact mass/damping accesses/setters, count-factor constants, pinned SDK excerpts and matching detach offsets; the seal checks closed files and receipt. Windows can include padding/neighbors and do not establish full ABI/CFG coverage.

A failed predecessor .c read is retained; its exact caller JSON was freshly retrieved. Saved live values, native list contract/stability, dynamic mass/damping effects/units, carry/throw/window/exit and Rust parity are **not tested**. Documentation/whitelist checks are appropriate; no gameplay build or Rust tests for this research-only milestone.

## Offline handoff and detach acceptance (2026-10-22)

Run `C:/Python310/python.exe -B ../../work/hl2-decompiled/scene-crate-detach-20261022i/verify_seal.py`: expected passed:true,20 retained files/12 bindings/three exact bodies/370 bounded instruction records. It checks immutable full-identity DB bodies, file-backed selected calls/branches/constants, pinned source excerpts and the closed file set/receipt. The original five root seals were also checked again, and `../../work/publishing/verify-window-exit-index-20261022d.py` still checks66 distinct bodies/69 occurrences/five missing startup stubs. The late detach root is separate from that inventory.

These are offline provenance/static checks. The initial short clear-helper listing ends inside an instruction and remains retained beside an extended loop/return window; only complete cited sites are promoted. No runtime game, gameplay rebuild, Rust tests or ordinary exit test occurred. The final private closeout checks reviewed documentation commit scope, actual remote identity and worktree/index state; any concurrent contributor changes must remain untouched and be reported accurately. Native live restoration/velocity/health/collision/break/exit and Rust parity are **not tested**.

## Throw/impact and continuation-index acceptance (2026-10-22)

Run `C:/Python310/python.exe -B ../../work/hl2-decompiled/scene-crate-throw-20261022g/verify_seal.py`, the sibling `scene-window-impact-20261022h/verify_seal.py`, and `C:/Python310/python.exe -B ../../work/publishing/verify-window-exit-index-20261022d.py`. Expected seals:54 files/13 bindings/13 bodies/914 records;25 files/nine bindings/five bodies/463 records. The index checks five pinned manifests/receipts, exact source identities and all66 distinct pass14 body hashes/69 occurrences, plus five absent exact startup procedures and paged queries. It is metadata/identity acceptance, not a native execution or full semantic test.

Raw checks retain branch precedence, Shutdown/drop-before-application ordering, canonical/interface identity, file-backed scalar/default sources, collision-wrapper floating-result use and original glass table order. The first index/verifier draft's five-body undercount was rejected; normalization now handles both manifest schemas and root-relative nested paths, with independent expected per-root counts. Effective live throw settings, physics implementations/units, actual release/throw/collision/window break, exit and Rust parity are **not tested**. No game launch or Rust build/tests occurred for these offline/documentation changes.

## Window/crate offline acceptance (2026-10-22)

Run `C:/Python310/python.exe -B ../../work/hl2-decompiled/scene-window-route-20261022e/verify_seal.py` and the sibling `scene-crate-use-20261022f/verify_seal.py`. Expected passed:true: window116 retained files/26 bindings,23 bodies/2,569 instruction records/13 selected dynamic metadata stores; crate76 files/14 bindings,18 bodies/1,093 records/five authored crates/five pickup edges. These independently verify exact immutable identities, PE bytes, owned BSP/propdata and closed file sets. The window query supports health/fields/bodies with bounded paging.

The owned propdata parser agrees with pinned SourcePP on ordered content and preserves stable source/read-lock identities. The three declared packs lack that member; the loose file is bound, without asserting all possible live mounts. Window health17 is authored and profile health15 is conditional successful-parser evidence; live health/max/modifiers remain unobserved. The crate route's outputs precede attachment and cannot prove actual carry/throw. No native game started in this continuation, no input/focus/cursor action followed the owner's deferral, and no Rust build/tests were required for the shared documentation. First-map exit and Rust parity remain false/untested.

## Exit-registration static acceptance (2026-10-22)

Run `C:/Python310/python.exe -B ../../work/hl2-decompiled/scene-exit-registration-20261022d/verify_seal.py`. It checks53 retained files (55 including manifest/receipt), seven external identities, exact immutable pass14 bodies and503 PE instruction records, RTTI/datamap/callback/factory bindings and preserved missing registration stub. A draft assertion at the conditional branch was rejected and corrected to the actual callback-store instruction; the rejected draft remains retained. Expected live_trigger_instance_bound/native_execution_tested/runtime_exit_tested/first_map_finished/rust_parity_tested:false. One separate guarded native launch was deferred before Popen because of existing cursor confinement; no runtime result is claimed and the owner's game remains undisturbed. No Rust build/test required for these documentation-only changes.

## Static exit-plan acceptance (2026-10-22)

Run `C:/Python310/python.exe -B ../../work/hl2-decompiled/scene-exit-plan-20261022c/verify_seal.py`. Expected passed:true, sealed_files:13, external_bindings:8; acceptance exact_bodies:2, instruction_records:280, authored_entities:27, duplicate_preserving_edges:47, models:5. It verifies immutable exact body identities, PE instruction bytes, original-BSP entity/brush/model bytes and repeated output rows. Retail class registration/live trigger binding, runtime exit and Rust parity remain false/untested. No native game was launched for this static addendum; no gameplay build or Rust tests were required.

## Final SECTION/storage acceptance (2026-10-22)

Run `C:/Python310/python.exe -B ../../work/hl2-decompiled/scene-section-route-20261022b/verify_seal.py` offline. Expected passed:true, sealed_files:1504, external_bindings:52, distinct_exact_native_bodies:31 and native_first_map_finished/rust_parity_tested:false. It checks the exact file set and external identities, repeats verify_static.py, verify_v3_windows.py, eta/verify.py, verify_control.py, verify_v5.py and verify_loop.py, and checks bounded queries/observer receipts. No gameplay rebuild is required for this documentation-only publication.

Selected V5:677 contiguous emitted events,157 complete call pairs,19 independently relocated observed hook signatures,5 scoped outputs and3 automatic resume groups. Three SECTION scene times are1.680023909,7.605195522,6.884835720. Fourteen sampled pause groups retain the complete transitions;155 unchanged frames were intentionally suppressed, with dropped-events0. Raw callback/readiness/registered-event arguments and fresh controller/actor generations are checked. Movement acceptance includes63 player snapshots and45 captures/4663 samples, with raw modes0/2/9 permitted only in the named ladder climb/dismount captures and9 only in the named on-ladder snapshot; other walking requires2. Final dismount(-3342.134521,92,80.03125), plus a capture, does not establish a window or exit crossing.

The fresh control has14,003 passive samples,63 player snapshots and47 motion captures/5158 samples; three paused scene-clock plateaus recover near1.6800264,7.6052876,6.8849359 while global time advances. Its first6734-sample watch retains7 clock rewinds while intro01b remains playing with unchanged identity. This is bracketed VM_READ observation, not atomic reads or method-order/timing-fidelity proof. Native research processes closed with checked installed settings/module identities unchanged. The original dense-trace gap and rejected routes remain preserved. Complete animation/audio/event replay, runtime ETA values/typed ABI, crate/window escape, map exit and Rust parity are **not tested**.

## Bounded doorway/SECTION acceptance (2026-10-22)

Run `C:/Python310/python.exe -B ../../work/hl2-decompiled/scene-section-route-20261022b/verify_static.py` and `C:/Python310/python.exe -B ../../work/hl2-decompiled/scene-section-route-20261022b/verify_v3_windows.py`. Both pass:23 exact bodies,19 entry signatures,15 instruction windows/988 records;1,183 closed call pairs,12 scoped OnStart outputs,43 raw player snapshots and39 motion captures/4,252 samples. Closed native call trees verify PauseThink -> successful CheckEventCompletion -> ResumePlayback at intro scene7.605485 and security03 scene6.883860. The dense windows retain seq1..1375 and1674..3852; the gap and orphan boundary records remain explicit. This is not a continuous native trace or timing-fidelity proof.

The sampled replacement passes13 offline API/memory checks, including suppressed unchanged pause groups and forced complete transition groups. Native sampling/control comparison is pending. Copied named snapshots' frida_attached:false metadata is not instrumentation attestation; actual attach/detach records define the process context. The serialized control route stopped after two blocked pulses; brush1019 is an authored overlap candidate, not a native contact trace. Settings/module cleanup checks pass for the closed research runs. Full scene audio/animation, map exit and Rust parity remain untested. No Rust build/tests were run for these documentation-only changes.

## Natural customs/pen/Barney route acceptance (2026-10-22)

Run `C:/Python310/python.exe -B ../../work/hl2-decompiled/scene-customs-route-20261022/verify.py` and `C:/Python310/python.exe -B ../../work/hl2-decompiled/scene-customs-route-20261022/verify_seal.py`. Expected passed:true,25 controller names,2183 selected events/1005 pairs,168 requests/166 accepted nested starts,164 authored caller joins,54 player snapshots and46 motion captures/4416 samples. Ten exact path/hash/address/baseline/original bodies and six relocated hook entries per script pass. Seal461 files/33 external bindings and the preceding doorway seal pass. Five paged-query cases passed;the query marks unpromoted output identity.

Three stale Barney actor reads and three output-only records are excluded from promoted identities. One additional empty guard capture is rejected;blocked/backtracking pulses remain among the46 raw captures. Native pen arguments distinguish the still-valid first*49 caller from three later null callers. Later fresh Barney binding is not backdated. Source clocks,raw entity/model generations and scoped scene-self output context are independently reconstructed. Large MCP replies were exported;record order is canonical,not chronological. The native game closed with selected installed cfg/module identities unchanged. No fresh uninstrumented customs replay,full camera/audio/animation/SECTION/helmet/exit validation or packaged Rust comparison was performed. No Rust builds/tests/Clippy were run for this documentation-only change.

## Native grate crossing and player input checks (2026-10-22)

Run `C:/Python310/python.exe -B ../../work/hl2-decompiled/scene-doorway-route-20261022/verify.py` and `C:/Python310/python.exe -B ../../work/hl2-decompiled/scene-doorway-route-20261022/verify_seal.py`. Expected passed:true,native_grate_crossing_verified:true,617 motion samples,eight captures,one exact native body,six live input records; seal99 files/seven external bindings. Acceptance reconstructs raw movement/input/clock/generation reads, independently compares live field-name strings to PE bytes, and checks BSP brush/side/plane ranges. It rejects the stationary direct-button attempt as input evidence. Native walk stops at the predicted boundary; jump crosses with feet>=48 over the grate and lands beyond it. Settled getpos corroborates raw feet+64 eye coordinates.

PID29660 was never Frida-attached and closed with checked installed cfg/module identities unchanged. Fine alignment altered requested input magnitudes35 and restored450 before walk/jump. Pulse delays are not exact game durations. This is controlled native route evidence, not full map traversal or a packaged Rust test. No Cargo/build/Clippy/gameplay checks were run for these documentation changes; first_map_finished and rust_parity_tested remain false. The prior player-route seal remains unchanged.

## Session 29 owner playtest fixes, zombies, fire (2026-10-22)

- Packaged runs (artifacts/s29, `run.sh <fixture> <frames>`): cancop-throw (pickup -> NPC_MetroPolice.Cupcop.PutItInTheTrash; thrown can: thrown_object_hits 1, logic_achievement.FireEvent 1, throwcan_trigger -> Cupcop.Failure), zombie-look (classic/fast/poison zombies with headcrab bodygroups, capture), zombie-hall (`ent_create npc_zombie` 190 units ahead of the walking player: chase, 9 claw hits in 19 s, zombie cues started after the preload), idle-plaza (60 s at (-2600,-2900,90): 6 METROPOLICE lines, 10 before), barrel-burn (`prop_physics_create props_c17/oildrum001_explosive.mdl`, two pistol shots: General.BurningObject at 2.85 s, explosion at 5.89 s).
- Native: oracle plaza dwell with `developer 1; snd_showstart 1` (snd_showstart prints nothing without developer 1): one !METROPOLICE_IDLE0 in 60 s.
- Regression artifacts/s29-regression: weapons 17/17, attention 26/26, movement 25/26 (doll01); owned suite 55 pass; hl2-simulation 347; workspace release tests fail only the two known release-only should_panic image tests.

## First-map player-triggered scene checks (2026-10-21)

From checkout run `C:/Python310/python.exe -B ../../work/hl2-decompiled/scene-player-route-20261021/verify.py` and `C:/Python310/python.exe -B ../../work/hl2-decompiled/scene-player-route-20261021/verify_seal.py`. Expected passed:true,512 events/253 paired calls,six exact baseline/original bodies/six relocated hook entries,and fresh_never_attached_playing_snapshot:true. Independent retained-byte checks reconstruct29 player snapshots plus fresh named scene/actor state,source clocks and entity generations,request input pointers and output self/self/0 identity. Instrumented actual player movement triggers the genuine policy0 passenger scene at114.180;fresh control observes it playing at114.420. Console positions corroborate settled raw feet+64 eye positions;in-motion timestamps are asynchronous.

Both owned native processes closed;selected installed cfg/four-module identities match before/after. Frida changes executable memory and may perturb timing. Earlier wait90 unchanged-position rehearsal,SDK-only input-gate hypothesis,rejected output-activator checker and unresolved strafe boundary remain explicit. Capture labels train-exit/platform are not accepted exits. first_map_finished,ordinary_full_traversal_tested,train_exit_completed and rust_parity_tested remain false. No Rust edits/build/gameplay tests by this research pass;source implementation changes are concurrent work. Parent20261020 evidence/seal remains unchanged;historical16h closure correction and missing exact procedures persist.

## F4 overlay, ent_create and the main merge (session 28, 2026-10-21)

- Unit tests: `ai::spawn` overlay clicks/scroll/filter/escape/common folder, ent_create placement and quoting; `spawn_panel` drawn-row hit test at 720 p; hl2-ui console routing keeps quoted values. Workspace release tests pass except the two known release-only should_panic image tests; owned suite (HL2_ROOT, `--ignored`) 54 pass; strict Clippy and fmt clean.
- Packaged script runs (artifacts/s28, `run.sh`, `mkfix.py`): mouse-props (clicks spawn oildrum001_explosive, wood_crate001a and the filtered items/ammocrate_grenade in the d1_trainstation_02 plaza), mouse-npc (npc_combine_s_elite with weapon_ar2 by clicks), mouse-panel/mouse-common/mouse-page (filter "barrel", common folder with names, wheel + PageDown with 28 rows), entcreate (angles yaw 180). Real mouse input not tested.
- Regression batch artifacts/s28-regression vs s27-regression-b: weapons 17/17, attention 26/26, movement 25/26 (known doll01 PHY root rotation warning); only the Breen slate exposure differs (max 12 levels).
- Final package `bin/hl2-bevy.exe` SHA-256 `3DD773E9607A1B3A324CDB97FFDEB8213290BC4BB20A89DAF93AAAC7FDCCB38F`.

## First-map completion audit and natural starts (2026-10-20)

Verification confirmed `passed:true`, two natural_starts, `first_map_finished:false`, `ordinary_full_traversal_tested:false`, `rust_parity_tested:false`. The new lane reran eleven retained checks,including consolidated seal,independent311-query integration and explicit16h drift acceptance. It rechecks six exact native bodies against immutable path/hash/address/baseline/original rows,six relocated hook prologues,1672 selected events/834 call pairs,raw callback clocks,actor/model generation,input pointers and OnStart output identity. Thirty-two pre-window events remain excluded. Complete exported Evidence is authoritative where MCP transport responses exceeded10MiB.

Native scene2_lcs_intro accepted at10.230 game seconds with genuine gman_high,policy0;train endline accepted at93.165 with genuine citizen_train_1/male_01,policy1 and speech-scene query0. Independent review reconstructed raw input stacks,actors,callers,output addresses and both fresh-control snapshots;these checks are now in acceptance. Relay dispatch/controller serials were not instrumented. Fresh PID53968 without Frida shows G-Man playing at25.860..25.875,then cleared/absent at107.955..107.985;it did not catch endline playback. All three owned launches closed with checked cfg/module identities unchanged. Retained failures include an EOF rehearsal,fixed-length model decoding,old-map prefix,transport overflow,a verifier string-guard/JSON-shape correction,and forward input with identical measured position. No full route,scene completion/audio/camera/animation equivalence or current packaged Rust replay/build/tests were performed. First map remains incomplete;the second-map lane was not advanced.

## Controlled scene-start native observation (2026-10-20)

Documentation validation: expected `passed:true`. It checks the sealed149-file research set without rewriting its manifest, runs verify_static.py and verify_runtime.py from scene-start-frida-20261020, rechecks consolidated first-map metadata, and checks the current documentation diff/whitelist. The original research verify.py also pins its research-time Git revision and document snapshot; that guard is historical and can fail after unrelated implementation commits. Do not reseal or weaken it to accommodate a later checkout.

The native components verify six exact immutable database bodies,496 native body bytes/173 independent instruction starts, bounded static164nodes/166edges, complete selected function-call captures, actor index/serial/raw-field/clock consistency, policy1/query transitions and policy0 control, fresh native output ordering and cleanup/settings identities. Two optional timer samples raced their clock reads and are rejected; decisive function callbacks match raw bytes. The original lifecycle full checker fails its changed Rust-source external binding, which remains a historical source snapshot rather than a native identity failure. Consolidated first-map metadata verification still passes. No Rust rebuild/tests or parity replay was rerun for documentation publication. Private README retains failed/partial probes and limitations.

## First-map scene successor checks (2026-10-18)

Additional parent reruns pass: `campaign-scenes-20261016j-animation-dependencies/verify.py verify`, `campaign-scenes-20261016k-responses/verify.py`, `campaign-scenes-20261018a-arrival/verify.py`, and the separate intro/arrival audit checkers. Animation validation independently reparses stock VPK/MDL/ANI references without producer imports (same research agent,not a separate-agent semantic audit). Arrival's separate audit rederives27 datamap rows,seven exact bodies/seven byte windows and115 selected entities/215 connections. First-map handoff `verify.py --check` checks all66 joins,62 resource plans,620 event occurrences across44 scene instances,seven groups and11 read-only query cases;605 events remain the count across40 distinct first-map resources. Its final package seal now passes with16 explicit gaps and finalized queue/response/speaker references. A separate preseal audit exposed cinematic actor-query KeyError1264;scene808,cinematic1264 and AI321 are now exercised separately. The consolidated package check passes. Queue/speaker verifiers also pass;the latter preserves missing exact SpeakerThink102c38d0 and labels recurring scheduling SDK-only. No runtime/ABI/Rust parity is implied.


Documentation/private evidence only. Scheduler, actor-eligibility, and intro-camera checks all pass. They check exact frozen identities, retained body bytes, private PE witnesses and authored metadata; scheduler additionally checks15 complete spans/2329 bytes. Intro's12 listing windows include padding/neighbors and must not be called complete function coverage. Its absent exact callback103f9110 and rejected56-byte datamap interpretation remain retained.

The scene-scheduler has62 manifest files/20 bindings; actor eligibility85/13. Intro checks12 exact bodies,11 input bindings,33 datamap records,1044 listing records/3098 bytes and52 selected intro entities/154 connections. Runtime audio/camera/movement, ABI, ordinary traversal and Rust parity are **not tested**. Model-dependency finalization and separate intro/arrival audits now pass; rejected intermediate model/datamap drafts remain unpromoted. Queue/speaker and consolidated package checks now pass;native runtime/ABI/Rust parity remain untested.


## Campaign NPC scene research checks (2026-10-16)

Documentation/private research only; no Rust implementation or packaged/native runtime tests were run. Verification checks the finalized root manifest, current installed entity-lump/cache bytes and sealed research bindings; expected `passed:true`. Per-lane verifiers are listed in the private handoff. The independently coded `inventory-audit/audit.py` and `verify_guide.py` reports bind complete installed map/cache identities and canonical guide-v3 derivations; their retained results passed with zero mismatches. The separate first-map audit passed 275 metadata checks. Do not rerun authoring producers or audits that write over finalized output.

The inventory establishes 69 numbered campaign maps plus intro/credits, 77,628 entity records, 1,837 fully decoded cache entries/15,114 events, 760 scene-controller instances, 641 cinematic sequences, 507 AI schedules and 26,488 output connections. Root verification rechecks installed file identities/entity-lump bytes and cache hashes; the independent inventory audit supplies full map SHA verification. The first-map dossier checks authored float timings, event order/enabled state, actor/target candidates and exact output relationships. These are static data/native-evidence checks, not gameplay, ABI, animation/rendering/audio, ordinary campaign or Rust parity. Owner-confirmed native gap `d2_coast_02` remains an expected rejected zero-byte stub, not missing campaign content.

Public acceptance: `git diff --check -- AGENTS.md STATUS.md MODLOG.md docs/DESIGN.md docs/research.md docs/validation.md`, source-only whitelist/path audit and review of those documentation changes. No gameplay rebuild is required for documentation-only changes. Concurrent source edits belong to Claude and were not staged or changed by this research.

Final checkpoint16h: primary seal and read-only rerun pass. Separate-agent final integrity review checks 566 root artifacts, 24 external bindings and 353 lane files against recorded hashes, with the exact root file set matching the declared lane exclusions. Private `research-campaign-scenes-finalization-20261016h.json` retains results; `campaign-scenes-20261016h-final-audit/REPORT.md` records the independent check. These are integrity results, not new gameplay verification.

First-map LOOP successor16i passes, including the parent read-only rerun: 29 manifest files, 15 external bindings, four exact native bodies, four selected complete spans/238 instruction records/673 PE bytes and a four-case disposition table. Actual loop/audio/frame-boundary behavior remains **not tested**. The metadata distinguishes inactive AI_AIM from enabled FACE/LOOKAT; no display/effect was inferred from an inactive authored channel.

Post-seal16j audit: first-map-path-completion's original exact file-set check now fails after11 stale-worker additions. No original file changed or disappeared; the27 original lane hashes and566 parent hashes match. Validation expected `passed:true` and `original_closed_set_checker_passes:false`. It verifies original database/PE checks plus an explicit combined file set and preserved byte copies of additions. Added material is not adopted as semantic evidence. Neither prior seal was rewritten; the original closure failure is preserved.

## Installation setup on main (2026-10-15)

Codex Desktop, GPT-6 family (exact model identifier unavailable). Integration base main `675c9df`; only setup source, dependencies and documentation are included. NPC branches and NPC/simulation source are unchanged. Windows 11, Rust 1.99.0, owned app220 content. This verifies bootstrap behavior, not campaign/native parity.

- `cargo test --locked --workspace -- --skip impact_damage::tests::glass_table_breaks_bottles_on_light_drops_and_keeps_its_row_order`: **459 passed, 48 ignored, one excluded**, doc tests pass (`artifacts/friendly-setup-20261015/main/setup-only-workspace-tests.log`). All five setup/control tests pass.
- The original physics test was explicitly run with `cargo test --locked -p hl2-simulation impact_damage::tests::glass_table_breaks_bottles_on_light_drops_and_keeps_its_row_order -- --exact` and failed at its `std::ptr::eq` assertion (`main/original-physics-test.log`). Its two test-only behavior assertions from the earlier NPC-based feature branch are deliberately excluded from main. The earlier 498-pass feature-branch result and 460-pass main run with that temporary test adjustment are superseded for this setup-only integration. No physics or NPC runtime/test edits are published.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all --check` and source diff/whitelist checks pass. The new Rust module requires no asset whitelist exceptions.
- `scripts/build-bevy.ps1` rebuilt the release package on main. `--setup-preview artifacts/friendly-setup-20261015/main/setup-ready.png` exited 0 and its screenshot was visually inspected: editable path, all four controls, status and Steam instructions are readable. `--no-focus --volume 0.01 --frames 60 --capture artifacts/friendly-setup-20261015/main/gameplay.png --report artifacts/friendly-setup-20261015/main/gameplay.json` finds the owned installation, saves the test preference and captures station02. Test preferences use `HL2_RS_CONFIG_DIR` under ignored artifacts. Startup remains unfocused/paused as requested; this is not a playthrough.
- Main release executable SHA-256: `E3E0E92CC727AF433C2934A98FE1637B517438E19E3A287FA5F7EFEE620EAA56`. Executable/shader hashes are recorded in `bin/build-bevy-info.json`.
- Earlier source-assets checks in `artifacts/friendly-setup-20261015`: install fixtures **4 passed, one owned test ignored by default**, and the owned readiness test explicitly passed. Reproduce with `cargo test --locked -p source-assets install::tests::owned_install_passes_launch_readiness_check -- --ignored --exact`. Cases include secondary libraries/custom installdir, root/subfolder normalization, missing/zero-byte maps, missing core resources, override precedence, stale preferences, Unicode preference paths, corrupt/replaced JSON and Continue/Cancel/validation-response paths. Invalid noninteractive roots exit 1 without a window.
- Earlier `setup-found.png` and `setup-missing.png` were blank and **rejected**. Layout diagnostics showed valid geometry; capture now waits for visible layout, renderer pipeline readiness and a startup interval. Accepted ready/missing views are `setup-ready.png` and `setup-recovery.png`; the additional main setup capture also shows the complete screen.
- Windows computer-use `list_apps` timed out initially, on retry, and after reset/reinitialization. No native input was sent. **Native picker/clipboard interaction, title-bar cancellation and the complete interactive Continue-to-game transition are not tested**; the control/error/response paths have programmatic tests. **macOS/Linux bootstrap is not tested.**

Readiness checks the selected map's leading BSP header and representative required VFS resources; it does not audit all archives/chunks, CRCs, versions or campaign support. Original game files are only read. Screenshots, logs, test settings and rejected evidence stay in ignored artifacts.

## Breakables, displacement/clip collision, teeter hinge (session 16, 2026-10-14)

Packaged d1_trainstation_02, `--volume 0.01 --no-focus --fly`, artifacts/s16; native views in private original-oracle (HDR Full, skill 2).
- **Pistol at bottle #712** (`bevy-pistol-breaks-bottle-skin4.json`, eye (-3300,-2500,128), Bevy pitch -34.4 yaw 4.34): one shot breaks it into 3 garbage_glassbottle003a chunks drawn in the parent's skin 4 (green; bottle-after1.png). Native T232957Z at the same view: one bottle breaks into green chunks too, but native's bullet struck the plastic crate first and pushed it back; ours hit the bottle directly.
- **Pistol at the teeter-totter seat** (`bevy-pistol-breaks-teeter-seat-chunks.json`, eye (-4851,-3611,80), pitch -26.6 yaw 90): seat (Wooden.Medium 30, bullets 0.75, pistol 5) breaks on shot 8 into 4 WoodChunks (no .phy gibs) spread along the board, debris, on the ground at z 1-2 (teeter-broken.png). Native T235049Z: level at rest (same as ours), visibly rocking after 2 shots and broken by shot 5; ours rocks ~5 degrees over 7 shots even with the SDK bullet impulse at the hit point. Open.
- **Terracotta pot #491** (playground, eye (-4205,-3596,40) pitch 25 yaw 180): before, it fell through the displacement 9 units onto the brush below and broke under the glass table (pot-ours.png); after the terrain winding fix it rests upright at z 0.39 (base 0.45 above the gravel, 0.5 margin) like native T231808Z (pot-ours3.png vs native pot.png). With Rapier's internal-edge correction still on, the resting pot jittered, wandered 2.5 units and tipped over within 5 s (rejected).
- **Teeter seat at rest:** before, it overlapped a DETAIL|MONSTERCLIP brush by ~14 units and was flung at up to 1600 u/s (contacts at -13.8 units with the clip brush); now clip-only brushes have no contacts with physics objects, the seat-base pair is disabled (logic_collision_pair) and the SF 8 hinge holds the seat level at (-4851.36,-3491.65,19.78) for 400 ticks.
- Native pot probes T231934Z/T232019Z (ent_dump/ent_text via !picker): rejected twice, the picker named nothing.
- **Idle sleep:** `awake_bodies` in the hall (eye (-3356,-2520,128), fly, no input): 37 at tick 60, 1 at ticks 300/600/1200/1800 (physics_ms 1.04-1.11). Session 15 counted 7-38 bodies that never slept.
- **Regression batch artifacts/s16-regression:** weapons 17/17, attention 26/26, movement 25/26 (the known doll01 PHY root-rotation warning only); finals vs s15b-regression mean 0.00-1.37 (Breen slate exposure; max pixel 8), Breen feeds identical, Kleiner feed 336 pixels at the mouth (lipsync timing), attention 0.05-0.58.

## Stacked props and prop CCD cost (session 15, 2026-10-13)

Constraints (`bevy-playground-constraints.json`): tic-tac-toe blocks at hinge heights z 63/46/29 (play1.png); teeter seat (-4848,-3488,19); swing seats and carousel at z -10 as before. Carried bottle (`bevy-carry-bottle-shove.json`, now from (-3356,-2520)): crouch works while carrying; bottle minimum z 66.5 (floor rest 66.6); held bottle overlapping the crouched hull by 0.6 froze the player via CheckStuck until debris props (SF 4) stopped colliding with the player (now 0 stuck samples). Regression batches artifacts/s15-regression and s15b-regression: 25/17/26 (doll01 only), finals vs previous mean <= 1.03 / 0.59, feeds identical.

Studio decals: `bevy-studio-decal-cop.json` (posed terminal_side_cop, 3 pistol shots): 2 blood decals built (sdecal3-zoom.png), 1 grazing hit empty (1/3 built before re-shooting along the shot onto the skin). `bevy-studio-decal-bugbait-cop.json` (200 units): 1 BeerSplash model decal over 1788 triangles, tint 0.08 (alley light). Native oracle T160814Z (bait at the queue citizen's back from (-3800,-2248)) shows no clear stain on dark clothing; ours at the same views (`bevy-studio-decal-bugbait-citizen.json`) neither; the queue state differs. Inconclusive.

Gibs: pallet #327 launched into the hall wall (`bevy-pallet-launched-into-wall.json`) spawns 7 wood_pallet001a chunks (gibcap.png mid-break; the intact pallet at the same spot, intact.png, has the same cool lighting); `bevy-pallet-gibs-fade.json` (look away at tick 100): alpha falls after each chunk's fadetime (2 s / 10 s) and all 7 are removed by 20.4 s. Native not compared.

Packaged d1_trainstation_02, `--volume 0.01 --no-focus`, artifacts/s15; `step_ms` = whole fixed host step.
- **Pallet off-center onto plaza cop #427** (`bevy-pallet-tips-off-plaza-cop.json`, eye (-1978.8,-2267.4,64) yaw 180, pallet #882 posed at (-2048,-2272,130) roll 10): before, physics 20-40 ms per step for ~0.7 s while it tipped off (the 15 ms tick falls behind: owner 46 -> 17 fps) and 3-7 ms idle; a temporary Rapier `profiler` build attributed it to the CCD time-of-impact search (17-26 ms; broad+narrow+solver ~2 ms) with 38 awake bodies at idle. After CCD gating: worst 2.5 ms, idle 1.1-2 ms. The posed terminal cop (`bevy-pallet-tips-off-npc-hull.json`) was already cheap (the pallet fell off flat).
- **Crate on box** (`bevy-crate-on-box-stack.json`, crate #715 dropped onto box #710): crate-box origin offset 20.404 with summed skins, 19.904 now (box z 76.392 both). Native not measured numerically.
- **Pallet launch** (`bevy-physcannon-pallet.json`): y -4235/-4012/-3849 at f305/f320/f335 as in session 14, rest -3479 (session 14 -3495; native -3572; tumbling diverges). Soft CCD variant rejected (first-bounce z 75, rest -3467).

## Player/prop contacts, NPC hulls, prop drag fractions (session 14, 2026-10-13)

All packaged runs on d1_trainstation_02 with `--volume 0.01 --no-focus`, reports in artifacts/s14 (`hull_contacts`, `shadow`, `physics_ms`, `carrying` sample fields).
- **Stuck on props** (`test-inputs/bevy-player-on-box.json`, `bevy-player-walk-into-boxes.json`, `bevy-player-push.json`): before, landing on cardboard box #710 sank it ~1.5 units into the floor (the explicit 85 kg weight impulse) and its rebound overlapped the hull 0.7-1.5 units (stuck); walking diagonally stuck the player on #711 for 230 ticks. After: 0/28 and 0/42 stuck samples; the wall push leaves only <0.01-unit momentary contacts; box rest (-3360.0,-2607.9) vs native (-3358.3,-2607.4). Rejected on the way: a 0.25-unit inset shadow (a box rebounded off the wall into the gap), a velocity clip alone and a "blocked" rule alone at the wall jam, a kinematic shadow (not retail).
- **Frag +USE** (`bevy-frag-use-pickup.json`): thrown at the feet, picked up via the FCAP_USE_IN_RADIUS sphere search, carried until the fuse (capture fraguse-held.png). Native not compared.
- **NPC hull** (`bevy-player-on-npc-hull.json`): feet stay at 137.0 (posed cop origin z 65 + 72) while moving 21 units over the shoulder; the pre-change build slid off. SDK-derived; native not captured.
- **Pallet on cop** (`bevy-pallet-on-npc-hull.json`, pallet placed 110 units up over the posed cop): physics step mean/max 25.5/62.2 ms with model colliders vs 4.7/9.4 ms with the hull; the pallet rests at z 76.9 on the hull.
- **Pallet launch vs native T021822Z** (`bevy-physcannon-pallet.json`): y at 0.08/0.45/0.9 s and rest: ours -4235/-3899/-3673/-3521, native -4258/-3904 (0.47 s)/-3711 (0.91 s)/-3572. Counterfactuals: old -4246/-3999/-3843/-3769; stored fractions only -4235/-3911/-3679/-3539; plus IVP axis pairing -4235/-3894/-3690/-3529; axis pairing only -4248/-3990/-3835/-3775. Frag replay (owned, best RMS lob/roll/high/plaza): stored fractions 31.7/16.3/3.4/40.1, both 21.2/65.0/-/-, kept estimate 27.6/16.4/3.8/12.8.
- **Cop impact kill** (`bevy-pallet-kills-cop-physdamagescale1.json`): terminal_side_cop has map physdamagescale 0; with `AddOutput physdamagescale 1` a 20 kg pallet at 1500 u/s kills it on contact with the hull; without, it is unhurt.
- **Contact margin and prop damage (late session 14):** crate #715 drop (`bevy-crate-drop-rest.json`) rests at 71.921 (native authored 71.957; 71.421 without the margin); pallet #327 into the hall wall (`bevy-pallet-launched-into-wall.json`) breaks with the launch flag, survives without; pallet #882 street launch rests at y -3495 (native -3572) unbroken (map physdamagescale 0.0001).

## Bug bait, player shadow and plaza cop vs native (session 13, 2026-10-13)

Bug bait (eye (-4200,-224,1) setang 0 180 in the d1_trainstation_02 spawn room; native T012846Z, ours `test-inputs/bevy-bugbait-wall.json`): native leaves a dark reddish stain right of the crosshair; ours with the retail subrect decal lit by the wall lightmap gives a similar stain (artifacts/frag-s13/bait-wall-crop.png). The standalone decals/beersplash texture (10% mean alpha) was nearly invisible: rejected. Earlier floor/outdoor comparisons were invalid (the "splash" was a floor texture stain in both).
Player push (eye (-3356,-2520,128) yaw 270 into cardboard_box001a #710, 5 kg): native 0.5 s y -2605.7, rest (-3358.3,-2607.4) (T012535Z); ours 0.45 s -2603.2, rest (-3359.0,-2608.7).
Plaza cop #427 (physdamagescale 0): native wood_crate001a launch shatters the crate, cop health stays 40 (ACT_FLINCH_PHYSICS); with `ent_fire metropolice_plaza addoutput "physdamagescale 1"` a 30 kg oildrum001 brings him to -60. Native spawned pallet launched from (-2330,-2272,60) setang 4 0 rests ~845 units away near (-1485,-2223) (T020048Z; no cop in that run: the plaza cops spawn randomly, so the pallet kill is unconfirmed).

## Frag rigid body (session 13, 2026-10-13)

Owned replay (`cargo test --release -p hl2-bevy owned_frag -- --ignored --nocapture`, HL2 installed): SDK launch math from the native eye/angles, seven spins per case, compared by time since the release command minus the native spawn delay. Best/mean RMS: lob 27.6/77.9, roll 16.4/16.5, overhand (setang -30 -20) 3.8/7.8, plaza (-10 12) 12.8/40.4, wall 44.1/81.5. Swept box: 92.1/22.3/13.1/44.7/105.8 (best). Bounds asserted: 35/30/8/25/55.
Native wall throws (oracle, eye (-3000,-1998,128) setang 0 180, timescale 0.25, `ent_text npc_grenade_frag`): T000533Z 0.661 s (-3157.6,-2002.0,163.5), 1.639 (-3134.0,-1999.3,66.2), 2.085 (-3123.3,-1992.8,66.1); T002157Z 0.707 (~-3145,-2044,~66), 1.217 (-3144.3,-2064.4,66.1), 1.673 (~-3192,-2069,66.1), 2.061 (~-3169,-2077,66.1); T002243Z 1.498 (-3192.1,-2092.2,69.7), 2.087 (~-3192,-2092,66.2). Native stays within ~18-60 units of the wall, sometimes pops up (z 163 at 0.66 s), drops and rolls along it (once into the door recess). Ours (packaged `test-inputs/bevy-frag-wall.json`, spin -0.5): pops up to z 156, lands ~90 units out: larger rebound than native for that spin.
Rest height: native frags at rest z 66.1-66.2 on the z 64 platform; ours 65.7 (hull radius 1.808). Ground-level captures artifacts/frag-s13/box-close-nw.png and top-se.png.

## Cloud arsenal/physics validation (session 12, 2026-10-12)

Frag rigid body (`--frag-rigid-body`, Rapier body from w_grenade.phy) vs the session-11 native samples, packaged, compared by time since spawn: overhand flight RMS 3.4 (max 5.4); roll RMS 14.0 (swept box 22.4); lob RMS 106.6, end 169.8 (stops on landing, body upright, native rolls on); trainstation throw RMS 42.6, end 56.7 (swept box rest within 4). Default stays the swept box.

Drag: retail law (drag.rs) with native `air_density` 2.00 gives the frag hull a tumbling-average 5-7e-4/unit vs the native fit 6.4e-4 (unit test). Gravity gun: native hold view T191304Z (eye -3490,-2110,128, setang 35 236): after the world-FOV size factor our holding glow matches the native size and colour (artifacts/cloud-validate/hold-compare2.png); claws open while holding in both. Launch: native draws a beam and sparks (ours none). Rejected twice each: native launch trajectory (ent_text overlays) and native bolt trajectory (command buffer overflow at host_timescale 0.1; no overlay at 0.25).

RPG: native T194927Z (timescale 0.25) rpg_missile at (-2291.0,-1846.2,261.4) ~0.80 s after +attack; ours at that x after 0.78 s: y -1847, z 264 (packaged bevy-rpg.json report).

Regression batch artifacts/cloud-regression: weapons 17/17, attention 26/26, movement 25/26 (doll01 "unsupported PHY root rotation" warning: new reporting from the physics branch's wider .phy reading; the prop keeps its render hull as before).

## Frag grenade vs native (session 11, 2026-10-10)

Native (oracle, HDR Full, skill 2, `ORACLE_GIVE=weapon_frag`, d1_trainstation_02 eye (-3104,-2018,128)): positions from `ent_text npc_grenade_frag` with `developer 1` (SDK DrawAllDebugOverlays draws entity overlays only with developer), times from the oracle command log at host_timescale 0.25/0.1 (latency uncertainty ~0.05 s game time). Throw A (setang -30 -20): 8 in-flight samples 0.20-1.34 s after release, e.g. (-2954.4,-2080.9,231.4) at 0.199 s and (-2210.7,-2351.5,373.4) at 1.335 s. Throw B (setang -10 12): (-2287.2,-1852.6,143.0) at 0.96 s, bounce off the monument, rest (-2098.9,-1747.5,18.1), gone (exploded) between 2.9 and 3.4 s.
Model fit (SDK launch: eye + 18 forward + 8 right, (forward + 0.1 z) x 1200, sv_gravity 600): no drag RMS 195.6 units, linear drag 10.3, quadratic drag c = 6.4e-4/unit 3.7 (spawn 0.045 s after the release command). Throw B predicted (-2288.6,-1852.9,140.4) vs native (-2287.2,-1852.6,143.0). Packaged Bevy after the change (`test-inputs/bevy-frag-high.json`): 19.7, 11.8, 10.4, 12.7, 15.0, 14.2, 7.1, 9.0 units from the native samples (consistent ~1 tick lag inside the timing uncertainty); before the change the model error reached ~370 units. Throw B in Bevy (`bevy-frag-native.json`) rests at (-2102.6,-1747.6,20.0).
Lob/roll (secondary standing / crouched; native sessions view-d1_trainstation_02-20261010T040222Z and T040313Z, release view setang 25 -20, timescale 0.1, positions by ent_text): native lob lands on the sidewalk (z ~68) ~0.27 s after spawn, rolls at ~155 u/s along +x to the curb and drops to the street (z 2.1) at ~1.4 s; native roll goes ~570 u/s on the sidewalk, drops off the curb (~530 -> 335 u/s), and slows to ~76 u/s by 2.2 s while curving. Surfaceprops (installed scripts/surfaceproperties.txt): grenade friction 0.9 elasticity 0.01 (base metalpanel), concrete 0.8/0.2, tile 0.8/0.3, default 0.8/0.25. Packaged after the contact model (`bevy-frag-lob.json`, `bevy-frag-roll.json`, compared by time since spawn): roll RMS 22.4, end 21.2 units; lob 4.7 / 0.5 units through the landing, then 11-151 units (ours keeps its heading at ~120 u/s; native rolls along its own axis); overhand monument throw 10.9 / 8.3 units in flight, then ours stops on the ledge where native rolls ~45 u/s for 1.4 s (end 69.5). Constant vs speed-proportional rolling resistance was swept (a 0-190 u/s^2, k 0-1.4/s): only the constant 190 fits the roll; neither fixes lob/overhand post-landing.
Fuse (native session view-d1_trainstation_02-20261010T043539Z, timescale 0.1, overhand setang -10 12): grenade present 2.99 s after the release command, explosion fireball at 3.20 s, smoke only at 3.32 s; with the 0.045 s spawn delay native detonates 2.94-3.15 s after spawn (ours 3.045 s, SDK think ticks). The same throw came to rest at (-2100.8,-1775.4,17.3), 28 units from the earlier native run (-2098.9,-1747.5,18.1): random spin makes native post-landing positions vary run to run.
Audio (per-process recordings, private arsenal-20261009/audio, native volume 0.05 vs ours --volume 0.05 --mute-ambient, lob at the plaza): native Grenade.Blip onsets found by cross-correlation with our clean blip at 0, 1.057, 2.102, 2.415, 2.740 s after the spawn blip (SDK think ticks 0, 1.05, 2.10, 2.415, 2.73; ours identical); blip window levels -44.4 dB native (ambient-subtracted, noisy) vs -45.7 dB ours. Second recording with `stopsound` in the pull view (ambient was back at -44.6 dB by the throw): native blip peak -38.4 dB vs ours -37.3 dB (match within ~1 dB); tentative native impact transient -45.7 dB (about -47.7 dB after ambient subtraction) 0.31 s after spawn vs ours -50.6 dB at 0.24 s, so ours may be ~3 dB quieter. Two attempts at isolating the impact in the plaza; next try a quiet map area instead.
Trail (owner report: wonky/less smooth): replaced per-segment quads by one strip; native vs ours at 0.21 s after release (artifacts/arsenal-s11/trail-compare.png: native, before, after). Shape and fade now match; the native core is ~2x brighter (red excess ~100 vs ~50, half-max width 4-5 px vs 3 px) because Sprite materials blend in gamma space natively (DESIGN 11f). Regression batch on the branch at 0e41ccb (artifacts/arsenal-regression): movement 26/26, weapons 17/17, attention 26/26; final captures vs artifacts/damage-regression: mean absolute differences 0.00-0.83 except the Breen slate 2.68 (known 0.64-0.68 exposure variance) and attention book-edge pixels (physics noise); no Breen feed change. The *-difference.png masks compare against old historical baselines and are not identity evidence. The batch has no frag fixture; rerun before the single 13b merge.

## Player damage, fades, damage indicator, HEV voice (session 10, 2026-10-09)

Packaged on d1_trainstation_04 (`--map d1_trainstation_04`, no globals when loaded directly): `test-inputs/bevy-player-death.json` (fatal landing at (-4584,-4163) from eye z 1400; Player.FallDamage + Player.FallGib, no HEV, respawn with 100 health) and `test-inputs/bevy-player-hurt.json` (suit; trigger_hurt *32 DMG_FALL at (-5608,-4783): !HEV_DMG5 at 0.15 s, death at 0.54 s with Player.FallGib and HEV_DEAD1). trainstation_02 drops keep health 100 (gordon_invulnerable on). Native oracle with `ORACLE_SETUP="noclip;cl_drawhud 1"` (the setup's own noclip toggled off, HUD shown), 6 s after death: trigger_hurt death native (255,64,50) / ours (255,60,47) mean RGB, red saturated everywhere in both; fatal landing native (253,0,0) / ours (255,0,0) uniform (black fade under HudPlayerDeath). Owned test: HEV sentences render within 5 ms of their authored {Len} (HEV_MED0 stale). Batch artifacts/damage-regression 26/17/26. Not compared: ramp timing (the oracle captures after a dwell) and HEV audio levels. Private notes: work/hl2-decompiled/player-damage-20261009/README.md.

## Soundscapes and barrier hum (session 9, 2026-10-08)

Barrier: packaged `test-inputs/bevy-barrier-hum.json` with `--audio-trace` and a per-process recording; the touch loop (2.69 s) stays continuous for 7.8 s and 6.7 s inside the trigger and stops on leaving. Soundscapes: packaged `test-inputs/bevy-soundscapes.json` (spawn, hall at (-4304,-722) eye 128, outside at (-3104,-2018) eye 128, 12 s each) and native oracle views at the same places with `ORACLE_SETUP="volume 0.05;snd_mute_losefocus 0"`, both recorded per process. Background medians (dB): spawn -56.0 native / -58.7 ours, hall -53.5 / -51.9, outside -44.7 / -38.1 (open). Native stays silent while unfocused unless `snd_mute_losefocus 0`. Batches artifacts/ambient-inputs-regression and artifacts/soundscapes-regression: 26/17/26.

## Audio verification method (session 8, 2026-10-08)
Session 27 (2026-10-21): for short distance-dependent sounds compare a far-only stretch of a per-process recording (trace: every play at the same premix, e.g. `#dv16`) against native at the same layout; check the trace's `started` volume too. Our far NPC gunfire measured -99 dB there because spatial players started at volume 0 for a frame (fixed a782904); the 25 ms far crack vanished while the 125 ms near layer survived, so level-only checks at mid range had missed it. Native layout observed via per-process recordings.


Use this method for any "sound is cut / sounds different" report. Sink lifetimes and sample counts alone missed a real defect.

1. **Lifecycle:** run the packaged game with `--audio-trace <file.jsonl>`. Each line records a request (cue, frame, scene time, paused), a sink start, and a removal with `last_position` vs `duration`. A sound removed early shows `remaining` well above the ~10 ms position granularity.
2. **Decoder values:** the owned ignored test `installed_weapon_selection_backend_preserves_full_duration` compares the backend decoder's samples with the WAV's PCM, from the first sample.
3. **What actually reaches Windows:** record only the game's process tree with the private Microsoft ApplicationLoopback build. Arguments: `<pid> includetree <out.wav> <dwell ms>`. The tool prints "10 seconds" but honours the dwell; the session 8 files are 40 s long. Do not use whole-endpoint loopback, which picks up other apps. Use `--mute-ambient` so the non-positional map ambients do not mask cues.
4. **Measure, don't eyeball:** align each occurrence to the source WAV by cross-correlation, using later parts of the sound so an altered onset cannot bias the alignment. Then compare the energy of each authored segment relative to the first (dB), and per-window correlation and gain over the first 30 ms. Avoid fixed-window envelopes normalised to their maximum: window alignment with a sharp attack biases them. Avoid high-pass cutoffs inside the sound's own band.
5. Reference recordings: the original file, and when useful a local one-off recording of another player (e.g. the historical Macroquad host). Native HL2 per-process recording is the parity reference when available.

Known backend behaviour: rodio 0.22 misreads the first 512 samples of any sound not in its mono 48 kHz filler format, so all waves are converted to 48 kHz at load (`hl2-bevy/src/audio.rs`).

## Owner report milestone (2026-10-08, Codex gpt-6.1-sol)

Owner follow-up correction: only plaza barrier proximity fading is fixed; scrolling animation remains broken. Earlier changing-UV observations did not prove visible motion. Frozen bursts establish rendering stability only. Revalidate temporal animation in an unpaused packaged run beside native HDR before marking it fixed. This correction changes documentation only; runtime tests/builds were **not rerun**.

344 normal and 33 owned workspace tests passed; strict `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check` and diff checks passed. Packaged SHA256 `6E6C7FE4ED4C5988553739F37F596F33CB632F8765937AEAFC3B0B4D0330A3B2`.

Run `C:/Python310/python.exe ../../work/publishing/run_gesture_layer_checks.py <new-outdir>` then verify_bevy_movement.py `<outdir>/partition-movement-final.json --capture <outdir>/partition-movement-final.png`, verify_bevy_entities_weapons.py `<outdir>/partition-weapons-final.json` and verify_bevy_attention.py `<outdir>`: 26/17/26 passed in artifacts/owner-reports-regression. Captures completed with zero pose/visibility mismatches; station03 retained its 25 known material errors. Historical pixel baselines differ; no image-identity claim.

Weapons `--capture-burst 120` with volume0.01/no-focus and flicker_check.py: 0/121 lost. Barrier positions (-1856,-650/-800/-1200,eyeZ60), yaw90/pitch0, exposure2, times1.50/1.95 select near frame0, middle24/25 (close layer2), far30 black. 61-frame bursts have no shield-region disappearance; small unrelated pixel variations remain. Four monitor times10.65/10.8/10.95/11.1 show Y translations -9.6915/-9.828/-9.9645/-10.101; frozen 31-frame bursts are identical and only establish rendering stability. Corrected seeded HUD fixture renders the separate owned additive brush. Native HDR references are private oracle T012742Z (shield; module-hash helper failed) and T014407Z (monitor; actual loaded hashes verified). These are controlled views, not a campaign playthrough or complete pixel parity.

Audio: exact decoded sample counts and isolated full-tail correlations passed on the prior F5930198 package. Two contaminated system-loopback captures were rejected. Owner cutoff not reproduced; focused manual/native audible switching **not tested**. The one 640x360 window was this scoped audio run; future audio runs use1280x720. New runtime captures use normal1280x720.

Viewmodel flicker fix (2026-10-08): weapons fixture with `--capture-burst 120`, two runs, 0 of 121 frames without the viewmodel; before the fix 1-9 of 61 frames per run lost it (bisect 0dc8291/1028388/6a7455d clean, 8ee25d2/1640df0/994df99 affected; captures in artifacts/flicker). Batch artifacts/bloomfix-regression: captures within 9 levels of artifacts/bloom-regression, movement 26/26, weapons 17/17, attention 26/26.

## Session 5 HDR path vs native HDR (2026-10-07)

Native reference: owner's real game runs mat_hdr_level 2; the oracle now passes +mat_hdr_level 2 +skill 2 (console confirms). Earlier native captures in this file were LDR (mat_hdr_level 0 in the oracle's Steam-Cloud-mirrored config). Settled native session view-d1_trainstation_02-20261007T164900Z (ORACLE_DWELL 15): mat_hdr_tonemapscale plaza 1.998, hall 1.880. Bevy (wip/hdr-path at merge) plaza scale 1.80-1.98: street (132,120,87) vs native (162,144,100), sky (93,92,93) vs (102,101,102). Hall during the broadcast (tick 1660) scale 1.92: wall (143,127,93) vs (159,139,100), screen (184,158,123) vs (181,153,121), left windows (68,58,37) vs (159,150,118) (envmap absent). Slate (native T165556Z scale 0.65): Bevy 0.69, cyan screen with white glyphs as native; native brighter while its exposure was still adapting. Regression batch artifacts/hdr-regression: complete, 0 mismatches, movement 26/26, weapons 17/17, attention 26/26; images change everywhere (exposure); baselines archived to artifacts/baseline-archive-20261007-hdr and replaced.

## Session 5 jumbotron, lightmap headroom and regression batch (2026-10-07)

Slate (pre-broadcast, tick 230, test-inputs/bevy-monitors-breen-slate.json) vs native view-d1_trainstation_02-20261007T054643Z/slate.png: the toolsblack frame now hides Breen around the slate as in native; screen mean RGB went (76,97,82) -> (80,118,100) after the 2x lightmap headroom -> (78,122,114) after linear UnlitTwoTexture modulation and `srgb?$color2`; native (76,156,154). Glyphs (165,199,199) vs native (175,214,218). The slate luxels are linear 128 in both lightmap lumps; native's HDR path saturates G and B further. A rejected 16x cap gave (105,229,230).

Plaza (d1_trainstation_02, eye -2206 -1669 100.18, yaw 90): new native capture view-d1_trainstation_02-20261007T152043Z/plaza.png. Mean RGB native/old/new: sunlit street (136,121,80)/(94,90,74)/(114,103,74), yellow building (93,78,48)/(73,63,41)/(75,64,42), hotel (87,79,58)/(72,66,49)/(73,66,49). Every sampled region moves toward native; the remaining gap is consistent with the unimplemented HDR path.

Batch artifacts/session5-regression (run_gesture_layer_checks.py): all cases exit 0, captures complete, 0 pose/visibility mismatches; movement 26/26, weapons 17/17, attention 26/26. Pixel changes are expected: brighter sunlit luxels (movement, campaign), static decals (Kleiner scene notes, Breen backdrop) and the monitor colour path. The Breen broadcast now matches native session T053505Z (backdrop panels, logo, scanlines, colour); Breen's framing still differs. Weapons and the Kleiner feed are unchanged within 3 levels. Old baselines: artifacts/baseline-archive-20261007-s5; the batch images are accepted, plus new partition-breen-slate baselines.

## Native trainstation_02 jumbotron comparison (2026-10-07)

Native view session view-d1_trainstation_02-20261007T053505Z (run_view_oracle.py, eye -3753 -2290 469, yaw -90, about 24 s after load) vs Bevy partition-breen-screen-final. Breen's broadcast, framing and hall lighting match. Correction (owner review): the yellow shape behind Breen is not see-through. It is his studio backdrop, the infodecal decals/decal_posterbreentv (Combine panels and logo), which Bevy did not render. Static infodecals are now projected (see the static-decal entry); the feed then matches native, apart from a slightly cooler native tint. Native colors are slightly more saturated. Shaft brightness after the rendercolor fix is comparable; no exact luminance match is claimed.

## Kleiner movement and new regression views (2026-10-07)

npc_kleiner uses the shared human ground controller (SDK and retail spawn agree). Batch artifacts/kleiner-regression (run_gesture_layer_checks.py, now 11 cases including partition-breen-screen-final and partition-kleiner-scene-final): all complete, 0 pose/visibility mismatches, attention 26/26. The previous partition baselines are archived in artifacts/baseline-archive-20261007, and the batch's images are accepted as new baselines (lighting, godray fix, new views).

## Source model lighting (2026-10-07)

328 normal and 25 owned tests, strict Clippy/fmt. Native close-ups (oracle session security-scene-20261006T204141Z) vs packaged Bevy test-inputs/bevy-face-security-1805/2245.json in artifacts/lighting: skin RGB is 95/66/45 vs 93/64/42 at 18.05 s, and helmet/hair tones match at 22.45 s. Native corridor view-d1_trainstation_03-20261007T041303Z vs the campaign fixture (artifacts/lighting/compare-corridor.png): door and pistol lighting match. The earlier corridor session T040936Z is rejected (cursor captured). Regression batch artifacts/lighting-regression: all cases complete, 0 pose/visibility mismatches, and station03 keeps its known 25 texture errors. Pixel changes are model relighting; the Breen baseline predates the godray fix. Retained smoke artifacts/lighting/retained-smoke.png.

## Facial flexes render in Bevy (2026-10-06)

Bevy loads each NPC model's flex data at setup (`LoadedMap::flexes` -> `rendering::FlexModels`; load failures go to texture_errors). Batches carry each vertex's (bodypart, mesh, mesh-local vertex). Each frame, when an actor's flex controller values change, controllers are mapped by name into model order, flex rules run, and the retail weights produce bind-space position deltas. These are written into AnimatedMesh bind positions; GPU-skinned meshes get the deformed positions patched in place, and the CPU path is forced to resample. Normals are placeholders in this renderer (lightmapped), so normal deltas are not applied. The retained host does not apply flexes yet (**not implemented**). Packaged security_02 captures: Barney's face visibly changes at 18.05 s (lowered lids, brow and mouth shape) against the earlier no-flex capture, and only subtly at 22.45 s (profile; diff box around the head). Attention 26/26; strict Clippy/fmt; 315 normal tests. Not compared with native facial frames yet; delayed-weight smoothing, eyelid/iris interaction and lip sync are missing. Bevy package `CFDBD76D928084EE2B0863E2D7F488DD319FB87E649D276BA59E5F902E060530`. Evidence: artifacts/flex-render/face-compare.png.

## Scene flex tracks drive controller values (2026-10-06)

Compiled-scene flex tracks evaluate like SDK CFlexAnimationTrack::GetIntensity. Samples are bounded by virtual zero samples at 0 and the event duration, and the zero value is the default edge zero position (binary scenes carry no edge overrides). Per-sample curve types cover hold, linear, ease in/out/in-out and the default normalized-X Catmull-Rom; B-spline, Kochanek-Bartels, simple cubic and exponential decay fall back to the default. Values clamp to 0..1 and remap to the track range, and stereo tracks use the balance samples for `right_`/`left_` controllers. CBaseFlex::AddFlexAnimation blending is applied per tick (orig x (1 - w) + value x w, with w = event intensity; the CSceneEventInfo weight ramp is approximated as one), after ProcessSceneEvents' 0.95-per-think decay scaled to the fixed step. Owned security_02 run: Barney's controllers include smile, left/right cheek raisers, corner pullers, inner/outer raisers, lid tighteners, lowerers and upper raisers, plus head_rightleft/updown/tilt (which in the SDK also feed head control; not yet connected). Tests: 315 normal tests, strict Clippy/fmt. Rendering of the vertex deltas is the next step. Model surfaces now record their studio bodypart/model/mesh and each vertex's mesh-local index (`Surface::flex_source`); the owned test confirms every flexed Barney/Kleiner mesh has a surface covering at least 90% of its flex vertices.

## Facial flex data reader (2026-10-06)

`source-assets::flexes` reads flex descriptors, controllers (type, name, range), flex rules (RPN ops) and per-mesh flexes. It traverses every model of every bodypart, decodes the four-point target ramp, flexpair and vertex animations (float16 position/normal deltas; wrinkle stride 18), and rejects pre-converted fixed-point files. Rule evaluation follows SDK CStudioHdr::RunFlexRules, including 2-way, n-way, combo, dominate and DME eyelid ops; invalid ops are skipped. Vertex weighting follows the retail StudioRender R_StudioFlexVerts (client 1001c0b0, private static review): target ramp per descriptor and pair, side blend, and speed blend between current and delayed weights (delayed = current here). Owned check: Barney has 73 descriptors, 54 controllers, 63 rules, 4 flexed meshes and 11,329 vertex deltas; Kleiner 71/54/63/7/12,575. Driving `jaw_drop` to its maximum activates mesh flexes on both. Not yet connected to scene tracks or rendering.

## Native comparison with matched campaign state (2026-10-06)

The native oracle (`security-scene-20261006T163254Z`, 1% volume, no focus steals logged) now folds the room cameras and removes the faceplates before security_02. At 18.05 s, both the original and Bevy Barney turn their heads toward the player; at 22.45 s both face the consoles. Remaining visible differences: the original still wears the separate `helmetBack` prop, his face animates (flexes, not implemented), and our Barney renders about 5 units beside his mark although the simulation reports his feet exactly on `mark_barneyroom_monitor_3` (by projection the original stands on it), so a root offset in his idle pose is suspected and not yet diagnosed. Comparison image: artifacts/head-security/native-vs-bevy.png.

## Pose-parameter blends, autoplay sequences and head control (2026-10-06)

Owned census: the shared human model `models/humans/male_shared.mdl` defines pose parameters move_yaw, aim_yaw/pitch, body_yaw, spine_yaw, neck_trans and head_yaw/pitch/roll. Its hidden sequences body_rot_z, spine_rot_z, neck_trans_x and head_rot_z/y/x are DELTA|POST|AUTOPLAY three-way blends, one per parameter, with reversed authored ranges. Barney's gesture sub-sequences are 3x3 blends on gesture_height/width. Rigs now keep model pose parameters merged by name, every blend animation of a sequence (`BlendGrid`) and the autoplay list; autoplay sequences always load. Sampling follows SDK Studio_LocalPoseParameter (local range, loop wrap, group index estimate) and BlendBones (QuaternionBlend, position lerp, bones with sequence weight). Selections whose animations are all zeros skip SlerpBones like CalcPoseSingle returning false, which also stops all-zero non-delta sequences pulling bones toward bind. Composition order follows C_BaseAnimating::StandardBlendingRules: base, gesture layers, then autoplay (cycle = time x Studio_CPS, weight one); sequence transitions, IK and bone controllers are not implemented.

Head control follows CAI_BaseActor::MaintainLookTargets/UpdateHeadControl, scaled from the 0.1 s think to the 15 ms step. Each tick's look interests blend by interest into one head direction, and the goal decays by 0.8 per think. With scene_clamplookat it eases by debounce 0.3 at up to 30 degrees yaw and 10 degrees pitch per think, then clamps to the pose-parameter range. Approximations: the eye is the origin plus 64 units and the "forward" attachment is the body yaw. Chest-bias edge limits, head roll and random/synthetic interests are not implemented. The Bevy pose cache keys on the combined gesture/head signature.

Tests: 312 normal and 21 owned-install tests, strict Clippy/fmt. Synthetic: reversed-range blend mapping and interpolation, all-zero skips, and head think rates and relaxation. Owned: Kleiner's rig lists the six autoplay sequences; head_rot_z is a three-blend grid on head_yaw; default values are neutral and turning head_yaw moves bones. The first expected value for the reversed-range test was wrong (blend 0 belongs to paramstart); the code matches the SDK. **Attention: 26 of 26 again.** Barney's body faces the monitor while his head turns over his shoulder toward the player (artifacts/head-attention/bevy-attention-player.png). Regression batch: attention 26, movement 26 and weapons/doors 17 assertions pass. Weapons, Kleiner and campaign captures are pixel-identical to 28c0bb3. Movement (751 pixels) and Breen (3,383 pixels, feed 6,654) differ deterministically, the same as the autoplay-only batch: small changes on NPCs and in animated monitor/screen content from applying the autoplay sequences, which inspected crops show as no visible regression. An earlier batch with autoplay alone (default pose values) showed small pixel changes only around NPCs, monitor feeds and animated screens, with no visible pose regression.

The native oracle setup now also fires `logic_disable_cameras`; by security_02 the campaign has folded the room cameras (owner observation), so the folded Bevy camera is correct for this point.

## Scene FACE, MOVETO arrival distance and the Combine camera model (2026-10-06)

FACE events follow SDK ProcessFacingSceneEvent for standing NPCs. The yaw at first process is latched (and re-latched when the moving state changes); the ideal yaw is `initial + AngleDiff(goal, initial) * intensity`, and the motor turns toward it at MaxYawSpeed 45 per 0.1 s (450 degrees/s; CAI_BaseNPC default, no override in npc_barney, npc_playercompanion or ai_playerally at the pinned SDK commit). Facing while moving (AddFacingTarget) is not implemented. MOVETO keeps the hull half-width tolerance (13) only for the "already there" decision. While moving, the NPC walks until within the event's arrival distance (SetArrivalDistance), measured horizontally, and a blocked step within tolerance counts as arrival. The first version measured in 3D and ended at the mark as NoRoute because the target is 8 units above the floor; a unit test caught it. `npc_combine_camera` now uses `models/combine_camera/combine_camera.mdl`, confirmed beside the class name in retail server.dll strings; its open/closed activities are not driven, so it renders folded.

Tests: 309 normal tests (new FACE test), strict Clippy/fmt; earlier in this round 21 owned tests passed. Packaged security fixtures: Barney arrives exactly on his mark (-3337 -318) and turns to face the monitor, as in the original. Movement 26 assertions pass. **Attention: 24 of 26** ("player two eyes use authored target" and "player current camera eye" fail). In that fixture FACE 1.47-8.71 s now turns Barney's body toward the security monitor while LOOKAT `!player` (6.07-8.74 s) asks for the player. The original turns his head toward the player with head pose parameters; we have no head pose control yet, so the player falls outside the eye cone of the body-facing head. The fixture passed before only because Barney never turned. Head pose control is the next task. A quiet regression batch on the FACE build was otherwise unchanged (the weapons difference is AR2 particle variation; the intermittent Breen 168-pixel difference recurred).

Unattended testing: Bevy `--volume 0.01 --no-focus`; the native oracle passes `+volume 0.01`, starts its window with SW_SHOWNOACTIVATE and returns the foreground to the owner's last window if the game takes it (no synthesized input; one return was logged in the first test).

## NPC door lookahead and Barney's security walk (2026-10-06)

The `trigger_cop_close_door_1` brush (model *80: ±8 by ±48 units around -3576 -512) crosses the corridor about 56 units before the doorway node. It unlocks `barney_door_2` for one second. Opening on contact came about 1.05 s later, after it relocked. Routes now record the door blocking each segment, and the NPC requests the open while walking that segment or within 96 units of its start. In packaged security_02 fixtures Barney walks from his corridor spawn through `introom_door_1` and `barney_door_2` and arrives at the desk (-3346 -315), layering posture05/gesture06np/bg_accentfwd at 18.05 s and posture01/gesture08 at 22.45 s. The native/Bevy side-by-side now shows both Barneys at the consoles. Remaining differences: native Barney stands nearer the console and faces it, because scene FACE events are unimplemented; the native frames predate the faceplate fix (mask still on); the wall Combine camera is missing; and the desk-top material looks lighter. Tests: 308 normal tests, strict Clippy/fmt, packaged movement 26 and attention 26 assertions. The full regression batch was not rerun for this change. Bevy `C7345632D8802BF07B5EC3BCF68E461A87A9C8F4F7D084ECF8637FC31FF9BC94`, retained `3C13760B1364DBC840811DDCCA20D71D494A38914870C71FFFC882C32D3AC08E`. Evidence: artifacts/lookahead/, artifacts/lookahead-attention/.

## Trigger toucher filters (2026-10-06)

`trigger_once`/`trigger_multiple` now follow CBaseTrigger toucher flags: clients (1), NPCs (2) and everything (0x40). A visible, live NPC whose feet are inside the brush activates NPC triggers and becomes the activator. Previously the player activated every trigger, including NPC-only ones. Other trigger classes keep player-only touch. The security fixtures now also enable `trigger_cop_close_door_1`: the campaign's security_intro_02 enables it before security_02, and the fixtures skip that scene. Barney still stops at `barney_door_2`: the trigger's 1-second unlock/relock window does not yet line up with his door request, or his feet are outside its brush when he reaches the door. Not diagnosed yet. Tests: 308 normal tests, strict Clippy/fmt. Packaged movement (26 assertions) and campaign captures are pixel-identical to the 28c0bb3 baseline; campaign keeps its 25 known material errors. There is no dedicated unit test for NPC touch yet, and the full regression batch was not rerun for this change. Bevy `DAFBB8E32B1E7107ECDE9929A2F676CD7D265BAE18372456B3CB14174D599BC6`, retained `656591DA7E8A77BD09B30A79E085F1223C33F0EDE55EE9531598BC6F608569AE`.

## Native security-scene oracle, templates, prop flags and NPC doors (2026-10-06)

Native oracle: private wrapper `original-oracle/run_security_gesture_oracle.py`. It refuses to run while any hl2 process exists, writes only to the private game folder, uses the official `-hijack` command route, and records loaded module hashes and stock cfg hashes. Every session closed its own process with settings unchanged. The setup lets the G-Man intro finish at host_timescale 4; deactivating script_intro and the view cameras was rejected because its relays re-enable them. It then force-spawns `gordon_cop_template`, turns off the half-mask faceplates (by security_02 the campaign has run ss_Helmet_Reveal), disables triggers, and starts security_02 at timescale 0.25 with pause/capture/resume at measured scene times 13.56, 18.05 and 22.45 s.

Rejected oracle attempts, all recorded: intro camera still active; `ent_keyvalue origin` does not move a live NPC; Barney absent before ForceSpawn; `ent_teleport` through the forwarded command moved him out of view instead of to the aim point; cfg names containing the scene time (`start-7.65`) were treated as having an extension and never executed; restarting the scene per capture repeated the MOVETO. Native Barney walks from his corridor spawn through two doors to the desk.

Findings against Bevy (same camera): (1) Barney stayed at spawn: his MOVETO was `NoRoute`. Diagnosis found a valid 8-node graph route rejected at two prop_door_rotating links and at a link blocked by a `prop_physics` desk. (2) That desk has spawnflag 8 (motion disabled), but Rapier simulated it and lifted it 27 units out of initial overlap. Start-asleep props (flag 1) also settled at load. (3) The wall Combine camera is not rendered. (4) The mask difference was an oracle setup error, now fixed: Bevy's unmasked Barney is correct for this point in the campaign.

Changes: point_template children (without spawnflag 1) start dormant and are created by ForceSpawn, which fires OnEntitySpawned once. Repeat spawns are reported unsupported. Barney now appears only when `trigger_once` 358 or a fixture force-spawns `gordon_cop_template`; all Barney fixtures send it at tick 1, and `actor_pose` moved to tick 2. Motion-disabled prop_physics are fixed colliders and start-asleep bodies start asleep. NPC routes accept graph links whose only blocker is a door entity, with a 128-unit cost. On contact, the NPC opens an unlocked door away from itself through the shared door-use path. Barney now opens `introom_door_1` and reaches `barney_door_2`. That door starts locked; the campaign unlocks it with the NPC-only, start-disabled `trigger_cop_close_door_1`. Our triggers do not respond to NPC touch yet, so Barney stops there.

Tests: 308 normal and 21 owned-install tests, strict Clippy and fmt. New synthetic template test. The owned security tests now force-spawn Barney like the campaign; the first owned run failed because security03 expected Barney at map start. The first-map speech census force-spawns templates before resolving speakers; its first run listed Barney and templated `!Target` speakers as unresolved. A first attention rerun failed (monitor eye target `None`): `actor_pose` ran in the same tick as the queued ForceSpawn, while Barney was still dormant. Packaged Bevy `535C7027613E9EF07D9C68D12203F08468BF9B2A35B7C91634E048063E0EA433`, retained `26542ECE056B97804F5177FFECDF9944FA682D7F2DB3ED8D3E5253F7077CA4C4`. Regression batch: movement 26, weapons/doors 17 and attention 26 assertions pass. Movement, weapons, Kleiner and campaign captures are pixel-identical to 28c0bb3. Campaign keeps the 25 known station03 material errors. Breen again differs by the same 168 bright-area pixels (box 1081,552-1280,671) with no gesture layers; that difference has now appeared in 2 of 4 runs, so it is an intermittent, unexplained presentation variation, not a gesture effect. One earlier weapons run differed inside the AR2 explosion flash and this run is identical (particle variation).

## Scene gesture layers (2026-10-06)

Choreography GESTURE events now drive layered poses in both hosts. Each fixed tick, a started gesture inside its event window updates a per-actor layer. The playback fraction is `(scene time - start) / duration`. The weight is event intensity times the SDK posture spline: gestures use one, postures lose 0.2 per tick while the actor has a scene movement animation. After the event ends, the layer fades with RemoveLayer's 0.1 s rate, or 0.5 s when the scene is canceled. The model cycle comes from the SDK playback-to-original tag retiming: Faceposer loop tags are linear, and ORIGINAL tags are repositioned from model tag frames (`numframes - 2` divisor, 0.05 threshold). `Scene::actor_matrices` composes the base clip, then layers by channel and start order. Bevy rendering, Bevy eyes, impact decals and the retained viewer all use it. The Bevy pose cache key now includes a layer signature. MDL Faceposer keyvalues are parsed into clip metadata. The per-rig load budget is now 1,024 clips plus about 4 Mi sampled poses, replacing the 64-clip cap. Skipped clips are named in warnings.

Tests: 307 normal and 21 owned-install tests, strict Clippy and fmt pass. Synthetic tests cover SDK tag retiming (linear loop segments, eased edge segments, mismatched or missing tags), layer start/update/fade rates, posture suppression, Faceposer tag repositioning and composition. The owned test plays security_02 on d1_trainstation_01 with Barney's model. Layers appear, playback and weight stay within 0..1, composed bones move, and a cancel drops every layer within 0.5 s. Its first run was rejected: the parsed test world has no model instances, so no clips loaded. The second run showed a real gap: 27 requested gestures plus their children exceeded the old 64-clip budget and `gesture13` was dropped. The budget change fixed it, and that rig now loads 73 clips without warnings.

Packaged Bevy `D07357BA57F95EA37CB747CD9E1297EDC402AA08D1EA7ECE045457A260B33A96`, retained `266A678F385627BCB006B166CA7BB2C05AB189B39B367941529D6B7B1F18C5AA`. A 1,600-tick security_02 timeline (test-inputs/bevy-gestures-timeline.json) reports gesture layers for Barney (gesture13, gesture07, bg_accentup, posture02, gesture01np...), Kleiner and the G-Man actor, with no load warnings; the final close-up renders without distortion. The 28c0bb3 movement, weapons, Kleiner and campaign captures stay pixel-identical. One Breen capture differed in 168 pixels (at most 4 levels) with no gesture layers active; an immediate repeat was pixel-identical to the baseline. See the later entry: the variation is intermittent. Movement 26, weapons/doors 17 and attention 26 assertions pass. A retained station01 smoke capture completed. Native original-game pose comparison is **not tested** yet.

## Shared animation layer composition (2026-10-06)

Source sequence composition is now shared in modkit-core: Source QuaternionScale/aligned QuaternionMult, delta pre-multiplication (QuaternionSM), STUDIO_POST post-multiplication (QuaternionMA), per-bone sequence weights and non-local autolayer ramps (spline, cross-fade, no-blend). `Rig::accumulate_pose` follows the SDK AccumulatePose order without IK, local-context, world-space or pose-parameter layers; those return errors or are skipped as the SDK does. source-assets now keeps raw delta offsets instead of converting them against the bind pose, treats all-zero animations as identity/base frames, preserves sequence flags, fade times, bone weights and named autolayers, and loads autolayer children as dependencies of requested sequences. Standalone delta clips are composed onto the bind pose by `Rig::matrices`.

Retail evidence (private, static): the `anim_3wayblend` ConVar leads through raw initializer bytes to one client.dll function each in client/server; its caller client 10092230 has the SDK AccumulatePose structure. Its callees 10092b50 (AddSequenceLayers) and 10098720 (SlerpBones) agree with the SDK branch by branch, including the scale-then-multiply order for delta and post layers; float constants were read from PE bytes. Names are inferred. This is a decompiler/SDK comparison, not a native runtime trace. Two earlier constant-pattern scans found nothing and are recorded as rejected.

Tests: 302 normal and 20 owned-install tests (`cargo test --locked --workspace`, then `-- --ignored` with HL2_ROOT), strict workspace all-target Clippy and fmt pass. New synthetic tests cover quaternion scaling/alignment, delta versus post order, bone masks, every autolayer ramp case, recursion/local/missing-child errors and standalone delta playback. The owned test loads Barney `g_pointRight`: six loaded children, an all-zero parent bone mask, delta children with finite frames and composed poses that move bones away from bind. The first strict Clippy run rejected a tuple return type; a named struct replaced it.

Packaged checks: Bevy `65565B2FF438513DE88780BF8B311EC657C0B713A56A83E92AE2BD498E3B5380` and retained `981E0EFEDE024A4F5073CF90A084785FD266CA4D0FEF4366BE9B9C5B467313D7`, shaders unchanged. The five 28c0bb3 partition fixtures (movement, weapons, Breen, Kleiner, campaign) were repeated with identical arguments; all captures and both monitor feeds are pixel-identical to the accepted images. Movement 26, weapons/doors 17 and attention 26 assertions pass; campaign keeps the known 25 station03 material errors. A retained station01 smoke capture rendered actors normally. The first replay batch failed to start because cmd does not search the working directory in this shell; the corrected runner uses an absolute launcher path. Scene gestures are not executed yet, so no gesture is visible in game.

## Continued private discovery and optional scope (2026-10-05)

The final continuation through pass14 supersedes these earlier counts. Its audits check13source datasets and every normalized row:133,556export variants,133,473observed addresses,51,823candidate/control rows,268,939recorded calls and94,824strings. All source database hashes match; original project50files/633,910,865bytes are unchanged. Module/dataset identity and real/synthetic exact retrieval/refusal controls pass. Thirty-nine failed variants are retained; no type/implementation/native-verification flags were adopted.

Pass07 audits36,509artifacts/8,992bodies and8,992independent rollbacks. Pass08 audits436artifacts/62bodies and rollback for all2,462targets. Pass10 checks16x87 contexts/recovery bytes/original metadata; the two-double-width pilot fails. Pass11 checks2,333data units/2,074relocation words. Pass12/13 export10/34existing-function controls with unchanged metadata;45/46orphan dispatcher/seed cases remain unmatched. Navigator omissions/range errors and rejected mixed/incomplete indexes are preserved. These checks do not establish a complete native denominator, ABI or gameplay parity.

Recorded command from the active checkout:

```powershell
C:/Python310/python.exe ../../work/hl2-decompiled/scripts/extended_native_index.py audit --root ../../work/hl2-decompiled/coverage-pass-20261005-14 --db ../../work/hl2-decompiled/coverage-pass-20261005-14/native-index-v5-23cee333e05b5f7c.sqlite3
```

Result: passed,13datasets,126identities,42modules with exports,133,473observed addresses and zero adopted completion flags. Reports are fresh-write only; repeat with `reaudit_native_index.py` using the same root/database and `--out` pointing to a fresh private directory. Final repeat reports are under pass14/reaudit-final-20261005.

Public checks passed: `git diff --check`, private publishing/validate_decompilation_handoff.py and audit_public_snapshot.py against this checkout. Six documentation files changed; all 214 tracked whitelist files, local links, ignored native/generated examples and unchanged package hashes passed their checks. The publication snapshot had zero failures and two existing absolute-path warnings. The 2026-10-06 commit cleanup repeats these checks with validate_decompilation_handoff_20261006.py and a fresh publication snapshot. Runtime tests, Clippy/builds, owned parsers, gameplay/captures and native comparisons were **not rerun** for documentation-only changes. Earlier runtime results retain their original revision/scope.

The original project/catalogue remains unchanged. On a private clone, 9,850 additional candidate entries export with zero failures; before/after executable-gap and pointer accounting is recorded. The discovery ledger audits 20,842 artifacts, 30,275 candidate attempt records, 47,512 table slots, 9,850 bodies, 13,266 recorded calls and 7,001 strings with zero discrepancies. The discovery navigator indexes all 9,850 bodies but has one range mismatch. Exact VPhysics/client retrieval and a scoped navigator control pass. An initially chosen client control lacked defined instructions and was correctly absent; later bounded disassembly provides its separately catalogued body.

Raw-prototype-flow and bounded-disassembly passes audit 34 and 11 temporary exports respectively. Independent read-only checks verify rollback for all 34 entries and restoration of function/instruction absence for all 15 disassembly targets. Ambiguous entry/shared-tail cases remain unresolved.

The 17 optional modules contain 23,987 saved entries: 23,982 original bodies, five original errors and five separate recovery bodies. All five original signatures remain unchanged; final saved entry sets equal the export snapshots. Their ledger audits 393 artifacts, 23,992 export/attempt rows, 51,968 recorded calls and 17,222 strings with zero discrepancies. Its navigator omits 2,544 original bodies and has 779 range mismatches. Exact Vulkan failure/recovery and Bink factory retrieval pass. The initial missing project-directory failure and Vulkan exception-handler analyzer error remain in private logs. Recovered ABIs, exception coverage and actual optional-loader activation are **not tested** against original-game behavior.

From private `work/hl2-decompiled`, executed integrity/retrieval commands include:

```powershell
C:\Python310\python.exe scripts/discovery_ledger.py audit --db coverage-pass-20261005-02/native-discovery-v3-208cb39c195a81df.sqlite3
C:\Python310\python.exe scripts/discovery_ledger.py query --db coverage-pass-20261005-02/native-discovery-v3-208cb39c195a81df.sqlite3 --module bin/vphysics.dll --address 0x10001000
C:\Python310\python.exe scripts/optional_evidence_ledger.py audit --root coverage-pass-20261005-03 --db coverage-pass-20261005-03/native-evidence-v4-bc332d20089c50b1.sqlite3
C:\Python310\python.exe scripts/optional_evidence_ledger.py query --db coverage-pass-20261005-03/native-evidence-v4-bc332d20089c50b1.sqlite3 --module bin/shaderapivk.dll --address 0x1006ae3d
C:\Python310\python.exe scripts/module_evidence_ledger.py audit --root coverage-pass-20261005-04 --db coverage-pass-20261005-04/native-evidence-v4-1d7e205973063ce0.sqlite3
C:\Python310\python.exe scripts/module_evidence_ledger.py audit --root coverage-pass-20261005-05 --db coverage-pass-20261005-05/native-evidence-v4-552c8d9f7686b145.sqlite3
```

Expected and recorded results: audit status passed/zero discrepancies; exact path/hash/address queries preserve original failures and identify successful body SHA256s. Naming/types, Rust implementation and native verification tables remain empty. Native tools, databases, binaries and recovered C stay outside public source checkouts. Runtime source/packages are unchanged; gameplay rebuild/tests/Clippy/performance are **not tested** again for these documentation edits.

## Private decompilation database pass (2026-10-05)

This pass validates private research integrity, not gameplay. It hashes 126 installed DLL/EXE paths and records 632 static/delay import edges; all 22 baseline private and installed hashes match. Read-only Ghidra discovery counts equal the original 87,793 identified functions. The three separate loader-referenced module exports contain 1,091 identified functions/1,090 successes/one failure. Eleven baseline/extension failures have supplemental bodies after unlocking inferred input metadata, with eleven original-signature rollback proofs. Recovered ABIs and native semantics are **not tested**.

From private `work/hl2-decompiled`, the executed catalogue commands were:

```powershell
C:\Python310\python.exe scripts/native_catalog.py audit --db native-catalog/hl2-native-v1-f37c8da5d5e019b1.sqlite3
C:\Python310\python.exe scripts/audit_extension_gamedb.py
C:\Python310\python.exe scripts/coverage_ledger.py build
C:\Python310\python.exe scripts/coverage_ledger.py audit --db coverage-pass-20261005-01/native-evidence-v2-6d6fa488a250ac9a.sqlite3
C:\Python310\python.exe scripts/coverage_ledger.py query --db coverage-pass-20261005-01/native-evidence-v2-6d6fa488a250ac9a.sqlite3 --module bin/vphysics.dll --address 0x100977ad
```

Recorded results: original catalogue passes before/after with unchanged SHA256 and zero discrepancies. Supplement audit passes 301 exact artifacts, 1,124 evidence rows, 1,090 extension bodies and 11 recovery bodies; naming/types, Rust implementation and native verification tables have zero rows. Exact recovered VPhysics/Speex retrieval files match their saved bodies; scoped gamedb CreateInterface read matches its whole normalized export. The fresh navigator indexes 827 bodies, misses 263 and has 36 range mismatches. Its original 22-module omissions/range report was not re-audited. Build/audit scripts create exclusive versioned outputs; use the existing reports for this completed pass rather than overwriting them.

Discovery is incomplete: 1,459,199 initialized executable bytes lie outside identified baseline function bodies; 13,830 unresolved pointer-slot observations and 133,588 computed calls remain. Uniform bytes and shared tails cannot be counted as missing functions. No candidates were promoted. Rejected debug setup, return-type-only recovery, basename-path catalogue build and raw-newline range audit remain in private logs/reports. Runtime code, packaged executables and build metadata are unchanged; gameplay/build/Clippy/performance were not rerun for public documentation edits.

## Native crosshairs, launchers and optional spatial batches (2026-10-05)

All294 normal workspace tests and19 owned-install checks pass, with strict workspace all-target Clippy and formatting. New tests cover monochrome bitmap row padding/bounds, native pistol/SMG offsets at720p/1080p and an odd viewport, exact partition triangle payloads/winding/spanning bounds, malformed indices/attributes and chunk-budget fallback.

The actual owned original game was run at1920x1080 using its installed engine/client/server modules. Native white has five single-pixel dots over22x22 pixels; pistol and SMG have five over21x17. All seven packaged Bevy white/pistol/SMG captures at720p,1080p and borderless match those offsets. Both new Bevy launchers produce1920x1080 captures on this desktop. Windows crosshair setup uses the owned font through a private in-memory GDI registration, releases handles before gameplay and draws nearest-sampled monochrome bitmaps at integer positions. Other HUD fonts, glow, colors/tone mapping, non-Windows platforms and untested native reticles remain outside this pixel claim. The native test process closed and six installed settings remained byte-identical.

Spatial splitting is opt-in. Packaged movement26 and weapon/door17 assertions pass; another18 checks preserve selected Breen/Kleiner feeds, both active view clusters, camera cleanup, two authored landmark/inventory transfers and pose agreement. Station03's25 unsupported-material errors exactly match the prior campaign fixture; they are not newly missing textures from splitting. Plaza on/off images are pixel-identical. An initial hall camera inside a pillar is rejected as visibility evidence. Broadcast staging, complete choreography/AI and ordinary campaign completion remain unfinished.

Bevy release SHA256 `78348779124CD24019E41969A6EBC33D61EE2852C96CA4673C02DA9736FA9D9D`. Captures/reports stay in ignored artifacts/crosshair-*-final and partition-*-final; original-game captures/module hashes and research helpers remain outside the checkout. No owned font or game files are distributed. See [performance scope](performance.md).

## Model activity lookup and NPC idle (2026-10-05)

All291 normal tests and18 owned-content checks pass, with strict workspace all-target Clippy, formatting and diff checks. New tests validate sequence-relative activity names and signed weights, label priority, weighted ticket intervals, negative-current retention for explicit activity selection, cohort merge order, invalid random intervals and actual scripted/post-idle deadlines. Owned Kleiner/Police activity candidates decode to finite poses and expected model-authored names/weights.

The packaged station01 lab-room capture shows Kleiner playing idle_subtle instead of an unloaded idle/bind pose. Seven checks verify implicit selection, ACT_IDLE input resolution/restart, clock advancement, capture completion, textures and pose agreement. The14 existing station02 idle_baton metrocop reports remain loaded; that is a valid label and was not replaced. Packaged movement26 and weapon/door17 assertions still pass. The rebuilt retained host reports Kleiner's selected clip. Its lab-room captures show occluding geometry even after correcting a feet/eye coordinate mistake, so they are rejected as actor-image evidence. The accepted actor capture is from Bevy.

The profiled1080p station02 spawn averages154.96 FPS over780 frames after120 warm-up frames, preserving the PVS improvement. This remains a room-specific measurement. Bevy SHA256 `BF4D9D568EAC552375CEF3653263760D8DA7953F48699092B85172C8535A9B98`; retained `6E379B5184F1D1F41A2D091F4C3D9BC33D7CDEC2C37FDCB8E45A19F794BD820C`. Evidence is in ignored artifacts/activity-* and performance-activity.json.

This adds label/activity resolution and repairs unsupported implicit idle labels while preserving explicit defaults and existing loaded poses. Native RNG/prediction streams, activity modifiers/AI translation, idle reselection, complete virtual sequence IDs, blends, gesture layers, facial animation and campaign staging remain unfinished. No ordinary first-level completion or complete NPC AI is claimed.

## BSP PVS and packaged culling (2026-10-05)

All288 normal workspace tests and17 owned-install tests pass. Cached/raw queries agree on1,200 bounds across the first three station maps. Synthetic tests cover active player/monitor view union, current moved bounds, coplanar bounds, unchanged-result caching, script-hidden entities and missing-cluster fail-open behavior. Strict workspace all-target Clippy, formatting and diff checks pass; the final cache change repeats the affected visibility/animation tests.

Packaged movement26, weapon/door17 and visibility/monitor/campaign/retained20 assertions pass. Player and off-map monitor view clusters both participate in culling; Breen and Kleiner feeds remain visible and were inspected. Breen's slate and unstaged Kleiner scene remain known limitations. Barney's inspected actor region is pixel-identical to the previous GPU reference, and eye projections agree. The spawn PVS-on/off images are pixel-identical;824 of886 tagged draws are rejected. Two controlled trigger transitions preserve landmarks and pistol17→16 inventory, recreate the PVS resource for station03 and retain six host cameras with no pose/visibility disagreement. This is not an ordinary campaign completion test. The retained projectile replay also completes after shared-parser changes.

Matched-package profiled1080p spawn runs average66.69 FPS with --no-pvs and156.94 FPS with PVS after120 warm-up frames. Rendering preparation falls5.86→1.52ms; graph CPU wall interval falls7.39→3.32ms. Other positions and sustained144 FPS remain unverified; see [performance scope](performance.md). Geometry bounds, player/monitor PVS and frusta are implemented; area portals, occluders, native LODs and spatial batch splitting remain incomplete.

Bevy package SHA256 `D18C13D43B018FA12B28FA96BEEAAD2BB8277CB7F4E09798A0448DD12D994438`; rebuilt retained package `2387434FB15580F4E05E19A7E330F365C624D65109123058E0C0CF6161D32C23`. Evidence stays in ignored artifacts/pvs-* and performance-pvs-*-final.json; private validation helpers/native research stay outside the checkout.

## GPU skinning and release performance (2026-10-05)

All 287 normal workspace tests pass, including normalized influence/basis parity and production joint/bounds checks through translated, rotated and scaled poses. The joint bounds enclose the CPU-skinned vertices without mutating bind positions, and linked joint entities are removed with their root. Strict Bevy all-target Clippy, formatting and diff checks pass. Shared parsers/simulation are unchanged from the 16 passing owned-install checks at the preceding milestone.

Packaged station01 close-ups compare CPU and GPU paths for Barney's monitor and player LOOKAT targets. Both eye projections/targets agree exactly in reports; the inspected actor region differs by less than 0.006 mean RGB channel values on the 0-255 scale. Dynamic props outside that region differ between runs, so whole-image identity is not claimed. Station01 uses 436 GPU-skinned surfaces, no CPU fallback surfaces and 2,769 joints; station02 uses 186 surfaces and 2,077 joints. The packaged door/weapon fixture passes all 17 existing assertions with no rendered/simulated pose disagreement.

Matched-package uncapped 1920x1080 station02 runs use 900 frames, a 120-frame warm-up and `--profile`. Over the remaining 780 frames, CPU skinning averages 35.05 FPS (28.53 ms), while GPU skinning averages 66.85 FPS (14.96 ms). Mean animation cost falls from 4.65 to 0.26 ms and asset preparation from 11.63 to 0.59 ms. These are RTX 3070/Vulkan local measurements with unchanged texture quality. Render preparation and graph/presentation still cost about 5.8 and 7.4 ms; 144 FPS and matched native performance remain unverified. Profiling adds diagnostic queries/timing fences. See [performance](performance.md).

Bevy package SHA256 `C9AFFB5B3058BF139C35F8A1EA6D948FBE6939E2BB6CC5C6A54311853BE0B0DD`. The retained executable is unchanged at `0A6AC5F3C4966ACD9C10AA239056C16399E69EACC49909C3CB79183CC4C9F987`. Private evidence is in ignored artifacts/skin-* and performance-skin-*-final.json. An initial attention capture was run on station02, where the station01 actor IDs do not apply; it was rejected as actor evidence and replaced with the correctly mapped close-ups above. Complete Source animation layers, visibility/areaportal/occluder behavior and campaign fidelity remain unfinished.

## Live monitor feeds and CPU performance work (2026-10-05)

All **285 normal workspace tests and 16 owned-install tests** pass, with strict all-target Clippy, formatting and diff checks. Incremental collision queries match 400 independent sweeps against full rebuilds across moves, rotations and enable changes. Packaged movement (26 assertions) and weapon/door (17 assertions) regressions pass. A further 22 checks verify live monitor captures, camera selection/state, zero missing textures, pose agreement, two landmark/inventory transitions and the retained projectile replay. Captures were inspected locally.

Bevy now renders the owned Breen and Kleiner camera rooms into a shared 256x256 target, with authored scanline overlays and a supported subset of material proxies. The Breen capture still has the map's Combine slate over the portrait, and the Kleiner scene is not staged correctly. These controlled feeds do not prove the normal broadcast or security scene works. Camera sky/fog, area connectivity, client camera ordering, recursive feedback and remaining TV noise proxies are incomplete. The retained host shares camera/entity state but has no live render-target adapter.

The performance regression remains material. Default-VSync 1080p measured about 30 FPS both before and after CPU improvements. Mean fixed simulation fell from 4.02 to 2.61 ms per tick; diagnostics fell from 1.39 to 0.022 ms per frame. A subsequent constant-pose upload fix plus no-VSync measured about 35 FPS; this is not an isolated comparison for that fix. GPU/render-thread cost and 144 FPS are unverified. See [measurements and reproduction](performance.md).

Final Bevy package SHA256 `443618608F260D49BF504FA457CD318104EF8F4833B56DBA137B329CA7CC8E16`; retained package `0A6AC5F3C4966ACD9C10AA239056C16399E69EACC49909C3CB79183CC4C9F987`. Evidence is in ignored artifacts/bevy-monitors*, retained-monitors-physics and performance*. Native decompiler evidence, databases, reference tools and captures remain outside the public source snapshot.

## Owned explosion emitters and developer overlay (2026-10-05)

The shared emitter adds distinct grenade smoke/fire/embers/debris, electric bounce/expiry sparks and two expanding AR2 shock rings. Five emitter tests cover emission/lifetime ranges, ring diameter/continuous geometry, pause-safe dispatch, delayed catch-up and bounded/reset state. All **278 normal tests and16 owned-install tests** pass, with strict all-target Clippy and formatting checks. Owned tests decode the nine particle materials with their intended blend modes.

Packaged Bevy captures `bevy-particles-fire`, `bevy-particles-smoke`, `bevy-particles-ball` and `bevy-dev-overlay` pass **26 checks**: actual weapon dispatch/ammo use, distinct combustion/smoke lifetimes, unique effect IDs, two live final rings, sparks, no missing effect assets, frozen clocks and visible live developer statistics. The retained host also completed `secondary-projectiles.json`, including both projectile types. F1 preserves the prototype's compact four-row header, amber control hints and bottom status bar using its MIT-licensed ProggyClean font. Packaged 720p/1080p captures were inspected, and real Windows F1 presses independently verified toggling on and off; the frame rate is a rolling real-time measurement, including paused frames, not a benchmark against Source. Screenshots were inspected locally.

Bevy package SHA256 `8B1F83391676239D2A0BFF31F396519390B6544555ADD7E0069F2FD0B70EFA50`; retained package `9A1FF807EBC705E1BD5408B2EB0AD2DDF8C2E3FBB80B67E256D5DBD9868AB5B6`. Native comparison attempts preserved stock configuration fingerprints but missed the requested explosion states; their captures are rejected as fidelity evidence. Emission constants are corroborated by private retail binary anchors and the pinned published SDK, rather than those unsuccessful captures. Ambient smoke tint, RNG, cached particle collision planes/random bounce damping, no-Z glow pixel visibility, exact beam tessellation/scrolling, soft-depth shading and particle-manager scheduling remain incomplete. The new effects are a verified implementation increment, not a 1:1 rendering claim.

## Retail animation research checkpoint (2026-10-05)

Private read-only evidence identifies the inherited retail actor eye-position entry through RTTI-associated tables, the m_latchedEyeOrigin datamap field and the literal eyes attachment path. The station gesture census resolves 65 nonempty event occurrences and their 311 automatic child-layer references, with 59 empty-parameter events recorded separately. This research changes no runtime behavior and does not count as a gesture playback test. See [research notes](research.md) for anchors, scope and remaining work. The latest code tests, captures and executable fingerprints remain those of the attachment milestone below; the subsequent documentation update requires no rebuild.

## Animated gaze attachments (2026-10-05)

273 normal workspace tests and 15 owned installed-game tests passed, with strict all-target Clippy, formatting and diff checks. The reader validates attachment tables, names, bone indices, supported flags, finite nonsingular transforms and view offsets. Owned Barney, Kleiner, human, G-Man and metrocop models decode attachment bases that match their loaded skeletons.

The rebuilt Bevy package repeated all 26 authored-attention assertions. Thirteen additional checks verify active animated attachments, finite unit forward vectors, pose-cache coverage and self targeting exactly 100 units along attachment forward. Monitor/player/self/final images were inspected. The new presentation used 30 actor pose samples for 40 eye draws and 29 attached NPC origins in this controlled scene; this is instrumentation of the eye pass, not an FPS comparison. A rebuilt retained-host scene/pause/cancel capture completed and was inspected. It continues sharing scene interests without the Bevy iris adapter.

Bevy package SHA-256 7F058DDC849FA4FBB4C7888D911F0AA57B1DD9C72CA49D049B5D7331D7762B37; retained 9845C39907430C898E0B772D57D6AEC37185A6AAA918DB05BB5041B5B6F151A3. Latest evidence is in ignored artifacts/bevy-attachments*. Published SDK attachment behavior and the subsequent reviewed retail attachment path support this adapter; complete native PVS/think-time latching and derived actor eye overrides remain unverified. Dynamic monitor textures remain unsupported and explicitly reported as absent procedural _rt_camera content. Head poses, layered gestures, flexes and ordinary first-level campaign completion remain unfinished.


## Authored LOOKAT attention (2026-10-05)

272 normal release workspace tests and 14 installed-game ignored tests passed, with strict all-target Clippy and formatting. The added owned security02 test checks named marks, monitor/player overlap, authored intensity, held scene time with continued interest refresh, cancellation tail expiry and target deletion. Synthetic tests check start-time binding, target aliases, missing-target diagnostics, non-NPC no-op, queue priority and zero-time ramp boundaries.

The packaged Bevy security02 fixture passed 26 assertions across monitor/player/self/end captures: both eye meshes consume the same authored world target, iris bases change, scene pause refreshes interests, UI pause freezes both clocks, overlapping events retain queue order independently of importance, and cancellation expires the tail. Captures were visually inspected. Barney's pose is explicitly seeded and unrelated entry triggers disabled before room entry; no ordinary campaign or native differential behavior is inferred. The retained packaged fixture verifies shared queue execution/pause/expiry, while its iris presentation remains unchanged. The combined packaged weapon regression passed 17 assertions.

Private pinned-SDK C++ curve reference comparison passed all 2,974 sampled points from 28 owned LOOKAT events; maximum absolute error was 0.000000476. This validates compiled default-curve math against the SDK, not the installed retail AI implementation. The retail catalogue supplies view-target network registration anchors only. Head poses, gestures, facial/eyelid flexes, lipsync, previous-target retention, random/tactical/synthetic interests, PVS gating and exact actor scheduling remain unfinished.

Bevy package SHA-256 627A5C0B7ACE97175771FD5D9655536A8C4DC6CD71589A92B6157F81AFB86014; retained package C4441464244EDBE0D170704E1D14A3A24DB33CDBC6F649B824D406643D018DC9. Evidence remains in ignored artifacts/bevy-attention*, with reference sources/harness/data kept outside public checkouts.


## Bevy shared pause/console and campaign host (2026-10-05)

All 267 normal workspace release tests and thirteen owned-install ignored tests pass, with strict all-target workspace Clippy, formatting and diff checks passing. Portable extraction preserves the parser/history command bodies; added regressions cover Unicode editing/completion, menu selection, input-transition consumption, setpos/getpos semantics, landmark placement, weapon deadline rebasing and input-only changelevel behavior. The first synthetic trigger test lacked collision brush planes; its fixture was corrected before the final passing run.

The packaged console/campaign fixtures pass 45 checks: cheat gating, quoted separators, Unicode input/deletion, queued entity input, frozen scene clocks, selection-before-pause cancellation, returning from Console to Pause, outside clicks, held-attack consumption and release, actual paused/resumed audio sinks, 720p/borderless1920x1080 captures, two real station exit transitions and failed/direct map loads. Station01-to02 arrival is (-4256,-152,64), and station02-to03 is (-5176,-4486,64), matching the owned landmarks. Consumed pistol ammunition carries 17 then16 rounds and the reset scene clock permits firing. A preliminary run exposed repeated station03 reloads from a no-touch changelevel brush; the final run has exactly two transitions. Source's published SF_CHANGELEVEL_NOTOUCH0x2 contract is the reference for this repair, not complete retail transition equivalence.

Successful loads remove old map draws/audio and replace resources; the final direct-map return to station02 observes233 entity mesh draws, zero pose/visibility mismatches, five cameras,972 live mesh assets and336 image assets. Those are bounded fixture counts, not total memory/performance measurements. Missing-map failure preserves player/scene state; console history/cheat state survives successful loads. Captures of the menu, console, direct map and final station03 view were inspected.

Bevy package SHA256 `2451FA2168C164D773A0D0E4F65DA5EF25AE963B070E041E7D2D8A1E9CDA8411`, built `2026-10-05T04:19:51.8087859Z`; shaders are unchanged from the prior effects milestone. The same package passes all17 combined door/weapon and26 bench movement assertions. Retained package SHA256 `F5783625C947D08DA441BF59A2F60C04781029ADE0CD662F4C55D900FFC6FDDA`, built `2026-10-05T04:21:58.6495258Z`, completed its existing timed pause/console replay with cheats/unknown/missing-map handling, zero unintended shots and inspected captures. The initial direct invocation omitted the retained CLI's `view` subcommand and was corrected before this successful run. Local evidence is artifacts/bevy-console-*, bevy-campaign* and bevy-console-regression-*.

Resume, Console and Quit are the implemented menu subset; save/load/options and the full Source registry remain missing. Input/layout migration and controlled teleported trigger progression do not establish ordinary campaign completion, native font/render parity or complete transition behavior. Saved entity/AI/global state, full player motion/crouch transfer, scripted attention/facial animation and first-level gameplay remain unfinished.

## Bevy projectile and impact presentation (2026-10-05)

All 260 normal workspace release tests and thirteen owned-install ignored tests pass, with strict all-target workspace Clippy, formatting and diff checks passing. The ignored checks require HL2_ROOT for the HUD fixture; the initial unset-variable run was corrected and the complete suite rerun. Added coverage includes bounded receiver projection, 256-mark retention, preload-only impact dispatch and a real trainstation door collision hit against its posed render mesh. No original-engine particle/studio-decal equivalence is claimed.

Five packaged effect fixtures pass 40 assertions: real observed grenade/decal draws, pose agreement, paused simulation, secondary-ammo consumption, delayed ball launch, contact explosion, bullet-mark raster changes and marked-door opening. Captures were inspected. Their functional package was `8FAB2598F1C4E9C0CAF1A8F1F512169F5FA2BDE9FED8B094905765F0203BB2B1`; the final report-wording-only rebuild is `E01AF842D27665F861A8892A981A3B3C78F5238D9F1E865B544182BEC2DA0DE0`, built `2026-10-05T03:45:24.7691506Z`, and passes the combined 17-assertion door/weapon fixture. The effects shader hash is `1A2755078D438BC13A90C558D873F75102343FCA3283B6C4BCEEFD15C665673C`. The retained host was rebuilt to `A858A78F2D4D11E6DFD82CA10976873489EC2C00F2AF293E4CEA841135565E42` and its bounded smoke capture inspected. Local evidence is under artifacts/bevy-effect-final-*, bevy-effects-final-* and bevy-effects-retained-*.

Five unused sand/shot base textures are absent in the tested owned installation and remain named preload errors. Sprite/model loads and emitted playback requests succeed in the fixtures. Marks follow current entity/rigid transforms, but later skeletal deformation is unfinished. Native particles, dynamic lights, sparks/rings and exact gamma/shader behavior remain incomplete. The door projection repair fixes a retained-host gap as well: raw bind vertices and an approximately one-unit collision/render separation prevented marks on this leaf.

## Deferred door opener inputs (2026-10-05)

The shared OpenAwayFrom input now resolves a named entity, player, caller or activator at delivery time and chooses the prop-door swing using its current origin. Missing names retain the default forward swing. Locks and explicit opendir override the target; repeated Open/OpenAwayFrom inputs do not retarget an already opening/open prop door. The published props.cpp handler corroborates this contract; native linkage/blocker handling remains incomplete.

All 258 normal workspace release tests and strict all-target Clippy pass. The prior eleven owned-reader checks remain valid: no asset reader changed in this increment. The packaged 330-tick test-inputs/bevy-door-inputs.json fixture passes eleven assertions for both opener sides, both leaves, closed collision, input support, capture completion and pose agreement. Its screenshot was inspected. Final Bevy package SHA256 `0CB871816976CDDC867AD2D2C8CF26E14C93270BE14AF275E6D53657C39E47AF`, built `2026-10-05T00:33:28.3574059Z`; shader hashes are unchanged. The retained package was rebuilt to `EA9182C7EFE35EED59574166ED8239AD4F31E7695F7BF51619BB596ACB811AF5`. A final packaged combined weapon/door rerun also passed all 17 assertions after the repeated-open guard. Evidence stays local in artifacts/bevy-door-input*.

## Bevy sky, eyes and paired-door milestone (2026-10-05)

All 256 normal workspace release tests and eleven explicit owned-install ignored tests pass, including bounded MDL eyeball parsing/projection and real Barney, citizen, G-Man and Vortigaunt metadata. Strict all-target workspace Clippy passes with the same two vendored warnings; formatting and diff checks pass. Logs are local under artifacts/bevy-sky-eyes-doors-{final-tests,owned-tests,clippy}.log.

Packaged Bevy executable SHA256 `3080C3F9478AE0F4895DF50D7F820AD829A04DF8EE1A7956B07F4C3F92D29823`, built `2026-10-05T00:19:33.2396796Z`. Source shader SHA256 `B8142E26AEB52878C7A93BAA8C43C734946FBFE347B67FB0D0F55B0CC70781E4`; HUD shader remains unchanged. Retained-host package `59637DF46F3E736DA2325BCFEAD4CB223BB39C8D4DFCD44B2F58E320898FC80A` was rebuilt after the shared door/sky changes and completed a bounded packaged smoke capture. The separate main checkout was not modified.

The packaged weapon/door fixture passes all 17 assertions, with paired use now completing both entrance leaves. The new 330-tick door-swing fixture plus two eye captures pass 14 checks: both leaves swing away from inside and outside openers, return to their authored closed poses, block the closed-door ray, and keep visible/collider poses aligned. Both Barney eyes select the player from either side and change their projection rows; other actors select visible NPC targets. The final door, two close eye views and outdoor/blocked sky captures were inspected. Evidence stays local under artifacts/bevy-door-swing.*, bevy-eyes-final-{left,right}.*, bevy-sky-final-{outside,blocked}.* and bevy-doors-eyes-assertions.json. Clouds and miniature Citadel scenery render behind the station; opaque closed doors and entrance pillars occlude them.

Eyes use owned sclera/iris textures and bounded animated-bone projection, corroborated with selected retail StudioRender methods and published shader references. Four Vortigaunt materials now use their explicit authored Eyes_dx8 fallback; the first-map material errors are the three unsupported dynamic monitors. Full EyeRefract, glints, eyelid/facial flexes, native attention queues and VCD LOOKAT remain unfinished. The nearest visible player/NPC policy is an explicit approximation. Sky rendering is LDR with leaf eligibility and independent world depth, without native polygon sky masks, HDR or fog. Paired door use respects fixed opendir overrides, but blocked-door reversal, native master/slave ownership and arbitrary initial positions remain unfinished. These controlled checks do not establish a complete campaign or native parity.

## Bevy shared audio milestone (2026-10-05 local)

Commit 4c443658 shares the retained symbolic sound resolver, actor-gender alternatives, WAV/MS ADPCM/MP3 decoder and viewmodel sound-event cursor. Bevy preloads referenced alternatives before App.run, emits owned PCM through monitored audio sinks, and pauses/resumes pending and active players. The recorded milestone passed 252 normal tests, ten owned-install tests and strict Clippy. Its packaged 625-tick weapon fixture passed 17 checks with 23 actual sinks started and no playback errors. The controlled first-map scene fixture started seven sinks including Barney speech, exercised two pauses and one resume, and ended with all five remaining sinks paused. All 155 registered wave references decoded; three unused named references were missing, with no emitted request failures. Logs/reports stay in artifacts/bevy-audio-*.

Audio milestone package hashes are superseded by the sky/eye/door packages above. Streaming, spatial mixing, Source DSP/soundscapes, channel semantics, native random selection and lip sync remain incomplete. Linux audio linking is unverified; Windows is the tested platform.

## Bevy shared HUD migration (2026-10-05 local)

The retained Source-resource HUD now lives in engine-independent `hl2-ui`. Both hosts consume its explicit CPU draw canvas; Bevy uses a separate 2D overlay camera with pooled meshes and normal/additive materials. Parsing/layout, font-cell metrics, glyph blur/scanlines, numeric animations, secondary-panel geometry, bucket selection and white-default remapping remain shared. File reads happen before the app runs. All 250 normal workspace release tests and ten owned-install ignored tests pass; strict all-target workspace Clippy, formatting and diff checks pass with the same two vendored warnings. Logs: artifacts/bevy-hud-final-{tests,owned-tests,clippy,build}.log.

Final Bevy package SHA256 `692041E2AFE8D6A6838960B136427CF76C419D8CBACF92193C9736DD77C667C0`, built `2026-10-04T23:18:16.0187428Z`. HUD shader SHA256 `AB587C5AB3A5022D24F0FDAA6872FC7561827C085C2F7705DB5AAB12CC54EE38`; Source shader remains unchanged. The final packaged crosshair captures at 1280x720 and borderless 1920x1080 each have exactly five single pixels at the accepted retail offsets, spanning 22x22. The final 345-tick HUD fixture retains pistol ammo while SMG is pending, changes to SMG, fires one grenade and ends with shotgun pending while SMG remains active. The inspected final image contains selection plus primary/ALT ammo; render diagnostics report 114 quads, 115 pooled meshes and 66 owned textures. These counts do not prove performance gains.

Start/intermediate/end secondary-panel captures were inspected on the preceding 4FE789 package with the same UI adapter and animation logic. Its door/weapon fixture passed 17 assertions and bench fixture passed 26; final source additionally calls the same draw_status observation before receiving the suit, updates CLI/report wording and adds the HUD fixture. Retained host package `F0603FA01C67FD4EB49CD2E6C5FD32DD83A4F3BD4CAB08B9F2AAF14B5EF05C06` passed the white-crosshair raster measurement and nine-capture hud-slide fixture, with no audio/texture errors; the SMG endpoint image was inspected. Evidence is local under artifacts/bevy-hud-* and hud-slide-*.

Native GDI font output, full hide gates/animation commands and exact linear-versus-gamma blending remain incomplete. Pause menu/console, audio, projectile/impact visuals and campaign host transitions still need migration. Underlying scene/AI/physics and Source-rendering limits remain; this is not a completed campaign or full original-engine parity.


## Bevy entity/gameplay migration (2026-10-04)

The shared simulation now retains entities, weapons, projectiles, selection, NPC locomotion and actor preparation; both hosts use the same implementation. The five moved full modules were compared byte-for-byte after newline normalization, with only scene_sound_request visibility widened to preserve a retained host test. All 249 workspace release tests and nine explicit owned-install tests pass; strict workspace/all-target Clippy passes with the same two vendored warnings. Logs: artifacts/bevy-gameplay-tests.log, bevy-gameplay-owned-tests.log and bevy-gameplay-clippy.log.

Packaged executable SHA256 `604244930786CD48B5C16FDB0F1C82C476FDB3572B7FA5DA60E628AC0DCD7238`, built `2026-10-04T22:47:33.7451967Z`. The shader remains unchanged. The packaged bench fixture passed all 26 existing movement assertions with rigid bodies enabled. The packaged station doorway/weapon fixture passed 17 assertions: closed/open/reclosed walking and ray behavior, E use, selection retaining the active weapon until confirmation, pistol consuming one round, SMG spawning one grenade/consuming one secondary round, and AR2 charging before launching/consuming one round. Final 233 owned entity mesh draws have zero transform/visibility mismatches. Private validators and captures stay outside public source. Closed/open doorway and AR2 viewmodel captures were visually inspected.

This validates host integration, not original-engine parity. Retained AI, choreography and physics limitations persist. HUD/audio, impact/projectile effects and campaign host transitions remain unmigrated; audio cues are explicitly reported as unplayed. Missing clips remain bind poses, dynamic monitors remain unsupported, and sky/Source shaders remain incomplete. No FPS improvement or playable first-level campaign is claimed.


## Bevy player/collision migration

The walking preview package is SHA256 `3167DB5CAA551628CC0CF07CC9588F5425907BCD8834E33FC58A1D32B57C448A`, built 2026-10-04T21:45:47Z. Its packaged shader remains `DF39B6090638EC5238CAE9F93A1F29198D15EDE2B45295348E66E2463A3CF249`. `bin/build-bevy-info.json` is authoritative after rebuilding. The static-only package below is superseded.

The collision adapter, private convex implementation and NPC probes moved unchanged into `hl2-simulation`; both hosts import the same modules. Their 25 tests are preserved. The workspace suite passed 243 tests; a final Bevy rerun passed 11, including an additional input-capture regression (244 unique normal tests across the final source). Strict all-target workspace Clippy passes. The Bevy adapter uses an explicit 15 ms fixed timestep, samples look/input before the fixed loop and publishes the eye position afterward. Capture/resume discards transition-frame pointer motion and suppresses held jump until release.

The packaged station02 movement fixture completed all 280 host ticks and passed 26 report assertions. It settled on the bench at feet Z 36.17742157; ordinary and airborne-crouched jumps retain the same eye height and velocity while crouching raises feet by 36 once. Clear air uncrouching restores the matching trajectory. Paused samples preserve the complete player state and tick count; held jump cannot jump again on landing until released. The final 1280Ã—720 capture was visually inspected. This is a controlled shared-player/collision replay, not an original-engine differential replay or campaign proof. Two named dynamic monitor materials remain unsupported; there are no model-load, texture-budget or capture-write failures.

Evidence stays local: `artifacts/bevy-movement.{json,png}`, `bevy-movement.assertions.json`, `bevy-movement-tests.log`, `bevy-movement-final-bevy-tests.log` and `bevy-movement-final-clippy.log`. Rendering still uses initial entity poses/bind poses. Rigid-body dynamics remain frozen to avoid moving colliders without their visible meshes. Weapons/HUD, interactive pause UI/console, NPC animation/AI, scene/entity I/O, audio and campaign progression have not migrated. See [migration instructions](bevy-migration.md).

## Bevy migration preview

The separate `bevy-migration` branch packages a static owned-map renderer, not the retained gameplay runtime. Final `bin/hl2-bevy.exe` SHA256 is `CD497A0BE933FEB94083C709A0CFC15E67C4877876C81A4EE9FAC902BA1ADD09`, built 2026-10-04T21:17:54Z. Packaged `source.wgsl` SHA256 is `DF39B6090638EC5238CAE9F93A1F29198D15EDE2B45295348E66E2463A3CF249`; `bin/build-bevy-info.json` is authoritative after rebuilding. See [build instructions and limits](bevy-migration.md).

The final source passes all 241 normal workspace tests (eight Bevy, 141 retained runtime, 34 core, 58 asset readers), all nine explicitly invoked owned-install checks, formatting and strict all-target workspace Clippy. Existing vendored miniquad warnings remain. New tests cover coordinate handedness/pitch, BSP-only winding normalization, material properties, visible-material selection and shared texture caching under an allocation budget. Existing gameplay tests validate the retained runtime; they do not establish gameplay in Bevy.

Both final packaged runs used `launch-bevy.cmd`, completed GPU capture at 188 frames, exited successfully and were visually inspected. Installed Steam build 19307283, patch 9912070, NVIDIA RTX 3070/Vulkan:

| Map and capture | Result |
| --- | --- |
| `d1_trainstation_01`, borderless 1920Ã—1080, Source camera (-4690, -1186, 32.03125), yaw 165 | 314 shared base textures, 273,979,396 decoded bytes (about 261 MiB). Model clock front restored after correcting the different BSP/model winding. Seven named unsupported texture cases remain: three dynamic monitors and four Vortigaunt eye materials. |
| `d1_trainstation_02`, 1280Ã—720, Source camera (-4304, -224, 1) | 233 shared base textures, 201,117,696 decoded bytes (about 192 MiB). Stairs, fences, props and walls render; two named dynamic monitor textures remain unsupported. |

Neither final report has texture-budget failures, model-load failures or capture-write failures. The 512 MiB cap is the adapter's decoded-base-texture budget, not a Bevy limit or a total memory measurement. An earlier preview failed that budget by decoding aliases separately and loading unused textures; it is retained as rejected evidence. A subsequent preview exposed wrongly culled model fronts and is also superseded. Bevy currently selects texture mips up to 2048 pixels versus the retained viewer's 512, so these are not equivalent performance comparisons.

Local captures/reports are `artifacts/bevy-station01-1080-winding.*` and `artifacts/bevy-station02-winding.*`. Static bind poses and initial door transforms are visible limitations. No movement/collision, weapons/HUD, NPC animation/AI, scene playback, audio or campaign progression is running in Bevy yet. Sky passes, Source material proxies, eyes, dynamic lighting, PVS/areaportals and accurate HDR/gamma remain missing. These captures do not prove Source rendering parity or improved performance.

## Retained runtime validation

This remains a partial Rust reconstruction. Iteration14's final release package is SHA256 07561D4CC09310D7C8837B95D44201C1F6B1871AEDCC6C016BD676669D74DCAD, built 2026-10-04T11:01:23.1984992Z and tested through launch.cmd. bin/build-info.json is authoritative after a later rebuild. The 1D9193BB preview below used the same behavior before the final test-source/format rebuild.

## Iteration14: actor speech and collision-checked scene movement

233 workspace tests passed (141 runtime, 34 core, 58 asset readers), as did all nine explicitly invoked owned-install checks, formatting and strict all-target Clippy. New tests cover dedicated NPC masks/self-exclusion, player/NPC blockers, doors, support/steps/drop limits, ground graph ordering, scene ownership, authored motion, cancellation and arrival-only gates. The ordinary Barney model and MDL eye-offset check catches the missing factory default found in the first packaged preview. The two pre-existing vendored compiler warnings remain.

The corrected 25-second Barney fixture passed 33/33 report assertions. It explicitly seeds the actor at the authored security02 monitor target; it does not reproduce or record the whole preceding retail campaign state. The owned run clip moves Barney, a live player blocks him, UI pause freezes actor/game/scene clocks, and SECTION holds the authored 6.8806338-second clock while the game clock continues. Moving the player away allows both real goals to arrive within their 3D tolerances and the actual scene output unlocks/opens the door. A second run verifies cancellation restores loaded idle and retains a fixed position. No scene Resume command bypasses this gate. Captures of movement and the opened door were inspected; audio/model/texture errors and simulation budget exhaustions are zero. Artifacts are local: iteration14-barney-grounded-report.json and its assertions. The earlier BF65 preview failed with MissingActor because Barney had no loaded default model; it is superseded.

The first-map speech census now resolves all 72 SPEAK actors across 44 scene references/40 unique scenes and 67 cues. All 75 registered wave files decode (71 PCM WAV, four MS ADPCM), with zero failures; 74 are eligible for those actual actors. Symbolic npc_citizen.pain04 now resolves the installed male actor's wave while preserving the female alternative. Literal WAV/MP3 emission does not implicitly expand gender templates. Native RNG equivalence, spatial mixing, non-English combined speech and lip sync remain unverified.

The final package repeated the Barney fixture (33/33 assertions) and G-Man actor/speech fixture (12/12). Its initial projectile rerun passed 19/20: the charge-veto capture at playback time8.0 occurred after the recorded charge deadline7.9849998, so it correctly observed a launched ball instead of a pending charge. AR2 stayed selected, clip27/reserve60 were unchanged, and the other19 checks passed. The fixture's capture now occurs at7.91, after its7.9 confirmation and before release; the failed report is retained. The earlier 1D9193BB preview passed20/20 and the compiled scene/sequence control33/33. Those runs had no audio/model/texture errors. The older scene control still uses manual Resume; the new Barney fixture is the movement-gate evidence. A final20-frame borderless 1920x1080 capture retains the verified default crosshair's five single pixels and22x22 footprint. The three older weapon/HUD fixtures remain historical iteration12 checks and were not rerun in this batch.

The corrected final-package projectile fixture passed20/20. Its charge-veto snapshot records simulation time7.9049998 against deadline7.9999998, with AR2 selected and clip27/reserve60 unchanged. Runtime weapon logic was unchanged; this corrects a capture scheduled exactly at the release boundary. Evidence: iteration14-final-package-secondary-timing-report.json and its assertions; the preceding19/20 report remains available.

Only ordinary Barney is registered for movement in this increment. The packaged room route is direct; graph routing has synthetic validation, not a matched retail route trace. Central forward gait steering, arrival-only SECTION resolution and conservative blocked retry do not reproduce native motor acceleration, turning, positive ETA preload, give-way, triangulation, dynamic links or hint locks. Weighted animation blends, eyes/facial shaders, gestures, attachments and the rest of the first campaign level remain unfinished. Barney's visible eye materials currently show the renderer's shader fallback.

## Historical iteration13 checks

| Earlier check | Result |
| --- | --- |
| Workspace unit tests | 210 passed: 122 runtime, 34 core and 54 asset-reader tests. |
| Explicit owned-install checks | All six passed: MP3, compressed G-Man speech, security03, G-Man actor events/model and authored walk/run records. These are ignored in the normal suite. |
| Formatting and all-target strict Clippy | Passed; the vendored miniquad dependency retains two existing compiler warnings. |
| Existing weapon/shotgun/selection fixtures | All 27 assertions passed on the preceding iteration12 package; their implementation is unchanged in this batch. |
| Packaged SMG/AR2 projectile fixture | All 20 assertions passed, including partial-clip AR2 charge-time reload/holster veto. |
| Compiled choreography | All 1,837 installed scenes and 15,114 events parsed privately with zero failures. The first map loaded all 44 referenced scenes in the packaged run. |
| First-map scene fixture | 33 authored-controller and sequence-sampling checks passed; explicit Resume bypasses missing movement readiness. |
| Packaged G-Man intro actor fixture | All 12 checks passed: owned model/rig, 17 actor events, two ADPCM voice decodes and navigation reports. Captures use a debug camera, not the original intro view. |
| PC navigation graph reader | Seven synthetic parser regressions pass. Owned first map: AIN37, revision 5366, 163 nodes/341 links/zero tail. |
| Authored locomotion data | Six new synthetic reader/sampler regressions pass. Owned forward walk is 80 units/1s; run is 125.87412 units/0.6s. No runtime locomotion or weighted blending yet. |

The latest G-Man fixture renders the previously excluded `cycler_actor` model and resolves all 17 authored actor events through 18 seconds. Both compressed voice files successfully request backend playback with zero audio errors: mono 22.05kHz, 146,740 frames (6.654875s) for riseshine and 461,823 frames (20.944354s) for gman_02. Output is trimmed to each original `fact` count. Synthetic mono/stereo controls compare independent signed sample values and reject malformed blocks, custom coefficient tables and excessive allocations. Complete blocks and an explicit `fact` count are required; other WAV codecs remain unsupported. These checks do not prove native mixing or that facial/gesture animation accompanies speech. The inspected capture is an idle G-Man in the map's offstage area; intro cameras, fades, compositing, flexes and gestures remain unfinished.

A wider iteration13 private speech diagnostic explicitly failed on the unexpanded `vo/npc/$gender01/pain04.wav`. That result remains historical evidence; iteration14's actor-aware registry and complete first-map wave census above supersede the template-resolution gap.

The navigation census on the earlier preview package, with identical graph-reader/report code, decoded 72 installed graphs and reported six BSP revision mismatches. It loaded 78/79 maps; the known zero-byte d2_coast_02 remains rejected. Reports treat missing/rejected navigation as optional data diagnostics instead of aborting world display. Graph decoding and movement curves are groundwork for NPC movement; they do not establish a working first level.

The crosshair fix corrects the screen accessor from width to height and follows the native surface's second UV inset. Fresh 1280x720 and borderless 1920x1080 captures from the latest 4BBA946E package each match the original's five single-pixel positions and 22x22 footprint, with zero audio/model/texture errors. A preceding 1281x721 capture checks integer-center rounding without claiming an original odd-size comparison. Tests cover the owned VTF POINTSAMPLE flag and non-square coordinates. Earlier E0A62CB0 controls used identical HUD code. A destination half-pixel experiment enlarged the dots and was rejected. RGB/tone mapping and native font rasterization are not claimed equal.

The current projectile fixture records three contact grenades and two energy balls, three grenade detonations, ball bounces and two expirations. Secondary reserves are consumed independently of magazine rounds. AR2 reserve stays unchanged during windup; attempted reload/switch cannot interrupt it, and expiry does not inflict radial damage. Grenade flight/explosion and ball flight captures were inspected. The run reports zero audio, texture, projectile-model or visual errors and no capacity/collision-budget failures. Rapier contact behavior, guidance, damage relationships/filters, underwater gates, dissolve animation and complete particles remain approximate or missing.

Accepted original-engine controls corroborate SMG held-trigger reserve progression from three to zero with primary ammo unchanged, its magazine-reload interruption, and AR2 delayed reserve consumption plus charge-time reload/holster veto. Native AR2 convars report radius10, mass150 and duration2. Captures/logs show one ball after release and none later; exact native trajectory, damage and expiry timing were not measured. Unfocused startup, forwarded wait commands and surplus ammo pickups contaminated earlier attempts; those captures were rejected. The original process closed and all six freshly hashed stock configuration entries matched. Three automatic stock metadata writes were recorded separately; this is not a claim that the entire installation was unchanged.

The scene fixture uses actual security03 SECTION6.8806338, Trigger3 7.013968, early STOPPOINT9.5939703 and tail11.1273012. It verifies scene-clock pause/resume, the locked door before its real trigger, opening to90degrees, early completion, tail completion and cancellation. A customs officer's installed motionright SEQUENCE verifies scene-owned sampling, pause freezing, resume and baseline restoration. The first NPC camera intersected a wall and was rejected; the fixture was corrected using owned BSP geometry. Gesture layers, actor movement/readiness, facial flexes, lip sync, camera/train intro and full AI remain unfinished. Unsupported behavior is diagnosed; the run has zero audio/model/texture errors.

Installed MP3 playback now decodes the trainstation cue to stereo44.1kHz, 3,997,440 frames (90.6449s) and successfully requests backend playback. The earlier door-music failure is resolved. Whole-file first decode can block; streaming, spatial mixing, soundscapes, DSP and paused audio remain open.

Current package evidence is under ignored artifacts/iteration13-*; iteration12-* retains the preceding HUD/native-comparison controls. Private native comparisons, cache census and validators stay in ../../work/hl2-decompiled and ../../work/publishing. No installed game data, native code or research database is a build dependency.

## Historical validation â€” 2026-10-03

This validates a partial Rust reconstruction, not a completed Source translation or playable campaign. The current packaged executable was tested through launch.cmd. Its SHA256 is B8B94759463E387362E6935174DE7CE07608E8218AAAB1956F243E2373622F31, built at 2026-10-03T09:26:51.6771832Z. bin/build-info.json is authoritative after a later rebuild.

## Automated checks

| Check | Result |
| --- | --- |
| cargo test --workspace --locked | 118 tests passed: 62 runtime, 21 common simulation/animation, 35 asset-reader tests. |
| cargo clippy --workspace --all-targets --locked -- -D warnings | Passed; two existing compiler warnings remain inside the vendored miniquad dependency. |
| cargo fmt --all --check and git diff --check | Passed. |
| Release packaging | scripts/build.ps1 compiled/copied bin/hl2-rs.exe used by launch.cmd and recorded its fingerprint. |
| Installed maps | 78/79 parsed. Installed d2_coast_02.bsp is zero bytes and is correctly rejected. artifacts/verification-iteration9.json records the result. Parsing does not validate campaign gameplay. |

Regression tests cover Source resource conditions and localization, font alternatives, numeric HUD interpolation/health pulses/cancellation, QuickInfo thresholds/audio latches, wheel/slot order, reload transfer/interruption, primary cadence/spread, shotgun primary/secondary pellets/pump/deadlines/delayed input, health/battery pickup gates and sounds, bounded MDL sound events and melee ray/hull/facing refinement. The melee geometry remains Rapier-derived, not Source collision equivalence.

The SMG1/AR2 tests additionally cover first-empty-click versus next-attempt reload, unchanged animation/primary deadline, strict elapsed idle-reload deadline and independent half-second sound throttles across release/switches. These use reviewed static native and published SDK evidence; no original-engine dynamic empty-fire oracle was run in this pass. Next-best-weapon ranking and secondary deadlines are still missing.

The first published GitHub Actions run passed formatting, strict workspace Clippy and unit tests on a clean Windows runner: [run 37095841557](https://github.com/kvalls/hl2-rs/actions/runs/37095841557), public commit 002dbde. That run covers the initial 97-test source; later commits receive their own runs. A separate fresh public clone also passed all-target workspace cargo check without private research files.

## Packaged weapon/HUD regression

launch.cmd --input-script test-inputs/weapons-hud.json completed 1,369 frames and approximately 41 simulation seconds on the BC12E81B package. artifacts/iteration8-weapons-report.json and iteration8-weapons-assertions.json are local evidence. Earlier matching checks are recorded under iteration6 and iteration7.

- 64 attacks, 82 traced bullet/melee impacts, 82 receiver-clipped marks and zero unclippable impacts. Three crowbar floor hits played two distinct crowbar impact WAV variants as well as the swing sound.
- Six installed model sound events: four .357 reload events and AR2 rotate/push events. The AR2's distinct engine reload sound also played. 200 audio requests and no decode/playback errors; one low-ammo warning WAV played as the SMG crossed its warning threshold, and one installed empty-click WAV played before its automatic reload.
- All six supported primary weapons were selected, fired and reloaded where applicable. Shotgun shell insertion/interruption/pump and SMG empty-clip automatic reload were exercised. SMG reserve transfer and final clip count appear in the report.
- Primary and secondary attack actions confirmed the pending weapon without increasing attacks. Holding primary attack for over one second after confirmation caused no firing before release. Cancellation retained the current weapon; expiry cleared the menu without switching.
- Stock resources determine the five small boxes/expanded column, colors, rounded corners, labels and icon fonts. Numeric armor at zero fades out. QuickInfo brackets are present during selection; the center crosshair is a separate HUD element. F1 development banners remain hidden by default.

The shipped test input is runtime testing data; no native decompiler scripts or game-derived assets are in the source tree. The starting map still lacks normal campaign weapon acquisition; F3/test loadout does not demonstrate campaign progression. The first pistol and revolver scripted clicks occurred before their draw animations ended and were correctly blocked.

## Packaged shotgun secondary regression

launch.cmd --input-script test-inputs/shotgun-secondary.json completed nine simulation seconds on the BC12E81B package: 277 rendered frames, six attacks, 52 traced impacts/receiver-clipped marks and 73 audio requests with no audio errors. It played two double-shot WAVs, four single-shot WAVs, four pump sounds and all three installed reload variants. Evidence: artifacts/iteration8-shotgun-report.json and iteration8-shotgun-assertions.json.

Held secondary confirmation left six shells and zero attacks until release. Double fire consumed two shells; a second request during cooldown was blocked. With one shell, secondary fire used primary attack. Holding secondary during a one-shell reload left reloading active. At two shells it interrupted reload, and the retained shot fired after release when the insertion deadline elapsed. The last case consumed two shells and twelve pellets. This does not validate Source prediction seeds, recoil or pellet hull traces.

## Physical wheel/click checks

The current package also completed test-inputs/selection-held.json: 203 frames, four attacks and 37 audio requests with no audio errors. Slot selection while secondary was already held confirmed pistol and shotgun immediately without shooting. A new primary click fired while the consumed secondary remained held. Secondary release/repress in one poll fired the shotgun double shot; wheel selection while primary was held confirmed crowbar, retained suppression, then a new click after release and draw completion fired. The first fixture attempt clicked before the crowbar draw deadline and was revised rather than counted as successful release verification. Evidence: artifacts/iteration8-selection-report.json and iteration8-selection-assertions.json.

The preceding package (B941BF020B6B08192AE10AA3EEDB8F76CA0B8E6750B6CEC56D19739BD65D6C18) was launched with --time-scale 0.1 to keep the native-duration selector open long enough for separate Computer Use actions. This run used actual window input, with no input script. The current package additionally checks held secondary confirmation through the normal handlers in its input fixture.

Wheel selected pistol, left click confirmed it, wheel selected revolver, right click confirmed it, Q returned to pistol, and Escape canceled a pending SMG selection. The final report records zero attacks, pistol clip 18 and revolver clip 6. Both confirmation clicks were consumed. F12 saved the capture; F10 wrote the report and closed the game.

Evidence: artifacts/iteration5-manual-report.json, iteration5-manual-input.log and iteration5-manual-pistol.png. Slow simulation was a testing aid; normal launch defaults to time scale 1. A covered-window capture initially showed the foreground app and was rejected; the targeted game was activated and recaptured before input.

Earlier packaged mouse tests established that a 90-pixel upward move increases pitch by 0.103680 radians, and a rightward move decreases Source yaw by 0.103680 radians. The input conversion code remains in place; this iteration's physical check targeted selection rather than remeasuring mouse deltas.

## Original-game comparison and sky checks

The owned original engine was run through the separate work/hl2-decompiled/original-oracle write-path wrapper. Loaded engine/client/server DLL paths were confirmed to be installed stock modules. Post-load commands used the original launcher's built-in -hijack forwarding. The usable localized engine capture is oracle_pistol_localized.png in that private folder. Startup captures made before asynchronous map load are rejected.

The Rust and native captures establish the menu's bucket geometry, weapon labels/icons, font-cell sizing and QuickInfo placement. Fontdue coverage/hinting/glow is not identical to native GDI, and the world/viewmodel shading still differs visibly. This comparison does not prove pixel-equivalent UI or rendering.

The original shotgun was drawn at normal time, then tested at host_timescale 0.1. Separate post-load cfg invocations pressed/released +attack2/-attack2 through the original console. Engine captures oracle_shotgun_ready.png and oracle_shotgun_after.png show six shells before and four after one discharge, with reserve unchanged at 30. Retail button registrations/callbacks and IN_ATTACK2 aggregation were reviewed separately. The earlier baseline showed a crossbow and was rejected; shotgun selection was repeated and visually verified before firing. Computer Use could not activate this original-game window on two attempts (foreground process-id error), so no further UI inputs were issued; the engine's own command forwarding/capture route completed the check.

3D sky uses the camera leaf's packed LEAF_FLAGS_SKY, separately from LEAF_FLAGS_SKY2D. At trainstation spawn that flag is true, as expected. At camera (-4494, 96, 22), a 60-frame run of the preceding package recorded false visibility, zero sky frames and no lookup error; its capture shows an opaque wall with no distant scenery. Evidence: artifacts/iteration5-hidden-sky-report.json and iteration5-hidden-sky.png. The ordinary stairwell/floor captures also show opaque occlusion.

The background pass already cleared depth before the world pass. Sky visibility gating is a verified missing rule that is now implemented; these checks do not reproduce the user's exact through-wall viewpoint and do not establish a universal fix. 2D sky textures, fog and general PVS/areaportal culling remain missing.

## Translucent depth comparisons

The preceding 4056 package reproduced bright window/light-shaft patches, hidden entrance columns and cyan barrier effects drawing over opaque map walls. The reviewed backend disabled depth testing whenever a material disabled depth writes. Depth comparison and the write mask now apply independently, and clears temporarily enable writes then restore the current mask.

The AD3ABF2A package repeated 60-frame captures at camera (-4494,96,22), yaw zero, and (-4100,-2000,150), yaw zero, on d1_trainstation_02 in flight. Matching before/after images show the hidden effects and columns removed while the visible hall geometry remains. A front-side capture at (-2700,-2000,150), yaw180, still shows the entrance columns. No texture/model load errors were reported. Evidence: artifacts/iteration7-depth-before.png, iteration7-depth-after.png, iteration7-hall-before.png, iteration7-hall-after.png, iteration7-entry-visible.png and the corresponding reports. The later package changes automatic empty-fire behavior and retains the same rendering code. These views establish correction of the reproduced depth defect, not full Source shader equivalence or exhaustive visibility coverage.

## Movement checks

Native-method-backed tests cover the first standing/duck jump tick, held-jump release, ledge departure, diagonal command cropping before boost, signed overspeed correction and next-tick air friction across the 140 threshold and ascent/descent transition. See docs/research.md for the exact anchors and arithmetic. These tests use controlled collision worlds; they do not validate full retail movement over installed campaign geometry. Surface properties, moving-ground handling, ground quadrants, suit/sprint gating and crouch transitions remain separate work.

The README showcase was captured with the AD3ABF2A package through launch.cmd at the tested hall viewpoint, using a slow simulation and developer pistol loadout. Computer Use activated the exact Rust window, toggled F1 diagnostics, requested F12 capture, then closed the run with F10. An initial screenshot covered by Discord was rejected before game input. docs/images/hl2-rs-trainstation.png is the deliberately selected public preview; other captures/reports and native analysis remain local. It does not establish campaign completion or serve as a redistributable game asset.

All six stock configuration manifest entries remained unchanged after original-game testing. No installed executable/DLL was patched. Test game processes were closed.

## Native research coverage and limits

The separate native batch covered 22 inventoried runtime modules: 87,793 identified functions attempted, 87,783 exported, 10 individual failures and 20,132 warning comments. coverage.json and hashes stay in work/hl2-decompiled. These are approximate pseudocode exports; most methods remain untranslated, and the inventory is not every installed tool/library/episode.

Source reconstruction is manual: installed formats/resources, published SDK references, selected retail binary methods and original-engine tests support the implemented behavior. Export completion is not exact source recovery or automatic Rust translation.

## Public source audit

A source-only snapshot of the 112 intended public files passed universal-modder `publish check` against the owned installation: zero failures, one warning. The warning is the upstream miniquad Metal debug-capture path `/Users/fedor/wtf1.gputrace`, behind a disabled capture flag; it is not a local user's path or a game file. The vendored crate retains its MIT/Apache licenses. Cargo metadata resolves the workspace from the snapshot without native research. The public main branch starts from the reviewed current source tree; older private development history is retained locally because it once included a research-only Java inspection helper.

## Remaining gaps

NPC navigation/schedules/combat, player damage/death, remaining weapons and other secondary/projectile attacks, recoil/muzzle effects, VCD choreography/dialogue, HEV sentence scheduling, animation blends/IK/attachments, vehicles, save/load and complete campaign state remain missing. Rapier shapes/mass, movement, door blocking, scripted movement and shaders remain partial. Numeric damage-message dispatch and several HUD panels/gates/animation commands are incomplete. The README lists the current boundaries.

Trainstation still reports unsupported inputs including SetPoliceGoal, SetParentAttachment, act-busy queues, tone-map controls, template NPC spawning and logic_choreographed_scene.Start. These prevent faithful campaign progression even though the map loads, renders and the test weapons function. No FAL or crossover was used.


## Earlier LDR sky loader foundation

Eight synthetic tests verify all six actual LDR material/texture references, transforms, bounded patch includes, missing/malformed assets and static 2D VTF limits. The earlier D6E5D582 package contained this loader without rendering integration and completed a 20-frame smoke run. The current iteration integrates it as described below.

## LDR sky and bench iteration

The A598E2C8 sky package completed nine 30-frame camera runs: four cardinal views, up/down, a translated up view, the previous hidden-wall viewpoint and the hall. It loaded all six actual LDR material/texture references with no sky asset, visibility, texture or model errors. Eight sky-visible views recorded 30 background frames each; the hidden-wall view recorded zero 2D and 3D frames. Captures remain in artifacts/iteration9-sky-*. Cloud orientation was visually compared against original-engine captures at matched cardinal viewpoints; the near-vertical views differ slightly in pitch because Rust clamps below 89 degrees. Full native shaders/fog/polygon masking are not reproduced.

The final B8B94759 package repeated the sky preview and all three weapon/HUD fixtures. All 27 private report assertions passed, including confirmation/held-input suppression, reload/secondary behavior, clipped impacts and audio-error checks. Evidence: artifacts/iteration9-{weapons,shotgun,selection}-report.json and corresponding assertions. Prior iteration8 results above remain historical evidence.

An actual packaged drop onto the station bench at x=-2206,y=-1669 previously settled at feet z=64.03125 on its oversized NPC-only blocker. The corrected player filter settles at z=36.59562 on the retained model geometry. The private installed-map sweep independently found z=36.60043, and a bench-gap overlap changed from blocked to clear. Both policies retain the clip brushes/colliders; synthetic tests ensure SOLID, PLAYERCLIP and mixed solid/monster-clip still block players. Evidence: artifacts/iteration9-bench-{before,after}-report.json and private bench-runtime-trace.json. Render-derived model collision remains approximate; native PHY equivalence is unfinished.

Three new movement tests cover crouched jump boost using independent maximum speed, full airborne crouch acceleration, signed backward overspeed and three released chain hops with gravity/landing-friction ordering. These tests use controlled worlds and reviewed retail methods. No complete original-game movement trace over campaign geometry was recorded.

The owned original game confirmed the reported three-shell secondary sequence as 3â†’1â†’0, with reserve unchanged: a double shot followed by a one-shell primary fallback. Accepted screenshots and rejected setup trials are documented in the separate original-oracle/three-shell-check.md. All six stock configuration entries still matched afterward, and the original process was closed.

The private Database Method audit and exact-byte catalogue are documented in research.md. Full export accounting passes; native/Rust semantic parity is not inferred from it. No database, pseudocode, decompiler tool or game-derived asset was added to this repository.

## Iteration10: air crouch, contacts and console

139 workspace tests passed (74 runtime, 30 core, 35 asset readers), along with strict all-target Clippy, formatting and diff checks. New regressions exercise the one-time air crouch lift, blocked unducking, a 40-unit ledge, sequential/corner plane clipping, overlap escape, rotated convex bevels and translated collider-cache invalidation. Controlled wall-jump probes now retain the unobstructed apex instead of stopping on false contacts; these are not complete original-engine movement traces.

The E11C2795 package repeated all three weapon fixtures with all 27 report assertions passing, and completed pause/console and bench movement fixtures with 20 additional assertions passing. Captures show crouched apex feet approximately 36 units above an ordinary jump while head height stays nearly equal; both jumps land back on the bench. Console/menu freeze simulation, reject disabled cheats and unknown commands, grant six supported weapons, prevent focused/held-resume firing and return to gameplay. A separate direct-map fixture loaded d1_trainstation_01 and retained it after a missing-map request. Captures/reports remain in ignored artifacts/iteration10-final-*. The final packaged fingerprint is recorded in bin/build-info.json.

The owned pause-menu capture guided title/font/spacing corrections. This remains a partial working menu: Resume, Console and Quit. Ambient audio does not pause; save/load/options and the complete command registry are missing. Original bench-height evidence corroborates private PHY decoding, not public integration. Latest playtest reports of double-door pose/collision disagreement and missing SMG/AR2 secondary-ammo HUD remain open and are next priorities.

## Iteration11: static PHY, entrance doors and secondary HUD

154 workspace tests passed (83 runtime, 30 core, 41 asset readers), strict all-target Clippy and formatting passed. New tests cover bounded PHY offsets/checksums/coordinates/tree cycles, root-bone guards, separate convex pieces, solid-mode/cache separation, door idle-pose collision, secondary-ammo reserves and selection eligibility. Public reader tests use synthetic data. The later HUD geometry and window-mode checks are recorded separately below.

The BD11C68D package, built 2026-10-03T21:05:21Z, ran all three existing weapon fixtures with all 27 report assertions passing. Separate HUD, door and bench movement fixtures passed 21 assertions. Captures/reports remain in ignored artifacts/iteration11-*. Station02 loaded 65 model/skin collision assets with 327 pieces and instantiated 1,312 static convex colliders, with zero native-hull fallbacks or collision warnings. A private census also decoded all 60 unique solid static model names in this map; other maps/formats are not implied supported.

The station bench now settles at feet z=36.17742157, matching the accepted original 36.177422 float-height measurement within 0.0001. Ordinary and crouched jumps land on that height; crouching raises the feet approximately 36 while retaining nearly equal head height. This does not verify complete Havok, prediction or ground-duck timing.

The closed entrance door stops the player at x=-3216.0393. Scripted E changes entity350's rotation from 180 to 270 degrees without moving the player; subsequent forward movement reaches x=-3032.3394 beyond the door plane. Captures show the closed panel and the exterior after passage. The door run reports one existing audio gap: music/HL2_song26_trainstation1.mp3 cannot decode because only PCM WAV is supported. The validator explicitly expects and records this known failure; other model/texture/audio errors are absent. Weapon, HUD and movement runs have no audio errors.

The HUD fixture verifies unarmed/no-suit state, real secondary grants, active pistol ammo retained during pending selection, SMG and AR2 secondary reserves, and return to pistol. Actual captures were inspected; an initial AR2 capture still had SMG selected and was rejected, then the corrected fixture selected AR2. Secondary attack requests currently retain ammo because the grenade/energy-ball attacks remain unsupported; no placeholder projectile is counted as implemented.

Accepted original-engine captures verify static unarmed white and armed yellow pistol reticles at 1280x720. Original selector/SMG captures failed setup and are rejected as dynamic comparison evidence. All six fresh stock configuration entries were unchanged after testing, and the stock process was closed. No native DLL patch, game-derived fixture, scene cache, database or pseudocode was added to the public repository.

The installed HUD Position/Size rules now move the primary ammo panel to make room for ALT ammo: 0.5-second Deaccel outward and 0.4-second return. StopAnimation interrupts an in-progress geometry track. Added intermediate/endpoint/reversal and 1920x1080 geometry regressions; the combined suite passed 156 tests before the final deferred-animation regression was added. The owned 24px crosshair sprite remains unchanged; accepted original/Rust 720p crops differ by one pixel in footprint, so exact raster parity is not claimed. New borderless mode uses the primary desktop's current resolution; explicit width/height provides a 1920x1080 window.

The final packaged SHA256 is 5602BC4857ACFC280FFB734A50ECED212BC506A42D7C360EAD13BA873B1CA695, release built 2026-10-03T21:16:51Z. launch-1080p.cmd completed the nine-capture hud-slide fixture with the requested active weapons and no model/texture/audio errors; start, intermediate and endpoint captures were inspected. Windows fitted that titled window to 1920x1061. launch-borderless.cmd then produced a confirmed 1920x1080 capture. The source-only 125-file publication audit reported zero failures and two warnings for the upstream Metal debug path and its documentation mention.

SetDefaultAnimation now preserves active playback/start time/completion while storing the deferred name, as confirmed by retail callback server.dll 0x10223100. Its targeted regression passed; choosing the default clip after completion remains separate work.

## Bevy shared audio migration (2026-10-05 Canary)

Release workspace tests passed (252), all ten owned ignored tests passed with HL2_ROOT set, and strict all-target Clippy passed. The packaged entity/weapon fixture retained all 17 assertions and started 23 real audio sinks with zero pending/failed requests or preload errors, including MP3 music, pistol, AR2 charge/release, SMG explosion and HUD cues. The controlled first-map security-room fixture started seven sinks including Barney speech, paused twice/resumed once and ended with all five remaining sinks paused. Its 155 decoded references include PCM, MS ADPCM and MP3; three unused installed references remain named absent-asset/script diagnostics (Intro.zoomout, ambience/particle_suck1.wav, ambience/steamburst1.wav), with no failed emitted request. Both packaged captures were inspected. A retained-host packaged smoke run also completed after decoder/event-cursor extraction.

This validates request selection, decoder compatibility, sink startup and host pause integration. It is not a captured-audio comparison against native mixing or an ordinary campaign playthrough. Spatialization, Source DSP/soundscapes, voice-channel replacement and lipsync are unfinished. The original ALSA resolver conflict was addressed by a tiny Rust re-export shim; its 27 used API names exist in the upstream bindings, but Linux compilation/playback has not been tested.

Audio package SHA-256 `389E440B8D3AB58FB22070682EE30E5F4C8BCD1DA295FA8E0725A250607A42CB`; final packaged scene rerun also passed sink/pause assertions. Retained package SHA-256 `254E4C50278CC33AA48A8F40AE7CBE8DFB6BF94A8BFCDC1F62F29EA926CC49D1`.

## Bevy shared-material validation - 2026-10-05

The packaged release executable SHA256 is973558A4A9E0D3C2C8B2AEEC28B01A737DE68F67D65719181E4A1F1D6999CF43. The retained executable remains6E379B5184F1D1F41A2D091F4C3D9BC33D7CDEC2C37FDCB8E45A19F794BD820C. All23 affected Bevy tests and strict all-target Bevy Clippy pass; workspace formatting and diff checks pass. Shared code is unchanged from the preceding291 normal/18 owned-install test milestone; those full suites were not repeated for this adapter-only change.

Packaged movement26, weapon/door17 and material/monitor15 assertions pass. Inspected captures retain Barney's player-interest body/eyes and both live camera feeds. Spawn and Breen feed comparisons are pixel-identical; Barney's matching actor crop is exact and his per-actor eye diagnostics agree. An initial comparison against the monitor-interest fixture used a different tick/attention target and was rejected, then replaced with the matching player-interest fixture. Kleiner's feed still shows the unstaged lab wall; broadcast/choreography completion is not inferred from it. Evidence remains in ignored artifacts/material-cache-*, with performance-material-cache.json and material-cache-validation.assertions.json. See performance.md for measured scope and limits.
