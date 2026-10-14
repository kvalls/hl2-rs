# MODLOG

## 2026-10-15: friendly game installation setup

**Agent/model:** Codex Desktop, GPT-6 family (exact model identifier not exposed). No subagents.

**Changed:** an independent bootstrap window with editable/pasteable folder, native Browse, Retry detection, Continue and Cancel; remembered per-user installation, automatic recovery/discovery, `--setup`, `--no-setup` and a setup-preview diagnostic. Root/hl2-subfolder normalization and bounded map/core-content checks reuse source-assets/VFS. A short-lived helper owns the setup window's event loop before gameplay starts. Added beginner **Setup steps** near the top of README. Fixed discovery returning its raw rather than validated root. Replaced two brittle constant-pointer assertions in an existing physics test with checks of the selected tables' damage behavior; no physics runtime changes.

**Why:** owner requested a friendly first launch so new playtesters can locate their installed game without command-line path configuration.

**Tested how:** `cargo test --locked --workspace` (498 passed, 48 ignored, none excluded); focused setup tests 5/5; install fixtures and explicit owned readiness test; strict workspace Clippy, formatting, diff/whitelist checks. Release package rebuilt and executable/shader hashes verified. Packaged auto-discovery saved a test preference and captured station02; invalid noninteractive root exited 1. Setup ready/missing screenshots were visually inspected. Evidence and rejected captures: `artifacts/friendly-setup-20261015`; details in docs/validation.md.

**Result:** setup, remembered-path recovery and beginner instructions are implemented on `codex/friendly-game-setup`, based on the existing NPC validation checkout. In-game menu/settings code is unchanged. Initial blank diagnostic captures were retained; preview now waits for renderer readiness. The initial full-suite failure from constant-address comparison is corrected by behavior assertions and the complete suite passes.

**Still broken or not tested:** native picker/clipboard interaction, title-bar cancellation and the complete interactive Continue-to-game transition could not be exercised because Windows computer-use timed out through recovery; control/validation/response paths have programmatic tests. macOS/Linux setup is **not tested**. Readiness is not a full integrity/compatibility scan. Existing campaign limitations remain.

**Next:** integrate the scoped setup commit independently of the unrelated NPC branch history; manually exercise native picker/clipboard on supported desktops when computer use is available.

## 2026-10-14 session 17: metrocop AI fixes from the first packaged AI-on runs (`wip/npc-ai-20261014`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents. Fixes the cloud sessions' SDK-only AI code (`claude-opus-5-5`, cloud) found in its first packaged runs.

**Changed:** (1) the metrocop AI's activities (ACT_PUSH_PLAYER, baton on/off, police harass, melee, flinches, weapon-translated move/idle/attack) are preloaded for npc_metropolice models (`models::METROPOLICE_AI_ACTIVITIES`): before, their sequences were never loaded, so shove/baton tasks failed and schedules reselected every think; (2) Weapon_TranslateActivity in `SceneMotor::set_activity` from the SDK stunstick/pistol/SMG1 acttables (`ai::host::weapon_translate_activity`; kept only when the model has the result), so ACT_MELEE_ATTACK1 plays the stunstick swing; (3) CAI_SentenceBase::Speak gating: FOkToMakeSound/JustMadeSound per NPC (equal or lower priority culled until 2 + U(1.5, 2.0) s, SDK priorities per sentence group, pain/die always), diagnostics `culled_sentences`; (4) `ai::Conditions` serializes as condition names (a u128 above u64::MAX blanked the whole `ai` report: "number out of range"); the report shows serialization errors instead of null; (5) Weapon_StunStick.Melee_Hit/Melee_Miss preloaded (the hit was refused at playback).

**Why:** owner: metrocops shove and escalate when the player walks up close, not only on +USE; validation checklist steps 3-4.

**Tested how:** hl2-simulation 270, owned tests (HL2_ROOT, all pass), strict Clippy, fmt. Packaged d1_trainstation_02 `--ai`, plaza cop #427: (a) +USE x3 from 50 units (SDK PrecriminalUse: warnings only, no shove at that range): warning 1 line, baton activation animation at 2, AdministerJustice swing at 3 knocks the player ~100 units back, return to post; before the fixes the cop never animated and queued a BACK_UP sentence every think (~80 in 13 s). (b) Walking into him: pushplayer shoves from warning 1, baton at 2, swing at 3 (player pushed back ~100 units), re-approach -> repeated swings with stunstick hit sounds; 3 sentences spoken, 7 culled; health stays 100 (precriminal). Native reference: research 14a native-metrocop movie (plaza cop swing, red fade, punch, displacement, health 100); not a side-by-side timing comparison.

**Result:** the trainstation metrocop escalation chain runs in the packaged game.

**Still broken or not tested:** NPC weapon models (baton not in hand); red fade/punch visuals not checked in captures; native timing comparison; checklist 5 (scripted cops with AI on) and 6 (F4 spawns); policing warnings at the terminal cop (route); citizen +USE speech.

**Next:** STATUS session 17.
## 2026-10-14 session 17: NPC random look targets and head turns (DESIGN 14h, `wip/npc-ai-20261014`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents.

**Changed:** `attention` gains the CAI_BaseActor random look queue: ramped interests (CAI_InterestTarget_t::Interest), position targets, `LookTargets::maintain` (explicit interests cut random ones to ramp + 0.2 s and delay the next pick 1 s; else PickLookTarget(1.5, 2.5): PickRandomLookTarget over drawn studio-model entities and the player within 360 units, a moving player or 1 in 3 standing held 1-4 s, others random importance x10 NPC x10 moving, FVisible + ValidHeadTarget, fewer considered than RandomInt(0,5) -> MakeRandomLookTarget 128 units ahead at influence 0.01; next pick at duration - 0.4), `attention::think` every 0.1 s for animated-face classes (cops, Combine soldiers, citizens, Barney, Alyx, Breen, Eli, Kleiner, Mossman, Monk, Dog, generic actors) with or without `--ai`, COND_IN_PVS gating from the host's BSP PVS (`eyes::attention_pvs`; outside the PVS head and eyes keep their last values; unknown fails open). Head control takes random + explicit interests at Interest() x HeadTargetValidity with EyePosition targets (NPC eyes attachment). Eyes take the newest valid active interest (explicit first); the old nearest-visible fallback (which also ignored walls for the player: it excluded the world entity id) stays only for actors without an animated face (cyclers). Report keys `random_look_targets`, `heads`.

**Why:** owner report (session 17): citizens side-eye the player with their eyes only and stare forever; native turns the head and glances periodically.

