//! Deterministic map entity I/O. Unsupported inputs are reported instead of silently emulated.
use crate::physics;
use glam::{Quat, Vec3};
use modkit_core::{parse_vec3, Entity, World};
use serde::Serialize;
use source_assets::scenes::ChoreoScene;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

mod choreography;
mod color_correction;
mod inputs;
mod io;
mod movers;
mod parenting;
mod sequences;
mod triggers;

/// Entity classes that move with VPhysics (MOVETYPE_VPHYSICS) for trigger touches.
fn is_vphysics_class(class: &str) -> bool {
    class.starts_with("prop_physics")
        || class.starts_with("weapon_")
        || class.starts_with("item_")
        || matches!(
            class,
            "physics_prop" | "func_physbox" | "func_physbox_multiplayer" | "prop_ragdoll"
        )
}
pub enum SceneMoveCommand {
    Start {
        key: crate::npc::GoalKey,
        request: crate::npc::MoveRequest,
    },
    CancelScene(usize),
}

#[derive(Clone)]
struct MovementAnimation {
    key: crate::npc::GoalKey,
    sequence: String,
    elapsed: f32,
    /// Path direction relative to the turning body (move_yaw pose parameter).
    move_yaw: f32,
    previous: String,
    previous_started: f64,
    previous_done: Option<f64>,
}

