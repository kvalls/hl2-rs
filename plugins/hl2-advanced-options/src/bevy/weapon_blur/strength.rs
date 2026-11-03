//! Exposure and contribution policy; sample spacing stays independent of strength.
use super::WeaponBlur;
use crate::motion_blur::MotionBlurMode;
use bevy::math::{UVec2, Vec4};

impl WeaponBlur {
    pub(in crate::bevy) fn scale(mode: MotionBlurMode) -> f32 {
        if mode.world() { 0.5 } else { 1. }
    }

    pub(super) fn parameters(&self, size: UVec2) -> Vec4 {
        Vec4::new(
            self.max * self.scale,
            1.25 * self.scale,
            size.x as f32,
            size.y as f32,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            let strong = make(MotionBlurMode::Advanced).parameters(size);
            let soft = make(MotionBlurMode::Ultra).parameters(size);
            assert_eq!([soft.z, soft.w], [strong.z, strong.w]);
            for speed in [0., 0.0001, 0.01, 0.1, 1., 32.] {
                let length = |p: Vec4| (speed * p.y).min(p.x);
                assert_eq!(length(soft), length(strong) * 0.5);
            }
            assert_eq!(make(MotionBlurMode::Ultra).scale, 0.5);
            let mut warming = make(MotionBlurMode::Ultra);
            warming.max = 0.;
            assert_eq!(warming.parameters(size).x, 0.);
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
