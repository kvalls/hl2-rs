//! Quads added every frame from the scene: env_beam segments and active stunsticks' halos.
use super::Effects;
use crate::gameplay::Gameplay;

/// A uniform 0..1 from the effects' xorshift state.
pub(super) fn xorshift(rng: &mut u64) -> f32 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    (*rng >> 40) as f32 / (1u32 << 24) as f32
}

/// env_beam segments this frame, facing script_intro's camera view when it shows one (the HL1
/// room's beams are only seen through it), else the player's view.
pub(super) fn env_beams(
    game: &Gameplay,
    effects: &mut Effects,
    (eye, direction): ([f32; 3], glam::Vec3),
) {
    let scene = &game.scene;
    let states = &scene.states;
    let view = scene
        .intro
        .script
        .frame(scene.time)
        .and_then(|frame| frame.camera)
        .and_then(|id| states.get(id))
        .map_or((glam::Vec3::from_array(eye), direction), |s| {
            (s.origin, s.rotation * glam::Vec3::X)
        });
    let Effects { source, rng, .. } = effects;
    let position = |id: usize| states.get(id).filter(|s| !s.killed).map(|s| s.origin);
    scene.beams.quads(
        scene.time,
        view,
        position,
        &mut || xorshift(rng),
        &mut source.quads,
    );
}

/// Active stunsticks' halos (C_WeaponStunStick::DrawModel), new sizes every frame.
pub(super) fn stunstick_glows(game: &Gameplay, effects: &mut Effects, direction: glam::Vec3) {
    if effects
        .source
        .sprites
        .contains_key(hl2_simulation::muzzle_effects::STUNSTICK_GLOW)
    {
        let right = direction.cross(glam::Vec3::Z).normalize_or(glam::Vec3::Y);
        let up = right.cross(direction).normalize_or(glam::Vec3::Z);
        let Effects { source, rng, .. } = &mut *effects;
        let mut random = || xorshift(rng);
        for frame in &game.stunstick_glows {
            source
                .quads
                .extend(hl2_simulation::muzzle_effects::stunstick_glow(
                    *frame,
                    right,
                    up,
                    &mut random,
                ));
        }
    }
}
