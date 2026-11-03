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
    /// A point already in viewmodel space (a world point moved by the inverse of
    /// FormatViewModelAttachment, e.g. the launch beam's far end).
    Local(Vec3),
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
                let holding = self.physcannon.held.is_some() || self.physcannon.holding_effect;
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
                // DoEffectLaunch: the blast sprite on, scale to 2.0 and brightness to 0
                // over 0.1 s (from the ready values).
                if let Some((at, _)) = self.physcannon.last_launch {
                    let t = ((time - at) / 0.1) as f32;
                    if (0. ..1.).contains(&t) {
                        out.sprites.push(core(
                            "sprites/orangecore2",
                            0.1 + (2. - 0.1) * t,
                            255. * (1. - t),
                        ));
                    }
                }
            }
            _ => {}
        }
        out
    }
    /// Normalized viewmodel pose parameters: the gravity gun's "active" follows the
    /// element position (C_WeaponPhysCannon sets it on the viewmodel).
    /// The active viewmodel's skin (CWeaponCrossbow::SetSkin: 1 while a bolt glows).
    pub fn viewmodel_skin(&self) -> u8 {
        if self.active == "weapon_crossbow" {
            self.crossbow.skin
        } else {
            0
        }
    }
    pub fn viewmodel_pose_values(&self, rig: &Rig) -> Vec<f32> {
        let mut params = rig.default_pose_values();
        if self.active == "weapon_physcannon" {
            if let Some(i) = rig.pose_parameter("active") {
                params[i] = rig.pose_parameters[i].normalize(self.physcannon.element_position);
            }
        }
        params
    }
    /// Resolves the current viewmodel animation state for the active weapon.
    pub fn viewmodel_animation_state<'a>(
        &'a self,
        rig: &'a Rig,
        scene_time: f64,
    ) -> ViewmodelAnimationState<'a> {
        viewmodel_animation_state(
            rig,
            &self.animation,
            self.animation_at,
            self.idle_animation(),
            scene_time,
        )
    }
    /// Posed bone matrices for the active weapon's viewmodel at the given scene time,
    /// smoothly cross-fading finishing non-looping animations into the idle sequence.
    pub fn viewmodel_matrices(&self, rig: &Rig, scene_time: f64) -> Vec<Mat4> {
        let state = self.viewmodel_animation_state(rig, scene_time);
        viewmodel_state_matrices(rig, &state, &self.viewmodel_pose_values(rig))
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

/// Sampling state for a weapon viewmodel animation.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewmodelAnimationState<'a> {
    /// Actively playing a clip at the given local animation time.
    Playing { clip: &'a str, time: f32 },
    /// Non-looping clip has completed its authored duration and is cross-fading
    /// into the idle animation over `fade_time` (Source SDK MaintainSequenceTransitions).
    Transition {
        from: &'a str,
        to: &'a str,
        fade_weight: f32,
        from_time: f32,
        to_time: f32,
    },
    /// Weapon is in steady idle at the given scene time.
    Idle { clip: &'a str, time: f32 },
}

impl<'a> ViewmodelAnimationState<'a> {
    /// Cache lookup label and time key for mesh skinning and pose caching.
    pub fn cache_key(&self, rig: &Rig) -> (String, u32) {
        match self {
            Self::Playing { clip, time } => {
                let key = rig.clips.get(*clip).map_or(time.to_bits(), |c| {
                    if c.frames.len() <= 1 {
                        0
                    } else if !c.looping && *time * c.fps >= (c.frames.len() - 1) as f32 {
                        u32::MAX
                    } else {
                        time.to_bits()
                    }
                });
                ((*clip).to_owned(), key)
            }
            Self::Idle { clip, time } => {
                let key = rig.clips.get(*clip).map_or(time.to_bits(), |c| {
                    if c.frames.len() <= 1 {
                        0
                    } else if !c.looping && *time * c.fps >= (c.frames.len() - 1) as f32 {
                        u32::MAX
                    } else {
                        time.to_bits()
                    }
                });
                ((*clip).to_owned(), key)
            }
            Self::Transition {
                from,
                to,
                fade_weight,
                to_time,
                ..
            } => {
                let key = fade_weight.to_bits() ^ (to_time.to_bits().rotate_left(16));
                (format!("{from}->{to}"), key)
            }
        }
    }
}

/// Resolves the viewmodel animation state, matching Source SDK sequence transitions.
pub fn viewmodel_animation_state<'a>(
    rig: &'a Rig,
    animation: &'a str,
    animation_at: f64,
    idle_animation: &'a str,
    scene_time: f64,
) -> ViewmodelAnimationState<'a> {
    let elapsed = (scene_time - animation_at).max(0.) as f32;
    let Some(clip) = rig.clips.get(animation) else {
        return ViewmodelAnimationState::Idle {
            clip: idle_animation,
            time: scene_time as f32,
        };
    };
    if clip.looping {
        return ViewmodelAnimationState::Playing {
            clip: animation,
            time: elapsed,
        };
    }
    let duration = clip.duration();
    let idle_clip = rig.clips.get(idle_animation);
    // A finished sequence holds its last frame until the weapon sends another. With no idle clip
    // to fade into (the gravity gun's ACT_VM_IDLE is "idle", not "idle01") the cross-fade would
    // blend toward the bind pose, which threw the viewmodel across the screen after each punt.
    if idle_clip.is_none() {
        return ViewmodelAnimationState::Playing {
            clip: animation,
            time: elapsed.min(duration),
        };
    }
    let fade_time = if let Some(idle) = idle_clip {
        if clip.layer.fade_out > 0. && idle.layer.fade_in > 0. {
            clip.layer.fade_out.min(idle.layer.fade_in)
        } else if clip.layer.fade_out > 0. {
            clip.layer.fade_out
        } else {
            0.2
        }
    } else if clip.layer.fade_out > 0. {
        clip.layer.fade_out
    } else {
        0.2
    };

    if elapsed < duration {
        ViewmodelAnimationState::Playing {
            clip: animation,
            time: elapsed,
        }
    } else if fade_time > 0.001 && elapsed < duration + fade_time {
        let frac = ((elapsed - duration) / fade_time).clamp(0.0, 1.0);
        let s_linear = 1.0 - frac;
        let s = s_linear * s_linear * (3.0 - 2.0 * s_linear);
        ViewmodelAnimationState::Transition {
            from: animation,
            to: idle_animation,
            fade_weight: s,
            from_time: duration,
            to_time: scene_time as f32,
        }
    } else {
        ViewmodelAnimationState::Idle {
            clip: idle_animation,
            time: scene_time as f32,
        }
    }
}

