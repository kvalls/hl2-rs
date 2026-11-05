//! Barnacles in the world (DESIGN 14o session 46): `ai::barnacle` decides, this stage gives it
//! the world's facts and carries out its commands. Barnacles sit beside the schedule actors:
//! they have no memory, senses or mover, and a map can hold dozens of them.

use super::super::barnacle::{
    self, Barnacle, Command, Facts, Phase, Prey, PreyFacts, Tongue, TonguePose,
};
use super::super::senses::SightWorld;
use super::{AiRuntime, Disposition, MeleeHit, PhysicsSight, PlayerHit, PlayerState};
use crate::ai_hull::{center_height, class_hull, eye_height};
use crate::entities::Scene;
use crate::npc::Controller;
use crate::npc_probe::{Hull, NpcCollisionWorld, Query};
use crate::physics::Physics;
use glam::Vec3;
use modkit_core::movement::TICK;
use modkit_core::World;
use serde::Serialize;

/// MASK_SOLID_BRUSHONLY: the tongue hangs through NPCs and props to the architecture.
const MASK_SOLID_BRUSHONLY: u32 = 0x400b;
/// The middle of the barnacle's box under its origin.
const BODY_CENTER: f32 = 20.;
/// A bite at the mouth kills whatever it holds (health is set lazily, so it cannot be read here).
const LETHAL: f32 = 1e6;
/// The fastest a held prop follows the tongue, u/s.
const MAX_PROP_SPEED: f32 = 300.;
/// How hard a tasted prop is thrown.
const SPIT_SPEED: f32 = 50.;
/// Items and weapons have wide trigger boxes: only ones this close are taken.
const ITEM_TOUCH: f32 = 16.;
/// How fast the rope's length follows the tongue, u/s. A think changes the length in steps;
/// taken at once, each step would throw the prey upward.
const ROPE_FOLLOW: f32 = 120.;
/// The rest out of the player's PVS: 1 to 1.5 s, staggered.
const FAR_THINK: f64 = 1.;
const FAR_THINK_SPREAD: f64 = 0.5;

/// One barnacle: its mind, its rope and what its events have reached.
#[derive(Clone, Debug, Serialize)]
pub struct BarnacleActor {
    pub mind: Barnacle,
    /// Built at the first think, which traces the floor.
    #[serde(skip)]
    tongue: Option<Tongue>,
    events_until: f64,
    /// `attack_player` loops, and studio events fire once per start: the next turn's bite.
    next_bite: Option<f64>,
    /// Damage since the last think flinches it and drops an ambushing tongue.
    hurt: bool,
    /// A held prop's grab point, from its center.
    grip: Vec3,
    /// The rope's length, which follows the tongue's at `ROPE_FOLLOW`.
    rope_altitude: f32,
    frame: u64,
    /// The player's PVS held it at the last think.
    near: bool,
}

impl BarnacleActor {
    /// The map's barnacles.
    pub fn all(world: &World) -> std::collections::BTreeMap<usize, Self> {
        let entities = world.entities.iter().enumerate();
        entities
            .filter_map(|(id, e)| Some((id, Self::for_entity(id, e)?)))
            .collect()
    }
    pub fn for_entity(id: usize, e: &modkit_core::Entity) -> Option<Self> {
        if e.class() != "npc_barnacle" {
            return None;
        }
        let number = |key: &str| e.get(key).and_then(|v| v.trim().parse::<f32>().ok());
        let flags = number("spawnflags").unwrap_or(0.) as u32;
        let rest = number("RestDist").unwrap_or(16.);
        Some(Self {
            mind: Barnacle::new(flags, rest, 0x2545_f491_4f6c_dd1d ^ id as u64),
            tongue: None,
            events_until: 0.,
            next_bite: None,
            hurt: false,
            grip: Vec3::ZERO,
            rope_altitude: 0.,
            frame: 0,
            near: false,
        })
    }
}

/// What the stage needs on every barnacle.
struct Stage<'a> {
    world: &'a World,
    physics: &'a Physics,
    player: PlayerState,
    precriminal: bool,
    now: f64,
}

