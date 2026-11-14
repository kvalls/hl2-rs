//! `light`, `light_spot` and `light_glspot` (SDK lights.cpp CLight): their switchable style in
//! [`crate::lightstyles::LightStyles`].

use super::{Delivery, Scene};
use crate::entities::number;
use modkit_core::World;

/// TurnOn, TurnOff, Toggle, SetPattern.
pub(super) fn light(scene: &mut Scene, d: &Delivery) -> bool {
    match d.input {
        "turnon" => scene.lightstyles.switch(d.id, Some(true)),
        "turnoff" => scene.lightstyles.switch(d.id, Some(false)),
        "toggle" => scene.lightstyles.switch(d.id, None),
        "setpattern" => scene.lightstyles.set_pattern(d.id, &d.pending.parameter),
        _ => return false,
    }
    true
}

/// CLight::Spawn: an unnamed light is inert; a named one sets its style.
pub(in crate::entities) fn spawn_lights(scene: &mut Scene, world: &World) {
    for (id, e) in world.entities.iter().enumerate() {
        if matches!(e.class(), "light" | "light_spot" | "light_glspot")
            && e.get("targetname").is_some_and(|n| !n.is_empty())
        {
            scene.lightstyles.spawn(
                id,
                number(e, "style", 0.) as i32,
                e.get("pattern").unwrap_or(""),
                number(e, "spawnflags", 0.) as u32,
            );
        }
    }
}
