//! The flexible game frame of Video > Aspect Ratio "Fit Display": the frame takes the aspect
//! ratio of the window or display inside the supported `9:16..=32:9` range
//! ([`crate::tuning::clamp_supported_aspect`]) and is letterboxed or pillarboxed beyond it.

use glam::{UVec2, Vec2};
use source_ui::viewport::ViewportBounds;

/// [`ViewportBounds::compute`] for a frame that is either fixed (`flexible == false`) or follows
/// the window's aspect ratio. A flexible frame ([`flexible_frame_resolution`]) fills the whole
/// window while the window's aspect ratio is inside the supported `9:16..=32:9` range (no
/// one-pixel stripes from rounding the frame size); beyond it the frame keeps the clamped
/// aspect and is letterboxed (narrower than 9:16) or pillarboxed (wider than 32:9).
pub fn compute_with_mode(
    target_resolution: UVec2,
    physical_window_size: UVec2,
    scale_factor: f32,
    flexible: bool,
) -> ViewportBounds {
    let window = physical_window_size.max(UVec2::ONE);
    let win_aspect = window.x as f32 / window.y as f32;
    if !flexible || crate::tuning::clamp_supported_aspect(win_aspect) != win_aspect {
        return ViewportBounds::compute(target_resolution, physical_window_size, scale_factor);
    }
    let scale = scale_factor.max(1.0);
    ViewportBounds {
        target_resolution: target_resolution.max(UVec2::ONE),
        physical_offset: UVec2::ZERO,
        physical_size: window,
        logical_offset: Vec2::ZERO,
        logical_size: window.as_vec2() / scale,
    }
}

/// Physical size of the area a flexible frame is presented in inside a window of
/// `physical_window_size`: the whole window when its aspect ratio is in the supported
/// `9:16..=32:9` range, otherwise the largest centred area of the clamped aspect.
pub fn flexible_presented_size(physical_window_size: UVec2) -> Vec2 {
    let window = physical_window_size.max(UVec2::ONE).as_vec2();
    let win_aspect = window.x / window.y;
    let aspect = crate::tuning::clamp_supported_aspect(win_aspect);
    if win_aspect > aspect {
        Vec2::new(window.y * aspect, window.y)
    } else if win_aspect < aspect {
        Vec2::new(window.x, window.x / aspect)
    } else {
        window
    }
}

/// Physical size of the content area without letterbox or pillarbox black stripes in a window of
/// `physical_window_size` when flexible aspect ratio is enabled.
pub fn flexible_presented_resolution(physical_window_size: UVec2) -> UVec2 {
    flexible_presented_size(physical_window_size)
        .round()
        .max(Vec2::ONE)
        .as_uvec2()
}

/// Game-frame resolution of a flexible-aspect frame (Video > Aspect Ratio "Fit Display") in a
/// window of `physical_window_size` pixels, for the `configured` video resolution.
///
/// The frame takes the presented area's aspect ratio (the window's, clamped to `9:16..=32:9`).
/// The configured resolution sets the detail: the frame's short side is the configured short
/// side (`1920x1080` renders 1080 lines in landscape and 1080 columns in portrait), never more
/// than the presented area has pixels, so a small window renders one frame pixel per window
/// pixel and a large display upscales like a fixed frame does.
pub fn flexible_frame_resolution(configured: UVec2, physical_window_size: UVec2) -> UVec2 {
    let presented = flexible_presented_size(physical_window_size);
    let short = configured.max(UVec2::ONE).min_element() as f32;
    let scale = (short / presented.min_element().max(1.0)).min(1.0);
    (presented * scale).round().max(Vec2::ONE).as_uvec2()
}