/// Distance from `root` straight down to the architecture.
fn floor_distance(physics: &Physics, root: Vec3, id: usize) -> Option<f32> {
    let point = Hull {
        mins: Vec3::ZERO,
        maxs: Vec3::ZERO,
    };
    // The origin sits on the ceiling, often a hair inside it.
    [1., 8.].into_iter().find_map(|down| {
        let start = root - Vec3::Z * down;
        let end = start - Vec3::Z * barnacle::FLOOR_TRACE;
        let query = Query {
            contents_mask: MASK_SOLID_BRUSHONLY,
            excluded_entities: &[id],
            ..Default::default()
        };
        let hit = physics.npc_trace_hull(start, end, point, query);
        (!hit.trace.start_solid).then_some(down + hit.trace.fraction * barnacle::FLOOR_TRACE)
    })
}
fn overlaps(a: (Vec3, Vec3), b: (Vec3, Vec3)) -> bool {
    a.0.cmple(b.1).all() && a.1.cmpge(b.0).all()
}
/// A prop the tongue can lift: light enough, moveable, and an item only when right at it.
fn liftable(stage: &Stage, entity: usize, origin: Vec3, touch: (Vec3, Vec3)) -> Option<Vec3> {
    let (mass, _, moveable) = stage.physics.entity_rest_state(entity)?;
    if mass > barnacle::MAX_LIFT_MASS || !moveable {
        return None;
    }
    let bounds = stage.physics.entity_world_bounds(entity)?;
    if !overlaps(bounds, touch) {
        return None;
    }
    let center = (bounds.0 + bounds.1) * 0.5;
    let class = stage.world.entities.get(entity).map_or("", |e| e.class());
    let item = class.starts_with("item_") || class.starts_with("weapon_");
    if item && center.truncate().distance(origin.truncate()) > ITEM_TOUCH {
        return None;
    }
    Some(Vec3::new(center.x, center.y, bounds.1.z))
}

impl AiRuntime {
    /// A hit landed on this barnacle (`Scene::npc_damage`).
    pub(super) fn barnacle_hurt(&mut self, id: usize) {
        if let Some(barnacle) = self.barnacles.get_mut(&id) {
            barnacle.hurt = true;
        }
    }
    /// How far the barnacle holding the player moves him this tick.
    pub fn barnacle_hold(&self) -> Option<Vec3> {
        self.player_hold
    }
    /// A barnacle has the player: no walking and no +USE.
    pub fn player_held(&self) -> bool {
        self.player_holder.is_some()
    }
    /// Eaten at the mouth: no corpse follows.
    pub fn take_eaten(&mut self, npc: usize) -> bool {
        self.eaten.remove(&npc)
    }
    /// The barnacle's map inputs (lowercased).
    pub(super) fn barnacle_input(&mut self, id: usize, input: &str, value: f32) {
        let Some(mut barnacle) = self.barnacles.remove(&id) else {
            return;
        };
        let mut commands = Vec::new();
        match input {
            "droptongue" => barnacle.mind.drop_tongue(),
            "setdroptonguespeed" => barnacle.mind.pull_speed = value,
            "letgo" => barnacle.mind.input_let_go(&mut commands),
            _ => {}
        }
        self.barnacle_releases(&mut barnacle, &commands);
        self.barnacles.insert(id, barnacle);
    }
    /// The releases among `commands`, which need no world.
    fn barnacle_releases(&mut self, barnacle: &mut BarnacleActor, commands: &[Command]) {
        for command in commands {
            if let Command::Release(prey) = command {
                self.free_prey(*prey);
                if let Some(tongue) = &mut barnacle.tongue {
                    tongue.detach();
                }
            }
        }
    }
    fn free_prey(&mut self, prey: Prey) {
        match prey {
            Prey::Player => self.player_holder = None,
            Prey::Npc(npc) => {
                self.held_npcs.remove(&npc);
            }
            Prey::Prop(_) => {}
        }
    }

    pub(super) fn tick_barnacles(
        &mut self,
        world: &World,
        scene: &mut Scene,
        controller: &mut Controller,
        physics: &Physics,
        player: PlayerState,
    ) {
        self.player_hold = None;
        if self.barnacles.is_empty() {
            return;
        }
        let stage = Stage {
            world,
            physics,
            player,
            precriminal: scene.globals.is_on("gordon_precriminal"),
            now: scene.time,
        };
        let mut barnacles = std::mem::take(&mut self.barnacles);
        for (&id, barnacle) in barnacles.iter_mut() {
            // Killed by the map, or waiting in a spawn pool or template.
            if scene.states[id].killed || !scene.states[id].visible {
                self.barnacle_gone(id, barnacle, scene);
                continue;
            }
            if scene.states[id].corpse && barnacle.mind.alive() {
                let mut commands = Vec::new();
                barnacle.mind.die(&mut commands);
                self.barnacle_commands(id, barnacle, commands, &stage, scene, controller);
            }
            if !matches!(barnacle.mind.phase, Phase::Dead) {
                self.barnacle_think(id, barnacle, &stage, scene, controller);
            }
            self.barnacle_step(id, barnacle, &stage, scene);
        }
        self.barnacles = barnacles;
    }

