# Lighting follow-up, 2026-10-26

## Ordinary shadows and local lamps

The reported global/vertical direction is the normal HL2 shadow policy, rather than a
request to aim every ordinary prop/NPC shadow away from each nearby lamp:

- Low/simple shadows project straight down. SDK `game/client/clientshadowmgr.cpp`
  `GetShadowDirection` explicitly returns (0, 0, -1) when render-to-texture is inactive.
- Medium/High ordinary shadows use the map's `shadow_control` direction (or the level
  default). This vector is independent of each local baked emitter; it need not represent
  the sun. `SetShadowDirection` normalizes one map-wide vector and refreshes the shadows.
- High additionally casts flashlight depth shadows from the placed flashlight. This is a
  separate light-relative shadow, so the flashlight and ordinary projection can coexist.
- The SDK allows an entity-specific direction override. `C_BaseEntity` normally declines
  it, or delegates to an authored shadow-direction entity. Special entity overrides and
  live `shadow_control` input changes are not implemented/verified by this report.

Installed-client evidence: the exact `r_shadowdir` registrar binds its callback; that
callback selects either the fixed downward vector or the stored global direction based
on the render-to-texture mode. Its setter copies/normalizes one vector and updates shadows
when that mode is active. Exports were retrieved from the frozen index by full installed
module identity, dataset/variant and body hash, and reviewed privately. This confirms the
mode/global-vector contract; a full native scene comparison and all entity overrides
are **not tested**. SDK references: `GetShadowDirection`, `SetShadowDirection` and
`game/client/c_baseentity.cpp` `GetShadowCastDirection`.

HL2-RS already follows this ordinary direction policy in `hl2-simulation::shadows` and
`hl2-bevy::shadows`. No nearest-local-light direction change is made. Visible emitter
brightness is addressed independently below; paper receiver policy is described next.

The owned `d1_canals_01` map supplies direction approximately
(-0.1677313, 0.0449435, -0.9848077), colour (128,128,128) and cast distance 75.
This is authored map direction, not a nearest-lamp query. The 15 existing shadow-policy
unit tests pass; no runtime code is changed by this report checkpoint.

## Paper and junk at the canal corridor

The sheets inspected near the start corridor are the static model
`models/props_junk/garbage256_composite002b.mdl` (origin approximately 631,-8107,-124),
using the garbage002a/garbage003a materials. They are not BSP overlays. The baseline
packaged capture shows these sheets; the owned-map test verifies their baked static-prop
receiver identity.

SDK `game/client/clientshadowmgr.cpp` `AddShadowToReceiver`, static-prop branch:
ordinary prop/blob shadows are excluded. Render-to-texture shadows may reach static props
when cast by an NPC/player; flashlight projections have a separate path. HL2-RS follows
this policy. Its new owned regression projects onto the actual paper geometry: no
ordinary-prop receiver triangles, 17 receiver triangles with NPC-style reception enabled.
A faint flashlight shadow affects only the added beam, preserving the baked illumination.

No nearest-light or static-prop receiver policy is changed. An NPC Medium/High failure on
these papers has not been reproduced visually, and a native visual comparison is
**not tested**. This result does not establish that all BSP decals/overlays composite
correctly: their depth offset was an initial hypothesis, not the cause of these sheets.
The owner confirmed a physics object at Medium/High: this is the SDK-excluded ordinary
prop-to-static-prop case, so no receiver-policy change is made.

## Fluorescent fixture emission

The corridor model `models/props_interiors/lights_florescent01a.mdl` selects its installed
`FlorescentLight01a_on` material. Its VMT requests `$selfillum 1`; the base texture's alpha
is the emissive mask. The material was decoded correctly. Our shader multiplied its
emissive albedo by the static prop's baked vertex illumination, making the lit tubes dark.
The corrected path uses texture and material tint independently of baked illumination,
then applies the installed alpha mask. Unmasked fixture/wall pixels retain their lighting.
No replacement asset, extra light, glow sprite or render pass is introduced.

Source SDK `materialsystem/stdshaders/vertexlitgeneric_dx9_helper.cpp` selects the
self-illumination shader combination separately from vertex lighting. A bounded exact
installed shader export confirms an alternate envmap-alpha mask parameter; that parameter
is absent from this fluorescent VMT. This is parameter evidence, not native pixel-output
parity. The packaged regression compares the corrected shader with the previous baked-
albedo expression in a disposable copy and restores the shader in `finally`.

Accepted front-on camera: eye 607.53,-8173.45,-25.545, yaw/pitch 0. The earlier angled
capture viewed the edge/inside of the wall and was rejected as emission proof; retained
alongside the accepted captures. Native side-by-side, every lamp material, bloom and
Windows runtime remain **not tested**.

## Flashlight availability and player lifetime

Activation requires an equipped suit and an enabled flashlight. It does not require a
weapon. HL2/Episode One/ Lost Coast use the suit device; Episode Two uses its separate
battery, with the same suit gate. The installed server's exact on function confirms the
disabled flag and suit checks, legacy-device path, sound and output dispatch; its exact
off function confirms device removal, light-flag clearing and sound/output dispatch.
SDK references: `game/server/hl2/hl2_player.cpp` FlashlightTurnOn/Off and Enable/Disable
inputs; `game/server/player.cpp` Event_Killed, RemoveSuit and PostThink;
`game/shared/baseplayer_shared.cpp` ItemPostFrame.

Death switches an active light off; normal impulses are not processed for a dead player.
DisableFlashlight extinguishes and blocks activation. EnableFlashlight restores permission
without switching it on. Removing the suit only clears the wearing-suit flag in the SDK:
an active light is not forcibly extinguished by that operation, but cannot be reactivated
without the suit. The existing battery/cutoff/recharge behavior is retained.

The simulation now owns the player flashlight controller and deferred permission inputs.
Registered player inputs use the local-player sentinel; `!player` resolves to the player
independently of a relay's activator. The host supplies suit, game/skill, alive state,
impulses and frame time, then presents sounds and light. Campaign changelevel transfers
the controller with the player and rebases recharge timing. Direct map loads create a
fresh controller. This corrects the old host resource's game-name-only lifetime, which
could leak enabled/on/power state into a fresh map or respawn.

Unit tests cover suit acquisition/removal, both battery modes, ordered permission requests,
death/dead impulses, pause, delayed recharge across a clock reset, first-frame Episode Two
permission input, relay-activator independence, invalid-target rejection and campaign transfer. Actual suit removal
is not yet exposed through a gameplay entity input in HL2-RS. Native visual comparison,
all vehicle/control-lock cases, player-proxy flashlight output integration, save/restore,
Windows runtime and an ordinary campaign playthrough remain **not tested**. The new
Enable/Disable wrappers and lifetime behavior have SDK evidence; only the named on/off
functions received new exact-native review in this checkpoint.
