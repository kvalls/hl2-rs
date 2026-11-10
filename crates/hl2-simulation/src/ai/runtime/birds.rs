//! Birds in the world (DESIGN 14o session 46): `ai::bird` decides, this stage gives it the map's
//! hints and path_corners and moves the body. A flying bird steers once per think and travels
//! every tick, like MOVETYPE_STEP with FL_FLY; a landing goes to the step fall, and a bird on the
//! ground costs nothing here.

use super::super::bird::{self, Bird, FlyHint, Waypoint, AIR_SPEED, TAKEOFF_SPEED};
use super::super::host::{flight_trace, yaw_of, PhysicsSight};
use super::super::metropolice::{CopRequest, PLAYER};
use super::super::senses::SightWorld;
use super::crabs::{fall_hull, step_fraction};
use super::{event_window_start, AiActor, AiClass, AiRuntime, PlayerState, THINK_INTERVAL};
use crate::ai::conditions::COND_PROVOKED;
use crate::entities::Scene;
use crate::npc_probe::Hull;
use crate::physics::{angles, Physics};
use crate::view_control::angles_toward;
use glam::Vec3;
use modkit_core::World;
use std::sync::Arc;

/// DMG_GENERIC, for a stuck bird's end.
const DMG_GENERIC: u32 = 0;

/// The map's hints birds fly to.
pub(super) fn fly_hints(world: &World) -> Arc<[FlyHint]> {
    world
        .entities
        .iter()
        .filter(|e| matches!(e.class(), "info_node_hint" | "info_node_air_hint"))
        .filter(|e| {
            let number = |key| e.get(key).and_then(|v| v.trim().parse::<i32>().ok());
            number("hinttype") == Some(bird::HINT_TYPE)
                && number("StartHintDisabled").unwrap_or(0) == 0
        })
        .map(|e| FlyHint {
            position: e.origin(),
            group: e.get("Group").unwrap_or("").trim().to_owned(),
        })
        .collect()
}

/// The entity `name` refers to.
pub(super) fn named(world: &World, name: &str) -> Option<usize> {
    world.entities.iter().position(|e| {
        e.get("targetname")
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
    })
}

/// The body's facing along a flight, with its bank.
fn face_flight(scene: &mut Scene, id: usize, direction: Vec3, roll: f32) {
    let (pitch, yaw) = angles_toward(direction);
    scene.states[id].rotation = angles(Vec3::new(pitch, yaw, roll));
}

/// The AE_CROW_TAKEOFF event fell in this tick.
fn takeoff_event(actor: &AiActor, id: usize, world: &World, scene: &Scene) -> bool {
    if actor.npc.activity != "ACT_CROW_TAKEOFF" {
        return false;
    }
    let state = &scene.states[id];
    let current = (state.animation.clone(), state.animation_started);
    let now = scene.time;
    let since = event_window_start(
        actor.events_until.min(now),
        actor.events_sequence.as_ref(),
        &current,
    );
    scene
        .ai_events(world, id, since, now)
        .iter()
        .any(|(name, _)| name == "AE_CROW_TAKEOFF")
}

/// Off the ground and straight at the goal.
fn take_off(bird: &mut Bird, scene: &mut Scene, id: usize) {
    let Some(waypoint) = bird.waypoint else {
        return;
    };
    let now = scene.time;
    scene.states[id].origin.z += 1.;
    let origin = scene.states[id].origin;
    let direction = (waypoint.position - origin).normalize_or_zero();
    bird.start_flying(origin, now);
    bird.velocity = direction * TAKEOFF_SPEED;
    bird.last_step = now;
    bird.next_steer = now + THINK_INTERVAL;
    face_flight(scene, id, direction, 0.);
}

/// The corner after this waypoint's: a hint, or the end of a chain, has none.
fn next_corner(world: &World, waypoint: Option<Waypoint>) -> Option<usize> {
    let corner = waypoint?.corner?;
    named(world, world.entities[corner].get("target")?)
}

/// A route's next leg, or a corner passed (firing its OnPass) and the next taken; true when there
/// is nowhere further.
fn arrive(bird: &mut Bird, world: &World, scene: &mut Scene, id: usize) -> bool {
    if bird.next_leg() {
        return false;
    }
    let Some(next) = next_corner(world, bird.waypoint) else {
        return true;
    };
    if let Some(corner) = bird.waypoint.and_then(|w| w.corner) {
        scene.fire(corner, "OnPass", id);
    }
    let waypoint = Waypoint {
        position: world.entities[next].origin(),
        corner: Some(next),
    };
    bird.waypoint = Some(waypoint);
    bird.goal_corner = Some(waypoint);
    false
}

