//! Scanners in the world (DESIGN 14o session 46): `ai::scanner` decides; this stage steers once per
//! think and carries the body every tick, the part a physics motor does in the SDK. Bodies go
//! through NPCs and the player here, as birds do: a scanner bumping into them is not modeled.

use super::super::host::PhysicsSight;
use super::super::host::{clamp_yaw, yaw_of};
use super::super::metropolice::{CopRequest, PLAYER};
use super::super::scanner::{self, Mode, Scanner, COND_FLY_BLOCKED, COND_FLY_CLEAR};
use super::super::senses::SightWorld;
use super::super::state::NpcState;
use super::birds::{self, Air};
use super::crabs::{fall_hull, step_fraction};
use super::{AiActor, AiClass, AiRuntime, PlayerState, THINK_INTERVAL};
use crate::entities::Scene;
use crate::physics::{angles, Physics};
use crate::sounds::{entity_emitter, PatchOp, SoundRequest};
use crate::view_control::angles_toward;
use glam::Vec3;
use modkit_core::World;

/// Stopped this close to what it flies at, the way still counts as clear.
const CLEAR_WITHIN: f32 = 50.;
/// The lift of being low: this much speed over the share of the clearance left, which stops
/// counting below `LOWEST_SHARE`.
const GROUND_LIFT: f32 = 50.;
const LOWEST_SHARE: f32 = 0.1;
/// How fast the lean eases, degrees a second. Not from the SDK, whose motor reaches it in a moment.
const LEAN_RATE: f32 = 120.;

/// Whether something stops a straight flight well short of `to`.
fn blocked(air: &Air, from: Vec3, to: Vec3) -> bool {
    let hit = air.trace(from, to);
    (1. - step_fraction(&hit)) * from.distance(to) >= CLEAR_WITHIN
}

/// `point`, raised as far as the ground under it is too near.
fn above_ground(air: &Air, point: Vec3) -> Vec3 {
    let clearance = Vec3::Z * scanner::GROUND_CLEARANCE;
    let hit = air.trace(point, point - clearance);
    point + clearance * (1. - step_fraction(&hit))
}

/// VelocityToAvoidObstacles: it bounces off what it is about to run into, else rises off the
/// ground under it.
fn avoid(s: &mut Scanner, air: &Air, origin: Vec3, interval: f32) {
    let ahead = air.trace(origin, origin + s.hover.velocity * interval);
    if ahead.trace.fraction < 1. {
        s.hover.bounce(ahead.trace.normal);
        return;
    }
    let below = air.trace(origin, origin - Vec3::Z * scanner::GROUND_CLEARANCE);
    if below.trace.fraction < 1. {
        let share = below.trace.fraction.max(LOWEST_SHARE);
        s.hover.velocity.z += GROUND_LIFT / share;
    }
}

/// The corner after `corner`, when its `target` names one.
fn next_corner(world: &World, corner: usize) -> Option<usize> {
    let name = world.entities.get(corner)?.get("target")?;
    let next = birds::named(world, name)?;
    (world.entities[next].class() == "path_corner").then_some(next)
}

/// The start of a placed scanner's path, read once it lives.
fn start_path(s: &mut Scanner, world: &World, id: usize) {
    if !std::mem::replace(&mut s.path_read, true) {
        s.corner = next_corner(world, id);
        s.route.extend(s.corner.map(|c| world.entities[c].origin()));
    }
}

/// A corner reached: its OnPass fires and the next one comes, until the chain ends.
fn pass_corner(s: &mut Scanner, world: &World, scene: &mut Scene, id: usize) {
    let Some(corner) = s.corner.filter(|_| s.route.is_empty()) else {
        return;
    };
    scene.fire(corner, "OnPass", id);
    s.corner = next_corner(world, corner);
    s.route.extend(s.corner.map(|c| world.entities[c].origin()));
}

/// Whether it can see what it is placed to photograph.
fn can_photograph(s: &Scanner, air: &Air, scene: &Scene) -> bool {
    let (Some(target), Some(subject)) = (s.inspect_at, s.inspect) else {
        return false;
    };
    let origin = scene.states[air.id].origin;
    let aim = angles_toward(target - origin).1;
    let sight = PhysicsSight {
        physics: air.physics,
        world: air.world,
    };
    s.photo_placed(origin, target, yaw_of(scene, air.id), aim)
        && sight.visible(origin, target, &[air.id, subject])
}

