//! UI tuning values embedded from `assets/ui/tuning.json` and parsed once: numbers that shape
//! the UI but come from no game resource.

use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Debug, Deserialize)]
struct Tuning {
    /// Width:height of the narrowest screen the 480-line UI design fits ([`crate::proportional`]).
    narrowest_design_aspect: (u32, u32),
    /// Width:height of the widest screen the menu keeps its retail size on; wider screens grow
    /// the menu ([`crate::proportional::widescreen_menu_scale`]).
    widest_design_aspect: (u32, u32),
    /// Largest growth of the menu on wide screens.
    widescreen_menu_scale_max: f32,
    /// Rows of the tallest menu: the stack whose bottom stays put when the menu grows.
    menu_stack_rows: u32,
    /// Duration of UI focus and window fade animations in seconds (matching sourcescheme Frame.FocusTransitionEffectTime).
    ui_fade_duration: f32,
    /// Unfocused title/subtitle opacity factor (ratio of Frame.OutOfFocusBgColor to Frame.BgColor).
    unfocused_alpha_factor: f32,
    /// Authentic backdrop overlay opacity from MainMenu.Backdrop in sourcescheme.res (156 / 255).
    backdrop_alpha: f32,
    /// Unverified fallback opacity for mid-gameplay pause menu dimming overlay.
    gameplay_dim_alpha: f32,
    /// Where a font engine without GDI's hinting draws the weapon reticle.
    weapon_crosshair: WeaponCrosshair,
    /// Width:height of each centered HUD aspect of Video > HUD Aspect Ratio
    /// ([`crate::proportional::hud_frame`]).
    hud_aspects: HudAspects,
}

/// Native 1080p dots sit 10 pixels to the sides and one pixel left of the view center; the font
/// outline alone spaces them 11 apart on center.
#[derive(Debug, Deserialize)]
struct WeaponCrosshair {
    width_scale: (u32, u32),
    x_offset: f32,
}

/// The centered HUD aspects, width:height, named like [`crate::config::HudAspect`].
#[derive(Debug, Deserialize)]
struct HudAspects {
    normal: (u32, u32),
    wide16x9: (u32, u32),
    wide16x10: (u32, u32),
}

fn data() -> &'static Tuning {
    static DATA: LazyLock<Tuning> = LazyLock::new(|| {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/ui/tuning.json"
        )))
        .expect("embedded UI tuning data is valid")
    });
    &DATA
}

/// Width / height of the narrowest screen the UI design fits.
pub fn narrowest_design_aspect() -> f32 {
    let (width, height) = data().narrowest_design_aspect;
    width as f32 / height as f32
}

/// Width / height of the widest screen the menu keeps its retail size on.
pub fn widest_design_aspect() -> f32 {
    let (width, height) = data().widest_design_aspect;
    width as f32 / height as f32
}

/// Largest growth of the menu on wide screens.
pub fn widescreen_menu_scale_max() -> f32 {
    data().widescreen_menu_scale_max
}

/// Column scale and horizontal offset of the weapon reticle without GDI.
pub fn weapon_crosshair() -> (f32, f32) {
    let crosshair = &data().weapon_crosshair;
    let (numerator, denominator) = crosshair.width_scale;
    (
        numerator as f32 / denominator.max(1) as f32,
        crosshair.x_offset,
    )
}

/// Rows of the tallest menu.
pub fn menu_stack_rows() -> f32 {
    data().menu_stack_rows as f32
}

/// Duration of UI focus and window fade animations in seconds.
pub fn ui_fade_duration() -> f32 {
    data().ui_fade_duration
}

/// Unfocused title and subtitle alpha multiplier.
pub fn unfocused_alpha_factor() -> f32 {
    data().unfocused_alpha_factor
}

/// Authentic backdrop overlay opacity from MainMenu.Backdrop in sourcescheme.res (156 / 255).
pub fn backdrop_alpha() -> f32 {
    data().backdrop_alpha
}

/// Fallback opacity for mid-gameplay pause menu dimming overlay.
pub fn gameplay_dim_alpha() -> f32 {
    data().gameplay_dim_alpha
}

/// Width / height the HUD is laid out in on a wider screen, or `None` for Fit Display.
pub fn hud_aspect_ratio(aspect: crate::config::HudAspect) -> Option<f32> {
    use crate::config::HudAspect;
    let (width, height) = match aspect {
        HudAspect::Fit => return None,
        HudAspect::Normal => data().hud_aspects.normal,
        HudAspect::Wide16x9 => data().hud_aspects.wide16x9,
        HudAspect::Wide16x10 => data().hud_aspects.wide16x10,
    };
    Some(width as f32 / height as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuning_data_parses_with_usable_values() {
        assert!(narrowest_design_aspect() >= 1.0, "a landscape design");
        assert!(widest_design_aspect() > narrowest_design_aspect());
        assert!(widescreen_menu_scale_max() >= 1.0 && menu_stack_rows() >= 1.0);
        assert_eq!(hud_aspect_ratio(crate::config::HudAspect::Fit), None);
        for aspect in &crate::config::HudAspect::ALL[1..] {
            assert!(
                hud_aspect_ratio(*aspect).is_some_and(|r| r >= 1.),
                "{aspect:?}"
            );
        }
    }
}