/// Client size of a windowed-mode window for a flexible-aspect frame, in the unit of `bounds`.
///
/// Unlike [`source_ui::viewport::windowed_size`], the request is not scaled uniformly to fit: each side is limited
/// to `bounds` on its own (the frame follows whatever shape the window gets), and only then is
/// the window narrowed so its aspect ratio stays in `9:16..=32:9`.
pub fn flexible_windowed_size(resolution: UVec2, bounds: Option<Vec2>) -> Vec2 {
    let mut size = resolution.max(UVec2::ONE).as_vec2();
    if let Some(monitor) = bounds.filter(|m| m.x >= 1.0 && m.y >= 1.0) {
        size = size.min(monitor);
    }
    let aspect = size.x / size.y;
    let clamped = crate::tuning::clamp_supported_aspect(aspect);
    if aspect > clamped {
        size.x = size.y * clamped;
    } else if aspect < clamped {
        size.y = size.x / clamped;
    }
    size.floor().max(Vec2::ONE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flexible_frames_follow_the_window_within_the_supported_range() {
        let configured = UVec2::new(1920, 1080);
        // Landscape and portrait windows: the frame takes their shape, short side = 1080.
        assert_eq!(
            flexible_frame_resolution(configured, UVec2::new(3456, 2234)),
            UVec2::new(1671, 1080)
        );
        assert_eq!(
            flexible_frame_resolution(configured, UVec2::new(1080, 1920)),
            UVec2::new(1080, 1920)
        );
        assert_eq!(
            flexible_frame_resolution(configured, UVec2::new(5120, 1440)),
            UVec2::new(3840, 1080)
        );
        // A small window renders one frame pixel per window pixel.
        assert_eq!(
            flexible_frame_resolution(configured, UVec2::new(1000, 700)),
            UVec2::new(1000, 700)
        );
        // Beyond 32:9 and below 9:16 the frame keeps the clamped aspect.
        let wide = flexible_frame_resolution(configured, UVec2::new(8000, 1000));
        assert!(
            (wide.x as f32 / wide.y as f32 - 32. / 9.).abs() < 0.01,
            "{wide}"
        );
        let tall = flexible_frame_resolution(configured, UVec2::new(500, 2000));
        assert!(
            (tall.x as f32 / tall.y as f32 - 9. / 16.).abs() < 0.01,
            "{tall}"
        );

        for window in [
            UVec2::new(3456, 2234),
            UVec2::new(1080, 1920),
            UVec2::new(1001, 999),
            UVec2::new(5120, 1440),
        ] {
            let frame = flexible_frame_resolution(configured, window);
            let vp = compute_with_mode(frame, window, 2.0, true);
            assert_eq!(vp.physical_size, window, "{window}: no stripes");
            assert!(!vp.has_black_stripes());
            assert_eq!(vp.logical_size, window.as_vec2() / 2.0);
            let corner = vp.cursor_to_canvas(vp.logical_size).unwrap();
            assert!((corner - frame.as_vec2()).abs().max_element() < 1e-3);
        }
        let window = UVec2::new(8000, 1000);
        let frame = flexible_frame_resolution(configured, window);
        let vp = compute_with_mode(frame, window, 1.0, true);
        assert!(vp.is_vertical_stripes(), "pillarboxed beyond 32:9");
        let window = UVec2::new(500, 2000);
        let frame = flexible_frame_resolution(configured, window);
        let vp = compute_with_mode(frame, window, 1.0, true);
        assert!(vp.is_horizontal_stripes(), "letterboxed below 9:16");
        // A fixed frame is unchanged by the mode switch.
        assert_eq!(
            compute_with_mode(UVec2::new(1024, 768), UVec2::new(1920, 1080), 1.0, false),
            ViewportBounds::compute(UVec2::new(1024, 768), UVec2::new(1920, 1080), 1.0)
        );
    }

    #[test]
    fn flexible_windowed_size_limits_each_side_and_the_aspect() {
        let room = Some(Vec2::new(1512.0, 900.0));
        assert_eq!(
            flexible_windowed_size(UVec2::new(1280, 720), room),
            Vec2::new(1280.0, 720.0)
        );
        assert_eq!(
            flexible_windowed_size(UVec2::new(1080, 1920), room),
            Vec2::new(1080.0, 900.0),
            "a too-tall request takes the room's height; the frame follows the window"
        );
        assert_eq!(
            flexible_windowed_size(UVec2::new(1080, 1920), Some(Vec2::new(1512.0, 2000.0))),
            Vec2::new(1080.0, 1920.0)
        );
        assert_eq!(
            flexible_windowed_size(UVec2::new(400, 1920), room),
            Vec2::new(400.0, 711.0),
            "narrower than 9:16 after the cut is shortened"
        );
        assert_eq!(
            flexible_windowed_size(UVec2::new(2560, 1440), room),
            Vec2::new(1512.0, 900.0),
            "each side limited on its own"
        );
        assert_eq!(
            flexible_windowed_size(UVec2::new(4000, 500), None),
            Vec2::new(1777.0, 500.0),
            "wider than 32:9 is narrowed"
        );
    }

    #[test]
    fn flexible_presented_resolution_excludes_letter_and_pillar_box_stripes() {
        // Supported landscape, square, and portrait aspect ratios have no black stripes
        assert_eq!(
            flexible_presented_resolution(UVec2::new(1920, 1080)),
            UVec2::new(1920, 1080)
        );
        assert_eq!(
            flexible_presented_resolution(UVec2::new(5120, 1440)),
            UVec2::new(5120, 1440)
        );
        assert_eq!(
            flexible_presented_resolution(UVec2::new(1080, 1920)),
            UVec2::new(1080, 1920)
        );

        // Ultrawide beyond 32:9 has pillarbox black stripes; width is clamped to 32:9
        let pillarboxed = flexible_presented_resolution(UVec2::new(8000, 1000));
        assert_eq!(pillarboxed, UVec2::new(3556, 1000));

        // Ultra-tall narrower than 9:16 has letterbox black stripes; height is clamped to 9:16
        let letterboxed = flexible_presented_resolution(UVec2::new(500, 2000));
        assert_eq!(letterboxed, UVec2::new(500, 889));
    }
}
