//! `env_beam` (SDK EnvBeam.cpp CEnvBeam): its state in [`crate::env_beam::EnvBeams`].

use super::{Delivery, Scene};

/// TurnOn, TurnOff, Toggle, StrikeOnce.
pub(super) fn beam(scene: &mut Scene, d: &Delivery) -> bool {
    let time = scene.time;
    scene.beams.input(d.id, d.input, time)
}
