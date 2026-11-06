//! This frame's new shot effects and the scanners' lit eyes.
use super::Effects;
use crate::gameplay::Gameplay;

pub(super) fn shot_effects(game: &Gameplay, effects: &mut Effects) {
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
    let mut random = || {
        *rng ^= *rng << 13;
        *rng ^= *rng >> 7;
        *rng ^= *rng << 17;
        (*rng >> 40) as f32 / (1u32 << 24) as f32
    };
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