    /// Out of the world: the tongue and any hold go with it.
    fn barnacle_gone(&mut self, id: usize, barnacle: &mut BarnacleActor, scene: &mut Scene) {
        if barnacle.mind.holding() {
            let mut commands = Vec::new();
            barnacle.mind.let_go(&mut commands);
            self.barnacle_releases(barnacle, &commands);
        }
        scene.tongues.remove(&id);
    }

    fn barnacle_think(
        &mut self,
        id: usize,
        barnacle: &mut BarnacleActor,
        stage: &Stage,
        scene: &mut Scene,
        controller: &mut Controller,
    ) {
        let mut commands = Vec::new();
        Self::barnacle_events(id, barnacle, stage, scene, &mut commands);
        if stage.now >= barnacle.mind.next_think {
            let root = scene.states[id].origin;
            let floor = floor_distance(stage.physics, root, id);
            let reach = barnacle.mind.reach(floor.unwrap_or(barnacle::FLOOR_TRACE));
            let first = barnacle.tongue.is_none();
            if first {
                if !barnacle.mind.ambush {
                    barnacle.mind.altitude = reach;
                }
                barnacle.rope_altitude = barnacle.mind.altitude;
                barnacle.tongue = Some(Tongue::hanging(root, barnacle.mind.altitude));
            }
            let facts = self.barnacle_facts(id, barnacle, stage, scene, floor, reach);
            if std::mem::take(&mut barnacle.hurt) {
                barnacle.mind.drop_tongue();
                if barnacle.mind.phase == Phase::Hanging {
                    commands.push(Command::Activity("ACT_SMALL_FLINCH"));
                }
            }
            barnacle.mind.think(&facts, &mut commands);
            barnacle.mind.last_think = stage.now;
            // The host's PVS list needs the tongue, so the first think cannot know.
            barnacle.near = scene.look_targets.in_pvs(id);
            let awake = first || barnacle.mind.phase != Phase::Hanging || barnacle.near;
            let rest = if awake {
                barnacle::THINK_INTERVAL
            } else {
                FAR_THINK + FAR_THINK_SPREAD * f64::from(barnacle.mind.roll())
            };
            barnacle.mind.next_think = stage.now + rest;
        }
        self.barnacle_commands(id, barnacle, commands, stage, scene, controller);
    }

    /// Studio events of the current sequence, and the looping player bite's next turn.
    fn barnacle_events(
        id: usize,
        barnacle: &mut BarnacleActor,
        stage: &Stage,
        scene: &mut Scene,
        commands: &mut Vec<Command>,
    ) {
        let since = barnacle.events_until.min(stage.now);
        barnacle.events_until = stage.now;
        let turn = f64::from(scene.ai_sequence_duration(stage.world, id));
        for (name, _) in scene.ai_events(stage.world, id, since, stage.now) {
            match name.as_str() {
                "AE_BARNACLE_BITE" if barnacle.mind.holding() => {
                    barnacle.mind.bite(stage.now, commands);
                    barnacle.next_bite =
                        (barnacle.mind.prey == Some(Prey::Player)).then_some(stage.now + turn);
                }
                "AE_BARNACLE_SPIT" => barnacle.mind.spit(commands),
                _ => {}
            }
        }
        if barnacle.mind.phase != Phase::Biting {
            barnacle.next_bite = None;
        } else if barnacle.next_bite.is_some_and(|at| stage.now >= at) {
            barnacle.mind.bite(stage.now, commands);
            barnacle.next_bite = barnacle.next_bite.map(|at| at + turn);
        }
    }

    fn barnacle_facts(
        &self,
        id: usize,
        barnacle: &BarnacleActor,
        stage: &Stage,
        scene: &Scene,
        floor: Option<f32>,
        reach: f32,
    ) -> Facts {
        let origin = scene.states[id].origin;
        let mind = &barnacle.mind;
        let hanging = mind.phase == Phase::Hanging && mind.altitude >= reach - 1.;
        let finished = scene.ai_activity_finished(id)
            && !scene.states[id].animation.eq_ignore_ascii_case("idle01");
        Facts {
            now: stage.now,
            dt: (stage.now - mind.last_think).clamp(0., 1.5) as f32,
            origin,
            reach,
            floor_distance: floor,
            touching: hanging
                .then(|| self.barnacle_touch(id, stage, scene, origin, reach))
                .flatten(),
            prey: mind
                .prey
                .and_then(|prey| self.prey_facts(id, prey, stage, scene, origin)),
            sequence_finished: finished,
            tip_z: barnacle.tongue.as_ref().map_or(origin.z, |t| t.tip.z),
        }
    }

