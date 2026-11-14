//! How particular sprites are drawn, where that comes from the entity and not the material.
use hl2_simulation::ai::scanner::{
    flash::EYE_SPRITE,
    spotlight::{BEAM_SPRITE, HALO_SPRITE},
};
use hl2_simulation::weapon_rpg::DOT_SPRITE;

/// The shader's mode for a sprite: -1 for a spotlight's, dim by their color.
pub(super) fn shading(name: &str) -> f32 {
    if name == BEAM_SPRITE || name == HALO_SPRITE {
        -1.
    } else {
        0.
    }
}

/// The sprites drawn in kRenderGlow mode: the RPG laser dot, a scanner's eye and its halo, and
/// env_sprite's sprites (their kRenderGlow draws take the glow variant).
pub(super) fn glows(name: &str) -> bool {
    name == DOT_SPRITE
        || name == EYE_SPRITE
        || name == HALO_SPRITE
        || hl2_simulation::env_sprite::MATERIALS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_spotlight_takes_the_dim_shading() {
        assert_eq!(shading(BEAM_SPRITE), -1.);
        assert_eq!(shading(HALO_SPRITE), -1.);
        assert_eq!(shading(EYE_SPRITE), 0.);
        assert_eq!(shading(DOT_SPRITE), 0.);
    }

    #[test]
    fn the_beam_itself_is_depth_tested_and_its_halo_is_not() {
        assert!(!glows(BEAM_SPRITE));
        assert!(glows(HALO_SPRITE));
        assert!(glows(EYE_SPRITE));
        assert!(glows(DOT_SPRITE));
    }
}