enum ScenePause {
    Input,
    Section {
        blocked: bool,
        automated: Option<(bool, f64)>,
    },
    UnsupportedControl,
}
struct Playback {
    elapsed: f64,
    next: usize,
    completed_early: bool,
    pause: Option<ScenePause>,
    actors: Vec<Option<usize>>,
    look_targets: BTreeMap<usize, crate::attention::Target>,
    /// FACE event targets and (initial yaw, actor was moving) latched on first process.
    face_targets: BTreeMap<usize, crate::attention::Target>,
    face_initial: BTreeMap<usize, (f32, bool)>,
}
struct Choreography {
    data: Arc<ChoreoScene>,
    order: Vec<usize>,
    playback: Option<Playback>,
}
#[derive(Clone)]
struct SceneAnimation {
    owner: usize,
    event: usize,
    previous: String,
    previous_started: f64,
    previous_done: Option<f64>,
}
fn number(e: &Entity, key: &str, default: f32) -> f32 {
    e.get(key)
        .and_then(|s| s.parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(default)
}
/// `!target1`..`!target8` name the scene entity's targetN keys.
fn resolve_target_slot<'a>(world: &'a World, scene: usize, name: &'a str) -> &'a str {
    if let Some(slot) = name
        .strip_prefix('!')
        .and_then(|s| s.to_lowercase().strip_prefix("target").map(str::to_owned))
        .and_then(|s| s.parse::<usize>().ok())
    {
        if (1..=8).contains(&slot) {
            return world.entities[scene]
                .get(&format!("target{slot}"))
                .unwrap_or("");
        }
    }
    name
}
fn scene_actor_class(class: &str) -> bool {
    class.starts_with("npc_")
        || class.starts_with("prop_dynamic")
        // Retail cycler_actor is a CFlextalkActor, whose native hierarchy includes CBaseFlex.
        || class == "cycler_actor"
}
#[derive(Clone)]
struct Output {
    name: String,
    target: String,
    input: String,
    parameter: String,
    delay: f64,
    remaining: i32,
}
fn output(name: &str, text: &str) -> Option<Output> {
    let separator = if text.contains('\x1b') { '\x1b' } else { ',' };
    let values: Vec<_> = text.split(separator).collect();
    if values.len() != 5 {
        return None;
    }
    let delay = values[3].parse::<f64>().ok()?;
    if !delay.is_finite() || delay < 0. {
        return None;
    }
    Some(Output {
        name: name.into(),
        target: values[0].into(),
        input: values[1].into(),
        parameter: values[2].into(),
        delay,
        remaining: values[4].parse().ok()?,
    })
}
#[derive(Clone)]
struct Pending {
    due: f64,
    sequence: u64,
    target: String,
    input: String,
    parameter: String,
    caller: usize,
    activator: usize,
    opener: Option<Vec3>,
}
#[derive(Clone)]
pub struct State {
    pub origin: Vec3,
    pub rotation: Quat,
    pub visible: bool,
    pub enabled: bool,
    pub killed: bool,
    pub animation: String,
    pub default_animation: String,
    pub animation_started: f64,
    animation_done: Option<f64>,
    scene_animation: Option<SceneAnimation>,
    movement_animation: Option<MovementAnimation>,
    /// The AI motor's facing target while moving (AddFacingTarget: point, expiry); the
    /// body turns to it and move_yaw keeps the legs on the path.
    pub ai_move_facing: Option<(Vec3, f64)>,
    script_actor: Option<usize>,
    pub scripted_by: Option<usize>,
    /// The scripted_sequence whose post idle holds this actor in NPC_STATE_SCRIPT
    /// (SDK SCRIPT_POST_IDLE: m_hCine stays set until PostIdleDone's CineCleanup; with
    /// SF_SCRIPT_LOOP_IN_POST_IDLE that is never, until cancelled or another script
    /// takes the actor). Only the random look queue reads it; scene ownership stays
    /// with `scripted_by`.
    pub script_post_idle: Option<usize>,
    script_finish: Option<f64>,
    /// A scripted_sequence's walk/run to its mark (SCHED_SCRIPTED_WALK/RUN) in progress.
    script_move: Option<crate::npc::GoalKey>,
    post_idle_done: Option<(f64, usize)>,
    shuffle: Vec<usize>,
    last_shuffle: Option<usize>,
    base_origin: Vec3,
    base_rotation: Quat,
    translation: Vec3,
    angle: f32,
    axis: Vec3,
    fraction: f32,
    target: f32,
    rate: f32,
    pub locked: bool,
    return_at: Option<f64>,
    wait: f64,
    touching: bool,
    last_trigger: f64,
    pub value: f32,
    timer_at: f64,
    outputs: Vec<Output>,
    /// Movement hierarchy parent (CBaseEntity::SetParent), optionally an attachment.
    pub parent: Option<Parent>,
}
impl State {
    /// The actor plays authored locomotion (IsMoving for look-target choice).
    pub fn is_moving(&self) -> bool {
        self.movement_animation.is_some()
    }
    /// Whether the entity's collider takes part in queries. Gear parented to an animated
    /// attachment (helmets on NPCs) moves with its parent's hierarchy and must not block
    /// that parent; native traces skip hierarchy children of the moving entity.
    pub fn collides(&self) -> bool {
        !self.killed && self.visible && self.parent.as_ref().is_none_or(|p| p.attachment.is_none())
    }
}
/// A child follows its parent's origin/rotation, or one of its animated attachments,
/// with a fixed local offset.
#[derive(Clone, Debug, PartialEq)]
pub struct Parent {
    pub entity: usize,
    pub attachment: Option<String>,
    pub origin: Vec3,
    pub rotation: Quat,
}
/// Classes whose own movement code positions them; map-spawn `parentname` is not
/// applied to them here (moving hierarchies remain unfinished).
fn self_positioned(class: &str) -> bool {
    ["func_", "prop_door", "prop_physics", "npc_", "trigger_"]
        .iter()
        .any(|p| class.starts_with(p))
}
/// The local player in trigger_hurt toucher lists.
const PLAYER: usize = usize::MAX;
#[derive(Clone, Debug)]
struct HurtState {
    /// m_flDamage (grows under damagemodel 1).
    damage: f32,
    /// Next HurtThink; None when the think is cleared.
    next: Option<f64>,
    reset_at: f64,
    /// m_hurtEntities: hurt by the last HurtAllTouchers (PLAYER is the player).
    hurt: Vec<usize>,
    /// Touchers seen in the previous tick, for EndTouch.
    touching: Vec<usize>,
}
#[derive(Default, Serialize)]
pub struct Diagnostics {
    pub inputs_delivered: u64,
    pub outputs_fired: u64,
    pub trigger_entries: u64,
    pub door_completions: u64,
    pub unsupported: BTreeMap<String, u64>,
    pub malformed_outputs: usize,
    pub budget_exhaustions: u64,
    pub scenes_loaded: usize,
    pub scene_events_started: u64,
    pub scene_completions: u64,
    /// Scene captions dropped by the caption radius (AttenuateCaption).
    pub attenuated_captions: u64,
}
#[derive(Serialize)]
pub struct ChoreographyState {
    pub entity: usize,
    pub targetname: String,
    pub file: String,
    pub elapsed: Option<f64>,
    pub pause: bool,
    pub pause_reason: Option<&'static str>,
    pub completed_early: bool,
    pub active: bool,
}
/// PhysGunPickup_t reasons that fire prop outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysGunPickup {
    /// PICKED_UP_BY_PLAYER (+USE).
    Player,
    /// PICKED_UP_BY_CANNON.
    Cannon,
    /// PUNTED_BY_CANNON.
    Punt,
}
#[derive(Serialize)]
pub struct SceneAnimationState {
    pub entity: usize,
    pub targetname: String,
    pub animation: String,
    pub sample_time: f32,
    pub scene_owner: Option<usize>,
    pub scripted_by: Option<usize>,
    pub clip_loaded: bool,
    pub origin: [f32; 3],
    pub yaw_degrees: f32,
}
/// The idle activity a metrocop starts and returns to (CAI_BaseNPC idles on ACT_IDLE;
/// CBaseCombatCharacter::Weapon_TranslateActivity maps it through the held weapon's
/// acttable: weapon_pistol -> ACT_IDLE_PISTOL, weapon_smg1/ar2/shotgun -> ACT_IDLE_SMG1;
/// weapon_stunstick has no ACT_IDLE entry). The sequence is then a weighted choice
/// (SelectWeightedSequence: police.mdl batonidle1 x2 / batonidle2 x1). Before this,
/// every cop without DefaultAnim played "Idle_Baton", a sequence without an activity
/// that leans forward with an arm across the chest (owner report, session 17).
pub fn metropolice_idle_activity(entity: &Entity) -> &'static str {
    weapon_idle_activity(entity.get("additionalequipment").unwrap_or(""))
}
/// ACT_IDLE through a held weapon's acttable (CAI_BaseNPC::TranslateActivity:
/// NPC_TranslateActivity, identity at idle outside combat and the player's squad, then
/// Weapon_TranslateActivity): weapon_pistol -> ACT_IDLE_PISTOL, weapon_smg1 ->
/// ACT_IDLE_SMG1 ...; unarmed or a weapon without an ACT_IDLE row: ACT_IDLE.
pub fn weapon_idle_activity(weapon: &str) -> &'static str {
    crate::npc_weapons::translate(&weapon.trim().to_ascii_lowercase(), "ACT_IDLE")
        .unwrap_or("ACT_IDLE")
}
pub struct Scene {
    pub states: Vec<State>,
    /// KeyValues set at runtime by AddOutput, by (entity, lowercase key).
    pub keyvalue_overrides: BTreeMap<(usize, String), String>,
    pub time: f64,
    sequence: u64,
    random: u64,
    queue: Vec<Pending>,
    pub diagnostics: Diagnostics,
    pub transition: Option<(String, String)>,
    pub sounds: Vec<crate::sounds::SoundRequest>,
    /// Captions from choreographed speech this tick.
    pub captions: Vec<crate::captions::CaptionRequest>,
    /// ambient_generic m_fActive after inputs; unset entities keep their spawn state.
    ambient_active: BTreeMap<usize, bool>,
    /// env_soundscape selection for the local player (SDK server soundscape system).
    pub soundscape: crate::soundscapes::Selector,
    /// Damage dealt to the player this tick; negative amounts heal.
    pub player_damage: Vec<crate::player_damage::DamageInfo>,
    /// Global entity states (env_global); carried across level changes by the host.
    pub globals: crate::globals::Globals,
    /// Damage dealt to entities by scene logic (trigger_hurt) this tick.
    pub entity_damage: Vec<crate::projectiles::Damage>,
    /// Burning props (CEntityFlame).
    pub fires: crate::fire::Fires,
    /// Player-thrown or launched props that hit an NPC this tick (prop, NPC):
    /// g_interactionHitByPlayerThrownPhysObj for the AI.
    pub thrown_hits: Vec<(usize, usize)>,
    /// Props broken this tick (CBreakableProp::Break), for the gib spawner.
    pub prop_breaks: Vec<crate::gibs::PropBreak>,
    /// Explosive props broken this tick (CBreakableProp::Break -> ExplosionCreate):
    /// world-space center, integer magnitude and radius, PropaneTank.Burst, attacker.
    pub prop_explosions: Vec<(Vec3, f32, f32, bool, crate::projectiles::Attacker)>,
    /// env_physexplosion entities whose Explode input arrived (phys_explosion).
    pub phys_explosions: Vec<usize>,
    /// env_physimpact entities whose Impact input arrived (phys_explosion).
    pub phys_impacts: Vec<usize>,
    /// Set by the host: a dead player no longer takes damage (m_takedamage).
    pub player_dead: bool,
    pub flashlight: crate::flashlight::PlayerFlashlight,
    /// ScreenFade messages for the local player (env_fade).
    pub screen_fades: Vec<crate::player_damage::ScreenFade>,
    hurt: BTreeMap<usize, HurtState>,
    unsupported: BTreeSet<String>,
    choreography: BTreeMap<usize, Choreography>,
    pub movement_commands: Vec<SceneMoveCommand>,
    movement_ready: BTreeMap<crate::npc::GoalKey, bool>,
    pub look_targets: crate::attention::LookTargets,
    pub gestures: crate::gestures::GestureLayers,
    /// CAI_BaseActor head control per actor, driven by look interests.
    pub heads: BTreeMap<usize, crate::attention::Head>,
    /// Flex controller values per actor (controller name lowercase -> value in its range).
    pub flex_controllers: BTreeMap<usize, BTreeMap<String, f32>>,
    /// Speech visemes added on top of the scene flex controllers.
    pub lipsync: crate::lipsync::LipSync,
    pub monitors: crate::monitors::Cameras,
    /// env_tonemap_controller auto-exposure limits and rate.
    pub tonemap: crate::tonemap::Control,
    /// color_correction and color_correction_volume entities.
    pub color_correction: crate::color_correction::ColorCorrection,
    /// The point_viewcontrol and env_zoom the local player's view follows.
    pub view_control: crate::view_control::ViewControl,
    /// Developer commentary state and `point_commentary_node` nodes.
    pub commentary: crate::commentary::CommentaryState,
    /// point_template children that do not exist until ForceSpawn, with their authored
    /// visible/enabled state.
    templates: BTreeMap<usize, Vec<(usize, bool, bool)>>,
    /// Player feet from the latest tick; FACE targets of `!player` turn toward it.
    player_feet: Vec3,
    /// Duration of the latest tick (NPC movement turning runs after the scene tick).
    tick_dt: f32,
    /// Inputs for the AI (step 14): (target entity, lowercase input, parameter,
    /// activator). The AI runtime drains them; without it they are dropped.
    pub ai_inputs: Vec<(usize, String, String, usize)>,
    /// npc_template_maker inputs for `npc_makers` (target, lowercase input, parameter).
    pub maker_inputs: Vec<(usize, String, String)>,
    /// Template NPCs and npc_template_makers (DESIGN 14s).
    pub makers: crate::npc_makers::NpcMakers,
    /// skill.cfg values (NPC health, hitgroup damage); SDK defaults when absent.
    pub skill: crate::skill::Skill,
    /// Diagnostics: the last bullet/club hit on an NPC.
    pub last_npc_hit: Option<crate::gameplay::NpcHitReport>,
    /// Bullet/club hits on NPCs this tick (victim, by the player, damage) for the AI's
    /// OnTakeDamage_Alive conditions; the AI runtime drains it.
    pub npc_damage: Vec<(usize, bool, f32)>,
    /// NPCs killed by damage this tick (CBaseCombatCharacter::Event_Killed, not removal
    /// by Kill inputs): the host drops their active weapons (DESIGN 14r part B).
    pub npc_deaths: Vec<usize>,
    /// The killing damage's force (kg in/s) per NPC killed this tick (CTakeDamageInfo
    /// damage force; the ragdoll's nudge) and the point whose nearest solid takes it
    /// (m_nForceBone; None: the root solid, the server's default force bone 0).
    pub npc_death_forces: BTreeMap<usize, (Vec3, Option<Vec3>)>,
    /// The latest trace attack's position per NPC (CAI_BaseNPC::TraceAttack keeps the
    /// hit hitbox's physics bone in m_nForceBone): later deaths without a trace (blasts)
    /// push that solid.
    pub npc_force_points: BTreeMap<usize, Vec3>,
    /// CBaseCombatCharacter m_LastHitGroup: the hitgroup of each NPC's last trace attack
    /// (the death pose frame).
    pub npc_hit_groups: BTreeMap<usize, i32>,
    /// Damage forces of projectile hits applied through the entity damage list this tick
    /// (crossbow bolts: CalculateMeleeDamageForce x 0.7), with the hit position, by NPC.
    pub npc_hit_forces: BTreeMap<usize, (Vec3, Vec3)>,
    /// Client ragdoll effects requested this tick (FX_AffectRagdolls for every bullet
    /// and club impact, C_TEExplosion::AffectRagdolls, the BoltImpact effect).
    pub ragdoll_impacts: Vec<crate::ragdolls::Impact>,
    /// Debug option (ours, F4 overlay): the player's hits hurt friendly-immune NPCs.
    pub player_hurts_friendlies: bool,
    /// CBreakableProp m_hLastAttacker: the last player or NPC whose damage a prop took.
    pub prop_attackers: BTreeMap<usize, crate::projectiles::Attacker>,
    /// CBreakableProp::SetPhysicsAttacker: when the player last picked up or dropped a
    /// prop (+USE, gravity gun pickup/drop/launch/punt; m_flLastPhysicsInfluenceTime).
    pub physics_attackers: BTreeMap<usize, f64>,
    /// Ragdolled NPCs' bone matrices (model space), replacing their animation pose.
    pub ragdoll_matrices: BTreeMap<usize, Vec<glam::Mat4>>,
    /// Advances every ragdoll update (render pose cache key).
    pub ragdoll_frame: u64,
    /// NPCs whose AI has the player as enemy or in enemy memory (CAI_BaseHumanoid
    /// sneak attacks exclude them); refreshed by the host after each AI tick.
    pub npc_knows_player: std::collections::BTreeSet<usize>,
    /// When the sequence the AI started last ends, per actor.
    ai_sequence_end: BTreeMap<usize, f64>,
}
impl Scene {
    /// The spawn state of one entity (map load and runtime spawns share this).
    fn initial_state(world: &World, e: &Entity, diagnostics: &mut Diagnostics) -> State {
        let base_rotation =
            physics::angles(parse_vec3(e.get("angles").unwrap_or("0 0 0")).unwrap_or(Vec3::ZERO));
        let flags = number(e, "spawnflags", 0.) as u32;
        let model = e
            .get("model")
            .and_then(|m| m.strip_prefix('*'))
            .and_then(|m| m.parse::<usize>().ok())
            .and_then(|id| world.brush_models.iter().find(|m| m.id == id));
        let bounds = model.map(|m| m.maxs - m.mins).unwrap_or(Vec3::splat(72.));
        let direction = parse_vec3(e.get("movedir").unwrap_or("0 0 0")).unwrap_or(Vec3::ZERO);
        let direction = if direction.x == -1. {
            Vec3::Z
        } else if direction.x == -2. {
            -Vec3::Z
        } else {
            let p = direction.x.to_radians();
            let y = direction.y.to_radians();
            Vec3::new(p.cos() * y.cos(), p.cos() * y.sin(), -p.sin())
        };
        let travel = number(
            e,
            "movedistance",
            (bounds - Vec3::splat(2.))
                .max(Vec3::ZERO)
                .dot(direction.abs())
                - number(e, "lip", 0.),
        )
        .max(0.01);
        let rotating = matches!(e.class(), "func_door_rotating" | "prop_door_rotating");
        let angle = if e.class() == "prop_door_rotating" {
            let distance = number(e, "distance", 90.).abs();
            (if distance == 0. { 90. } else { distance }).to_radians()
                * if number(e, "opendir", 0.) as i32 == 2 {
                    1.
                } else {
                    -1.
                }
        } else {
            number(e, "distance", 90.).to_radians() * if flags & 2 != 0 { -1. } else { 1. }
        };
        let axis = if flags & 64 != 0 {
            Vec3::X
        } else if flags & 128 != 0 {
            Vec3::Y
        } else {
            Vec3::Z
        };
        let fraction = if e.class() == "prop_door_rotating" {
            number(e, "spawnpos", 0.).min(1.)
        } else if matches!(e.class(), "func_door" | "func_door_rotating") && flags & 1 != 0 {
            1.
        } else {
            0.
        };
        let speed = number(e, "speed", 100.).max(0.01);
        let mut outputs = Vec::new();
        for (k, v) in &e.properties {
            if k.starts_with("On") || k == "OutValue" {
                match output(k, v) {
                    Some(o) => outputs.push(o),
                    None => diagnostics.malformed_outputs += 1,
                }
            }
        }
        let enabled = e.get("StartDisabled") != Some("1") && e.get("startdisabled") != Some("1");
        let mut s = State {
            origin: e.origin(),
            rotation: base_rotation,
            // CDynamicProp::Spawn adds EF_NODRAW for StartDisabled.
            visible: e.get("rendermode") != Some("10")
                && e.get(crate::ai::spawn::POOL_KEY).is_none()
                && (!matches!(
                    e.class(),
                    "func_brush" | "func_monitor" | "prop_dynamic" | "prop_dynamic_override"
                ) || enabled),
            enabled,
            killed: false,
            animation: e
                .get("DefaultAnim")
                .unwrap_or(if e.class() == "npc_metropolice" {
                    "idle_baton"
                } else if matches!(e.class(), "npc_citizen" | "npc_barney") {
                    "idle_subtle"
                } else {
                    "idle"
                })
                .to_lowercase(),
            animation_started: 0.,
            default_animation: e.get("DefaultAnim").unwrap_or("").into(),
            animation_done: None,
            scene_animation: None,
            movement_animation: None,
            ai_move_facing: None,
            script_actor: None,
            scripted_by: None,
            script_post_idle: None,
            script_finish: None,
            script_move: None,
            post_idle_done: None,
            shuffle: Vec::new(),
            last_shuffle: None,
            base_origin: e.origin(),
            base_rotation,
            translation: if rotating {
                Vec3::ZERO
            } else {
                direction * travel
            },
            angle: if rotating { angle } else { 0. },
            axis,
            fraction,
            target: fraction,
            rate: if rotating {
                speed.to_radians() / angle.abs().max(0.001)
            } else {
                speed / travel
            },
            locked: flags & 2048 != 0 || e.get("locked") == Some("1"),
            return_at: None,
            wait: number(
                e,
                if e.class() == "prop_door_rotating" {
                    "returndelay"
                } else {
                    "wait"
                },
                -1.,
            ) as f64,
            touching: false,
            last_trigger: -1e6,
            value: number(e, "startvalue", 0.),
            timer_at: number(e, "RefireTime", 1.).max(0.015) as f64,
            outputs,
            parent: None,
        };
        s.origin = s.base_origin + s.translation * s.fraction;
        s.rotation = s.base_rotation * Quat::from_axis_angle(s.axis, s.angle * s.fraction);
        s
    }
    /// CreateEntityByName + DispatchSpawn's state part: append a runtime entity to the
    /// world (the host owns it: DESIGN 14r part A grows the map's World in place) and
    /// its spawn state; returns its id (never reused; removal marks it killed). Map
    /// entity ids never change. Physics, actors, AI and the host register it
    /// separately.
    pub fn spawn_entity(&mut self, world: &mut World, entity: Entity) -> usize {
        let state = Self::initial_state(world, &entity, &mut self.diagnostics);
        let id = self.states.len();
        debug_assert_eq!(id, world.entities.len());
        self.states.push(state);
        world.entities.push(entity);
        id
    }
    /// A map or runtime entity by id.
    pub fn entity<'a>(&self, world: &'a World, id: usize) -> Option<&'a Entity> {
        world.entities.get(id)
    }
    /// A map or runtime entity by a known-valid id (state indices).
    fn ent<'a>(&self, world: &'a World, id: usize) -> &'a Entity {
        &world.entities[id]
    }
    pub fn new(world: &World) -> Self {
        Self::with_campaign(world, true)
    }
    fn initialize_idle_activities(&mut self, world: &World) {
        // Preserve explicit/default labels and existing loaded poses. Unsupported
        // implicit NPC labels may resolve through the model-authored idle activity.
        for id in 0..world.entities.len() {
            let entity = &world.entities[id];
            if !entity.class().starts_with("npc_") || entity.get("DefaultAnim").is_some() {
                continue;
            }
            let rig = world
                .model_instances
                .iter()
                .find(|i| i.entity == Some(id))
                .and_then(|i| world.rigs.get(&i.asset_key()));
            if rig.is_some_and(|r| {
                !r.clips.contains_key(&self.states[id].animation)
                    && r.sequences
                        .iter()
                        .any(|s| s.activity.eq_ignore_ascii_case("ACT_IDLE"))
            }) {
                self.animate(world, id, "ACT_IDLE", false);
            }
        }
    }
    pub fn with_campaign(world: &World, new_game: bool) -> Self {
        let mut scene = Self {
            states: Vec::new(),
            time: 0.,
            sequence: 0,
            random: 1,
            queue: Vec::new(),
            diagnostics: Diagnostics::default(),
            transition: None,
            sounds: Vec::new(),
            captions: Vec::new(),
            ambient_active: BTreeMap::new(),
            soundscape: crate::soundscapes::Selector::new(world),
            player_damage: Vec::new(),
            globals: crate::globals::Globals::spawn(world),
            entity_damage: Vec::new(),
            fires: Default::default(),
            prop_breaks: Vec::new(),
            prop_explosions: Vec::new(),
            phys_explosions: Vec::new(),
            phys_impacts: Vec::new(),
            thrown_hits: Vec::new(),
            keyvalue_overrides: BTreeMap::new(),
            player_dead: false,
            flashlight: Default::default(),
            screen_fades: Vec::new(),
            hurt: BTreeMap::new(),
            unsupported: BTreeSet::new(),
            choreography: BTreeMap::new(),
            movement_commands: Vec::new(),
            movement_ready: BTreeMap::new(),
            ai_inputs: Vec::new(),
            maker_inputs: Vec::new(),
            makers: Default::default(),
            skill: Default::default(),
            last_npc_hit: None,
            npc_damage: Vec::new(),
            npc_deaths: Vec::new(),
            npc_death_forces: BTreeMap::new(),
            npc_force_points: BTreeMap::new(),
            npc_hit_groups: BTreeMap::new(),
            npc_hit_forces: BTreeMap::new(),
            ragdoll_impacts: Vec::new(),
            player_hurts_friendlies: false,
            prop_attackers: BTreeMap::new(),
            physics_attackers: BTreeMap::new(),
            ragdoll_matrices: BTreeMap::new(),
            ragdoll_frame: 0,
            npc_knows_player: Default::default(),
            ai_sequence_end: BTreeMap::new(),
            look_targets: Default::default(),
            gestures: Default::default(),
            heads: BTreeMap::new(),
            flex_controllers: BTreeMap::new(),
            lipsync: Default::default(),
            monitors: crate::monitors::Cameras::new(world),
            tonemap: Default::default(),
            color_correction: crate::color_correction::ColorCorrection::new(world),
            view_control: Default::default(),
            commentary: Default::default(),
            templates: BTreeMap::new(),
            player_feet: Vec3::ZERO,
            tick_dt: 0.,
        };
        let diagnostics = &mut scene.diagnostics;
        scene.states = (world.entities.iter())
            .map(|e| Self::initial_state(world, e, diagnostics))
            .collect();
        scene.initialize_idle_activities(world);
        scene.spawn_combine_cameras(world);
        // Metrocops without a DefaultAnim start on their (weapon-translated) idle activity,
        // as do other weapon users given a weapon (a citizen's SMG1: ACT_IDLE_SMG1).
        for id in 0..world.entities.len() {
            let e = &world.entities[id];
            if e.get("DefaultAnim").is_some() || !crate::npc_weapons::uses_weapons(e.class()) {
                continue;
            }
            let weapon = e.get("additionalequipment").unwrap_or("");
            if e.class() == "npc_metropolice" || !matches!(weapon.trim(), "" | "0") {
                scene.animate_activity_if_present(world, id, weapon_idle_activity(weapon));
            }
        }
        scene.hold_template_children(world);
        // MapEntity_ParseAllEntities removes SF_NPC_TEMPLATE NPCs (npc_template_maker).
        scene.makers = crate::npc_makers::NpcMakers::new(world, &mut scene.states);
        scene.setup_spawn_parents(world);
        // CTimerEntity::Spawn -> Enable -> ResetTimer: random timers draw their first delay.
        for (id, e) in world.entities.iter().enumerate() {
            if e.class() == "logic_timer" && number(e, "UseRandomTime", 0.) != 0. {
                scene.states[id].timer_at = scene.time + scene.timer_interval(e);
            }
        }
        for id in 0..world.entities.len() {
            if world.entities[id].class() == "logic_auto" {
                scene.fire(id, "OnMapSpawn", usize::MAX);
                if new_game {
                    scene.fire(id, "OnNewGame", usize::MAX);
                }
            }
        }
        scene
    }
    /// CPointTemplate removes its Template01..16 entities at map spawn unless spawnflag 1
    /// ("don't remove template entities") is set; they are created by ForceSpawn.
    fn hold_template_children(&mut self, world: &World) {
        for (id, e) in world.entities.iter().enumerate() {
            if e.class() != "point_template" || number(e, "spawnflags", 0.) as u32 & 1 != 0 {
                continue;
            }
            let mut children: Vec<(usize, bool, bool)> = Vec::new();
            for slot in 1..=16 {
                let Some(name) = e
                    .get(&format!("Template{slot:02}"))
                    .filter(|n| !n.is_empty())
                else {
                    continue;
                };
                for (child, c) in world.entities.iter().enumerate() {
                    if child != id
                        && c.get("targetname")
                            .is_some_and(|t| t.eq_ignore_ascii_case(name))
                        && !children.iter().any(|(existing, ..)| *existing == child)
                    {
                        let state = &mut self.states[child];
                        children.push((child, state.visible, state.enabled));
                        state.killed = true;
                        state.visible = false;
                        state.enabled = false;
                    }
                }
            }
            if !children.is_empty() {
                self.templates.insert(id, children);
            }
        }
    }
    /// Whether an entity is a point_template child that has not been spawned yet.
    pub fn template_pending(&self, id: usize) -> bool {
        self.templates
            .values()
            .flatten()
            .any(|(child, ..)| *child == id)
    }
    pub fn tick(&mut self, world: &World, player_feet: Vec3, dt: f32) {
        self.time += dt as f64;
        self.tick_dt = dt;
        if let Some(camera) = self.view_control.expired(world, self.time) {
            self.view_control.disable(camera);
            self.fire(camera, "OnEndFollow", usize::MAX);
        }
        self.player_feet = player_feet;
        self.monitors.tick(&self.states, self.time);
        self.update_color_correction(world);
        self.look_targets.cleanup(self.time, |id| {
            self.states.get(id).is_some_and(|s| !s.killed)
        });
        let states = &self.states;
        self.gestures
            .advance(dt, |id| states.get(id).is_some_and(|s| !s.killed));
        self.tick_choreography(world, dt);
        self.update_heads(world, dt);
        self.lipsync.cleanup(self.time);
        // ProcessSceneEvents decays every flex controller by 0.95 per NPC think (0.1 s)
        // before scene tracks are applied on the next choreography tick.
        let decay = 0.95f32.powf((dt / 0.1).clamp(0., 2.));
        self.flex_controllers.retain(|_, values| {
            values.retain(|_, v| {
                *v *= decay;
                v.abs() > 1e-4
            });
            !values.is_empty()
        });
        for id in 0..world.entities.len() {
            let e = self.ent(world, id);
            if self.states[id].killed {
                continue;
            }
            if let Some((due, actor)) = self.states[id].post_idle_done {
                if due <= self.time {
                    self.states[id].post_idle_done = None;
                    // PostIdleDone: CineCleanup unless the script loops its post idle.
                    if number(e, "spawnflags", 0.) as u32 & 256 == 0
                        && self.states[actor].script_post_idle == Some(id)
                    {
                        self.states[actor].script_post_idle = None;
                    }
                    self.fire(id, "OnPostIdleEndSequence", actor);
                }
            }
            if e.class() == "scripted_sequence" {
                self.finish_script_move(world, id);
                self.script_events(world, id);
            }
            if self.states[id]
                .script_finish
                .is_some_and(|t| t <= self.time)
            {
                self.end_sequence(world, id, false);
            }
            if self.states[id]
                .animation_done
                .is_some_and(|t| t <= self.time)
            {
                self.states[id].animation_done = None;
                self.fire(id, "OnAnimationDone", usize::MAX);
                if e.class() == "npc_combine_camera" {
                    self.camera_activity(world, id, false);
                }
            }
            if matches!(
                e.class(),
                "func_door" | "func_door_rotating" | "prop_door_rotating" | "func_movelinear"
            ) {
                self.advance_mover(id, dt);
            }
            if e.class() == "logic_timer"
                && self.states[id].enabled
                && self.states[id].timer_at <= self.time
            {
                self.states[id].timer_at = self.time + self.timer_interval(e);
                self.fire(id, "OnTimer", usize::MAX);
            }
            if e.class().starts_with("trigger_") && self.states[id].enabled {
                // CBaseTrigger PassesTriggerFilters: clients (1), NPCs (2), physics objects
                // (8, MOVETYPE_VPHYSICS), everything (0x40), then the trigger's filter
                // entity (filtername). Other trigger classes keep the earlier player-only
                // touch.
                let gated = matches!(e.class(), "trigger_once" | "trigger_multiple");
                let flags = number(e, "spawnflags", 1.) as u32;
                let clients = !gated || flags & (1 | 0x40) != 0;
                let npcs = gated && flags & (2 | 0x40) != 0;
                let physics_objects = gated && flags & (8 | 0x40) != 0;
                let filter = gated
                    .then(|| e.get("filtername").map(str::trim).filter(|f| !f.is_empty()))
                    .flatten()
                    .map(str::to_owned);
                let passes = |scene: &Self, other: usize| {
                    filter
                        .as_deref()
                        .is_none_or(|f| scene.passes_filter(world, f, other))
                };
                let player = clients
                    && self.player_inside(world, id, player_feet)
                    && passes(self, usize::MAX);
                let npc = if npcs && !player {
                    (0..self.states.len()).find(|&n| {
                        self.ent(world, n).class().starts_with("npc_")
                            && !self.states[n].killed
                            && self.states[n].visible
                            && self.player_inside(world, id, self.states[n].origin)
                            && passes(self, n)
                    })
                } else {
                    None
                };
                // A VPhysics object's origin inside the brush (small-object test).
                let object = if physics_objects && !player && npc.is_none() {
                    (0..self.states.len()).find(|&n| {
                        is_vphysics_class(self.ent(world, n).class())
                            && !self.states[n].killed
                            && self.states[n].visible
                            && self.touches(world, id, self.states[n].origin, Vec3::splat(2.))
                            && passes(self, n)
                    })
                } else {
                    None
                };
                let inside = player || npc.is_some() || object.is_some();
                let activator = npc.or(object).unwrap_or(usize::MAX);
                if inside && !self.states[id].touching {
                    self.diagnostics.trigger_entries += 1;
                    self.fire(id, "OnStartTouch", activator);
                    self.fire(id, "OnStartTouchAll", activator);
                }
                if !inside && self.states[id].touching {
                    self.fire(id, "OnEndTouch", activator);
                    self.fire(id, "OnEndTouchAll", activator);
                }
                if inside
                    && gated
                    && self.time - self.states[id].last_trigger >= number(e, "wait", 0.2) as f64
                {
                    self.fire(id, "OnTrigger", activator);
                    self.states[id].last_trigger = self.time;
                    if e.class() == "trigger_once" {
                        self.states[id].enabled = false;
                    }
                }
                if e.class() == "trigger_hurt" {
                    // Every touching player/NPC, before the trigger filters (Touch).
                    let mut touching: Vec<usize> = (0..self.states.len())
                        .filter(|&n| {
                            self.ent(world, n).class().starts_with("npc_")
                                && !self.states[n].killed
                                && self.player_inside(world, id, self.states[n].origin)
                        })
                        .collect();
                    if self.player_inside(world, id, player_feet) {
                        touching.insert(0, PLAYER);
                    }
                    self.trigger_hurt(world, id, touching);
                }
                // SF_CHANGELEVEL_NOTOUCH (0x2) leaves the ChangeLevel input available,
                // but must not change maps when the player overlaps the brush.
                if inside
                    && e.class() == "trigger_changelevel"
                    && number(e, "spawnflags", 0.) as u32 & 2 == 0
                    && !self.states[id].touching
                {
                    self.send(id, "ChangeLevel", "");
                }
                self.states[id].touching = inside;
            }
        }
        // Parented children follow this tick's parent poses and animated attachments.
        for id in 0..self.states.len() {
            if self.states[id].parent.is_some() && !self.states[id].killed {
                self.follow_parent(world, id);
            }
        }
        for _ in 0..2048 {
            let Some(index) = self
                .queue
                .iter()
                .enumerate()
                .filter(|(_, p)| p.due <= self.time)
                .min_by(|(_, a), (_, b)| a.due.total_cmp(&b.due).then(a.sequence.cmp(&b.sequence)))
                .map(|(i, _)| i)
            else {
                return;
            };
            let p = self.queue.remove(index);
            let targets = if let Some(id) = p
                .target
                .strip_prefix('#')
                .and_then(|s| s.parse::<usize>().ok())
            {
                vec![id]
            } else {
                self.targets(world, &p)
            };
            for id in targets {
                self.deliver(world, id, &p, player_feet);
            }
        }
        self.diagnostics.budget_exhaustions += 1;
    }
}
/// UTIL_AngleDiff: signed shortest difference `a - b` in degrees.
fn angle_diff(a: f32, b: f32) -> f32 {
    let mut d = (a - b) % 360.;
    if d > 180. {
        d -= 360.;
    } else if d < -180. {
        d += 360.;
    }
    d
}
/// AI_ClampYaw: move `current` toward `target` by at most `speed` degrees.
fn clamp_yaw(speed: f32, current: f32, target: f32) -> f32 {
    let step = angle_diff(target, current).clamp(-speed, speed);
    (current + step).rem_euclid(360.)
}
pub(crate) fn glob(pattern: &str, value: &str) -> bool {
    if let Some((first, last)) = pattern.split_once('*') {
        value.starts_with(first) && value.ends_with(last)
    } else {
        pattern == value
    }
}
/// Seconds after its start when CBaseAnimating::StudioFrameAdvanceInternal marks a
/// sequence finished (m_bSequenceFinished, read by IsActivityFinished): at its end, or
/// for a non-looping sequence once the cycle passes GetLastVisibleCycle (one minus
/// fadeouttime times the cycle rate), i.e. fadeouttime (0.2 s on the HL2 shoot/reload
/// sequences) before the end; a sequence shorter than its fade-out finishes at the first frame advance. Native
/// NPCs therefore restart 0.1-0.28 s shoot sequences every think (0.1 s), not once
/// they play through.
pub fn sequence_finish(clip: &modkit_core::animation::Clip) -> f64 {
    let duration = clip.duration().max(0.015);
    if clip.looping {
        f64::from(duration)
    } else {
        f64::from((duration - clip.layer.fade_out).max(0.001))
    }
}

#[cfg(test)]
mod tests;
