//! Registered inputs for the local player, which has no map entity index.
use super::{Delivery, Scene};

pub(super) fn input(scene: &mut Scene, delivery: &Delivery) -> bool {
    match delivery.input {
        "enableflashlight" => scene.flashlight.request_enabled(true),
        "disableflashlight" => scene.flashlight.request_enabled(false),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flashlight::Context;
    use glam::Vec3;
    use modkit_core::{Entity, World};

    #[test]
    fn invalid_entity_id_does_not_deliver_to_the_player() {
        let world = World::default();
        let mut scene = Scene::new(&world);
        scene.send(42, "DisableFlashlight", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        let state = scene.flashlight.step(
            true,
            1,
            true,
            None,
            Context {
                suit: true,
                ..Default::default()
            },
        );
        assert!(state.on && !state.disabled);
    }

    #[test]
    fn player_target_is_independent_of_the_output_activator() {
        let world = World {
            entities: vec![Entity {
                properties: vec![
                    ("classname".into(), "logic_relay".into()),
                    ("OnTrigger".into(), "!player,DisableFlashlight,,0,-1".into()),
                ],
            }],
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        let context = Context {
            suit: true,
            ..Default::default()
        };
        scene.flashlight.step(true, 1, true, None, context);
        scene.fire(0, "OnTrigger", 0); // The relay, rather than the player, is activator.
        scene.tick(&world, Vec3::ZERO, 0.015);
        let state = scene.flashlight.step(true, 0, true, None, context);
        assert!(state.disabled && !state.on);
        assert!(scene.send_named("!player", "EnableFlashlight", "", 0.));
        scene.tick(&world, Vec3::ZERO, 0.015);
        assert!(scene.flashlight.step(true, 1, true, None, context).on);
        assert_eq!(scene.diagnostics.inputs_delivered, 2);
    }
}
