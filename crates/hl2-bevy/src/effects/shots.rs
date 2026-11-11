//! This frame's new shot effects and the scanners' lit eyes.
use super::Effects;
use crate::gameplay::Gameplay;

/// The effects' xorshift: the next value in [0, 1).
pub(super) fn next_random(rng: &mut u64) -> f32 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    (*rng >> 40) as f32 / (1u32 << 24) as f32
}

pub(super) fn shot_effects(
    game: &Gameplay,
    physics: &hl2_simulation::physics::Physics,
    effects: &mut Effects,
) {
    use crate::gameplay::ShotEffect;
    use hl2_simulation::muzzle_effects as fx;
    let Effects {
        source,
        rng,
        shot_serial,
        ..
    } = effects;
    source.eye_flares.clear();
    let eyes = game.ai.scanner_eyes(&game.world, &game.scene);
    source.eye_flares.extend(eyes);
    source.manhack_glows.clear();
    let glows = game.ai.manhack_glows(&game.world, &game.scene);
    source.manhack_glows.extend(glows);
    source.beams.clear();
    let beams = game.ai.scanner_beams(&game.scene);
    let seen = &game.scene.look_targets;
    source.beams.extend(beams.filter(|b| seen.in_pvs(b.entity)));
    // Lit point_spotlights, in the view's PVS like the scanners' beams (attention_pvs).
    let lamps = game.scene.spotlights.beams(&game.world, physics);
    source
        .beams
        .extend(lamps.copied().filter(|b| seen.in_pvs(b.entity)));
    let mut random = || next_random(rng);
    for (serial, at, effect) in &game.shot_effects {
        if *serial <= *shot_serial {
            continue;
        }
        *shot_serial = *serial;
        match effect {
            ShotEffect::NpcFlash(kind, frame) => {
                source.muzzle.flash(*kind, false, *frame, *at, &mut random)
            }
            ShotEffect::MuzzleLight(npc, origin, combine) => {
                source
                    .muzzle
                    .elights
                    .muzzle_flash(*npc, *origin, *combine, *at, &mut random)
            }
            ShotEffect::NpcTracer(start, end, ar2) => source.muzzle.tracer(if *ar2 {
                fx::npc_ar2_tracer(*start, *end, *at, &mut random)
            } else {
                fx::npc_tracer(*start, *end, *at, &mut random)
            }),
            ShotEffect::PlayerTracer(start, end, right, ar2) => source.muzzle.tracer(if *ar2 {
                fx::player_ar2_tracer(*start, *end, *right, *at, &mut random)
            } else {
                fx::player_tracer(*start, *end, *right, *at, &mut random)
            }),
            ShotEffect::Light(light) => source.muzzle.elights.flash(*light, *at),
            ShotEffect::Ar2Impact(origin, normal) => {
                source
                    .muzzle
                    .surface_quads
                    .push(fx::SurfaceQuad::ar2_impact(
                        *origin,
                        *normal,
                        *at,
                        &mut random,
                    ))
            }
        }
    }
}