/// Posed viewmodel skinning matrices for the given animation state with autoplay layers.
pub fn viewmodel_state_matrices(
    rig: &Rig,
    state: &ViewmodelAnimationState<'_>,
    params: &[f32],
) -> Vec<Mat4> {
    let mut pose = match state {
        ViewmodelAnimationState::Playing { clip, time } => rig.local_pose(clip, *time),
        ViewmodelAnimationState::Idle { clip, time } => rig.local_pose(clip, *time),
        ViewmodelAnimationState::Transition {
            from,
            to,
            fade_weight,
            from_time,
            to_time,
        } => {
            let mut base = rig.local_pose(to, *to_time);
            let finish = rig.local_pose(from, *from_time);
            if let Some(from_clip) = rig.clips.get(&from.to_lowercase()) {
                let _ = modkit_core::animation::slerp_bones(
                    &mut base,
                    &finish,
                    &from_clip.layer,
                    *fade_weight,
                );
            }
            base
        }
    };
    if !rig.autoplay.is_empty() {
        let mut trial = pose.clone();
        let autoplay_time = match state {
            ViewmodelAnimationState::Playing { time, .. } => *time,
            ViewmodelAnimationState::Idle { time, .. } => *time,
            ViewmodelAnimationState::Transition { to_time, .. } => *to_time,
        };
        if rig
            .accumulate_autoplay(&mut trial, autoplay_time, params)
            .is_ok()
        {
            pose = trial;
        }
    }
    rig.local_matrices(&pose)
}

/// Viewmodel skinning matrices for a base clip with the autoplay layers evaluated at
/// the given pose parameters (the actor path's composition without gesture layers).
pub fn viewmodel_matrices(rig: &Rig, clip: &str, time: f32, params: &[f32]) -> Vec<Mat4> {
    viewmodel_state_matrices(
        rig,
        &ViewmodelAnimationState::Playing { clip, time },
        params,
    )
}

/// Model-space position of an anchor on the posed rig.
pub fn anchor_position(rig: &Rig, matrices: &[Mat4], anchor: &Anchor) -> Option<Vec3> {
    let attachment = match anchor {
        Anchor::Index(i) => rig.attachments.get(i.checked_sub(1)?)?,
        Anchor::Name(name) => rig.attachment(name)?,
        Anchor::Local(point) => return Some(*point),
    };
    Some(
        rig.attachment_matrix(matrices, attachment)?
            .transform_point3(Vec3::ZERO),
    )
}

