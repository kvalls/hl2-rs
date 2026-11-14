//! `env_beam` (SDK EnvBeam.cpp CEnvBeam) and `env_sprite` (sprite.cpp CSprite): their state
//! in [`crate::env_beam::EnvBeams`] and [`crate::env_sprite::EnvSprites`].

use super::{Delivery, Scene};

/// TurnOn, TurnOff, Toggle, StrikeOnce.
pub(super) fn beam(scene: &mut Scene, d: &Delivery) -> bool {
    let time = scene.time;
    scene.beams.input(d.id, d.input, time)
}

/// ShowSprite, HideSprite, ToggleSprite.
pub(super) fn sprite(scene: &mut Scene, d: &Delivery) -> bool {
    scene.sprites.input(d.id, d.input)
}
