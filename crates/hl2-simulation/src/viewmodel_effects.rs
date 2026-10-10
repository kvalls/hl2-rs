//! Sprites and beams the SDK attaches to the player's viewmodel (CSprite::SetAttachment,
//! CBeam start/end attachments): the crossbow charger and load blast, the RPG laser beam
//! and muzzle sprite, and the gravity gun's glow, end-cap, core and beam effects. The
//! description is per SDK state; quads are built in viewmodel model space, which is the
//! viewmodel camera's space (origin, looking down +X). Sprite size follows the existing
//! quad path (half the texture width x scale). CBeam noise is approximated by random
//! perpendicular offsets per segment; the RPG beam's SF_BEAM_SHADEIN fades its start.
use crate::{gameplay::Inventory, projectile_visuals::Quad, weapon_crossbow::ChargerState};
use glam::{Mat4, Vec3};
use modkit_core::animation::Rig;

/// An attachment by 1-based index (LookupAttachment / SetAttachment numbers) or name.
#[derive(Clone, Debug, PartialEq)]
pub enum Anchor {
    Index(usize),
    Name(&'static str),
}
#[derive(Clone, Debug, PartialEq)]
pub struct ViewmodelSprite {
    pub anchor: Anchor,
    pub material: &'static str,
    /// CSprite scale range drawn uniformly per frame (flicker); equal ends for steady.
    pub scale: (f32, f32),
    /// Render brightness 0-255 range.
    pub brightness: (f32, f32),
    pub color: [u8; 3],
}
#[derive(Clone, Debug, PartialEq)]
pub struct ViewmodelBeam {
    pub start: Anchor,
    pub end: Anchor,
    pub material: &'static str,
    pub width: f32,
    pub end_width: (f32, f32),
    pub brightness: f32,
    pub color: [u8; 3],
    /// CBeam::SetNoise amplitude range (0: straight).
    pub noise: (f32, f32),
    /// SF_BEAM_SHADEIN: the start end is dark.
    pub shade_in: bool,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewmodelEffects {
    pub sprites: Vec<ViewmodelSprite>,
    pub beams: Vec<ViewmodelBeam>,
}

const ORANGE: [u8; 3] = [255, 128, 0];
const WHITE: [u8; 3] = [255, 255, 255];
/// StartEffects attachment names.
const GLOW_ATTACHMENTS: [&str; 6] = ["fork1b", "fork1m", "fork1t", "fork2b", "fork2m", "fork2t"];
const BEAM_ATTACHMENTS: [&str; 4] = ["fork1t", "fork2t", "fork1t", "fork2t"];

impl Inventory {
    /// What the active weapon draws on the viewmodel at `time` (nothing while the +USE
    /// carry holsters the weapon).
    pub fn viewmodel_effects(&self, time: f64) -> ViewmodelEffects {
        let mut out = ViewmodelEffects::default();
        if self.carrying() {
            return out;
        }
        match self.active.as_str() {
            "weapon_crossbow" => {
                // m_hChargerSprite at BOLT_TIP_ATTACHMENT (2), kRenderTransAdd 255,128,0;
                // on for START_CHARGE/READY, off after DISCHARGE/OFF.
                let c = &self.crossbow;
                let brightness = c.charger_brightness.value(time);
                if matches!(c.charger, ChargerState::StartCharge | ChargerState::Ready)
                    && brightness > 0.
                {
                    let scale = c.charger_scale.value(time);
                    out.sprites.push(ViewmodelSprite {
                        anchor: Anchor::Index(2),
                        material: crate::weapon_crossbow::CHARGER_SPRITE,
                        scale: (scale, scale),
                        brightness: (brightness, brightness),
                        color: ORANGE,
                    });
                }
                // DoLoadEffect: blueflare1 at attachment 1, brightness 128, scale 0.2,
                // FadeOutFromSpawn (FadeAndDie(0.25) after 0.01 s).
                for at in &c.load_effects {
                    let age = (time - at) as f32;
                    if !(0. ..0.26).contains(&age) {
                        continue;
                    }
                    let fade = 1. - ((age - 0.01).max(0.) / 0.25).min(1.);
                    out.sprites.push(ViewmodelSprite {
                        anchor: Anchor::Index(1),
                        material: crate::weapon_crossbow::LOAD_SPRITE,
                        scale: (0.2, 0.2),
                        brightness: (128. * fade, 128. * fade),
                        color: WHITE,
                    });
                }
            }
            "weapon_rpg" if self.rpg.guiding && !self.rpg.hide_guiding => {
                // StartGuiding: CBeam laser1_noz "laser" -> "laser_end", red, width 0.5,
                // SF_BEAM_SHADEIN; m_hLaserMuzzleSprite redglow1 at "laser".
                out.beams.push(ViewmodelBeam {
                    start: Anchor::Name("laser"),
                    end: Anchor::Name("laser_end"),
                    material: "effects/laser1_noz",
                    width: 0.5,
                    end_width: (0.5, 0.5),
                    brightness: self.rpg.beam_brightness,
                    color: [255, 0, 0],
                    noise: (0., 0.),
                    shade_in: true,
                });
                out.sprites.push(ViewmodelSprite {
                    anchor: Anchor::Name("laser"),
                    material: crate::weapon_rpg::DOT_SPRITE,
                    scale: (self.rpg.muzzle_scale, self.rpg.muzzle_scale),
                    brightness: (255., 255.),
                    color: WHITE,
                });
            }
            "weapon_physcannon" => {
                let holding = self.physcannon.held.is_some();
                let ready = !holding && self.physcannon.open;
                // DoEffectIdle flickers the glow sprites every frame (16-24, 0.3-0.35).
                for name in GLOW_ATTACHMENTS {
                    out.sprites.push(ViewmodelSprite {
                        anchor: Anchor::Name(name),
                        material: "sprites/glow04_noz",
                        scale: (0.3, 0.35),
                        brightness: (16., 24.),
                        color: ORANGE,
                    });
                }
                if holding {
                    // DoEffectHolding: end caps on (flicker 200-255, 0.1-0.15), the four
                    // fork beams to attachment 1, the center core at 255 / 0.2.
                    for name in ["fork1t", "fork2t"] {
                        out.sprites.push(ViewmodelSprite {
                            anchor: Anchor::Name(name),
                            material: "sprites/orangeflare1",
                            scale: (0.1, 0.15),
                            brightness: (200., 255.),
                            color: WHITE,
                        });
                    }
                    for name in BEAM_ATTACHMENTS {
                        out.beams.push(ViewmodelBeam {
                            start: Anchor::Name(name),
                            end: Anchor::Index(1),
                            material: "sprites/orangelight1",
                            width: 0.,
                            end_width: (2., 4.),
                            brightness: 128.,
                            color: WHITE,
                            noise: (8., 16.),
                            shade_in: false,
                        });
                    }
                    out.sprites.push(core("sprites/orangecore1", 0.2, 255.));
                } else if ready {
                    // DoEffectReady: center 128 / 0.15, blast 255 / 0.1.
                    out.sprites.push(core("sprites/orangecore1", 0.15, 128.));
                    out.sprites.push(core("sprites/orangecore2", 0.1, 255.));
                }
            }
            _ => {}
        }
        out
    }
    /// Normalized viewmodel pose parameters: the gravity gun's "active" follows the
    /// element position (C_WeaponPhysCannon sets it on the viewmodel).
    pub fn viewmodel_pose_values(&self, rig: &Rig) -> Vec<f32> {
        let mut params = rig.default_pose_values();
        if self.active == "weapon_physcannon" {
            if let Some(i) = rig.pose_parameter("active") {
                params[i] = rig.pose_parameters[i].normalize(self.physcannon.element_position);
            }
        }
        params
    }
}

fn core(material: &'static str, scale: f32, brightness: f32) -> ViewmodelSprite {
    ViewmodelSprite {
        anchor: Anchor::Index(1),
        material,
        scale: (scale, scale),
        brightness: (brightness, brightness),
        color: WHITE,
    }
}

/// Viewmodel skinning matrices for a base clip with the autoplay layers evaluated at
/// the given pose parameters (the actor path's composition without gesture layers).
pub fn viewmodel_matrices(rig: &Rig, clip: &str, time: f32, params: &[f32]) -> Vec<Mat4> {
    if rig.autoplay.is_empty() {
        return rig.matrices(clip, time);
    }
    let mut pose = rig.local_pose(clip, time);
    let mut trial = pose.clone();
    if rig.accumulate_autoplay(&mut trial, time, params).is_ok() {
        pose = trial;
    }
    rig.local_matrices(&pose)
}

/// Model-space position of an anchor on the posed rig.
pub fn anchor_position(rig: &Rig, matrices: &[Mat4], anchor: &Anchor) -> Option<Vec3> {
    let attachment = match anchor {
        Anchor::Index(i) => rig.attachments.get(i.checked_sub(1)?)?,
        Anchor::Name(name) => rig.attachment(name)?,
    };
    Some(
        rig.attachment_matrix(matrices, attachment)?
            .transform_point3(Vec3::ZERO),
    )
}

/// Viewmodel-space quads: billboards facing the viewmodel camera (+X) and camera-facing
/// beam strips (8 segments with noise). `random` returns [0, 1); `sprite_width` the
/// texture width of a material (None skips it).
pub fn viewmodel_quads(
    effects: &ViewmodelEffects,
    resolve: &dyn Fn(&Anchor) -> Option<Vec3>,
    sprite_width: &dyn Fn(&str) -> Option<f32>,
    random: &mut dyn FnMut() -> f32,
) -> Vec<Quad> {
    let mut quads = Vec::new();
    let (right, up) = (-Vec3::Y, Vec3::Z);
    let pick = |range: (f32, f32), random: &mut dyn FnMut() -> f32| {
        range.0 + (range.1 - range.0) * random()
    };
    for sprite in &effects.sprites {
        let (Some(center), Some(width)) = (resolve(&sprite.anchor), sprite_width(sprite.material))
        else {
            continue;
        };
        let size = width * 0.5 * pick(sprite.scale, random);
        let brightness = pick(sprite.brightness, random).clamp(0., 255.) / 255.;
        if size <= 0. || brightness <= 0. {
            continue;
        }
        let color = sprite
            .color
            .map(|c| (f32::from(c) * brightness).round() as u8);
        let (r, u) = (right * size, up * size);
        quads.push(Quad {
            material: sprite.material.into(),
            positions: [
                center - r - u,
                center - r + u,
                center + r + u,
                center + r - u,
            ],
            color: [color[0], color[1], color[2], 255],
            uv: crate::explosion_particles::UV,
        });
    }
    for beam in &effects.beams {
        let (Some(a), Some(b)) = (resolve(&beam.start), resolve(&beam.end)) else {
            continue;
        };
        if sprite_width(beam.material).is_none() || beam.brightness <= 0. {
            continue;
        }
        let end_width = pick(beam.end_width, random);
        let noise = pick(beam.noise, random);
        const SEGMENTS: usize = 8;
        let along = b - a;
        let (side_axis, lift_axis) = crate::projectile_visuals::basis(along);
        let points: Vec<Vec3> = (0..=SEGMENTS)
            .map(|i| {
                let t = i as f32 / SEGMENTS as f32;
                // Ends stay fixed; interior points wander by up to noise / 2 (in the
                // beam's units: CBeam noise is a fraction of its length / 16).
                let wander = if i == 0 || i == SEGMENTS || noise <= 0. {
                    Vec3::ZERO
                } else {
                    let amplitude = noise * along.length() / 16. / 16.;
                    (side_axis * (random() - 0.5) + lift_axis * (random() - 0.5)) * amplitude
                };
                a + along * t + wander
            })
            .collect();
        for i in 0..SEGMENTS {
            let (p, q) = (points[i], points[i + 1]);
            let t0 = i as f32 / SEGMENTS as f32;
            let t1 = (i + 1) as f32 / SEGMENTS as f32;
            let w0 = beam.width + (end_width - beam.width) * t0;
            let w1 = beam.width + (end_width - beam.width) * t1;
            let side = (q - p).cross((p + q) * 0.5).normalize_or_zero();
            let shade = |t: f32| if beam.shade_in { t } else { 1. };
            let level = beam.brightness / 255. * (shade(t0) + shade(t1)) * 0.5;
            let color = beam.color.map(|c| (f32::from(c) * level).round() as u8);
            quads.push(Quad {
                material: beam.material.into(),
                positions: [
                    p - side * w0 * 0.5,
                    q - side * w1 * 0.5,
                    q + side * w1 * 0.5,
                    p + side * w0 * 0.5,
                ],
                color: [color[0], color[1], color[2], 255],
                uv: [[0., t0], [0., t1], [1., t1], [1., t0]],
            });
        }
    }
    quads
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpg_guiding_draws_the_red_beam_and_muzzle_sprite_and_physcannon_states_differ() {
        let mut inv = Inventory::default();
        inv.active = "weapon_rpg".into();
        inv.rpg.guiding = true;
        inv.rpg.beam_brightness = 128.;
        inv.rpg.muzzle_scale = 0.25;
        let effects = inv.viewmodel_effects(0.);
        assert_eq!(effects.beams.len(), 1);
        assert_eq!(effects.beams[0].start, Anchor::Name("laser"));
        assert_eq!(effects.sprites.len(), 1);
        inv.rpg.hide_guiding = true;
        assert_eq!(inv.viewmodel_effects(0.), ViewmodelEffects::default());

        let mut cannon = Inventory::default();
        cannon.active = "weapon_physcannon".into();
        let closed = cannon.viewmodel_effects(0.);
        assert_eq!((closed.sprites.len(), closed.beams.len()), (6, 0));
        cannon.physcannon.open = true;
        let ready = cannon.viewmodel_effects(0.);
        assert_eq!((ready.sprites.len(), ready.beams.len()), (8, 0));
        cannon.physcannon.held = Some(3);
        let holding = cannon.viewmodel_effects(0.);
        assert_eq!((holding.sprites.len(), holding.beams.len()), (9, 4));
    }

    #[test]
    fn quads_face_the_viewmodel_camera_and_shade_in_beams_start_dark() {
        let effects = ViewmodelEffects {
            sprites: vec![ViewmodelSprite {
                anchor: Anchor::Name("tip"),
                material: "glow",
                scale: (0.5, 0.5),
                brightness: (255., 255.),
                color: [255, 128, 0],
            }],
            beams: vec![ViewmodelBeam {
                start: Anchor::Name("tip"),
                end: Anchor::Index(1),
                material: "glow",
                width: 1.,
                end_width: (1., 1.),
                brightness: 255.,
                color: [255, 0, 0],
                noise: (0., 0.),
                shade_in: true,
            }],
        };
        let resolve = |anchor: &Anchor| match anchor {
            Anchor::Name(_) => Some(Vec3::new(20., 0., -5.)),
            Anchor::Index(_) => Some(Vec3::new(40., 0., -5.)),
        };
        let mut rng = || 0.5;
        let quads = viewmodel_quads(&effects, &resolve, &|_| Some(32.), &mut rng);
        assert_eq!(quads.len(), 1 + 8);
        // 32 / 2 x 0.5 = 8 units half size in the camera plane (x constant).
        let sprite = &quads[0];
        assert!(sprite.positions.iter().all(|p| (p.x - 20.).abs() < 1e-5));
        assert!((sprite.positions[2] - sprite.positions[0]).length() - 16. * 2f32.sqrt() < 1e-4);
        assert_eq!(sprite.color, [255, 128, 0, 255]);
        // Shade-in: the first beam segment is darker than the last.
        assert!(quads[1].color[0] < quads[8].color[0]);
    }
}
