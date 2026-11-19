//! The last steps of a scene's construction at map start.
use super::{number, output, parse_vec3, Diagnostics, Scene, State};
use crate::physics;
use crate::player_damage::{ScreenFade, FFADE_IN, FFADE_OUT, FFADE_PURGE, FFADE_STAYOUT};
use glam::{Quat, Vec3};
use modkit_core::{Entity, World};

/// CLogicAuto::Activate's SetNextThink(curtime + 0.2) (Source SDK game/server/logicauto.cpp).
const LOGIC_AUTO_THINK: f64 = 0.2;

/// How a door or other mover travels, from its keyvalues.
struct Motion {
    direction: Vec3,
    travel: f32,
    rotating: bool,
    angle: f32,
    axis: Vec3,
    fraction: f32,
    speed: f32,
}
impl Motion {
    fn of(world: &World, e: &Entity, flags: u32) -> Self {
        let model = e
            .get("model")
            .and_then(|m| m.strip_prefix('*'))
            .and_then(|m| m.parse::<usize>().ok())
            .and_then(|id| world.brush_models.iter().find(|m| m.id == id));
        // Mod_LoadSubmodels (retail engine.dll 0x1013f8c0) spreads a brush model's compiled
        // bounds by 1.0 on every side and CBaseDoor::Spawn / CFuncMoveLinear::Spawn subtract
        // 2 from that OBB size (SDK doors.cpp, func_movelinear.cpp): the compiled size.
        let bounds = model.map(|m| m.maxs - m.mins).unwrap_or(Vec3::splat(70.));
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
        let sized = bounds.max(Vec3::ZERO).dot(direction.abs()) - number(e, "lip", 0.);
        // CFuncMoveLinear::Spawn sizes a MoveDistance of 0 or less from the brush.
        let travel = Some(number(e, "movedistance", 0.))
            .filter(|d| *d > 0.)
            .unwrap_or(sized)
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
        Self {
            direction,
            travel,
            rotating,
            angle,
            axis,
            fraction,
            speed,
        }
    }
}
/// The entity's outputs; malformed ones are counted.
fn entity_outputs(e: &Entity, diagnostics: &mut Diagnostics) -> Vec<super::Output> {
    let mut outputs = Vec::new();
    for (k, v) in &e.properties {
        if k.starts_with("On") || k == "OutValue" {
            match output(k, v) {
                Some(o) => outputs.push(o),
                None => diagnostics.malformed_outputs += 1,
            }
        }
    }
    outputs
}

impl Scene {
    /// The weapon users' first idle.
    pub(super) fn start_weapon_idles(&mut self, world: &World) {
        // Metrocops without a DefaultAnim start on their (weapon-translated) idle activity,
        // as do other weapon users given a weapon (a citizen's SMG1: ACT_IDLE_SMG1).
        for id in 0..world.entities.len() {
            let e = &world.entities[id];
            if e.get("DefaultAnim").is_some() || !crate::npc_weapons::uses_weapons(e.class()) {
                continue;
            }
            let weapon = e.get("additionalequipment").unwrap_or("");
            if e.class() == "npc_metropolice" || !matches!(weapon.trim(), "" | "0") {
                self.animate_activity_if_present(world, id, weapon_idle_activity(weapon));
            }
        }
    }
    /// The map's entities spawn: their states, idle activities, cameras, templates, makers,
    /// spotlights, parents and the logic that starts with the map.
    pub(super) fn spawn_entities(&mut self, world: &World, new_game: bool) {
        let diagnostics = &mut self.diagnostics;
        self.states = (world.entities.iter())
            .map(|e| Self::initial_state(world, e, diagnostics))
            .collect();
        self.initialize_idle_activities(world);
        self.spawn_combine_cameras(world);
        self.start_weapon_idles(world);
        self.hold_template_children(world);
        // MapEntity_ParseAllEntities removes SF_NPC_TEMPLATE NPCs (npc_template_maker).
        self.makers = crate::npc_makers::NpcMakers::new(world, &mut self.states);
        self.spotlights = crate::point_spotlight::Spotlights::new(world);
        self.setup_spawn_parents(world);
        self.start_logic(world, new_game);
    }