/// What one flight needs from the world.
pub(super) struct Air<'a> {
    pub world: &'a World,
    pub physics: &'a Physics,
    pub hull: Hull,
    pub id: usize,
}
impl Air<'_> {
    /// One steer. False when the flight has ended here.
    fn steer(&self, actor: &mut AiActor, scene: &mut Scene) -> bool {
        let id = self.id;
        let now = scene.time;
        let origin = scene.states[id].origin;
        let forward = scene.states[id].rotation * Vec3::X;
        let interval = THINK_INTERVAL as f32;
        let AiClass::Bird(bird) = &mut actor.class else {
            return true;
        };
        let Some(steer) = bird.steer(origin, forward, interval, now) else {
            return true;
        };
        if steer.arrived && arrive(bird, self.world, scene, id) {
            land(actor, scene, id);
            return false;
        }
        let reach = steer.direction * AIR_SPEED * interval;
        let probe = (origin, origin + reach);
        let hit = self.trace(probe.0, probe.1);
        let direction = if hit.trace.fraction < 1. {
            bird::deflect(steer.direction, hit.trace.normal)
        } else {
            steer.direction
        };
        bird.velocity = direction * AIR_SPEED;
        let landing = steer.distance < AIR_SPEED
            && bird.last_leg()
            && next_corner(self.world, bird.waypoint).is_none();
        let activity = if landing {
            "ACT_CROW_LAND"
        } else {
            let rolls = [bird.roll(), bird.roll()];
            bird.fly_activity(now, rolls)
        };
        let (_, yaw) = angles_toward(direction);
        face_flight(scene, id, direction, bird::bank(yaw, steer.to_goal));
        if actor.npc.activity != activity {
            actor.npc.activity = activity.into();
            scene.ai_set_activity(self.world, id, activity);
        }
        true
    }
    /// A hull trace of this flight's body.
    pub fn trace(&self, from: Vec3, to: Vec3) -> crate::npc_probe::HullTrace {
        flight_trace(self.physics, (from, to), self.hull, self.id)
    }
    /// One tick along the velocity, which what stops the body takes that part of away. The
    /// tick's length.
    pub fn step(&self, velocity: &mut Vec3, last_step: &mut f64, scene: &mut Scene) -> f32 {
        let dt = tick(last_step, scene.time);
        if let Some(hit) = self.sweep(*velocity, dt, scene) {
            let normal = hit.trace.normal;
            *velocity -= normal * velocity.dot(normal).min(0.);
        }
        dt
    }
    /// Moves the body `velocity` for `dt` until something stops it; what stopped it.
    pub fn sweep(
        &self,
        velocity: Vec3,
        dt: f32,
        scene: &mut Scene,
    ) -> Option<crate::npc_probe::HullTrace> {
        let origin = scene.states[self.id].origin;
        let end = origin + velocity * dt;
        let hit = self.trace(origin, end);
        let fraction = flight_fraction(&hit);
        scene.states[self.id].origin = origin + (end - origin) * fraction;
        (fraction < 1.).then_some(hit)
    }
}

/// The time since the body last moved, at most a tenth of a second; `last_step` becomes now.
pub fn tick(last_step: &mut f64, now: f64) -> f32 {
    let dt = (now - *last_step).clamp(0., 0.1) as f32;
    *last_step = now;
    dt
}

/// How much of a step a flying body takes. One that starts inside something moves freely: physics
/// would push it out in the SDK, and stopping it there would hold it for good.
fn flight_fraction(hit: &crate::npc_probe::HullTrace) -> f32 {
    if hit.trace.start_solid {
        1.
    } else {
        step_fraction(hit)
    }
}

/// The flight ends: the landing activity, and the step fall has it from here. It goes on with its
/// speed; a perch stops it, and in mid-air the schedule takes off again.
fn land(actor: &mut AiActor, scene: &mut Scene, id: usize) {
    let AiClass::Bird(bird) = &mut actor.class else {
        return;
    };
    bird.flying = false;
    bird.waypoint = None;
    let velocity = bird.velocity;
    actor.fall = Some((velocity, scene.time));
    let yaw = yaw_of(scene, id);
    scene.ai_set_yaw(id, yaw);
}

/// FlyAway, now that the world can be asked: a corner out of sight is refused outright; any other
/// corner becomes the goal, and no parameter clears it.
fn fly_away(
    bird: &mut Bird,
    name: &str,
    (world, physics): (&World, &Physics),
    eye: Vec3,
    id: usize,
) {
    if name.is_empty() {
        bird.goal_corner = None;
    } else if let Some(corner) = named(world, name) {
        let position = world.entities[corner].origin();
        if !(PhysicsSight { physics, world }).visible(eye, position, &[id]) {
            return;
        }
        bird.goal_corner = Some(Waypoint {
            position,
            corner: Some(corner),
        });
    }
    bird.forced_fly = true;
}