    /// Prey inside the box around the tongue, and the point the tongue takes it by.
    fn barnacle_touch(
        &self,
        id: usize,
        stage: &Stage,
        scene: &Scene,
        origin: Vec3,
        length: f32,
    ) -> Option<(Prey, Vec3)> {
        let half = Vec3::new(barnacle::TOUCH_HALF_WIDTH, barnacle::TOUCH_HALF_WIDTH, 0.);
        let touch = (origin - half - Vec3::Z * length, origin + half);
        let player = stage.player;
        if !player.noclip && !player.dead && self.player_holder.is_none() {
            let height = if player.crouched { 36. } else { 72. };
            let hull = (
                player.feet - Vec3::new(16., 16., 0.),
                player.feet + Vec3::new(16., 16., height),
            );
            if overlaps(hull, touch) {
                return Some((Prey::Player, player.eye()));
            }
        }
        for &npc in self.actors.keys() {
            let state = &scene.states[npc];
            if state.killed || !state.visible || self.held_npcs.contains(&npc) {
                continue;
            }
            let class = stage.world.entities.get(npc).map_or("", |e| e.class());
            let Some(bounds) = class_hull(class).map(|h| h.bounds()) else {
                continue;
            };
            let hull = (state.origin + bounds.mins, state.origin + bounds.maxs);
            if overlaps(hull, touch)
                && self.npc_disposition(stage.world, id, npc, stage.precriminal) == Disposition::HT
            {
                return Some((Prey::Npc(npc), state.origin + Vec3::Z * eye_height(class)));
            }
        }
        stage.physics.dynamic.keys().find_map(|&entity| {
            liftable(stage, entity, origin, touch).map(|point| (Prey::Prop(entity), point))
        })
    }

    fn prey_facts(
        &self,
        id: usize,
        prey: Prey,
        stage: &Stage,
        scene: &Scene,
        origin: Vec3,
    ) -> Option<PreyFacts> {
        let (position, center, eye_z, alive, height, skip) = match prey {
            Prey::Player => {
                let p = stage.player;
                let height = if p.crouched { 36. } else { 72. };
                (p.feet, p.center(), p.eye().z, !p.dead, height, None)
            }
            Prey::Npc(npc) => {
                let state = scene.states.get(npc)?;
                let class = stage.world.entities.get(npc).map_or("", |e| e.class());
                let height = class_hull(class).map_or(72., |h| {
                    let b = h.bounds();
                    b.maxs.z - b.mins.z
                });
                let center = state.origin + Vec3::Z * center_height(class);
                let eye = state.origin.z + eye_height(class);
                (state.origin, center, eye, !state.killed, height, Some(npc))
            }
            Prey::Prop(prop) => {
                let (position, _) = stage.physics.entity_pose(prop)?;
                (position, position, position.z, true, 0., Some(prop))
            }
        };
        let sight = PhysicsSight {
            physics: stage.physics,
            world: stage.world,
        };
        let body = origin - Vec3::Z * BODY_CENTER;
        Some(PreyFacts {
            position,
            eye_z,
            alive,
            blocked: !sight.visible(body, center, &[id, skip.unwrap_or(id)]),
            noclip: prey == Prey::Player && stage.player.noclip,
            height,
        })
    }

    fn barnacle_commands(
        &mut self,
        id: usize,
        barnacle: &mut BarnacleActor,
        commands: Vec<Command>,
        stage: &Stage,
        scene: &mut Scene,
        controller: &mut Controller,
    ) {
        let origin = scene.states[id].origin;
        for command in commands {
            match command {
                Command::Sound(cue) => scene.sounds.push(crate::sounds::SoundRequest {
                    origin: Some(origin),
                    ..cue.into()
                }),
                Command::Activity(activity) => {
                    if let Some(picked) = scene.select_weighted_sequence(stage.world, id, activity)
                    {
                        scene.ai_set_activity(stage.world, id, &picked);
                    }
                }
                Command::Grab(prey, point) => {
                    self.diagnostics.barnacle_grabs += 1;
                    self.barnacle_grab(id, barnacle, (prey, point), stage, scene, controller)
                }
                Command::Release(prey) => {
                    self.diagnostics.barnacle_releases += 1;
                    self.free_prey(prey);
                    if let Some(tongue) = &mut barnacle.tongue {
                        tongue.detach();
                    }
                }
                Command::Bite(prey) => {
                    self.diagnostics.barnacle_bites += 1;
                    self.barnacle_bite(id, prey, origin)
                }
                Command::Spit(Prey::Prop(prop)) => {
                    self.prop_swats
                        .push((prop, Vec3::NEG_Z * SPIT_SPEED, Vec3::ZERO));
                }
                Command::Spit(_) | Command::CoughGib => {}
            }
        }
    }

