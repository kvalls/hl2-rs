//! A manhack's body against what it flies into (DESIGN 14r session 52; Source SDK
//! ai_basenpc_physicsflyer.cpp Simulate, hl2/npc_manhack.cpp CheckCollisions, Bump,
//! HitPhysicsObject; client fx_sparks.cpp ManhackSparks). `ai::manhack::body` holds the
//! arithmetic; this carries the body every tick and shows each think's bumps.

use super::super::super::manhack::body::{self, BumpMarks, Bumped};
use super::super::super::manhack::{Manhack, PHYSICS_MASS};
use super::super::super::scanner::damage::effect;
use super::super::birds::{tick, Air};
use crate::entities::Scene;
use crate::muzzle_effects::FlashLight;
use crate::projectiles::EffectKind;
use glam::Vec3;

/// One tick of its body: the motor pulls it toward the velocity the AI wants (not while the
/// gravity gun holds it: the grab moves it), it flies, and what it meets stops it, keeping the
/// surface for the next think's CheckCollisions. A prop it runs into takes the momentum it lost.
/// The tick's length.
pub(super) fn carry(m: &mut Manhack, air: &Air, scene: &mut Scene, marks: &mut BumpMarks) -> f32 {
    let now = scene.time;
    let dt = tick(&mut m.last_step, now);
    if !m.held {
        let blades = (m.attitude.blade_speed, m.stalled(now));
        m.body = body::motor(m.body, m.hover.velocity, blades, dt);
    }
    let Some(hit) = air.sweep(m.body, dt, scene) else {
        return dt;
    };
    let normal = hit.trace.normal;
    let before = m.body;
    m.body = body::rebound(before, normal);
    let at = scene.states[air.id].origin;
    if let Some(prop) = hit.entity.filter(|&e| movable(air, e)) {
        let edge = at - normal * air.hull.maxs.x;
        marks
            .pushes
            .push((prop, (before - m.body) * PHYSICS_MASS, edge));
    }
    m.contact = Some(Bumped {
        at,
        normal,
        entity: hit.entity,
        start_solid: false,
        sparks: None,
    });
    dt
}

/// A prop the physics simulates, not an NPC.
fn movable(air: &Air, entity: usize) -> bool {
    let npc = air
        .world
        .entities
        .get(entity)
        .is_some_and(|e| e.class().starts_with("npc_"));
    !npc && air.physics.dynamic.contains_key(&entity)
}

/// What this think's bumps show: HitPhysicsObject's push on a prop; where the blades met
/// something across their plane, Bump's spark fan, its light and, on a wall, a cut decal.
pub(super) fn show(m: &mut Manhack, air: &Air, scene: &mut Scene, marks: &mut BumpMarks) {
    let origin = scene.states[air.id].origin;
    for bumped in std::mem::take(&mut m.bumped) {
        if let Some(prop) = bumped.entity.filter(|&e| movable(air, e)) {
            if let Some(state) = scene.states.get(prop) {
                let (impulse, at) = body::blade_push(origin, state.origin, air.hull.maxs.x);
                marks.pushes.push((prop, impulse, at));
            }
        }
        let Some(direction) = bumped.sparks else {
            continue;
        };
        let fan = effect(EffectKind::ManhackSparks, bumped.at, direction, scene.time);
        scene.impact_effects.push(fan);
        marks.lights.push(FlashLight {
            origin,
            ..body::BUMP_LIGHT
        });
        if bumped.normal == Vec3::ZERO {
            continue;
        }
        let back = origin - bumped.normal * body::DECAL_REACH;
        let wall = air.physics.projectile_ray(origin, back, &[air.id]);
        if let Some(hit) = wall.filter(|h| body::takes_decal(h.normal)) {
            let pick = (m.dice() * body::CUT_DECALS.len() as f32) as usize;
            let material = body::CUT_DECALS[pick.min(body::CUT_DECALS.len() - 1)];
            marks.decals.push((hit, material));
        }
    }
}
