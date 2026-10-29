//! Proportional UI scaling. Source scales its 480-line UI design by the screen height, which
//! fits every screen at least as wide as the narrowest retail aspect (5:4). On narrower screens
//! (square, portrait) the height would make the UI wider than the screen, so the scale follows
//! the [`design_height`]: the height of the widest-design area that fits the screen. On every
//! screen from 5:4 to ultrawide it equals the screen height, exactly as in retail.

use crate::gameui::menu::BASE_SCREEN_HEIGHT;
use glam::Vec2;
use source_ui::console::dialog::Scaling;
use source_ui::widgets::Painter;

/// The height the UI scales by: the screen height, limited to the width divided by the
/// narrowest design aspect ([`crate::tuning::narrowest_design_aspect`]).
pub fn design_height(canvas: Vec2) -> f32 {
    canvas
        .y
        .min(canvas.x / crate::tuning::narrowest_design_aspect())
}

/// How much the main menu grows on a screen wider than the widest design aspect
/// ([`crate::tuning::widest_design_aspect`], 16:9): a wide screen has the height of a 16:9 one, so
/// the menu would look small beside all that width. It grows in proportion to the extra width up
/// to [`crate::tuning::widescreen_menu_scale_max`]; 1 on every screen up to 16:9.
pub fn widescreen_menu_scale(canvas: Vec2) -> f32 {
    let aspect = canvas.x / canvas.y.max(1.);
    (aspect / crate::tuning::widest_design_aspect())
        .clamp(1., crate::tuning::widescreen_menu_scale_max())
}

/// The dialog scale for a canvas: [`Painter::scale_for`] its [`design_height`], so dialogs fit
/// narrow (square, portrait) screens.
pub fn dialog_scale(canvas: Vec2) -> f32 {
    Painter::scale_for(design_height(canvas))
}

/// HL2's dialog scaling for the toolkit's dialogs: [`dialog_scale`] and [`design_height`].
pub const SCALING: Scaling = Scaling {
    dialog_scale,
    design_height,
};

/// Pixels per 480-line design unit for sizes and horizontal offsets.
pub fn scale(canvas: Vec2) -> f32 {
    design_height(canvas) / BASE_SCREEN_HEIGHT
}

/// Pixels per 480-line design unit for vertical positions of HUD panels: the full screen height,
/// so panels keep their place on the screen (bottom panels stay at the bottom) while
/// [`scale`] limits their size.
pub fn y_scale(canvas: Vec2) -> f32 {
    canvas.y / BASE_SCREEN_HEIGHT
}

/// The horizontal span the HUD is laid out in for Options > Video > HUD Aspect Ratio (client
/// `hud_aspect`, "force the aspect ratio of the hud"): `(x, width)` in screen pixels. A screen
/// wider than `aspect` (width / height) gets a centered span of that aspect, full height; a
/// narrower screen, and Fit Display (`None`), keep the whole width.
pub fn hud_frame(screen: Vec2, aspect: Option<f32>) -> (f32, f32) {
    match aspect {
        Some(aspect) if screen.x > screen.y * aspect => {
            let width = screen.y * aspect;
            ((screen.x - width) * 0.5, width)
        }
        _ => (0., screen.x),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::vec2;

    #[test]
    fn the_hud_frame_is_centered_on_wide_screens_and_whole_on_narrow_ones() {
        let screen = vec2(3440., 1440.);
        assert_eq!(hud_frame(screen, None), (0., 3440.));
        assert_eq!(hud_frame(screen, Some(16. / 9.)), (440., 2560.));
        assert_eq!(hud_frame(screen, Some(4. / 3.)), (760., 1920.));
        // At exactly the chosen aspect, or narrower, the frame is the screen.
        assert_eq!(hud_frame(vec2(1920., 1080.), Some(16. / 9.)), (0., 1920.));
        assert_eq!(hud_frame(vec2(1280., 1024.), Some(16. / 9.)), (0., 1280.));
        assert_eq!(hud_frame(vec2(1080., 1920.), Some(4. / 3.)), (0., 1080.));
        let (x, width) = hud_frame(vec2(5120., 1440.), Some(16. / 10.));
        assert!((x * 2. + width - 5120.).abs() < 1e-3, "centered");
    }

    #[test]
    fn the_menu_grows_only_on_screens_wider_than_16_by_9() {
        for size in [
            vec2(1080., 1920.),
            vec2(1280., 1024.),
            vec2(1920., 1080.),
            vec2(1920., 1200.),
        ] {
            assert_eq!(widescreen_menu_scale(size), 1., "{size}");
        }
        let ultrawide = widescreen_menu_scale(vec2(3440., 1440.));
        assert!(
            (ultrawide - (3440. / 1440.) / (16. / 9.)).abs() < 1e-5,
            "{ultrawide}"
        );
        assert_eq!(
            widescreen_menu_scale(vec2(5120., 1440.)),
            crate::tuning::widescreen_menu_scale_max(),
            "capped"
        );
    }

    #[test]
    fn landscape_screens_keep_the_height_and_narrow_ones_follow_the_width() {
        for size in [
            vec2(1024., 768.),
            vec2(1280., 1024.),
            vec2(1920., 1080.),
            vec2(5120., 1440.),
        ] {
            assert_eq!(design_height(size), size.y, "{size}");
        }
        assert_eq!(design_height(vec2(1080., 1080.)), 864.);
        assert_eq!(design_height(vec2(1080., 1920.)), 864.);
    }
}
