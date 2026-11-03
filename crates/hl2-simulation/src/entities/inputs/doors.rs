//! Doors and linear movers: `func_door`, `func_door_rotating`, `prop_door_rotating` and
//! `func_movelinear` (Open, Close, Toggle, OpenAwayFrom, SetPosition).

use super::{Delivery, Scene};
use crate::entities::number;
use glam::Vec3;

/// Open / Close / Toggle, and OpenAwayFrom on `prop_door_rotating`.
pub(super) fn door(scene: &mut Scene, d: &Delivery) -> bool {
    let (input, class, id, p) = (d.input, d.class, d.id, d.pending);
    if !matches!(input, "open" | "close" | "toggle" | "openawayfrom")
        || input == "openawayfrom" && class != "prop_door_rotating"
    {
        return false;
    }
    if scene.states[id].locked && input != "close" {
        scene.fire(id, "OnLockedUse", p.activator);
        return true;
    }
    if class == "prop_door_rotating"
        && matches!(input, "open" | "openawayfrom")
        && scene.states[id].target > 0.
    {
        return true;
    }
    let target = if input == "open" || input == "openawayfrom" {
        1.
    } else if input == "close" {
        0.
    } else {
        1. - scene.states[id].target
    };
    if class == "prop_door_rotating" && target > 0. && scene.states[id].fraction.abs() < 0.00001 {
        // Resolve the named opener when the deferred input is delivered,
        // using its current origin and the original caller/activator.
        let opener = if input == "openawayfrom" {
            let name = p.parameter.to_lowercase();
            if name == "!player" || name == "!activator" && p.activator == usize::MAX {
                Some(d.player_feet)
            } else {
                let mut lookup = p.clone();
                lookup.target = p.parameter.clone();
                scene
                    .targets(d.world, &lookup)
                    .into_iter()
                    .find_map(|other| {
                        scene
                            .states
                            .get(other)
                            .filter(|s| !s.killed)
                            .map(|s| s.origin)
                    })
            }
        } else {
            p.opener
        };
        let state = &mut scene.states[id];
        let back = match number(d.entity, "opendir", 0.) as i32 {
            1 => false,
            2 => true,
            _ => opener
                .is_some_and(|origin| (state.rotation * Vec3::X).dot(origin - state.origin) > 0.),
        };
        state.angle = state.angle.abs() * if back { 1. } else { -1. };
    }
    scene.states[id].target = target;
    scene.states[id].return_at = None;
    scene.fire(
        id,
        if target > 0. { "OnOpen" } else { "OnClose" },
        p.activator,
    );
    true
}

/// A door that also takes SetPosition (0..1 along its travel).
pub(super) fn movelinear(scene: &mut Scene, d: &Delivery) -> bool {
    if door(scene, d) {
        return true;
    }
    if d.input != "setposition" {
        return false;
    }
    scene.states[d.id].target = d.value.clamp(0., 1.);
    true
}
