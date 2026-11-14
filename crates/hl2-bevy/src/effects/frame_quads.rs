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

/// The view map effects face: script_intro's camera view when it shows one (the HL1 room is
/// only seen through it), else the player's view.
fn effect_view(
    game: &Gameplay,
    (eye, direction): ([f32; 3], glam::Vec3),
) -> (glam::Vec3, glam::Vec3) {
    let scene = &game.scene;
    scene
        .intro
        .script
        .frame(scene.time)
        .and_then(|frame| frame.camera)
        .and_then(|id| scene.states.get(id))
        .map_or((glam::Vec3::from_array(eye), direction), |s| {
            (s.origin, s.rotation * glam::Vec3::X)
        })
}

/// env_sprite quads this frame; glows are hidden where the world blocks the line to them.
pub(super) fn env_sprites(
    game: &Gameplay,
    effects: &mut Effects,
    physics: &hl2_simulation::physics::Physics,
    view: ([f32; 3], glam::Vec3),
) {
    let (eye, forward) = effect_view(game, view);
    let right = forward.cross(glam::Vec3::Z).normalize_or(glam::Vec3::Y);
    let up = right.cross(forward).normalize_or(glam::Vec3::Z);
    let states = &game.scene.states;
    let source = &mut effects.source;
    let sprites = &source.sprites;
    let size = |m: &str| {
        let image = &sprites.get(m)?.image;
        Some((f32::from(image.width), f32::from(image.height)))
    };
    let position = |id: usize| states.get(id).filter(|s| !s.killed).map(|s| s.origin);
    let visible = |p: glam::Vec3| {
        physics
            .brush_ray(eye, p)
            .is_none_or(|hit| hit.distance(p) < 8.)
    };
    game.scene
        .sprites
        .quads((eye, right, up), position, size, visible, &mut source.quads);
}

/// env_beam segments this frame, facing [`effect_view`].
pub(super) fn env_beams(game: &Gameplay, effects: &mut Effects, view: ([f32; 3], glam::Vec3)) {
    let scene = &game.scene;
    let states = &scene.states;
    let view = effect_view(game, view);
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