/// One think of flight: where it wants to be, the thrust that takes it there, and its sounds.
fn steer(s: &mut Scanner, air: &Air, scene: &mut Scene, combat: bool) {
    let origin = scene.states[air.id].origin;
    let interval = THINK_INTERVAL as f32;
    let after = s.inspect_at.or(s.enemy_at);
    s.fly_blocked = after.map(|at| blocked(air, origin, at));
    s.can_photograph = can_photograph(s, air, scene);
    let hover = |s: &Scanner| s.hover_point(origin).map(|p| above_ground(air, p));
    let mut wanted = s.next_leg(origin);
    if wanted.is_none() {
        pass_corner(s, air.world, scene, air.id);
        wanted = s.next_leg(origin).or_else(|| hover(s));
    }
    match wanted {
        Some(point) => s.hover.fly_scanner(point - origin, interval),
        None => s.hover.coast(interval),
    }
    avoid(s, air, origin, interval);
    s.hover.add_noise(s.bob, scanner::NOISE, scene.time as f32);
    s.hover.limit(scanner::MAX_SPEED);
    if let Some(look) = s.enemy_eye.or(s.inspect_at).or(wanted) {
        s.look_yaw = Some(angles_toward(look - origin).1);
    }
    let right = scene.states[air.id].rotation * Vec3::NEG_Y;
    let speed = s.hover.velocity.length();
    s.lean = wanted.map_or(0., |point| scanner::lean(right, point - origin, speed));
    s.engine_sound();
    if let Some(cue) = s.idle_cue(combat) {
        s.requests.push(CopRequest::Sound(cue));
    }
}

/// The body turns toward what it looks at and eases into its lean.
fn turn(s: &mut Scanner, scene: &mut Scene, id: usize, dt: f32) {
    let current = yaw_of(scene, id);
    let yaw = clamp_yaw(s.turn_rate(), current, s.look_yaw.unwrap_or(current), dt);
    s.roll = clamp_yaw(LEAN_RATE, s.roll, s.lean, dt);
    scene.states[id].rotation = angles(Vec3::new(0., yaw, s.roll));
}

/// A killed scanner's engine stops.
fn stop_engine(s: &mut Scanner, scene: &mut Scene, id: usize) {
    if std::mem::take(&mut s.engine_on) {
        let emitter = entity_emitter(id);
        let cue = Scanner::engine_cue();
        scene.sounds.push(SoundRequest {
            origin: Some(scene.states[id].origin),
            ..SoundRequest::patch(cue, emitter, PatchOp::Destroy)
        });
    }
}

impl AiRuntime {
    /// This tick's hovering: a steer once per think and each scanner's step.
    pub(super) fn fly_scanners(&mut self, world: &World, scene: &mut Scene, physics: &Physics) {
        if !self.enabled {
            return;
        }
        let now = scene.time;
        let held = &self.held_npcs;
        for (&id, actor) in self.actors.iter_mut() {
            let combat = actor.npc.state == NpcState::Combat;
            let AiClass::Scanner(s) = &mut actor.class else {
                continue;
            };
            actor.fall = None;
            if scene.states[id].killed {
                stop_engine(s, scene, id);
                continue;
            }
            if held.contains(&id) {
                continue;
            }
            let air = Air {
                world,
                physics,
                hull: fall_hull(world, id),
                id,
            };
            start_path(s, world, id);
            if now >= s.next_steer {
                s.next_steer = now + THINK_INTERVAL;
                steer(s, &air, scene, combat);
            }
            let dt = air.step(&mut s.hover.velocity, &mut s.last_step, scene);
            turn(s, scene, id, dt);
        }
    }
}

/// The eyes of what a scanner is after: an entity with no eyes has its origin.
fn eyes(world: &World, scene: &Scene, player: &PlayerState, subject: usize) -> Option<Vec3> {
    if subject == PLAYER {
        return Some(player.eye());
    }
    let class = world.entities.get(subject)?.class();
    let height = if class.starts_with("npc_") {
        crate::ai_hull::eye_height(class)
    } else {
        0.
    };
    Some(scene.states.get(subject)?.origin + Vec3::Z * height)
}

