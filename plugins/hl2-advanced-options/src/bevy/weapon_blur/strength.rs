//! Exposure and contribution policy; sample spacing stays independent of strength.
use super::WeaponBlur;
use crate::motion_blur::MotionBlurMode;
use bevy::{
    ecs::{
        resource::Resource,
        system::{Local, Res, ResMut},
    },
    math::{UVec2, Vec4},
    render::extract_resource::ExtractResource,
    time::{Real, Time},
};

/// The frame time the trail length was tuned at: motion over a frame is the trail at 60 FPS.
const REFERENCE_FRAME: f32 = 1. / 60.;
/// The longest exposure, relative to one frame; the trail's maximum length still clamps it.
const MAX_SHUTTER: f32 = 2.5;

/// How many frames of motion one trail spans, so it shows the weapon's speed over a fixed
/// 1/60 s exposure instead of the shorter distance it moves between faster frames (owner,
/// 2026-11-27). Never below one frame: at 60 FPS and slower the trail is what it was.
#[derive(Resource, Clone, Copy, PartialEq, Debug, ExtractResource)]
#[extract_app(bevy::render::RenderApp)]
pub(in crate::bevy) struct Shutter(pub f32);
impl Default for Shutter {
    fn default() -> Self {
        Self(1.)
    }
}

/// The shutter for a smoothed frame time, in steps of 1/20 so a steady rate keeps one value.
fn shutter_for(frame_seconds: f32) -> f32 {
    if frame_seconds <= 0. {
        return 1.;
    }
    ((REFERENCE_FRAME / frame_seconds).clamp(1., MAX_SHUTTER) * 20.).round() / 20.
}

/// Follows the real frame time, smoothed over about five frames so single slow frames do not
/// make the trail flicker.
pub(in crate::bevy) fn update_shutter(
    time: Res<Time<Real>>,
    mut smoothed: Local<f32>,
    mut shutter: ResMut<Shutter>,
) {
    let frame = time.delta_secs();
    *smoothed = if *smoothed <= 0. {
        frame
    } else {
        *smoothed + (frame - *smoothed) * 0.2
    };
    let next = Shutter(shutter_for(*smoothed));
    if *shutter != next {
        *shutter = next;
    }
}

impl WeaponBlur {
    pub(in crate::bevy) fn scale(mode: MotionBlurMode) -> f32 {
        if mode.world() { 0.5 } else { 1. }
    }

    pub(super) fn parameters(&self, size: UVec2, shutter: Shutter) -> Vec4 {
        Vec4::new(
            self.max * self.scale,
            1.25 * self.scale * shutter.0,
            size.x as f32,
            size.y as f32,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shutter_keeps_a_sixtieth_of_a_second_of_motion_from_60_fps_up() {
        assert_eq!(super::shutter_for(1. / 30.), 1.);
        assert_eq!(super::shutter_for(1. / 60.), 1.);
        assert_eq!(super::shutter_for(1. / 120.), 2.);
        assert_eq!(super::shutter_for(1. / 90.), 1.5);
        assert_eq!(super::shutter_for(1. / 500.), 2.5);
        assert_eq!(super::shutter_for(0.), 1.);
        let size = UVec2::new(1031, 773);
        let blur = WeaponBlur {
            max: 0.12,
            scale: 1.,
            world: false,
            camera_moving: false,
        };
        let slow = blur.parameters(size, Shutter(1.));
        let fast = blur.parameters(size, Shutter(2.));
        assert_eq!(fast.y, slow.y * 2.);
        assert_eq!(fast.x, slow.x, "the maximum trail length does not grow");
    }

    #[test]
    fn ultra_halves_both_clamped_and_unclamped_exposure_for_each_camera() {
        for world in [false, true] {
            let make = |mode| WeaponBlur {
                max: 0.12,
                scale: WeaponBlur::scale(mode),
                world,
                camera_moving: true,
            };
            let size = UVec2::new(1031, 773);
            let strong = make(MotionBlurMode::Advanced).parameters(size, Shutter::default());
            let soft = make(MotionBlurMode::Ultra).parameters(size, Shutter::default());
            assert_eq!([soft.z, soft.w], [strong.z, strong.w]);
            for speed in [0., 0.0001, 0.01, 0.1, 1., 32.] {
                let length = |p: Vec4| (speed * p.y).min(p.x);
                assert_eq!(length(soft), length(strong) * 0.5);
            }
            assert_eq!(make(MotionBlurMode::Ultra).scale, 0.5);
            let mut warming = make(MotionBlurMode::Ultra);
            warming.max = 0.;
            assert_eq!(warming.parameters(size, Shutter::default()).x, 0.);
        }
        for mode in [
            MotionBlurMode::Disabled,
            MotionBlurMode::Enabled,
            MotionBlurMode::Advanced,
        ] {
            assert_eq!(WeaponBlur::scale(mode), 1.);
        }
    }
}