/// The sprites and beams are world entities attached to the viewmodel: the client's
/// FormatViewModelAttachment moves the attachment so it projects at the same screen point
/// with the world FOV, keeping its depth, and the world pass draws them at their world
/// size. Drawn in the viewmodel camera instead, sizes and widths scale by
/// tan(viewmodel FOV / 2) / tan(world FOV / 2) (Source 4:3 horizontal degrees).
/// The retail `viewmodel_fov` (the host's viewmodel camera, Source 4:3 horizontal degrees).
pub const VIEWMODEL_FOV: f32 = 54.;
/// CViewRender::SetUpView: the viewmodel FOV follows any FOV offset of the view
/// (`viewmodel_fov - (default_fov - fov)`, `base` being the `viewmodel_fov` setting), so the
/// retail crossbow zoom (FOV 20) drives it to -1 and the view moves past the viewmodel; at or
/// below 0 it is not drawn.
pub fn viewmodel_fov(base: f32, default_fov: f32, fov: f32) -> f32 {
    base - (default_fov - fov)
}
pub fn attachment_size_factor(viewmodel_fov: f32, world_fov: f32) -> f32 {
    let world = (world_fov.to_radians() * 0.5).tan();
    if world > 0. {
        (viewmodel_fov.to_radians() * 0.5).tan() / world
    } else {
        1.
    }
}
/// Viewmodel-space quads: billboards facing the viewmodel camera (+X) and camera-facing
/// beam strips (8 segments with noise). `random` returns [0, 1); `sprite_width` the
/// texture width of a material (None skips it); `size_factor` from
/// `attachment_size_factor`.
pub fn viewmodel_quads(
    effects: &ViewmodelEffects,
    resolve: &dyn Fn(&Anchor) -> Option<Vec3>,
    sprite_width: &dyn Fn(&str) -> Option<f32>,
    random: &mut dyn FnMut() -> f32,
    size_factor: f32,
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
        let size = width * 0.5 * pick(sprite.scale, random) * size_factor;
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
            let w0 = (beam.width + (end_width - beam.width) * t0) * size_factor;
            let w1 = (beam.width + (end_width - beam.width) * t1) * size_factor;
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
    #[test]
    fn attached_sprites_shrink_to_world_fov_size() {
        let f = super::attachment_size_factor(54., 75.);
        assert!((f - 0.664).abs() < 0.001, "{f}");
        assert!(super::attachment_size_factor(54., 20.) > 2.8);
        assert_eq!(super::viewmodel_fov(VIEWMODEL_FOV, 75., 75.), 54.);
        assert_eq!(super::viewmodel_fov(VIEWMODEL_FOV, 75., 20.), -1.);
        assert_eq!(
            super::viewmodel_fov(70., 75., 75.),
            70.,
            "a larger viewmodel_fov setting"
        );
    }
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
            Anchor::Local(point) => Some(*point),
        };
        let mut rng = || 0.5;
        let quads = viewmodel_quads(&effects, &resolve, &|_| Some(32.), &mut rng, 1.);
        assert_eq!(quads.len(), 1 + 8);
        // 32 / 2 x 0.5 = 8 units half size in the camera plane (x constant).
        let sprite = &quads[0];
        assert!(sprite.positions.iter().all(|p| (p.x - 20.).abs() < 1e-5));
        assert!((sprite.positions[2] - sprite.positions[0]).length() - 16. * 2f32.sqrt() < 1e-4);
        assert_eq!(sprite.color, [255, 128, 0, 255]);
        // Shade-in: the first beam segment is darker than the last.
        assert!(quads[1].color[0] < quads[8].color[0]);
    }

    #[test]
    fn viewmodel_animation_state_transitions_smoothly_to_idle() {
        use modkit_core::animation::{Bone, Clip, ClipLayer, Pose};
        let rig = Rig {
            bones: vec![Bone {
                name: "root".into(),
                parent: None,
                bind: Pose {
                    position: Vec3::ZERO,
                    rotation: glam::Quat::IDENTITY,
                },
                inverse_bind: Mat4::IDENTITY,
            }],
            clips: [
                (
                    "attack".into(),
                    Clip {
                        fps: 30.,
                        looping: false,
                        frames: (0..21)
                            .map(|_| {
                                vec![Pose {
                                    position: Vec3::new(0., -10., 0.),
                                    rotation: glam::Quat::IDENTITY,
                                }]
                            })
                            .collect(),
                        layer: ClipLayer {
                            fade_out: 0.2,
                            fade_in: 0.2,
                            ..Default::default()
                        },
                        events: vec![],
                    },
                ),
                (
                    "idle".into(),
                    Clip {
                        fps: 30.,
                        looping: true,
                        frames: (0..60)
                            .map(|_| {
                                vec![Pose {
                                    position: Vec3::new(0., 10., 0.),
                                    rotation: glam::Quat::IDENTITY,
                                }]
                            })
                            .collect(),
                        layer: ClipLayer {
                            fade_out: 0.2,
                            fade_in: 0.2,
                            ..Default::default()
                        },
                        events: vec![],
                    },
                ),
            ]
            .into(),
            ..Default::default()
        };

        // Attack duration is 20 / 30 = 0.6667s. Fadeout is 0.2s (up to 0.8667s).
        // 1. Before duration: actively playing.
        let state_play = viewmodel_animation_state(&rig, "attack", 0.0, "idle", 0.5);
        assert_eq!(
            state_play,
            ViewmodelAnimationState::Playing {
                clip: "attack",
                time: 0.5,
            }
        );
        let (label, _) = state_play.cache_key(&rig);
        assert_eq!(label, "attack");

        // 2. Exactly at duration (0.6667s): transition starts with weight ~1.0.
        let duration = rig.clips["attack"].duration();
        let state_start = viewmodel_animation_state(&rig, "attack", 0.0, "idle", duration as f64);
        match state_start {
            ViewmodelAnimationState::Transition {
                from,
                to,
                fade_weight,
                from_time,
                ..
            } => {
                assert_eq!(from, "attack");
                assert_eq!(to, "idle");
                assert!((fade_weight - 1.0).abs() < 1e-4);
                assert_eq!(from_time, duration);
            }
            other => panic!("expected Transition, got {other:?}"),
        }

        // 3. Halfway through the 0.2s fadeout (+0.1s): smoothstep weight is exactly 0.5.
        let state_mid =
            viewmodel_animation_state(&rig, "attack", 0.0, "idle", (duration + 0.1) as f64);
        match state_mid {
            ViewmodelAnimationState::Transition { fade_weight, .. } => {
                assert!((fade_weight - 0.5).abs() < 1e-4);
            }
            other => panic!("expected Transition, got {other:?}"),
        }
        let (label_mid, key_mid) = state_mid.cache_key(&rig);
        assert_eq!(label_mid, "attack->idle");
        assert_ne!(key_mid, 0);

        // 4. After duration + fade_out (>= 0.8667s): steady idle.
        let state_idle =
            viewmodel_animation_state(&rig, "attack", 0.0, "idle", (duration + 0.21) as f64);
        assert_eq!(
            state_idle,
            ViewmodelAnimationState::Idle {
                clip: "idle",
                time: (duration + 0.21),
            }
        );

        // 5. Looping animation always plays without fading out.
        let state_loop = viewmodel_animation_state(&rig, "idle", 0.0, "idle", 5.0);
        assert_eq!(
            state_loop,
            ViewmodelAnimationState::Playing {
                clip: "idle",
                time: 5.0,
            }
        );

        // 6. Test matrices blend smoothly.
        let m_play = viewmodel_state_matrices(&rig, &state_play, &[]);
        let p_play = m_play[0].transform_point3(Vec3::ZERO);
        assert_eq!(p_play.y, -10.0);

        let m_mid = viewmodel_state_matrices(&rig, &state_mid, &[]);
        let p_mid = m_mid[0].transform_point3(Vec3::ZERO);
        assert!(
            (p_mid.y - 0.0).abs() < 1e-4,
            "midpoint should be 0.0, got {p_mid:?}"
        );

        let m_idle = viewmodel_state_matrices(&rig, &state_idle, &[]);
        let p_idle = m_idle[0].transform_point3(Vec3::ZERO);
        assert_eq!(p_idle.y, 10.0);

        // 7. Without the idle clip (the gravity gun has no "idle01") a finished clip holds its
        // last frame instead of fading toward the bind pose.
        for after in [0.05, 0.3, 2.0] {
            let held = viewmodel_animation_state(
                &rig,
                "attack",
                0.0,
                "idle01",
                f64::from(duration) + after,
            );
            assert_eq!(
                held,
                ViewmodelAnimationState::Playing {
                    clip: "attack",
                    time: duration,
                }
            );
            let p_held = viewmodel_state_matrices(&rig, &held, &[])[0].transform_point3(Vec3::ZERO);
            assert_eq!(p_held.y, -10.0);
        }
    }
}