/// GetGoalDirection: an info_target or an air hint says which side to hover on.
fn marked_side(world: &World, scene: &Scene, subject: usize) -> Option<Vec3> {
    let class = world.entities.get(subject)?.class();
    matches!(class, "info_target" | "info_hint_air")
        .then(|| scene.states[subject].rotation * Vec3::X)
}

/// A scanner's part of GatherConditions: its enemy, its health, what it inspects and the way.
fn gather(actor: &mut AiActor, world: &World, scene: &Scene, player: &PlayerState, id: usize) {
    let enemy = actor.npc.enemy;
    let known = enemy
        .and_then(|e| actor.npc.enemies.find(Some(e)))
        .map(|memory| memory.last_known);
    let AiClass::Scanner(s) = &mut actor.class else {
        return;
    };
    s.enemy_at = known;
    s.enemy_eye = enemy.and_then(|e| eyes(world, scene, player, e));
    s.inspect_at = s.inspect.and_then(|e| eyes(world, scene, player, e));
    s.inspect_facing = s.inspect.and_then(|e| marked_side(world, scene, e));
    s.enemy_in_suit = enemy == Some(PLAYER) && player.suit;
    let health = scene.states[id].value;
    s.max_health = s.max_health.max(health);
    s.health = health / s.max_health.max(1.);
    let conditions = &mut actor.npc.conditions;
    match s.fly_blocked {
        Some(true) => conditions.set(COND_FLY_BLOCKED),
        Some(false) => conditions.set(COND_FLY_CLEAR),
        None => {}
    }
    s.sense_inspection(conditions, enemy.is_some(), scene.time);
}

/// What the classes in the air add to GatherConditions.
pub(super) fn gather_air(
    actor: &mut AiActor,
    world: &World,
    scene: &Scene,
    player: &PlayerState,
    id: usize,
) {
    birds::gather(actor, scene, player);
    gather(actor, world, scene, player, id);
}

/// FindEntityGeneric: what an input's parameter names.
fn subject(world: &World, name: &str, activator: usize) -> Option<usize> {
    match name.trim().to_ascii_lowercase().as_str() {
        "!player" => Some(PLAYER),
        "!activator" => Some(activator),
        wanted => birds::named(world, wanted).or_else(|| {
            let by_class = |e: &modkit_core::Entity| e.class().eq_ignore_ascii_case(wanted);
            world.entities.iter().position(by_class)
        }),
    }
}

impl AiRuntime {
    /// A think's dice. A scanner throws its own, so that one more scanner on a map leaves the
    /// random choices of every other NPC there as they were.
    pub(super) fn roll_for(&mut self, id: usize) -> f32 {
        match self.actors.get_mut(&id).map(|actor| &mut actor.class) {
            Some(AiClass::Scanner(s)) => s.dice(),
            _ => self.random(),
        }
    }
    /// The inputs maps send a scanner.
    pub(super) fn scanner_input(
        &mut self,
        world: &World,
        now: f64,
        target: usize,
        (input, parameter, activator): (&str, &str, usize),
    ) {
        let Some(actor) = self.actors.get_mut(&target) else {
            return;
        };
        let AiClass::Scanner(s) = &mut actor.class else {
            return;
        };
        let mode = match input {
            "inspecttargetphoto" => Mode::Photo,
            "inspecttargetspotlight" => Mode::Spot,
            "setfollowtarget" => Mode::Follow,
            "clearfollowtarget" => return s.clear_follow(),
            "setdistanceoverride" => {
                s.distance_override = parameter.trim().parse().unwrap_or(0.);
                return;
            }
            "inputshouldinspect" => {
                s.ignores_enemies = parameter.trim().parse().unwrap_or(0) == 0;
                if s.ignores_enemies {
                    s.inspect = None;
                }
                return;
            }
            _ => return,
        };
        if let Some(found) = subject(world, parameter, activator) {
            s.inspect_target(found, mode, now);
        }
    }
}

/// The class of a map's scanner.
pub(super) fn class_for(e: &modkit_core::Entity, gagged: bool, id: usize) -> AiClass {
    let mut scanner = Scanner::new(gagged, 0xbb67_ae85_84ca_a73b ^ id as u64);
    scanner.ignores_enemies = e.get("ShouldInspect").map(str::trim) == Some("0");
    AiClass::Scanner(Box::new(scanner))
}