impl AiRuntime {
    /// The FlyAway input. The flight stage takes it from here.
    pub(super) fn bird_input(&mut self, id: usize, parameter: &str) {
        if let Some(AiClass::Bird(bird)) = self.actors.get_mut(&id).map(|a| &mut a.class) {
            bird.fly_request = Some(parameter.trim().to_owned());
        }
    }
    /// This tick's flying: takeoffs, steering once per think and each bird's step. Without the AI
    /// it only keeps going, as MOVETYPE_STEP does.
    pub(super) fn fly_birds(&mut self, world: &World, scene: &mut Scene, physics: &Physics) {
        let now = scene.time;
        let enabled = self.enabled;
        let held = &self.held_npcs;
        for (&id, actor) in self.actors.iter_mut() {
            if !matches!(actor.class, AiClass::Bird(_))
                || scene.states[id].killed
                || held.contains(&id)
            {
                continue;
            }
            let took_off = takeoff_event(actor, id, world, scene);
            let AiClass::Bird(bird) = &mut actor.class else {
                continue;
            };
            if let Some(name) = bird.fly_request.take() {
                let eye = scene.states[id].origin + Vec3::Z * bird::EYE_HEIGHT;
                fly_away(bird, &name, (world, physics), eye, id);
            }
            if took_off && !bird.flying {
                take_off(bird, scene, id);
            }
            if std::mem::take(&mut bird.drop) {
                actor.fall.get_or_insert((bird.velocity, now));
            }
            if !bird.flying {
                continue;
            }
            actor.fall = None;
            scene.makers.stop_falling(id);
            let air = Air {
                world,
                physics,
                hull: fall_hull(world, id),
                id,
            };
            if enabled && now >= bird.next_steer {
                bird.next_steer = now + THINK_INTERVAL;
                if bird.stuck(scene.states[id].origin, now) {
                    let end = CopRequest::DieInstantly(DMG_GENERIC);
                    actor.class.requests_mut().push(end);
                    continue;
                }
                if !air.steer(actor, scene) {
                    continue;
                }
            }
            if let AiClass::Bird(bird) = &mut actor.class {
                air.step(&mut bird.velocity, &mut bird.last_step, scene);
            }
        }
    }
}

/// The class of a map's bird.
pub(super) fn class_for(e: &modkit_core::Entity, kind: bird::Kind, id: usize) -> AiClass {
    let flags: u32 = e
        .get("spawnflags")
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    let deaf = e.get("deaf").is_some_and(|v| v.trim() == "1");
    let group = e.get("hintgroup").unwrap_or("").trim().to_owned();
    let seed = 0x2545_f491_4f6c_dd1d ^ id as u64;
    let flying = flags & bird::SF_FLYING != 0;
    AiClass::Bird(Box::new(Bird::new(kind, flying, deaf, group, seed)))
}

/// A bird's IdleSound, at its body.
pub(super) fn idle_call(bird: &Bird, scene: &mut Scene, origin: Vec3) {
    scene.sounds.push(crate::sounds::SoundRequest {
        origin: Some(origin + Vec3::Z * bird::EYE_HEIGHT),
        ..bird.kind.idle_sound().into()
    });
}

/// A bird's part of GatherConditions: how near its enemy is, and a FlyAway still to be answered
/// (conditions start empty every think, so the request stands until a schedule takes it).
pub(super) fn gather(actor: &mut AiActor, scene: &Scene, player: &PlayerState) {
    let enemy = actor.npc.enemy;
    let origin = Vec3::from(actor.origin);
    let AiClass::Bird(bird) = &mut actor.class else {
        return;
    };
    let position = enemy.and_then(|e| {
        if e == PLAYER {
            Some(player.feet)
        } else {
            scene.states.get(e).map(|s| s.origin)
        }
    });
    if let Some(position) = position {
        let distance = position.distance(origin);
        bird.sense_enemy(&mut actor.npc.conditions, distance);
    }
    if bird.forced_fly {
        actor.npc.conditions.set(bird::COND_FORCED_FLY);
        actor.npc.conditions.set(COND_PROVOKED);
    }
}

#[cfg(test)]
mod flight_tests {
    use super::*;
    use crate::npc_probe::HullTrace;

    #[test]
    fn a_body_in_the_open_stops_where_it_hits_and_one_inside_something_moves() {
        let mut hit = HullTrace::clear();
        assert_eq!(flight_fraction(&hit), 1.);
        hit.trace.fraction = 0.25;
        assert_eq!(flight_fraction(&hit), 0.25);
        hit.trace.fraction = 0.;
        hit.trace.start_solid = true;
        assert_eq!(flight_fraction(&hit), 1.);
    }
}