**Tested how:** unit tests (Interest ramp curve, glance expiry/truncation/explicit priority, made-up point and scripted actors), hl2-simulation 270 passed, strict Clippy, fmt. Packaged d1_trainstation_02 fly eye (-4110,-2180,128) yaw 290 beside the dispenser queue (#580-582), 13 s: #580 turns his head up to ~49 degrees toward the player, holds 1-2 s and looks ahead again, about half the samples on the player; #581 alternates between #580 and the player; #582 looks at the others or ahead. Native at the same spot (original-oracle T071459Z, 14 frames ~2 s apart): the front citizen's head is turned to the camera in roughly 60-70% of frames and ahead in the rest; no continuous stare in either. A first spot (eye -4180,-2256) was behind the dispenser wall (rejected; the dispenser correctly blocked sight). PVS gating on the player alone froze Kleiner on Barney's monitor (head pitched up, eyes rolled back; rejected), so the PVS test includes the active monitor's point_camera (CPointCamera::SetupVisibility): Kleiner's feed is back to 6 changed pixels, Breen's broadcast feed differs by 1186 pixels from random glances when no LOOKAT is active (not verified against native). Regression batch artifacts/s17-look2-regression (player-only gating): 25/17/26 (doll01 only), no panics.

**Result:** NPCs glance at the player and around with head turns instead of staring.

**Still broken or not tested:** Breen's broadcast glances vs native; citizen +USE speech (TLK_USE via the response system: "We can't be seen talking", owner); tactical (enemy), navigation look-ahead and combat look-forward branches; synthetic queues; expressions; moving brush entities as candidates; entity sphere order (ours: player first, then load order); native random stream; no numerical native head-angle comparison (frames only); the citizen model variety (natives use random citizen heads).

**Next:** AI-on checklist (warnings/shove at the plaza and terminal cops), NPC weapon models, merge after validation.
## 2026-10-14 session 17: step 14 merge onto main (local validation) and metrocop idle (`local/validate-npc-ai-20261014`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents. Builds on the cloud sessions' `wip/cloud-npc-ai-20261010` (Claude Code cloud, `claude-opus-5-5`, SDK-only).

**Changed:** merged `wip/cloud-npc-ai-20261010` onto main 675c9df (conflicts kept both sides: report keys ai/npc_spawns + phys_explosions/gibs, `--ai` + `--frag-swept-box` help, the AI's pre-move player impulse before the gravity-gun max speed, Scene `spawned` + `keyvalue_overrides`, physics `add_entity_hull` after the constraint code). The owned HEV sentence length test skips METROPOLICE_* (their `{Len 1.5}` is a placeholder). **Metrocop idle (owner report: cops lean as if against a wall):** cops without DefaultAnim played "Idle_Baton" (a sequence with no activity; leaning forward, arm across the chest); they now start (and return after scripts) on ACT_IDLE translated by their weapon's acttable (pistol -> ACT_IDLE_PISTOL, smg1/ar2/shotgun -> ACT_IDLE_SMG1, stunstick none) with weighted selection (batonidle1 x2 / batonidle2 x1); rigs preload ACT_IDLE_PISTOL/ACT_IDLE_SMG1. Oracle (private): `!cmd` view suffix runs ORACLE_CMD (backup run_view_oracle.py.bak-20261014b).

**Why:** owner direction (session 17): merge the physics/arsenal work, then step 14; owner report on the cop idle pose.

**Tested how:** workspace tests (hl2-simulation 266 + new `metrocops_idle_on_their_weapon_translated_idle_activity`; hl2-bevy's two release-only should_panic image tests fail as before), owned tests (HL2_ROOT, all pass after the HEV test fix), strict Clippy, fmt. AI-off regression batch artifacts/s17-ai-regression (before the idle fix): 25/17/26 (doll01 only), finals vs s17-regression max pixel <= 7 (exposure noise), Breen/Kleiner feeds identical. Native (original-oracle): T065205Z ent_text shows terminal_side_cop idling on sequence 8 batonidle2 (ACT_IDLE, SCHED_IDLE_STAND, behavior Policing) upright with arms down; T065343Z a prop_dynamic police.mdl on batonidle1 is upright too; T065122Z at 80 units the cop runs SCHED_POLICE_FACE_ALONG_GOAL with ACT_POLICE_HARASS2. Our offline decode of batonidle1/2 is smooth across its 30-frame sections (the clip decoder was not the cause). Packaged after the fix: cop #577 upright, arms down, same view as native (artifacts/s17/cop577-fixed-compare.png). d1_trainstation_02 launch to frame 30 with the spawn pool: ~7.5 s (main not timed side by side).

**Result:** metrocops idle like native (minus the baton in hand).

**Still broken or not tested:** NPC weapon models (native cops hold the baton); AI-on plaza/terminal checks (checklist 3-6) in progress: the hall walk to terminal_side_cop hits the hall wall (route needed); citizens/Barney keep their idle_subtle default (not compared); head/eye attention (owner: NPCs stare with their eyes only; native turns the head and glances periodically).

**Next:** AI-on validation, NPC head/look attention research, weapon models.
## 2026-10-14 session 17: explosive props (`wip/cloud-physics-20261010`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents.

**Changed:** prop_data `explosive_damage`/`explosive_radius` (PropData, over the entity's ExplodeDamage/ExplodeRadius keyvalues, not on *_override props). CBreakableProp::Break of such a prop queues ExplosionCreate at its center (magnitude and radius scaled by modelscale, truncated to integers; radius 0 = magnitude x 2.5, env_explosion); the host detonates them after the tick's damage and before gibs spawn (native order: the explosion precedes PropBreakableCreateAll; a one-tick deferral let the barrel's own gibs block its blast), repeating so chains resolve in the tick (bounded at 32). `Projectiles::explode_props`; `grenade_explosion` takes position/magnitude/radius.

**Why:** owner question (session 17): explosive barrels did not explode.

**Tested how:** unit test `breaking_an_explosive_prop_queues_its_explosion`, hl2-simulation 233 passed, owned tests (HL2_ROOT, `--ignored`, workspace: 47 passed), strict workspace Clippy, fmt. Packaged d1_canals_01 (oildrum001_explosive: health 20, explosive 120/256), walking, eye (406,-2790,452) yaw 90 pitch -14, pistol at barrel #635: it breaks on the 4th shot, explodes (magnitude 120, radius 256) and chain-detonates #634 in the same tick; 17 gibs; the player (128-160 units away) takes about 60 + 45 and dies (SDK falloff). In fly mode the player takes no damage at all (pre-existing: player damage runs only in the walking branch). The packaged asset cache (bin/bevy-assets) must be rebuilt for new PropData fields: target/release runs used the old cache and saw no explosion.

**Result:** explosive barrels explode and chain.

**Still broken or not tested:** native comparison not done; fire is not modeled: native barrels ("ignite" "halfhealth", "explosive_resist") catch fire at half health from bullets and explode after burning, and blasts make them burn 0.1-5 s before exploding; ours explode at 0 health, chained ones instantly; PROPINTER_PHYSGUN_BREAK_EXPLODE (propane tanks) variant; explosion effect/decal flags (SURFACEONLY etc.) not distinguished; barrel gibs/flames.

**Next:** record the physics/arsenal leftovers, regression batch, merge to main, then step 14 (owner, session 17).
## 2026-10-14 session 17: explosions break prop_data props; SDK blast blocking (`wip/cloud-physics-20261010`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents.

**Changed:** `Projectiles::grenade_explosion` (frag, SMG grenade, RPG) now sends DMG_BLAST damage to every prop_physics (the entity damage dispatch resolves prop_data health and skips unbreakable props); before, only props with an explicit `health` keyvalue were damaged, so prop_data props (crates, pallets, bottles, boxes) were only pushed. Blast line of sight follows CGameRules::RadiusDamage: the world or a brush entity blocks; an interposing studio-model entity never shields fully (the CONTENTS_SOLID re-trace's StandardFilterRules skips non-brush models): a physics object absorbs mass/350 of the damage (all from 350 kg), others 25% (`blast_blocked_fraction`); before, any prop shielded completely. `Physics::filtered_ray`, `Physics::entity_mass`.

**Why:** owner question (session 17): do explosives break props? They did not (owner saw bottle #702 untouched next to an RPG blast).

**Tested how:** unit test `interposing_props_absorb_blast_by_mass_instead_of_shielding`, hl2-simulation tests 232 passed, strict Clippy, fmt. Packaged d1_trainstation_02, eye (-3490,-2110,128) fly, yaw 236: RPG (pitch -30, blast at (-3565,-2210,64)): before the fix bottle #701 broke only by impact, bottle #702 survived because box #700 blocked the ray 1.5 units before it; after: box #700 and bottles #701/#702 break (6 glass chunks). SMG grenade (pitch -20) and frag lob (pitch -60, blast (-3516,-2165)): the same three break. Native (original-oracle T060459Z, same eye, pitch 30, weapon_rpg): box and both bottles gone after the blast.

**Result:** frag, SMG grenade and RPG blasts break prop_data props like native in the tested spot.

**Still broken or not tested:** the cardboard box's gib is a ragdoll gib (skipped until ragdolls); our glass chunks flew ~300 units while native shards stayed near the spot (not investigated); no scorch decal; explosive props (prop_data explosive_damage/radius: barrels) do not explode; non-lethal blast damage does not ignite props (CBreakableProp::OnTakeDamage Ignite); robust NPC radius damage (edge spill) and BodyTarget points not modeled.

**Next:** explosive props, then the session 16 open items.
## 2026-10-14 session 16 (continued): env_physexplosion, prop OnPlayerUse, TraceBleed, world-collision-disabled constraints (`wip/cloud-physics-20261010`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents.

**Changed:** `hl2_simulation::phys_explosion` (env_physexplosion Explode input: targets by name, else class within the radius, else everything in the radius; SDK falloff and explosive damage force applied at the explosion origin; DMG_BLAST damage unless NODAMAGE; report `gameplay.phys_explosions`); prop_physics with spawnflag 256 fire OnPlayerUse on +USE (pickup path and ray-use path); world constraints with SF_CONSTRAINT_DISABLE_COLLISION make their body ignore static colliders without an entity (BSP, displacements, static props); TraceBleed (CBaseEntity::TraceBleed: 1/2/4 noisy traces 172 units along the shot behind a hit NPC, Blood/YellowBlood decal groups by BloodColor, machines none; `Inventory::blood`, `Impacts::add_blood`); `Physics::remove_body` updates the query pipeline (a faded gib left a stale entry: one intermittent Rapier 'No element at index' panic in batch s16b, d1_trainstation_01, where debris bottles #869/#870 authored ~47 units above the displacement now shatter on landing under the glass table); movement-report sample field `awake_bodies`. Fixtures bevy-tictactoe-use-spins-block, bevy-trace-bleed-cop-wall. Oracle (private): `!use`/`!unuse` and `!npc` (npc_create, ORACLE_NPC) view suffixes (backup run_view_oracle.py.bak-20261014).

**Why:** owner items 0c (env_physexplosion, the playground tic-tac-toe blocks and ghost swings) and 0b (TraceBleed world blood behind NPCs, native blood comparison).

**Tested how:** unit tests (explosion impulse band and blast damage; TraceBleed trace counts, wall hits, mechanical NPCs), owned tests (decal preload count 56), strict Clippy, fmt, workspace tests; packaged: +USE on block 5 fires one explosion and the block turns (before: it sat 4.7 units inside the frame rod and could not move), native T001349Z at eye (-4540,-3599,64) pitch 15 yaw 0; three pistol shots at terminal_side_cop posed at (-3330,-2560,64) in front of the hall wall: 3 blood decals on the wall behind him (bleed-side.png), native T002821Z (npc_create cop at the same spot): splats on the same wall/pillar and on the chest; native's are larger and the native cop died after 3 shots (ours survives: NPC health is item 3). Regression batch artifacts/s16b-regression: weapons 17/17, attention 26/26, movement 25/26 (doll01 only), no panics; finals vs s16-regression mean <= 0.51; Breen screen feed differs by 24 pixels at the mouth (one lip-sync frame, to recheck), slate feed identical.

**Result:** the tic-tac-toe blocks react to +USE; shot NPCs leave blood on the wall behind them like native.

**Still broken or not tested:** native turns the block about a quarter turn and it settles within 0.3 s; ours spins at the angular speed cap for over 6 s (open, research needed: IVP hinge friction units / impulse transfer / angular drag); swing pushes need ropes (off); PUSH_PLAYER; carousel thruster.

**Next:** STATUS session 16.
## 2026-10-14 session 16: prop data, chunk gibs, displacement and clip collision, world hinges (`wip/cloud-physics-20261010`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents.

**Changed:** (1) DESIGN 13e steps 4-5: the whole prop_data block resolves over its propdata.txt base chain (`World::model_prop_data`: dmg.bullets/club/explosive, damage_table, breakable_model/count/skin) and the BreakableModels chunk sets (`World::breakable_chunks`); entity damage carries DMG_* bits (`projectiles::Damage::kind`); breakables take GetBreakableDamage (`gameplay::breakable_damage`, CBreakable func_breakdmg defaults for func_breakable/func_physbox); player bullets and melee on breakables go through the entity damage path (props with prop_data health now break under fire: before, only props with an explicit health keyvalue did, without gibs or break sound); glass impact table for damage_table "glass" (`impact_damage::GLASS_TABLE`, retail registry = SDK); gibs take the parent's skin and the break block's collision group (fadetime -> debris); breakable props without .phy gibs break into their chunk set (`Gibs::spawn_chunks`: count, max size, mid-plane points, 100 u/s burst, AlignBoxes, breakable_skin, 5-10 s fade); the host preloads gib skins and used chunk models. (2) Physics fixes found on the way: displacement colliders reversed (BSP clockwise winding made Rapier's oriented trimesh inside-out: props fell through displacements) and built without internal-edge correction; player/NPC clip-only brushes have no contacts with physics objects and shot/use traces skip them (`Physics::ground_ray` keeps them for the player's ground traces); logic_collision_pair startdisabled; SF_CONSTRAINT_ASSUME_WORLD_GEOMETRY world hinges as revolute joints after the SDK mass-center snap onto the axis; bullet/melee push = ApplyForceOffset at the hit point with the ammo's BULLET_IMPULSE / melee damage x 300 kg in/s (`Physics::impulse_at`, `gameplay::bullet_impulse`). Fixtures bevy-pistol-breaks-bottle-skin4, bevy-pistol-breaks-teeter-seat-chunks.

**Why:** owner item 0a (propdata damage tables/modifiers, chunk sets, parent skin on gibs) and 0c (assume-world-geometry hinges). The pot falling through the playground and the teeter seat flying off surfaced while testing them.

**Tested how:** unit tests (modifiers, glass table order, chunk placement/alignment/skin, gib skin/render key, clip brushes vs bodies and traces), owned tests (prop_data modifiers/tables/chunk sets; 18 + 24 owned pass), workspace tests (the two known release-only Bevy should_panic tests run in debug), strict Clippy, fmt; packaged fixtures and native oracle views (docs/validation.md session 16); regression batch artifacts/s16-regression (same as s15b).

**Result:** bottles and chunk-set wood props break under fire like native, in the right skin; props rest on displacements; the teeter seat sits on its hinge.

**Still broken or not tested:** the teeter seat rocks far less than native under pistol fire and native broke it in 5 shots (ours 8); ragdoll gibs; explosive props; gib limits; carousel (SF 9 + phys_thruster) not checked; other displacement maps not checked visually.

**Next:** env_physexplosion + OnPlayerUse (tic-tac-toe blocks, ghost swings), then the remaining item-0 work (STATUS session 16).
## 2026-10-13 session 15 (end): debris props (`wip/cloud-physics-20261010`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents.

**Changed:** prop_physics with SF_PHYSPROP_DEBRIS (4) use a DEBRIS collision group (SDK CGameRules::ShouldCollide: COLLISION_GROUP_DEBRIS collides only with COLLISION_GROUP_NONE): world and ordinary props yes; player movement (incl. the exact convex path), the player shadow, NPC hulls/models, thrown frags and other debris no; traces (bullets, +USE, gravity gun) still hit them. 18 of 66 prop_physics on d1_trainstation_02 carry the flag (e.g. bottle #701, box #700). Fixture bevy-carry-bottle-shove now starts at (-3356,-2520,128) (the old spot put the walking hull inside world geometry: stuck, so no crouch).

**Why:** owner item 0d (carried bottle shoved into the floor, crouch while carrying): crouch works while carrying; the carried debris bottle overlapping the crouched hull froze the player via CheckStuck, which native avoids because debris never collides with the player.

**Tested how:** unit test (a debris prop lets the player hull through and still stops a pick ray; a non-debris prop blocks), workspace tests, strict Clippy, fmt; packaged: crouched carry looking down: no stuck samples, no hull contacts (before: stuck from d186 with a 0.6-unit overlap), bottle minimum z 66.5 (floor rest 66.6). Regression batch artifacts/s15b-regression: weapons 17/17, attention 26/26, movement 25/26 (doll01 only); finals vs s15-regression mean <= 0.59, feeds identical.

**Result:** debris props behave like native toward the player; the carried-bottle freeze is gone.

**Still broken or not tested:** interactive-debris return to debris at rest (irrelevant until non-debris props are converted); native comparison of walking through debris; owned tests not rerun for this change (no asset path changed).

**Next:** see STATUS session 15 and work/publishing/next-session-prompt-20261013c.txt.
## 2026-10-13 session 15 (continued): studio decals along the shot (`wip/cloud-physics-20261010`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents.

**Changed:** weapon hits keep their shot direction (`Inventory::impacts` entries gain it; `Impacts::add_shot`), and NPC studio decals are shot along it instead of the reversed trace-shape normal.

**Why:** FX_ImpactTrace shoots decals along the ray; the trace shape's normal missed the skin on grazing hits.

**Tested how:** workspace tests, strict Clippy, fmt; packaged `bevy-studio-decal-cop.json`: 3/3 shots build blood decals at the hit points (was 2/3), sdecal4-zoom.png. Owned tests not rerun for this change (no owned-data path changed).

**Result:** blood lands where the bullet hits.

**Still broken or not tested:** native comparison of NPC blood.

**Next:** handoff.
## 2026-10-13 session 15 (continued): VPhysics constraints (hinges), spawn-embedded CCD (`wip/cloud-physics-20261010`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents.

**Changed:** (1) `Physics::add_constraints` (SDK physconstraint.cpp ActivateConstraint/CreateConstraint): phys_hinge (revolute about hingeaxis - origin; hingefriction x 1000 kg in^2/s^2 as a capped zero-velocity motor), phys_ballsocket, phys_ragdollconstraint (spherical with x/y/z limits in the constraint angles), phys_constraint (fixed) as Rapier impulse joints between the attach1/attach2 bodies (missing or non-physics = a fixed world body); SF_CONSTRAINT_DISABLE_COLLISION. Reported, not built (`constraint_errors`): phys_lengthconstraint (rope joints rejected: the swing seats first dropped ~19 units then were flung to z 245, also with CCD off for jointed bodies), constraints with SF_CONSTRAINT_ASSUME_WORLD_GEOMETRY (native BecomeHinged; the impulse teeter-totter hinge inside its static base drifted 115 units; multibody joints panicked in Rapier: "Matrix slicing out of bounds"), unknown attachments. (2) CCD gating fix: props authored interpenetrating a static entity (found once after the first step, contact deeper than 1 unit; world geometry and NPC hulls excluded) keep permanent CCD: with this morning's gating the teeter-totter seat was ejected from its base (to -4756 / -4406). A per-tick variant was rejected (kept CCD on for props pressed into hulls/terrain: plaza tip mean 7.7 ms). (3) The owned impacts test now expects the 11 studio-decal textures (32; it failed after the studio decal commit, which was pushed without rerunning owned tests).

**Why:** owner item 0c: the playground tic-tac-toe blocks fall and jitter; native holds them on hinges.

**Tested how:** unit test (hinged prop pushed off-axis keeps its pivot within 0.5 and does not fall; ropes reported; unknown attachment reported), workspace tests, owned tests (all pass; frag replay bests unchanged 27.6/16.4/3.8/12.8/44.1), strict Clippy, fmt. Packaged (`bevy-playground-constraints.json`, artifacts/s15/play*.json/png): the 8 tic-tac-toe blocks stay at their hinge heights z 63/46/29 (play1.png); teeter-totter seat stays at (-4848,-3488,19) (authored -4851,-3492,20); swing seats fall to z -10 as before; carousel rests at z -10 as before; plaza cop tip-off worst physics step 6.5 ms, mean 3.0 (before the CCD work 20-40 ms).

**Result:** the tic-tac-toe board works as in native; nothing else on the playground regressed.

**Still broken or not tested:** native comparison of the spinning blocks (only SDK + visual); swings (ropes), teeter-totter and carousel hinges (assume-world-geometry), constraint systems, breaking, TurnOn/TurnOff, phys_thruster, env_physexplosion (the playground ghost-swing pushes), func_physbox as dynamic bodies; props authored inside static entities keep permanent CCD (cost as before today).

**Next:** regression batch, then the owner's carried-bottle crouch question.
## 2026-10-13 session 15 (continued): studio decals on NPCs (`wip/cloud-physics-20261010`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents.

**Changed:** DESIGN 13f. (1) `modkit_core::studio_decals`: `project` clips the posed triangles facing the shot to the decal square (depth = radius), carrying barycentric bind positions, blended top-4 bone weights and UVs; `ray_distance` finds the front face along a ray. (2) Simulation: bullet hits on NPCs queue `Impacts::studio_requests` with Impact.Flesh (TranslationData F -> decals/flesh/blood1-5_subrect, drawn with their `$modelmaterial` decals/flesh/bloodN at the subrect's size), and bug bait splats on NPCs queue BeerSplash (drawn with decals/beersplashmodel, 64 units); `$modelmaterial` read from the VMTs, model decal textures preloaded. (3) Host `studio_decals.rs`: builds each request from the NPC's GPU-skinned draws (main-world mesh + joint GlobalTransforms x inverse binds), re-shoots along the shot onto the skin, spawns a decal mesh sharing the draw's `SkinnedMesh` joints (follows animation), tinted by the light state at the hit, hidden with the entity, 50 per model. Report `presentation.studio_decals`, `impacts.studio_created`. Fixtures bevy-studio-decal-cop, -bugbait-cop, -bugbait-citizen.

**Why:** owner item 0b: NPCs must take decals like native studio decals (bug bait stains, blood, paint).

**Tested how:** unit tests (clipped area and bind-space positions, UVs, weights across a bone seam, back faces/out of depth/degenerate rejected, ray front-face hit), workspace and owned tests, strict Clippy, fmt. Packaged (artifacts/s15): three pistol shots at posed terminal_side_cop: 2 blood decals on the belt/abdomen (sdecal3-zoom.png), one grazing hit builds nothing; before the skin re-shoot only 1/3 built (the hit lies on the convex trace shape). Bug bait at the posed cop from 200 units (80 units is inside the bait's non-solid grace period) builds a 1788-triangle stain over the torso; a tint-1 diagnostic showed the geometry and z-fighting (lift raised 0.05 -> 0.25). Native (oracle T160710Z: terminal_side_cop is behind a fence natively, bait hit the fence; T160814Z: bait at the queue citizen's back): no clearly visible stain on the dark clothing natively, nor in ours at the same views (ours hit a different citizen: the queue had progressed differently). Inconclusive; native blood on NPCs not captured.

**Result:** bullets leave blood on NPCs that moves with them; bug bait stains NPCs.

**Still broken or not tested:** native comparison of blood/bait decals on NPCs (needs an open, deterministic native NPC); shot direction is the reversed trace normal; TraceBleed blood on world surfaces behind NPCs; other game materials (antlion/alien/metal NPCs use flesh); per-vertex lighting of translucent model decals; ragdolls; thin seams at creases from the flat lift.

**Next:** VPhysics constraints (owner item 0c; also needed for ragdoll gibs).
## 2026-10-13 session 15 (continued): breakable prop gibs (`wip/cloud-physics-20261010`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents.

**Changed:** DESIGN 13e steps 2-3. `hl2_simulation::gibs`: a broken prop (CBreakableProp::Break) records its pose and motion in `Scene::prop_breaks`; `Gibs::spawn` (PropBreakableCreateAll / BreakModelCreateSingle / BreakModelCreate_Prop) creates each non-ragdoll .phy break block as a runtime prop body (`Physics::add_prop_body`, new; the frag keeps `add_projectile_body`) at the parent's pose and offset with the parent's velocity at that point, its angular velocity and the default 100 u/s outward burst (`burst` key parsed); gibs get projectile-range entity ids (holdable). `Gibs::tick` follows SUB_StartFadeOut/SUB_FadeOut/SUB_AllowedToFade: after `fadetime`, 3 alpha per 15 ms tick while outside the player's 2D view cone (0.766) and not held, removed at 0; PerformanceMode 1 spawns none. Host: gib meshes preloaded with the projectile models (`PreparedEffects::load` takes the world), `effects::present_gibs` draws them with a per-gib material lit from the map light state and alpha-blended while fading; report `gameplay.gibs`, `gib_draws`. Fixture bevy-pallet-gibs-fade.

**Why:** owner item 0a: breakable props must break into pieces like native. Owner question during the work: should gibs be newly spawned entities? Native: yes (BreakModelCreate_Prop CreateNoSpawn("prop_physics") at the break; models precached by PrecacheGibsForModel). Ours are created at the break too, as runtime bodies; full entities wait for DESIGN 14r.

**Tested how:** unit tests (offset in parent model space, inherited velocity + burst, ragdoll gibs skipped and reported, entity binding; fade postponed while seen or held, 85-tick fade, removal, no-fadetime gib stays), workspace and owned tests, strict Clippy, fmt. Packaged (artifacts/s15): pallet #327 launched into the hall wall breaks into its 7 chunks (gibs1/gibcap.png), lit like the intact pallet at that spot (intact.png); after looking away all 7 fade and are removed by 20 s (two at 2 s, the rest at their 10 s fadetime). Owned check (scratch test, removed): cardboard_box001a/002a break into `ragdoll` gibs (6 solids); pallet chunk .phy hulls match their render bounds in model space.

**Result:** wooden props break into physical pieces that fade like native.

**Still broken or not tested:** native comparison of the pieces (counts/spread/fade timing) not done; cardboard boxes produce no pieces (ragdoll gibs need constraints, owner item c); parent skin, placementOrigin attachments, propdata chunk sets, gib health/DAMAGE_EVENTS_ONLY, gib visibility culling; gibs are runtime bodies, not map entities (DESIGN 14r).

**Next:** NPC studio decals (owner item 0b), then constraints (0c, also unlocks ragdoll gibs).
## 2026-10-13 session 15: stacked-prop margin, prop CCD frame drop (`wip/cloud-physics-20261010`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents.

**Changed:** (1) Prop-on-prop contacts keep one VPhysics margin: Rapier adds both colliders' contact skins (narrow_phase: `contact.dist - skin1 - skin2`), so two props rested 1.0 apart while a prop rests 0.5 above the floor; dynamic prop colliders now run the solver-contact hook, which adds the shared skin back for pairs where both sides carry one. (2) Prop CCD gating: map props and runtime bodies (frag) no longer keep Rapier's hard CCD on permanently; each tick an awake body enables it only while its linear travel per step exceeds the body's own thickness (smallest extent of its pieces' union in the body frame). (3) Model-shaped hitboxes of hull-box NPCs are excluded in the contact-pair filter (Rapier enables a pair when either collider's active collision types allow it, so clearing the hitboxes' types alone did not stop props touching them: a pallet balanced on a plaza cop was pushed off by the animated hitboxes). (4) Movement-report sample field `step_ms` (whole fixed host step). Fixtures bevy-crate-on-box-stack, bevy-pallet-tips-off-plaza-cop, bevy-pallet-tips-off-npc-hull.

**Why:** owner playtest: props stacked on each other rest visibly higher than native; dropping the pallet off-center onto a plaza cop so it tips over still drops 46 -> 17 fps.

**Tested how:** unit tests (two stacked boxes: floor and stack gaps both 0.5 +- 0.1; hull-box test extended with a box across the hitbox face), workspace tests, owned tests with HL2_ROOT (all pass; frag replay bests lob 27.6, roll 15.6 (was 16.4), high 3.8, plaza 12.8, wall 44.1), strict Clippy, fmt. Packaged d1_trainstation_02 (artifacts/s15): Rapier profiler (temporary build, reverted) showed the cost was the CCD time-of-impact search (17-26 ms per step while the pallet rocked, 2-4.5 ms at idle; collision+solver ~2 ms); plaza cop #427 tip-off (`bevy-pallet-tips-off-plaza-cop.json`): worst physics step 2.5 ms over the run (before 20-40 ms for ~0.7 s while falling, 3-7 ms idle); crate #715 dropped on box #710 rests exactly 0.500 lower than with the old skin sum (crate-box 20.404 -> 19.904; box unchanged). Pallet street launch vs native T021822Z: identical to session 14 through the first bounces (y -4235/-4012/-3849 at f305/f320/f335), then diverges while tumbling: rest -3479 (session 14 -3495, native -3572). Rejected: Rapier soft CCD (speculative 8-unit prediction) for slow props: the launched pallet bounced to z 75 on its first ground hit and rested at -3467.

**Result:** stacked props sit one margin apart; the pallet-on-cop frame drop is gone in the measured case.

**Still broken or not tested:** native stack gap not measured numerically (pairwise 0.5 follows the floor measurement and the owner's visual comparison); tunneling of thin props moving less than their own thickness per tick through zero-thickness geometry is not tested; why ~7-38 props never sleep at idle (tic-tac-toe blocks without constraints and props near z -4) is open; owner's live gravity-gun drop not re-run by the owner.

**Next:** owner item 0 (gibs, NPC decals, constraints, carried-bottle crouch).
## 2026-10-13 session 14 (late): VPhysics contact margin, prop impact damage (`wip/cloud-physics-20261010`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents.

**Changed:** (1) Dynamic prop colliders get a 0.5-unit Rapier contact skin (`VPHYSICS_CONTACT_MARGIN`): the maps' authored rest poses put .phy hull bottoms 0.497 (PlasticCrate01a) and 0.500 (a lying glass bottle) units above the floor, i.e. IVP's collision margin; without it props sat half a unit into floors (owner screenshot). The player shadow is now the hull at its sides/top with the bottom raised by the margin (replaces the +0.25 oversize). (2) Prop impact damage (DESIGN 13e step 0): CPhysicsProp::VPhysicsCollision -> CalculatePhysicsImpactDamage with allowStaticDamage (both bodies' lost speed, the other weighted by its mass, per kg of the prop), prop m_impactEnergyScale (0/absent -> 0.1, AddOutput override), x10 on the first collision after a gravity-gun launch (`Physics::launched`, set by the launch, cleared by any collision); prop_physics without health stay unbreakable (events only). (3) `prop_pose` gains `launched`. (4) Break sound: CBreakableProp::Break -> PhysBreakSound plays the surfaceprop break cue at the prop (all surface break cues preloaded); packaged: pallet #327 into the wall plays Wood.Break at the break. (5) Gib lists: `phy::break_models` (CBreakParser keys/defaults), `World::model_breaks` and gib physics preloaded at map load (owned test: wood_crate001a lists 7 chunk gibs, all present); spawning/rendering gibs not done. (6) SF_PHYSPROP_PREVENT_PICKUP (0x200) blocks +USE and gravity-gun pickup (SDK FVPHYSICS_NO_PLAYER_PICKUP / CanBePickedUpByPhyscannon; unit test). Found: the d1_trainstation_02 playground tic-tac-toe blocks (playgroundTick-tack-toe_block01a, authored z 46-62) fall to z ~-4 and jitter forever: native holds them with physics constraints (phys_hinge); constraints are not implemented. Fixtures bevy-pallet-launched-into-wall, bevy-crate-drop-rest.

**Why:** owner: red plastic crate and bottles clip into the floor; crates/pallets must break like native (native, owner-verified: they break when launched into a close wall, not when launched far).

**Tested how:** unit tests (launch bonus/energy scale/small-mass cap; a prop launched into a wall queues >= 50), owned tests (all pass, frag replay unchanged), packaged: crate #715 dropped on the hall floor rests at z 71.921 vs the native authored 71.957 (71.421 before); box push rest (-3359.4,-2607.4) vs native (-3358.3,-2607.4), stuck only 2/16 samples while pushing the pinned box; standing on a box and walking into boxes 0 stuck; pallet #327 (physdamagescale 0.1) fired at 1500 u/s into the hall wall with the launch flag breaks on impact, without it survives (3 hits); the native-compared street launch of pallet #882 does not break (its map physdamagescale is 0.0001), rest y -3495 vs native -3572 (-3521 before the margin). Regression batch artifacts/s14b-regression: weapons 17/17, attention 26/26, movement 25/26 (doll01 only); finals vs s14-regression mean 0.00-0.48, Breen feeds identical.

**Result:** props rest on floors like native; launched props break against close walls.

**Still broken or not tested:** native comparison of the wall break (which pallet the owner broke: #882 has physdamagescale 0.0001 and cannot break by impacts per the SDK; #327/#349 have 0.1); a clean far-launch test of a breakable pallet (the hall run hit an obstacle); gibs, break sounds and propdata damage tables/modifiers (13e steps 1-4); bottle shoved into the floor with +USE not re-measured with a bottle; props near z -4 keep jittering (never sleep; possibly water) - not investigated; the margin is not applied to the thrown frag or prop-prop pairs are doubled (0.5 + 0.5, not verified vs native).

**Next:** 13e gibs/break sounds, then the remaining owner items and 13b, then the merge.
## 2026-10-13 session 14 (continued): NPC hull boxes, prop drag fractions, AddOutput keyvalues (`wip/cloud-physics-20261010`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents. Research input: Codex REA pass physics-research-20261013b/drag-axes (read-only).

**Changed:** (1) NPC hull boxes (DESIGN 13d): SOLID_BBOX NPCs of HULL_HUMAN classes (metropolice, combine soldiers, player companions, Kleiner/Eli/Breen/Mossman/Magnusson/G-Man/Monk, zombies, stalkers, assassins) get an axis-aligned (-13,-13,0)-(13,13,72) box collider at the entity origin (follows set_entity, never rotates, disabled with the entity); player movement and physics props collide with it, traces (bullets, use, projectiles) and NPC navigation do not; the model-shaped NPC colliders stay as trace hitboxes with no contacts. (2) Drag: the VPHY header's stored orthographic fractions (retail copies them) are read (`Phy::drag_fractions`, `PhysicsSolid::drag_fractions`) and used for map prop bodies instead of the convex-silhouette estimate; the thrown frag keeps the estimate. (3) `AddOutput "key value"` (SDK keyvalue form) sets a per-entity keyvalue override; impact damage reads `physdamagescale` from it first. (4) Test fixture action `prop_pose` (teleport a map prop_physics body with a velocity; stands in for prop_physics_create until DESIGN 14r) and report fields `physics_ms`. Fixtures: bevy-player-on-box, bevy-player-walk-into-boxes, bevy-player-on-npc-hull, bevy-pallet-on-npc-hull, bevy-pallet-kills-cop-physdamagescale1, bevy-frag-use-pickup.

**Why:** owner: in native you stand on a citizen's box-shaped collider above the shoulders; a pallet resting on a cop dropped the frame rate; the pallet launch drag was ~1.7x native (queued item 0); can we spawn a pallet like native; did the cop test set physdamagescale 1 (it had not: terminal_side_cop has map physdamagescale 0).

**Tested how:** unit tests (player lands on the hull past the model, traces hit only the model, hull follows/disables, a thrown box strikes the hull with an NPC contact event; AddOutput keyvalue form), owned tests (pallet .phy fractions 0.9417/0.4026/0.9936; frag replay unchanged: lob 27.6, roll 16.4, high 3.8, plaza 12.8, wall 44.1), packaged d1_trainstation_02 (artifacts/s14): standing over terminal_side_cop's shoulder holds feet at 137 (hull top) while shuffling (before: slid off); pallet resting on a cop: physics step 4.7 ms mean / 9.4 max (model colliders: 25.5 / 62 ms and the pallet slid off); pallet launch at the owner's street spot vs native T021822Z: y -4235/-3899/-3673/rest -3521 at 0.08/0.45/0.9 s (native -4258/-3904 at 0.47/-3711 at 0.91/rest -3572; before -4246/-3999/-3843/-3769); pallet launched at the cop at 1500 u/s: dies with AddOutput physdamagescale 1, unhurt with the map's 0. Workspace tests, strict Clippy, fmt. Regression batch artifacts/s14-regression: weapons 17/17, attention 26/26, movement 25/26 (only the known doll01 PHY root-rotation warning); finals vs artifacts/s13-regression mean differences 0.00-1.07, Breen feeds identical, Kleiner feed 6 pixels.

**Result:** standing on NPCs works like native boxes; the pallet-on-cop frame drop is gone; the pallet launch now tracks native within ~40 units (was ~200 short).

**Still broken or not tested:** other NPC hull types (scanners, antlions, striders, vortigaunts...) keep model colliders; crouch/small hulls; NPC-vs-NPC navigation still uses model shapes; native comparison of standing on an NPC not captured (SDK only). Drag: the retail IVP-frame axis pairing (Source (vx, vz, vy)) was tried and rejected for now (pallet barely changed, frag roll replay 16 -> 65); stored fractions broke the frag's plaza replay (12.8 -> 40.1), so the frag keeps its estimate: an unmodelled object/core frame difference remains (drag-axes CONTINUATION). AddOutput output-connection form unsupported. No real prop spawn yet (14r).

**Next:** regression review, then the remaining owner items (breakables/gibs, NPC studio decals), 13b leftovers and the merge.
## 2026-10-13 session 14: thrown frag +USE pickup; player stuck on/against props (`wip/cloud-physics-20261010`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents.

**Changed:** (1) Thrown frags get a projectile entity id (`physics::projectile_entity`) bound to their VPhysics body, so the +USE carry and the gravity gun can hold them (SDK CBaseGrenade: FCAP_IMPULSE_USE | FCAP_USE_IN_RADIUS, Use -> PickupObject; HL2 OnPhysGunPickup leaves the fuse running). FindUseEntity and the gravity gun's FindObject/punt traces use COLLISION_GROUP_NONE (`pick_ray`/`pick_hull` also hit weapon-group bodies); FindUseEntity now follows the SDK order (eye line returns at once, the last usable hull trace wins) and adds the PLAYER_USE_RADIUS sphere search (0.8 cone, nearest to the view line, occlusion trace). (2) Player stuck on props: `Player` ports CGameMovement::CheckStuck (stuck table, 0.2 s single-player interval, one offset per 0.05 s, move skipped while stuck) and exports `step_height` (m_outStepHeight). The VPhysics shadow: the explicit 85 kg weight impulse on movable ground is removed (not SDK/retail: the retail shadow has no gravity and light supports are not rideable; it sank a 5 kg box ~1.5 units into the floor and its rebound filled the player's hull); a step teleports the shadow (SetPosition/StepUp) instead of driving it through the prop; retail contact-plane limits (50 u/s into a pushable object, 0 into a heavy or blocked one), also applied ahead of contact by a sweep; the collider extends 0.25 past the hull's sides/top with its bottom raised 0.25. Report gains `hull_contacts`, `shadow` and `carrying`.

**Why:** owner playtest: a thrown grenade cannot be picked up with E (native can); the player often gets stuck walking into or standing on physics props. Owner corrections this session: test by standing on top of a prop; a box pinned at a wall should not move.

**Tested how:** unit tests (frag pickup until detonation; CheckStuck nudge; shadow pushes light not heavy, held object, no sinking of the prop stood on), owned tests with HL2_ROOT (44 pass), workspace tests (430), strict Clippy, fmt. Packaged d1_trainstation_02 runs (artifacts/s14): teleport onto cardboard box #710 and walk around (before: stuck on landing; after: 0/28 samples stuck, box unmoved); diagonal walk into boxes #710/#711 (before: stuck 230 ticks on #711; after 0/42); native-compared push into the wall (box rests (-3360.0,-2607.9), native (-3358.3,-2607.4), stays at the wall; momentary <0.01-unit contacts while pushing a pinned box); frag thrown at the feet then +USE: carried in front of the view until the fuse (capture fraguse-held.png).

**Result:** frag pickup works with +USE (and the gravity gun path is enabled, not separately tested); the stuck-on-top and walk-into-box cases no longer stick.

**Still broken or not tested:** native comparison of frag pickup (hold distance, timing) not done; gravity gun frag grab not tested; light props are no longer weighed down by the player (retail lead: controller support contacts, normal Z < -0.7); retail two/multi-normal controller constraints, bounded per-axis corrections and VPhysicsShadowUpdate (shadow pushing the player) not ported; props sinking into floors under load or when shoved with a held object (owner report: red crate, bottles) is Rapier contact softness, not fixed; NPC movement colliders are model-shaped where the SDK uses a HULL_HUMAN box (owner report, next).

**Next:** NPC hull boxes (SOLID_BBOX HULL_HUMAN for player movement and physics), then the pallet-on-cop frame drop, prop floor penetration, then the queued owner items.
## 2026-10-13 session 13 (continued): bug bait splat, player shadow, physdamagescale (`wip/cloud-physics-20261010`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents.

**Changed:** bug bait BeerSplash decal (retail subrect of decals/decals_lit, lightmap-lit translucent decals through `Impacts::add_decal`) and the SporeExplosion emitter; the player's 85 kg VPhysics shadow (pushes movable props <= 350 kg, weighs down movable ground, skips the held object; `Player::wish_velocity`); NPC impact damage scaled by the entity's `physdamagescale`. Fixtures `test-inputs/bevy-bugbait-wall.json`, `test-inputs/bevy-player-push.json`. Private oracle gained `!keep` (no setpos), `!forward/!unforward`, `!name` (targetname the prop under the crosshair), `!spawn` (prop_physics_create, ORACLE_SPAWN) and `!second` (ent_text on ORACLE_TEXT2).

**Why:** owner playtest reports (bug bait leaves no stain or particles; walking into props does not push them; a launched pallet barely hurt a cop).

**Tested how:** unit tests (spore schedule/direction/decal request, shadow push light vs heavy, held object, weight; physdamagescale), owned tests (decal textures incl. the translucent subrect), packaged captures vs native oracle: bug bait at the spawn-room wall (T012846Z), cardboard box push (T012535Z), crate/oil drum/pallet vs plaza cop (T015556Z, T015900Z, T020048Z). Workspace tests, strict Clippy, fmt.

**Result:** stain and push match native closely; the plaza cop's immunity is the map's physdamagescale 0, now honoured.

**Still broken or not tested:** spores dimmer/sparser than native (11f); our pallet launch flight vs native not compared (needs a spawn command, 14r); NPC studio decals; breakables; push speed limit and the shadow pushing the player.

**Next:** breakable props (propdata), NPC decals, a debug prop spawn for open-space launch tests, then the rest of 13b and the merge.

## 2026-10-13 session 13: frag as a real w_grenade.phy rigid body (`wip/cloud-physics-20261010`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents.

**Changed:** the thrown frag is a Rapier rigid body from w_grenade.phy by default (`--frag-swept-box` keeps the old fitted box): SDK spawn orientation (vec3_angle for throw/lob, (0, yaw, -90) for roll; ProjectileSpawn gained `angles`); the 12-sided .phy prism collides as a round cylinder; mass center and rotation inertia from the .phy IVP compact surface (source-assets phy.rs -> PhysicsSolid::compact_inertia); VPhysics performance-default speed limits (2000 u/s, 3600 deg/s) on every dynamic body. Owned test `owned_frag_throws_follow_native_samples` replays five native throws; unit test for the rolling prism; owned test for the frag .phy inertia; fixture `test-inputs/bevy-frag-wall.json`.

**Why:** owner report: the frag floated above surfaces (4-unit swept box vs the 1.8-unit hull), rolled artificially, flopped at walls instead of bouncing; the session-12 rigid body stopped dead because it spawned aligned with the throw (SDK spawns it upright) and the faceted hull lost energy on every edge.

**Tested how:** headless replay on d1_trainstation_02 collision vs native ent_text samples (best of seven spins, RMS units): lob 27.6, roll 16.4, overhand 3.8, plaza 12.8, wall 44.1 (old swept box 92.1/22.3/13.1/44.7/105.8). Three new native wall throws (oracle T000533Z, T002157Z, T002243Z). Packaged captures (artifacts/frag-s13): ground-level views of landed frags on the hall floor and on top of cardboard box #710 (rests at hull radius, no gap). Workspace tests, strict Clippy, fmt.

**Result:** frag lies on its side on surfaces and rolls with real rotation; wall impacts pop it up and deflect it.

**Still broken or not tested:** wall rebound for some spins is larger than the three native throws (in game, spin -0.5: lands ~90 units out vs native 18-60); IVP impact friction is not reproduced (retail friction product kept); packaged audio of the rigid frag vs native not rerun; two Bevy image `should_panic` tests fail in release builds only (debug assertions; pre-existing, unrelated).

**Next:** bug bait splat (BeerSplash decal + SporeExplosion), player physics shadow (push props), pallet launch flop, breakables, rest of 13b.

## 2026-10-12 session 12: local validation of the cloud arsenal + physics branches (`wip/cloud-physics-20261010`)

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents. Validates work written by the cloud sessions (Claude Code cloud, `claude-opus-5-5`, SDK-only: crossbow, RPG, bug bait, gravity gun, viewmodel effects, sound stop/patch path, view punch, authored VPhysics props, surfaceprops, impact sounds, frag rigid body, +USE carry, ground surfaces, per-side world materials).

**Changed (this session, local):** VFS searches VPKs before loose folders (gameinfo order); Weapon_Equip clip rule; retail facts in owned tests (no RPG reload sound, no crossbow unloaded activities); projectile models posed by sequence 0; viewmodel-attached sprites at world-FOV size; viewmodel FOV follows the zoom; RPG laser dot in glow mode (no depth test); crossbow skin-1 glowing bolt and CrossbowLoad sparks; per-object retail VPhysics drag (drag.rs) with native air density 2.00; held-object mass reduction and drag toggle; frag rigid body opt-in (`--frag-rigid-body`); fixtures bevy-physcannon-props.json and bevy-use-carry.json.

**Why:** owner: validate the cloud branches locally and fix what the installation, private research and native HL2 show; owner reports during the session (zoom past the viewmodel, laser dot clipping, plain vs incandescent bolt).

**Tested how:** Windows build, workspace unit tests, strict Clippy, fmt; owned tests with HL2_ROOT; packaged fixtures (frag x4, crossbow, RPG, bug bait, gravity gun at a prop spot, +USE carry; captures in artifacts/cloud-validate); native oracle (gravity gun hold/launch and air_density T191304Z, crossbow T192857Z); frag positions vs session-11 native samples; regression batch artifacts/cloud-regression(2).

**Result:** owned tests pass; gravity gun glow matches native; rigid-body frag worse than the swept box on lob/trainstation landings (kept opt-in); batch weapons 17/17, attention 26/26, movement 25/26 (new doll01 collision warning, explained in STATUS).

**Later in the session:** crossbow skin-1 glowing bolt + CrossbowLoad sparks, viewmodel FOV follows the zoom, RPG laser dot in glow mode, gravity gun DoEffectLaunch, physics impact damage to NPCs (SDK/retail NPC table; owner reports and requests). Native RPG missile path matches within ~3 units (T194927Z).

**Still broken or not tested:** native trajectories of bolt/bait/launched props (two rejected attempts each for bolt and launch); weapon audio vs native; DoEffectLaunch beam, bolt motion-blur trail, ragdoll pinning; 11f gamma sprites; doll01 root-rotated .phy; the AI branch is not validated yet.

**Next:** remaining 13b native checks, then fold into wip/arsenal-20261009 and the single 13b merge; then validate `wip/cloud-npc-ai-20261010` (step 14).

## 2026-10-12 cloud session: ground surface movement and per-side world materials (`wip/cloud-physics-20261010`)

**Agent/model:** Claude Code cloud session (Linux sandbox), `claude-opus-5-5` per the session environment. No subagents.

**Changed:** player ground friction/acceleration/max speed/jump from the ground surfaceprop (CategorizeGroundSurface); surfaceprops parses jumpfactor/maxspeedfactor/thickness/dampening; restitution clamped to 0..1; BSP brush-side materials -> $surfaceprop -> per-contact friction/restitution for world brush colliders through a Rapier contact-modification hook.

**Why:** owner: physics should use Source's materials (friction, bounciness) everywhere; the owner supplied the installed surfaceproperties scripts.

**Tested how:** unit tests with synthetic data (surface factors on a floor, icy side vs default slide distance, parser quirks); temporary uncommitted parser check against the owner's uploaded scripts (91 entries); strict Clippy, fmt, workspace tests.

**Result:** passes on Linux. Not run in the game (cloud: no owned files/native).

**Still broken or not tested:** native stopping distances and prop slides; brush entities/displacements keep "default"; slipperyslime jump question (STATUS).

**Next:** STATUS physics checklist items 7-9.

## 2026-10-10 second cloud session: VPhysics props, frag rigid body, +USE carry (`wip/cloud-physics-20261010`)

**Agent/model:** Claude Code cloud session (Linux sandbox), `claude-opus-5-5` per the session environment. No subagents.

**Changed:** new branch from the arsenal branch (owner request mid-session to separate the physics work). .phy solid parameters and surfaceproperties (source-assets) into `World`; Rapier prop bodies with authored pieces, mass, inertia, damping and surfaceprop friction/elasticity; air drag and collision events in `Physics::tick`; PhysCollisionSound/AddImpactSound/PlayImpactSounds for props and frags; the thrown frag as a Rapier body from w_grenade.phy with VPhysicsUpdate's NPC reflection; `grab.rs` shared CGrabController; `player_pickup.rs` +USE carry; host wiring (frag .phy load, collision sounds, E/use routing, holstered viewmodel); merge of the arsenal branch's 13b 3-5.

**Why:** owner, 2026-10-10: physics must follow Source/Havok (weights, bounciness, materials), the frag lacked a real physics object, and E should pick up small props.

**Tested how:** unit tests on synthetic data (solid parsing defaults, surfaceprop inheritance and hard/soft choice, authored mass/inertia/damping/material on bodies, drag per tick and contact events, frag body landing/sound/release, carry pickup/hold/throw/deny/drop, alignment); hl2-bevy compiled/tested on Linux; strict Clippy (exit code, color off), fmt.

**Result:** hl2-simulation 202 / 18 ignored, source-assets 92 / 19, modkit-core 53, hl2-ui 32 / 1, hl2-bevy 37 / 2.

**Still broken or not tested:** nothing compared with native or run packaged; the frag body is not fitted (the swept box was); player does not push props; see STATUS physics handoff.

**Next:** local validation per the physics checklist, decide frag body vs swept box, then merge after the arsenal branch.

## 2026-10-10 second cloud session: 13b items 3-5 (`wip/cloud-arsenal-20261010`)

**Agent/model:** Claude Code cloud session (Linux sandbox), `claude-opus-5-5` per the session environment. No subagents.

**Changed:** sound stop path and CSoundEnvelopeController patches for game cues (Missile.Ignite stop, gravity gun HoldSound); RPG lowered idle without rockets; gravity gun launch spin and carry max speed (new modkit-core `Input::max_speed`); ViewPunch/DecayPunchAngle for crossbow and gravity gun with the host camera; host drawing for stuck bolts, FX_ElectricSpark sparks and the SDK viewmodel sprites/beams (crossbow, RPG, gravity gun) on the viewmodel layer; viewmodel pose parameters ("active").

**Why:** owner priority 0, DESIGN 13b items (3)-(5) of the cloud prompt.

**Tested how:** unit tests (patch envelopes, missile stop request, motor patch sequence, lowered RPG, launch spin, carry speed, max-speed override, punch spring, sparks counts/expiry, viewmodel effect states and quads); hl2-bevy compiled and tested on Linux; strict Clippy (exit code, color off) and fmt.

**Result:** hl2-simulation 196 passed / 18 ignored, hl2-bevy 37/2, modkit-core 53, source-assets 90/18, hl2-ui 32/1. Commit cc82bb8 alone failed strict Clippy (doc lint), fixed in 615d630.

**Still broken or not tested:** nothing was run or compared with native (visual, audio, timing); viewmodel skin swap, CrossbowLoad effect and the launch beam are not done; punch does not affect aim/viewmodel; Windows build.

**Next:** local validation per the STATUS checklist items 7-12; merge order with `wip/cloud-physics-20261010` (stacked on this branch).

## 2026-10-10 cloud session: bug bait and gravity gun (13b d/e, `wip/cloud-arsenal-20261010`)

**Agent/model:** Claude Code cloud session (Linux sandbox), `claude-opus-5-5` per the session environment. No subagents.

**Changed:** weapon_bugbait / npc_grenade_bugbait / point_bugbait (`weapon_bugbait.rs`, BugBaitEvent hook for step 14) and weapon_physcannon (`weapon_physcannon.rs`: punt, pull, pickup, hold controller, drop, launch, deny) on the Rapier props; host hooks; fixtures test-inputs/bevy-bugbait.json, bevy-physcannon.json. Clippy fixes for the RPG commit.

**Why:** owner priority 0, DESIGN 13b (d) and (e).

**Tested how:** unit tests (bug bait haul/throw/squeeze and splat + sensors; launch spline, punt impulse, pickup/hold/launch, deny, pull on synthetic Rapier bodies); hl2-bevy compiled/tested on Linux; strict Clippy/fmt (verified with color off).

**Result:** builds; 190 hl2-simulation tests and 37 hl2-bevy tests pass on Linux.

**Still broken or not tested:** prop masses are not the authored ones; hold controller approximates VPhysics; everything vs native; Windows build. An earlier claim in this session that Clippy was clean (crossbow/RPG commit) was wrong; fixed in ded5dcc.

**Next:** local validation per the STATUS checklist; then step 14 (separate branch).

## 2026-10-10 cloud session: crossbow and RPG (13b b/c, `wip/cloud-arsenal-20261010`)

**Agent/model:** Claude Code cloud session (Linux sandbox), `claude-opus-5-5` per the session environment. No subagents.

**Changed:** weapon_crossbow + crossbow_bolt (`hl2-simulation/src/weapon_crossbow.rs`) and weapon_rpg + rpg_missile + env_laserdot (`weapon_rpg.rs`) from SDK 2013 weapon_crossbow.cpp / weapon_rpg.cpp; player FOV ramp (SetFOV/GetFOV); thin hooks in gameplay.rs/projectiles.rs; host wiring in hl2-bevy (models, cue preloads, FOV, impacts, suit updates, laser dot, missile events); fixtures test-inputs/bevy-crossbow.json, bevy-rpg.json. Details and SDK-vs-prompt differences in STATUS "Cloud session handoff".

**Why:** owner priority 0, DESIGN 13b (b) and (c), prepared in the cloud for local validation.

**Tested how:** unit tests only (state machines in 15 ms ticks, FOV spline, bolt flight/stick/reflect/NPC hit, missile ignition/homing/grace/blast, laser-dot line following); hl2-bevy compiled and its tests run on Linux; strict Clippy/fmt. Owned tests written but ignored (no game files).

**Result:** builds and unit tests pass on Linux.

**Still broken or not tested:** everything visual/audio/timing vs native; owned scripts/viewmodels; Windows build; stuck bolts/sparks/charger/beam not drawn; see STATUS.

**Next:** bug bait, gravity gun, then local validation per the checklist.

## 2026-10-10 session 11 (late): frag impact sounds, native fuse check

**Agent/model:** Claude Code desktop, `claude-opus-5-5` (session system context). No subagents.

**Changed (branch `wip/arsenal-20261009`, not merged):** frag impacts play the grenade surfaceprop's impact sound per SDK PhysCollisionSound/PlayImpactSounds: normal speed >= 70 u/s, >= 0.05 s since the previous collision, volume min(1, (speed/320)^2), `Grenade.ImpactHard` (installed surfaceproperties.txt: grenade impacthard/impactsoft, hardness 1.0 and no hard thresholds, so hard is always chosen), positioned at the grenade; the cue is preloaded.

**Why:** native physics objects sound on impact; ours were silent.

**Tested how:** unit test (floor drop: one sound, SDK volume, friction impulse, no bounce); 385 workspace tests, strict Clippy/fmt; packaged lob `--audio-trace`: request 0.24 s after spawn at the sidewalk landing, started physics/metal/metal_grenade_impact_hard2.wav. Native fuse (timescale 0.1, T043539Z): detonation 2.94-3.15 s after spawn vs ours 3.045 s. Rejected: oracle timescale 0.05 (user command buffer overflow drops +attack, two runs).

Hits on characters (npc_*) now use the SDK's VPhysicsUpdate ray reflection (0.2 restitution, spin x -0.5, no physics sound), since COLLISION_GROUP_WEAPON passes through characters in VPhysics; unit test added (386 tests).

Per-process recordings: native blip onsets match ours (0, 1.057, 2.102, 2.415, 2.740 s); blip peaks match within ~1 dB; the impact is masked by the plaza ambient (two attempts; tentatively ours ~3 dB quieter).

**Still broken or not tested:** impact sound level vs native (repeat at a quiet place); the 0.1 DMG_CRUSH "bonk" to NPCs (step 14); the player as a reflecting character; rotational dynamics; NPC reflection not compared with native.

**Next:** see the 13b list (crossbow, RPG, bug bait, gravity gun; optional frag rigid body; DESIGN 11f).

## 2026-10-10 session 11 (later): frag world contact, lob and roll vs native

**Agent/model:** Claude Code desktop, `claude-opus-5-5` (session system context). No subagents.

**Changed (branch `wip/arsenal-20261009`, not merged):** frag world contacts follow VPhysics instead of the entity-only 0.2 reflection (SDK CGrenadeFrag::VPhysicsUpdate reflects only off entities its collision group skips): surfaceprops grenade (friction 0.9, elasticity 0.01) x default/concrete/tile (0.8, 0.2) give practically no bounce and a Coulomb friction impulse 0.72 x |normal speed| on impact; resting contacts keep their tangential motion (gravity carries the body off edges instead of snagging), floors add a constant 190 u/s^2 rolling resistance fitted to native. Fixtures `test-inputs/bevy-frag-lob.json` (secondary, standing) and `bevy-frag-roll.json` (secondary while crouched); both look 25 degrees down at release like the native captures.

**Why:** owner priority 13b (lob/roll comparison; owner reminder: secondary fire standing and crouched).

**Tested how:** native HDR lob and roll (oracle '!attack2'/'!release2', roll with '!duck'; sessions view-d1_trainstation_02-20261010T040222Z lob, T040313Z roll; 14 ent_text samples each at timescale 0.1). Native spawn ~0.09 s after the release command (fit from the first sample; ours spawns at the owned event, 0.105 s). Packaged comparison by time since spawn: roll RMS 22.4 units over 2.2 s (end 21.2; curb drop impulse ~530 -> 335 u/s as native); lob flight/landing 4.7 and 0.5 units; overhand monument throw in flight 10.9 and 8.3 units. 384 workspace tests, strict Clippy/fmt.

**Still broken or not tested:** no rotational dynamics: native turns spin into rolling on contact and rolls along the grenade's own axis, so after landing the lob (ends 151 units off, native rolls straight to the curb at ~155 u/s) and the overhand throw (ends 70 units off; native rolls ~45 u/s off the monument ledge) differ. Swept 4-unit box instead of the w_grenade convex hull. Sprite gamma blending (DESIGN 11f). Regression batch (artifacts/arsenal-regression): 26/17/26, final captures unchanged vs damage-regression except known slate exposure and book-edge noise.

**Next:** rigid-body frag (Rapier body from w_grenade.phy with the fitted drag and surface friction) if the owner wants post-landing fidelity; otherwise crossbow, RPG, bug bait, gravity gun.

## 2026-10-10 session 11: frag flight drag, spin range, sprite-trail strip (arsenal WIP)

**Agent/model:** Claude Code desktop, `claude-opus-5-5` (session system context). No subagents.

**Changed (branch `wip/arsenal-20261009`, not merged):** npc_grenade_frag flight gets quadratic air drag dv/dt = -c|v|v, c = 6.4e-4 per unit, fitted to native (VPhysics drag lives in vphysics.dll, not the SDK; w_grenade.phy sets damping 0). The frag spin used a [-3, 1] random range (random() is already [-1, 1] and was remapped again); now RandomInt(-1200, 1200) as the SDK. The frag trail is one C_SpriteTrail/CBeamSegDraw strip (shared edge vertices, normal from neighbouring points, alpha and width 8->1 by each point's remaining life, u across the width, v = 0 because grenade_frag never sets a texture resolution) instead of separate per-segment quads with full alpha and a full texture per segment (owner report: wonky, less smooth than native). Fixtures `test-inputs/bevy-frag-native.json` (plaza, yaw 12, Bevy pitch 10) and `bevy-frag-high.json` (yaw -20, Bevy pitch 30). The private oracle gained per-view weapon input suffixes (`!attack`, `!release`, `!attack2`, `!release2`, `!duck`, `!unduck`, `!text` = ent_text on ORACLE_TEXT; positions need `developer 1`).

**Why:** owner priority 13b (native frag comparison) and owner report on the trail.

**Tested how:** native HDR captures (oracle sessions view-d1_trainstation_02-20261010T013831Z, T014922Z, T015224Z; ent_text positions at host_timescale 0.25/0.1). Drag fit: 8 in-flight samples, RMS 3.7 units (linear drag 10.3, none 195.6); the independent pitch -10/yaw 12 throw is predicted within 3 units. Packaged run after the change: 7-20 units from native over 1.3 s (before: up to ~370). Rest position after the monument bounce: ours (-2102.6,-1747.6,20.0) vs native (-2098.9,-1747.5,18.1). New unit tests (native drag samples, trail strip); 384 workspace tests, strict Clippy/fmt.

**Result:** frag flight matches native closely; the trail is smooth and fades like native.

**Still broken or not tested:** native draws Sprite-shader materials with `$nosrgb` 1 (SDK sprite_dx9.cpp default: no sRGB read/write, TONEMAP_SCALE_GAMMA), i.e. additive in gamma space into the integer-HDR framebuffer; ours adds in linear space, so the trail core is about half as bright (red excess ~50 vs ~100) and looks thinner. CBeamSegDraw geometry was checked against the SDK's compiled tier2.lib (half width per side, vertex colour = rgb + alpha). Gamma-space sprite blending is a renderer item for all sprites (planned, not started). Explosion timing vs native not measured (not in view); lob/roll not compared; regression batch not run; the remaining four weapons.

**Next:** lob/roll native comparison, then crossbow, RPG, bug bait, gravity gun; gamma-space sprite blending as a separate renderer step (DESIGN 11f).
## 2026-10-12 cloud session: runtime entity creation steps 1-3 started (`wip/cloud-npc-ai-20261010`)

**Agent/model:** Claude Code cloud session (Linux sandbox), `claude-opus-5-5` per the session environment. No subagents.

**Changed:** `Scene::initial_state` factored out of Scene::new; `Scene::spawned` / `spawn_entity` / `entity` / `ent` (spawned entities leaked `&'static`); 23 runtime lookups in entities.rs plus AI speech and eye tracking safe for spawned ids; `AiRuntime::add_actor`.

**Why:** owner decision: spawn like Source (DESIGN 14r) instead of the load-time pool.

**Tested how:** unit tests (spawned entity state/lookup, 40 scene+physics ticks with a spawned cop and a Kill input, runtime-spawned cop gets an AI actor and thinks); workspace tests; strict Clippy (ca1a40b failed it: the command chain did not gate on Clippy; fixed next commit); fmt.

**Result:** passes on Linux; the host does not call spawn_entity yet (F4 still uses the pool).

**Still broken or not tested:** steps 3 (controller, colliders), 4 (async late precache), 5 (Bevy instancing), 6 (remove pool).

**Next:** DESIGN 14r step 3 remainder.

## 2026-10-12 cloud session: runtime entity creation plan (`wip/cloud-npc-ai-20261010`)

**Agent/model:** Claude Code cloud session (Linux sandbox), `claude-opus-5-5` per the session environment. No subagents.

**Changed:** AGENTS.md asset-read rule clarified (no blocking reads in frame systems; runtime loads via asynchronous workers; entities may be created at runtime); DESIGN "14r" plan for runtime entity creation with late precache.

**Why:** owner: spawning should work like HL2/Garry's Mod (Source's CC_NPC_Create creates entities and late-precaches at runtime).

**Tested how:** documentation only (no code change).

**Result:** plan recorded; the F4 pool stays until 14r steps land.

**Still broken or not tested:** everything in 14r.

**Next:** 14r step 1.

## 2026-10-12 cloud session: F4 roster, scrolling list and AI debug switches (`wip/cloud-npc-ai-20261010`)

**Agent/model:** Claude Code cloud session (Linux sandbox), `claude-opus-5-5` per the session environment. No subagents.

**Changed:** 41 HL2 NPC classes with SDK default models in the F4 pool; scrolling class list with weapon choices; F4 debug keys I (AI on/off, stopping NPCs to idle), O (ignore the player), P (stance toward the player).

**Why:** owner: missing NPCs, a scrolling list, and debug switches to watch NPCs without the player or the AI interfering.

**Tested how:** unit tests (pool covers every class, list scrolls, ignored player is not shoved, forced hate makes the player the enemy, AI off clears schedules); strict Clippy; fmt; workspace tests.

**Result:** passes on Linux. Not run in the game (cloud: no owned files/native).

**Still broken or not tested:** visuals, load time of ~30 extra models, AI only for metrocops, no console equivalents.

**Next:** STATUS AI checklist item 6.

## 2026-10-12 cloud session: F4 spawns real NPCs (`wip/cloud-npc-ai-20261010`)

**Agent/model:** Claude Code cloud session (Linux sandbox), `claude-opus-5-5` per the session environment. No subagents.

**Changed:** load-time hidden spawn pool (3 per F4 class, SDK default models); F4 Enter / npc_create activate the next slot at the CC_NPC_Create placement; cop baton from the equipment; overlay shows the last result.

**Why:** owner: F4 placed nothing ("why does F4 still not work").

**Tested how:** unit tests (pool slots, hidden pooled cop does not think, placed cop thinks and warns); strict Clippy; fmt; workspace tests.

**Result:** passes on Linux. Not run in the game (cloud: no owned files/native).

**Still broken or not tested:** visual placement, weapon models in hands, AI for non-cop classes, names, load-time cost of the extra models.

**Next:** STATUS AI checklist item 6.

## 2026-10-10 cloud session: AI host wiring, npc_metropolice, F4 overlay (`wip/cloud-npc-ai-20261010`)

**Agent/model:** Claude Code cloud session (Linux sandbox), `claude-opus-5-5` per the session environment. No subagents.

**Changed:** `ai::host` (SceneMotor, PhysicsSight, PhysicsPlacement), `ai::runtime` (NPCThink loop, off by default, yields to scripts/scenes), `ai::metropolice` (npc_metropolice.cpp precriminal behavior: shove, warnings, baton, stunstick, AdministerJustice, PrecriminalUse, police goal via SetPoliceGoal), scene AI hooks (activity, yaw, anim events, inputs), npc_metropolice as AI-only ground actor; hl2-bevy AI tick in the NPC stage, player hits (damage/impulse/fade), `--ai` / console `ai_enable`, metrocop sentences, +USE on cops, F4 spawn overlay and `npc_create*` routing (placement logged only).

**Why:** owner step 14: trainstation metrocops through their real AI logic; F4 developer spawns.

**Tested how:** unit tests with synthetic worlds (motor yaw, sight, placement, think loop off/on, scripted yield, SetPoliceGoal warnings and outputs, +USE warnings, console routing); workspace `cargo test --locked`; strict Clippy (exit 0); fmt.

**Result:** passes on Linux. Not run in the game: no owned files or native HL2 in the cloud.

**Still broken or not tested:** everything visual/audio/timing (not tested (cloud: no owned files/native)); runtime NPC creation (plan in DESIGN 14); citizens/Combine; view punch on this branch; thrown-object hook; gameplay sound insertion into CSoundEnt; think throttling.

**Next:** STATUS AI LOCAL VALIDATION CHECKLIST (plaza comparison with AI on, regression batch with AI off).

## 2026-10-10 cloud session: policing behavior (`wip/cloud-npc-ai-20261010`)

**Agent/model:** Claude Code cloud session (Linux sandbox), `claude-opus-5-5` per the session environment. No subagents.

**Changed:** `ai::police` (ai_goal_police + CAI_PolicingBehavior from SDK 2013); Behavior::select_schedule takes the NPC mutably; TASK_WAIT_RANDOM; face tasks fall back to the remembered enemy position.

**Why:** owner: trainstation metrocops must hold posts and escalate through the real logic (step 14).

**Tested how:** unit tests (warning cadence/outputs/baton/hostility, suppress on crossing the radius, knock-out plane, return to post); strict Clippy/fmt; hl2-bevy cargo check.

**Result:** passes on Linux; not wired into the game.

**Still broken or not tested:** the npc_metropolice class, host wiring, sentences/gestures in game, native comparison.

**Next:** STATUS handoff checklist step 3.

## 2026-10-10 cloud session: AI schedules, senses, squads, spawn model (`wip/cloud-npc-ai-20261010`)

**Agent/model:** Claude Code cloud session (Linux sandbox), `claude-opus-5-5` per the session environment. No subagents.

**Changed:** `ai::schedule` (+ generated default schedule table), `ai::senses`, `ai::squad`, `ai::spawn` (npc_create commands, F4 overlay model).

**Why:** step 14 core (owner priority 1).

**Tested how:** unit tests (13 ai:: tests), strict Clippy/fmt on Linux.

**Result:** builds/passes on Linux; no runtime change (not wired).

**Still broken or not tested:** host adapters, per-class NPCs, F4 drawing; everything vs native.

**Next:** see STATUS checklist step 3.

## 2026-10-10 cloud session: step 14 AI core start (`wip/cloud-npc-ai-20261010`)

**Agent/model:** Claude Code cloud session (Linux sandbox), `claude-opus-5-5` per the session environment. No subagents.

**Changed:** DESIGN step 14 plan; `hl2-simulation/src/ai/` conditions, NPC state selection, enemy memory, relationship table (SDK 2013).

**Why:** owner priority 1 (step 14), prepared in the cloud.

**Tested how:** unit tests; strict Clippy/fmt on Linux.

**Result:** builds and passes on Linux; not wired into the game.

**Still broken or not tested:** everything behavioral; see STATUS handoff.

**Next:** schedules/tasks, senses, squads, F4/npc_create data model, metropolice.

## 2026-10-09 session 10 (later): arsenal WIP (frag grenade), suit logon

**Agent/model:** Claude Code desktop, `claude-opus-5-5` (session system context). No subagents.

**Changed (branch `wip/arsenal-20261009`, not merged):** DESIGN 13b plan (all five weapons, one merge). weapon_frag per SDK CWeaponFrag with the owned v_grenade.mdl events (owned test asserts the sequences/events); npc_grenade_frag fuse/blips/explosion on Source think ticks; projectile models render per model (SMG grenade, w_grenade); frag glow + trail at the fuse attachment (additive by entity render mode); clipless weapons give default ammo as reserve (GiveDefaultAmmo); item_suit plays !HEV_AAx (CItemSuit::MyTouch).

**Why:** owner priority: the rest of the arsenal as one feature after player damage.

**Tested how:** unit tests (launch math, fuse schedule), owned test (v_grenade events), workspace tests, strict Clippy/fmt; packaged trainstation_02 throw (pullback, release, throw at the event, AMMO 5 -> 4, blips, explosion at +3.045 s, redraw) and trainstation_06 suit pickup (!HEV_AAx). No native comparison yet.

**Still broken or not tested:** native frag comparison; lob/roll packaged runs; the frag as a real physics body (friction/rolling approximated); grenade punt/pickup (gravity gun); crossbow, RPG, bug bait, gravity gun.

**Next:** finish 13b on the same branch (native frag check, then crossbow, RPG, bug bait, gravity gun), regression batch, single merge.

## 2026-10-09 session 10: player damage path, gordon_invulnerable, fades, damage indicator, HEV voice

**Agent/model:** Claude Code desktop, `claude-opus-5-5` (session system context). No subagents.

**Changed:**
- One SDK-ordered player damage intake, `hl2_simulation::player_damage` (CHL2_Player::OnTakeDamage skill scale with the DROWN/CRUSH/FALL/POISON/SNIPER exemption, CBasePlayer::OnTakeDamage armor with integer m_ArmorValue, the fractional damage accumulator, DamageEffect, suit diagnosis, CHL2_Player::UpdateClientData "Damage" message). trigger_hurt, the player's grenade blasts and falls all use it; `Inventory::damage_player` is removed.
- env_global and the global table (`hl2_simulation::globals`; SF_GLOBAL_SET only when absent, TurnOn/Off/Remove/Toggle/counters), carried on changelevel. While `gordon_invulnerable` is on (owned d1_trainstation_01-03 and the 04 knockout) the player takes no damage, as the owner reported for native.
- trigger_hurt hurts every toucher that passes its filters (spawnflags), fires OnHurt for NPCs and OnHurtPlayer for the player, counts both for doubling/forgiveness; NPC damage goes through the shared entity damage path; dead players are not hurt.
- Screen fades (`hl2_ui::fades`, CViewEffects): DamageEffect fades, env_fade inputs and the fatal-landing black fade (PlayerFallingDamage), drawn under the HUD on the game clock.
- HUD damage indicator: the HudDamageIndicator panel with the owned HudTakeDamage*/HudPlayerDeath sequences, SDK MsgFunc_Damage selection (angle bins, DAMAGE_HIGH, damage bits), white_additive trapezoids and fullscreen flash. HudPlayerDeath now comes from the Damage message (CHudHealth never started it).
- HEV suit voice: `hl2_simulation::suit` (SetSuitUpdate/CheckSuitUpdate), `source_assets::vox` (sentences.txt and retail engine.dll VOX word rules, 100bb370/100bc9d0/100bb4b0), sentence synthesis from preloaded words in audio.rs; suitvolume 0.25; HEV_DEAD at death; the queue clears at death (Event_Killed). Nothing without the suit; DeathSound picks FallGib from the accumulated DMG_FALL bit.
- Fixtures: `test-inputs/bevy-player-death.json` now drops on d1_trainstation_04 (run with `--map d1_trainstation_04`; trainstation_02 no longer allows damage); new `test-inputs/bevy-player-hurt.json` (suit, trigger_hurt *32).
- DESIGN: 13a-2 plan, owner corrections, 13b as one feature, step 14 NPC AI with an F4 developer NPC spawn overlay (native F5/F6/F9 binds avoided).

**Why:** DESIGN 13a remainder (owner priority: player features first), and owner corrections: no damage in d1_trainstation_01-03 (gordon_invulnerable), HEV sounds only with the suit.

**Tested how:**
- Unit tests: damage order/armor/accumulator/skill exemptions, Damage message, DamageEffect and diagnosis, globals (trainstation sequence, carry, counters), trigger_hurt NPC touchers and filters, fade math (CViewEffects), indicator angle bins and sequences, suit queue spacing/no-repeat/no-suit, VOX tokenizer and parameter scope. Owned: 60+ HEV sentences render within 5 ms of their authored {Len} (HEV_MED0's Len is stale). 380 workspace tests, strict Clippy, fmt.
- Packaged: trainstation_02 drops leave health 100 (globals on); d1_trainstation_04 trigger_hurt *32 kills at four street spots; suit run plays !HEV_DMG5 at 0.15 s, Player.FallGib and HEV_DEAD1 at death (audio trace); no-suit fatal fall plays Player.FallDamage + FallGib only and respawns with 100 health.
- Native HDR comparison (oracle, `ORACLE_SETUP="noclip;cl_drawhud 1"`): trigger_hurt death at (-5608,-4783) after 6 s: native mean (255,64,50) / ours (255,60,47), red saturated on 100% of pixels in both; fatal landing at (-4584,-4163): native uniform (253,0,0) / ours (255,0,0); both engines land on the same player clip at z 640 at (-5608,-4783).
- Batch artifacts/damage-regression: 26/17/26; Breen feeds pixel-identical; the weapons view's HEALTH now reads 100 (trainstation_02 refuses the own-grenade damage that used to leave 16.4); attention book-edge differences match a rerun's physics noise.

**Still broken or not tested:** drowning (needs water level/swimming), knockback/punch angles, time-based damage, explosion ear ringing, NPC health from sk_*_health, parented triggers (canals_01 trains), modulate fade blend (inferred), VOX word pitch vs channel pitch and time compression (retail mixer unread), the friendlies-talking suit volume cut; timing of the indicator/death ramps vs native; native HEV audio levels; the 2-level (253 vs 255) death red.

**Next:** merge, then DESIGN 13b (the rest of the arsenal in one feature).

## 2026-10-08 session 9 (end): player fall damage, death and respawn; detail sprite WIP

**Agent/model:** Claude Code desktop, `claude-opus-5-5` (session system context). No subagents.

**Changed:**
- Owner priority change: player features first, then the rest of the arsenal (DESIGN 13).
- Player (DESIGN 13a, SDK CSingleplayRules::FlPlayerFallDamage, CBasePlayer::Event_Killed/DeathSound/PlayerDeathThink, respawn): HL2 fall damage from 526.5 to 922.5 u/s (armor ignored) with Player.FallDamage; death holsters the weapon, sets the dead view height (14), freezes input, plays Player.Death or Player.FallGib; after every button is released, one press restarts the map (singleplayer reload without saves). `Inventory::damage_player`, shared `Player::dead`/`landed`. Death cues are preloaded. Fixture `test-inputs/bevy-player-death.json`.
- trigger_hurt (SDK CTriggerHurt, player only): immediate hit on touch, damage x 0.5 every 0.5 s, damagemodel 1 doubling to damagecap with 3 s forgiveness, leave damage when the last think missed, negative damage heals, OnHurtPlayer; DMG_FALL/DROWN/POISON/RADIATION bypass armor. Unit-tested; batch artifacts/hurt-regression 26/17/26. Maps: trainstation_01 turret_hurt_1 (start disabled), trainstation_04 one, canals_01 two; not exercised in a packaged run.
- Detail sprites (DESIGN 11c, branch `wip/detail-sprites-20261008`, not merged): `source_assets::details` decoder and a per-view billboard renderer (vertex shader facing and squared-distance fade).

**Why:** Owner request to finish player features (death etc.) before more renderer work; detail sprites were the queued item 2.

**Tested how:**
- Unit tests: fall-damage curve and armor bypass, landing speed and frozen dead input, detail decoder (synthetic and owned: 7 sprites, 3,622 records, 66 leaves), fade/corner/lighting math. Workspace tests, strict Clippy/fmt.
- Packaged death run: fatal drop -> pl_fallpain1 + body_medium_break2 (FallGib), eye 14, weapon holstered, respawn at the map start with 100 health after a press.
- Packaged park-grass capture: 3,622 sprites in 66 leaf meshes render bottom-anchored and facing the camera (not compared with native yet).
- Batch artifacts/death-regression: movement 26, weapons 17, attention 26; Breen feed pixel-identical to the footsteps baseline.

**Still broken or not tested:** native death-view comparison (oracle `hurtme` had no effect twice; rejected); trigger_hurt, skill scaling, damage fades/indicator, pain/HEV sentences, drowning; the death view's exact native interpolation/tint; detail sprites vs native, flicker bursts and per-leaf sort order.

**Next:** DESIGN 13a remainder, then 13b arsenal (frag, crossbow, RPG, gravity gun, bug bait).

## 2026-10-08 session 9 (later): crowbar surface sounds, footsteps, script cue levels

**Agent/model:** Claude Code desktop, `claude-opus-5-5` (session system context). No subagents.

**Changed:**
- Crowbar (SDK basebludgeonweapon/weapon_crowbar/fx_hl2_impacts): a player melee hit plays the hit surface's `bulletimpact` sound like bullets; the player crowbar no longer plays melee_hit/melee_hit_world (NPC-operator cues). NPC hits from any weapon play the flesh impact sound. Fixture `test-inputs/bevy-crowbar-surfaces.json`.
- Footsteps (`hl2_simulation::footsteps`, SDK UpdateStepSound/PlayStepSound, HL2 jump/landing constants): step timer 400/300 ms (+100 crouched), walk/run thresholds 90/220 (60/80 crouched), material volumes (0.2/0.5, dirt 0.25/0.55, vent 0.4/0.7, x0.65 crouched), alternating stepleft/stepright, jump step 1.0, landing 0.85/1.0 above 303 u/s. Ground surface from `Impacts::ground_property` (shared receiver lookup). The shared Player reports `jumped`/`landed`. Fixture `test-inputs/bevy-footsteps.json`.
- Game cues use the sound script's volume and pitch drawn per emission (separate random stream) instead of a fixed 0.4/100; impact sounds are positioned at the hit with the script soundlevel. `SoundRequest` gained `volume` and `origin`.

**Why:** Owner reports: no player footsteps; the crowbar sounded the same on every hit. Script levels are the DESIGN 12b follow-up.

**Tested how:**
- 365 normal tests, owned tests, strict Clippy/fmt.
- Packaged crowbar run: Concrete/Tile/Concrete BulletImpact by floor with rotating variants; open-air swing plays only the swing.
- Packaged footstep run: tile walk 400 ms at 0.2, concrete sprint 300 ms at 0.5, crouch 500 ms at 0.13, jump 1.0; normal jump landing silent.
- Batches artifacts/melee-regression, footsteps-regression, cues-regression: 26/17/26. The first cue build changed Breen's monitor lip sync (0.16% of feed pixels) because parameter draws shared the wave-selection generator; with a separate stream two reruns are pixel-identical to the previous batch.

**Result:** Crowbar hits sound by surface, the player has footsteps, and cue levels follow the scripts.

**Still broken or not tested:** native comparison of footstep and weapon levels is inconclusive (low correlation, likely native DSP/pitch; private footsteps-melee-20261008, cue-levels-20261008); how native spatializes the local player's own sounds; NPC speech stays unpositioned; ladders/water steps; model-level surfaceprops for props.

**Next:** detail sprites (DESIGN 11c, branch wip/detail-sprites-20261008).

## 2026-10-08 session 9: barrier close-hum loop, env_soundscape backgrounds

**Agent/model:** Claude Code desktop, `claude-opus-5-5` (session system context). No subagents.

**Changed:**
- ambient_generic PlaySound/StopSound/ToggleSound follow SDK CAmbientGeneric m_fActive and restart/stop that entity's own sound with its parameters (`AmbientControl`); `--audio-trace` request lines name the control. Fixture `test-inputs/bevy-barrier-hum.json`.
- New `hl2_simulation::soundscapes`: soundscape manifest/map scripts, SDK server selection for env_soundscape/proxy (Scene::update_soundscape after player movement; Enable/Disable/ToggleEnabled; OnPlay), SDK client playback commands. The Bevy host preloads reachable waves, plays ambient and positional loops and random one-shots, fades by scene time (pause freezes it), honours `--mute-ambient`, traces commands and reports `audio.soundscape`. Fixture `test-inputs/bevy-soundscapes.json`.
- docs/research.md: LuckerParty specs as unverified pointers; parallel REA workflow. DESIGN 12d/12e plans.

**Why:** Owner reports: the barrier's close hum played once instead of looping; the background should change between the spawn room, the station hall and outside.

**Tested how:**
- `cargo test --locked --workspace` 360 passed; owned soundscape test (trainstation scripts and waves) passed; strict all-target Clippy and fmt.
- Packaged barrier walk-in/out with `--audio-trace` and per-process recording: loop continuous while inside (7.8 s and 6.7 s), stops on leaving, twice.
- Packaged soundscape tour (fly and walking): Interrogation -> Turnstyle -> TerminalSquare -> Turnstyle, with crossfades and random city sounds.
- Native per-process recordings at the same places (oracle, HDR, `snd_mute_losefocus 0`): background medians within 3 dB at spawn and hall; outside ours is 6.6 dB louder.
- Regression batches artifacts/ambient-inputs-regression and artifacts/soundscapes-regression: movement 26, weapons 17, attention 26; images match the previous batch (Breen slate exposure varies run to run, 0.64-0.68).

**Result:** The close hum loops while touching the barrier. Each area has its own background, switching with the original crossfade.

**Still broken or not tested:** outside background level (+6.6 dB vs native, cause unverified); DSP room effects, stereo panning, the close hum's spin-up/fade presets, trigger_soundscape, soundscape carry-over across level transitions; owner listening check. Rejected: console soundscape debug through the oracle hung native twice.

**Next:** crowbar surface sounds (branch wip/melee-footsteps-20261008) and footsteps (DESIGN 12e).

## 2026-10-08 session 8 (later): positional ambients, security fixtures, monitor brightness investigation, PR #2 review

**Agent/model:** Claude Code desktop, `claude-opus-5-5`. No subagents.

**Changed:**
- `ambient_generic` playback per SDK CAmbientGeneric. Distance gain from the retail engine's `GetDistGainFromSoundLevel` (engine.dll 10216bd0 -> 10216a20, disassembled privately; stock convars snd_refdb 60, refdist 36, gain_min 0.01, foliage loss 4).
- Sound script volume/pitch/soundlevel parsing in `Library::params`.
- Report `audio.spatial`.
- Seeded security fixtures (`bevy-attention*`, `bevy-monitors-kleiner-scene`) now enable `overlay_kleinertv` (security_01 OnTrigger2) and kill `scene1_start`, `scene2_start` and `intro_music`. The G-man intro had started six seconds in.
- Resampler test WAV header uses escapes, not raw control bytes.
- Monitor feeds: kept-exposure and fixed-2 experiments were reverted; scale 1 remains.

**Why:** Owner reports: loud hum everywhere, shield hum should fade with distance, the blue HUD was missing in the Barney batch cases, G-man speech played during those cases, and the Kleiner screen looked too dim.

**Tested how:**
- 349 normal tests, strict Clippy/fmt.
- Packaged reports at five plaza distances (gate hum 1.0/0.85/0.30/0.14/0.08 at 14/132/332/632/988 units).
- Audio trace of the attention fixture: the G-man cues are gone.
- Attention verifier 26/26 with the overlay visible.
- Native HDR security scene at 10.5 s from the Kleiner camera, plus native lab views after the intro.
- Regression batch artifacts/session8-audio.

**Result:** The shield hum is positional. The HUD overlay renders in the Barney cases, and those runs no longer play the G-man intro.

**Still broken or not tested:**
- The Kleiner screen and plaza slate are ~2x dim (linear) vs native. A feed-scale fix regressed hall exposure (wall 109 vs 129) and was reverted.
- Bumped lightmaps (rocky brick).
- env_soundscape, panning/DSP, positional PlaySound inputs, script cue volume for game requests.
- Native audio recording of the hum.
- Rejected this session: native lab captures blocked by the G-man intro camera (wait out the intro with ORACLE_INTRO_SECONDS=55 plus intro deactivation), and the `kept`/`fixed 2` monitor scales.

**Next:** env_soundscape (DESIGN 12d), then detail sprites (11c).

## 2026-10-08 session 8: weapon-selection tick, godray regression, pause freeze, shield mipmaps

**Agent/model:** Claude Code desktop, `claude-opus-5-5` (session system context). No subagents.

**Changed:**
- Audio (fixes the owner's weapon-switch report): every preloaded wave is resampled once to 48 kHz 16-bit (Catmull-Rom) in `hl2-bevy/src/audio.rs`. Stereo one-shots get one 512-sample silent span first. rodio 0.22 starts each player with a mono 48 kHz, 512-sample filler span, so a sound in another format had its first 512 samples played as 48 kHz mono. The 22 kHz `wpn_moveselect.wav` tick lost its attack: about 12 ms missing, then 2.2x too fast. New diagnostics: `--audio-trace JSONL` (per-sound request/start/removal with last sink position) and `--mute-ambient` (skip map ambient loops for isolated recordings). Shared `hl2_simulation::sounds::wav_samples`.
- Godrays: UnlitTwoTexture tint is linear($color x entity rendercolor) again (SDK `SetModulationPixelShaderDynamicState_LinearColorSpace`). Session 7's barrier path had overwritten it with $color alone, which whited out the trainstation vol_light shafts (rendercolor ~49 45 34).
- Pause: auto exposure and bloom adaptation freeze while paused (owner: native freezes everything). `--capture-live` keeps simulating during a capture burst, to verify motion over time.
- Plaza shields: `source_assets::vtf::decode_frame_mips` and mip-chain upload for UnlitTwoTexture, so far shields show the authored smudged-plasma mips instead of sharp cracks.

**Why:** Owner reports 2026-10-08 (tick cutoff, godrays, pause, far shield look).

**Tested how:**
- Tick: process-tree loopback recordings (private Microsoft ApplicationLoopback build) of the packaged game with ambient muted, plus a one-off local recording of the historical Macroquad host. Burst energies relative to the attack:
  - file: -10.4/-10.4/-7.4/-10.4/-10.4 dB
  - Macroquad: -10.3/-10.2/-7.4/-9.9/-10.3 dB
  - Bevy before: -7.4/-7.3/-4.4/-7.3/-7.4 dB
  - Bevy after: -10.5/-10.4/-7.5/-10.4/-10.5 dB
  Attack windows after the fix: correlation 0.993-0.998, gain 0.99-1.01. The owner confirmed by ear.
- Barrier scroll: matched native HDR frame series by normalised cross-correlation (0.73-0.83 vs 0.43 median), and the times advance like native's (~2 s steps, 14.1 s period).
- Pause: Escape pause menu, 150 live frames pixel-identical.
- Godrays: native HDR hall views vs before/after.
- Shields: native HDR at 600-1000 units.
- `cargo test --locked --workspace`: 347 passed; owned ignored tests: 33 passed; strict all-target Clippy and fmt pass.
- Packaged batch artifacts/session8-final: verifiers movement 26, weapons 17, attention 26. The Breen hall view's mean luma is 91 (session 6: 90.9; session 7: 170, the godray regression).

**Result:** The selection tick plays like the original file. Godrays match native again. Pause freezes exposure. Shield scrolling was already native-equivalent; its far look now matches.

**Still broken or not tested:**
- Map ambients are still non-positional and play everywhere: no Source distance attenuation, no `env_soundscape`. This is the owner's loud-hum report and the barrier hum not fading.
- Native HL2 audio was not recorded for the tick; Macroquad and the file are the references.
- Stereo one-shots start 5.3 ms late.
- Combine wall panels are olive vs native blue-steel (likely envmap/bump). The green light sprite and the plaza screen's blue glow are missing.
- Rejected this session: the ambient-masking hypothesis (Macroquad had the same ambients), a biased 10 ms high-pass envelope comparison, and a `setpause` native capture whose screenshots were taken outside the pause.

**Next:** Source ambient_generic attenuation (retail engine `GetDistGainFromSoundLevel` 10216bd0/10216a20, recorded privately) and soundscapes. Then detail sprites (DESIGN 11c) and the remaining owner-order items.

## 2026-10-08 plaza barrier materials and monitor proxy motion

**Agent/model:** Codex Desktop, `gpt-6.1-sol` (verified session metadata); materially involved research/implementation subagents inherited this model.

**Changed:** Bounded 2D VTF frame decoding, ordered PlayerProximity/Subtract/Clamp frame proxies, independent brush material instances and UnlitTwoTexture texture multiplication/scrolling. Monitor proxies now evaluate literal LinearRamp and translation-only TextureTransform in authored order. The seeded Kleiner fixture enables its separate blue Combine HUD brush. AGENTS requires model attribution; historical entries without individual evidence say "not recorded".

**Why:** Native plaza shields select darker authored frames with distance from the local player's center; they are not permanently bright panes. Kleiner's scanline material uses a negative Y ramp to scroll top to bottom. The blue graphics come from `overlay_kleinertv`, enabled by security_01, not the camera shader.

**Tested how:** SDK b8cfb12 first, exact read-only pass14 retail retrievals and owned VMT/VTF/BSP checks (private barriers-20261008, monitor-scroll-20261008 and weapon-switch-audio-20261008). `cargo test --locked --workspace`: 344 passed; owned `cargo test --locked --workspace -- --ignored`: 33 passed; strict all-target Clippy and fmt passed. `scripts/build-bevy.ps1`: SHA256 `6E6C7FE4ED4C5988553739F37F596F33CB632F8765937AEAFC3B0B4D0330A3B2`, built 2026-10-08T01:39:14.6464456Z. Packaged regression batch + movement26, weapons17, attention26 assertions passed. Weapons burst: 0/121 missing with flicker_check.py. Barrier near/mid/far two phases and 61-frame bursts; monitor four clock samples and 31-frame bursts; corrected HUD capture at 1280x720. Native HDR2 shield and monitor references; owner's native screenshot independently identifies the HUD texture.

**Result:** Authored distance-frame fading is fixed, confirmed by the owner. Correction after the owner's 2026-10-08 follow-up: barrier scrolling animation is still broken; the earlier scrolling-success claim is withdrawn. Changing uploaded UV values did not establish visible animation. Kleiner's monitor scanline motion was verified separately. HUD graphics render over the camera. No playback fix: per-process audio recordings reproduce full sound tails; the owner's halfway cutoff remains unresolved. Owned decoder regression added.

**Still broken or not tested:** Plaza barrier scrolling animation remains broken; proximity fading alone is fixed. Ordinary campaign security/HUD timing, focused manual weapon selection and native audible comparison, complete proxy vocabulary, UnlitTwoTexture envmaps, full lighting/color parity, detail sprites, displacement overlays and overlay fade distances. Barrier native capture lacked fresh loaded-module hash proof (failed helper); later monitor capture verified hashes. Initial paused barrier and black intro monitor captures were rejected. Two system-loopback attempts were contaminated; per-process capture replaced that approach. The regression helper's older image baselines differ substantially, so assertion success does not establish identical pixels. No new FPS benchmark.

**Next:** Diagnose weapon cutoff with a new reproduction and fix remaining barrier scrolling animation; detail sprites and leaf decals per DESIGN11c, then displacement overlays/fades, Metrocop movement/idle + campaign security comparison, renderer LDR/brush-model cubemaps/directional lightmaps. OrbitalMan retains menu/settings ownership.

Newest entries first. Historical entries retain their original wording/test scope; current state is in STATUS.md. This is a Rust rewrite. New entries follow the [requested MODLOG template](https://github.com/trevaintdead/ai-game-modding-guides/blob/main/templates/MODLOG-template.md).

Each entry records **Agent/model**. Historical entries without per-entry provenance are marked "not recorded"; general session environment notes do not establish an individual entry's author or model.

## 2026-10-08 info_overlay fragments on brush faces

**Agent/model:** not recorded.

**Changed:** source-assets::overlays builds fragments per the retail COverlayMgr brush path (private overlay-20261008/README.md: engine.dll 10149db0, 10147b70, 1014c970, 1014d230, 1014c700, 1014cbd0, 1014a6c0):
- V = normalize(N x U), negated by the VBSP flip bit. Quad corners are origin + uv.x U + uv.y V, with texcoords (U0,V0), (U0,V1), (U1,V1), (U1,V0).
- Each listed face is fanned (triangles of area <= 1 are skipped); each triangle is flattened onto the overlay plane and clips the quad.
- Clipped vertices get bilinear texcoords from their inverse-bilinear position in the quad (SDK PointInQuadToBarycentric convention), are projected back onto the face along N and pushed 0.1 units off it (0x1031d910 = 0.1f).

The BSP world fills the new `World::overlays` (lightmapped by the face's lightmap; not decal/impact receivers), and the Bevy host draws them like world surfaces.

**Why:** DESIGN 11b. The plaza "leaves" attribution in the session 5 plan was wrong: d1_trainstation_02's 27 overlays are ivy, wall posters/graffiti, floor stains, blood and two trash decals; none are leaves.

**Tested how:**
- 338 normal tests (new: one-face quad, face clipping, flipped V and bilinear corners) and 30 owned tests (new: trainstation_02 gives 26 overlay surfaces / 474 triangles with valid lightmap pages; the 27th overlay's two trash decals are on displacements). Strict Clippy and fmt.
- Packaged captures vs native HDR: the ivy on the station's outer wall (view T005456Z) matches in placement and shape; the floor around a hall stain (T005326Z) is within 1-2 levels.
- Regression batch artifacts/overlays-regression: all exits 0, verifiers 26/17/26, no viewmodel drops (burst 120). Image changes are confined to the Breen views (the wall stain behind the translucent jumbotron).

**Result:** Overlay decals appear on brush faces.

**Still broken or not tested:** Overlays on displacement faces (the retail displacement-space path, 10148b90), overlay fade distances (HDR fade lump), render-order sorting, and overlays on brush-entity faces.

**Next:** Owner reports: weapon-switch sound cut halfway, plaza Combine barrier fade/animation, Kleiner monitor scanlines; then the plaza leaves/park grass (detail props, DESIGN 11c).

## 2026-10-08 $selfillum for LightmappedGeneric and VertexLitGeneric

**Agent/model:** not recorded.

**Changed:** Materials with `$selfillum` (LightmappedGeneric, VertexLitGeneric) blend toward `$selfillumtint` x albedo by the base alpha before the envmap is added, as in SDK lightmappedgeneric_ps2_3_x.h and vertexlit_and_unlit_generic_ps2x.fxc. The flag is cleared when the base texture has no alpha (SDK dx9 helpers). `$selfillummask`, `$selfillumfresnel` and `$selfillum_envmapmask_alpha` are not supported.

**Why:** The trainstation hall windows (window002e and the left arch windows) were far darker than native.

**Tested how:** 335 normal tests (new self-illum rule test), strict Clippy and fmt. Packaged hall capture vs native HDR view-d1_trainstation_02-20261007T164900Z: upper windows (147,154,164)/(135,140,150) vs native (146,153,163)/(131,137,147), up from (99,102,94)/(59,57,49); left windows (197,187,141)/(192,182,142) vs native (200,189,142)/(196,186,145), up from (72,61,37)/(82,69,43); wall unchanged (140,123,86) vs (140,124,87). Regression batch artifacts/selfillum-regression: all cases exit 0, no pose/visibility mismatches, campaign's 25 known texture errors only. Changes against artifacts/envmap-regression are self-illuminated surfaces: hall windows in the Breen views, the AR2's red rings, the pistol's sight dots and a book in the attention room. Verifiers: movement 26/26, weapons 17/17, attention 26/26. Viewmodel burst: 0 of 121 frames lost.

**Result:** The hall's upper and left windows match native within about 4 levels.

**Still broken or not tested:** The self-illum mask/fresnel modes. Native viewmodel glows were not compared side by side.

**Next:** info_overlay projection (DESIGN 11b).

## 2026-10-08 LightmappedGeneric $envmap cubemap reflections with masks and bump normals

**Agent/model:** not recorded.

**Changed:**
- source-assets::vtf decodes cubemap faces (`decode_cube`: the six faces of frame 0; the 7.0-7.4 spheremap is skipped; RGBA16161616F HDR cubemaps are kept as half floats) and reports translucency (one/eight-bit alpha flags).
- BSP world vertices carry the face plane normal, oriented to the side the winding faces.
- hl2-bevy loads `$envmap` for LightmappedGeneric (HDR mode prefers the map's .hdr.vtf), `$envmaptint`, float3 `$envmapcontrast`/`$envmapsaturation` and `$fresnelreflection`.
- Masks follow SDK lightmappedgeneric_dx9_helper.cpp: a `$bumpmap` undefines `$envmapmask`, `$normalmapalphaenvmapmask` masks by the bump map's alpha, `$envmapmask` by its RGB, and `$basealphaenvmapmask` by 1 - base alpha (cleared when the base texture is opaque).
- Bump-mapped materials (except `$ssbump`) reflect off the normal map: rgb * 2 - 1 in a tangent frame along the texture's s/t axes, as in SDK GetBumpNormals and `mul(vNormal, tangentSpaceTranspose)`. The frame is built from screen-space derivatives.
- The shader adds the SDK specular term: cube(reflect) capped at 16, then mask, tint, contrast, saturation and Fresnel.
- Envmap parameter or mask errors drop only the envmap, never the material.

**Why:** DESIGN 11a. The owner compared the trainstation_02 windows and floors with native. Retail ENV_MAP_SCALE is 16 under integer HDR (shaderapidx9 1004b7b0; private material-20261007 notes), so HDR cube texels are capped at 16. The first unmasked build gave the main hall floor a mirror glare; the owner noted that native's sheen looks like real tile, which comes from the bump normals.

**Tested how:** 334 normal tests and 29 owned tests, strict Clippy and fmt. New tests cover cube decoding (synthetic and HDR), owned window cubemaps, owned brush-normal orientation, and mask selection/vector parameters. Packaged captures were compared against native HDR:
- Outside the window002c wall (view-d1_trainstation_02-20261007T231545Z): bright panes (116,106,76) vs native (121,110,79), right panes (121,114,84) vs (126,118,87); with the envmap off (84,73,43)/(82,72,40).
- Main hall tile floor pitched down (T002014Z halltiles): left/mid/right (54,43,29)/(50,35,17)/(54,39,24) vs native (53,42,29)/(50,36,17)/(53,39,23). The bumped sheen breaks up along the tiles as native's does; flat normals gave a mirror streak.
- Regression batch artifacts/envmap-regression: all cases exit 0, no pose/visibility mismatches, only campaign's 25 known station03 texture errors. Differences against artifacts/bloomfix-regression are confined to envmapped surfaces (plaza windows, a metal piece under Breen's screen, a metal surface in the attention room). Verifiers: movement 26/26, weapons 17/17, attention 26/26. Viewmodel burst: 0 of 121 frames lost.

**Result:** Window and floor reflections appear where native has them and at similar brightness.

**Still broken or not tested:**
- env_cubemap on brush entities and models (runtime nearest-cubemap lookup); VertexLitGeneric envmaps; `$ssbump`.
- Bumped diffuse lighting: the three directional bump lightmaps are not used.
- Mask and bump UVs reuse the base texture transform. Displacement normals use the base face.
- Textures have no mip chain, so distant normal-mapped sheen can alias.
- LDR-mode envmaps (an sRGB path is coded) are **not tested**.
- The hall's left and upper windows needed `$selfillum` (next entry); an intermediate attribution to the trainstation_arch001 prop's lighting was wrong.

**Next:** `$selfillum`, then info_overlay projection (DESIGN 11b).

## 2026-10-08 Fix the blinking weapon viewmodel (bloom pass ordering)

**Agent/model:** not recorded.

**Changed:** The Source bloom pass (hl2-bevy bloom.rs) now runs in Bevy's `Core3dSystems::PostProcess` set, after the camera's main pass, instead of being ordered only `.before(tonemapping)`. New test option `--capture-burst N` saves the N frames after `--capture` as `<capture>-1.png` ... `<capture>-N.png`, so flicker can be measured. Private helpers: work/publishing/flicker_check.py (counts frames without viewmodel pixels) and flicker_bisect_step.sh.

**Why:** Owner report: the weapon viewmodel blinked in and out. The bloom system was not placed in a Core3d set, so the parallel render schedule could run it before the viewmodel camera's main pass. On those frames its post-process swap discarded the weapon drawn afterwards. Bisected with burst captures (the burst option patched onto each tested commit): 0dc8291, 1028388 and 6a7455d clean (0 of 61 frames lost); 8ee25d2 (bloom, session 5) and later lose 1-9 of 61 frames, including 1640df0 (before this session and the macOS merge) and 994df99. SDK viewrender.cpp RenderView draws the viewmodel (DrawViewModels), then the fade/overlays, then DoEnginePostProcessing (bloom), so bloom on the viewmodel camera after its main pass matches Source.

**Tested how:** Weapons fixture with `--capture-burst 120`: two runs, 0 of 121 frames without the viewmodel (before: 1-9 of 61 per run). 331 normal tests, strict Clippy and fmt. Regression batch artifacts/bloomfix-regression: all cases exit 0, captures within 9 levels of the accepted bloom-regression baselines, no pose/visibility mismatches, campaign's 25 known station03 texture errors only; verifiers movement 26/26, weapons 17/17, attention 26/26.

**Result:** The viewmodel draws every frame in the tested fixture. Earlier single-frame captures could not show the bug; the accepted bloom-regression weapons capture happened to land on a good frame.

**Still broken or not tested:** Other Core3d ordering assumptions were not audited beyond this pass. Ordinary play was **not tested** by an automated check; the owner's live report is the reference.

**Next:** Envmap masks (`$normalmapalphaenvmapmask`, `$envmapmask`) on wip/envmap before it merges.

## 2026-10-07 README platforms table; feature list moved to docs/features.md

**Agent/model:** not recorded.

**Changed:** README.md gains a "Platforms" table near the top: Windows (primary tested), macOS Apple Silicon (tested by a contributor, PR #1), Linux and other systems (**not tested**). The long "Implemented so far" list moved verbatim to docs/features.md; the README keeps a one-paragraph summary with a link.

**Why:** Owner request: show confirmed platforms at a glance and shorten the README. A versioned docs page was chosen over a GitHub wiki so the list stays reviewable in PRs alongside the code.

**Tested how:** Documentation only. Checked that docs/*.md is whitelisted, that no document links to the old README anchor, and that relative links in docs/features.md resolve.

**Result:** README is shorter; the feature list content is unchanged.

**Still broken or not tested:** Linux builds and runs are **not tested**; CI runs on Windows only.

**Next:** Envmap cubemaps (DESIGN 11a) on wip/envmap.

## 2026-10-06 Apple Silicon (macOS aarch64) support for hl2-bevy

**Agent/model:** not recorded.

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

**Agent/model:** not recorded.

**Changed:** Source 8-bit bloom as a Bevy Core3d post pass on the viewmodel camera (the last 3D camera, so it covers sky, world and viewmodel and runs before the HUD): gamma-space Shape (pow 2.2 times luminance with r_bloomtint 0.3/0.59/0.11) over a quarter-size 4-tap (4x4) downsample, 13-tap Gaussian blur in X and Y (SDK offsets/weights; Y times the bloom amount), additive composite onto the gamma frame (Engine_Post BloomFactor 1). env_tonemap_controller SetBloomScale sets the scale; the amount eases by 0.05 per frame from 1 (GetBloomAmount). The exposure histogram now reads a 320x180 pre-bloom presample by GPU readback (the SDK histogram runs before bloom) instead of window screenshots. Monitor (camera feed) views render at tonemap scale 1. Dev profile: no debug info for dependencies, line tables for our crates (target/debug 36 GB -> 3 GB).

**Why:** Native HDR shows bloom around bright windows and screens, and our world stayed 10-20% darker. With bloom in the screenshot the histogram dropped our exposure (plaza 1.45), which is why it must measure before bloom. SDK viewrender.cpp draws monitors before TurnOnToneMapping after the previous main view reset the integer-HDR scale to 1 (lines 2080, 2090, 2214), so applying the scale to the feed and the screen doubled it.

**Tested how:** 328 normal tests, strict Clippy/fmt. Packaged captures vs settled native HDR: hall wall (162,142,101) vs (159,139,100), screen (170,140,106) vs (181,153,121), plaza street (152,137,97) vs (162,144,100), sky (103,102,104) vs (102,101,102), exposure plaza 1.95 vs 2.00, hall 2.00 vs 1.88, slate 0.651 vs 0.65. Regression batch artifacts/bloom-regression: complete, 0 mismatches, movement 26/26, weapons 17/17, attention 26/26; contact sheet inspected (bloom on bright monitors/explosion, HUD unaffected).

**Still broken or not tested:** Window panes need `$envmap` cubemaps (native left windows 159 vs ours ~70). Plaza buildings remain ~15% darker. Bloom is not applied to monitor feeds (matches Source). No LDR mode. Batch images depend on frame timing (auto exposure).

**Next:** Environment maps, then the plaza items (info_overlay leaves, detail props, metrocop).

## 2026-10-07 Source HDR path: HDR lightmaps, auto exposure, HDR sky

**Agent/model:** not recorded.

**Changed:** Rendering now follows Source's HDR path (mat_hdr_level 2). Maps with HDR lighting use the HDR lightmap, ambient and world-light lumps; the lightmap atlas is linear RGBA16F capped at 16 (retail integer HDR range). Every Source material and sprite multiplies its output by the tonemap scale (Bevy camera exposure on the world, viewmodel, monitor and sky cameras). Auto exposure follows SDK viewpostprocess.cpp (17-bin histogram of the presented frame's linear luminance over the central 90% x 85%, 2% bright pixels at 60%, minimum 3% median, V-weighted 10-sample goal) and retail materialsystem 10062660 adaptation (rate x 2, accelerated darkening, step capped at 1/64), with env_tonemap_controller SetAutoExposureMin/Max, SetTonemapRate and UseDefaultAutoExposure. Sky faces with `$hdrcompressedTexture` decode RGBS (rgb x alpha x 8). VMT conditionals evaluate `hdr?` true and `ldr?` false. `--tonemap-scale S` forces a scale (mat_force_tonemap_scale). Histogram readbacks never share a frame with the final capture.

**Why:** Owner screenshots showed their game uses "High Dynamic Range: Full" (mat_hdr_level 2, tonemap scale 1.35 in the console) and a much brighter Combine slate. The native oracle's private config (mirrored by Steam Cloud) had mat_hdr_level 0, so all earlier native captures were LDR; the oracle now passes +mat_hdr_level 2 +skill 2 and can echo cvars per view (ORACLE_QUERY) after a dwell (ORACLE_DWELL). Evidence: SDK stdshaders (FinalOutput TONEMAP_SCALE_LINEAR in LightmappedGeneric, VertexLitGeneric, UnlitTwoTexture, Sky_HDR_DX9, sprites), SDK viewpostprocess.cpp; retail materialsystem 10062660 (adaptation), 10062ae0 (integer HDR lightmap encoding), 10057340/10062b20 (LDR lightmap table, for a future LDR mode), client 101d8490 (histogram query). Private notes: work/hl2-decompiled/material-20261007/README.md.

**Tested how:** Unit tests for histogram bins/target, goal averaging, retail adaptation, Bevy exposure mapping, HDR atlas cap, conditionals; 328 normal tests, strict Clippy/fmt. Native HDR captures with settled exposure (view-d1_trainstation_02-20261007T164900Z: plaza 2.00, hall 1.88; T165556Z slate 0.65). Bevy: plaza 1.80-1.98, hall during the broadcast 1.92 with the screen at (184,158,123) vs native (181,153,121), slate 0.69 with native's cyan/white-glyph look. Regression batch artifacts/hdr-regression: all cases complete, 0 pose mismatches, movement 26/26, weapons 17/17, attention 26/26; every image changes (exposure). Readback cost about +0.3 ms CPU and +0.2 ms GPU at the 60 fps cap. Rejected: gamma-space histogram (hall pinned at 0.5) and unclamped float lightmaps (white slate).

**Still broken or not tested:** The world is still about 10-20% darker than native (no bloom; SetBloomScale .4 is visible in native around windows). Window panes need `$envmap` cubemaps. No LDR mode or options menu yet (owner idea: Video options with HDR None/Full). Auto exposure depends on frame timing, so batch images are not bit-reproducible. HDR on maps without HDR lumps falls back to LDR data with the tonemap scale.

**Next:** Bloom (SDK Bloom/Downsample shaders), environment maps, then the LDR mode for the options menu.

## 2026-10-07 Eyes keep a latched view target (Breen looks into the broadcast camera)

**Agent/model:** not recorded.

**Changed:** Actor eyes now follow SDK CBaseFlex/CAI_BaseActor view-target semantics: the last chosen eye target persists per actor while ValidEyeTarget holds (at least 1 unit away, within 75 degrees of the head). With no scene interest or visible candidate, the eyes look at a point 128 units ahead of the head with the SDK's right +-32 / up +-16 jitter (deterministic per actor and scene clock), so both eyes converge instead of staying parallel.

**Why:** Owner review: native Breen's eyes focus on the broadcast camera while ours looked into infinity. instinct.vcd's `LookAt camera_tv_breen` event is authored inactive, so the native focus comes from the default view target (SDK ai_baseactor.cpp MaintainLookTargets random view and ValidEyeTarget). The camera is near the 128-unit point in front of Breen.

**Tested how:** New unit test; 324 normal tests, strict Clippy/fmt; packaged Breen broadcast report: eye target was none, now "view" at (919, 7517, -237) with both eyes sharing it; attention fixtures 26/26. Feed close-up shows slightly converged irises (256 px feed; not compared side by side with native at that resolution).

**Still broken or not tested:** Native random look-target selection among nearby entities, blink on target change and head-direction decay are approximated by the existing nearest-visible fallback. No native close-up comparison yet.

**Next:** HDR lighting path (owner report: native broadcast screen is much brighter).

## 2026-10-07 Monitor screen colour: VMT conditionals and linear UnlitTwoTexture modulation

**Agent/model:** not recorded.

**Changed:** The VMT reader applies retail MaterialSystem key conditionals (`test?$var`, optional `!`): passing keys replace the plain value, failing keys are skipped. Tests are evaluated for a DX9 sRGB-capable renderer without HDR (`srgb`, `ldr` true; `hdr`, `lowfill`, `360` and unknown tests false). Camera monitor materials (UnlitTwoTexture) now read $texture2 through an sRGB view and convert the ($color x $color2) modulation to linear like SDK SetModulationPixelShaderDynamicState_LinearColorSpace (channels above 1 unchanged; mathlib GammaToLinear table, 1.0 from 0.95).

**Why:** The jumbotron screen (dev/dev_combinemonitor_3) sets `srgb?$color2 "[2.5 2.5 2.5]"`, which we ignored, so the 0.4 $color proxy dimmed the feed, and the scanline texture was multiplied as gamma bytes. Evidence: SDK 2013 unlittwotexture_dx9.cpp/_ps2x.fxc (both samplers sRGB-read, result = base x texture2 x modulation), BaseVSShader.cpp:652, BaseShader.h ApplyColor2Factor, mathlib color_conversion.cpp; retail materialsystem.dll 1003c230 (conditional tests, return value = skip) and 1003b300 (a passing conditional replaces the plain var). Private notes: work/hl2-decompiled/material-20261007/README.md.

**Tested how:** New unit tests for conditionals and modulation; 323 normal tests, strict Clippy/fmt. Packaged slate fixture vs native slate.png: screen mean RGB (78,122,114) vs native (76,156,154) (was (80,118,100)); white glyphs (165,199,199) vs (175,214,218); scanline grid and black frame match. World regions unchanged. Rejected experiment: a 16x lightmap cap gave (105,229,230), far brighter than native; no constant was fitted.

**Still broken or not tested:** Native runs the HDR path: the slate luxels are linear 128 in both lumps, and native's HDR lightmap range and TONEMAP_SCALE_LINEAR saturate the poster further (native G and B equal, ours G > B). HDR lightmaps/tonemapping are not implemented. Fallback block selection (">=DX90", "hdr_dx9", ...) is not implemented. The conditional change affects every material that uses `ldr?`/`srgb?` keys; the regression batch is the next check.

**Next:** Rerun the regression batch and accept baselines; HDR lighting remains a separate, larger plan.

## 2026-10-07 Combine slate frame and lightmap overbright headroom

**Agent/model:** not recorded.

**Changed:** World and brush-entity faces using tools/toolsblack* now render (an owned UnlitGeneric black texture); every other tools/ material stays skipped. The lightmap atlas stores gamma(linear / 2) and lightmapped materials restore the factor 2 before the gamma decode, so baked light between 1.0 and 2.0 is no longer clipped. New fixture test-inputs/bevy-monitors-breen-slate.json views the jumbotron before the broadcast starts.

**Why:** Native trainstation_02 shows only the bright Combine slate on the jumbotron at map start; ours showed Breen around a dark slate. The slate func_brush (*87) is framed by four toolsblack faces we skipped. Its `_minlight 255` is not a runtime value: SDK 2013 VRAD (radial.cpp:676/813) bakes a per-luxel floor of `_minlight*128` into the lightmap. Source LDR lightmaps keep 2x headroom (imaterialsystem.h OVERBRIGHT 2.0), which our clamp at 1.0 removed.

**Tested how:** Updated lightmap unit test (linear 2.0 saturates, 1.0 encodes gamma(0.5)=186), 321 normal tests, strict Clippy/fmt. Packaged slate fixture against native view-d1_trainstation_02-20261007T054643Z/slate.png (artifacts/slate): black frame now matches native; screen mean RGB before/after/native (76,97,82)/(80,118,100)/(76,156,154); wall, rail and window regions unchanged within 2 levels.

**Still broken or not tested:** The screen is still greener and darker than native's cyan (UnlitTwoTexture colour path, next step). Native runs the HDR path (HDR lightmaps and auto-exposure tonemapping), which is not implemented. Regression batch not yet rerun.

**Next:** UnlitTwoTexture modulation/sRGB and the pixel grid; rerun the batch and accept baselines.

## 2026-10-07 Static map decals (Breen's studio backdrop)

**Agent/model:** not recorded.

**Changed:** infodecal entities without a targetname are now projected at map load onto lightmapped world brush faces (modkit-core::decals::static_decal; hl2-bevy assets add_static_decals). Each decal is sized from its base texture times $decalscale, centered on the plane point within 5 units, oriented by the receiving face's texture axes, clipped to its rectangle and lit by the face's lightmap.

**Why:** Owner review of the native jumbotron: the yellow Combine logo behind Breen is his studio backdrop (decals/decal_posterbreentv), not see-through screen areas as I had first recorded. SDK 2013 world.cpp CDecal::StaticDecal places static decals on world brushes; props are excluded by its trace filter.

**Tested how:** New decal unit test, 321 normal tests, strict Clippy/fmt. Packaged Breen fixture: the feed now shows the backdrop with the logo, matching native session view-d1_trainstation_02-20261007T053505Z. **Not run:** the full regression batch with decals (baselines will change wherever static decals exist).

**Still broken or not tested:** Named (triggered) infodecals, decals on brush entities, engine-exact decal projection/orientation (inferred from Source behavior, engine code not reviewed), decal shaders (DecalModulate). Combine slate: native shows only a bright slate while the rest of the screen is off. The slate func_brush (*87) is surrounded by four tools/toolsblack faces (SURF_NOLIGHT), which our BSP builder skips as tools/ materials, so Breen shows around the slate. Native slate brightness (_minlight 255) and the screen's "pixel" look (UnlitTwoTexture: sRGB-read base x texture2, HDR tonemap scale) are not matched yet.

**Next:** Render tools/toolsblack as opaque black, apply func_brush _minlight, compare the screen shader and pixel grid with native, rerun the batch and accept baselines.

## 2026-10-07 Scene gestures for template-spawned actors (Barney's head-down at 21-22.5 s)

**Agent/model:** not recorded.

**Changed:** Choreography clip preparation now resolves every entity a scene actor name can match, including point_template children that are still pending (Barney, Kleiner), so their scene gesture and posture clips load with the map. Head control now measures its correction in the full 3D frame of the animated "forward" attachment, which is parented to the head bone (SDK UpdateHeadControl), instead of a level yaw-only frame. The Bevy report lists per-actor gesture composition errors (`gesture_compose_errors`). The regression batch drops the superseded edge-on Breen and empty-lab cases, whose fixtures are deleted and baselines retired.

**Why:** Owner report: Bevy Barney was stiff (no arm/body motion) compared with native. Investigation of the unexplained native head-down at security_02 21-22.5 s: native plays rubNeck (Gesture08) and thinking (posture01). The shared composition raised Barney's right hand from z 40.8 to 64.7 at 21.6 s, but the Bevy report showed MissingSequence for posture01/gesture08 (Barney) and kposture01/kgesture04 (Kleiner). scene_actor skips killed entities, and template children are killed until ForceSpawn, so their clips were never prepared at load. Every scene gesture for these actors was dropped in Bevy.

**Tested how:** 320 normal and 26 owned tests (new owned test: template-pending Barney/Kleiner require gesture08/posture01/kposture01 before ForceSpawn; new head-frame unit test), strict Clippy/fmt. Packaged close-ups at 21.00/21.30/22.45 s now match native (head bowed, hand at neck); wide 21.30 s view matches the native pose. Batch artifacts/trimmed-regression: all complete, 0 pose/visibility mismatches, attention 26/26. Kleiner-scene changes (Kleiner now gestures); weapons varies only in the HUD health box between runs. Baselines accepted.

**Result:** Barney's and Kleiner's authored gestures and postures play in Bevy, which explains the native head pitch (it is the gesture, not head control).

**Still broken or not tested:** Native comparison of the full timeline after this fix; other template-spawned NPCs are covered by the same fix but untested.

**Next:** Native trainstation_02 jumbotron/godray comparison; campaign-order reveal chain.

## 2026-10-07 Kleiner scene movement and new regression views

**Agent/model:** not recorded.

**Changed:** npc_kleiner is registered with the shared human ground-movement controller, like Barney, so scene MOVETO events move him. New regression cases: test-inputs/bevy-monitors-breen-screen.json (face-on jumbotron during the broadcast) and test-inputs/bevy-monitors-kleiner-scene.json (Kleiner on Barney's monitor in campaign state). The new baselines are accepted, with the previous ones archived locally.

**Why:** Owner report: the old Kleiner case showed an empty lab and the old Breen case an edge-on screen. Kleiner is a kleiner_template child, and security02 opens with `MoveTo marks_kleiner_catwalk_1` (0.01 s); because only Barney was registered, Kleiner stayed at his spawn, 110 units outside the lab camera. SDK 2013 npc_kleiner.cpp and the retail CNPC_Kleiner::Spawn (server.dll 10391cd0, reviewed privately) agree on HULL_HUMAN, SOLID_BBOX, MOVETYPE_STEP and capabilities 0x801801 (ground move, open doors, turn head, animated face) plus friendly-damage immunity.

**Tested how:** hl2-simulation tests (139), strict Clippy/fmt. Regression batch artifacts/kleiner-regression: all 11 cases complete with 0 pose/visibility mismatches. Diff counts against the old baselines are unchanged from the lighting batch, except weapons (view-model lighting basis). Attention verifier 26/26. Kleiner reaches (-850, 2334) and speaks on the monitor at 10.5 s; Barney's faceplate is off and the cameras are folded in the attention fixtures.

**Result:** The lab feed shows Kleiner talking; the jumbotron shows Breen face-on. New accepted baselines cover the lighting change, the godray fix and the new views.

Follow-up (owner report: Barney was talking to the default blue screen): all eight security_02 fixtures now also force-spawn kleiner_template, enable barney_security_monitor_1 and switch on kleiner_security_camera_1, as the campaign's triggers do. Rerun batch artifacts/kleiner-monitor-regression: pixel-identical to the new baselines, attention 26/26, and Kleiner is on Barney's monitor in the attention view. The private native security oracle setup gets the same three inputs (not yet run).

**Still broken or not tested:** Kleiner's walk/turn timing against native is not compared. Other NPC classes (metrocops, citizens) are still not registered for movement. No native comparison of the lab feed or jumbotron yet.

**Next:** Native head-pitch analysis; native jumbotron/lab feed comparison once the oracle cursor fix is approved.

## 2026-10-07 Bevy becomes main; Macroquad host removed

**Agent/model:** not recorded.

**Changed:** At the owner's request, bevy-migration was merged into main (main's docs-only commit b4b1731 was merged in first) and main now fast-forwards to it. The former Macroquad/OpenGL host was removed: crates/hl2-runtime, its launchers (launch.cmd, launch-borderless.cmd, launch-1080p.cmd), scripts/build.ps1, the vendored third_party/miniquad and quad-alsa-sys shim, its 13 input-script fixtures and the mods/*.json sandbox files. Cargo.lock only drops the 20 packages of that stack. AGENTS, README, CONTRIBUTING, STATUS, docs and CI now describe a Bevy-only rewrite on main. macroquad-prototype, bevy-migration (frozen at the merge) and wip/scripted-scenes are preserved.

**Why:** Owner decision: Bevy is far ahead of the old host, so checking both hosts after shared changes was overhead. Removing the old host keeps the repository clean.

**Tested how:** After removal: 319 normal tests (9 removed with the old host) and 25 owned tests, strict Clippy/fmt, packaged Bevy build, and the movement (26/26) and entities/weapons (17/17) packaged fixtures with their verifiers.

**Result:** One host. The shared crates (source-assets, modkit-core, hl2-simulation, hl2-ui) are unchanged.

**Still broken or not tested:** History and the macroquad-prototype branch still hold the old host; its features that Bevy lacks (JSON sandbox, inspect/verify/export subcommands) are gone from main.

**Next:** Scene MOVETO for Kleiner (lab feed), Breen face-on regression case, native head-pitch analysis.

## 2026-10-07 Source model lighting (leaf ambient cubes and world lights)

**Agent/model:** not recorded.

**Changed:** Models are now lit the way the retail engine's light cache does it, instead of with a constant gray. New `modkit-core::lighting`: leaf ambient samples weighted by 1/(d^2+1); world lights with Source falloff, styles and a world-only visibility trace (8-unit slack); up to four local lights kept by luminance and the rest folded into the ambient cube; skipping lights flagged as already baked into the cube; the ambient boost for flagged models; SDK vertex shader terms. New `source-assets::model_lighting` reads the leaf tree, the LDR/HDR leaf ambient lumps (matching the lightmap choice), world lights and sky faces. `Vertex.normal` carries VVD normals, and the studio illumposition/flags are read per model. Static props are baked once per vertex at load, which applies to both hosts. Bevy model draws get a per-entity lighting uniform that is recomputed when the illumination origin moves. The shader evaluates ambient cube + diffuse (+ $halflambert) on the linearized base texture, with skinned normals on both GPU and CPU paths. The view model maps its camera space onto the player's view for lighting.

**Why:** Owner report that entities lack the map's lighting (DESIGN step 8). Retail engine.dll was reviewed privately (lighting-20261007/README.md): Mod_LoadLeafs, Mod_LeafAmbientColorAtPos, light-cache selection, the draw-time ambient boost and the light-to-shader conversion. SDK 2013 corroborated ColorRGBExp32ToVector (its factor of 255) and the light-descriptor cone math.

**Tested how:** 328 normal tests (7 new lighting/decoder tests) and 25 owned tests (d1_trainstation_01: 535 world lights, 13,479 ambient samples), strict Clippy/fmt. Packaged Barney close-ups against native: at 18.05 s, mean skin RGB is 95/66/45 (Bevy) vs 93/64/42 (native), with luminance percentiles 48/67/93 vs 45/65/90; at 22.45 s the helmet and hair tones match (confirmed by the owner). A new private generic native view oracle (run_view_oracle.py) captured the d1_trainstation_03 corridor: the door is equally dark and the pistol view model lit similarly. Regression batch artifacts/lighting-regression: every case completes with 0 pose/visibility mismatches; images change wherever models appear (expected relighting). Retained smoke capture renders with baked static props.

**Result:** Barney, props and the view model now pick up the map's ambient color and nearby lights. Face shading follows the native key light.

**Still broken or not tested:** Shadows; bump/phong/specular (native helmet sheen); flex normal deltas; styled lights are treated as on; the skylight uses sky-face polygons instead of a surface-flag trace; HDR tonemapping; the retail 162-sample static-prop path. The retained host does not light dynamic models. The first native corridor capture was rejected because the game took focus and trapped the owner's cursor. The oracle now records the user's foreground window before launch and passes +cl_mouseenable 0.

**Next:** Explain the native head-down pitch at 21-22.5 s; trainstation_02 jumbotron/godray native comparison; campaign-order reveal chain.

## 2026-10-06 Entity render color in Bevy (godray brightness)

**Agent/model:** not recorded.

**Changed:** Bevy entity materials now apply Source color modulation. rendercolor tints the model, and renderamt sets its alpha outside kRenderNormal. Materials are keyed per (material, lightmap, modulation), so untinted entities still share handles. New fixture test-inputs/bevy-monitors-breen-screen.json frames the trainstation_02 jumbotron face-on at about 25 s, during the broadcast.

**Why:** Owner report: the trainstation godrays were overexposed in Bevy (the retained host already applied rendercolor). The shafts are additive prop_dynamic vol_light models tinted to about 19% (rendercolor ~49 45 34), which Bevy drew at full white. The owner also noted the old Breen fixture looks at a wall. jumbotron1's screen layers face +-Y, so the old yaw-180 view is edge-on, and its 7 s capture precedes the broadcast (the Combine slate brush is correct then; the scene starts at about 11 s and its OnTrigger1 hides the slate).

**Tested how:** Packaged captures (artifacts/godrays). Regression batch artifacts/godray-regression: only the Breen main view changes (whole frame, dimmer shafts); weapons/Kleiner/campaign identical; movement unchanged from the previous batch (735 px against the old baseline); attention 26/26.

**Result:** Godrays are subtle instead of whiting out the hall. The new fixture shows Breen speaking on the jumbotron, with lip sync.

**Still broken or not tested:** No native trainstation_02 comparison yet for shaft brightness or the jumbotron, whose dark feed areas render see-through. The accepted partition-breen baseline image predates this change.

**Next:** Model lighting (docs/DESIGN.md step 8).

## 2026-10-06 Scripted-sequence script events (faceplate removal)

**Agent/model:** not recorded.

**Changed:** While a scripted_sequence plays, SCRIPT_EVENT_FIREEVENT (1003) animation events crossed by the actor's clip fire the script's OnScriptEventNN output, with NN from the event options and the actor as activator.

**Why:** ss_Helmet_Reveal (helmet_reveal, events "1" at cycle 0.417 and "2" at 0.762) removes Barney's head faceplate and toggles the hand copy through these outputs.

**Tested how:** Packaged Bevy captures before, between and after the events (artifacts/reveal): Barney lifts the faceplate, then his face is revealed with helmetBack kept. hl2-simulation tests and strict Clippy/fmt pass.

**Still broken or not tested:** The native side-by-side of security_01 and the reveal was not run. Other studio script events (1000-1008) are not dispatched.

**Next:** Model lighting; Breen fixture framing; godray brightness.

## 2026-10-06 Head/body flexes, Combine camera activities and movement turning

**Agent/model:** not recorded.

**Changed:** Actor pose parameters now add the server-side flex controllers, following SDK CAI_BaseActor: head_rightleft/updown/tilt on top of the head look correction (UpdateHeadControl), body_yaw/spine_yaw/neck_trans from body_rightleft/chest_rightleft/head_forwardback (UpdateBodyControl), and gesture_height/width from gesture_updown/rightleft (MaintainLookTargets). The values are included in the pose signature. Head control takes its eye position from the animated "eyes" attachment and the self-look direction and head frame from "forward" (shared `Scene::attachment_frames`). npc_combine_camera deploys at spawn (open idle) unless StartInactive. Enable/Disable/Toggle play the retract transition into closed idle, or return to open idle. Walking NPCs turn toward the path at MaxYawSpeed (45 x 10 deg/s) instead of snapping, and the move_yaw pose parameter keeps the legs on the path.

**Why:** Owner steps 5 and 7. Step 6 check: Barney's state origin at his desk equals mark_barneyroom_monitor_3 exactly, and the 18.05 s native/Bevy silhouettes line up within about one unit, so the session 2 ~5-unit offset no longer reproduces.

**Tested how:** 321 normal tests (new synthetic head/body flex and camera activity tests), strict Clippy/fmt. Packaged close-ups at 18.05/21.00/22.45 s against native sessions security-scene-20261006T204141Z and T214010Z. Camera open/retracting/closed captures (artifacts/camera-acts). Walk samples along Barney's route (artifacts/facing).

**Result:** Scene head flexes move Barney's head by their authored degrees. Cameras fold when logic_disable_cameras fires. Path corners turn smoothly.

**Still broken or not tested:** At security_02 21-22.5 s native Barney's head pitches down toward the consoles while Bevy's stays level. Head flexes and gesture pose flexes are ruled out (both near zero there), and the cause is unexplained. The reversed camera-open transition, camera aim pose and eye sprites are not modeled. Entity lighting and shadows: plan recorded in docs/DESIGN.md step 8.

**Next:** Source model lighting (ambient cubes, world lights, normals).

## 2026-10-06 Lip sync from speech phonemes

**Agent/model:** not recorded.

**Changed:** New `source-assets::sentence` reads the text VDAT chunk of owned WAVs (SDK CSentence 1.0: word phonemes and emphasis samples, Catmull-Rom emphasis intensity) and flex settings files (`expressions/phonemes*.vfe`). New `hl2-simulation::lipsync` implements the client viseme path: box filter (0.08 s), neighbor crossfade extension, and weak/normal/strong emphasis blending. The pinned SDK was statically compared with retail client.dll AddVisemesForSentence 100bb720, AddViseme 100bb630 and ComputeBlendedSetting 100bbbd0 (private review). Both hosts parse VDAT when loading waves, register the actor's voice with the chosen wave when a scene line plays, and render `Scene::actor_flex_values` (scene controllers plus visemes) through the shared FaceModel path. Scenes flagged ignorePhonemes play without lip sync.

**Why:** Owner step 4: mouth movement timed to speech.

**Tested how:** Unit tests (VDAT parse/intensity, viseme box filter/cleanup). Owned tests: phoneme VFE files (48 normal, 1 weak, 1 strong settings; aa opens jaw_drop); all 23 Barney waves in d1_trainstation_01 scenes carry phonemes and move the mouth. Strict Clippy/fmt. Packaged Bevy close-ups during ba_thinking01 (21.00/21.30 s) show the lips parting and closing.

**Result:** Barney's mouth follows his lines in Bevy, and the retained host shares the path.

**Still broken or not tested:** Voice time is scene time since the request, not mixer position. phonemedelay/phonemesnap/LOD and streaming delays are fixed at defaults. Native frame-by-frame mouth comparison is limited by about 0.05 s oracle timing jitter. Retained lip sync was not visually checked.

**Next:** Head flexes into head control; Barney's mark offset.

## 2026-10-06 Retained-host facial flexes and volume option

**Agent/model:** not recorded.

**Changed:** The retained host now applies the shared `FaceModel` flex path: per-vertex studio flex keys in its draw batches, NPC face models loaded at setup, and scene controller values plus rest-gaze FACS eyelids added to bind positions before CPU skinning. It also accepts `--volume 0..1` as a master scale for all sounds, so unattended tests run at 1%.

**Why:** Shared changes must work in both hosts. Quiet testing had no option on the retained host.

**Tested how:** Strict workspace Clippy/fmt, hl2-runtime tests. Packaged retained G-Man intro close-ups (artifacts/retained-flex/gman-closeup.png, launched without activation at volume 0.01): faces deform sanely and the expression changes over time.

**Result:** Retained actors show scene facial expressions.

**Still broken or not tested:** The retained host has no eye-target presentation (eyelids use the rest gaze) and no iris shader. In a retained security_02 smoke run Barney did not spawn from his template (helmetBack stayed at its map position), a retained-specific gap not investigated. The retained input script needs an explicit `quit` before it writes its report.

**Next:** Lip sync from phoneme data.

## 2026-10-06 Entity parenting and Barney's helmet props

**Agent/model:** not recorded.

**Changed:** The simulation now supports movement parenting. Map-spawn `parentname` keeps the spawn offset, and the `SetParent`, `SetParentAttachment`, `SetParentAttachmentMaintainOffset` and `ClearParent` inputs work. Each tick, children follow the parent's pose or animated attachment, using the same composed pose both hosts render. `prop_dynamic` StartDisabled hides the prop. Gear attached to an animated attachment has no collider, so a helmet no longer blocks its wearer. The security fixtures now include the campaign state by security_02: faceplates off (ss_Helmet_Reveal runs in security_01) and logic_disable_cameras triggered.

**Why:** Owner priority: in security_02 Barney wears the separate helmetBack prop, parented by logic_barney_init to helmet_attachment after the template spawn. Only the faceplate comes off, during ss_Helmet_Reveal.

**Tested how:** 317 normal + 22 owned tests (synthetic parenting test), strict Clippy/fmt. Packaged security captures compared with native session security-scene-20261006T204141Z. Regression batch artifacts/helmet-regression: weapons/Kleiner/campaign identical; movement 751 px and Breen 3,383/6,654 px, the same known autoplay differences as the accepted 1bf087f batch. Attention 26/26.

**Result:** helmetBack renders on Barney's head through the whole scene, matching the native close-ups. With the faceplates off, Barney walks to his desk again. (An intermediate build where the solid helmet blocked his route was rejected and fixed.)

**Still broken or not tested:** The faceplate removal in security_01/ss_Helmet_Reveal has not been played or compared yet. Moving brush/physics hierarchies are not parented. Nested hierarchies lag one tick. Retained host not visually checked (shared simulation only).

**Next:** Retained-host flexes, lip sync, head flexes, Barney's mark offset.

## 2026-10-06 FACS eyelids and native face close-ups

**Agent/model:** not recorded.

**Changed:** Eyeball records now carry their FACS eyelid fields, and actor flexes apply the retail eyelid step (private review: StudioRender 1001bd80). It converts lid raiser/neutral/lowerer weights and the eye's look direction into lid descriptor values before vertex deltas. Flex math moved into shared `source-assets` (`FaceModel`, `descriptor_weights`, `vertex_deltas`) so the retained host can reuse it. Rigs now keep every model attachment. Movement scripts accept `pitch` and `fov` (Source horizontal 4:3 degrees) for zoomed comparison shots, and the private native oracle takes a matching `ORACLE_CLOSEUP`.

**Why:** Native face close-ups at 18.10 s showed Barney's eyes fully open. Bevy left the lid descriptors at zero, which half-closed the upper lids and raised the lower ones.

**Tested how:** 316 normal + 22 owned tests (synthetic eyelid test; owned Barney/Kleiner neutral face has no lid deformation), strict Clippy/fmt. Packaged close-ups `test-inputs/bevy-face-security-1805/2245.json` compared with native session security-scene-20261006T204141Z (scene times 18.10/22.50 s).

**Result:** Bevy lids now match the native open eyes at both times.

**Still broken or not tested:** Lids use the previous frame's eye direction. Head pose (native head turns and tilts further toward the player), brow intensity, Source model lighting (ambient cube/local lights), helmet props, delayed flex weights, lip sync and the retained-host flex path. Full regression batch not rerun.

**Next:** Helmet props parented to Barney's attachments, then retained flexes, lip sync and head flexes.

## 2026-10-06 Facial flexes render in Bevy

**Agent/model:** not recorded.

**Changed:** Scene-driven flex controllers now deform actor faces in the Bevy renderer (setup-loaded flex data, per-vertex flex references, retail weighting, GPU and CPU skinning paths). Earlier the same day: the flex track evaluator and Surface flex sources.

**Why:** Scripted scenes need facial animation; Barney's security_02 expressions were missing.

**Tested how:** 315 normal tests, strict Clippy/fmt, owned flex/surface mapping test, packaged security captures, attention 26/26.

**Result:** Barney's expression changes during security_02.

**Still broken or not tested:** Retained host flexes, native facial comparison, delayed weights, lip sync/phonemes, eyelid-eye interaction, normals.

**Next:** Native face comparison at matching times, then lip sync (phoneme tracks to flex settings).

## 2026-10-06 Facial flex data reader

**Agent/model:** not recorded.

**Changed:** New `source-assets::flexes`: MDL flex descriptors, controllers, rules and mesh vertex deltas; SDK RunFlexRules and the retail vertex weighting.

**Why:** First step toward facial animation in scenes (FlexAnimation tracks, expressions, later lip sync).

**Tested how:** Synthetic rule/ramp tests; owned Barney/Kleiner parse and jaw_drop evaluation; strict Clippy.

**Result:** Real flex data parses and evaluates.

**Still broken or not tested:** Not driven by scenes and not rendered yet; delayed weights and wrinkles are not applied.

**Next:** Scene flex tracks to controller values, then CPU vertex deltas in both renderers.

## 2026-10-06 Pose-parameter blends, autoplay head sequences and head control

**Agent/model:** not recorded.

**Changed:** Rigs keep pose parameters, full blend grids and autoplay sequences. Composition samples blends by pose parameter and adds autoplay after layers. NPC head control drives head_yaw/pitch from look interests with the SDK think rates.

**Why:** FACE turned Barney's body away from the player; the original turns his head on pose parameters. Blend grids and autoplay are also how the models encode head, body and gesture variation.

**Tested how:/** 312 normal and 21 owned-install tests, strict Clippy/fmt; synthetic blend/head tests; owned Kleiner autoplay/blend checks; attention 26/26; regression batch.

**Result:** Barney looks over his shoulder at the player while facing the monitor.

**Still broken or not tested:** Chest-bias limits, roll, random/synthetic looks, eye-position attachments in the simulation, sequence transitions, IK, bone controllers, the posekey 2D path and locomotion move_yaw blending for base clips.

**Next:** Rerun the native security comparison; then facial flexes or locomotion move_yaw blending.

## 2026-10-06 Scene FACE events, arrival distance, Combine camera model

**Agent/model:** not recorded.

**Changed:** FACE events turn standing NPCs toward targets at SDK yaw speed. MOVETO walks until within the event arrival distance (2D). The npc_combine_camera model is set. Unattended-test options: Bevy `--volume`/`--no-focus`, and a quiet, unfocused native oracle.

**Why:** In the native comparison, Barney faced the wrong way, stopped 9 units short of his mark, and the wall camera was missing.

**Tested how:** 310 normal tests, strict Clippy/fmt; FACE unit test; packaged security fixtures; movement and attention verifiers.

**Result:** Barney stands on his mark and faces the monitor; the camera renders.

**Still broken or not tested:** Attention 24/26: without head pose control Barney cannot look at the player while his body faces the monitor. Facing while moving, camera open/close activities and facial flexes are not implemented.

**Next:** Pose-parameter sequence blending and CAI_BaseActor head control (head_yaw/head_pitch) driven by attention targets.

## 2026-10-06 NPC door lookahead: Barney reaches his desk

**Agent/model:** not recorded.

**Changed:** NPC routes remember door segments and request the door ahead of contact (within 96 units).

**Why:** The cop-door trigger unlocks barney_door_2 for only one second, before Barney touches the door.

**Tested how:** 308 normal tests, strict Clippy/fmt; packaged security fixtures; movement 26 and attention 26 assertions; native/Bevy side-by-side.

**Result:** Barney walks through both doors and gestures at the desk in Bevy, as in the original.

**Still broken or not tested:** FACE events, exact arrival position, the wall Combine camera, desk material, and a native rerun with the mask fix. The full regression batch was not rerun.

**Next:** FACE events, then rerun the native comparison (mask off) and compare poses.

## 2026-10-06 Trigger toucher filters

**Agent/model:** not recorded.

**Changed:** trigger_once/trigger_multiple honor client/NPC/everything toucher flags, and NPCs can activate NPC triggers. Security fixtures enable trigger_cop_close_door_1 as the campaign does earlier.

**Why:** NPC-only triggers fired for the player, and NPCs could not trigger anything, which blocks Barney's door sequence.

**Tested how:** 308 normal tests, strict Clippy/fmt; packaged movement assertions and movement/campaign pixel comparisons.

**Result:** No regression in the checked fixtures. Barney still stops at barney_door_2.

**Still broken or not tested:** Why the cop-door trigger does not let Barney through; no NPC-touch unit test; the full regression batch was not rerun.

**Next:** Instrument trigger_cop_close_door_1 bounds and door lock timing against Barney's feet, then rerun the native comparison.

## 2026-10-06 Native scene oracle, point_template spawning, prop flags and NPC doors

**Agent/model:** not recorded.

**Changed:** Private native oracle for the security scene. point_template children wait for ForceSpawn. prop_physics motion-disabled/start-asleep flags are honored. NPC routes pass through doors and open them on contact. Barney fixtures force-spawn him like the campaign trigger.

**Why:** The first native comparison showed Barney never reaching his desk in Bevy. The causes: closed doors, a desk that physics wrongly moved, and templated actors existing too early, which is also wasted simulation.

**Tested how:** 308 normal and 21 owned tests, strict Clippy/fmt; synthetic template test; owned security tests with ForceSpawn; native oracle captures; packaged security fixtures and the regression batch.

**Result:** The desk stays at its authored pose. Barney spawns on demand, opens the interrogation-room door and walks to the next door.

**Still broken or not tested:** NPC-touch triggers (trigger_cop_close_door_1 unlocks barney_door_2), so the desk walk and a side-by-side gesture comparison are unfinished. The Combine wall camera is not rendered. Repeat ForceSpawn copies, EnableMotion and door-blocked replanning edge cases are untested.

**Next:** NPC trigger touch (spawnflags 2), then rerun the native/Bevy security comparison at 18.05/22.45 s.

## 2026-10-06 Scene gesture execution

**Agent/model:** not recorded.

**Changed:** Scene GESTURE events create per-actor layers: SDK tag retiming, intensity weights, posture suppression and RemoveLayer fades. Both hosts compose base clip and layers through `Scene::actor_matrices`. Faceposer keyvalues are parsed, and the clip budget accounts for gesture children.

**Why:** Authored scene gestures were ignored, so actors only played base clips.

**Tested how:** 307 normal and 21 owned tests, strict Clippy/fmt; synthetic retiming/layer tests; owned security_02 playback with Barney's rig; packaged 1,600-tick security timeline and the regression batch (pixel comparisons, movement/weapons/attention verifiers, retained smoke).

**Result:** Barney, Kleiner and the G-Man actor now layer their authored gestures in Bevy. Existing fixtures without gestures are unchanged.

**Still broken or not tested:** Comparison against the original game, cross-scene layer priority, IK, head/facial animation and lip sync. Empty-name gestures produce no layer, and native handling of them is unverified. Posture motion uses scene movement as IsMoving.

**Next:** Capture the same security scene moments in the original game and compare poses/timing; then head pose and facial flexes.

## 2026-10-06 Shared Source animation layer composition

**Agent/model:** not recorded.

**Changed:** modkit-core composes Source sequences: delta/post layers, per-bone weights and autolayer ramps through `Rig::accumulate_pose`. source-assets keeps raw delta frames, sequence flags, bone weights, fades and named autolayers, and loads autolayer children with their parents.

**Why:** Real NPC gestures are masked parents whose children are delta layers. The old absolute delta conversion and base-clip-only path could not reproduce them.

**Tested how:** 302 normal and 20 owned tests, strict Clippy/fmt; synthetic composition tests; owned Barney g_pointRight load/compose check; retail client.dll AccumulatePose/AddSequenceLayers/SlerpBones static comparison; five packaged fixtures pixel-identical to 28c0bb3 captures, plus movement 26, weapons 17 and attention 26 assertions and a retained smoke capture.

**Result:** Composition matches the SDK and the reviewed retail code paths. Existing scenes are unchanged.

**Still broken or not tested:** Scene gestures are not executed yet. IK, local-context, world-space and pose-parameter layers, fixed-alignment slerp, 3-way blends and native runtime pose comparisons are missing.

**Next:** Scene gesture layers: faceposer tag retiming, intensity and end fades, then compose them in both hosts.

## 2026-10-06 Priority change: resume Rust rewrite, on-demand decompilation

**Agent/model:** not recorded.

**Changed:** Owner decision: pause whole-corpus decompilation coverage after pass14 and resume the Rust rewrite with authored NPC gestures. AGENTS.md, STATUS.md and docs/DESIGN.md now make decompilation feature-driven: retrieve and review the relevant retail functions from the private index before implementing each step. A drafted pass15 dispatcher-trace plan was dropped before any run.

**Why:** About 94% of baseline executable bytes already sit inside identified functions with pseudocode (133,473 addresses), but zero have reviewed names/types. Further coverage passes have diminishing returns for the campaign; reviewed semantics of specific systems are what the rewrite needs.

**Tested how:** Documentation-only change: whitelist, local-link and public snapshot checks. No runtime build/test rerun.

**Result:** Handoff documents agree on the new priority. Private research databases are unchanged.

**Still broken or not tested:** Everything listed in STATUS.md; full decompilation, names/ABIs and native parity remain unfinished.

**Next:** Identify retail animation-layer/gesture functions in the index, record reviewed findings privately, then implement raw delta/post/mask/layer readers.

## 2026-10-05 Callback discovery, ABI and switch evidence; quota handoff

**Agent/model:** not recorded.

**Changed:** Continued private research through pass14. The exact-byte index now combines13 source datasets,133,473 observed addresses with some pseudocode and133,556 export variants across42 selected modules. Saved entries and10,752 temporary discovery entries remain separate. Public changes document research; runtime code is unchanged.

**Why:** Export success, listing ownership and decompiler switch recovery do not establish complete discovery or trustworthy ABIs. Preserve failures, variants and module/hash/address identity before using native evidence in Rust.

**Tested how:** Exact database/source-row/hash audits, original project bytes, callback/disassembly rollback, private PE data/relocations,16 x87 context/recovery checks,44 existing-function switch/owner controls, ambiguity/overwrite refusal and C-only gamedb range audits. Detailed commands are in docs/validation.md.

**Result:** All research audits pass. Callback discovery adds8,992 bodies and bounded disassembly62. All2,462 disassembly addresses restore original entry/listing absence. All2,333 data records match PE bytes. Original project/catalogues remain unchanged; type/implementation/native-verification flags remain empty.

**Still broken or not tested:** Full decompilation, scope/ABI/names, native gameplay and campaign parity. The two-double-width pilot fails;45/46 orphan dispatch controls remain unmatched. Navigator omissions/range errors and rejected mixed/incomplete indexes remain recorded. Runtime tests were not rerun for documentation-only changes.

**Next:** Raw branch/table-bound/shared-tail evidence for unresolved dispatchers, x87 helper storage and runtime scope. Preserve the final private checkpoint near the owner's requested quota margin; gestures remain queued.

## 2026-10-05 Continued discovery and optional-module databases

**Agent/model:** not recorded.

**Changed:** Resumed authorized private research after the owner corrected an early stop. A private baseline clone adds 9,850 candidate exports. Seventeen optional modules expand saved coverage to 42 modules/122,721 entries with some pseudocode. Separate raw-flow/disassembly passes produce 34 and 11 temporary bodies. Public changes document evidence only.

**Why:** Whole-module export counts omitted callable entries and optional scope. Checkpoints are validation boundaries; preserve baseline provenance while continuing discovery, rather than treating a checkpoint as task completion.

**Tested how:** Original project/catalogue hashes; relocated pointer/RTTI records and bounded instruction flow; exact SQLite artifact/body audits; saved optional entry sets; five new failure recoveries/signature checks; independent rollback of temporary function/listing state; exact body retrieval and scoped gamedb control. Commands/counts are in docs/validation.md. Installed content stayed read-only.

**Result:** Discovery ledger:20,842 artifacts/9,850 bodies/zero discrepancies. Optional ledger:393 artifacts/23,987 successful body rows plus five preserved original errors/zero discrepancies. All16 known original failures have separate recovery bodies. Temporary 34/11 body ledgers and rollback checks pass. Discovery navigator has one range mismatch; optional navigator omits2,544 bodies and has779 mismatches, so exact path/hash/address retrieval remains authoritative.

**Still broken / not tested:** Complete discovery/runtime scope, original names/ABIs, exception coverage, optional activation, middleware/tool/variant exclusions and native behavior remain unresolved. Two analysis-adjusted flow attempts, initial writable-RTTI rejection, missing project-directory launch and Vulkan exception-handler analyzer error remain recorded. No Rust change, rebuild, gameplay capture, performance or multiplayer test.

**Next:** Continue optional orphan-target discovery and relocated callback/instruction-operand references across all42 snapshots, retaining versioned databases and rollback proofs. Gestures remain queued behind the documented research priority.

## 2026-10-05 Private database coverage and failed-export recovery

**Agent/model:** not recorded.

**Changed:** Continued private decompilation with a 126-path hashed PE inventory, discovery audits, three new loader-referenced module exports, eleven supplemental failed-body recoveries and a path/hash/address SQLite evidence ledger. Public changes are evidence documentation only.

**Why:** The 22-module export denominator omitted runtime modules and did not establish function discovery. Preserve original errors and uncertain signatures while improving database retrieval and recording unresolved coverage.

**Tested how:** Fresh baseline catalogue audits before/after; all 22 saved/installed/private hashes; read-only Ghidra instruction/storage/discovery inspection; eleven rollback signature checks; supplemental 301-artifact/body audit; exact VPhysics/Speex retrieval and a scoped gamedb CreateInterface control. See docs/validation.md for commands. Installed files were read-only.

**Result:** Across 25 selected modules: 88,884 identified addresses, 88,873 original bodies plus 11 supplemental recoveries. Original failure rows remain. Baseline 22-module audit finds 1,459,199 executable bytes outside functions and 13,830 unresolved pointer-slot observations. Supplement audit has zero discrepancies; fresh gamedb extension indexes 827 bodies, misses 263 and has 36 range mismatches. Native research remains outside Rust source checkouts.

**Still broken / not tested:** Complete runtime scope, discovery, recovered ABIs and native semantics remain unverified. No Rust behavior, game capture, campaign, performance or multiplayer test in this pass. Failed return-type recovery, initial debug setup, old extension-builder path rejection and rejected newline audit remain recorded privately.

**Next:** Review bounded unlabelled RTTI/vtable/orphan-code candidates and remaining optional loader branches on disposable projects, then extend a new audited database version. NPC gestures remain queued.

## 2026-10-05 Decompilation priority for the new chat

**Agent/model:** not recorded.

**Changed:** Updated AGENTS.md, STATUS.md and docs/DESIGN.md with the owner's new decompilation priority, plus a private inventory/failure checkpoint.

**Why:** Continue toward complete original-game coverage in a fresh chat while retaining the verified Rust build and precise limits of the database method.

**Tested how:** Read the existing 22-module coverage, export scripts, catalogue and gamedb audit documentation; enumerated installed DLL/EXE files read-only. Checked document consistency, Git diff and the public snapshot.

**Result:** Handoff records 87,783 successful exports/ten failures, exact-byte catalogue coverage, gamedb omissions/read-range errors and 126 installed DLL/EXE paths awaiting scope/dependency classification. The plan separates inventory, failure recovery, discovery, type/behavior recovery and verified Rust translation.

**Still broken / not tested:** No new Ghidra run, recovery/export, database rebuild or runtime modification occurred. Existing audit reports were inspected, not rerun. Complete decompilation, function discovery and semantic parity remain unverified; the failed exports and parser discrepancies remain open.

**Next:** The owner starts a fresh chat in outputs/hl2-rs-bevy. Read AGENTS.md, STATUS.md, docs/DESIGN.md and the newest private checkpoint, then execute the bounded decompilation plan before queued NPC gesture work.

## 2026-10-05 Fresh-chat documentation handoff

**Agent/model:** not recorded.

**Changed:** Reworked AGENTS.md, added STATUS.md and docs/DESIGN.md, updated this log and changed .gitignore to a source-only whitelist.

**Why:** Preserve verified state, existing authorization and failed approaches without rereading the entire chat.

**Tested how:** Reviewed the three templates against repository instructions, Git revisions, build metadata, recorded logs and the private checkpoint. Checked links, whitelist coverage/exclusions and the staged public snapshot.

**Result:** Handoff identifies active branches, both release packages, tested capabilities, unresolved fidelity work and the next bounded gesture plan. Historical entries remain available.

**Still broken / not tested:** Runtime code is unchanged; gameplay/build checks belong to28c0bb3 and are not rerun claims for this documentation edit. Full campaign/1:1 parity remains unfinished.

**Next:** Start a fresh chat in the Bevy checkout; read AGENTS.md, STATUS.md and docs/DESIGN.md before continuing.

## 2026-10-05 Native crosshairs and optional spatial batches —28c0bb3

**Agent/model:** not recorded.

**Changed:** Shared owned Windows glyph rasterization/placement, Bevy1080p/borderless launchers, optional spatial batches/render-candidate diagnostics and targeted retained build tooling.

**Why:** Correct blur/fractional reticles and investigate broad world batches without changing physics or forcing equal native sizes.

**Tested how:** Ran original HL2 at1080p,294 unit tests/19 owned checks, strict Clippy/fmt, packaged captures/replays and source audit.

**Result:** Seven Bevy captures and retained1080 captures match dot offsets: white22x22, pistol/SMG21x17. Movement26, weapons17, monitor/campaign18 and accepted images9 pass. Plaza/spawn partition images are identical; audit212 files/0 failures.

**Still broken / not tested:** Splitting stays opt-in; hall camera was rejected, frozen benchmark script was incomplete and25 known station03 material errors remain. Other fonts/colors/platforms and full campaign parity are not established.

**Next:** Preserve verified reticles and reconstruct authored NPC gestures.

## 2026-10-05 Shared materials and model activity —c2b5220 /322afaa

**Agent/model:** not recorded.

**Changed:** Shared authored material/lightmap handles and avoided unchanged GPU uploads; added owned label/activity/weight resolution and corrected unsupported Kleiner idle.

**Why:** Reduce preparation overhead and use model-authored animations instead of assumed labels.

**Tested how:** Unit/owned checks, controlled Kleiner capture, movement/weapon replays, matched actor/eye/monitor comparisons and local1080 profiles.

**Result:** Station02 material handles921→743. Kleiner resolves idle_subtle; valid metrocop idle_baton stays. Material package measured174.72FPS at the tested spawn, with limited scope.

**Still broken / not tested:** Native RNG/modifiers/full virtual IDs, gestures/facial animation and natural staging remain unfinished. Global eye reports differ after the intervening idle fix.

**Next:** Preserve raw delta/post transforms, masks and child dependencies before gestures.

## 2026-10-05 GPU skinning and cached BSP PVS —df6af1b /b888144

**Agent/model:** not recorded.

**Changed:** GPU skinning/current bounds/shared iris poses; conservative PVS across active player/monitor views with script visibility/fail-open behavior.

**Why:** Profiling found CPU skinning/uploads and preparation of out-of-view geometry.

**Tested how:** Synthetic/owned bounds/visibility checks, packaged actor/eye/monitor/campaign/gameplay fixtures and900-frame1080 profiles.

**Result:** Selected actor/eye comparisons preserve tested scope. CPU→GPU35.05→66.85FPS and PVS-off→on66.69→156.94 are separate local comparisons; simulation remains enabled.

**Still broken / not tested:** Area portals/occluders/LODs, complete animation and whole-campaign/native144FPS parity remain unverified.

**Next:** Preserve current bounds/view-union ordering and expand fidelity with the same regressions.

## 2026-10-05 - animated attention attachments

**Agent/model:** not recorded.

Actor gaze origin/forward now come from the owned animated eyes attachment, with model view-offset fallback. Eye presentation caches one skin pose per visible actor and shares it with iris projection, preserving cycler meshes. Malformed/singular/overflowing attachment records are rejected; PVS latching, native head-pose controls and full layered animation remain unfinished.


## 2026-10-05 - authored scene attention

**Agent/model:** not recorded.

Shared simulation now executes compiled LOOKAT events with normalized-time scene/event ramps, timed deduplicated interests, target aliases, pause refresh and cancellation expiry. Bevy projects both eyes toward one selected actor target; monitor/player/self behavior is capture-verified in a deliberately seeded security02 fixture. Retained simulation/reporting consumes the same queue. Head/facial controls and native random/tactical attention remain unfinished; this is not campaign or retail AI parity.


## 2026-10-05: shared pause/console and Bevy campaign host

**Agent/model:** not recorded.

Moved the retained console parser/history/cheat gates and resource-driven pause layout into hl2-ui. Both hosts now provide explicit input and consume the same canvas. Bevy opens Resume/Console/Quit, supports the existing command subset and preserves selection/capture/held-input ordering. Console rendering clips output to its panel and handles Unicode character boundaries.

Added asynchronous owned-map decoding with simulation/script clocks frozen while loading. Successful transitions replace map-owned draws/audio and carry landmark-relative eye position/inventory with rebased weapon deadlines; direct map starts fresh. Missing maps preserve the current level. Testing exposed repeated same-map loads from input-only changelevel brushes; shared touch handling now honors their 0x2 flag while retaining explicit ChangeLevel input. Full saved entity/global/player state and ordinary campaign completion remain unfinished.

267 normal tests, thirteen owned-install tests and strict Clippy pass. Packaged console/campaign captures pass 45 checks, including 720p/borderless1080p UI, actual paused audio sinks, two real station exit transitions, failure recovery and camera cleanup. Retained-host and weapon/movement regressions are recorded in validation.md. No game files or native research are published.


## 2026-10-05: shared effects presented in Bevy

**Agent/model:** not recorded.

Extracted retained projectile billboard/RNG/blur and impact selection/clipping into hl2-simulation, leaving rendering adapters in each host. Bevy now presents owned SMG grenades, energy balls, impact/explosion sprites and bullet marks with depth testing. Decals retain destination-color blending and doubled modulation, and follow moving receiver transforms. Testing found a retained door-mark failure: projection used raw MDL vertices instead of the animated idle pose and rejected the one-unit collision/visual gap. Both hosts now project onto the current posed mesh with a bounded same-receiver correction. Native studio deformation and complete particles remain unfinished.

Packaged effect fixtures pass 40 checks, including real observed draws, frozen simulation, secondary reserves, explosion completion and marked-door transform agreement. The full combined weapon fixture and retained packaged smoke are checked separately. Source assets stay installed and private capture evidence stays ignored.

## 2026-10-05: shared HUD presentation in Bevy

**Agent/model:** not recorded.

Moved the retained owned-resource HUD into engine-independent hl2-ui with an explicit ordered CPU canvas. Both renderers use the same layout/font/crosshair/animation logic. Bevy now draws health/ammo, weapon buckets, quick-info and secondary ammo through a separate overlay camera with normal/additive materials. Asset loading stays outside systems; glyph textures and meshes are reused.

Packaged white-crosshair captures match the accepted retail five-pixel positions at 720/1080. Selection preserves active ammo, secondary-panel motion is captured at start/intermediate/end, and the door/weapon and bench fixtures remain passing. Native font rasterization and blend/gamma equivalence are still partial. Audio, pause/console, projectile/impact presentation and campaign host migration remain open.


## 2026-10-04: Bevy entity, weapon and animated presentation bridge

**Agent/model:** not recorded.

Moved the tested entities/gameplay/NPC/projectile/selection implementations into hl2-simulation and retained host re-exports. Shared actor/viewmodel preparation preserves clip loading. Bevy now runs scene, weapon, moving collider, NPC/projectile, rigid-body and player state in retained order. Local entity meshes follow authoritative poses/visibility; CPU skeletal animation updates bounds; an independent viewmodel pass preserves the existing projection. F3, weapon buckets/wheel, confirming/fire/reload/previous, E use and G test impulse are connected. Quick-click and pause/selection suppression regressions pass. Packaged bench and station-door/secondary-weapon fixtures pass; HUD/audio/effects/campaign host migration remains next.


## 2026-10-04: shared collision and Bevy player movement

**Agent/model:** not recorded.

Extracted the existing collision/Rapier adapter, convex sweeps and NPC probes into `hl2-simulation` unchanged, preserving their 25 tests and retained-runtime imports. Bevy now uses the same Source-coordinate player and collision code at 15 ms per step, with input/look before simulation and camera presentation afterward. Default walking, flight toggle and pause/resume are supported; input capture consumes transition-frame mouse movement and held jump until release.

The packaged bench fixture passes 26 movement/pause assertions, including the native collision height, one-time air crouch lift with retained eye/momentum, air uncrouching and jump rearming. Rendering remains static and rigid bodies are frozen until presentation is synchronized. Weapons/HUD, pause UI/console, entity I/O/scenes, NPC animation/AI and audio remain migration work. See `docs/validation.md` for package and test evidence.

## 2026-10-04: separate Bevy/wgpu host preview

**Agent/model:** not recorded.

Preserved the original host on `macroquad-prototype` and added `bevy-migration` for the long-term Bevy/wgpu work. Main keeps the retained runtime until verified replacements are available. README and contributor guidance identify the branch and executable each feature belongs to.

The new host reads owned maps through the shared format/core crates and renders static BSP, displacement, prop and entity geometry with custom base/lightmap materials and a fly camera. Shared CPU/GPU texture caching and visible-material filtering correct the first preview's loader-budget exhaustion. BSP render winding is normalized before appending already-normalized models, correcting culled model fronts without altering shared collision data.

Validation: 241 normal tests and nine owned-install checks pass, with formatting and strict Clippy. Final packaged station-map captures at 720p and borderless 1080p were inspected, with no texture-budget or capture failures. Dynamic monitor and eye materials remain unsupported. Gameplay, collision, animation, AI, choreography, HUD and audio have not migrated; Source rendering and performance parity are not established. See `docs/bevy-migration.md` and `docs/validation.md`.

## 2026-10-04: G-Man speech and authored locomotion data

**Agent/model:** not recorded.

Fixed the first-map G-Man omission with explicit `cycler_actor` model and scene-actor support, corroborated by retail factory/RTTI and actor lookup. The packaged debug-camera fixture resolves 17 authored intro events. Its initial run exposed compressed voice failures; standard Microsoft ADPCM now decodes in memory, preserves recorded frame counts and successfully requests playback of both opening lines. Facial animation, gestures, intro cameras and compositing remain unfinished.

Added bounded PC AIN37 decoding and optional inspect/runtime reports, preserving hull offsets, raw masks/metadata and Hammer IDs. Added v48 authored movement records and activity/all-blend metadata, plus piecewise motion sampling, turning and positive/negative loops. Owned walk/run tracks match 80 units/1s and 125.87412 units/0.6s. These readers do not yet implement NPC pathfinding, weighted blending, motor planning or movement readiness.

Validation: 210 workspace tests, six owned-install checks, formatting and strict Clippy pass. The rebuilt launcher package passes 12 G-Man, 33 scene-controller/sequence and 20 secondary-projectile assertions. A graph census decodes 72 and rejects six mismatched revisions across 78 loadable installed maps; the known empty coast map remains rejected. Private evidence and game files remain outside public source.

## 2026-10-04: crosshair, secondary projectiles and authored scenes

**Agent/model:** not recorded.

Corrected the oversized unarmed crosshair by identifying the native height accessor and reproducing both texture-coordinate insets. Packaged720/1080 captures now match the original five pixel positions; odd viewport rounding also passes. Font rasterization and tone mapping remain separate work.

Added real SMG contact grenades and AR2 charging balls, separate cooldowns/reserves, reload interruption/veto, continuous collision queries, bounded blast obstruction/damage, bounce/expiry and owned effects. Native controls corroborate selected weapon-state behavior. Rapier physics, damage policy and effects still do not reproduce the full Source implementation.

Added a bounded Rust reader for compiled choreography and authored scene control/triggers/completion. Selected installed SEQUENCE clips follow the paused scene clock and restore their baseline. Unsupported movement readiness holds SECTION rather than inventing success. The first level still lacks NPC schedules/movement, gesture/facial layers and intro systems needed for complete playback.

MP3 cues decode into an in-memory PCM cache, resolving the trainstation music gap. Added bounded cheat-gated ent_fire through normal entity I/O and more detailed snapshots.

Validation:192 workspace tests, both owned-install checks, strict Clippy and formatting pass. The latest package passes27 existing weapon checks,20 projectile checks and33 first-map scene-controller/sample checks. Captures and limitations are recorded in docs/validation.md; private research and installed assets remain excluded from Git.

## 2026-10-03: air crouch, stable contacts and pause/console

**Agent/model:** not recorded.

Air crouching now tucks the feet once while preserving head height and momentum; standing clearance gates air unducking. Reviewed retail sliding rules and analytical world-brush/convex sweeps correct reproduced wall-contact jump interruptions. Native PHY compound integration and full VPhysics parity remain unfinished.

Added a working resource-based pause menu and bounded developer console with cheat gates, history/editing/completion, position/angle commands and owned map loading. Focus freezes simulation and suppresses attack/jump leakage. The menu uses the owned title font and calibrated 720p metrics. Full console, save/options and paused audio remain open.

139 tests and strict Clippy pass; packaged weapon, movement and console fixtures pass their meaningful assertions. Latest double-door collision and secondary-ammo HUD reports are queued for the next iteration. Private bench PHY research matches the measured original standing height but is not integrated into Rust yet.

## 2026-10-03: publication and automatic empty fire

**Agent/model:** not recorded.

Published the reviewed Rust source to kvalls/hl2-rs with fresh-checkout instructions, contribution boundaries and a labeled runtime screenshot. The public main branch starts from a clean source snapshot; older local research history stays private. Windows formatting, Clippy and unit checks run for main pushes and contributor pull requests.

SMG1/AR2 now use a reviewed empty-fire latch and per-weapon half-second sound throttle. Empty clicks retain the primary deadline and animation; the next eligible attempt reloads, and idle reload uses a strictly elapsed primary deadline. Four regression tests cover held/released input, throttle boundaries, independent weapon state and reload completion. Other secondary attacks, autoswitch ranking and custom reload flags remain incomplete.

## 2026-10-03: depth, secondary fire and retail input/movement

**Agent/model:** not recorded.

Separated OpenGL depth testing and writes in the vendored backend, preserving depth clears between sky/world/viewmodel passes. Matching trainstation views reproduce then remove hidden light shafts, entrance columns and barrier effects; a front-side capture retains the visible columns.

Added shotgun alternate fire with twelve pellets/two shells, one-shell primary fallback, native/script-derived pump deadlines and retained secondary reload interruption. Accepted health/battery pickups now use their installed item sounds. The original engine confirms a six-to-four-shell alternate discharge.

Selection now consumes already-held attack buttons when Slot/Wheel opens the menu, rearms each button independently, and observes ordered script release/repress transitions. The F1 overlay identifies the Rust runtime. Movement now preserves standing/duck jump gravity ordering, categorizes after the sweep, crops diagonal boost, permits negative speed-cap additions and retains the native next-tick rising-air friction factor.

Fresh-clone build instructions and contribution boundaries were added for public source sharing. Game assets, native analysis, captures and research tools remain separate. Full campaign, NPC AI and Source rendering/collision parity remain unfinished; docs/validation.md records the checks and package fingerprint.

## 2026-10-02

**Agent/model:** not recorded.

The original executable remains read-only, with the earlier save/config snapshots preserved. Work is focused on HL2 fidelity; no FAL assets or crossovers were added.

The first prototype supported map/prop display. This iteration adds brush submodels, displacement/prop collision, fixed-step movement, timed entity I/O and doors, primary lightmap atlases, material transparency/tint/two-texture scrolling, compressed skeletal animation, PCM WAV audio, crowbar/pistol gameplay and limited scripted sequences/map transitions. These are partial implementations; the README lists the missing systems.

Mouse-look uses macroquad's previous-minus-current delta: upward movement increases pitch, rightward movement decreases Source yaw. The Windows miniquad reader additionally converts absolute RAWINPUT positions into deltas; treating those positions as motion caused extreme jumps during automation. Interactive verification is recorded in docs/validation.md.

Regression tests cover ordered/delayed duplicate outputs and fire limits, door locking/completion, shuffle batches, post-idle completion timing, ammunition transfer, stationary hull overlap at a floor, and disabling killed dynamic colliders. Movement and animation tests cover deterministic replay, acceleration/friction, jump apex and hierarchy/interpolation.

Research tools and private decompiler output belong in ../../work/hl2-decompiled. They are development references only. The runtime builds from Rust and reads owned assets in place. The packaged executable, rather than target/debug, is the final launch.cmd validation target.


### 2026-10-02: weapon selector, impacts and separate sky world

**Agent/model:** not recorded.

- Preserve nested sound-script wave alternatives; use distinct crowbar flesh/world sounds and hit animation.
- Add receiver-clipped installed impact textures and inherited surface-property bullet sounds. Marks follow rigid props and brush entities; keep a bounded local pool.
- Implement six-bucket UI with installed weapon icons, wheel cycling, confirmation, cancellation and previous-weapon switching; cancel reload on switch. Only crowbar and pistol combat are implemented.
- Separate background geometry/props using BSP leaf/PVS data and sky-camera scale. No native game code or decompiler tooling entered this project.
- Validation: 36/36 unit tests and workspace Clippy pass; packaged interactive run recorded three hit decals and multiple metal-impact sound variants with zero audio errors. Wheel UI rendered correctly; live confirmation needs a faster-input retest because its timeout expired between automation calls. Installed maps 78/79 parse with only the known empty coast map failure.

### 2026-10-02: retail HUD, six primary weapons and sky visibility

**Agent/model:** not recorded.

The earlier selector was a provisional design. This pass replaces it with installed resource dimensions, fonts/cell metrics, corners, labels and input behavior, checked against selected retail methods and a localized original-engine capture. Numeric HUD animation rules, bounded low-health loops and QuickInfo progress/fades/warning audio now run. Native GDI rasterization, damage messages and the remaining HUD panels/gates are still incomplete.

Added .357, SMG1, AR2 and shotgun primaries, separate ammo reserves, native/script-derived cadence/spread, shotgun shell/pump/interruption behavior and installed model sound events. Crowbar collision now tries the full ray, shortened +/-16 hull, facing test and closest corner refinement over approximate Rapier geometry. The background renderer now checks the camera BSP leaf's 3D-sky flag.

Validation: 79/79 workspace tests, strict Clippy, formatting and diff checks pass. Packaged input regression records 64 attacks, 82 impacts/marks, 6 model sound events, 199 audio requests, zero audio errors, two crowbar impact variants and one low-ammo warning. Actual Computer Use wheel/left/right/Q/Escape input verified selection with zero shots. A 60-frame hidden-sky test recorded zero sky frames. Installed maps remain 78/79, with the existing empty coast map rejected. See docs/validation.md for artifacts, current package fingerprint and limits.

Original-game research writes, localization copy, inspection tools and native pseudocode stay outside the Rust project. The original test process was closed, stock cfg manifest unchanged, no native DLL patched and no FAL/crossover used. Full NPC/campaign/shader fidelity remains substantial work; parser coverage is not campaign completion.

### LDR sky asset groundwork

**Agent/model:** not recorded.

Added a bounded six-face LDR sky loader resolving VMT base textures and transforms, with eight synthetic tests. HDR rendering, cube drawing and leaf gating integration remain unfinished. All 109 workspace tests and strict Clippy passed; the final package was rebuilt and smoke-tested.

### 2026-10-03: database audit, sky rendering and movement collision

**Agent/model:** not recorded.

Integrated the six-face LDR sky background with verified retail orientation, installed material transforms, clamped sampling and separate BSP 2D/3D eligibility. It draws before scenery/world without depth writes. Matched original-engine views corroborate cloud orientation; HDR, fog and sky polygon masks remain unfinished.

Player collision now excludes retained NPC-only world clip brushes, correcting the oversized invisible station-bench blockers. Grounded crouch commands and maximum speed are separated for jump boost; airborne crouch retains full air acceleration. Regression checks cover clip masks, duck boost caps, backward overspeed, air strafing and chained released jumps. Native PHY geometry and full movement equivalence remain open.

The supplied Database Method was tested privately across all exports. Unchanged gamedb misses functions and some boundaries/call targets, so an exact-byte catalogue keyed by module/address preserves the complete export inventory and known failures alongside the supplemental navigator. This improves traceability without marking semantic parity automatically. The original game also confirmed three-shell shotgun secondary behavior: double discharge leaves one shell, and the next secondary action fires it as a single shot. See research and validation for measured coverage and packaged checks.

### 2026-10-03: air crouch, console, static PHY and secondary HUD

**Agent/model:** not recorded.

Air crouching preserves head height with a one-time foot lift, and stable world/convex hull sweeps correct reproduced wall-jump contacts. Added a resource-driven pause menu and bounded console with cheat gates, history, completion and map loading. Original ground-duck timers, prediction and full Source command/UI coverage remain unfinished.

Added a bounded original Rust PHY reader and separate installed convex pieces for supported solid static props. Bench standing height now agrees with the owned original measurement. Rotating-door collision applies the visible model's initial idle pose, fixing the station entrance's closed-door walkthrough and invisible open-shaped blocker.

Primary ammo stays visible during weapon selection. Unarmed crosshairs use the installed white default sprite; armed crosshairs retain installed glyphs. SMG/AR2 secondary reserves, carry limits, pickups and ALT counters now work. Their actual grenade/energy-ball attacks and the remaining weapons are still missing. See validation for package fingerprints, tests and the known trainstation MP3 music gap.

### 2026-10-04: Barney scene movement and actor-aware speech

**Agent/model:** not recorded.

Added Barney's native default model and owned MDL eye metadata, dedicated NPC collision masks/probes, bounded human ground routes and authored central walk/run locomotion. Scene MOVETO requests now persist while SECTION waits for actual arrival; the controller continues while only the scene clock is paused. UI pause freezes both, blocked routes hold the gate, and cancel/script ownership prevents stale pose changes. The controlled security03 fixture verifies player blocking, arrival-driven door output and cancellation without a manual scene Resume. It explicitly seeds a preceding authored target and is not a complete campaign replay.

Symbolic speech carries the resolved actor model through the installed gender registry and retains native tagged wave/fallback behavior. The first-map owned census resolves all 72 actors and decodes all 75 registered alternatives. Native RNG/mixing, localized combined lines and lip sync remain open. See validation for tests and packaged checks; native motor timing, weighted blends, general NPC AI, facial/eye rendering and the playable first level remain unfinished.

### Bevy audio adapter

**Agent/model:** not recorded.

Shared owned sound-script selection, actor gender, WAV/MS ADPCM/MP3 decoding and viewmodel event cursor now serve both hosts. Bevy preloads references outside systems and plays ambient/scene/weapon/HUD requests through monitored audio sinks, including host pause/resume. Packaged door/weapon and controlled scene fixtures, full tests and strict Clippy passed. Source spatial audio/DSP, soundscapes and lipsync remain unfinished; this does not complete the campaign.

### Bevy sky, iris projection and prop door swing

**Agent/model:** not recorded.

Restored owned LDR cube/miniature sky passes before the playable world, with current-leaf visibility and depth occlusion. Eyes now use authored studio metadata and separate iris textures; gaze uses a bounded visible player/NPC approximation. EyeRefract uses its owned Eyes_dx8 fallback, with refraction/flex/glints still missing. Prop door use carries the opener position to linked leaves and respects explicit swing direction, fixing the opposite inward/outward entrance behavior. Native door blockers and full gaze/choreography logic remain unfinished.

## Deferred prop-door opener inputs

**Agent/model:** not recorded.

Added shared OpenAwayFrom target resolution, preserved locks/fixed directions and stopped repeated opening inputs from resetting the swing. Two regression tests and an owned packaged entrance fixture cover named/current origins, player/caller/activator lookup and both sides. Full native door linkage/blocking remains unfinished.
