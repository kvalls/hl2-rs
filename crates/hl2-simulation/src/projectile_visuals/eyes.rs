//! Eyes drawn as sprites: scanners' kRenderGlow eyes, and manhacks' eye and light.
use super::{ProjectileVisuals, GLOW_SUFFIX};
use crate::ai::manhack::{
    GLOW_BRIGHTNESS, GLOW_COLOR, GLOW_SCALE, GLOW_SPRITE, STUNNED_COLOR, STUNNED_SCALE,
};
use crate::ai::scanner::flash::EYE_SPRITE;
use crate::physics::Physics;
use glam::Vec3;

impl ProjectileVisuals {
    /// Scanners' lit eyes: kRenderGlow sprites, drawn without a depth test once a line from the
    /// view shows them. The SDK asks the renderer how much of the sprite shows; here it is all or
    /// nothing, and a body seen from behind hides its eye.
    pub(super) fn eye_quads(&mut self, axes: (Vec3, Vec3), view: Vec3, physics: &Physics) {
        let flares = std::mem::take(&mut self.eye_flares);
        let width = self.sprites.get(EYE_SPRITE).map(|s| s.image.width as f32);
        for flare in flares.iter().filter(|f| f.forward.dot(view - f.at) > 0.) {
            if physics
                .projectile_ray(view, flare.at, &[flare.entity])
                .is_some()
            {
                continue;
            }
            let before = self.quads.len();
            let size = width.unwrap_or(0.) * 0.5 * flare.eye.scale;
            self.quad(EYE_SPRITE, flare.at, axes, size, 0., 1.);
            if let Some(quad) = self.quads.get_mut(before) {
                quad.material.push_str(GLOW_SUFFIX);
                let [r, g, b] = flare.eye.color;
                quad.color = [r, g, b, 255];
            }
        }
        self.eye_flares = flares;
    }

    /// Manhacks' eye and light: kRenderTransAdd sprites, so the world hides them as it hides the
    /// body, at brightness 164 of 255; red, or orange and smaller while stunned.
    pub(super) fn manhack_glow_quads(&mut self, axes: (Vec3, Vec3)) {
        let glows = std::mem::take(&mut self.manhack_glows);
        let width = self
            .sprites
            .get(GLOW_SPRITE)
            .map_or(0., |s| s.image.width as f32);
        for &(at, stunned) in &glows {
            let (color, scale) = if stunned {
                (STUNNED_COLOR, STUNNED_SCALE)
            } else {
                (GLOW_COLOR, GLOW_SCALE)
            };
            let before = self.quads.len();
            self.quad(GLOW_SPRITE, at, axes, width * 0.5 * scale, 0., 1.);
            if let Some(quad) = self.quads.get_mut(before) {
                let [r, g, b] = color.map(|c| (f32::from(c) * GLOW_BRIGHTNESS).round() as u8);
                quad.color = [r, g, b, 255];
            }
        }
        self.manhack_glows = glows;
    }
}
