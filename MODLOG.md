# MODLOG

Newest entries first. Historical entries retain their original wording/test scope; current state is in STATUS.md. This is a Rust rewrite. New entries follow the [requested MODLOG template](https://github.com/trevaintdead/ai-game-modding-guides/blob/main/templates/MODLOG-template.md).

## 2026-10-08 LightmappedGeneric $envmap cubemap reflections

**Changed:** source-assets::vtf decodes cubemap faces (`decode_cube`: six faces of frame 0, the 7.0-7.4 spheremap skipped; RGBA16161616F HDR cubemaps kept as half floats). BSP world vertices carry the face plane normal, oriented to the side the winding faces. hl2-bevy loads `$envmap` for LightmappedGeneric (HDR mode prefers the map's .hdr.vtf), plus `$envmaptint`, `$envmapcontrast`/`$envmapsaturation` (float3), `$fresnelreflection` and `$basealphaenvmapmask`. SourceMaterial binds a cube texture, and the shader adds the SDK specular term: cube(reflect) capped at 16, then mask, tint, contrast, saturation and Fresnel.

**Why:** DESIGN 11a. Owner comparison of trainstation_02 windows. The retail envmap scale is 16 under integer HDR (shaderapidx9 1004b7b0, private material-20261007 notes), so HDR cube texels are capped at 16.

**Tested how:** 333 normal tests and 29 owned tests (new: cube decode synthetic/HDR tests, owned window cubemaps, owned brush-normal orientation), strict Clippy and fmt. Packaged captures against native HDR session view-d1_trainstation_02-20261007T231545Z (outside view of the window002c wall): bright panes (116,106,76) vs native (121,110,79), right panes (121,114,84) vs (126,118,87); with the envmap disabled (84,73,43)/(82,72,40); roof unchanged (exposure equal). Regression batch: BATCH_RESULT.

**Result:** Window reflections now appear and match native's layout (sky/cloud pattern and dark patches). The first build of this branch dropped the viewmodel (fallback cube held by a UUID handle; fixed with strong per-map handles) and rejected vector `$envmapsaturation` values in four model materials (fixed). Both were caught before merging.

**Still broken or not tested:** env_cubemap on brush entities/models (runtime nearest-cubemap lookup), `$envmapmask` textures, bumped and VertexLitGeneric envmaps, displacement normals (base-face approximation), and LDR-mode envmaps (sRGB path coded but **not tested**). The hall's left "windows" are the trainstation_arch001 prop texture (static-prop lighting), and the upper hall windows need `$selfillum`. Both remain different from native.

**Next:** `$selfillum`, then info_overlay projection (DESIGN 11b).
## 2026-10-08 Fix the blinking weapon viewmodel (bloom pass ordering)

**Changed:** The Source bloom pass (hl2-bevy bloom.rs) now runs in Bevy's `Core3dSystems::PostProcess` set, after the camera's main pass, instead of being ordered only `.before(tonemapping)`. New test option `--capture-burst N` saves the N frames after `--capture` as `<capture>-1.png` ... `<capture>-N.png`, so flicker can be measured. Private helpers: work/publishing/flicker_check.py (counts frames without viewmodel pixels) and flicker_bisect_step.sh.

**Why:** Owner report: the weapon viewmodel blinked in and out. The bloom system was not placed in a Core3d set, so the parallel render schedule could run it before the viewmodel camera's main pass. On those frames its post-process swap discarded the weapon drawn afterwards. Bisected with burst captures (the burst option patched onto each tested commit): 0dc8291, 1028388 and 6a7455d clean (0 of 61 frames lost); 8ee25d2 (bloom, session 5) and later lose 1-9 of 61 frames, including 1640df0 (before this session and the macOS merge) and 994df99. SDK viewrender.cpp RenderView draws the viewmodel (DrawViewModels), then the fade/overlays, then DoEnginePostProcessing (bloom), so bloom on the viewmodel camera after its main pass matches Source.

**Tested how:** Weapons fixture with `--capture-burst 120`: two runs, 0 of 121 frames without the viewmodel (before: 1-9 of 61 per run). 331 normal tests, strict Clippy and fmt. Regression batch artifacts/bloomfix-regression: all cases exit 0, captures within 9 levels of the accepted bloom-regression baselines, no pose/visibility mismatches, campaign's 25 known station03 texture errors only; verifiers movement 26/26, weapons 17/17, attention 26/26.

**Result:** The viewmodel draws every frame in the tested fixture. Earlier single-frame captures could not show the bug; the accepted bloom-regression weapons capture happened to land on a good frame.

**Still broken or not tested:** Other Core3d ordering assumptions were not audited beyond this pass. Ordinary play was **not tested** by an automated check; the owner's live report is the reference.

**Next:** Envmap masks (`$normalmapalphaenvmapmask`, `$envmapmask`) on wip/envmap before it merges.

## 2026-10-07 README platforms table; feature list moved to docs/features.md

**Changed:** README.md gains a "Platforms" table near the top: Windows (primary tested), macOS Apple Silicon (tested by a contributor, PR #1), Linux and other systems (**not tested**). The long "Implemented so far" list moved verbatim to docs/features.md; the README keeps a one-paragraph summary with a link.

**Why:** Owner request: show confirmed platforms at a glance and shorten the README. A versioned docs page was chosen over a GitHub wiki so the list stays reviewable in PRs alongside the code.

**Tested how:** Documentation only. Checked that docs/*.md is whitelisted, that no document links to the old README anchor, and that relative links in docs/features.md resolve.

**Result:** README is shorter; the feature list content is unchanged.

**Still broken or not tested:** Linux builds and runs are **not tested**; CI runs on Windows only.

**Next:** Envmap cubemaps (DESIGN 11a) on wip/envmap.

## 2026-10-06 Apple Silicon (macOS aarch64) support for hl2-bevy

**Changed:** Added first-class macOS / Apple Silicon support to `main`:
- `source-assets::install`: Auto-discovers installed Steam content in `~/Library/Application Support/Steam`.
- `hl2-ui::console`: Multi-target test assertions for non-Windows platforms, eliminating unused variable warnings under strict Clippy.
- `scripts/build-bevy.sh` & `launch-bevy.sh`: Added POSIX build and launch scripts (with 1080p and borderless variants) generating `bin/build-bevy-info.json` and packaging `bin/bevy-assets`.
- `.gitignore`: Whitelisted new POSIX scripts.

**Why:** Enable running the Bevy/wgpu host natively on macOS Apple Silicon without manual path flags or Windows-only script dependencies.

**Tested how:**
- `cargo test --workspace --locked` (109 shared crate tests, 25 Bevy tests passed).
- `cargo clippy --workspace --all-targets --locked -- -D warnings` (zero warnings).
- `cargo fmt --all --check`.
- Packaged release build with `./scripts/build-bevy.sh` (`bin/hl2-bevy`, `bin/bevy-assets`, SHA256 metadata verified).
- Launched `./launch-bevy.sh`: Auto-discovered owned Steam installation on macOS, initialized wgpu/Metal clustering and preprocessing, created native AppKit window, and loaded `d1_trainstation_01`.

**Result:** Native macOS execution on Apple Silicon via Metal and CoreAudio works out of the box with zero runtime errors.

**Still broken or not tested:** Retained Macroquad/OpenGL host on macOS (deprecated by Apple; main is primary target). Native Windows GDI font parity (macOS uses portable fontdue). Retained scripts remain Windows-focused.

## 2026-10-07 Source bloom, pre-bloom exposure histogram, monitors at scale 1

**Changed:** Source 8-bit bloom as a Bevy Core3d post pass on the viewmodel camera (the last 3D camera, so it covers sky, world and viewmodel and runs before the HUD): gamma-space Shape (pow 2.2 times luminance with r_bloomtint 0.3/0.59/0.11) over a quarter-size 4-tap (4x4) downsample, 13-tap Gaussian blur in X and Y (SDK offsets/weights; Y times the bloom amount), additive composite onto the gamma frame (Engine_Post BloomFactor 1). env_tonemap_controller SetBloomScale sets the scale; the amount eases by 0.05 per frame from 1 (GetBloomAmount). The exposure histogram now reads a 320x180 pre-bloom presample by GPU readback (the SDK histogram runs before bloom) instead of window screenshots. Monitor (camera feed) views render at tonemap scale 1. Dev profile: no debug info for dependencies, line tables for our crates (target/debug 36 GB -> 3 GB).

**Why:** Native HDR shows bloom around bright windows and screens, and our world stayed 10-20% darker. With bloom in the screenshot the histogram dropped our exposure (plaza 1.45), which is why it must measure before bloom. SDK viewrender.cpp draws monitors before TurnOnToneMapping after the previous main view reset the integer-HDR scale to 1 (lines 2080, 2090, 2214), so applying the scale to the feed and the screen doubled it.

**Tested how:** 328 normal tests, strict Clippy/fmt. Packaged captures vs settled native HDR: hall wall (162,142,101) vs (159,139,100), screen (170,140,106) vs (181,153,121), plaza street (152,137,97) vs (162,144,100), sky (103,102,104) vs (102,101,102), exposure plaza 1.95 vs 2.00, hall 2.00 vs 1.88, slate 0.651 vs 0.65. Regression batch artifacts/bloom-regression: complete, 0 mismatches, movement 26/26, weapons 17/17, attention 26/26; contact sheet inspected (bloom on bright monitors/explosion, HUD unaffected).

**Still broken or not tested:** Window panes need `$envmap` cubemaps (native left windows 159 vs ours ~70). Plaza buildings remain ~15% darker. Bloom is not applied to monitor feeds (matches Source). No LDR mode. Batch images depend on frame timing (auto exposure).

**Next:** Environment maps, then the plaza items (info_overlay leaves, detail props, metrocop).

## 2026-10-07 Source HDR path: HDR lightmaps, auto exposure, HDR sky

**Changed:** Rendering now follows Source's HDR path (mat_hdr_level 2). Maps with HDR lighting use the HDR lightmap, ambient and world-light lumps; the lightmap atlas is linear RGBA16F capped at 16 (retail integer HDR range). Every Source material and sprite multiplies its output by the tonemap scale (Bevy camera exposure on the world, viewmodel, monitor and sky cameras). Auto exposure follows SDK viewpostprocess.cpp (17-bin histogram of the presented frame's linear luminance over the central 90% x 85%, 2% bright pixels at 60%, minimum 3% median, V-weighted 10-sample goal) and retail materialsystem 10062660 adaptation (rate x 2, accelerated darkening, step capped at 1/64), with env_tonemap_controller SetAutoExposureMin/Max, SetTonemapRate and UseDefaultAutoExposure. Sky faces with `$hdrcompressedTexture` decode RGBS (rgb x alpha x 8). VMT conditionals evaluate `hdr?` true and `ldr?` false. `--tonemap-scale S` forces a scale (mat_force_tonemap_scale). Histogram readbacks never share a frame with the final capture.

**Why:** Owner screenshots showed their game uses "High Dynamic Range: Full" (mat_hdr_level 2, tonemap scale 1.35 in the console) and a much brighter Combine slate. The native oracle's private config (mirrored by Steam Cloud) had mat_hdr_level 0, so all earlier native captures were LDR; the oracle now passes +mat_hdr_level 2 +skill 2 and can echo cvars per view (ORACLE_QUERY) after a dwell (ORACLE_DWELL). Evidence: SDK stdshaders (FinalOutput TONEMAP_SCALE_LINEAR in LightmappedGeneric, VertexLitGeneric, UnlitTwoTexture, Sky_HDR_DX9, sprites), SDK viewpostprocess.cpp; retail materialsystem 10062660 (adaptation), 10062ae0 (integer HDR lightmap encoding), 10057340/10062b20 (LDR lightmap table, for a future LDR mode), client 101d8490 (histogram query). Private notes: work/hl2-decompiled/material-20261007/README.md.

**Tested how:** Unit tests for histogram bins/target, goal averaging, retail adaptation, Bevy exposure mapping, HDR atlas cap, conditionals; 328 normal tests, strict Clippy/fmt. Native HDR captures with settled exposure (view-d1_trainstation_02-20261007T164900Z: plaza 2.00, hall 1.88; T165556Z slate 0.65). Bevy: plaza 1.80-1.98, hall during the broadcast 1.92 with the screen at (184,158,123) vs native (181,153,121), slate 0.69 with native's cyan/white-glyph look. Regression batch artifacts/hdr-regression: all cases complete, 0 pose mismatches, movement 26/26, weapons 17/17, attention 26/26; every image changes (exposure). Readback cost about +0.3 ms CPU and +0.2 ms GPU at the 60 fps cap. Rejected: gamma-space histogram (hall pinned at 0.5) and unclamped float lightmaps (white slate).

**Still broken or not tested:** The world is still about 10-20% darker than native (no bloom; SetBloomScale .4 is visible in native around windows). Window panes need `$envmap` cubemaps. No LDR mode or options menu yet (owner idea: Video options with HDR None/Full). Auto exposure depends on frame timing, so batch images are not bit-reproducible. HDR on maps without HDR lumps falls back to LDR data with the tonemap scale.

**Next:** Bloom (SDK Bloom/Downsample shaders), environment maps, then the LDR mode for the options menu.

## 2026-10-07 Eyes keep a latched view target (Breen looks into the broadcast camera)

**Changed:** Actor eyes now follow SDK CBaseFlex/CAI_BaseActor view-target semantics: the last chosen eye target persists per actor while ValidEyeTarget holds (at least 1 unit away, within 75 degrees of the head). With no scene interest or visible candidate, the eyes look at a point 128 units ahead of the head with the SDK's right +-32 / up +-16 jitter (deterministic per actor and scene clock), so both eyes converge instead of staying parallel.

**Why:** Owner review: native Breen's eyes focus on the broadcast camera while ours looked into infinity. instinct.vcd's `LookAt camera_tv_breen` event is authored inactive, so the native focus comes from the default view target (SDK ai_baseactor.cpp MaintainLookTargets random view and ValidEyeTarget). The camera is near the 128-unit point in front of Breen.

**Tested how:** New unit test; 324 normal tests, strict Clippy/fmt; packaged Breen broadcast report: eye target was none, now "view" at (919, 7517, -237) with both eyes sharing it; attention fixtures 26/26. Feed close-up shows slightly converged irises (256 px feed; not compared side by side with native at that resolution).

**Still broken or not tested:** Native random look-target selection among nearby entities, blink on target change and head-direction decay are approximated by the existing nearest-visible fallback. No native close-up comparison yet.

**Next:** HDR lighting path (owner report: native broadcast screen is much brighter).

## 2026-10-07 Monitor screen colour: VMT conditionals and linear UnlitTwoTexture modulation

**Changed:** The VMT reader applies retail MaterialSystem key conditionals (`test?$var`, optional `!`): passing keys replace the plain value, failing keys are skipped. Tests are evaluated for a DX9 sRGB-capable renderer without HDR (`srgb`, `ldr` true; `hdr`, `lowfill`, `360` and unknown tests false). Camera monitor materials (UnlitTwoTexture) now read $texture2 through an sRGB view and convert the ($color x $color2) modulation to linear like SDK SetModulationPixelShaderDynamicState_LinearColorSpace (channels above 1 unchanged; mathlib GammaToLinear table, 1.0 from 0.95).

**Why:** The jumbotron screen (dev/dev_combinemonitor_3) sets `srgb?$color2 "[2.5 2.5 2.5]"`, which we ignored, so the 0.4 $color proxy dimmed the feed, and the scanline texture was multiplied as gamma bytes. Evidence: SDK 2013 unlittwotexture_dx9.cpp/_ps2x.fxc (both samplers sRGB-read, result = base x texture2 x modulation), BaseVSShader.cpp:652, BaseShader.h ApplyColor2Factor, mathlib color_conversion.cpp; retail materialsystem.dll 1003c230 (conditional tests, return value = skip) and 1003b300 (a passing conditional replaces the plain var). Private notes: work/hl2-decompiled/material-20261007/README.md.

**Tested how:** New unit tests for conditionals and modulation; 323 normal tests, strict Clippy/fmt. Packaged slate fixture vs native slate.png: screen mean RGB (78,122,114) vs native (76,156,154) (was (80,118,100)); white glyphs (165,199,199) vs (175,214,218); scanline grid and black frame match. World regions unchanged. Rejected experiment: a 16x lightmap cap gave (105,229,230), far brighter than native; no constant was fitted.

**Still broken or not tested:** Native runs the HDR path: the slate luxels are linear 128 in both lumps, and native's HDR lightmap range and TONEMAP_SCALE_LINEAR saturate the poster further (native G and B equal, ours G > B). HDR lightmaps/tonemapping are not implemented. Fallback block selection (">=DX90", "hdr_dx9", ...) is not implemented. The conditional change affects every material that uses `ldr?`/`srgb?` keys; the regression batch is the next check.

**Next:** Rerun the regression batch and accept baselines; HDR lighting remains a separate, larger plan.

## 2026-10-07 Combine slate frame and lightmap overbright headroom

**Changed:** World and brush-entity faces using tools/toolsblack* now render (an owned UnlitGeneric black texture); every other tools/ material stays skipped. The lightmap atlas stores gamma(linear / 2) and lightmapped materials restore the factor 2 before the gamma decode, so baked light between 1.0 and 2.0 is no longer clipped. New fixture test-inputs/bevy-monitors-breen-slate.json views the jumbotron before the broadcast starts.

**Why:** Native trainstation_02 shows only the bright Combine slate on the jumbotron at map start; ours showed Breen around a dark slate. The slate func_brush (*87) is framed by four toolsblack faces we skipped. Its `_minlight 255` is not a runtime value: SDK 2013 VRAD (radial.cpp:676/813) bakes a per-luxel floor of `_minlight*128` into the lightmap. Source LDR lightmaps keep 2x headroom (imaterialsystem.h OVERBRIGHT 2.0), which our clamp at 1.0 removed.

**Tested how:** Updated lightmap unit test (linear 2.0 saturates, 1.0 encodes gamma(0.5)=186), 321 normal tests, strict Clippy/fmt. Packaged slate fixture against native view-d1_trainstation_02-20261007T054643Z/slate.png (artifacts/slate): black frame now matches native; screen mean RGB before/after/native (76,97,82)/(80,118,100)/(76,156,154); wall, rail and window regions unchanged within 2 levels.

**Still broken or not tested:** The screen is still greener and darker than native's cyan (UnlitTwoTexture colour path, next step). Native runs the HDR path (HDR lightmaps and auto-exposure tonemapping), which is not implemented. Regression batch not yet rerun.

**Next:** UnlitTwoTexture modulation/sRGB and the pixel grid; rerun the batch and accept baselines.

## 2026-10-07 Static map decals (Breen's studio backdrop)

**Changed:** infodecal entities without a targetname are now projected at map load onto lightmapped world brush faces (modkit-core::decals::static_decal; hl2-bevy assets add_static_decals). Each decal is sized from its base texture times $decalscale, centered on the plane point within 5 units, oriented by the receiving face's texture axes, clipped to its rectangle and lit by the face's lightmap.

**Why:** Owner review of the native jumbotron: the yellow Combine logo behind Breen is his studio backdrop (decals/decal_posterbreentv), not see-through screen areas as I had first recorded. SDK 2013 world.cpp CDecal::StaticDecal places static decals on world brushes; props are excluded by its trace filter.

**Tested how:** New decal unit test, 321 normal tests, strict Clippy/fmt. Packaged Breen fixture: the feed now shows the backdrop with the logo, matching native session view-d1_trainstation_02-20261007T053505Z. **Not run:** the full regression batch with decals (baselines will change wherever static decals exist).

**Still broken or not tested:** Named (triggered) infodecals, decals on brush entities, engine-exact decal projection/orientation (inferred from Source behavior, engine code not reviewed), decal shaders (DecalModulate). Combine slate: native shows only a bright slate while the rest of the screen is off. The slate func_brush (*87) is surrounded by four tools/toolsblack faces (SURF_NOLIGHT), which our BSP builder skips as tools/ materials, so Breen shows around the slate. Native slate brightness (_minlight 255) and the screen's "pixel" look (UnlitTwoTexture: sRGB-read base x texture2, HDR tonemap scale) are not matched yet.

**Next:** Render tools/toolsblack as opaque black, apply func_brush _minlight, compare the screen shader and pixel grid with native, rerun the batch and accept baselines.

## 2026-10-07 Scene gestures for template-spawned actors (Barney's head-down at 21-22.5 s)

**Changed:** Choreography clip preparation now resolves every entity a scene actor name can match, including point_template children that are still pending (Barney, Kleiner), so their scene gesture and posture clips load with the map. Head control now measures its correction in the full 3D frame of the animated "forward" attachment, which is parented to the head bone (SDK UpdateHeadControl), instead of a level yaw-only frame. The Bevy report lists per-actor gesture composition errors (`gesture_compose_errors`). The regression batch drops the superseded edge-on Breen and empty-lab cases, whose fixtures are deleted and baselines retired.

**Why:** Owner report: Bevy Barney was stiff (no arm/body motion) compared with native. Investigation of the unexplained native head-down at security_02 21-22.5 s: native plays rubNeck (Gesture08) and thinking (posture01). The shared composition raised Barney's right hand from z 40.8 to 64.7 at 21.6 s, but the Bevy report showed MissingSequence for posture01/gesture08 (Barney) and kposture01/kgesture04 (Kleiner). scene_actor skips killed entities, and template children are killed until ForceSpawn, so their clips were never prepared at load. Every scene gesture for these actors was dropped in Bevy.

**Tested how:** 320 normal and 26 owned tests (new owned test: template-pending Barney/Kleiner require gesture08/posture01/kposture01 before ForceSpawn; new head-frame unit test), strict Clippy/fmt. Packaged close-ups at 21.00/21.30/22.45 s now match native (head bowed, hand at neck); wide 21.30 s view matches the native pose. Batch artifacts/trimmed-regression: all complete, 0 pose/visibility mismatches, attention 26/26. Kleiner-scene changes (Kleiner now gestures); weapons varies only in the HUD health box between runs. Baselines accepted.

**Result:** Barney's and Kleiner's authored gestures and postures play in Bevy, which explains the native head pitch (it is the gesture, not head control).

**Still broken or not tested:** Native comparison of the full timeline after this fix; other template-spawned NPCs are covered by the same fix but untested.

**Next:** Native trainstation_02 jumbotron/godray comparison; campaign-order reveal chain.

## 2026-10-07 Kleiner scene movement and new regression views

**Changed:** npc_kleiner is registered with the shared human ground-movement controller, like Barney, so scene MOVETO events move him. New regression cases: test-inputs/bevy-monitors-breen-screen.json (face-on jumbotron during the broadcast) and test-inputs/bevy-monitors-kleiner-scene.json (Kleiner on Barney's monitor in campaign state). The new baselines are accepted, with the previous ones archived locally.

**Why:** Owner report: the old Kleiner case showed an empty lab and the old Breen case an edge-on screen. Kleiner is a kleiner_template child, and security02 opens with `MoveTo marks_kleiner_catwalk_1` (0.01 s); because only Barney was registered, Kleiner stayed at his spawn, 110 units outside the lab camera. SDK 2013 npc_kleiner.cpp and the retail CNPC_Kleiner::Spawn (server.dll 10391cd0, reviewed privately) agree on HULL_HUMAN, SOLID_BBOX, MOVETYPE_STEP and capabilities 0x801801 (ground move, open doors, turn head, animated face) plus friendly-damage immunity.

**Tested how:** hl2-simulation tests (139), strict Clippy/fmt. Regression batch artifacts/kleiner-regression: all 11 cases complete with 0 pose/visibility mismatches. Diff counts against the old baselines are unchanged from the lighting batch, except weapons (view-model lighting basis). Attention verifier 26/26. Kleiner reaches (-850, 2334) and speaks on the monitor at 10.5 s; Barney's faceplate is off and the cameras are folded in the attention fixtures.

**Result:** The lab feed shows Kleiner talking; the jumbotron shows Breen face-on. New accepted baselines cover the lighting change, the godray fix and the new views.

Follow-up (owner report: Barney was talking to the default blue screen): all eight security_02 fixtures now also force-spawn kleiner_template, enable barney_security_monitor_1 and switch on kleiner_security_camera_1, as the campaign's triggers do. Rerun batch artifacts/kleiner-monitor-regression: pixel-identical to the new baselines, attention 26/26, and Kleiner is on Barney's monitor in the attention view. The private native security oracle setup gets the same three inputs (not yet run).

**Still broken or not tested:** Kleiner's walk/turn timing against native is not compared. Other NPC classes (metrocops, citizens) are still not registered for movement. No native comparison of the lab feed or jumbotron yet.

**Next:** Native head-pitch analysis; native jumbotron/lab feed comparison once the oracle cursor fix is approved.

## 2026-10-07 Bevy becomes main; Macroquad host removed

**Changed:** At the owner's request, bevy-migration was merged into main (main's docs-only commit b4b1731 was merged in first) and main now fast-forwards to it. The former Macroquad/OpenGL host was removed: crates/hl2-runtime, its launchers (launch.cmd, launch-borderless.cmd, launch-1080p.cmd), scripts/build.ps1, the vendored third_party/miniquad and quad-alsa-sys shim, its 13 input-script fixtures and the mods/*.json sandbox files. Cargo.lock only drops the 20 packages of that stack. AGENTS, README, CONTRIBUTING, STATUS, docs and CI now describe a Bevy-only rewrite on main. macroquad-prototype, bevy-migration (frozen at the merge) and wip/scripted-scenes are preserved.

**Why:** Owner decision: Bevy is far ahead of the old host, so checking both hosts after shared changes was overhead. Removing the old host keeps the repository clean.

**Tested how:** After removal: 319 normal tests (9 removed with the old host) and 25 owned tests, strict Clippy/fmt, packaged Bevy build, and the movement (26/26) and entities/weapons (17/17) packaged fixtures with their verifiers.

**Result:** One host. The shared crates (source-assets, modkit-core, hl2-simulation, hl2-ui) are unchanged.

**Still broken or not tested:** History and the macroquad-prototype branch still hold the old host; its features that Bevy lacks (JSON sandbox, inspect/verify/export subcommands) are gone from main.

**Next:** Scene MOVETO for Kleiner (lab feed), Breen face-on regression case, native head-pitch analysis.

## 2026-10-07 Source model lighting (leaf ambient cubes and world lights)

**Changed:** Models are now lit the way the retail engine's light cache does it, instead of with a constant gray. New `modkit-core::lighting`: leaf ambient samples weighted by 1/(d^2+1); world lights with Source falloff, styles and a world-only visibility trace (8-unit slack); up to four local lights kept by luminance and the rest folded into the ambient cube; skipping lights flagged as already baked into the cube; the ambient boost for flagged models; SDK vertex shader terms. New `source-assets::model_lighting` reads the leaf tree, the LDR/HDR leaf ambient lumps (matching the lightmap choice), world lights and sky faces. `Vertex.normal` carries VVD normals, and the studio illumposition/flags are read per model. Static props are baked once per vertex at load, which applies to both hosts. Bevy model draws get a per-entity lighting uniform that is recomputed when the illumination origin moves. The shader evaluates ambient cube + diffuse (+ $halflambert) on the linearized base texture, with skinned normals on both GPU and CPU paths. The view model maps its camera space onto the player's view for lighting.

**Why:** Owner report that entities lack the map's lighting (DESIGN step 8). Retail engine.dll was reviewed privately (lighting-20261007/README.md): Mod_LoadLeafs, Mod_LeafAmbientColorAtPos, light-cache selection, the draw-time ambient boost and the light-to-shader conversion. SDK 2013 corroborated ColorRGBExp32ToVector (its factor of 255) and the light-descriptor cone math.

**Tested how:** 328 normal tests (7 new lighting/decoder tests) and 25 owned tests (d1_trainstation_01: 535 world lights, 13,479 ambient samples), strict Clippy/fmt. Packaged Barney close-ups against native: at 18.05 s, mean skin RGB is 95/66/45 (Bevy) vs 93/64/42 (native), with luminance percentiles 48/67/93 vs 45/65/90; at 22.45 s the helmet and hair tones match (confirmed by the owner). A new private generic native view oracle (run_view_oracle.py) captured the d1_trainstation_03 corridor: the door is equally dark and the pistol view model lit similarly. Regression batch artifacts/lighting-regression: every case completes with 0 pose/visibility mismatches; images change wherever models appear (expected relighting). Retained smoke capture renders with baked static props.

**Result:** Barney, props and the view model now pick up the map's ambient color and nearby lights. Face shading follows the native key light.

**Still broken or not tested:** Shadows; bump/phong/specular (native helmet sheen); flex normal deltas; styled lights are treated as on; the skylight uses sky-face polygons instead of a surface-flag trace; HDR tonemapping; the retail 162-sample static-prop path. The retained host does not light dynamic models. The first native corridor capture was rejected because the game took focus and trapped the owner's cursor. The oracle now records the user's foreground window before launch and passes +cl_mouseenable 0.

**Next:** Explain the native head-down pitch at 21-22.5 s; trainstation_02 jumbotron/godray native comparison; campaign-order reveal chain.

## 2026-10-06 Entity render color in Bevy (godray brightness)

**Changed:** Bevy entity materials now apply Source color modulation. rendercolor tints the model, and renderamt sets its alpha outside kRenderNormal. Materials are keyed per (material, lightmap, modulation), so untinted entities still share handles. New fixture test-inputs/bevy-monitors-breen-screen.json frames the trainstation_02 jumbotron face-on at about 25 s, during the broadcast.

**Why:** Owner report: the trainstation godrays were overexposed in Bevy (the retained host already applied rendercolor). The shafts are additive prop_dynamic vol_light models tinted to about 19% (rendercolor ~49 45 34), which Bevy drew at full white. The owner also noted the old Breen fixture looks at a wall. jumbotron1's screen layers face +-Y, so the old yaw-180 view is edge-on, and its 7 s capture precedes the broadcast (the Combine slate brush is correct then; the scene starts at about 11 s and its OnTrigger1 hides the slate).

**Tested how:** Packaged captures (artifacts/godrays). Regression batch artifacts/godray-regression: only the Breen main view changes (whole frame, dimmer shafts); weapons/Kleiner/campaign identical; movement unchanged from the previous batch (735 px against the old baseline); attention 26/26.

**Result:** Godrays are subtle instead of whiting out the hall. The new fixture shows Breen speaking on the jumbotron, with lip sync.

**Still broken or not tested:** No native trainstation_02 comparison yet for shaft brightness or the jumbotron, whose dark feed areas render see-through. The accepted partition-breen baseline image predates this change.

**Next:** Model lighting (docs/DESIGN.md step 8).

## 2026-10-06 Scripted-sequence script events (faceplate removal)

**Changed:** While a scripted_sequence plays, SCRIPT_EVENT_FIREEVENT (1003) animation events crossed by the actor's clip fire the script's OnScriptEventNN output, with NN from the event options and the actor as activator.

**Why:** ss_Helmet_Reveal (helmet_reveal, events "1" at cycle 0.417 and "2" at 0.762) removes Barney's head faceplate and toggles the hand copy through these outputs.

**Tested how:** Packaged Bevy captures before, between and after the events (artifacts/reveal): Barney lifts the faceplate, then his face is revealed with helmetBack kept. hl2-simulation tests and strict Clippy/fmt pass.

**Still broken or not tested:** The native side-by-side of security_01 and the reveal was not run. Other studio script events (1000-1008) are not dispatched.

**Next:** Model lighting; Breen fixture framing; godray brightness.

## 2026-10-06 Head/body flexes, Combine camera activities and movement turning

**Changed:** Actor pose parameters now add the server-side flex controllers, following SDK CAI_BaseActor: head_rightleft/updown/tilt on top of the head look correction (UpdateHeadControl), body_yaw/spine_yaw/neck_trans from body_rightleft/chest_rightleft/head_forwardback (UpdateBodyControl), and gesture_height/width from gesture_updown/rightleft (MaintainLookTargets). The values are included in the pose signature. Head control takes its eye position from the animated "eyes" attachment and the self-look direction and head frame from "forward" (shared `Scene::attachment_frames`). npc_combine_camera deploys at spawn (open idle) unless StartInactive. Enable/Disable/Toggle play the retract transition into closed idle, or return to open idle. Walking NPCs turn toward the path at MaxYawSpeed (45 x 10 deg/s) instead of snapping, and the move_yaw pose parameter keeps the legs on the path.

**Why:** Owner steps 5 and 7. Step 6 check: Barney's state origin at his desk equals mark_barneyroom_monitor_3 exactly, and the 18.05 s native/Bevy silhouettes line up within about one unit, so the session 2 ~5-unit offset no longer reproduces.

**Tested how:** 321 normal tests (new synthetic head/body flex and camera activity tests), strict Clippy/fmt. Packaged close-ups at 18.05/21.00/22.45 s against native sessions security-scene-20261006T204141Z and T214010Z. Camera open/retracting/closed captures (artifacts/camera-acts). Walk samples along Barney's route (artifacts/facing).

**Result:** Scene head flexes move Barney's head by their authored degrees. Cameras fold when logic_disable_cameras fires. Path corners turn smoothly.

**Still broken or not tested:** At security_02 21-22.5 s native Barney's head pitches down toward the consoles while Bevy's stays level. Head flexes and gesture pose flexes are ruled out (both near zero there), and the cause is unexplained. The reversed camera-open transition, camera aim pose and eye sprites are not modeled. Entity lighting and shadows: plan recorded in docs/DESIGN.md step 8.

**Next:** Source model lighting (ambient cubes, world lights, normals).

## 2026-10-06 Lip sync from speech phonemes

**Changed:** New `source-assets::sentence` reads the text VDAT chunk of owned WAVs (SDK CSentence 1.0: word phonemes and emphasis samples, Catmull-Rom emphasis intensity) and flex settings files (`expressions/phonemes*.vfe`). New `hl2-simulation::lipsync` implements the client viseme path: box filter (0.08 s), neighbor crossfade extension, and weak/normal/strong emphasis blending. The pinned SDK was statically compared with retail client.dll AddVisemesForSentence 100bb720, AddViseme 100bb630 and ComputeBlendedSetting 100bbbd0 (private review). Both hosts parse VDAT when loading waves, register the actor's voice with the chosen wave when a scene line plays, and render `Scene::actor_flex_values` (scene controllers plus visemes) through the shared FaceModel path. Scenes flagged ignorePhonemes play without lip sync.

**Why:** Owner step 4: mouth movement timed to speech.

**Tested how:** Unit tests (VDAT parse/intensity, viseme box filter/cleanup). Owned tests: phoneme VFE files (48 normal, 1 weak, 1 strong settings; aa opens jaw_drop); all 23 Barney waves in d1_trainstation_01 scenes carry phonemes and move the mouth. Strict Clippy/fmt. Packaged Bevy close-ups during ba_thinking01 (21.00/21.30 s) show the lips parting and closing.

**Result:** Barney's mouth follows his lines in Bevy, and the retained host shares the path.

**Still broken or not tested:** Voice time is scene time since the request, not mixer position. phonemedelay/phonemesnap/LOD and streaming delays are fixed at defaults. Native frame-by-frame mouth comparison is limited by about 0.05 s oracle timing jitter. Retained lip sync was not visually checked.

**Next:** Head flexes into head control; Barney's mark offset.

## 2026-10-06 Retained-host facial flexes and volume option

**Changed:** The retained host now applies the shared `FaceModel` flex path: per-vertex studio flex keys in its draw batches, NPC face models loaded at setup, and scene controller values plus rest-gaze FACS eyelids added to bind positions before CPU skinning. It also accepts `--volume 0..1` as a master scale for all sounds, so unattended tests run at 1%.

**Why:** Shared changes must work in both hosts. Quiet testing had no option on the retained host.

**Tested how:** Strict workspace Clippy/fmt, hl2-runtime tests. Packaged retained G-Man intro close-ups (artifacts/retained-flex/gman-closeup.png, launched without activation at volume 0.01): faces deform sanely and the expression changes over time.

**Result:** Retained actors show scene facial expressions.

**Still broken or not tested:** The retained host has no eye-target presentation (eyelids use the rest gaze) and no iris shader. In a retained security_02 smoke run Barney did not spawn from his template (helmetBack stayed at its map position), a retained-specific gap not investigated. The retained input script needs an explicit `quit` before it writes its report.

**Next:** Lip sync from phoneme data.

## 2026-10-06 Entity parenting and Barney's helmet props

**Changed:** The simulation now supports movement parenting. Map-spawn `parentname` keeps the spawn offset, and the `SetParent`, `SetParentAttachment`, `SetParentAttachmentMaintainOffset` and `ClearParent` inputs work. Each tick, children follow the parent's pose or animated attachment, using the same composed pose both hosts render. `prop_dynamic` StartDisabled hides the prop. Gear attached to an animated attachment has no collider, so a helmet no longer blocks its wearer. The security fixtures now include the campaign state by security_02: faceplates off (ss_Helmet_Reveal runs in security_01) and logic_disable_cameras triggered.

**Why:** Owner priority: in security_02 Barney wears the separate helmetBack prop, parented by logic_barney_init to helmet_attachment after the template spawn. Only the faceplate comes off, during ss_Helmet_Reveal.

**Tested how:** 317 normal + 22 owned tests (synthetic parenting test), strict Clippy/fmt. Packaged security captures compared with native session security-scene-20261006T204141Z. Regression batch artifacts/helmet-regression: weapons/Kleiner/campaign identical; movement 751 px and Breen 3,383/6,654 px, the same known autoplay differences as the accepted 1bf087f batch. Attention 26/26.

**Result:** helmetBack renders on Barney's head through the whole scene, matching the native close-ups. With the faceplates off, Barney walks to his desk again. (An intermediate build where the solid helmet blocked his route was rejected and fixed.)

**Still broken or not tested:** The faceplate removal in security_01/ss_Helmet_Reveal has not been played or compared yet. Moving brush/physics hierarchies are not parented. Nested hierarchies lag one tick. Retained host not visually checked (shared simulation only).

**Next:** Retained-host flexes, lip sync, head flexes, Barney's mark offset.

## 2026-10-06 FACS eyelids and native face close-ups

**Changed:** Eyeball records now carry their FACS eyelid fields, and actor flexes apply the retail eyelid step (private review: StudioRender 1001bd80). It converts lid raiser/neutral/lowerer weights and the eye's look direction into lid descriptor values before vertex deltas. Flex math moved into shared `source-assets` (`FaceModel`, `descriptor_weights`, `vertex_deltas`) so the retained host can reuse it. Rigs now keep every model attachment. Movement scripts accept `pitch` and `fov` (Source horizontal 4:3 degrees) for zoomed comparison shots, and the private native oracle takes a matching `ORACLE_CLOSEUP`.

**Why:** Native face close-ups at 18.10 s showed Barney's eyes fully open. Bevy left the lid descriptors at zero, which half-closed the upper lids and raised the lower ones.

**Tested how:** 316 normal + 22 owned tests (synthetic eyelid test; owned Barney/Kleiner neutral face has no lid deformation), strict Clippy/fmt. Packaged close-ups `test-inputs/bevy-face-security-1805/2245.json` compared with native session security-scene-20261006T204141Z (scene times 18.10/22.50 s).

**Result:** Bevy lids now match the native open eyes at both times.

**Still broken or not tested:** Lids use the previous frame's eye direction. Head pose (native head turns and tilts further toward the player), brow intensity, Source model lighting (ambient cube/local lights), helmet props, delayed flex weights, lip sync and the retained-host flex path. Full regression batch not rerun.

**Next:** Helmet props parented to Barney's attachments, then retained flexes, lip sync and head flexes.

## 2026-10-06 Facial flexes render in Bevy

**Changed:** Scene-driven flex controllers now deform actor faces in the Bevy renderer (setup-loaded flex data, per-vertex flex references, retail weighting, GPU and CPU skinning paths). Earlier the same day: the flex track evaluator and Surface flex sources.

**Why:** Scripted scenes need facial animation; Barney's security_02 expressions were missing.

**Tested how:** 315 normal tests, strict Clippy/fmt, owned flex/surface mapping test, packaged security captures, attention 26/26.

**Result:** Barney's expression changes during security_02.

**Still broken or not tested:** Retained host flexes, native facial comparison, delayed weights, lip sync/phonemes, eyelid-eye interaction, normals.

**Next:** Native face comparison at matching times, then lip sync (phoneme tracks to flex settings).

## 2026-10-06 Facial flex data reader

**Changed:** New `source-assets::flexes`: MDL flex descriptors, controllers, rules and mesh vertex deltas; SDK RunFlexRules and the retail vertex weighting.

**Why:** First step toward facial animation in scenes (FlexAnimation tracks, expressions, later lip sync).

**Tested how:** Synthetic rule/ramp tests; owned Barney/Kleiner parse and jaw_drop evaluation; strict Clippy.

**Result:** Real flex data parses and evaluates.

**Still broken or not tested:** Not driven by scenes and not rendered yet; delayed weights and wrinkles are not applied.

**Next:** Scene flex tracks to controller values, then CPU vertex deltas in both renderers.

## 2026-10-06 Pose-parameter blends, autoplay head sequences and head control

**Changed:** Rigs keep pose parameters, full blend grids and autoplay sequences. Composition samples blends by pose parameter and adds autoplay after layers. NPC head control drives head_yaw/pitch from look interests with the SDK think rates.

**Why:** FACE turned Barney's body away from the player; the original turns his head on pose parameters. Blend grids and autoplay are also how the models encode head, body and gesture variation.

**Tested how:/** 312 normal and 21 owned-install tests, strict Clippy/fmt; synthetic blend/head tests; owned Kleiner autoplay/blend checks; attention 26/26; regression batch.

**Result:** Barney looks over his shoulder at the player while facing the monitor.

**Still broken or not tested:** Chest-bias limits, roll, random/synthetic looks, eye-position attachments in the simulation, sequence transitions, IK, bone controllers, the posekey 2D path and locomotion move_yaw blending for base clips.

**Next:** Rerun the native security comparison; then facial flexes or locomotion move_yaw blending.

## 2026-10-06 Scene FACE events, arrival distance, Combine camera model

**Changed:** FACE events turn standing NPCs toward targets at SDK yaw speed. MOVETO walks until within the event arrival distance (2D). The npc_combine_camera model is set. Unattended-test options: Bevy `--volume`/`--no-focus`, and a quiet, unfocused native oracle.

**Why:** In the native comparison, Barney faced the wrong way, stopped 9 units short of his mark, and the wall camera was missing.

**Tested how:** 310 normal tests, strict Clippy/fmt; FACE unit test; packaged security fixtures; movement and attention verifiers.

**Result:** Barney stands on his mark and faces the monitor; the camera renders.

**Still broken or not tested:** Attention 24/26: without head pose control Barney cannot look at the player while his body faces the monitor. Facing while moving, camera open/close activities and facial flexes are not implemented.

**Next:** Pose-parameter sequence blending and CAI_BaseActor head control (head_yaw/head_pitch) driven by attention targets.

## 2026-10-06 NPC door lookahead: Barney reaches his desk

**Changed:** NPC routes remember door segments and request the door ahead of contact (within 96 units).

**Why:** The cop-door trigger unlocks barney_door_2 for only one second, before Barney touches the door.

**Tested how:** 308 normal tests, strict Clippy/fmt; packaged security fixtures; movement 26 and attention 26 assertions; native/Bevy side-by-side.

**Result:** Barney walks through both doors and gestures at the desk in Bevy, as in the original.

**Still broken or not tested:** FACE events, exact arrival position, the wall Combine camera, desk material, and a native rerun with the mask fix. The full regression batch was not rerun.

**Next:** FACE events, then rerun the native comparison (mask off) and compare poses.

## 2026-10-06 Trigger toucher filters

**Changed:** trigger_once/trigger_multiple honor client/NPC/everything toucher flags, and NPCs can activate NPC triggers. Security fixtures enable trigger_cop_close_door_1 as the campaign does earlier.

**Why:** NPC-only triggers fired for the player, and NPCs could not trigger anything, which blocks Barney's door sequence.

**Tested how:** 308 normal tests, strict Clippy/fmt; packaged movement assertions and movement/campaign pixel comparisons.

**Result:** No regression in the checked fixtures. Barney still stops at barney_door_2.

**Still broken or not tested:** Why the cop-door trigger does not let Barney through; no NPC-touch unit test; the full regression batch was not rerun.

**Next:** Instrument trigger_cop_close_door_1 bounds and door lock timing against Barney's feet, then rerun the native comparison.

## 2026-10-06 Native scene oracle, point_template spawning, prop flags and NPC doors

**Changed:** Private native oracle for the security scene. point_template children wait for ForceSpawn. prop_physics motion-disabled/start-asleep flags are honored. NPC routes pass through doors and open them on contact. Barney fixtures force-spawn him like the campaign trigger.

**Why:** The first native comparison showed Barney never reaching his desk in Bevy. The causes: closed doors, a desk that physics wrongly moved, and templated actors existing too early, which is also wasted simulation.

**Tested how:** 308 normal and 21 owned tests, strict Clippy/fmt; synthetic template test; owned security tests with ForceSpawn; native oracle captures; packaged security fixtures and the regression batch.

**Result:** The desk stays at its authored pose. Barney spawns on demand, opens the interrogation-room door and walks to the next door.

**Still broken or not tested:** NPC-touch triggers (trigger_cop_close_door_1 unlocks barney_door_2), so the desk walk and a side-by-side gesture comparison are unfinished. The Combine wall camera is not rendered. Repeat ForceSpawn copies, EnableMotion and door-blocked replanning edge cases are untested.

**Next:** NPC trigger touch (spawnflags 2), then rerun the native/Bevy security comparison at 18.05/22.45 s.

## 2026-10-06 Scene gesture execution

**Changed:** Scene GESTURE events create per-actor layers: SDK tag retiming, intensity weights, posture suppression and RemoveLayer fades. Both hosts compose base clip and layers through `Scene::actor_matrices`. Faceposer keyvalues are parsed, and the clip budget accounts for gesture children.

**Why:** Authored scene gestures were ignored, so actors only played base clips.

**Tested how:** 307 normal and 21 owned tests, strict Clippy/fmt; synthetic retiming/layer tests; owned security_02 playback with Barney's rig; packaged 1,600-tick security timeline and the regression batch (pixel comparisons, movement/weapons/attention verifiers, retained smoke).

**Result:** Barney, Kleiner and the G-Man actor now layer their authored gestures in Bevy. Existing fixtures without gestures are unchanged.

**Still broken or not tested:** Comparison against the original game, cross-scene layer priority, IK, head/facial animation and lip sync. Empty-name gestures produce no layer, and native handling of them is unverified. Posture motion uses scene movement as IsMoving.

**Next:** Capture the same security scene moments in the original game and compare poses/timing; then head pose and facial flexes.

## 2026-10-06 Shared Source animation layer composition

**Changed:** modkit-core composes Source sequences: delta/post layers, per-bone weights and autolayer ramps through `Rig::accumulate_pose`. source-assets keeps raw delta frames, sequence flags, bone weights, fades and named autolayers, and loads autolayer children with their parents.

**Why:** Real NPC gestures are masked parents whose children are delta layers. The old absolute delta conversion and base-clip-only path could not reproduce them.

**Tested how:** 302 normal and 20 owned tests, strict Clippy/fmt; synthetic composition tests; owned Barney g_pointRight load/compose check; retail client.dll AccumulatePose/AddSequenceLayers/SlerpBones static comparison; five packaged fixtures pixel-identical to 28c0bb3 captures, plus movement 26, weapons 17 and attention 26 assertions and a retained smoke capture.

**Result:** Composition matches the SDK and the reviewed retail code paths. Existing scenes are unchanged.

**Still broken or not tested:** Scene gestures are not executed yet. IK, local-context, world-space and pose-parameter layers, fixed-alignment slerp, 3-way blends and native runtime pose comparisons are missing.

**Next:** Scene gesture layers: faceposer tag retiming, intensity and end fades, then compose them in both hosts.

## 2026-10-06 Priority change: resume Rust rewrite, on-demand decompilation

**Changed:** Owner decision: pause whole-corpus decompilation coverage after pass14 and resume the Rust rewrite with authored NPC gestures. AGENTS.md, STATUS.md and docs/DESIGN.md now make decompilation feature-driven: retrieve and review the relevant retail functions from the private index before implementing each step. A drafted pass15 dispatcher-trace plan was dropped before any run.

**Why:** About 94% of baseline executable bytes already sit inside identified functions with pseudocode (133,473 addresses), but zero have reviewed names/types. Further coverage passes have diminishing returns for the campaign; reviewed semantics of specific systems are what the rewrite needs.

**Tested how:** Documentation-only change: whitelist, local-link and public snapshot checks. No runtime build/test rerun.

**Result:** Handoff documents agree on the new priority. Private research databases are unchanged.

**Still broken or not tested:** Everything listed in STATUS.md; full decompilation, names/ABIs and native parity remain unfinished.

**Next:** Identify retail animation-layer/gesture functions in the index, record reviewed findings privately, then implement raw delta/post/mask/layer readers.

## 2026-10-05 Callback discovery, ABI and switch evidence; quota handoff

**Changed:** Continued private research through pass14. The exact-byte index now combines13 source datasets,133,473 observed addresses with some pseudocode and133,556 export variants across42 selected modules. Saved entries and10,752 temporary discovery entries remain separate. Public changes document research; runtime code is unchanged.

**Why:** Export success, listing ownership and decompiler switch recovery do not establish complete discovery or trustworthy ABIs. Preserve failures, variants and module/hash/address identity before using native evidence in Rust.

**Tested how:** Exact database/source-row/hash audits, original project bytes, callback/disassembly rollback, private PE data/relocations,16 x87 context/recovery checks,44 existing-function switch/owner controls, ambiguity/overwrite refusal and C-only gamedb range audits. Detailed commands are in docs/validation.md.

**Result:** All research audits pass. Callback discovery adds8,992 bodies and bounded disassembly62. All2,462 disassembly addresses restore original entry/listing absence. All2,333 data records match PE bytes. Original project/catalogues remain unchanged; type/implementation/native-verification flags remain empty.

**Still broken or not tested:** Full decompilation, scope/ABI/names, native gameplay and campaign parity. The two-double-width pilot fails;45/46 orphan dispatch controls remain unmatched. Navigator omissions/range errors and rejected mixed/incomplete indexes remain recorded. Runtime tests were not rerun for documentation-only changes.

**Next:** Raw branch/table-bound/shared-tail evidence for unresolved dispatchers, x87 helper storage and runtime scope. Preserve the final private checkpoint near the owner's requested quota margin; gestures remain queued.

## 2026-10-05 Continued discovery and optional-module databases

**Changed:** Resumed authorized private research after the owner corrected an early stop. A private baseline clone adds 9,850 candidate exports. Seventeen optional modules expand saved coverage to 42 modules/122,721 entries with some pseudocode. Separate raw-flow/disassembly passes produce 34 and 11 temporary bodies. Public changes document evidence only.

**Why:** Whole-module export counts omitted callable entries and optional scope. Checkpoints are validation boundaries; preserve baseline provenance while continuing discovery, rather than treating a checkpoint as task completion.

**Tested how:** Original project/catalogue hashes; relocated pointer/RTTI records and bounded instruction flow; exact SQLite artifact/body audits; saved optional entry sets; five new failure recoveries/signature checks; independent rollback of temporary function/listing state; exact body retrieval and scoped gamedb control. Commands/counts are in docs/validation.md. Installed content stayed read-only.

**Result:** Discovery ledger:20,842 artifacts/9,850 bodies/zero discrepancies. Optional ledger:393 artifacts/23,987 successful body rows plus five preserved original errors/zero discrepancies. All16 known original failures have separate recovery bodies. Temporary 34/11 body ledgers and rollback checks pass. Discovery navigator has one range mismatch; optional navigator omits2,544 bodies and has779 mismatches, so exact path/hash/address retrieval remains authoritative.

**Still broken / not tested:** Complete discovery/runtime scope, original names/ABIs, exception coverage, optional activation, middleware/tool/variant exclusions and native behavior remain unresolved. Two analysis-adjusted flow attempts, initial writable-RTTI rejection, missing project-directory launch and Vulkan exception-handler analyzer error remain recorded. No Rust change, rebuild, gameplay capture, performance or multiplayer test.

**Next:** Continue optional orphan-target discovery and relocated callback/instruction-operand references across all42 snapshots, retaining versioned databases and rollback proofs. Gestures remain queued behind the documented research priority.

## 2026-10-05 Private database coverage and failed-export recovery

**Changed:** Continued private decompilation with a 126-path hashed PE inventory, discovery audits, three new loader-referenced module exports, eleven supplemental failed-body recoveries and a path/hash/address SQLite evidence ledger. Public changes are evidence documentation only.

**Why:** The 22-module export denominator omitted runtime modules and did not establish function discovery. Preserve original errors and uncertain signatures while improving database retrieval and recording unresolved coverage.

**Tested how:** Fresh baseline catalogue audits before/after; all 22 saved/installed/private hashes; read-only Ghidra instruction/storage/discovery inspection; eleven rollback signature checks; supplemental 301-artifact/body audit; exact VPhysics/Speex retrieval and a scoped gamedb CreateInterface control. See docs/validation.md for commands. Installed files were read-only.

**Result:** Across 25 selected modules: 88,884 identified addresses, 88,873 original bodies plus 11 supplemental recoveries. Original failure rows remain. Baseline 22-module audit finds 1,459,199 executable bytes outside functions and 13,830 unresolved pointer-slot observations. Supplement audit has zero discrepancies; fresh gamedb extension indexes 827 bodies, misses 263 and has 36 range mismatches. Native research remains outside Rust source checkouts.

**Still broken / not tested:** Complete runtime scope, discovery, recovered ABIs and native semantics remain unverified. No Rust behavior, game capture, campaign, performance or multiplayer test in this pass. Failed return-type recovery, initial debug setup, old extension-builder path rejection and rejected newline audit remain recorded privately.

**Next:** Review bounded unlabelled RTTI/vtable/orphan-code candidates and remaining optional loader branches on disposable projects, then extend a new audited database version. NPC gestures remain queued.

## 2026-10-05 Decompilation priority for the new chat

**Changed:** Updated AGENTS.md, STATUS.md and docs/DESIGN.md with the owner's new decompilation priority, plus a private inventory/failure checkpoint.

**Why:** Continue toward complete original-game coverage in a fresh chat while retaining the verified Rust build and precise limits of the database method.

**Tested how:** Read the existing 22-module coverage, export scripts, catalogue and gamedb audit documentation; enumerated installed DLL/EXE files read-only. Checked document consistency, Git diff and the public snapshot.

**Result:** Handoff records 87,783 successful exports/ten failures, exact-byte catalogue coverage, gamedb omissions/read-range errors and 126 installed DLL/EXE paths awaiting scope/dependency classification. The plan separates inventory, failure recovery, discovery, type/behavior recovery and verified Rust translation.

**Still broken / not tested:** No new Ghidra run, recovery/export, database rebuild or runtime modification occurred. Existing audit reports were inspected, not rerun. Complete decompilation, function discovery and semantic parity remain unverified; the failed exports and parser discrepancies remain open.

**Next:** The owner starts a fresh chat in outputs/hl2-rs-bevy. Read AGENTS.md, STATUS.md, docs/DESIGN.md and the newest private checkpoint, then execute the bounded decompilation plan before queued NPC gesture work.

## 2026-10-05 Fresh-chat documentation handoff

**Changed:** Reworked AGENTS.md, added STATUS.md and docs/DESIGN.md, updated this log and changed .gitignore to a source-only whitelist.

**Why:** Preserve verified state, existing authorization and failed approaches without rereading the entire chat.

**Tested how:** Reviewed the three templates against repository instructions, Git revisions, build metadata, recorded logs and the private checkpoint. Checked links, whitelist coverage/exclusions and the staged public snapshot.

**Result:** Handoff identifies active branches, both release packages, tested capabilities, unresolved fidelity work and the next bounded gesture plan. Historical entries remain available.

**Still broken / not tested:** Runtime code is unchanged; gameplay/build checks belong to28c0bb3 and are not rerun claims for this documentation edit. Full campaign/1:1 parity remains unfinished.

**Next:** Start a fresh chat in the Bevy checkout; read AGENTS.md, STATUS.md and docs/DESIGN.md before continuing.

## 2026-10-05 Native crosshairs and optional spatial batches —28c0bb3

**Changed:** Shared owned Windows glyph rasterization/placement, Bevy1080p/borderless launchers, optional spatial batches/render-candidate diagnostics and targeted retained build tooling.

**Why:** Correct blur/fractional reticles and investigate broad world batches without changing physics or forcing equal native sizes.

**Tested how:** Ran original HL2 at1080p,294 unit tests/19 owned checks, strict Clippy/fmt, packaged captures/replays and source audit.

**Result:** Seven Bevy captures and retained1080 captures match dot offsets: white22x22, pistol/SMG21x17. Movement26, weapons17, monitor/campaign18 and accepted images9 pass. Plaza/spawn partition images are identical; audit212 files/0 failures.

**Still broken / not tested:** Splitting stays opt-in; hall camera was rejected, frozen benchmark script was incomplete and25 known station03 material errors remain. Other fonts/colors/platforms and full campaign parity are not established.

**Next:** Preserve verified reticles and reconstruct authored NPC gestures.

## 2026-10-05 Shared materials and model activity —c2b5220 /322afaa

**Changed:** Shared authored material/lightmap handles and avoided unchanged GPU uploads; added owned label/activity/weight resolution and corrected unsupported Kleiner idle.

**Why:** Reduce preparation overhead and use model-authored animations instead of assumed labels.

**Tested how:** Unit/owned checks, controlled Kleiner capture, movement/weapon replays, matched actor/eye/monitor comparisons and local1080 profiles.

**Result:** Station02 material handles921→743. Kleiner resolves idle_subtle; valid metrocop idle_baton stays. Material package measured174.72FPS at the tested spawn, with limited scope.

**Still broken / not tested:** Native RNG/modifiers/full virtual IDs, gestures/facial animation and natural staging remain unfinished. Global eye reports differ after the intervening idle fix.

**Next:** Preserve raw delta/post transforms, masks and child dependencies before gestures.

## 2026-10-05 GPU skinning and cached BSP PVS —df6af1b /b888144

**Changed:** GPU skinning/current bounds/shared iris poses; conservative PVS across active player/monitor views with script visibility/fail-open behavior.

**Why:** Profiling found CPU skinning/uploads and preparation of out-of-view geometry.

**Tested how:** Synthetic/owned bounds/visibility checks, packaged actor/eye/monitor/campaign/gameplay fixtures and900-frame1080 profiles.

**Result:** Selected actor/eye comparisons preserve tested scope. CPU→GPU35.05→66.85FPS and PVS-off→on66.69→156.94 are separate local comparisons; simulation remains enabled.

**Still broken / not tested:** Area portals/occluders/LODs, complete animation and whole-campaign/native144FPS parity remain unverified.

**Next:** Preserve current bounds/view-union ordering and expand fidelity with the same regressions.

## 2026-10-05 - animated attention attachments

Actor gaze origin/forward now come from the owned animated eyes attachment, with model view-offset fallback. Eye presentation caches one skin pose per visible actor and shares it with iris projection, preserving cycler meshes. Malformed/singular/overflowing attachment records are rejected; PVS latching, native head-pose controls and full layered animation remain unfinished.


## 2026-10-05 - authored scene attention

Shared simulation now executes compiled LOOKAT events with normalized-time scene/event ramps, timed deduplicated interests, target aliases, pause refresh and cancellation expiry. Bevy projects both eyes toward one selected actor target; monitor/player/self behavior is capture-verified in a deliberately seeded security02 fixture. Retained simulation/reporting consumes the same queue. Head/facial controls and native random/tactical attention remain unfinished; this is not campaign or retail AI parity.


## 2026-10-05: shared pause/console and Bevy campaign host

Moved the retained console parser/history/cheat gates and resource-driven pause layout into hl2-ui. Both hosts now provide explicit input and consume the same canvas. Bevy opens Resume/Console/Quit, supports the existing command subset and preserves selection/capture/held-input ordering. Console rendering clips output to its panel and handles Unicode character boundaries.

Added asynchronous owned-map decoding with simulation/script clocks frozen while loading. Successful transitions replace map-owned draws/audio and carry landmark-relative eye position/inventory with rebased weapon deadlines; direct map starts fresh. Missing maps preserve the current level. Testing exposed repeated same-map loads from input-only changelevel brushes; shared touch handling now honors their 0x2 flag while retaining explicit ChangeLevel input. Full saved entity/global/player state and ordinary campaign completion remain unfinished.

267 normal tests, thirteen owned-install tests and strict Clippy pass. Packaged console/campaign captures pass 45 checks, including 720p/borderless1080p UI, actual paused audio sinks, two real station exit transitions, failure recovery and camera cleanup. Retained-host and weapon/movement regressions are recorded in validation.md. No game files or native research are published.


## 2026-10-05: shared effects presented in Bevy

Extracted retained projectile billboard/RNG/blur and impact selection/clipping into hl2-simulation, leaving rendering adapters in each host. Bevy now presents owned SMG grenades, energy balls, impact/explosion sprites and bullet marks with depth testing. Decals retain destination-color blending and doubled modulation, and follow moving receiver transforms. Testing found a retained door-mark failure: projection used raw MDL vertices instead of the animated idle pose and rejected the one-unit collision/visual gap. Both hosts now project onto the current posed mesh with a bounded same-receiver correction. Native studio deformation and complete particles remain unfinished.

Packaged effect fixtures pass 40 checks, including real observed draws, frozen simulation, secondary reserves, explosion completion and marked-door transform agreement. The full combined weapon fixture and retained packaged smoke are checked separately. Source assets stay installed and private capture evidence stays ignored.

## 2026-10-05: shared HUD presentation in Bevy

Moved the retained owned-resource HUD into engine-independent hl2-ui with an explicit ordered CPU canvas. Both renderers use the same layout/font/crosshair/animation logic. Bevy now draws health/ammo, weapon buckets, quick-info and secondary ammo through a separate overlay camera with normal/additive materials. Asset loading stays outside systems; glyph textures and meshes are reused.

Packaged white-crosshair captures match the accepted retail five-pixel positions at 720/1080. Selection preserves active ammo, secondary-panel motion is captured at start/intermediate/end, and the door/weapon and bench fixtures remain passing. Native font rasterization and blend/gamma equivalence are still partial. Audio, pause/console, projectile/impact presentation and campaign host migration remain open.


## 2026-10-04: Bevy entity, weapon and animated presentation bridge

Moved the tested entities/gameplay/NPC/projectile/selection implementations into hl2-simulation and retained host re-exports. Shared actor/viewmodel preparation preserves clip loading. Bevy now runs scene, weapon, moving collider, NPC/projectile, rigid-body and player state in retained order. Local entity meshes follow authoritative poses/visibility; CPU skeletal animation updates bounds; an independent viewmodel pass preserves the existing projection. F3, weapon buckets/wheel, confirming/fire/reload/previous, E use and G test impulse are connected. Quick-click and pause/selection suppression regressions pass. Packaged bench and station-door/secondary-weapon fixtures pass; HUD/audio/effects/campaign host migration remains next.


## 2026-10-04: shared collision and Bevy player movement

Extracted the existing collision/Rapier adapter, convex sweeps and NPC probes into `hl2-simulation` unchanged, preserving their 25 tests and retained-runtime imports. Bevy now uses the same Source-coordinate player and collision code at 15 ms per step, with input/look before simulation and camera presentation afterward. Default walking, flight toggle and pause/resume are supported; input capture consumes transition-frame mouse movement and held jump until release.

The packaged bench fixture passes 26 movement/pause assertions, including the native collision height, one-time air crouch lift with retained eye/momentum, air uncrouching and jump rearming. Rendering remains static and rigid bodies are frozen until presentation is synchronized. Weapons/HUD, pause UI/console, entity I/O/scenes, NPC animation/AI and audio remain migration work. See `docs/validation.md` for package and test evidence.

## 2026-10-04: separate Bevy/wgpu host preview

Preserved the original host on `macroquad-prototype` and added `bevy-migration` for the long-term Bevy/wgpu work. Main keeps the retained runtime until verified replacements are available. README and contributor guidance identify the branch and executable each feature belongs to.

The new host reads owned maps through the shared format/core crates and renders static BSP, displacement, prop and entity geometry with custom base/lightmap materials and a fly camera. Shared CPU/GPU texture caching and visible-material filtering correct the first preview's loader-budget exhaustion. BSP render winding is normalized before appending already-normalized models, correcting culled model fronts without altering shared collision data.

Validation: 241 normal tests and nine owned-install checks pass, with formatting and strict Clippy. Final packaged station-map captures at 720p and borderless 1080p were inspected, with no texture-budget or capture failures. Dynamic monitor and eye materials remain unsupported. Gameplay, collision, animation, AI, choreography, HUD and audio have not migrated; Source rendering and performance parity are not established. See `docs/bevy-migration.md` and `docs/validation.md`.

## 2026-10-04: G-Man speech and authored locomotion data

Fixed the first-map G-Man omission with explicit `cycler_actor` model and scene-actor support, corroborated by retail factory/RTTI and actor lookup. The packaged debug-camera fixture resolves 17 authored intro events. Its initial run exposed compressed voice failures; standard Microsoft ADPCM now decodes in memory, preserves recorded frame counts and successfully requests playback of both opening lines. Facial animation, gestures, intro cameras and compositing remain unfinished.

Added bounded PC AIN37 decoding and optional inspect/runtime reports, preserving hull offsets, raw masks/metadata and Hammer IDs. Added v48 authored movement records and activity/all-blend metadata, plus piecewise motion sampling, turning and positive/negative loops. Owned walk/run tracks match 80 units/1s and 125.87412 units/0.6s. These readers do not yet implement NPC pathfinding, weighted blending, motor planning or movement readiness.

Validation: 210 workspace tests, six owned-install checks, formatting and strict Clippy pass. The rebuilt launcher package passes 12 G-Man, 33 scene-controller/sequence and 20 secondary-projectile assertions. A graph census decodes 72 and rejects six mismatched revisions across 78 loadable installed maps; the known empty coast map remains rejected. Private evidence and game files remain outside public source.

## 2026-10-04: crosshair, secondary projectiles and authored scenes

Corrected the oversized unarmed crosshair by identifying the native height accessor and reproducing both texture-coordinate insets. Packaged720/1080 captures now match the original five pixel positions; odd viewport rounding also passes. Font rasterization and tone mapping remain separate work.

Added real SMG contact grenades and AR2 charging balls, separate cooldowns/reserves, reload interruption/veto, continuous collision queries, bounded blast obstruction/damage, bounce/expiry and owned effects. Native controls corroborate selected weapon-state behavior. Rapier physics, damage policy and effects still do not reproduce the full Source implementation.

Added a bounded Rust reader for compiled choreography and authored scene control/triggers/completion. Selected installed SEQUENCE clips follow the paused scene clock and restore their baseline. Unsupported movement readiness holds SECTION rather than inventing success. The first level still lacks NPC schedules/movement, gesture/facial layers and intro systems needed for complete playback.

MP3 cues decode into an in-memory PCM cache, resolving the trainstation music gap. Added bounded cheat-gated ent_fire through normal entity I/O and more detailed snapshots.

Validation:192 workspace tests, both owned-install checks, strict Clippy and formatting pass. The latest package passes27 existing weapon checks,20 projectile checks and33 first-map scene-controller/sample checks. Captures and limitations are recorded in docs/validation.md; private research and installed assets remain excluded from Git.

## 2026-10-03: air crouch, stable contacts and pause/console

Air crouching now tucks the feet once while preserving head height and momentum; standing clearance gates air unducking. Reviewed retail sliding rules and analytical world-brush/convex sweeps correct reproduced wall-contact jump interruptions. Native PHY compound integration and full VPhysics parity remain unfinished.

Added a working resource-based pause menu and bounded developer console with cheat gates, history/editing/completion, position/angle commands and owned map loading. Focus freezes simulation and suppresses attack/jump leakage. The menu uses the owned title font and calibrated 720p metrics. Full console, save/options and paused audio remain open.

139 tests and strict Clippy pass; packaged weapon, movement and console fixtures pass their meaningful assertions. Latest double-door collision and secondary-ammo HUD reports are queued for the next iteration. Private bench PHY research matches the measured original standing height but is not integrated into Rust yet.

## 2026-10-03: publication and automatic empty fire

Published the reviewed Rust source to kvalls/hl2-rs with fresh-checkout instructions, contribution boundaries and a labeled runtime screenshot. The public main branch starts from a clean source snapshot; older local research history stays private. Windows formatting, Clippy and unit checks run for main pushes and contributor pull requests.

SMG1/AR2 now use a reviewed empty-fire latch and per-weapon half-second sound throttle. Empty clicks retain the primary deadline and animation; the next eligible attempt reloads, and idle reload uses a strictly elapsed primary deadline. Four regression tests cover held/released input, throttle boundaries, independent weapon state and reload completion. Other secondary attacks, autoswitch ranking and custom reload flags remain incomplete.

## 2026-10-03: depth, secondary fire and retail input/movement

Separated OpenGL depth testing and writes in the vendored backend, preserving depth clears between sky/world/viewmodel passes. Matching trainstation views reproduce then remove hidden light shafts, entrance columns and barrier effects; a front-side capture retains the visible columns.

Added shotgun alternate fire with twelve pellets/two shells, one-shell primary fallback, native/script-derived pump deadlines and retained secondary reload interruption. Accepted health/battery pickups now use their installed item sounds. The original engine confirms a six-to-four-shell alternate discharge.

Selection now consumes already-held attack buttons when Slot/Wheel opens the menu, rearms each button independently, and observes ordered script release/repress transitions. The F1 overlay identifies the Rust runtime. Movement now preserves standing/duck jump gravity ordering, categorizes after the sweep, crops diagonal boost, permits negative speed-cap additions and retains the native next-tick rising-air friction factor.

Fresh-clone build instructions and contribution boundaries were added for public source sharing. Game assets, native analysis, captures and research tools remain separate. Full campaign, NPC AI and Source rendering/collision parity remain unfinished; docs/validation.md records the checks and package fingerprint.

## 2026-10-02

The original executable remains read-only, with the earlier save/config snapshots preserved. Work is focused on HL2 fidelity; no FAL assets or crossovers were added.

The first prototype supported map/prop display. This iteration adds brush submodels, displacement/prop collision, fixed-step movement, timed entity I/O and doors, primary lightmap atlases, material transparency/tint/two-texture scrolling, compressed skeletal animation, PCM WAV audio, crowbar/pistol gameplay and limited scripted sequences/map transitions. These are partial implementations; the README lists the missing systems.

Mouse-look uses macroquad's previous-minus-current delta: upward movement increases pitch, rightward movement decreases Source yaw. The Windows miniquad reader additionally converts absolute RAWINPUT positions into deltas; treating those positions as motion caused extreme jumps during automation. Interactive verification is recorded in docs/validation.md.

Regression tests cover ordered/delayed duplicate outputs and fire limits, door locking/completion, shuffle batches, post-idle completion timing, ammunition transfer, stationary hull overlap at a floor, and disabling killed dynamic colliders. Movement and animation tests cover deterministic replay, acceleration/friction, jump apex and hierarchy/interpolation.

Research tools and private decompiler output belong in ../../work/hl2-decompiled. They are development references only. The runtime builds from Rust and reads owned assets in place. The packaged executable, rather than target/debug, is the final launch.cmd validation target.


### 2026-10-02: weapon selector, impacts and separate sky world

- Preserve nested sound-script wave alternatives; use distinct crowbar flesh/world sounds and hit animation.
- Add receiver-clipped installed impact textures and inherited surface-property bullet sounds. Marks follow rigid props and brush entities; keep a bounded local pool.
- Implement six-bucket UI with installed weapon icons, wheel cycling, confirmation, cancellation and previous-weapon switching; cancel reload on switch. Only crowbar and pistol combat are implemented.
- Separate background geometry/props using BSP leaf/PVS data and sky-camera scale. No native game code or decompiler tooling entered this project.
- Validation: 36/36 unit tests and workspace Clippy pass; packaged interactive run recorded three hit decals and multiple metal-impact sound variants with zero audio errors. Wheel UI rendered correctly; live confirmation needs a faster-input retest because its timeout expired between automation calls. Installed maps 78/79 parse with only the known empty coast map failure.

### 2026-10-02: retail HUD, six primary weapons and sky visibility

The earlier selector was a provisional design. This pass replaces it with installed resource dimensions, fonts/cell metrics, corners, labels and input behavior, checked against selected retail methods and a localized original-engine capture. Numeric HUD animation rules, bounded low-health loops and QuickInfo progress/fades/warning audio now run. Native GDI rasterization, damage messages and the remaining HUD panels/gates are still incomplete.

Added .357, SMG1, AR2 and shotgun primaries, separate ammo reserves, native/script-derived cadence/spread, shotgun shell/pump/interruption behavior and installed model sound events. Crowbar collision now tries the full ray, shortened +/-16 hull, facing test and closest corner refinement over approximate Rapier geometry. The background renderer now checks the camera BSP leaf's 3D-sky flag.

Validation: 79/79 workspace tests, strict Clippy, formatting and diff checks pass. Packaged input regression records 64 attacks, 82 impacts/marks, 6 model sound events, 199 audio requests, zero audio errors, two crowbar impact variants and one low-ammo warning. Actual Computer Use wheel/left/right/Q/Escape input verified selection with zero shots. A 60-frame hidden-sky test recorded zero sky frames. Installed maps remain 78/79, with the existing empty coast map rejected. See docs/validation.md for artifacts, current package fingerprint and limits.

Original-game research writes, localization copy, inspection tools and native pseudocode stay outside the Rust project. The original test process was closed, stock cfg manifest unchanged, no native DLL patched and no FAL/crossover used. Full NPC/campaign/shader fidelity remains substantial work; parser coverage is not campaign completion.

### LDR sky asset groundwork

Added a bounded six-face LDR sky loader resolving VMT base textures and transforms, with eight synthetic tests. HDR rendering, cube drawing and leaf gating integration remain unfinished. All 109 workspace tests and strict Clippy passed; the final package was rebuilt and smoke-tested.

### 2026-10-03: database audit, sky rendering and movement collision

Integrated the six-face LDR sky background with verified retail orientation, installed material transforms, clamped sampling and separate BSP 2D/3D eligibility. It draws before scenery/world without depth writes. Matched original-engine views corroborate cloud orientation; HDR, fog and sky polygon masks remain unfinished.

Player collision now excludes retained NPC-only world clip brushes, correcting the oversized invisible station-bench blockers. Grounded crouch commands and maximum speed are separated for jump boost; airborne crouch retains full air acceleration. Regression checks cover clip masks, duck boost caps, backward overspeed, air strafing and chained released jumps. Native PHY geometry and full movement equivalence remain open.

The supplied Database Method was tested privately across all exports. Unchanged gamedb misses functions and some boundaries/call targets, so an exact-byte catalogue keyed by module/address preserves the complete export inventory and known failures alongside the supplemental navigator. This improves traceability without marking semantic parity automatically. The original game also confirmed three-shell shotgun secondary behavior: double discharge leaves one shell, and the next secondary action fires it as a single shot. See research and validation for measured coverage and packaged checks.

### 2026-10-03: air crouch, console, static PHY and secondary HUD

Air crouching preserves head height with a one-time foot lift, and stable world/convex hull sweeps correct reproduced wall-jump contacts. Added a resource-driven pause menu and bounded console with cheat gates, history, completion and map loading. Original ground-duck timers, prediction and full Source command/UI coverage remain unfinished.

Added a bounded original Rust PHY reader and separate installed convex pieces for supported solid static props. Bench standing height now agrees with the owned original measurement. Rotating-door collision applies the visible model's initial idle pose, fixing the station entrance's closed-door walkthrough and invisible open-shaped blocker.

Primary ammo stays visible during weapon selection. Unarmed crosshairs use the installed white default sprite; armed crosshairs retain installed glyphs. SMG/AR2 secondary reserves, carry limits, pickups and ALT counters now work. Their actual grenade/energy-ball attacks and the remaining weapons are still missing. See validation for package fingerprints, tests and the known trainstation MP3 music gap.

### 2026-10-04: Barney scene movement and actor-aware speech

Added Barney's native default model and owned MDL eye metadata, dedicated NPC collision masks/probes, bounded human ground routes and authored central walk/run locomotion. Scene MOVETO requests now persist while SECTION waits for actual arrival; the controller continues while only the scene clock is paused. UI pause freezes both, blocked routes hold the gate, and cancel/script ownership prevents stale pose changes. The controlled security03 fixture verifies player blocking, arrival-driven door output and cancellation without a manual scene Resume. It explicitly seeds a preceding authored target and is not a complete campaign replay.

Symbolic speech carries the resolved actor model through the installed gender registry and retains native tagged wave/fallback behavior. The first-map owned census resolves all 72 actors and decodes all 75 registered alternatives. Native RNG/mixing, localized combined lines and lip sync remain open. See validation for tests and packaged checks; native motor timing, weighted blends, general NPC AI, facial/eye rendering and the playable first level remain unfinished.

### Bevy audio adapter

Shared owned sound-script selection, actor gender, WAV/MS ADPCM/MP3 decoding and viewmodel event cursor now serve both hosts. Bevy preloads references outside systems and plays ambient/scene/weapon/HUD requests through monitored audio sinks, including host pause/resume. Packaged door/weapon and controlled scene fixtures, full tests and strict Clippy passed. Source spatial audio/DSP, soundscapes and lipsync remain unfinished; this does not complete the campaign.

### Bevy sky, iris projection and prop door swing

Restored owned LDR cube/miniature sky passes before the playable world, with current-leaf visibility and depth occlusion. Eyes now use authored studio metadata and separate iris textures; gaze uses a bounded visible player/NPC approximation. EyeRefract uses its owned Eyes_dx8 fallback, with refraction/flex/glints still missing. Prop door use carries the opener position to linked leaves and respects explicit swing direction, fixing the opposite inward/outward entrance behavior. Native door blockers and full gaze/choreography logic remain unfinished.

## Deferred prop-door opener inputs

Added shared OpenAwayFrom target resolution, preserved locks/fixed directions and stopped repeated opening inputs from resetting the swing. Two regression tests and an owned packaged entrance fixture cover named/current origins, player/caller/activator lookup and both sides. Full native door linkage/blocking remains unfinished.
