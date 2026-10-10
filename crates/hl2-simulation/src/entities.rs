//! Deterministic map entity I/O. Unsupported inputs are reported instead of silently emulated.
use crate::physics;
use glam::{Mat4, Quat, Vec3};
use modkit_core::{parse_vec3, Entity, World};
use serde::Serialize;
use source_assets::{
    scenes::{Cache, ChoreoScene, EventType},
    vpk::Vfs,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

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
    script_actor: Option<usize>,
    pub scripted_by: Option<usize>,
    script_finish: Option<f64>,
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
pub struct Scene {
    pub states: Vec<State>,
    pub time: f64,
    sequence: u64,
    random: u64,
    queue: Vec<Pending>,
    pub diagnostics: Diagnostics,
    pub transition: Option<(String, String)>,
    pub sounds: Vec<crate::sounds::SoundRequest>,
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
    /// Set by the host: a dead player no longer takes damage (m_takedamage).
    pub player_dead: bool,
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
    /// point_template children that do not exist until ForceSpawn, with their authored
    /// visible/enabled state.
    templates: BTreeMap<usize, Vec<(usize, bool, bool)>>,
    /// Player feet from the latest tick; FACE targets of `!player` turn toward it.
    player_feet: Vec3,
    /// Duration of the latest tick (NPC movement turning runs after the scene tick).
    tick_dt: f32,
}
impl Scene {
    pub fn new(world: &World) -> Self {
        Self::with_campaign(world, true)
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
            ambient_active: BTreeMap::new(),
            soundscape: crate::soundscapes::Selector::new(world),
            player_damage: Vec::new(),
            globals: crate::globals::Globals::spawn(world),
            entity_damage: Vec::new(),
            player_dead: false,
            screen_fades: Vec::new(),
            hurt: BTreeMap::new(),
            unsupported: BTreeSet::new(),
            choreography: BTreeMap::new(),
            movement_commands: Vec::new(),
            movement_ready: BTreeMap::new(),
            look_targets: Default::default(),
            gestures: Default::default(),
            heads: BTreeMap::new(),
            flex_controllers: BTreeMap::new(),
            lipsync: Default::default(),
            monitors: crate::monitors::Cameras::new(world),
            tonemap: Default::default(),
            templates: BTreeMap::new(),
            player_feet: Vec3::ZERO,
            tick_dt: 0.,
        };
        for e in &world.entities {
            let base_rotation = physics::angles(
                parse_vec3(e.get("angles").unwrap_or("0 0 0")).unwrap_or(Vec3::ZERO),
            );
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
                        None => scene.diagnostics.malformed_outputs += 1,
                    }
                }
            }
            let enabled =
                e.get("StartDisabled") != Some("1") && e.get("startdisabled") != Some("1");
            let mut s = State {
                origin: e.origin(),
                rotation: base_rotation,
                // CDynamicProp::Spawn adds EF_NODRAW for StartDisabled.
                visible: e.get("rendermode") != Some("10")
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
                script_actor: None,
                scripted_by: None,
                script_finish: None,
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
            scene.states.push(s);
        }
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
                !r.clips.contains_key(&scene.states[id].animation)
                    && r.sequences
                        .iter()
                        .any(|s| s.activity.eq_ignore_ascii_case("ACT_IDLE"))
            }) {
                scene.animate(world, id, "ACT_IDLE", false);
            }
        }
        // CNPC_CombineCamera::Spawn: deployed (open idle) unless spawnflag 0x80
        // (StartInactive), which keeps it closed and disabled.
        for id in 0..world.entities.len() {
            if world.entities[id].class() == "npc_combine_camera" {
                let enabled = number(&world.entities[id], "spawnflags", 0.) as u32 & 0x80 == 0;
                scene.states[id].enabled = enabled;
                scene.camera_activity(world, id, false);
            }
        }
        scene.hold_template_children(world);
        // SetupParentsForSpawnList: `parentname` ("name" or "name,attachment") keeps the
        // spawn world pose. Parents still held by a point_template do not exist yet.
        for id in 0..world.entities.len() {
            let e = &world.entities[id];
            let Some(spec) = e.get("parentname").filter(|n| !n.is_empty()) else {
                continue;
            };
            if self_positioned(e.class()) || scene.template_pending(id) {
                continue;
            }
            let (name, attachment) = match spec.split_once(',') {
                Some((n, a)) => (n.trim(), Some(a.trim().to_owned())),
                None => (spec.trim(), None),
            };
            let Some(parent) = world.entities.iter().position(|p| {
                p.get("targetname")
                    .is_some_and(|t| t.eq_ignore_ascii_case(name))
            }) else {
                continue;
            };
            if parent != id && !scene.template_pending(parent) {
                scene.set_parent(world, id, parent, attachment, true);
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
    /// Preload map choreography before simulation; no blocking asset reads in tick.
    pub fn load_choreography(&mut self, world: &World, vfs: &Vfs) -> anyhow::Result<usize> {
        use anyhow::Context;
        if !world
            .entities
            .iter()
            .any(|e| e.class() == "logic_choreographed_scene")
        {
            return Ok(0);
        }
        self.lipsync.load_classes(vfs);
        let bytes = vfs
            .read("scenes/scenes.image")?
            .context("installed scenes.image missing")?;
        let cache = Cache::parse(bytes)?;
        let mut parsed: BTreeMap<String, Arc<ChoreoScene>> = BTreeMap::new();
        for (id, entity) in world.entities.iter().enumerate() {
            if entity.class() != "logic_choreographed_scene" {
                continue;
            }
            let path = entity.get("SceneFile").unwrap_or("");
            let result = if let Some(data) = parsed.get(path) {
                Ok(Some(data.clone()))
            } else {
                cache.scene(path).map(|scene| {
                    scene.map(|data| {
                        let data = Arc::new(data);
                        parsed.insert(path.into(), data.clone());
                        data
                    })
                })
            };
            match result {
                Ok(Some(data)) => self.install_choreography(id, data),
                Ok(None) => {
                    self.unsupported_input(entity.class(), &format!("missing-scene:{path}"))
                }
                Err(error) => {
                    self.unsupported_input(entity.class(), &format!("scene:{path}:{error:#}"))
                }
            }
        }
        Ok(self.choreography.len())
    }
    fn install_choreography(&mut self, id: usize, data: Arc<ChoreoScene>) {
        let mut order: Vec<_> = (0..data.events.len()).collect();
        order.sort_by(|a, b| {
            data.events[*a]
                .start
                .total_cmp(&data.events[*b].start)
                .then_with(|| data.events[*a].name.cmp(&data.events[*b].name))
        });
        self.choreography.insert(
            id,
            Choreography {
                data,
                order,
                playback: None,
            },
        );
        self.diagnostics.scenes_loaded = self.choreography.len();
    }
    /// Every actor-class entity a scene actor name can resolve to, including killed and
    /// template-pending ones (`!activator` is unknown before playback and resolves to none).
    fn scene_actor_candidates(&self, world: &World, scene: usize, name: &str) -> Vec<usize> {
        let name = resolve_target_slot(world, scene, name).to_lowercase();
        world
            .entities
            .iter()
            .enumerate()
            .filter(|(_, entity)| {
                scene_actor_class(entity.class())
                    && entity
                        .get("targetname")
                        .is_some_and(|target| glob(&name, &target.to_lowercase()))
            })
            .map(|(id, _)| id)
            .collect()
    }
    fn scene_actor(
        &self,
        world: &World,
        scene: usize,
        name: &str,
        activator: usize,
    ) -> Option<usize> {
        let name = resolve_target_slot(world, scene, name);
        if name.eq_ignore_ascii_case("!activator") {
            return (activator < self.states.len()).then_some(activator);
        }
        world
            .entities
            .iter()
            .enumerate()
            .find(|(id, entity)| {
                !self.states[*id].killed
                    && scene_actor_class(entity.class())
                    && entity
                        .get("targetname")
                        .is_some_and(|target| glob(&name.to_lowercase(), &target.to_lowercase()))
            })
            .map(|(id, _)| id)
    }
    /// Developer fixture setup only; does not reproduce campaign actor placement.
    pub fn fixture_actor_pose(
        &mut self,
        world: &World,
        target: &str,
        origin: Vec3,
        yaw: f32,
    ) -> bool {
        let Some(id) = world
            .entities
            .iter()
            .enumerate()
            .find(|(id, e)| {
                e.class().starts_with("npc_")
                    && !self.states[*id].killed
                    && e.get("targetname")
                        .is_some_and(|n| n.eq_ignore_ascii_case(target))
            })
            .map(|(id, _)| id)
        else {
            return false;
        };
        self.states[id].origin = origin;
        self.states[id].rotation = physics::angles(Vec3::new(0., yaw, 0.));
        true
    }
    fn scene_look_target(
        &self,
        world: &World,
        scene: usize,
        actor: usize,
        name: &str,
    ) -> Option<crate::attention::Target> {
        use crate::attention::Target;
        if name.eq_ignore_ascii_case("!self") {
            return Some(Target::Entity(actor));
        }
        let name = name
            .strip_prefix('!')
            .and_then(|s| s.to_lowercase().strip_prefix("target").map(str::to_owned))
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|slot| (1..=8).contains(slot))
            .and_then(|slot| world.entities[scene].get(&format!("target{slot}")))
            .unwrap_or(name);
        if name.eq_ignore_ascii_case("!player") || name.eq_ignore_ascii_case("player") {
            return Some(Target::Player);
        }
        world
            .entities
            .iter()
            .enumerate()
            .find(|(id, e)| {
                !self.states[*id].killed
                    && e.get("targetname")
                        .is_some_and(|n| glob(&name.to_lowercase(), &n.to_lowercase()))
            })
            .map(|(id, _)| Target::Entity(id))
    }
    pub fn scene_sound_request(
        &self,
        world: &World,
        scene: usize,
        actor_name: &str,
        activator: usize,
        cue: &str,
    ) -> Option<crate::sounds::SoundRequest> {
        let actor = self.scene_actor(world, scene, actor_name, activator)?;
        Some(self.actor_sound_request(world, actor, cue))
    }
    fn actor_sound_request(
        &self,
        world: &World,
        actor: usize,
        cue: &str,
    ) -> crate::sounds::SoundRequest {
        crate::sounds::SoundRequest {
            ambient: None,
            volume: None,
            origin: None,
            name: cue.into(),
            actor: Some(crate::sounds::SoundActor {
                name: world.entities[actor].get("targetname").unwrap_or("").into(),
                model: world
                    .model_instances
                    .iter()
                    .find(|i| i.entity == Some(actor))
                    .map(|i| i.model.as_str())
                    .unwrap_or(world.entities[actor].get("model").unwrap_or(""))
                    .into(),
                entity: Some(actor),
            }),
        }
    }
    /// Precache SPEAK cues from every loaded scene without advancing playback.
    /// Unresolved dynamic actors retain a context-free request for broad preloading.
    pub fn required_sound_requests(&self, world: &World) -> Vec<crate::sounds::SoundRequest> {
        let mut result = Vec::new();
        for (id, scene) in &self.choreography {
            for event in &scene.data.events {
                if !event.active()
                    || event.kind != EventType::Speak
                    || event.parameters[0].is_empty()
                {
                    continue;
                }
                let request = event.actor.and_then(|actor| {
                    self.scene_sound_request(
                        world,
                        *id,
                        &scene.data.actors[actor].name,
                        usize::MAX,
                        &event.parameters[0],
                    )
                });
                result.push(request.unwrap_or_else(|| event.parameters[0].clone().into()));
            }
        }
        result
    }
    /// Installed animation names required by loaded VCDs, grouped by model/skin.
    /// Gestures are requested for future layer support, but are not substituted for a body sequence.
    pub fn required_animation_clips(&self, world: &World) -> BTreeMap<String, BTreeSet<String>> {
        let mut result: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (id, scene) in &self.choreography {
            for event in &scene.data.events {
                if !event.active()
                    || !matches!(event.kind, EventType::Sequence | EventType::Gesture)
                    || event.name.eq_ignore_ascii_case("NULL")
                    || event.parameters[0].is_empty()
                {
                    continue;
                }
                let Some(actor) = event.actor else {
                    continue;
                };
                // Preparation runs at load, before point_template children (Barney, Kleiner)
                // are force-spawned, so include every entity the name can resolve to.
                for actor in self.scene_actor_candidates(world, *id, &scene.data.actors[actor].name)
                {
                    if let Some(instance) = world
                        .model_instances
                        .iter()
                        .find(|i| i.entity == Some(actor))
                    {
                        result
                            .entry(instance.asset_key())
                            .or_default()
                            .insert(event.parameters[0].to_lowercase());
                    }
                }
            }
        }
        result
    }
    /// Skinning matrices for an actor: current base clip plus scene gesture layers.
    /// Both hosts use this so presentation, eyes and impacts see one pose.
    /// World frame an attached child follows: the parent's origin/rotation, or the
    /// parent's animated attachment (model-scaled). `None` for unknown attachments.
    fn parent_frame(&self, world: &World, parent: usize, attachment: Option<&str>) -> Option<Mat4> {
        let state = self.states.get(parent)?;
        match attachment.filter(|n| !n.is_empty()) {
            None => Some(Mat4::from_rotation_translation(
                state.rotation,
                state.origin,
            )),
            Some(name) => self.attachment_frames(world, parent, &[name])[0],
        }
    }
    /// World frames of named animated attachments (one pose evaluation for all names).
    pub fn attachment_frames(&self, world: &World, id: usize, names: &[&str]) -> Vec<Option<Mat4>> {
        let none = || vec![None; names.len()];
        let Some(state) = self.states.get(id) else {
            return none();
        };
        let Some(instance) = world.model_instances.iter().find(|i| i.entity == Some(id)) else {
            return none();
        };
        let Some(rig) = world.rigs.get(&instance.asset_key()) else {
            return none();
        };
        let found: Vec<_> = names.iter().map(|n| rig.attachment(n)).collect();
        if found.iter().all(Option::is_none) {
            return none();
        }
        let base = Mat4::from_rotation_translation(state.rotation, state.origin);
        let matrices = self.actor_matrices(rig, id);
        found
            .into_iter()
            .map(|a| {
                let local = rig.attachment_matrix(&matrices, a?)?;
                let (_, rotation, translation) = local.to_scale_rotation_translation();
                Some(
                    base * Mat4::from_rotation_translation(
                        rotation.normalize(),
                        translation * instance.scale,
                    ),
                )
            })
            .collect()
    }
    /// SetParent/SetParentAttachment: keep the current world pose as a local offset, or
    /// snap onto the parent frame.
    fn set_parent(
        &mut self,
        world: &World,
        id: usize,
        parent: usize,
        attachment: Option<String>,
        keep_offset: bool,
    ) {
        let Some(frame) = self.parent_frame(world, parent, attachment.as_deref()) else {
            self.unsupported_input(world.entities[id].class(), "SetParent:missing-attachment");
            return;
        };
        let (origin, rotation) = if keep_offset {
            let state = &self.states[id];
            let local =
                frame.inverse() * Mat4::from_rotation_translation(state.rotation, state.origin);
            let (_, rotation, origin) = local.to_scale_rotation_translation();
            (origin, rotation.normalize())
        } else {
            (Vec3::ZERO, Quat::IDENTITY)
        };
        self.states[id].parent = Some(Parent {
            entity: parent,
            attachment,
            origin,
            rotation,
        });
        self.follow_parent(world, id);
    }
    /// Move a parented child onto its parent frame and local offset.
    fn follow_parent(&mut self, world: &World, id: usize) {
        let Some(parent) = self.states[id].parent.clone() else {
            return;
        };
        if self.states[parent.entity].killed {
            return;
        }
        if let Some(frame) = self.parent_frame(world, parent.entity, parent.attachment.as_deref()) {
            let pose = frame * Mat4::from_rotation_translation(parent.rotation, parent.origin);
            let (_, rotation, origin) = pose.to_scale_rotation_translation();
            self.states[id].origin = origin;
            self.states[id].rotation = rotation.normalize();
        }
    }
    /// Flex controller values a renderer applies: scene/expression controllers plus the
    /// visemes of any line the actor is speaking (client ProcessVisemes order).
    pub fn actor_flex_values(&self, id: usize) -> BTreeMap<String, f32> {
        let mut values = self.flex_controllers.get(&id).cloned().unwrap_or_default();
        for (name, add) in self.lipsync.visemes(id, self.time) {
            *values.entry(name).or_insert(0.) += add;
        }
        values
    }
    pub fn actor_matrices(&self, rig: &modkit_core::animation::Rig, id: usize) -> Vec<glam::Mat4> {
        let params = self.actor_pose_values(rig, id);
        self.gestures
            .compose(
                rig,
                id,
                &self.states[id].animation,
                self.animation_time(id),
                &params,
            )
            .0
    }
    /// Normalized pose parameters: model defaults plus head control (head_pitch/yaw/roll)
    /// and the server-side body/head flex controllers (CAI_BaseActor UpdateHeadControl adds
    /// head_rightleft/updown/tilt to the look correction; UpdateBodyControl drives
    /// body_yaw, spine_yaw and neck_trans from body_rightleft, chest_rightleft and
    /// head_forwardback).
    pub fn actor_pose_values(&self, rig: &modkit_core::animation::Rig, id: usize) -> Vec<f32> {
        let mut params = rig.default_pose_values();
        let flexes = self.flex_controllers.get(&id);
        let flex = |name: &str| flexes.and_then(|v| v.get(name)).copied();
        let head = self.heads.get(&id).map(|h| h.goal);
        for (param, goal, controller) in [
            ("head_pitch", head.map(|g| g.x), "head_updown"),
            ("head_yaw", head.map(|g| g.y), "head_rightleft"),
            ("head_roll", head.map(|g| g.z), "head_tilt"),
            ("body_yaw", None, "body_rightleft"),
            ("spine_yaw", None, "chest_rightleft"),
            ("neck_trans", None, "head_forwardback"),
            // MaintainLookTargets: gesture blend position from the gesture flexes.
            ("gesture_height", None, "gesture_updown"),
            ("gesture_width", None, "gesture_rightleft"),
        ] {
            let offset = flex(controller);
            if goal.is_none() && offset.is_none() {
                continue;
            }
            if let Some(i) = rig.pose_parameter(param) {
                let value = goal.unwrap_or(0.) + offset.unwrap_or(0.);
                params[i] = rig.pose_parameters[i].normalize(value);
            }
        }
        if let (Some(movement), Some(i)) = (
            self.states[id].movement_animation.as_ref(),
            rig.pose_parameter("move_yaw"),
        ) {
            params[i] = rig.pose_parameters[i].normalize(movement.move_yaw);
        }
        params
    }
    /// Changes whenever an actor's composed pose inputs change; zero for plain base clips.
    pub fn pose_signature(&self, id: usize) -> u64 {
        let mut hash = self.gestures.signature(id);
        let mix = |hash: u64, v: f32| {
            (hash ^ v.to_bits() as u64)
                .wrapping_mul(0x100000001b3)
                .rotate_left(9)
        };
        if let Some(head) = self.heads.get(&id) {
            for v in head.goal.to_array() {
                hash = mix(hash, v);
            }
        }
        if let Some(m) = self
            .states
            .get(id)
            .and_then(|s| s.movement_animation.as_ref())
        {
            hash = mix(hash, m.move_yaw);
        }
        if let Some(values) = self.flex_controllers.get(&id) {
            for name in [
                "head_updown",
                "head_rightleft",
                "head_tilt",
                "body_rightleft",
                "chest_rightleft",
                "head_forwardback",
                "gesture_updown",
                "gesture_rightleft",
            ] {
                if let Some(v) = values.get(name) {
                    hash = mix(hash, *v);
                }
            }
        }
        hash
    }
    /// MaintainLookTargets: blend this tick's interests into each NPC's head control.
    fn update_heads(&mut self, world: &World, dt: f32) {
        let mut actors: BTreeSet<usize> = self.look_targets.report().keys().copied().collect();
        actors.extend(self.heads.keys().copied());
        for actor in actors {
            let state = &self.states[actor];
            if state.killed || !world.entities[actor].class().starts_with("npc_") {
                self.heads.remove(&actor);
                continue;
            }
            // CAI_BaseActor: eye position from the animated "eyes" attachment; self-interest
            // and the head frame follow the animated "forward" attachment.
            let frames = self.attachment_frames(world, actor, &["eyes", "forward"]);
            let state = &self.states[actor];
            let eye = frames[0].map_or(state.origin + Vec3::Z * 64., |m| m.w_axis.truncate());
            let frame = frames[1].map_or(glam::Mat3::from_quat(state.rotation), |m| {
                glam::Mat3::from_mat4(m)
            });
            let forward = frame.x_axis;
            let targets = self
                .look_targets
                .report()
                .get(&actor)
                .into_iter()
                .flatten()
                .filter_map(|interest| {
                    let position = match interest.target {
                        crate::attention::Target::Player => self.player_feet + Vec3::Z * 64.,
                        crate::attention::Target::Entity(t) if t == actor => {
                            return Some((forward, interest.importance));
                        }
                        crate::attention::Target::Entity(t) => self.states.get(t)?.origin,
                    };
                    Some((position - eye, interest.importance))
                })
                .collect::<Vec<_>>();
            let head = self.heads.entry(actor).or_default();
            head.update(&targets, frame, dt);
            if targets.is_empty() && head.influence == 0. && head.goal.length() < 0.01 {
                self.heads.remove(&actor);
            }
        }
    }
    /// Sequence sampling uses the paused choreography clock, not wall/game time.
    pub fn animation_time(&self, id: usize) -> f32 {
        let state = &self.states[id];
        if let Some(movement) = &state.movement_animation {
            return movement.elapsed;
        }
        if let Some(animation) = &state.scene_animation {
            if let Some(scene) = self.choreography.get(&animation.owner) {
                if let Some(play) = &scene.playback {
                    return (play.elapsed - scene.data.events[animation.event].start as f64).max(0.)
                        as f32;
                }
            }
        }
        (self.time - state.animation_started).max(0.) as f32
    }
    pub fn body_sequence_owner(&self, id: usize) -> Option<usize> {
        self.states
            .get(id)?
            .scene_animation
            .as_ref()
            .map(|s| s.owner)
    }
    /// Read-only capture metadata; this neither advances nor resolves scene readiness.
    pub fn choreography_states(&self, world: &World) -> Vec<ChoreographyState> {
        self.choreography
            .iter()
            .map(|(id, scene)| {
                let play = scene.playback.as_ref();
                let pause_reason = play.and_then(|p| p.pause.as_ref()).map(|p| match p {
                    ScenePause::Input => "input",
                    ScenePause::Section { blocked: true, .. } => "actor_condition",
                    ScenePause::Section { blocked: false, .. } => "section",
                    ScenePause::UnsupportedControl => "unsupported_control",
                });
                ChoreographyState {
                    entity: *id,
                    targetname: world.entities[*id].get("targetname").unwrap_or("").into(),
                    file: world.entities[*id].get("SceneFile").unwrap_or("").into(),
                    elapsed: play.map(|p| p.elapsed),
                    pause: pause_reason.is_some(),
                    pause_reason,
                    completed_early: play.is_some_and(|p| p.completed_early),
                    active: play.is_some(),
                }
            })
            .collect()
    }
    /// Named supported actor samples, including whether an installed rig supplies the clip.
    pub fn animation_states(&self, world: &World) -> Vec<SceneAnimationState> {
        world
            .entities
            .iter()
            .enumerate()
            .filter_map(|(id, entity)| {
                if !scene_actor_class(entity.class()) {
                    return None;
                }
                let targetname = entity.get("targetname").filter(|n| !n.is_empty())?;
                let state = &self.states[id];
                if state.killed || state.animation.is_empty() {
                    return None;
                }
                let clip_loaded = world
                    .model_instances
                    .iter()
                    .find(|i| i.entity == Some(id))
                    .and_then(|i| world.rigs.get(&i.asset_key()))
                    .is_some_and(|rig| rig.clips.contains_key(&state.animation));
                Some(SceneAnimationState {
                    entity: id,
                    targetname: targetname.into(),
                    animation: state.animation.clone(),
                    sample_time: self.animation_time(id),
                    scene_owner: state.scene_animation.as_ref().map(|a| a.owner),
                    scripted_by: state.scripted_by,
                    clip_loaded,
                    origin: state.origin.to_array(),
                    yaw_degrees: {
                        let forward = state.rotation * Vec3::X;
                        forward.y.atan2(forward.x).to_degrees()
                    },
                })
            })
            .collect()
    }
    fn restore_scene_animation(&mut self, actor: usize) {
        let state = &mut self.states[actor];
        if let Some(animation) = state.scene_animation.take() {
            state.animation = animation.previous;
            state.animation_started = animation.previous_started;
            state.animation_done = animation.previous_done;
        }
    }
    fn clear_scene_animations(&mut self, owner: usize, canceled: bool) {
        self.gestures.release(owner, None, canceled);
        self.movement_commands
            .push(SceneMoveCommand::CancelScene(owner));
        self.movement_ready.retain(|key, _| key.scene != owner);
        for actor in 0..self.states.len() {
            if self.states[actor]
                .movement_animation
                .as_ref()
                .is_some_and(|m| m.key.scene == owner)
            {
                self.restore_movement_animation(actor);
            }
            if self.states[actor]
                .scene_animation
                .as_ref()
                .is_some_and(|s| s.owner == owner)
            {
                self.restore_scene_animation(actor);
            }
        }
    }
    fn restore_movement_animation(&mut self, actor: usize) {
        let state = &mut self.states[actor];
        if let Some(previous) = state.movement_animation.take() {
            // A full-body sequence or scripted owner may have taken over since the last update.
            if state.animation == previous.sequence
                && state.scene_animation.is_none()
                && state.scripted_by.is_none()
            {
                state.animation = previous.previous;
                state.animation_started = previous.previous_started;
                state.animation_done = previous.previous_done;
            }
        }
    }
    pub fn apply_movement(
        &mut self,
        key: crate::npc::GoalKey,
        feet: Vec3,
        yaw_degrees: f32,
        animation: Option<(&str, f32)>,
        arrived: bool,
    ) {
        let Some(ready) = self.movement_ready.get_mut(&key) else {
            return;
        };
        let state = &self.states[key.actor];
        if state
            .movement_animation
            .as_ref()
            .is_some_and(|m| m.key != key)
        {
            *ready = arrived;
            return;
        }
        if state.killed
            || state.scripted_by.is_some()
            || state.scene_animation.is_some()
            || !feet.is_finite()
            || !yaw_degrees.is_finite()
        {
            *ready = false;
            self.restore_movement_animation(key.actor);
            return;
        }
        *ready = arrived;
        // CAI_Motor::MoveFacing/UpdateYaw: the body turns toward the path direction at
        // MaxYawSpeed (45, x10 per second); move_yaw keeps the legs on the path.
        let forward = self.states[key.actor].rotation * Vec3::X;
        let current = forward.y.atan2(forward.x).to_degrees();
        let body = if animation.is_some() {
            clamp_yaw(450. * self.tick_dt, current, yaw_degrees)
        } else {
            yaw_degrees
        };
        let move_yaw = angle_diff(yaw_degrees, body);
        self.states[key.actor].origin = feet;
        self.states[key.actor].rotation = physics::angles(Vec3::new(0., body, 0.));
        if let Some((sequence, elapsed)) = animation {
            if self.states[key.actor]
                .movement_animation
                .as_ref()
                .is_none_or(|m| m.key != key || m.sequence != sequence)
            {
                self.restore_movement_animation(key.actor);
                let state = &mut self.states[key.actor];
                state.movement_animation = Some(MovementAnimation {
                    key,
                    sequence: sequence.into(),
                    elapsed,
                    move_yaw,
                    previous: state.animation.clone(),
                    previous_started: state.animation_started,
                    previous_done: state.animation_done,
                });
                state.animation = sequence.into();
                state.animation_done = None;
            } else if let Some(m) = &mut self.states[key.actor].movement_animation {
                m.elapsed = elapsed;
                m.move_yaw = move_yaw;
            }
        } else {
            self.restore_movement_animation(key.actor);
        }
    }
    fn section_blocked(&self, id: usize, scene: &ChoreoScene, play: &Playback, at: f64) -> bool {
        scene.events.iter().enumerate().any(|(event, e)| {
            if !e.active() || !e.resume_condition() || e.start as f64 > at {
                return false;
            }
            let Some(actor) = e.actor.and_then(|a| play.actors[a]) else {
                return false;
            };
            e.kind != EventType::MoveTo
                || !self
                    .movement_ready
                    .get(&crate::npc::GoalKey {
                        scene: id,
                        event,
                        actor,
                    })
                    .copied()
                    .unwrap_or(false)
        })
    }
    fn start_choreography(&mut self, world: &World, id: usize, activator: usize) {
        let Some(mut scene) = self.choreography.remove(&id) else {
            self.unsupported_input("logic_choreographed_scene", "Start:scene-not-loaded");
            return;
        };
        if scene.playback.is_none() {
            let actors = scene
                .data
                .actors
                .iter()
                .map(|a| self.scene_actor(world, id, &a.name, activator))
                .collect();
            scene.playback = Some(Playback {
                elapsed: 0.,
                next: 0,
                completed_early: false,
                pause: None,
                actors,
                look_targets: BTreeMap::new(),
                face_targets: BTreeMap::new(),
                face_initial: BTreeMap::new(),
            });
            self.fire(id, "OnStart", id);
        }
        self.choreography.insert(id, scene);
    }
    /// ProcessGestureSceneEvent for started events inside their window; RemoveLayer
    /// after the end. Absent or empty gesture names produce no layer (LookupSequence fails).
    fn process_gestures(&mut self, id: usize, scene: &Choreography, play: &Playback) {
        for event in self.gestures.active_events(id) {
            let e = &scene.data.events[event];
            if play.elapsed > e.end.unwrap_or(e.start) as f64 {
                self.gestures.release(id, Some(event), false);
            }
        }
        for &index in scene.order.iter().take(play.next) {
            let event = &scene.data.events[index];
            let Some(end) = event.end else {
                continue;
            };
            let duration = end - event.start;
            if event.kind != EventType::Gesture
                || !event.active()
                || event.parameters[0].is_empty()
                || duration <= 0.
                || play.elapsed < event.start as f64
                || play.elapsed > end as f64
            {
                continue;
            }
            let Some(actor) = event.actor.and_then(|a| play.actors[a]) else {
                continue;
            };
            if self.states[actor].killed {
                continue;
            }
            self.gestures.update(crate::gestures::GestureUpdate {
                actor,
                scene: id,
                event: index,
                data: &scene.data,
                playback: (play.elapsed as f32 - event.start) / duration,
                weight: event.intensity(&scene.data, play.elapsed as f32),
                priority: event.channel.unwrap_or(0),
                moving: self.states[actor].movement_animation.is_some(),
            });
        }
    }
    /// ProcessFacingSceneEvent for a standing NPC: ideal yaw blends from the yaw latched at
    /// the first process toward the target by event intensity, and the motor turns at most
    /// MaxYawSpeed (45 per 0.1 s) toward it. Facing while moving (AddFacingTarget) is not
    /// implemented; a moving actor only re-latches its initial yaw.
    fn process_faces(&mut self, _id: usize, scene: &Choreography, play: &mut Playback, dt: f32) {
        for &index in scene.order.iter().take(play.next) {
            let event = &scene.data.events[index];
            if event.kind != EventType::Face
                || !event.active()
                || play.elapsed < event.start as f64
                || play.elapsed > event.end.unwrap_or(event.start) as f64
            {
                continue;
            }
            let (Some(target), Some(actor)) = (
                play.face_targets.get(&index).copied(),
                event.actor.and_then(|a| play.actors[a]),
            ) else {
                continue;
            };
            if self.states[actor].killed {
                continue;
            }
            let target_position = match target {
                crate::attention::Target::Player => self.player_feet,
                crate::attention::Target::Entity(t) if t == actor => continue,
                crate::attention::Target::Entity(t) => self.states[t].origin,
            };
            let forward = self.states[actor].rotation * Vec3::X;
            let current = forward.y.atan2(forward.x).to_degrees();
            let moving = self.states[actor].movement_animation.is_some();
            let latched = play.face_initial.entry(index).or_insert((current, moving));
            if latched.1 != moving {
                *latched = (current, moving);
            }
            if moving {
                continue;
            }
            let delta = target_position - self.states[actor].origin;
            if delta.truncate().length_squared() < 1e-6 {
                continue;
            }
            let goal = delta.y.atan2(delta.x).to_degrees();
            let initial = latched.0;
            let intensity = event.intensity(&scene.data, play.elapsed as f32);
            let ideal = initial + angle_diff(goal, initial) * intensity;
            let yaw = clamp_yaw(450. * dt.min(0.2), current, ideal);
            self.states[actor].rotation = physics::angles(Vec3::new(0., yaw, 0.));
        }
    }
    /// CBaseFlex::AddFlexAnimation: each active track blends its intensity into the actor's
    /// controller value by event intensity; stereo tracks drive right_/left_ controllers.
    fn process_flex_animation(&mut self, scene: &Choreography, play: &Playback) {
        let time = play.elapsed as f32;
        for &index in scene.order.iter().take(play.next) {
            let event = &scene.data.events[index];
            if event.kind != EventType::FlexAnimation
                || !event.active()
                || play.elapsed < event.start as f64
                || play.elapsed > event.end.unwrap_or(event.start) as f64
            {
                continue;
            }
            let Some(actor) = event.actor.and_then(|a| play.actors[a]) else {
                continue;
            };
            if self.states[actor].killed {
                continue;
            }
            let weight = event.intensity(&scene.data, time);
            let values = self.flex_controllers.entry(actor).or_default();
            for track in event.flex_tracks.iter().filter(|t| t.is_active()) {
                let name = track.controller.to_lowercase();
                let sides: &[(usize, String)] = &if track.is_combo() {
                    vec![(0, format!("right_{name}")), (1, format!("left_{name}"))]
                } else {
                    vec![(0, name)]
                };
                for (side, controller) in sides {
                    let value = track.intensity(time, event.start, event.end, *side);
                    let current = values.entry(controller.clone()).or_insert(0.);
                    *current = *current * (1. - weight) + value * weight;
                }
            }
        }
    }
    fn cancel_choreography(&mut self, id: usize) {
        if self
            .choreography
            .get_mut(&id)
            .is_some_and(|s| s.playback.take().is_some())
        {
            self.clear_scene_animations(id, true);
            self.fire(id, "OnCanceled", id);
        }
    }
    fn tick_choreography(&mut self, world: &World, dt: f32) {
        let ids: Vec<_> = self.choreography.keys().copied().collect();
        for id in ids {
            let mut scene = self.choreography.remove(&id).unwrap();
            if self.states[id].killed {
                scene.playback = None;
                self.clear_scene_animations(id, true);
            }
            if let Some(mut play) = scene.playback.take() {
                let mut canceled = false;
                let current_blocked = self.section_blocked(id, &scene.data, &play, play.elapsed);
                if let Some(ScenePause::Section { blocked, .. }) = &mut play.pause {
                    *blocked = current_blocked;
                }
                if let Some(ScenePause::Section { blocked, automated }) = &play.pause {
                    if let Some((resume, due)) = automated {
                        if self.time >= *due {
                            canceled = !resume;
                            play.pause = None;
                        }
                    } else if !blocked {
                        play.pause = None;
                    }
                }
                if canceled {
                    self.clear_scene_animations(id, true);
                    self.fire(id, "OnCanceled", id);
                } else {
                    if play.pause.is_none() {
                        let end = play.elapsed + dt as f64;
                        while let Some(index) = scene.order.get(play.next).copied() {
                            let event = &scene.data.events[index];
                            if event.start as f64 > end {
                                break;
                            }
                            play.next += 1;
                            if !event.active() || event.name.eq_ignore_ascii_case("NULL") {
                                continue;
                            }
                            let actor = event.actor.and_then(|actor| play.actors[actor]);
                            if event.actor.is_some() && actor.is_none() {
                                self.unsupported_input(
                                    "logic_choreographed_scene",
                                    "missing-actor",
                                );
                                continue;
                            }
                            self.diagnostics.scene_events_started += 1;
                            match event.kind {
                                EventType::FireTrigger => {
                                    if let Ok(trigger) = event.parameters[0].parse::<u8>() {
                                        if (1..=16).contains(&trigger) {
                                            self.fire(
                                                id,
                                                &format!("OnTrigger{trigger}"),
                                                actor.unwrap_or(id),
                                            );
                                        }
                                    }
                                }
                                EventType::StopPoint => {
                                    if !play.completed_early {
                                        play.completed_early = true;
                                        self.diagnostics.scene_completions += 1;
                                        self.fire(id, "OnCompletion", id);
                                    }
                                }
                                EventType::Speak => {
                                    if let Some(actor) =
                                        actor.filter(|_| !event.parameters[0].is_empty())
                                    {
                                        if let Some(mut request) = self.scene_sound_request(
                                            world,
                                            id,
                                            &scene.data.actors[event.actor.unwrap()].name,
                                            actor,
                                            &event.parameters[0],
                                        ) {
                                            // Scenes marked ignorePhonemes play without
                                            // lip sync (CVoiceData::ShouldIgnorePhonemes).
                                            if scene.data.ignore_phonemes {
                                                if let Some(a) = request.actor.as_mut() {
                                                    a.entity = None;
                                                }
                                            }
                                            self.sounds.push(request);
                                        }
                                    }
                                }
                                EventType::Gesture => {
                                    // Layers start and update in process_gestures each tick.
                                }
                                EventType::FlexAnimation => {
                                    // Tracks apply in process_flex_animation each tick.
                                }
                                EventType::Face => {
                                    // StartFacingSceneEvent needs a target; NPCs turn in process_faces.
                                    if let Some(actor) = actor
                                        .filter(|a| world.entities[*a].class().starts_with("npc_"))
                                    {
                                        if let Some(target) = self.scene_look_target(
                                            world,
                                            id,
                                            actor,
                                            &event.parameters[0],
                                        ) {
                                            play.face_targets.insert(index, target);
                                        } else {
                                            self.unsupported_input(
                                                "logic_choreographed_scene",
                                                "FACE:missing-target",
                                            );
                                        }
                                    }
                                }
                                EventType::LookAt => {
                                    // CBaseFlex LOOKAT refreshes NPC interests; non-NPC flex actors are a no-op.
                                    if let Some(actor) = actor
                                        .filter(|a| world.entities[*a].class().starts_with("npc_"))
                                    {
                                        if let Some(target) = self.scene_look_target(
                                            world,
                                            id,
                                            actor,
                                            &event.parameters[0],
                                        ) {
                                            play.look_targets.insert(index, target);
                                        } else {
                                            self.unsupported_input(
                                                "logic_choreographed_scene",
                                                "LOOKAT:missing-target",
                                            );
                                        }
                                    }
                                }
                                EventType::MoveTo => {
                                    if let Some(actor) = actor {
                                        let key = crate::npc::GoalKey {
                                            scene: id,
                                            event: index,
                                            actor,
                                        };
                                        let target = world
                                            .entities
                                            .iter()
                                            .enumerate()
                                            .find(|(target, e)| {
                                                !self.states[*target].killed
                                                    && e.get("targetname").is_some_and(|name| {
                                                        name.eq_ignore_ascii_case(
                                                            &event.parameters[0],
                                                        )
                                                    })
                                            })
                                            .map(|(id, _)| id);
                                        match (
                                            target,
                                            crate::npc::MoveStyle::parse(&event.parameters[1]),
                                        ) {
                                            (Some(target), Ok(style)) => {
                                                self.movement_ready.insert(key, false);
                                                self.movement_commands.push(
                                                    SceneMoveCommand::Start {
                                                        key,
                                                        request: crate::npc::MoveRequest {
                                                            target_entity: target,
                                                            target_feet: self.states[target].origin,
                                                            style,
                                                            event_distance: event.distance,
                                                            force_short: event.flags & 16 != 0,
                                                        },
                                                    },
                                                );
                                            }
                                            _ => self.unsupported_input(
                                                "logic_choreographed_scene",
                                                "MOVETO:target-or-style",
                                            ),
                                        }
                                    }
                                }
                                EventType::Sequence => {
                                    if let Some(actor) = actor {
                                        if self.states[actor].scripted_by.is_some()
                                            && event.flags & 32 == 0
                                        {
                                            self.unsupported_input(
                                                "logic_choreographed_scene",
                                                "SEQUENCE:scripted-actor-busy",
                                            );
                                        } else {
                                            self.restore_movement_animation(actor);
                                            self.restore_scene_animation(actor);
                                            let previous = SceneAnimation {
                                                owner: id,
                                                event: index,
                                                previous: self.states[actor].animation.clone(),
                                                previous_started: self.states[actor]
                                                    .animation_started,
                                                previous_done: self.states[actor].animation_done,
                                            };
                                            if self.animate(
                                                world,
                                                actor,
                                                &event.parameters[0],
                                                false,
                                            ) {
                                                self.states[actor].scene_animation = Some(previous);
                                            }
                                        }
                                    }
                                }
                                EventType::Section => {
                                    let blocked = self.section_blocked(
                                        id,
                                        &scene.data,
                                        &play,
                                        event.start as f64,
                                    );
                                    let tokens: Vec<_> =
                                        event.parameters[0].split_whitespace().collect();
                                    let automated = if tokens.len() == 3
                                        && tokens[0].eq_ignore_ascii_case("automate")
                                    {
                                        tokens[2]
                                            .parse::<f64>()
                                            .ok()
                                            .filter(|d| d.is_finite() && *d > 0.)
                                            .and_then(|d| {
                                                if tokens[1].eq_ignore_ascii_case("resume") {
                                                    Some((true, self.time + d))
                                                } else if tokens[1].eq_ignore_ascii_case("cancel") {
                                                    Some((false, self.time + d))
                                                } else {
                                                    None
                                                }
                                            })
                                    } else {
                                        None
                                    };
                                    let unknown_condition = scene
                                        .data
                                        .events
                                        .iter()
                                        .enumerate()
                                        .any(|(event_index, e)| {
                                            e.active()
                                                && e.resume_condition()
                                                && e.start <= event.start
                                                && e.actor.and_then(|a| play.actors[a]).is_some_and(
                                                    |actor| {
                                                        e.kind != EventType::MoveTo
                                                            || !self.movement_ready.contains_key(
                                                                &crate::npc::GoalKey {
                                                                    scene: id,
                                                                    event: event_index,
                                                                    actor,
                                                                },
                                                            )
                                                    },
                                                )
                                        });
                                    if blocked && automated.is_none() && unknown_condition {
                                        self.unsupported_input(
                                            "logic_choreographed_scene",
                                            "SECTION:actor-resume-condition",
                                        );
                                    }
                                    play.pause = Some(ScenePause::Section { blocked, automated });
                                }
                                EventType::Loop | EventType::SubScene => {
                                    self.unsupported_input(
                                        "logic_choreographed_scene",
                                        &format!("control:{:?}", event.kind),
                                    );
                                    play.pause = Some(ScenePause::UnsupportedControl);
                                }
                                EventType::Interrupt
                                | EventType::PermitResponses
                                | EventType::Unspecified => {
                                    self.unsupported_input(
                                        "logic_choreographed_scene",
                                        &format!("event:{:?}", event.kind),
                                    );
                                }
                                _ => self.unsupported_input(
                                    "logic_choreographed_scene",
                                    &format!("actor-event:{:?}", event.kind),
                                ),
                            }
                            if play.pause.is_some() {
                                play.elapsed = event.start as f64;
                                break;
                            }
                        }
                        if play.pause.is_none() {
                            play.elapsed = end;
                        }
                    }
                    // Preserve start-dispatch order for overlapping events. Pause freezes scene
                    // elapsed time while the global actor interest lifetime continues to refresh.
                    for &index in scene.order.iter().take(play.next) {
                        let event = &scene.data.events[index];
                        if event.kind != EventType::LookAt
                            || !event.active()
                            || play.elapsed < event.start as f64
                            || play.elapsed > event.end.unwrap_or(event.start) as f64
                        {
                            continue;
                        }
                        let Some(target) = play.look_targets.get(&index).copied() else {
                            continue;
                        };
                        let Some(actor) = event.actor.and_then(|a| play.actors[a]) else {
                            continue;
                        };
                        if self.states[actor].killed
                            || matches!(target, crate::attention::Target::Entity(t) if self.states[t].killed)
                        {
                            continue;
                        }
                        let importance = crate::attention::scene_importance(
                            event.intensity(&scene.data, play.elapsed as f32),
                            play.elapsed as f32 - event.start,
                        );
                        self.look_targets
                            .refresh(actor, target, importance, self.time, id, index);
                    }
                    if play.pause.is_none() {
                        self.process_gestures(id, &scene, &play);
                        self.process_faces(id, &scene, &mut play, dt);
                        self.process_flex_animation(&scene, &play);
                    }
                    if play.pause.is_none()
                        && play.next == scene.order.len()
                        && play.elapsed > scene.data.stop_time() as f64
                    {
                        if !play.completed_early {
                            self.diagnostics.scene_completions += 1;
                            self.fire(id, "OnCompletion", id);
                        }
                        self.clear_scene_animations(id, false);
                    } else {
                        for actor in 0..self.states.len() {
                            if self.states[actor]
                                .scene_animation
                                .as_ref()
                                .is_some_and(|a| {
                                    a.owner == id
                                        && play.elapsed
                                            > scene.data.events[a.event]
                                                .end
                                                .unwrap_or(scene.data.events[a.event].start)
                                                as f64
                                })
                            {
                                self.restore_scene_animation(actor);
                            }
                        }
                        scene.playback = Some(play);
                    }
                }
            }
            self.choreography.insert(id, scene);
        }
    }
    /// SDK FrameUpdatePostEntityThink soundscape pass for the player's ear (eye) position,
    /// after player movement; a newly active env_soundscape fires OnPlay.
    pub fn update_soundscape(&mut self, world: &World, ear: Vec3) {
        let states = &self.states;
        let enabled = |id: usize| states.get(id).is_some_and(|s| s.enabled && !s.killed);
        if let Some(id) = self.soundscape.update(world, ear, enabled) {
            self.fire(id, "OnPlay", id);
        }
    }
    /// SDK CTriggerHurt: Touch schedules HurtThink at once; HurtAllTouchers deals
    /// damage x 0.5 to every toucher that passes the filters every 0.5 s while it hurts
    /// anyone (damagemodel 1 doubles up to damagecap and forgives after 3 s without a
    /// hit); EndTouch deals damage x 0.5 to a toucher the last think did not hurt.
    fn trigger_hurt(&mut self, world: &World, id: usize, touching: Vec<usize>) {
        let e = &world.entities[id];
        let base = number(e, "damage", 0.);
        let cap = number(e, "damagecap", 20.);
        let doubling = number(e, "damagemodel", 0.) as u32 == 1;
        let time = self.time;
        let mut hurt = self.hurt.remove(&id).unwrap_or(HurtState {
            damage: base,
            next: None,
            reset_at: 0.,
            hurt: Vec::new(),
            touching: Vec::new(),
        });
        if !touching.is_empty() && hurt.next.is_none() {
            hurt.next = Some(time);
        }
        if hurt.next.is_some_and(|next| time >= next) {
            hurt.hurt.clear();
            let amount = hurt.damage * 0.5;
            let mut count = 0;
            for &target in &touching {
                if self.hurt_entity(world, id, target, amount) {
                    hurt.hurt.push(target);
                    count += 1;
                }
            }
            if doubling {
                if count == 0 {
                    if time > hurt.reset_at {
                        hurt.damage = base;
                    }
                } else {
                    hurt.damage = (hurt.damage * 2.).min(cap);
                    hurt.reset_at = time + 3.;
                }
            }
            hurt.next = (count > 0).then_some(time + 0.5);
        }
        for target in std::mem::take(&mut hurt.touching) {
            if !touching.contains(&target)
                && !hurt.hurt.contains(&target)
                && self.hurt_entity(world, id, target, hurt.damage * 0.5)
            {
                hurt.hurt.push(target);
            }
        }
        hurt.touching = touching;
        self.hurt.insert(id, hurt);
    }
    /// CTriggerHurt::HurtEntity: filters and m_takedamage, then TakeDamage (negative
    /// damage heals); the damage comes from the trigger's origin. OnHurtPlayer/OnHurt.
    fn hurt_entity(&mut self, world: &World, id: usize, target: usize, amount: f32) -> bool {
        let e = &world.entities[id];
        let flags = number(e, "spawnflags", 0.) as u32;
        let kind = number(e, "damagetype", 0.) as u32;
        let player = target == PLAYER;
        let passes = flags & 0x40 != 0 || flags & if player { 1 } else { 2 } != 0;
        let alive = if player {
            !self.player_dead
        } else {
            self.states.get(target).is_some_and(|s| !s.killed)
        };
        if !passes || !alive {
            return false;
        }
        if player {
            self.player_damage.push(crate::player_damage::DamageInfo {
                amount,
                kind,
                inflictor: self.states[id].origin,
            });
            self.fire(id, "OnHurtPlayer", usize::MAX);
        } else {
            self.entity_damage.push(crate::projectiles::Damage {
                target: crate::projectiles::DamageTarget::Entity(target),
                amount,
                dissolve: false,
                direction: Vec3::ZERO,
                origin: self.states[id].origin,
            });
            self.fire(id, "OnHurt", target);
        }
        true
    }
    pub fn fire(&mut self, id: usize, name: &str, activator: usize) {
        let Some(state) = self.states.get_mut(id) else {
            return;
        };
        for output in state
            .outputs
            .iter_mut()
            .filter(|o| o.name.eq_ignore_ascii_case(name) && o.remaining != 0)
        {
            if output.remaining > 0 {
                output.remaining -= 1;
            }
            self.sequence += 1;
            self.diagnostics.outputs_fired += 1;
            self.queue.push(Pending {
                due: self.time + output.delay,
                sequence: self.sequence,
                target: output.target.clone(),
                input: output.input.clone(),
                parameter: output.parameter.clone(),
                caller: id,
                activator,
                opener: None,
            });
        }
    }
    fn targets(&self, world: &World, p: &Pending) -> Vec<usize> {
        let target = p.target.to_lowercase();
        if target == "!self" || target == "!caller" {
            return vec![p.caller];
        }
        if target == "!activator" || target == "!player" {
            return vec![p.activator];
        }
        world
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.get("targetname")
                    .is_some_and(|n| glob(&target, &n.to_lowercase()))
            })
            .map(|(id, _)| id)
            .collect()
    }
    pub fn send(&mut self, id: usize, input: &str, parameter: &str) {
        self.sequence += 1;
        self.queue.push(Pending {
            due: self.time,
            sequence: self.sequence,
            target: format!("#{id}"),
            input: input.into(),
            parameter: parameter.into(),
            caller: id,
            activator: usize::MAX,
            opener: None,
        });
    }
    /// Console input uses the normal deferred I/O queue and target-name matching.
    pub fn send_named(&mut self, target: &str, input: &str, parameter: &str, delay: f64) -> bool {
        if target.is_empty()
            || input.is_empty()
            || !delay.is_finite()
            || delay < 0.
            || !(self.time + delay).is_finite()
        {
            return false;
        }
        self.sequence += 1;
        self.queue.push(Pending {
            due: self.time + delay,
            sequence: self.sequence,
            target: target.into(),
            input: input.into(),
            parameter: parameter.into(),
            caller: usize::MAX,
            activator: usize::MAX,
            opener: None,
        });
        true
    }
    /// An NPC blocked by a closed door opens it away from itself; locked doors stay shut.
    pub fn npc_open_door(&mut self, world: &World, id: usize, opener: Vec3) {
        let Some(state) = self.states.get(id) else {
            return;
        };
        if state.killed || state.locked || state.target != 0. {
            return;
        }
        match world.entities[id].class() {
            "prop_door_rotating" => self.use_with_opener(world, id, Some(opener)),
            "func_door" | "func_door_rotating" => self.send(id, "Open", ""),
            _ => {}
        }
    }
    pub fn use_entity(&mut self, world: &World, id: usize) {
        self.use_with_opener(world, id, None);
    }
    pub fn use_entity_at(&mut self, world: &World, id: usize, player_origin: Vec3) {
        self.use_with_opener(world, id, Some(player_origin));
    }
    fn use_with_opener(&mut self, world: &World, id: usize, opener: Option<Vec3>) {
        let Some(entity) = world.entities.get(id) else {
            return;
        };
        if entity.class() == "prop_door_rotating" {
            let opening = self.states[id].target == 0.;
            if opening && self.states[id].locked {
                self.send(id, "Open", "");
                return;
            }
            if !opening && number(entity, "spawnflags", 0.) as u32 & 8192 == 0 {
                return;
            }
            let group = entity
                .get("slavename")
                .filter(|s| !s.is_empty())
                .or_else(|| entity.get("targetname").filter(|s| !s.is_empty()));
            let ids: Vec<_> = world
                .entities
                .iter()
                .enumerate()
                .filter(|(other, e)| {
                    *other == id
                        || e.class() == "prop_door_rotating"
                            && group.is_some_and(|name| e.get("targetname") == Some(name))
                })
                .map(|(i, _)| i)
                .collect();
            for door in ids {
                self.send(door, if opening { "Open" } else { "Close" }, "");
                self.queue.last_mut().expect("queued door use").opener = opener;
            }
        } else if matches!(entity.class(), "func_door" | "func_door_rotating") {
            self.send(id, "Toggle", "");
        } else if entity.class() == "func_button" {
            self.fire(id, "OnPressed", usize::MAX);
        }
    }
    fn deliver(&mut self, world: &World, id: usize, p: &Pending, player_feet: Vec3) {
        if id >= self.states.len() {
            self.unsupported_input("player", &p.input);
            return;
        }
        if self.states[id].killed {
            return;
        }
        self.diagnostics.inputs_delivered += 1;
        let e = &world.entities[id];
        let class = e.class();
        let input = p.input.to_lowercase();
        let value = p.parameter.parse::<f32>().unwrap_or(0.);
        let door = matches!(
            class,
            "func_door" | "func_door_rotating" | "prop_door_rotating" | "func_movelinear"
        );
        if self
            .monitors
            .input(world, &self.states, id, &input, &p.parameter, self.time)
        {
            return;
        }
        if class == "env_global" && self.globals.input(e, &input, &p.parameter).0 {
            return;
        }
        match input.as_str() {
            // CEnvTonemapController: custom auto-exposure limits and the manual tonemap rate.
            "setautoexposuremin" if class == "env_tonemap_controller" => self.tonemap.min = value,
            "setautoexposuremax" if class == "env_tonemap_controller" => self.tonemap.max = value,
            "settonemaprate" if class == "env_tonemap_controller" => self.tonemap.rate = value,
            "setbloomscale" if class == "env_tonemap_controller" => self.tonemap.bloom = value,
            "usedefaultautoexposure" if class == "env_tonemap_controller" => {
                self.tonemap = crate::tonemap::Control {
                    rate: self.tonemap.rate,
                    bloom: self.tonemap.bloom,
                    ..Default::default()
                };
            }
            "start" if class == "logic_choreographed_scene" => {
                self.start_choreography(world, id, p.activator)
            }
            "pause" if class == "logic_choreographed_scene" => {
                if let Some(play) = self
                    .choreography
                    .get_mut(&id)
                    .and_then(|s| s.playback.as_mut())
                {
                    play.pause = Some(ScenePause::Input);
                }
            }
            "resume" if class == "logic_choreographed_scene" => {
                if let Some(play) = self
                    .choreography
                    .get_mut(&id)
                    .and_then(|s| s.playback.as_mut())
                {
                    if !matches!(play.pause, Some(ScenePause::UnsupportedControl)) {
                        play.pause = None;
                    }
                }
            }
            "cancel" if class == "logic_choreographed_scene" => self.cancel_choreography(id),
            "forcespawn" if class == "point_template" => {
                // Templates spawn copies; each authored child is created once here. Repeat
                // spawns would need new entity instances, which are not implemented.
                if let Some(children) = self.templates.remove(&id) {
                    for (child, visible, enabled) in children {
                        let state = &mut self.states[child];
                        state.killed = false;
                        state.visible = visible;
                        state.enabled = enabled;
                    }
                    self.fire(id, "OnEntitySpawned", p.activator);
                } else {
                    self.unsupported_input("point_template", "ForceSpawn:repeat");
                }
            }
            "fade" if class == "env_fade" => {
                // CEnvFade::InputFade: SF_FADE_IN 1, MODULATE 2, ONLYONE 4 (the activating
                // player only), STAYOUT 8; everyone else gets FFADE_PURGE.
                use crate::player_damage::*;
                let flags = number(e, "spawnflags", 0.) as u32;
                let mut fade = if flags & 1 != 0 { FFADE_IN } else { FFADE_OUT };
                if flags & 2 != 0 {
                    fade |= FFADE_MODULATE;
                }
                if flags & 8 != 0 {
                    fade |= FFADE_STAYOUT;
                }
                if flags & 4 == 0 {
                    fade |= FFADE_PURGE;
                }
                if flags & 4 == 0 || p.activator == usize::MAX {
                    let rgb = parse_vec3(e.get("rendercolor").unwrap_or("255 255 255"))
                        .unwrap_or(Vec3::splat(255.));
                    let alpha = number(e, "renderamt", 255.);
                    let byte = |v: f32| v.clamp(0., 255.) as u8;
                    self.screen_fades.push(ScreenFade::new(
                        [byte(rgb.x), byte(rgb.y), byte(rgb.z), byte(alpha)],
                        number(e, "duration", 0.),
                        number(e, "holdtime", 0.),
                        fade,
                    ));
                }
                self.fire(id, "OnBeginFade", p.activator);
            }
            "kill" => {
                self.states[id].killed = true;
                self.states[id].visible = false;
                self.states[id].enabled = false;
            }
            "enable" | "disable" | "toggle" if class == "npc_combine_camera" => {
                // CNPC_CombineCamera Enable/Disable/Toggle drive its open/closed activity.
                let enabled = match input.as_str() {
                    "enable" => true,
                    "disable" => false,
                    _ => !self.states[id].enabled,
                };
                if enabled != self.states[id].enabled {
                    self.states[id].enabled = enabled;
                    self.camera_activity(world, id, true);
                }
            }
            "enable" => {
                self.states[id].enabled = true;
                if matches!(class, "func_brush" | "func_monitor") {
                    self.states[id].visible = true;
                }
                self.states[id].timer_at =
                    self.time + number(e, "RefireTime", 1.).max(0.015) as f64;
            }
            "disable" => {
                self.states[id].enabled = false;
                if matches!(class, "func_brush" | "func_monitor") {
                    self.states[id].visible = false;
                }
            }
            "toggleenabled" if class.starts_with("env_soundscape") => {
                self.states[id].enabled = !self.states[id].enabled;
            }
            "toggle" if !door => {
                self.states[id].enabled = !self.states[id].enabled;
                if matches!(class, "func_brush" | "func_monitor") {
                    self.states[id].visible = self.states[id].enabled;
                }
            }
            "turnon" => self.states[id].visible = true,
            "turnoff" => self.states[id].visible = false,
            "setparent" => {
                let mut lookup = p.clone();
                lookup.target = p.parameter.clone();
                match self.targets(world, &lookup).first() {
                    Some(&parent) if !p.parameter.is_empty() && parent != id => {
                        self.set_parent(world, id, parent, None, true)
                    }
                    _ => self.states[id].parent = None,
                }
            }
            "setparentattachment" | "setparentattachmentmaintainoffset" => {
                // CBaseEntity::InputSetParentAttachment needs an existing parent; without
                // MaintainOffset the child moves onto the attachment.
                if let Some(parent) = self.states[id].parent.as_ref().map(|p| p.entity) {
                    let keep = input == "setparentattachmentmaintainoffset";
                    self.set_parent(world, id, parent, Some(p.parameter.clone()), keep);
                }
            }
            "clearparent" => self.states[id].parent = None,
            "lock" => self.states[id].locked = true,
            "unlock" => self.states[id].locked = false,
            "open" | "close" | "toggle" | "openawayfrom"
                if door && (input != "openawayfrom" || class == "prop_door_rotating") =>
            {
                if self.states[id].locked && input != "close" {
                    self.fire(id, "OnLockedUse", p.activator);
                    return;
                }
                if class == "prop_door_rotating"
                    && matches!(input.as_str(), "open" | "openawayfrom")
                    && self.states[id].target > 0.
                {
                    return;
                }
                let target = if input == "open" || input == "openawayfrom" {
                    1.
                } else if input == "close" {
                    0.
                } else {
                    1. - self.states[id].target
                };
                if class == "prop_door_rotating"
                    && target > 0.
                    && self.states[id].fraction.abs() < 0.00001
                {
                    // Resolve the named opener when the deferred input is delivered,
                    // using its current origin and the original caller/activator.
                    let opener = if input == "openawayfrom" {
                        let name = p.parameter.to_lowercase();
                        if name == "!player" || name == "!activator" && p.activator == usize::MAX {
                            Some(player_feet)
                        } else {
                            let mut lookup = p.clone();
                            lookup.target = p.parameter.clone();
                            self.targets(world, &lookup).into_iter().find_map(|other| {
                                self.states
                                    .get(other)
                                    .filter(|s| !s.killed)
                                    .map(|s| s.origin)
                            })
                        }
                    } else {
                        p.opener
                    };
                    let state = &mut self.states[id];
                    let back = match number(e, "opendir", 0.) as i32 {
                        1 => false,
                        2 => true,
                        _ => opener.is_some_and(|origin| {
                            (state.rotation * Vec3::X).dot(origin - state.origin) > 0.
                        }),
                    };
                    state.angle = state.angle.abs() * if back { 1. } else { -1. };
                }
                self.states[id].target = target;
                self.states[id].return_at = None;
                self.fire(
                    id,
                    if target > 0. { "OnOpen" } else { "OnClose" },
                    p.activator,
                );
            }
            "setposition" if class == "func_movelinear" => {
                self.states[id].target = value.clamp(0., 1.)
            }
            "trigger" if class == "logic_relay" => {
                if self.states[id].enabled {
                    self.fire(id, "OnTrigger", p.activator);
                    if number(e, "spawnflags", 0.) as u32 & 1 != 0 {
                        self.states[id].enabled = false;
                    }
                }
            }
            "firetimer" => self.fire(id, "OnTimer", p.activator),
            "add" | "subtract" | "setvalue" | "setvaluenofire" if class == "math_counter" => {
                let min = number(e, "min", 0.);
                let max = number(e, "max", 0.);
                let old = self.states[id].value;
                let mut next = match input.as_str() {
                    "add" => old + value,
                    "subtract" => old - value,
                    _ => value,
                };
                if max > min {
                    next = next.clamp(min, max);
                }
                self.states[id].value = next;
                if input != "setvaluenofire" {
                    self.fire_value(id, "OutValue", next, p.activator);
                    if max > min && next >= max && old < max {
                        self.fire(id, "OnHitMax", p.activator);
                    }
                    if max > min && next <= min && old > min {
                        self.fire(id, "OnHitMin", p.activator);
                    }
                }
            }
            "setvalue" | "setvaluetest" | "test" if class == "logic_branch" => {
                if input != "test" {
                    self.states[id].value = value;
                }
                if input != "setvalue" {
                    self.fire(
                        id,
                        if self.states[id].value != 0. {
                            "OnTrue"
                        } else {
                            "OnFalse"
                        },
                        p.activator,
                    );
                }
            }
            "invalue" if class == "logic_case" => {
                let matched =
                    (1..=16).find(|i| e.get(&format!("Case{i:02}")) == Some(p.parameter.as_str()));
                self.fire(
                    id,
                    &matched.map_or("OnDefault".into(), |i| format!("OnCase{i:02}")),
                    p.activator,
                );
            }
            "playsound" | "stopsound" | "togglesound" if class == "ambient_generic" => {
                // SDK CAmbientGeneric: looping sounds are active from spawn unless they start
                // silent; PlaySound restarts an inactive sound, StopSound stops an active one,
                // and only looping sounds stay active.
                let flags = e
                    .get("spawnflags")
                    .and_then(|v| v.trim().parse::<u32>().ok())
                    .unwrap_or(0);
                let looping = flags & 32 == 0;
                let active = *self
                    .ambient_active
                    .get(&id)
                    .unwrap_or(&(looping && flags & 16 == 0));
                let play = match input.as_str() {
                    "playsound" => !active,
                    "stopsound" => false,
                    _ => !active,
                };
                if let Some(sound) = e.get("message").filter(|m| !m.is_empty()) {
                    if play {
                        self.ambient_active.insert(id, looping);
                        self.sounds.push(crate::sounds::SoundRequest {
                            name: sound.into(),
                            actor: None,
                            ambient: Some(crate::sounds::AmbientControl::Play(id)),
                            volume: None,
                            origin: None,
                        });
                    } else if active && input != "playsound" {
                        self.ambient_active.insert(id, false);
                        self.sounds.push(crate::sounds::SoundRequest {
                            name: sound.into(),
                            actor: None,
                            ambient: Some(crate::sounds::AmbientControl::Stop(id)),
                            volume: None,
                            origin: None,
                        });
                    }
                }
            }
            "changelevel" if class == "trigger_changelevel" => {
                if let Some(map) = e.get("map") {
                    self.transition = Some((map.into(), e.get("landmark").unwrap_or("").into()));
                }
            }
            "sethealth" if class.starts_with("npc_") => self.states[id].value = value,
            "setdefaultanimation" if class.starts_with("prop_dynamic") => {
                // CDynamicProp stores the name without restarting or validating the active clip.
                self.states[id].default_animation = p.parameter.clone();
            }
            "setanimation" if class.starts_with("prop_dynamic") || class.starts_with("npc_") => {
                self.animate(world, id, &p.parameter, true);
            }
            "beginsequence" if class == "scripted_sequence" => self.begin_sequence(world, id),
            "cancelsequence" if class == "scripted_sequence" => self.end_sequence(world, id, true),
            "pickrandom" | "pickrandomshuffle" if class == "logic_case" => {
                let cases = self.states[id]
                    .outputs
                    .iter()
                    .filter_map(|o| {
                        o.name
                            .strip_prefix("OnCase")
                            .and_then(|n| n.parse::<usize>().ok())
                    })
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                if !cases.is_empty() {
                    self.random = self
                        .random
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1);
                    let chosen = if input == "pickrandomshuffle" {
                        let state = &mut self.states[id];
                        let new_batch = state.shuffle.is_empty();
                        if new_batch {
                            state.shuffle = cases;
                        }
                        let candidates: Vec<_> = state
                            .shuffle
                            .iter()
                            .enumerate()
                            .filter(|(_, case)| {
                                !new_batch
                                    || state.shuffle.len() == 1
                                    || Some(**case) != state.last_shuffle
                            })
                            .map(|(index, _)| index)
                            .collect();
                        let index = candidates[(self.random >> 32) as usize % candidates.len()];
                        let case = state.shuffle.swap_remove(index);
                        state.last_shuffle = Some(case);
                        case
                    } else {
                        cases[(self.random >> 32) as usize % cases.len()]
                    };
                    self.fire(id, &format!("OnCase{chosen:02}"), p.activator);
                }
            }
            _ => self.unsupported_input(class, &p.input),
        }
    }
    fn animate(&mut self, world: &World, id: usize, name: &str, finish: bool) -> bool {
        let key = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(id))
            .map(|i| i.asset_key());
        let rig = key.as_ref().and_then(|key| world.rigs.get(key));
        let random = &mut self.random;
        let resolved = rig.and_then(|r| {
            r.lookup_sequence(name, None, |upper| {
                *random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                (*random >> 32) as u32 % upper
            })
        });
        let clip = rig.zip(resolved).and_then(|(r, label)| r.clips.get(label));
        let Some((clip, resolved)) = clip.zip(resolved) else {
            self.unsupported_input(world.entities[id].class(), &format!("animation:{name}"));
            return false;
        };
        self.states[id].animation = resolved.to_owned();
        self.states[id].scene_animation = None;
        self.states[id].animation_started = self.time;
        self.states[id].animation_done =
            finish.then_some(self.time + clip.duration().max(0.015) as f64);
        true
    }
    /// Combine camera activity for its enabled state (MaintainActivity toward the ideal
    /// activity): open idle when enabled; when disabled the close transition plays first
    /// (if `transition`), then closed idle. The reversed opening transition is not modeled.
    fn camera_activity(&mut self, world: &World, id: usize, transition: bool) {
        let open = self.states[id].animation.eq_ignore_ascii_case("idlealert");
        if self.states[id].enabled {
            self.animate(world, id, "ACT_COMBINE_CAMERA_OPEN_IDLE", false);
        } else if transition && open {
            self.animate(world, id, "ACT_COMBINE_CAMERA_CLOSE", true);
        } else {
            self.animate(world, id, "ACT_COMBINE_CAMERA_CLOSED_IDLE", false);
        }
    }
    fn begin_sequence(&mut self, world: &World, id: usize) {
        let script = &world.entities[id];
        let name = script.get("m_iszEntity").unwrap_or("");
        let actor = world
            .entities
            .iter()
            .enumerate()
            .filter(|(i, e)| {
                !self.states[*i].killed
                    && e.class().starts_with("npc_")
                    && (e.get("targetname") == Some(name) || e.class() == name)
            })
            .min_by(|(a, _), (b, _)| {
                self.states[*a]
                    .origin
                    .distance_squared(script.origin())
                    .total_cmp(&self.states[*b].origin.distance_squared(script.origin()))
            })
            .map(|(i, _)| i);
        let Some(actor) = actor else {
            self.unsupported_input("scripted_sequence", &format!("actor:{name}"));
            return;
        };
        if self.states[actor]
            .scripted_by
            .is_some_and(|owner| owner != id)
        {
            self.sequence += 1;
            self.queue.push(Pending {
                due: self.time + 0.25,
                sequence: self.sequence,
                target: format!("#{id}"),
                input: "BeginSequence".into(),
                parameter: String::new(),
                caller: id,
                activator: usize::MAX,
                opener: None,
            });
            return;
        }
        let movement = number(script, "m_fMoveTo", 0.) as i32;
        if movement == 4 {
            self.states[actor].origin = script.origin();
            self.states[actor].rotation = self.states[id].rotation;
        } else if movement != 0 {
            self.unsupported_input("scripted_sequence", &format!("movement:{movement}"));
            return;
        }
        let play = script.get("m_iszPlay").unwrap_or("");
        if play.is_empty() {
            // Retail StartSequence completes a null action immediately, allowing the authored post idle.
            self.states[actor].scripted_by = Some(id);
            self.states[id].script_actor = Some(actor);
            self.fire(id, "OnBeginSequence", actor);
            self.end_sequence(world, id, false);
            return;
        }
        if !self.animate(world, actor, play, false) {
            return;
        }
        let duration = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(actor))
            .and_then(|i| world.rigs.get(&i.asset_key()))
            .and_then(|r| r.clips.get(&self.states[actor].animation))
            .map(|c| c.duration())
            .unwrap_or(0.);
        self.states[actor].scripted_by = Some(id);
        self.states[id].script_actor = Some(actor);
        self.states[id].script_finish = Some(self.time + duration.max(0.015) as f64);
        self.fire(id, "OnBeginSequence", actor);
    }
    /// SCRIPT_EVENT_FIREEVENT (1003) animation events crossed this tick by the actor's
    /// scripted clip fire the script's OnScriptEventNN output (options = NN).
    fn script_events(&mut self, world: &World, id: usize) {
        let Some(actor) = self.states[id].script_actor else {
            return;
        };
        let state = &self.states[actor];
        let Some(clip) = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(actor))
            .and_then(|i| world.rigs.get(&i.asset_key()))
            .and_then(|r| r.clips.get(&state.animation))
        else {
            return;
        };
        let duration = clip.duration().max(0.015);
        let now = ((self.time - state.animation_started) as f32 / duration).min(1.);
        let before = now - self.tick_dt / duration;
        let fired: Vec<String> = clip
            .events
            .iter()
            .filter(|e| e.id == 1003 && e.cycle > before && e.cycle <= now)
            .filter_map(|e| e.options.trim().parse::<u32>().ok())
            .map(|n| format!("OnScriptEvent{n:02}"))
            .collect();
        for output in fired {
            self.fire(id, &output, actor);
        }
    }
    fn end_sequence(&mut self, world: &World, id: usize, cancel: bool) {
        let actor = self.states[id].script_actor.take();
        self.states[id].script_finish = None;
        self.states[id].post_idle_done = None;
        if let Some(actor) = actor {
            self.states[actor].scripted_by = None;
            let name = world.entities[id]
                .get("m_iszPostIdle")
                .filter(|s| !s.is_empty())
                .or(world.entities[actor].get("DefaultAnim"))
                .unwrap_or_else(|| {
                    let preferred = if world.entities[actor].class() == "npc_metropolice" {
                        "idle_baton"
                    } else {
                        "idle_subtle"
                    };
                    let rig = world
                        .model_instances
                        .iter()
                        .find(|i| i.entity == Some(actor))
                        .and_then(|i| world.rigs.get(&i.asset_key()));
                    if rig.is_some_and(|r| {
                        !r.clips.contains_key(preferred)
                            && r.sequences
                                .iter()
                                .any(|s| s.activity.eq_ignore_ascii_case("ACT_IDLE"))
                    }) {
                        "ACT_IDLE"
                    } else {
                        preferred
                    }
                });
            let animated = self.animate(world, actor, name, false);
            self.fire(
                id,
                if cancel {
                    "OnCancelSequence"
                } else {
                    "OnEndSequence"
                },
                actor,
            );
            if !cancel {
                let duration = if animated
                    && world.entities[id]
                        .get("m_iszPostIdle")
                        .is_some_and(|n| !n.is_empty())
                {
                    world
                        .model_instances
                        .iter()
                        .find(|i| i.entity == Some(actor))
                        .and_then(|i| world.rigs.get(&i.asset_key()))
                        .and_then(|r| r.clips.get(&self.states[actor].animation))
                        .map_or(0., |c| c.duration())
                } else {
                    0.
                };
                self.states[id].post_idle_done = Some((self.time + duration as f64, actor));
            }
        }
    }
    fn fire_value(&mut self, id: usize, name: &str, value: f32, activator: usize) {
        let before = self.sequence;
        self.fire(id, name, activator);
        for p in self
            .queue
            .iter_mut()
            .filter(|p| p.sequence > before && p.parameter.is_empty())
        {
            p.parameter = value.to_string();
        }
    }
    fn unsupported_input(&mut self, class: &str, input: &str) {
        let key = format!("{class}.{input}");
        *self.diagnostics.unsupported.entry(key.clone()).or_default() += 1;
        if self.unsupported.insert(key.clone()) {
            eprintln!("Unimplemented entity input: {key}");
        }
    }
    pub fn tick(&mut self, world: &World, player_feet: Vec3, dt: f32) {
        self.time += dt as f64;
        self.tick_dt = dt;
        self.player_feet = player_feet;
        self.monitors.tick(&self.states, self.time);
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
            let e = &world.entities[id];
            if self.states[id].killed {
                continue;
            }
            if let Some((due, actor)) = self.states[id].post_idle_done {
                if due <= self.time {
                    self.states[id].post_idle_done = None;
                    self.fire(id, "OnPostIdleEndSequence", actor);
                }
            }
            if e.class() == "scripted_sequence" {
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
                if self.states[id].return_at.is_some_and(|t| t <= self.time) {
                    self.send(id, "Close", "");
                    self.states[id].return_at = None;
                }
                let s = &mut self.states[id];
                if (s.target - s.fraction).abs() > 0.00001 {
                    s.fraction += (s.target - s.fraction).clamp(-s.rate * dt, s.rate * dt);
                    s.origin = s.base_origin + s.translation * s.fraction;
                    s.rotation =
                        s.base_rotation * Quat::from_axis_angle(s.axis, s.angle * s.fraction);
                    if (s.target - s.fraction).abs() < 0.00001 {
                        let open = s.target > 0.;
                        if open && s.wait >= 0. {
                            s.return_at = Some(self.time + s.wait);
                        }
                        self.diagnostics.door_completions += 1;
                        self.fire(
                            id,
                            if open { "OnFullyOpen" } else { "OnFullyClosed" },
                            usize::MAX,
                        );
                    }
                }
            }
            if e.class() == "logic_timer"
                && self.states[id].enabled
                && self.states[id].timer_at <= self.time
            {
                self.states[id].timer_at =
                    self.time + number(e, "RefireTime", 1.).max(0.015) as f64;
                self.fire(id, "OnTimer", usize::MAX);
            }
            if e.class().starts_with("trigger_") && self.states[id].enabled {
                // CBaseTrigger PassesTriggerFilters: clients (1), NPCs (2), everything (0x40).
                // Other trigger classes keep the earlier player-only touch.
                let gated = matches!(e.class(), "trigger_once" | "trigger_multiple");
                let flags = number(e, "spawnflags", 1.) as u32;
                let clients = !gated || flags & (1 | 0x40) != 0;
                let npcs = gated && flags & (2 | 0x40) != 0;
                let player = clients && self.player_inside(world, id, player_feet);
                let npc = if npcs && !player {
                    (0..self.states.len()).find(|&n| {
                        world.entities[n].class().starts_with("npc_")
                            && !self.states[n].killed
                            && self.states[n].visible
                            && self.player_inside(world, id, self.states[n].origin)
                    })
                } else {
                    None
                };
                let inside = player || npc.is_some();
                let activator = npc.unwrap_or(usize::MAX);
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
                            world.entities[n].class().starts_with("npc_")
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
    fn player_inside(&self, world: &World, id: usize, feet: Vec3) -> bool {
        let e = &world.entities[id];
        let Some(model) = e
            .get("model")
            .and_then(|s| s.strip_prefix('*'))
            .and_then(|s| s.parse::<usize>().ok())
            .and_then(|id| world.brush_models.iter().find(|m| m.id == id))
        else {
            return false;
        };
        let state = &self.states[id];
        let center = feet + Vec3::Z * 36.;
        model.brushes.iter().any(|b| {
            !b.planes.is_empty()
                && b.planes.iter().all(|p| {
                    let normal = state.rotation * p.normal;
                    normal.dot(center - state.origin)
                        <= p.distance + normal.abs().dot(Vec3::new(16., 16., 36.))
                })
        })
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
fn glob(pattern: &str, value: &str) -> bool {
    if let Some((first, last)) = pattern.split_once('*') {
        value.starts_with(first) && value.ends_with(last)
    } else {
        pattern == value
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn entity(class: &str, name: &str, values: &[(&str, &str)]) -> Entity {
        Entity {
            properties: [("classname", class), ("targetname", name)]
                .into_iter()
                .chain(values.iter().copied())
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }
    #[test]
    fn ambient_inputs_follow_sdk_active_state() {
        use crate::sounds::AmbientControl::{Play, Stop};
        let world = World {
            entities: vec![
                // Looping, start silent, play everywhere: a barrier's touch hum.
                entity(
                    "ambient_generic",
                    "close",
                    &[("message", "hum"), ("spawnflags", "17")],
                ),
                // Not looping: every PlaySound plays it again.
                entity(
                    "ambient_generic",
                    "once",
                    &[("message", "beep"), ("spawnflags", "48")],
                ),
            ],
            ..World::default()
        };
        let mut scene = Scene::new(&world);
        let run = |scene: &mut Scene, id: usize, input: &str| {
            scene.send(id, input, "");
            scene.tick(&world, Vec3::ZERO, 0.015);
            std::mem::take(&mut scene.sounds)
                .into_iter()
                .filter_map(|r| r.ambient)
                .collect::<Vec<_>>()
        };
        assert_eq!(run(&mut scene, 0, "PlaySound"), [Play(0)]);
        assert_eq!(run(&mut scene, 0, "PlaySound"), []);
        assert_eq!(run(&mut scene, 0, "StopSound"), [Stop(0)]);
        assert_eq!(run(&mut scene, 0, "StopSound"), []);
        assert_eq!(run(&mut scene, 0, "ToggleSound"), [Play(0)]);
        assert_eq!(run(&mut scene, 0, "ToggleSound"), [Stop(0)]);
        assert_eq!(run(&mut scene, 1, "PlaySound"), [Play(1)]);
        assert_eq!(run(&mut scene, 1, "PlaySound"), [Play(1)]);
        assert_eq!(run(&mut scene, 1, "StopSound"), []);
    }
    fn choreography(events: &[(EventType, f32, &str)]) -> Arc<ChoreoScene> {
        let mut data = b"bvcd\x04".to_vec();
        data.extend(0u32.to_le_bytes());
        data.push(events.len() as u8);
        let mut strings = vec!["event".into(), "".into()];
        for (kind, start, parameter) in events {
            data.push(*kind as u8);
            data.extend(0i16.to_le_bytes());
            data.extend(start.to_le_bytes());
            data.extend((-1f32).to_le_bytes());
            data.extend((strings.len() as i16).to_le_bytes());
            strings.push((*parameter).into());
            data.extend(1i16.to_le_bytes());
            data.extend(1i16.to_le_bytes());
            data.extend([0, 8]);
            data.extend(0f32.to_le_bytes());
            data.extend([0, 0, 0, 0]);
            if *kind == EventType::Gesture {
                data.extend((-1f32).to_le_bytes());
            }
            data.extend([0, 0]);
            if *kind == EventType::Loop {
                data.push(255);
            }
            if *kind == EventType::Speak {
                data.push(0);
                data.extend(1i16.to_le_bytes());
                data.push(0);
            }
        }
        data.extend([0, 0, 0]);
        Arc::new(ChoreoScene::parse(&data, &strings).unwrap())
    }
    #[test]
    fn face_turns_standing_npc_toward_target_at_max_yaw_speed() {
        let world = World {
            entities: vec![
                entity("logic_choreographed_scene", "scene", &[]),
                entity("npc_citizen", "actor", &[]),
                entity("info_target", "mark", &[("origin", "0 100 0")]),
            ],
            ..Default::default()
        };
        let mut data = Arc::unwrap_or_clone(choreography(&[(EventType::Face, 0., "mark")]));
        data.actors.push(source_assets::scenes::Actor {
            name: "actor".into(),
            active: true,
            channels: Vec::new(),
        });
        data.events[0].actor = Some(0);
        data.events[0].end = Some(2.);
        let mut scene = Scene::new(&world);
        scene.install_choreography(0, Arc::new(data));
        let yaw = |scene: &Scene| {
            let f = scene.states[1].rotation * Vec3::X;
            f.y.atan2(f.x).to_degrees()
        };
        assert!(yaw(&scene).abs() < 1e-3);
        scene.send(0, "Start", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.tick(&world, Vec3::ZERO, 0.1);
        // 450 degrees per second: 45 degrees in this 0.1 sec step toward the 90 degree goal.
        let partial = yaw(&scene);
        assert!(partial > 40. && partial < 90., "{partial}");
        for _ in 0..5 {
            scene.tick(&world, Vec3::ZERO, 0.1);
        }
        assert!((yaw(&scene) - 90.).abs() < 1e-3, "{}", yaw(&scene));
        assert!(!scene
            .diagnostics
            .unsupported
            .contains_key("logic_choreographed_scene.actor-event:Face"));
        assert_eq!(angle_diff(10., 350.), 20.);
        assert_eq!(clamp_yaw(5., 358., 10.), 3.);
    }
    #[test]
    fn lookat_binds_target_alias_once_and_reports_missing_target_without_cycler_interest() {
        use crate::attention::Target;
        let mut world = World {
            entities: vec![
                entity("logic_choreographed_scene", "scene", &[("target1", "MARK")]),
                entity("npc_citizen", "actor", &[]),
                entity("cycler_actor", "cycler", &[]),
                entity("info_target", "mark", &[]),
            ],
            ..Default::default()
        };
        let mut data = Arc::unwrap_or_clone(choreography(&[
            (EventType::LookAt, 0., "!TaRgEt1"),
            (EventType::LookAt, 0., "!player"),
            (EventType::LookAt, 0.1, "absent"),
        ]));
        for name in ["actor", "cycler"] {
            data.actors.push(source_assets::scenes::Actor {
                name: name.into(),
                active: true,
                channels: Vec::new(),
            });
        }
        for (id, e) in data.events.iter_mut().enumerate() {
            e.actor = Some(if id == 1 { 1 } else { 0 });
            e.end = Some(1.);
        }
        let mut scene = Scene::new(&world);
        scene.install_choreography(0, Arc::new(data));
        scene.send(0, "Start", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.tick(&world, Vec3::ZERO, 0.05);
        assert_eq!(scene.look_targets.report()[&1][0].target, Target::Entity(3));
        assert!(!scene.look_targets.report().contains_key(&2));
        // Renaming a surviving entity does not re-resolve an already-started event.
        world.entities[3].properties = vec![
            ("classname".into(), "info_target".into()),
            ("targetname".into(), "renamed".into()),
        ];
        scene.tick(&world, Vec3::ZERO, 0.1);
        assert_eq!(scene.look_targets.report()[&1][0].target, Target::Entity(3));
        assert_eq!(
            scene.diagnostics.unsupported["logic_choreographed_scene.LOOKAT:missing-target"],
            1
        );
    }
    #[test]
    fn cycler_actor_named_and_target_aliases_deliver_real_actor_events() {
        let world = World {
            entities: vec![
                entity(
                    "logic_choreographed_scene",
                    "scene",
                    &[("target1", "gman"), ("OnTrigger1", "gate,Add,1,0,-1")],
                ),
                // A same-name non-actor must not shadow the flex-capable actor.
                entity("logic_relay", "gman", &[]),
                entity("cycler_actor", "gman", &[]),
                entity("math_counter", "gate", &[]),
                entity("cycler_actor_extra", "unproved_class", &[]),
                entity("npc_citizen", "existing_npc", &[]),
                entity("prop_dynamic", "existing_prop", &[]),
            ],
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        assert_eq!(scene.scene_actor(&world, 0, "GmAn", usize::MAX), Some(2));
        assert_eq!(
            scene.scene_actor(&world, 0, "!TaRgEt1", usize::MAX),
            Some(2)
        );
        assert_eq!(
            scene.scene_actor(&world, 0, "unproved_class", usize::MAX),
            None
        );
        assert_eq!(
            scene.scene_actor(&world, 0, "existing_npc", usize::MAX),
            Some(5)
        );
        assert_eq!(
            scene.scene_actor(&world, 0, "existing_prop", usize::MAX),
            Some(6)
        );
        let mut data = Arc::unwrap_or_clone(choreography(&[
            (EventType::Speak, 0., "OriginalTest.ActorLine"),
            (EventType::FireTrigger, 0.1, "1"),
            (EventType::StopPoint, 5., "noaction"),
        ]));
        for name in ["GmAn", "!TaRgEt1"] {
            data.actors.push(source_assets::scenes::Actor {
                name: name.into(),
                active: true,
                channels: Vec::new(),
            });
        }
        data.events[0].actor = Some(0);
        data.events[1].actor = Some(1);
        scene.install_choreography(0, Arc::new(data));
        scene.send(0, "Start", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.tick(&world, Vec3::ZERO, 0.2);
        assert_eq!(scene.sounds, vec!["OriginalTest.ActorLine"]);
        assert_eq!(scene.states[3].value, 1.);
        assert_eq!(scene.diagnostics.scene_events_started, 2);
        assert!(!scene
            .diagnostics
            .unsupported
            .contains_key("logic_choreographed_scene.missing-actor"));
        assert!(scene
            .animation_states(&world)
            .iter()
            .any(|s| s.entity == 2 && s.targetname == "gman"));
        scene.states[2].killed = true;
        assert_eq!(scene.scene_actor(&world, 0, "!target1", usize::MAX), None);
        assert!(!scene.animation_states(&world).iter().any(|s| s.entity == 2));
    }
    #[test]
    #[ignore = "requires an installed owned HL2 copy; actor I/O, not intro camera/face/AI parity"]
    fn owned_gman_intro_resolves_cycler_actor_and_starts_authored_speech() {
        let game = source_assets::install::discover().unwrap();
        let data = std::fs::read(game.join("hl2/maps/d1_trainstation_01.bsp")).unwrap();
        let bsp = source_assets::bsp::Bsp::parse(&data).unwrap();
        let world = bsp.world("d1_trainstation_01").unwrap();
        let vfs = Vfs::mount(&game).unwrap();
        let mut scene = Scene::new(&world);
        scene.queue.clear();
        scene.load_choreography(&world, &vfs).unwrap();
        let id = world
            .entities
            .iter()
            .position(|e| e.get("targetname") == Some("scene2_lcs_intro"))
            .unwrap();
        let actor = world
            .entities
            .iter()
            .position(|e| e.get("targetname") == Some("gman"))
            .unwrap();
        assert_eq!(world.entities[actor].class(), "cycler_actor");
        assert_eq!(
            world.entities[actor].get("model"),
            Some("models/gman_high.mdl")
        );
        assert_eq!(
            scene.scene_actor(&world, id, "!target1", usize::MAX),
            Some(actor)
        );
        scene.send(id, "Start", "");
        for _ in 0..1201 {
            scene.tick(&world, Vec3::ZERO, 0.015);
        }
        assert_eq!(scene.diagnostics.scene_events_started, 17);
        assert!(!scene
            .diagnostics
            .unsupported
            .contains_key("logic_choreographed_scene.missing-actor"));
        assert!(scene.sounds.iter().any(|s| s == "Trainride.gman_riseshine"));
        assert!(scene.sounds.iter().any(|s| s == "Trainride.gman_02"));
    }
    #[test]
    fn scene_stop_point_completes_once_without_cutting_off_tail() {
        let world = World {
            entities: vec![
                entity(
                    "logic_choreographed_scene",
                    "scene",
                    &[
                        ("OnStart", "started,Add,1,0,-1"),
                        ("OnCompletion", "finished,Add,1,0,-1"),
                        ("OnTrigger1", "tail,Add,1,0,-1"),
                    ],
                ),
                entity("math_counter", "started", &[]),
                entity("math_counter", "finished", &[]),
                entity("math_counter", "tail", &[]),
            ],
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        scene.install_choreography(
            0,
            choreography(&[
                (EventType::StopPoint, 0.2, "noaction"),
                (EventType::FireTrigger, 0.5, "1"),
            ]),
        );
        scene.send(0, "Start", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.send(0, "Start", "");
        scene.tick(&world, Vec3::ZERO, 0.25);
        assert_eq!(scene.states[1].value, 1.);
        assert_eq!(scene.states[2].value, 1.);
        assert_eq!(scene.states[3].value, 0.);
        assert!(scene.choreography[&0].playback.is_some());
        scene.tick(&world, Vec3::ZERO, 0.5);
        assert_eq!(scene.states[2].value, 1.);
        assert_eq!(scene.states[3].value, 1.);
        assert_eq!(scene.diagnostics.scene_completions, 1);
        assert!(scene.choreography[&0].playback.is_none());
    }
    #[test]
    fn scene_input_pause_resume_and_cancel_do_not_invent_completion() {
        let world = World {
            entities: vec![
                entity(
                    "logic_choreographed_scene",
                    "scene",
                    &[
                        ("OnCompletion", "count,Add,100,0,-1"),
                        ("OnCanceled", "count,Add,10,0,-1"),
                        ("OnTrigger1", "count,Add,1,0,-1"),
                    ],
                ),
                entity("math_counter", "count", &[]),
            ],
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        scene.install_choreography(
            0,
            choreography(&[
                (EventType::FireTrigger, 1., "1"),
                (EventType::StopPoint, 2., "noaction"),
            ]),
        );
        scene.send(0, "Start", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.send(0, "Pause", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        let before = scene.choreography[&0].playback.as_ref().unwrap().elapsed;
        scene.tick(&world, Vec3::ZERO, 4.);
        assert_eq!(
            scene.choreography[&0].playback.as_ref().unwrap().elapsed,
            before
        );
        assert_eq!(scene.states[1].value, 0.);
        scene.send(0, "Resume", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.tick(&world, Vec3::ZERO, 1.1);
        assert_eq!(scene.states[1].value, 1.);
        scene.send(0, "Cancel", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.tick(&world, Vec3::ZERO, 4.);
        assert_eq!(scene.states[1].value, 11.);
        assert_eq!(scene.diagnostics.scene_completions, 0);
    }
    #[test]
    fn scene_actor_conditions_hold_section_until_explicit_resume() {
        let world = World {
            entities: vec![
                entity(
                    "logic_choreographed_scene",
                    "scene",
                    &[
                        ("target1", "actor"),
                        ("OnTrigger1", "door,Unlock,,0,-1"),
                        ("OnTrigger1", "door,Open,,0.1,-1"),
                    ],
                ),
                entity("npc_citizen", "actor", &[]),
                entity("prop_door_rotating", "door", &[("locked", "1")]),
            ],
            ..Default::default()
        };
        let mut data = Arc::unwrap_or_clone(choreography(&[
            (EventType::MoveTo, 0., "mark"),
            (EventType::Section, 0.1, "noaction"),
            (EventType::FireTrigger, 0.2, "1"),
        ]));
        data.events[0].actor = Some(0);
        data.events[0].flags |= 1;
        data.actors.push(source_assets::scenes::Actor {
            name: "!target1".into(),
            active: true,
            channels: Vec::new(),
        });
        let mut scene = Scene::new(&world);
        scene.install_choreography(0, Arc::new(data));
        scene.send(0, "Start", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.tick(&world, Vec3::ZERO, 2.);
        assert!(scene.states[2].locked);
        assert_eq!(scene.states[2].target, 0.);
        scene.tick(&world, Vec3::ZERO, 2.);
        assert!(scene.states[2].locked);
        assert!(scene
            .diagnostics
            .unsupported
            .contains_key("logic_choreographed_scene.SECTION:actor-resume-condition"));
        scene.send(0, "Resume", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.tick(&world, Vec3::ZERO, 0.11);
        assert!(!scene.states[2].locked);
        assert_eq!(scene.states[2].target, 0.);
        scene.tick(&world, Vec3::ZERO, 0.11);
        assert_eq!(scene.states[2].target, 1.);
    }
    #[test]
    fn movement_arrival_resolves_section_and_cancel_restores_body_animation() {
        let world = World {
            entities: vec![
                entity(
                    "logic_choreographed_scene",
                    "scene",
                    &[("target1", "actor"), ("OnTrigger1", "count,Add,1,0,-1")],
                ),
                entity("npc_barney", "actor", &[("DefaultAnim", "idle")]),
                entity("info_target", "mark", &[("origin", "100 0 0")]),
                entity("math_counter", "count", &[]),
            ],
            ..Default::default()
        };
        let mut data = Arc::unwrap_or_clone(choreography(&[
            (EventType::MoveTo, 0., "mark"),
            (EventType::Section, 0.1, "noaction"),
            (EventType::FireTrigger, 0.2, "1"),
        ]));
        data.events[0].actor = Some(0);
        data.events[0].parameters[1] = "run".into();
        data.events[0].flags |= 1;
        data.actors.push(source_assets::scenes::Actor {
            name: "!target1".into(),
            active: true,
            channels: Vec::new(),
        });
        let mut scene = Scene::new(&world);
        scene.queue.clear();
        scene.install_choreography(0, Arc::new(data));
        scene.send(0, "Start", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.tick(&world, Vec3::ZERO, 0.2);
        let key = crate::npc::GoalKey {
            scene: 0,
            event: 0,
            actor: 1,
        };
        assert!(scene.movement_commands.iter().any(|command| matches!(command, SceneMoveCommand::Start {key: k, request} if *k == key && request.target_entity == 2 && request.target_feet == Vec3::new(100.,0.,0.))));
        let frozen = scene.choreography[&0].playback.as_ref().unwrap().elapsed;
        scene.apply_movement(
            key,
            Vec3::new(20., 0., 0.),
            0.,
            Some(("run_all", 0.3)),
            false,
        );
        scene.tick(&world, Vec3::ZERO, 1.);
        assert_eq!(
            scene.choreography[&0].playback.as_ref().unwrap().elapsed,
            frozen
        );
        assert_eq!(scene.states[3].value, 0.);
        assert_eq!(scene.animation_time(1), 0.3);
        scene.apply_movement(
            key,
            Vec3::new(40., 0., 0.),
            0.,
            Some(("run_all", 0.4)),
            false,
        );
        assert_eq!(scene.animation_time(1), 0.4);
        scene.states[1].scripted_by = Some(99);
        scene.apply_movement(key, Vec3::new(100., 0., 0.), 0., None, true);
        assert!(!scene.movement_ready[&key]);
        assert_eq!(scene.states[1].origin.x, 40.);
        scene.states[1].scripted_by = None;
        scene.states[1].animation = "idle".into();
        scene.apply_movement(key, Vec3::new(100., 0., 0.), 0., None, true);
        scene.tick(&world, Vec3::ZERO, 0.2);
        assert_eq!(scene.states[3].value, 1.);
        scene.tick(&world, Vec3::ZERO, 1.);
        scene.send(0, "Start", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.apply_movement(
            key,
            Vec3::new(20., 0., 0.),
            0.,
            Some(("run_all", 0.1)),
            false,
        );
        scene.send(0, "Cancel", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        assert_eq!(scene.states[1].animation, "idle");
        assert!(!scene.movement_ready.contains_key(&key));
        assert!(scene
            .movement_commands
            .iter()
            .any(|c| matches!(c, SceneMoveCommand::CancelScene(0))));
        scene.apply_movement(key, Vec3::new(100., 0., 0.), 0., None, true);
        assert_eq!(scene.states[1].origin.x, 20.);
    }
    #[test]
    fn scene_sequence_uses_installed_clip_scene_clock_and_restores_baseline() {
        use modkit_core::{
            animation::{Clip, Rig},
            ModelInstance,
        };
        let world = World {
            entities: vec![
                entity(
                    "logic_choreographed_scene",
                    "scene",
                    &[("target1", "actor")],
                ),
                entity("npc_citizen", "actor", &[("DefaultAnim", "idle")]),
            ],
            model_instances: vec![ModelInstance {
                background: false,
                model: "actor.mdl".into(),
                origin: Vec3::ZERO,
                angles: Vec3::ZERO,
                skin: 2,
                scale: 1.,
                kind: "npc_citizen".into(),
                solid: false,
                solid_mode: None,
                entity: Some(1),
            }],
            rigs: BTreeMap::from([(
                "actor.mdl#2".into(),
                Rig {
                    clips: BTreeMap::from([(
                        "performance".into(),
                        Clip {
                            layer: Default::default(),
                            fps: 30.,
                            looping: false,
                            frames: vec![vec![]; 31],
                            events: Vec::new(),
                        },
                    )]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let mut data = Arc::unwrap_or_clone(choreography(&[
            (EventType::Sequence, 0., "Performance"),
            (EventType::Gesture, 0.25, "wave"),
            (EventType::StopPoint, 3., "noaction"),
        ]));
        data.actors.push(source_assets::scenes::Actor {
            name: "!target1".into(),
            active: true,
            channels: Vec::new(),
        });
        for event in &mut data.events[..2] {
            event.actor = Some(0);
            event.end = Some(2.);
        }
        let mut scene = Scene::new(&world);
        scene.install_choreography(0, Arc::new(data));
        assert_eq!(
            scene.required_animation_clips(&world),
            BTreeMap::from([(
                "actor.mdl#2".into(),
                BTreeSet::from(["performance".into(), "wave".into()])
            ),])
        );
        scene.send(0, "Start", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.tick(&world, Vec3::ZERO, 0.4);
        // A gesture without layer support must not replace the actual body sequence.
        assert_eq!(scene.states[1].animation, "performance");
        assert!((scene.animation_time(1) - 0.4).abs() < 1e-6);
        scene.send(0, "Pause", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        let frozen = scene.animation_time(1);
        scene.tick(&world, Vec3::ZERO, 5.);
        assert_eq!(scene.states[1].animation, "performance");
        assert_eq!(scene.animation_time(1), frozen);
        let clocks = scene.choreography_states(&world);
        assert_eq!(clocks[0].pause_reason, Some("input"));
        assert!(clocks[0].active && clocks[0].pause);
        let samples = scene.animation_states(&world);
        assert_eq!(samples[0].scene_owner, Some(0));
        assert_eq!(samples[0].sample_time, frozen);
        assert!(samples[0].clip_loaded);
        scene.send(0, "Resume", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.tick(&world, Vec3::ZERO, 0.5);
        assert!((scene.animation_time(1) - frozen - 0.5).abs() < 1e-6);
        scene.tick(&world, Vec3::ZERO, 1.1);
        assert_eq!(scene.states[1].animation, "idle");
        assert!(scene.choreography[&0].playback.is_some());
        // Cancel restores the baseline too, rather than leaving the NPC in a scene pose.
        scene.send(0, "Cancel", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.send(0, "Start", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.tick(&world, Vec3::ZERO, 0.25);
        assert_eq!(scene.states[1].animation, "performance");
        scene.send(0, "Cancel", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        assert_eq!(scene.states[1].animation, "idle");
        let clocks = scene.choreography_states(&world);
        assert!(!clocks[0].active);
        assert!(clocks[0].elapsed.is_none());
        assert!(scene.animation_states(&world)[0].scene_owner.is_none());
        let mut missing =
            Arc::unwrap_or_clone(choreography(&[(EventType::Sequence, 0., "missing_clip")]));
        missing.actors.push(source_assets::scenes::Actor {
            name: "actor".into(),
            active: true,
            channels: Vec::new(),
        });
        missing.events[0].actor = Some(0);
        missing.events[0].end = Some(1.);
        scene.install_choreography(0, Arc::new(missing));
        scene.send(0, "Start", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.tick(&world, Vec3::ZERO, 0.1);
        assert_eq!(scene.states[1].animation, "idle");
        assert!(scene
            .diagnostics
            .unsupported
            .contains_key("npc_citizen.animation:missing_clip"));
    }
    #[test]
    fn named_input_uses_deferred_glob_delivery_and_rejects_invalid_delay() {
        let world = World {
            entities: vec![
                entity("math_counter", "count_a", &[]),
                entity("math_counter", "count_b", &[]),
            ],
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        assert!(!scene.send_named("count*", "Add", "10", f64::NAN));
        assert!(!scene.send_named("count*", "Add", "10", -1.));
        assert!(scene.send_named("count*", "Add", "2", 0.1));
        assert!(scene.send_named("COUNT_A", "Add", "3", 0.1));
        scene.tick(&world, Vec3::ZERO, 0.05);
        assert_eq!(scene.states[0].value, 0.);
        scene.tick(&world, Vec3::ZERO, 0.06);
        assert_eq!(scene.states[0].value, 5.);
        assert_eq!(scene.states[1].value, 2.);
    }
    #[test]
    #[ignore = "requires an installed owned HL2 copy; isolated scene I/O, not campaign parity"]
    fn owned_security03_blocks_movement_condition_and_delivers_real_gate_after_resume() {
        let game = source_assets::install::discover().unwrap();
        let data = std::fs::read(game.join("hl2/maps/d1_trainstation_01.bsp")).unwrap();
        let bsp = source_assets::bsp::Bsp::parse(&data).unwrap();
        let world = bsp.world("d1_trainstation_01").unwrap();
        let vfs = Vfs::mount(&game).unwrap();
        let mut scene = Scene::new(&world);
        // Isolate this one scene from the unrelated camera/train introductory chain.
        scene.queue.clear();
        assert_eq!(scene.load_choreography(&world, &vfs).unwrap(), 44);
        let id = world
            .entities
            .iter()
            .position(|e| e.get("targetname") == Some("security_03"))
            .unwrap();
        let door = world
            .entities
            .iter()
            .position(|e| e.get("targetname") == Some("storage_room_door"))
            .unwrap();
        assert!(scene.states[door].locked);
        // Barney is a point_template child; spawn him as the campaign does on arrival.
        assert!(scene.send_named("gordon_cop_template", "ForceSpawn", "", 0.));
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.queue.clear();
        assert!(scene.send_named("security_03", "Start", "", 0.));
        for _ in 0..600 {
            scene.tick(&world, Vec3::ZERO, 0.015);
        }
        let playback = scene.choreography[&id].playback.as_ref().unwrap();
        assert!((playback.elapsed - 6.88063383102417).abs() < 1e-6);
        assert!(scene.states[door].locked);
        scene.send_named("security_03", "Resume", "", 0.);
        for _ in 0..25 {
            scene.tick(&world, Vec3::ZERO, 0.015);
        }
        assert!(!scene.states[door].locked);
        assert_eq!(scene.states[door].target, 1.);
        let parsed = &scene.choreography[&id].data;
        assert!(parsed
            .events
            .iter()
            .any(|e| e.kind == EventType::FireTrigger
                && e.parameters[0] == "3"
                && (e.start - 7.013968).abs() < 1e-6));
        assert!(parsed
            .events
            .iter()
            .any(|e| e.kind == EventType::StopPoint && (e.start - 9.59397).abs() < 1e-6));
    }
    #[test]
    #[ignore = "requires owned HL2 scene cache"]
    fn owned_template_actors_get_scene_gesture_clips_before_force_spawn() {
        let game = source_assets::install::discover().unwrap();
        let data = std::fs::read(game.join("hl2/maps/d1_trainstation_01.bsp")).unwrap();
        let mut world = source_assets::bsp::Bsp::parse(&data)
            .unwrap()
            .world("d1_trainstation_01")
            .unwrap();
        let vfs = Vfs::mount(&game).unwrap();
        let entity = |world: &World, name: &str| {
            world
                .entities
                .iter()
                .position(|e| {
                    e.get("targetname")
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
                })
                .unwrap()
        };
        for (name, model) in [
            ("Barney", "models/barney.mdl"),
            ("kleiner", "models/kleiner.mdl"),
        ] {
            let id = entity(&world, name);
            world.model_instances.push(modkit_core::ModelInstance {
                background: false,
                model: model.into(),
                origin: world.entities[id].origin(),
                angles: Vec3::ZERO,
                skin: 0,
                scale: 1.,
                kind: world.entities[id].class().into(),
                solid: true,
                solid_mode: None,
                entity: Some(id),
            });
        }
        let mut scene = Scene::new(&world);
        scene.queue.clear();
        scene.load_choreography(&world, &vfs).unwrap();
        // Both are point_template children: pending (killed) until ForceSpawn.
        assert!(scene.template_pending(entity(&world, "Barney")));
        assert!(scene.template_pending(entity(&world, "kleiner")));
        let clips = scene.required_animation_clips(&world);
        let barney = &clips["models/barney.mdl#0"];
        assert!(
            barney.contains("gesture08") && barney.contains("posture01"),
            "{barney:?}"
        );
        let kleiner = &clips["models/kleiner.mdl#0"];
        assert!(kleiner.contains("kposture01"), "{kleiner:?}");
    }
    #[test]
    #[ignore = "requires owned HL2 scene cache; isolated LOOKAT execution, not native AI parity"]
    fn owned_security02_refreshes_overlapping_look_targets_and_pause_cancel_lifetime() {
        use crate::attention::Target;
        let game = source_assets::install::discover().unwrap();
        let data = std::fs::read(game.join("hl2/maps/d1_trainstation_01.bsp")).unwrap();
        let world = source_assets::bsp::Bsp::parse(&data)
            .unwrap()
            .world("d1_trainstation_01")
            .unwrap();
        let vfs = Vfs::mount(&game).unwrap();
        let mut scene = Scene::new(&world);
        scene.queue.clear();
        scene.load_choreography(&world, &vfs).unwrap();
        let entity = |name: &str| {
            world
                .entities
                .iter()
                .position(|e| {
                    e.get("targetname")
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
                })
                .unwrap()
        };
        let id = entity("security_02");
        let barney = entity("Barney");
        // Barney is a point_template child; the campaign force-spawns him on arrival.
        assert!(scene.template_pending(barney));
        scene.send(entity("gordon_cop_template"), "ForceSpawn", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        assert!(!scene.template_pending(barney) && !scene.states[barney].killed);
        let monitor = entity("mark_secmonitor_look_1");
        scene.send(id, "Start", "");
        for _ in 0..50 {
            scene.tick(&world, Vec3::ZERO, 0.015);
        }
        let first = scene.look_targets.report()[&barney].last().unwrap();
        assert_eq!(first.target, Target::Entity(monitor));
        assert_eq!(first.scene, id);
        let elapsed = scene.choreography[&id].playback.as_ref().unwrap().elapsed;
        let e = &scene.choreography[&id].data.events[first.event];
        assert!(
            (first.importance
                - crate::attention::scene_importance(
                    e.intensity(&scene.choreography[&id].data, elapsed as f32),
                    elapsed as f32 - e.start
                ))
            .abs()
                < 1e-6
        );
        scene.send(id, "Pause", "");
        // Entity input delivery follows choreography in the retained fixed tick.
        scene.tick(&world, Vec3::ZERO, 0.015);
        let paused_elapsed = scene.choreography[&id].playback.as_ref().unwrap().elapsed;
        assert!((paused_elapsed - elapsed - 0.015).abs() < 1e-6);
        for _ in 0..30 {
            scene.tick(&world, Vec3::ZERO, 0.015);
        }
        assert_eq!(
            scene.choreography[&id].playback.as_ref().unwrap().elapsed,
            paused_elapsed
        );
        assert!(scene.look_targets.report()[&barney].last().unwrap().end > scene.time);
        scene.send(id, "Resume", "");
        for _ in 0..365 {
            scene.tick(&world, Vec3::ZERO, 0.015);
        }
        let targets = &scene.look_targets.report()[&barney];
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].target, Target::Entity(monitor));
        assert_eq!(targets[1].target, Target::Player);
        // Explicit cancellation stops refreshing but retains the last 0.1 sec interest.
        scene.send(id, "Cancel", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        assert!(!scene.look_targets.report()[&barney].is_empty());
        for _ in 0..7 {
            scene.tick(&world, Vec3::ZERO, 0.015);
        }
        assert!(!scene.look_targets.report().contains_key(&barney));
        scene.send(id, "Start", "");
        for _ in 0..50 {
            scene.tick(&world, Vec3::ZERO, 0.015);
        }
        scene.states[monitor].killed = true;
        scene.tick(&world, Vec3::ZERO, 0.015);
        assert!(!scene.look_targets.report().contains_key(&barney));
    }
    #[test]
    #[ignore = "requires owned HL2 installation"]
    fn owned_security_gestures_layer_barney_and_fade_after_cancel() {
        let game = source_assets::install::discover().unwrap();
        let data = std::fs::read(game.join("hl2/maps/d1_trainstation_01.bsp")).unwrap();
        let world = source_assets::bsp::Bsp::parse(&data)
            .unwrap()
            .world("d1_trainstation_01")
            .unwrap();
        let vfs = Vfs::mount(&game).unwrap();
        let mut scene = Scene::new(&world);
        scene.queue.clear();
        scene.load_choreography(&world, &vfs).unwrap();
        let entity = |name: &str| {
            world
                .entities
                .iter()
                .position(|e| {
                    e.get("targetname")
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
                })
                .unwrap()
        };
        let id = entity("security_02");
        let barney = entity("Barney");
        // Barney is a point_template child; the campaign force-spawns him on arrival.
        assert!(scene.template_pending(barney));
        scene.send(entity("gordon_cop_template"), "ForceSpawn", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        assert!(!scene.template_pending(barney) && !scene.states[barney].killed);
        // The parsed test world has no model instances, so request this scene's names directly.
        let wanted = scene.choreography[&id]
            .data
            .events
            .iter()
            .filter(|e| matches!(e.kind, EventType::Gesture | EventType::Sequence))
            .map(|e| e.parameters[0].to_lowercase())
            .filter(|n| !n.is_empty())
            .collect::<BTreeSet<_>>();
        let rig = source_assets::animation::load(&vfs, "models/barney.mdl", &wanted).unwrap();
        assert!(rig.warnings.is_empty(), "{:?}", rig.warnings);
        scene.send(id, "Start", "");
        let (mut seen, mut composed_ticks, mut moved) = (BTreeSet::new(), 0, false);
        let mut flexed = BTreeSet::new();
        for _ in 0..2000 {
            scene.tick(&world, Vec3::ZERO, 0.015);
            if let Some(values) = scene.flex_controllers.get(&barney) {
                assert!(values.values().all(|v| v.is_finite()));
                flexed.extend(
                    values
                        .iter()
                        .filter(|(_, v)| v.abs() > 0.05)
                        .map(|(k, _)| k.clone()),
                );
            }
            let layers = scene.gestures.layers(barney);
            for layer in layers {
                assert!((0. ..=1.).contains(&layer.playback), "{layer:?}");
                assert!((0. ..=1.).contains(&layer.weight), "{layer:?}");
                seen.insert(layer.sequence.clone());
            }
            if layers.iter().any(|l| l.removal.is_none() && l.weight > 0.5) && composed_ticks < 40 {
                let base = scene.states[barney].animation.clone();
                let (matrices, errors) =
                    scene
                        .gestures
                        .compose(&rig, barney, &base, 1., &rig.default_pose_values());
                assert!(errors.is_empty(), "{errors:?}");
                assert!(matrices.iter().all(|m| m.is_finite()));
                moved |= matrices
                    .iter()
                    .zip(rig.matrices(&base, 1.))
                    .any(|(a, b)| !a.abs_diff_eq(b, 1e-3));
                composed_ticks += 1;
            }
        }
        assert!(
            !seen.is_empty(),
            "security_02 produced no Barney gesture layer"
        );
        // FlexAnimation tracks drive Barney's facial controllers during the scene.
        assert!(!flexed.is_empty(), "no flex controller moved");
        eprintln!("FLEXED {flexed:?}");
        assert!(
            composed_ticks > 0 && moved,
            "gesture layers did not move bones"
        );
        // Restart, let a layer start, then cancel: the 0.5 sec RemoveLayer fade drops it.
        scene.send(id, "Start", "");
        for _ in 0..2000 {
            scene.tick(&world, Vec3::ZERO, 0.015);
            if !scene.gestures.layers(barney).is_empty() {
                break;
            }
        }
        assert!(!scene.gestures.layers(barney).is_empty());
        scene.send(id, "Cancel", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        assert!(scene
            .gestures
            .layers(barney)
            .iter()
            .all(|l| l.removal.is_some()));
        for _ in 0..40 {
            scene.tick(&world, Vec3::ZERO, 0.015);
        }
        assert!(scene.gestures.layers(barney).is_empty());
    }
    #[test]
    fn point_template_children_wait_for_force_spawn_and_spawn_once() {
        let world = World {
            entities: vec![
                entity(
                    "point_template",
                    "maker",
                    &[
                        ("Template01", "actor"),
                        ("OnEntitySpawned", "relay,Trigger,,0,-1"),
                    ],
                ),
                entity("npc_citizen", "actor", &[]),
                entity("logic_relay", "relay", &[]),
                entity(
                    "point_template",
                    "keeper",
                    &[("Template01", "kept"), ("spawnflags", "1")],
                ),
                entity("npc_citizen", "kept", &[]),
            ],
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        assert!(scene.template_pending(1) && scene.states[1].killed && !scene.states[1].visible);
        // "Don't remove template entities" keeps the authored entity alive.
        assert!(!scene.template_pending(4) && !scene.states[4].killed);
        // Dormant children ignore inputs, like entities that do not exist yet.
        scene.send(1, "Kill", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        let fired = scene.diagnostics.outputs_fired;
        scene.send(0, "ForceSpawn", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        assert!(!scene.template_pending(1) && !scene.states[1].killed && scene.states[1].visible);
        assert!(
            scene.diagnostics.outputs_fired > fired,
            "OnEntitySpawned fires"
        );
        scene.send(0, "ForceSpawn", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        assert!(scene
            .diagnostics
            .unsupported
            .contains_key("point_template.ForceSpawn:repeat"));
    }
    #[test]
    fn trigger_hurt_follows_sdk_think_timing() {
        let cube = modkit_core::BrushModel {
            id: 1,
            brushes: vec![modkit_core::Brush {
                planes: [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z]
                    .into_iter()
                    .map(|normal| modkit_core::Plane {
                        normal,
                        distance: 8.,
                    })
                    .collect(),
                contents: 1,
            }],
            mins: Vec3::splat(-8.),
            maxs: Vec3::splat(8.),
            ..Default::default()
        };
        let hurt = |model: &str| World {
            entities: vec![entity(
                "trigger_hurt",
                "pain",
                &[
                    ("model", "*1"),
                    ("spawnflags", "1"),
                    ("damage", "10"),
                    ("damagecap", "30"),
                    ("damagemodel", model),
                    ("damagetype", "0"),
                ],
            )],
            brush_models: vec![cube.clone()],
            ..Default::default()
        };
        let world = hurt("0");
        let mut scene = Scene::new(&world);
        let mut dealt = Vec::new();
        // Inside for one second (67 ticks), then outside.
        for tick in 0..80 {
            let at = if tick < 67 {
                Vec3::ZERO
            } else {
                Vec3::X * 100.
            };
            scene.tick(&world, at, 0.015);
            for amount in scene.player_damage.drain(..).map(|d| d.amount) {
                dealt.push((tick, amount));
            }
        }
        // Immediate hit, then every 0.5 s (about 34 ticks): 5 per think, none on leaving
        // because the last think hurt the player.
        assert_eq!(dealt.len(), 2, "{dealt:?}");
        assert!(dealt.iter().all(|&(_, a)| a == 5.));
        assert!((33..=35).contains(&(dealt[1].0 - dealt[0].0)), "{dealt:?}");
        // Doubling: 5, 10, 15 (capped at 30 x 0.5).
        let world = hurt("1");
        let mut scene = Scene::new(&world);
        let mut amounts = Vec::new();
        for _ in 0..110 {
            scene.tick(&world, Vec3::ZERO, 0.015);
            amounts.extend(scene.player_damage.drain(..).map(|d| d.amount));
        }
        assert_eq!(amounts, [5., 10., 15., 15.]);
    }
    #[test]
    fn trigger_hurt_hurts_npc_touchers_and_respects_filters() {
        let cube = modkit_core::BrushModel {
            id: 1,
            brushes: vec![modkit_core::Brush {
                planes: [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z]
                    .into_iter()
                    .map(|normal| modkit_core::Plane {
                        normal,
                        distance: 8.,
                    })
                    .collect(),
                contents: 1,
            }],
            mins: Vec3::splat(-8.),
            maxs: Vec3::splat(8.),
            ..Default::default()
        };
        // NPCs only (spawnflags 2), fixed damage 20 from a trigger at (5 0 0).
        let world = World {
            entities: vec![
                entity(
                    "trigger_hurt",
                    "pain",
                    &[
                        ("model", "*1"),
                        ("spawnflags", "2"),
                        ("origin", "5 0 0"),
                        ("damage", "20"),
                        ("OnHurt", "relay,Trigger,,0,-1"),
                    ],
                ),
                entity("npc_metropolice", "cop", &[("origin", "0 0 0")]),
            ],
            brush_models: vec![cube],
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        let fired = scene.diagnostics.outputs_fired;
        scene.tick(&world, Vec3::ZERO, 0.015);
        assert!(scene.player_damage.is_empty());
        let hit = scene.entity_damage.pop().expect("NPC hurt");
        assert!(matches!(
            hit.target,
            crate::projectiles::DamageTarget::Entity(1)
        ));
        assert_eq!((hit.amount, hit.origin), (10., Vec3::new(5., 0., 0.)));
        assert!(scene.diagnostics.outputs_fired > fired);
        // A killed NPC stops the think: no hurt, no further damage.
        scene.states[1].killed = true;
        for _ in 0..40 {
            scene.tick(&world, Vec3::ZERO, 0.015);
        }
        assert!(scene.entity_damage.is_empty());
    }
    #[test]
    fn no_touch_changelevel_still_accepts_explicit_input() {
        let world = World {
            entities: vec![entity(
                "trigger_changelevel",
                "exit",
                &[
                    ("spawnflags", "2"),
                    ("model", "*1"),
                    ("map", "next"),
                    ("landmark", "arrival"),
                ],
            )],
            brush_models: vec![modkit_core::BrushModel {
                id: 1,
                brushes: vec![modkit_core::Brush {
                    planes: [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z]
                        .into_iter()
                        .map(|normal| modkit_core::Plane {
                            normal,
                            distance: 8.,
                        })
                        .collect(),
                    contents: 1,
                }],
                mins: Vec3::splat(-8.),
                maxs: Vec3::splat(8.),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        scene.tick(&world, Vec3::ZERO, 0.015);
        assert!(scene.transition.is_none());
        scene.send(0, "ChangeLevel", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        assert_eq!(scene.transition, Some(("next".into(), "arrival".into())));
        let mut touch_world = world.clone();
        touch_world.entities[0]
            .properties
            .retain(|(key, _)| key != "spawnflags");
        let mut touch_scene = Scene::new(&touch_world);
        touch_scene.tick(&touch_world, Vec3::ZERO, 0.015);
        assert_eq!(
            touch_scene.transition,
            Some(("next".into(), "arrival".into()))
        );
    }
    #[test]
    fn combine_cameras_deploy_retract_and_redeploy() {
        use modkit_core::{
            animation::{Clip, Rig, Sequence},
            ModelInstance,
        };
        let clip = |frames: usize, looping: bool| Clip {
            layer: Default::default(),
            events: Vec::new(),
            fps: 30.,
            looping,
            frames: vec![vec![]; frames],
        };
        let seq = |name: &str, activity: &str, order: u32| Sequence {
            name: name.into(),
            activity: activity.into(),
            weight: 1,
            order,
        };
        let w = World {
            entities: vec![
                entity("npc_combine_camera", "cam", &[]),
                entity("npc_combine_camera", "sleeper", &[("spawnflags", "128")]),
            ],
            model_instances: (0..2)
                .map(|id| ModelInstance {
                    background: false,
                    model: "camera.mdl".into(),
                    origin: Vec3::ZERO,
                    angles: Vec3::ZERO,
                    skin: 0,
                    scale: 1.,
                    kind: "npc_combine_camera".into(),
                    solid: false,
                    solid_mode: None,
                    entity: Some(id),
                })
                .collect(),
            rigs: BTreeMap::from([(
                "camera.mdl#0".into(),
                Rig {
                    clips: BTreeMap::from([
                        ("idle".into(), clip(11, true)),
                        ("idlealert".into(), clip(11, true)),
                        ("ai_retract".into(), clip(48, false)),
                    ]),
                    sequences: vec![
                        seq("idle", "ACT_COMBINE_CAMERA_CLOSED_IDLE", 0),
                        seq("idlealert", "ACT_COMBINE_CAMERA_OPEN_IDLE", 1),
                        seq("ai_retract", "ACT_COMBINE_CAMERA_CLOSE", 2),
                    ],
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let mut scene = Scene::new(&w);
        // Spawn: deployed unless StartInactive.
        assert_eq!(scene.states[0].animation, "idlealert");
        assert_eq!(scene.states[1].animation, "idle");
        assert!(!scene.states[1].enabled);
        // Toggle off plays the close transition, then rests in closed idle.
        scene.send(0, "Toggle", "");
        scene.tick(&w, Vec3::ZERO, 0.015);
        assert_eq!(scene.states[0].animation, "ai_retract");
        for _ in 0..120 {
            scene.tick(&w, Vec3::ZERO, 0.015);
        }
        assert_eq!(scene.states[0].animation, "idle");
        // Enable redeploys; enabling an enabled camera changes nothing.
        scene.send(0, "Enable", "");
        scene.send(1, "Enable", "");
        scene.tick(&w, Vec3::ZERO, 0.015);
        assert_eq!(
            (
                scene.states[0].animation.as_str(),
                scene.states[1].animation.as_str()
            ),
            ("idlealert", "idlealert")
        );
        assert!(
            scene.diagnostics.unsupported.is_empty(),
            "{:?}",
            scene.diagnostics.unsupported
        );
    }
    #[test]
    fn head_and_body_flexes_offset_actor_pose_parameters() {
        use modkit_core::animation::{PoseParameter, Rig};
        let w = World {
            entities: vec![entity("npc_barney", "barney", &[])],
            ..Default::default()
        };
        let param = |name: &str, start: f32, end: f32| PoseParameter {
            name: name.into(),
            start,
            end,
            looping: 0.,
        };
        let rig = Rig {
            pose_parameters: vec![
                param("head_pitch", -30., 30.),
                param("head_yaw", -60., 60.),
                param("body_yaw", -30., 30.),
                param("gesture_height", -1., 1.),
            ],
            ..Default::default()
        };
        let mut scene = Scene::new(&w);
        let rest = scene.actor_pose_values(&rig, 0);
        let rest_signature = scene.pose_signature(0);
        // UpdateHeadControl adds head flexes to the look correction; UpdateBodyControl
        // drives body_yaw from body_rightleft. Values are in controller degrees.
        scene.flex_controllers.insert(
            0,
            BTreeMap::from([
                ("head_updown".into(), 15.),
                ("body_rightleft".into(), -15.),
                ("smile".into(), 1.),
            ]),
        );
        let p = scene.actor_pose_values(&rig, 0);
        assert!(
            (p[0] - 0.75).abs() < 1e-6 && (p[2] - 0.25).abs() < 1e-6,
            "{p:?}"
        );
        assert_eq!((p[1], p[3]), (rest[1], rest[3]));
        assert_ne!(scene.pose_signature(0), rest_signature);
        scene.heads.insert(
            0,
            crate::attention::Head {
                goal: Vec3::new(10., 30., 0.),
                ..Default::default()
            },
        );
        let p = scene.actor_pose_values(&rig, 0);
        assert!((p[0] - (25. + 30.) / 60.).abs() < 1e-6 && (p[1] - 0.75).abs() < 1e-6);
    }
    #[test]
    fn props_parent_to_entities_and_follow_animated_attachments() {
        use modkit_core::{
            animation::{Attachment, Bone, Pose, Rig},
            ModelInstance,
        };
        let w = World {
            entities: vec![
                entity("npc_barney", "barney", &[("origin", "100 0 0")]),
                entity(
                    "prop_dynamic",
                    "helmet",
                    &[("origin", "100 0 50"), ("parentname", "barney")],
                ),
                entity(
                    "prop_dynamic",
                    "plate",
                    &[("origin", "0 0 0"), ("StartDisabled", "1")],
                ),
            ],
            model_instances: vec![ModelInstance {
                background: false,
                model: "barney.mdl".into(),
                origin: Vec3::new(100., 0., 0.),
                angles: Vec3::ZERO,
                skin: 0,
                scale: 1.,
                kind: "npc_barney".into(),
                solid: false,
                solid_mode: None,
                entity: Some(0),
            }],
            rigs: BTreeMap::from([(
                "barney.mdl#0".into(),
                Rig {
                    bones: vec![Bone {
                        name: "root".into(),
                        parent: None,
                        bind: Pose {
                            position: Vec3::ZERO,
                            rotation: Quat::IDENTITY,
                        },
                        inverse_bind: Mat4::IDENTITY,
                    }],
                    attachments: vec![Attachment {
                        name: "helmet_attachment".into(),
                        bone: 0,
                        local: Mat4::from_translation(Vec3::new(2., 0., 64.)),
                    }],
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let mut scene = Scene::new(&w);
        // Map-spawn parentname keeps the authored world offset; StartDisabled hides props.
        assert_eq!(scene.states[1].parent.as_ref().map(|p| p.entity), Some(0));
        assert!(!scene.states[2].visible);
        scene.states[0].origin = Vec3::new(110., 0., 0.);
        scene.tick(&w, Vec3::ZERO, 0.015);
        assert!(scene.states[1]
            .origin
            .abs_diff_eq(Vec3::new(110., 0., 50.), 1e-4));
        // SetParent then SetParentAttachment snaps onto the attachment (logic_barney_init).
        scene.send(2, "SetParent", "barney");
        scene.send(2, "SetParentAttachment", "helmet_attachment");
        scene.tick(&w, Vec3::ZERO, 0.015);
        let parent = scene.states[2].parent.clone().unwrap();
        assert_eq!(parent.attachment.as_deref(), Some("helmet_attachment"));
        assert!(scene.states[2]
            .origin
            .abs_diff_eq(Vec3::new(112., 0., 64.), 1e-4));
        // The child follows the parent's rotation and position.
        scene.states[0].rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        scene.tick(&w, Vec3::ZERO, 0.015);
        assert!(scene.states[2]
            .origin
            .abs_diff_eq(Vec3::new(110., 2., 64.), 1e-4));
        assert!(scene.states[2]
            .rotation
            .abs_diff_eq(scene.states[0].rotation, 1e-5));
        // TurnOn/TurnOff toggle drawing; ClearParent keeps the last world pose.
        scene.send(2, "TurnOn", "");
        scene.send(2, "ClearParent", "");
        scene.tick(&w, Vec3::ZERO, 0.015);
        assert!(scene.states[2].visible && scene.states[2].parent.is_none());
        scene.states[0].origin = Vec3::ZERO;
        scene.tick(&w, Vec3::ZERO, 0.015);
        assert!(scene.states[2]
            .origin
            .abs_diff_eq(Vec3::new(110., 2., 64.), 1e-4));
        // An unknown attachment is reported and leaves the parent unchanged.
        scene.send(1, "SetParentAttachment", "missing");
        scene.tick(&w, Vec3::ZERO, 0.015);
        assert_eq!(scene.states[1].parent.as_ref().unwrap().attachment, None);
        assert!(scene
            .diagnostics
            .unsupported
            .keys()
            .any(|k| k.contains("missing-attachment")));
    }
    #[test]
    fn setting_default_animation_preserves_playback_and_completion() {
        use modkit_core::{
            animation::{Clip, Rig},
            ModelInstance,
        };
        let w = World {
            entities: vec![
                entity(
                    "prop_dynamic",
                    "prop",
                    &[
                        ("DefaultAnim", "closed"),
                        ("OnAnimationDone", "result,Add,1,0,-1"),
                    ],
                ),
                entity("math_counter", "result", &[]),
            ],
            model_instances: vec![ModelInstance {
                background: false,
                model: "prop.mdl".into(),
                origin: Vec3::ZERO,
                angles: Vec3::ZERO,
                skin: 0,
                scale: 1.,
                kind: "prop_dynamic".into(),
                solid: false,
                solid_mode: None,
                entity: Some(0),
            }],
            rigs: BTreeMap::from([(
                "prop.mdl#0".into(),
                Rig {
                    clips: BTreeMap::from([(
                        "opening".into(),
                        Clip {
                            layer: Default::default(),
                            events: Vec::new(),
                            fps: 30.,
                            looping: false,
                            frames: vec![vec![]; 31],
                        },
                    )]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let mut scene = Scene::new(&w);
        scene.send(0, "SetAnimation", "opening");
        scene.tick(&w, Vec3::ZERO, 0.015);
        let started = scene.states[0].animation_started;
        let due = scene.states[0].animation_done;
        scene.tick(&w, Vec3::ZERO, 0.2);
        // Native accepts a deferred name even when no such sequence is loaded.
        scene.send(0, "SetDefaultAnimation", "DeferredCase");
        scene.tick(&w, Vec3::ZERO, 0.015);
        assert_eq!(scene.states[0].default_animation, "DeferredCase");
        assert_eq!(scene.states[0].animation, "opening");
        assert_eq!(scene.states[0].animation_started, started);
        assert_eq!(scene.states[0].animation_done, due);
        assert!(scene.diagnostics.unsupported.is_empty());
        scene.send(0, "SetDefaultAnimation", "");
        scene.tick(&w, Vec3::ZERO, 0.015);
        assert!(scene.states[0].default_animation.is_empty());
        assert_eq!(scene.states[0].animation_done, due);
        scene.tick(&w, Vec3::ZERO, 0.9);
        assert_eq!(scene.states[1].value, 1.);
        scene.tick(&w, Vec3::ZERO, 0.9);
        assert_eq!(scene.states[1].value, 1.);
    }
    #[test]
    fn shuffle_exhausts_cases_and_avoids_repeating_across_batches() {
        let w = World {
            entities: vec![
                entity(
                    "logic_case",
                    "choose",
                    &[
                        ("OnCase01", "result,SetValue,1,0,-1"),
                        ("OnCase02", "result,SetValue,2,0,-1"),
                        ("OnCase03", "result,SetValue,3,0,-1"),
                    ],
                ),
                entity("math_counter", "result", &[]),
            ],
            ..Default::default()
        };
        let mut s = Scene::new(&w);
        let mut chosen = Vec::new();
        for _ in 0..30 {
            s.send(0, "PickRandomShuffle", "");
            s.tick(&w, Vec3::ZERO, 0.015);
            chosen.push(s.states[1].value as i32);
        }
        for batch in chosen.as_chunks::<3>().0 {
            assert_eq!(
                batch.iter().copied().collect::<BTreeSet<_>>(),
                BTreeSet::from([1, 2, 3])
            );
        }
        for boundary in (3..chosen.len()).step_by(3) {
            assert_ne!(chosen[boundary - 1], chosen[boundary]);
        }
    }
    #[test]
    fn scripted_post_idle_output_waits_for_its_clip() {
        use modkit_core::{
            animation::{Clip, Rig},
            ModelInstance,
        };
        let mut w = World {
            entities: vec![
                entity(
                    "scripted_sequence",
                    "script",
                    &[
                        ("m_iszEntity", "actor"),
                        ("m_iszPlay", "play"),
                        ("m_iszPostIdle", "post"),
                        ("OnEndSequence", "result,SetValue,1,0,-1"),
                        ("OnPostIdleEndSequence", "result,SetValue,2,0,-1"),
                    ],
                ),
                entity("npc_citizen", "actor", &[]),
                entity("math_counter", "result", &[]),
            ],
            model_instances: vec![ModelInstance {
                background: false,
                model: "actor.mdl".into(),
                origin: Vec3::ZERO,
                angles: Vec3::ZERO,
                skin: 0,
                scale: 1.,
                kind: "npc_citizen".into(),
                solid: true,
                solid_mode: None,
                entity: Some(1),
            }],
            rigs: BTreeMap::from([(
                "actor.mdl#0".into(),
                Rig {
                    clips: BTreeMap::from([
                        (
                            "play".into(),
                            Clip {
                                layer: Default::default(),
                                events: Vec::new(),
                                fps: 30.,
                                looping: false,
                                frames: vec![vec![]; 31],
                            },
                        ),
                        (
                            "post".into(),
                            Clip {
                                layer: Default::default(),
                                events: Vec::new(),
                                fps: 30.,
                                looping: false,
                                frames: vec![vec![]; 61],
                            },
                        ),
                    ]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let mut s = Scene::new(&w);
        s.send(0, "BeginSequence", "");
        s.tick(&w, Vec3::ZERO, 0.015);
        s.tick(&w, Vec3::ZERO, 1.1);
        assert_eq!(s.states[2].value, 1.);
        s.tick(&w, Vec3::ZERO, 1.5);
        assert_eq!(s.states[2].value, 1.);
        s.tick(&w, Vec3::ZERO, 0.6);
        assert_eq!(s.states[2].value, 2.);
        // Both an omitted action and an explicitly empty one must enter post idle immediately.
        for omitted in [false, true] {
            w.entities[0].properties.retain(|(k, _)| k != "m_iszPlay");
            if !omitted {
                w.entities[0]
                    .properties
                    .push(("m_iszPlay".into(), String::new()));
            }
            let mut s = Scene::new(&w);
            s.send(0, "BeginSequence", "");
            s.tick(&w, Vec3::ZERO, 0.015);
            assert_eq!(s.states[1].animation, "post");
            assert_eq!(s.states[2].value, 1.);
            assert!(!s
                .diagnostics
                .unsupported
                .keys()
                .any(|k| k.contains("animation:")));
            s.tick(&w, Vec3::ZERO, 1.5);
            assert_eq!(s.states[2].value, 1.);
            s.tick(&w, Vec3::ZERO, 0.6);
            assert_eq!(s.states[2].value, 2.);
        }
    }
    #[test]
    fn implicit_idle_and_scripted_activity_use_real_clip_and_deadlines() {
        use modkit_core::{
            animation::{Clip, Rig, Sequence},
            ModelInstance,
        };
        let mut world = World {
            entities: vec![
                entity(
                    "scripted_sequence",
                    "script",
                    &[
                        ("m_iszEntity", "actor"),
                        ("m_iszPlay", "ACT_IDLE"),
                        ("m_iszPostIdle", "ACT_IDLE"),
                    ],
                ),
                entity("npc_kleiner", "actor", &[]),
            ],
            model_instances: vec![ModelInstance {
                model: "actor.mdl".into(),
                origin: Vec3::ZERO,
                angles: Vec3::ZERO,
                skin: 0,
                scale: 1.,
                kind: "npc_kleiner".into(),
                background: false,
                solid: false,
                solid_mode: None,
                entity: Some(1),
            }],
            rigs: BTreeMap::from([(
                "actor.mdl#0".into(),
                Rig {
                    clips: BTreeMap::from([(
                        "model_idle".into(),
                        Clip {
                            layer: Default::default(),
                            events: vec![],
                            fps: 2.,
                            looping: true,
                            frames: vec![vec![]; 5],
                        },
                    )]),
                    sequences: vec![Sequence {
                        name: "model_idle".into(),
                        activity: "ACT_IDLE".into(),
                        weight: 1,
                        order: 0,
                    }],
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        assert_eq!(scene.states[1].animation, "model_idle");
        scene.send(0, "BeginSequence", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        assert_eq!(scene.states[1].animation, "model_idle");
        let finish = scene.states[0].script_finish.unwrap();
        assert!((finish - scene.time - 2.).abs() < 0.0001);
        scene.tick(&world, Vec3::ZERO, 2.1);
        assert_eq!(scene.states[1].animation, "model_idle");
        assert!(scene.states[1].scripted_by.is_none());
        let post = scene.states[0].post_idle_done.unwrap().0;
        assert!((post - scene.time - 2.).abs() < 0.0001);
        world.entities[1]
            .properties
            .push(("DefaultAnim".into(), "authored_missing".into()));
        let mut scene = Scene::new(&world);
        assert_eq!(scene.states[1].animation, "authored_missing");
        // Explicit missing label does not silently fall back to an activity.
        assert!(!scene.animate(&world, 1, "authored_missing", false));
        assert_eq!(scene.states[1].animation, "authored_missing");
    }
    #[test]
    fn duplicate_outputs_delays_and_fire_limits() {
        let w = World {
            entities: vec![
                entity(
                    "logic_relay",
                    "start",
                    &[
                        ("OnTrigger", "count,Add,2,0.1,1"),
                        ("OnTrigger", "count,Add,3,0.1,-1"),
                    ],
                ),
                entity("math_counter", "count", &[]),
            ],
            ..Default::default()
        };
        let mut s = Scene::new(&w);
        s.send(0, "Trigger", "");
        s.tick(&w, Vec3::ZERO, 0.015);
        assert_eq!(s.states[1].value, 0.);
        s.tick(&w, Vec3::ZERO, 0.1);
        assert_eq!(s.states[1].value, 5.);
        s.send(0, "Trigger", "");
        s.tick(&w, Vec3::ZERO, 0.015);
        s.tick(&w, Vec3::ZERO, 0.1);
        assert_eq!(s.states[1].value, 8.);
    }
    #[test]
    fn rotating_door_lock_and_completion() {
        let w = World {
            entities: vec![entity(
                "prop_door_rotating",
                "door",
                &[("locked", "1"), ("speed", "90")],
            )],
            ..Default::default()
        };
        let mut s = Scene::new(&w);
        s.send(0, "Open", "");
        s.tick(&w, Vec3::ZERO, 0.015);
        assert_eq!(s.states[0].target, 0.);
        s.send(0, "Unlock", "");
        s.send(0, "Open", "");
        for _ in 0..70 {
            s.tick(&w, Vec3::ZERO, 0.015);
        }
        assert!((s.states[0].rotation * Vec3::X + Vec3::Y).length() < 0.001);
        assert_eq!(s.diagnostics.door_completions, 1);
    }
    #[test]
    fn escaped_output_parameters_can_contain_commas() {
        let o = output("OnTrigger", "x\x1bSetValue\x1ba,b\x1b0\x1b-1").unwrap();
        assert_eq!(o.parameter, "a,b");
    }
    #[test]
    fn paired_prop_doors_use_opener_side_and_return_to_authored_closed_pose() {
        let w = World {
            entities: vec![
                entity(
                    "prop_door_rotating",
                    "pair",
                    &[
                        ("origin", "0 -47 0"),
                        ("angles", "0 0 0"),
                        ("spawnflags", "8192"),
                        ("speed", "90"),
                    ],
                ),
                entity(
                    "prop_door_rotating",
                    "pair",
                    &[
                        ("origin", "0 47 0"),
                        ("angles", "0 180 0"),
                        ("spawnflags", "8192"),
                        ("speed", "90"),
                    ],
                ),
            ],
            ..Default::default()
        };
        let mut s = Scene::new(&w);
        for side in [-1., 1.] {
            s.use_entity_at(&w, 0, Vec3::X * side * 64.);
            for _ in 0..70 {
                s.tick(&w, Vec3::ZERO, 0.015);
            }
            for state in &s.states {
                let panel = state.origin + state.rotation * Vec3::Y * 47.;
                assert!(
                    panel.x * side < -46.,
                    "both panels must swing away from opener"
                );
            }
            s.use_entity_at(&w, 1, Vec3::X * side * 64.);
            for _ in 0..70 {
                s.tick(&w, Vec3::ZERO, 0.015);
            }
            for state in &s.states {
                assert_eq!(state.fraction, 0.);
                assert!(state.rotation.dot(state.base_rotation).abs() > 0.99999);
            }
        }
        assert_eq!(s.diagnostics.door_completions, 8);
    }
    #[test]
    fn explicit_prop_door_direction_and_lock_override_opener_side() {
        for (direction, sign) in [(1, -1.), (2, 1.)] {
            let w = World {
                entities: vec![entity(
                    "prop_door_rotating",
                    "door",
                    &[
                        ("opendir", if direction == 1 { "1" } else { "2" }),
                        ("locked", "1"),
                    ],
                )],
                ..Default::default()
            };
            let mut s = Scene::new(&w);
            s.use_entity_at(&w, 0, Vec3::X * 64.);
            s.tick(&w, Vec3::ZERO, 0.015);
            assert_eq!(s.states[0].target, 0.);
            s.send(0, "Unlock", "");
            s.tick(&w, Vec3::ZERO, 0.015);
            s.use_entity_at(&w, 0, Vec3::X * 64.);
            s.tick(&w, Vec3::ZERO, 0.015);
            assert_eq!(s.states[0].target, 1.);
            assert_eq!(s.states[0].angle.signum(), sign);
        }
    }
    #[test]
    fn open_away_input_resolves_current_named_player_and_caller_origins() {
        let w = World {
            entities: vec![
                entity("prop_door_rotating", "door", &[("wait", "-1")]),
                entity("info_target", "actor", &[("origin", "-64 0 0")]),
                entity(
                    "logic_relay",
                    "relay",
                    &[
                        ("origin", "64 0 0"),
                        ("OnTrigger", "door,OpenAwayFrom,!caller,0,-1"),
                    ],
                ),
            ],
            ..Default::default()
        };
        for (name, expected) in [
            ("actor", 1.),
            ("!player", 1.),
            ("!activator", 1.),
            ("missing", -1.),
        ] {
            let mut s = Scene::new(&w);
            // The input uses the current state, not the map's initial actor origin.
            s.states[1].origin = Vec3::X * 64.;
            s.send(0, "OpenAwayFrom", name);
            s.tick(&w, Vec3::X * 64., 0.015);
            assert_eq!(s.states[0].target, 1.);
            assert_eq!(s.states[0].angle.signum(), expected, "{name}");
            assert_eq!(
                s.diagnostics
                    .unsupported
                    .get("prop_door_rotating.openawayfrom"),
                None
            );
        }
        let mut s = Scene::new(&w);
        s.send(0, "OpenAwayFrom", "!player");
        s.send(0, "OpenAwayFrom", "missing");
        s.tick(&w, Vec3::X * 64., 0.015);
        assert_eq!(
            s.states[0].angle.signum(),
            1.,
            "second open input must not retarget the opening door"
        );
        let mut s = Scene::new(&w);
        s.send(2, "Trigger", "");
        s.tick(&w, -Vec3::X * 64., 0.015);
        assert_eq!(s.states[0].angle.signum(), 1., "caller relay, not player");
    }
    #[test]
    fn open_away_input_obeys_lock_fixed_direction_and_keeps_opening_swing() {
        let w = World {
            entities: vec![entity(
                "prop_door_rotating",
                "door",
                &[("locked", "1"), ("opendir", "1"), ("wait", "-1")],
            )],
            ..Default::default()
        };
        let mut s = Scene::new(&w);
        s.send(0, "OpenAwayFrom", "!player");
        s.tick(&w, Vec3::X * 64., 0.015);
        assert_eq!(s.states[0].target, 0.);
        s.send(0, "Unlock", "");
        s.send(0, "OpenAwayFrom", "!player");
        s.tick(&w, Vec3::X * 64., 0.015);
        assert_eq!(s.states[0].target, 1.);
        assert_eq!(s.states[0].angle.signum(), -1.);
        s.send(0, "OpenAwayFrom", "!player");
        s.tick(&w, -Vec3::X * 64., 0.015);
        assert_eq!(s.states[0].angle.signum(), -1.);
    }
}