    fn barnacle_grab(
        &mut self,
        id: usize,
        barnacle: &mut BarnacleActor,
        (prey, point): (Prey, Vec3),
        stage: &Stage,
        scene: &mut Scene,
        controller: &mut Controller,
    ) {
        let origin = scene.states[id].origin;
        let mut yaw = None;
        match prey {
            Prey::Player => {
                self.player_holder = Some(id);
                let facing = stage.player.facing;
                yaw = Some(facing.y.atan2(facing.x).to_degrees());
            }
            Prey::Npc(npc) => {
                self.held_npcs.insert(npc);
                // Under the mouth, where the rope hangs straight.
                let state = &mut scene.states[npc];
                state.origin.x = origin.x;
                state.origin.y = origin.y;
                self.yield_to_script(npc, stage.world, scene, controller);
                if let Some(actor) = self.actors.get_mut(&npc) {
                    actor.fall = None;
                }
            }
            Prey::Prop(prop) => {
                let center = stage.physics.entity_pose(prop).map_or(point, |p| p.0);
                barnacle.grip = point - center;
            }
        }
        barnacle.rope_altitude = barnacle.mind.altitude;
        if let Some(tongue) = &mut barnacle.tongue {
            tongue.attach(point, yaw);
        }
    }

    fn barnacle_bite(&mut self, id: usize, prey: Prey, origin: Vec3) {
        match prey {
            Prey::Player => self.player_hits.push(PlayerHit {
                attacker: id,
                damage: barnacle::PLAYER_BITE_DAMAGE,
                punch: Vec3::ZERO,
                impulse: Vec3::ZERO,
                fade: None,
            }),
            Prey::Npc(npc) => {
                self.held_npcs.remove(&npc);
                self.eaten.insert(npc);
                self.melee_hits.push(MeleeHit {
                    attacker: id,
                    victim: npc,
                    damage: LETHAL,
                    position: origin,
                    direction: Vec3::Z,
                });
            }
            Prey::Prop(_) => {}
        }
    }

    /// Every tick: the tip, whatever hangs from it, and the rope.
    fn barnacle_step(
        &mut self,
        id: usize,
        barnacle: &mut BarnacleActor,
        stage: &Stage,
        scene: &mut Scene,
    ) {
        let root = scene.states[id].origin;
        let Some(tongue) = &mut barnacle.tongue else {
            return;
        };
        let gap = barnacle.mind.altitude - barnacle.rope_altitude;
        let changed = gap != 0.;
        let follow = ROPE_FOLLOW * TICK;
        barnacle.rope_altitude += gap.clamp(-follow, follow);
        let altitude = barnacle.rope_altitude;
        if changed || !tongue.asleep || barnacle.mind.holding() {
            match barnacle.mind.prey {
                Some(Prey::Player) => {
                    // Prey that hits something holds the tip back with it.
                    let eye = stage.player.eye();
                    tongue.tip = eye;
                    tongue.swing(root, altitude, TICK);
                    self.player_hold = Some(tongue.tip - eye);
                }
                Some(Prey::Npc(npc)) => {
                    tongue.swing(root, altitude, TICK);
                    let class = stage.world.entities.get(npc).map_or("", |e| e.class());
                    scene.states[npc].origin = tongue.tip - Vec3::Z * eye_height(class);
                }
                Some(Prey::Prop(prop)) => {
                    tongue.swing(root, altitude, TICK);
                    if let Some((center, _)) = stage.physics.entity_pose(prop) {
                        let pull = (tongue.tip - barnacle.grip - center) / TICK;
                        self.prop_moves
                            .push((prop, pull.clamp_length_max(MAX_PROP_SPEED)));
                    }
                }
                None => tongue.swing(root, altitude, TICK),
            }
        }
        if tongue.step_rope(root, altitude, TICK) || !scene.tongues.contains_key(&id) {
            barnacle.frame += 1;
            scene.tongues.insert(
                id,
                TonguePose {
                    nodes: tongue.nodes,
                    frame: barnacle.frame,
                },
            );
        }
    }
}