    /// Timers' first delays and logic_auto's outputs.
    pub(super) fn start_logic(&mut self, world: &World, new_game: bool) {
        // CTimerEntity::Spawn -> Enable -> ResetTimer: random timers draw their first delay.
        for (id, e) in world.entities.iter().enumerate() {
            if e.class() == "logic_timer" && number(e, "UseRandomTime", 0.) != 0. {
                self.states[id].timer_at = self.time + self.timer_interval(e);
            }
        }
        self.start_scripts_on_spawn(world);
        super::inputs::spawn_lights(self, world);
        self.beams.spawn(world, self.time);
        self.sprites.spawn(world);
        self.dynamic_lights.spawn(world, self.time);
        for (id, origin) in self.func_trains.spawn(world, self.time) {
            self.states[id].origin = origin;
        }
        // CLogicAuto::Activate: its outputs wait for the think 0.2 s on.
        self.logic_auto_at = Some((self.time + LOGIC_AUTO_THINK, new_game));
        // The client draws nothing of the map before that think's outputs have run (the
        // owner saw d1_trainstation_01's G-Man, in front of the master player start, for
        // the 0.2 s before OnNewGame's viewcontrol_black and intro fade): black until it.
        self.screen_fades.push(ScreenFade::new(
            [0, 0, 0, 255],
            0.,
            0.,
            FFADE_OUT | FFADE_STAYOUT,
        ));
    }
    /// CLogicAuto::Think once its time has come: OnMapSpawn, and OnNewGame on a new game.
    pub(super) fn think_logic_autos(&mut self, world: &World) {
        let Some((at, new_game)) = self.logic_auto_at else {
            return;
        };
        if self.time + 1e-6 < at {
            return;
        }
        self.logic_auto_at = None;
        for (id, entity) in world.entities.iter().enumerate() {
            if entity.class() == "logic_auto" {
                self.fire(id, "OnMapSpawn", usize::MAX);
                if new_game {
                    self.fire(id, "OnNewGame", usize::MAX);
                }
            }
        }
        // Lift the spawn hold; the outputs just fired are delivered in this same tick.
        self.screen_fades.push(ScreenFade::new(
            [0, 0, 0, 0],
            0.,
            0.,
            FFADE_IN | FFADE_PURGE,
        ));
    }
    /// The map's first logic_auto think has not run yet (nothing of the map is drawn).
    pub fn logic_autos_pending(&self) -> bool {
        self.logic_auto_at.is_some()
    }
    /// The logic_auto think at once: a background level's first frame is drawn through the
    /// camera its spawn outputs enable, before any time passes (no flash of the spawn view).
    pub fn think_logic_autos_now(&mut self, world: &World) {
        if let Some((at, _)) = self.logic_auto_at.as_mut() {
            *at = self.time;
        }
        self.think_logic_autos(world);
    }
    /// An enabled logic_timer whose time has come fires OnTimer and draws its next time.
    pub(super) fn think_timer(&mut self, world: &World, id: usize) {
        let e = &world.entities[id];
        let state = &self.states[id];
        if e.class() == "logic_timer" && state.enabled && state.timer_at <= self.time {
            self.states[id].timer_at = self.time + self.timer_interval(e);
            self.fire(id, "OnTimer", usize::MAX);
        }
    }
    /// The spawn state of one entity (map load and runtime spawns share this).
    pub(super) fn initial_state(world: &World, e: &Entity, diagnostics: &mut Diagnostics) -> State {
        let base_rotation =
            physics::angles(parse_vec3(e.get("angles").unwrap_or("0 0 0")).unwrap_or(Vec3::ZERO));
        let flags = number(e, "spawnflags", 0.) as u32;
        let Motion {
            direction,
            travel,
            rotating,
            angle,
            axis,
            fraction,
            speed,
        } = Motion::of(world, e, flags);
        let outputs = entity_outputs(e, diagnostics);
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
            script_wait: false,
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
            touchers: Vec::new(),
            touch_armed: false,
            last_trigger: -1e6,
            value: number(e, "startvalue", 0.),
            timer_at: number(e, "RefireTime", 1.).max(0.015) as f64,
            outputs,
            parent: None,
            corpse: false,
        };
        s.origin = s.base_origin + s.translation * s.fraction;
        s.rotation = s.base_rotation * Quat::from_axis_angle(s.axis, s.angle * s.fraction);
        s
    }
    pub(super) fn initialize_idle_activities(&mut self, world: &World) {
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
    /// CPointTemplate removes its Template01..16 entities at map spawn unless spawnflag 1
    /// ("don't remove template entities") is set; they are created by ForceSpawn.
    pub(super) fn hold_template_children(&mut self, world: &World) {
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
}

/// ACT_IDLE through a held weapon's acttable (CAI_BaseNPC::TranslateActivity:
/// NPC_TranslateActivity, identity at idle outside combat and the player's squad, then
/// Weapon_TranslateActivity): weapon_pistol -> ACT_IDLE_PISTOL, weapon_smg1 ->
/// ACT_IDLE_SMG1 ...; unarmed or a weapon without an ACT_IDLE row: ACT_IDLE.
pub fn weapon_idle_activity(weapon: &str) -> &'static str {
    crate::npc_weapons::translate(&weapon.trim().to_ascii_lowercase(), "ACT_IDLE")
        .unwrap_or("ACT_IDLE")
}

#[cfg(test)]
mod tests {
    use super::Scene;
    use crate::player_damage::{FFADE_OUT, FFADE_PURGE, FFADE_STAYOUT};
    use glam::Vec3;
    use modkit_core::World;

    #[test]
    fn the_map_stays_black_until_the_logic_auto_think() {
        let world = World::default();
        let mut scene = Scene::new(&world);
        let hold = std::mem::take(&mut scene.screen_fades);
        assert_eq!(hold.len(), 1);
        assert_eq!(hold[0].color, [0, 0, 0, 255]);
        assert_eq!(hold[0].flags, FFADE_OUT | FFADE_STAYOUT);
        for _ in 0..13 {
            scene.tick(&world, Vec3::ZERO, 0.015);
        }
        assert!(scene.screen_fades.is_empty());
        scene.tick(&world, Vec3::ZERO, 0.015);
        assert_eq!(scene.screen_fades.len(), 1);
        assert_ne!(scene.screen_fades[0].flags & FFADE_PURGE, 0);
        // A background level thinks at once: the hold and its purge arrive together.
        let mut background = Scene::new(&world);
        background.think_logic_autos_now(&world);
        let flags: Vec<_> = background.screen_fades.iter().map(|f| f.flags).collect();
        assert_eq!(flags.len(), 2);
        assert_ne!(flags[1] & FFADE_PURGE, 0);
    }
}
