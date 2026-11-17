//! `light`, `light_spot` and `light_glspot` (SDK lights.cpp CLight): their switchable style in
//! [`crate::lightstyles::LightStyles`].

use super::{Delivery, Scene};
use crate::entities::number;
use modkit_core::World;

/// TurnOn, TurnOff, Toggle, SetPattern, FadeToPattern.
pub(super) fn light(scene: &mut Scene, d: &Delivery) -> bool {
    let time = scene.time;
    let pattern = &d.pending.parameter;
    match d.input {
        "turnon" => scene.lightstyles.switch(d.id, Some(true)),
        "turnoff" => scene.lightstyles.switch(d.id, Some(false)),
        "toggle" => scene.lightstyles.switch(d.id, None),
        "setpattern" => scene.lightstyles.set_pattern(d.id, pattern),
        "fadetopattern" => scene.lightstyles.fade_to_pattern(d.id, pattern, time),
        _ => return false,
    }
    true
}

/// `light_dynamic` (SDK dynamiclight.cpp CDynamicLight): TurnOn, TurnOff, Toggle.
pub(super) fn dynamic(scene: &mut Scene, d: &Delivery) -> bool {
    scene.dynamic_lights.input(d.id, d.input)
}

/// CLight::Spawn: an unnamed light is inert; a named one sets its style. The styles the map's
/// lightmap faces use are the ones that animate.
pub(in crate::entities) fn spawn_lights(scene: &mut Scene, world: &World) {
    let faces = world.lightmap_styles.iter();
    (scene.lightstyles).use_styles(faces.flat_map(|f| f.styles.iter().map(|(s, _)| *s)));
    for (id, e) in world.entities.iter().enumerate() {
        if matches!(e.class(), "light" | "light_spot" | "light_glspot")
            && e.get("targetname").is_some_and(|n| !n.is_empty())
        {
            scene.lightstyles.spawn(
                id,
                number(e, "style", 0.) as i32,
                e.get("pattern").unwrap_or(""),
                number(e, "defaultstyle", 0.) as i32,
                number(e, "spawnflags", 0.) as u32,
            );
        }
    }
}
