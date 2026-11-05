//! Scanners in the world (DESIGN 14o session 46): `ai::scanner` decides; this stage steers once per
//! think and carries the body every tick, the part a physics motor does in the SDK. Bodies go
//! through NPCs and the player here, as birds do: a scanner bumping into them is not modeled.

use super::super::host::{clamp_yaw, yaw_of};
use super::super::metropolice::{CopRequest, PLAYER};
use super::super::scanner::{self, Scanner, COND_FLY_BLOCKED, COND_FLY_CLEAR};
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

/// One think of flight: where it wants to be, the thrust that takes it there, and its sounds.
fn steer(s: &mut Scanner, air: &Air, scene: &Scene, combat: bool) {
    let origin = scene.states[air.id].origin;
    let interval = THINK_INTERVAL as f32;
    s.fly_blocked = s.enemy_at.map(|at| blocked(air, origin, at));
    let hover = |s: &Scanner| s.hover_point(origin).map(|p| above_ground(air, p));
    let wanted = s.next_leg(origin).or_else(|| hover(s));
    match wanted {
        Some(point) => s.hover.fly_scanner(point - origin, interval),
        None => s.hover.coast(interval),
    }
    avoid(s, air, origin, interval);
    s.hover.add_noise(s.bob, scanner::NOISE, scene.time as f32);
    s.hover.limit(scanner::MAX_SPEED);
    if let Some(look) = s.enemy_eye.or(wanted) {
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
            if now >= s.next_steer {
                s.next_steer = now + THINK_INTERVAL;
                steer(s, &air, scene, combat);
            }
            let dt = air.step(&mut s.hover.velocity, &mut s.last_step, scene);
            turn(s, scene, id, dt);
        }
    }
}

/// A scanner's part of GatherConditions: its enemy, its health and the way between them.
fn gather(actor: &mut AiActor, scene: &Scene, player: &PlayerState, id: usize) {
    let enemy = actor.npc.enemy;
    let known = enemy
        .and_then(|e| actor.npc.enemies.find(Some(e)))
        .map(|memory| memory.last_known);
    let AiClass::Scanner(s) = &mut actor.class else {
        return;
    };
    let eye = enemy.and_then(|e| {
        if e == PLAYER {
            Some(player.eye())
        } else {
            let height = Vec3::Z * super::super::host::EYE_HEIGHT;
            scene.states.get(e).map(|state| state.origin + height)
        }
    });
    s.enemy_at = known;
    s.enemy_eye = eye;
    s.enemy_in_suit = enemy == Some(PLAYER) && player.suit;
    let health = scene.states[id].value;
    s.max_health = s.max_health.max(health);
    s.health = health / s.max_health.max(1.);
    match s.fly_blocked {
        Some(true) => actor.npc.conditions.set(COND_FLY_BLOCKED),
        Some(false) => actor.npc.conditions.set(COND_FLY_CLEAR),
        None => {}
    }
}

/// What the classes in the air add to GatherConditions.
pub(super) fn gather_air(actor: &mut AiActor, scene: &Scene, player: &PlayerState, id: usize) {
    birds::gather(actor, scene, player);
    gather(actor, scene, player, id);
}
