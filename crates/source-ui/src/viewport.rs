//! Viewport geometry, aspect ratio calculations, and UI canvas hit-test coordinate translation.

use crate::canvas::Canvas;
use glam::{UVec2, Vec2};

/// Describes the viewport placement within a window/display frame for aspect-ratio preservation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportBounds {
    /// Target rendering resolution (e.g. 1024x768).
    pub target_resolution: UVec2,
    /// Physical pixel offset within the host window frame (x = left stripe, y = top stripe).
    pub physical_offset: UVec2,
    /// Physical pixel dimensions of the active viewport area.
    pub physical_size: UVec2,
    /// Logical point offset within the host window frame.
    pub logical_offset: Vec2,
    /// Logical point dimensions of the active viewport area.
    pub logical_size: Vec2,
}

impl Default for ViewportBounds {
    fn default() -> Self {
        Self {
            target_resolution: UVec2::new(1280, 720),
            physical_offset: UVec2::ZERO,
            physical_size: UVec2::new(1280, 720),
            logical_offset: Vec2::ZERO,
            logical_size: Vec2::new(1280.0, 720.0),
        }
    }
}

impl ViewportBounds {
    /// Computes viewport bounds fitting `target_resolution` into a window of `physical_window_size`
    /// with display `scale_factor`, adding letterbox or pillarbox bars when aspect ratios differ.
    pub fn compute(
        target_resolution: UVec2,
        physical_window_size: UVec2,
        scale_factor: f32,
    ) -> Self {
        let win_w = physical_window_size.x.max(1);
        let win_h = physical_window_size.y.max(1);
        let scale = scale_factor.max(1.0);

        let target_w = target_resolution.x.max(1);
        let target_h = target_resolution.y.max(1);

        let target_aspect = target_w as f32 / target_h as f32;
        let win_aspect = win_w as f32 / win_h as f32;

        let (vp_w, vp_h, offset_x, offset_y) = if (win_aspect - target_aspect).abs() < 1e-4 {
            (win_w, win_h, 0, 0)
        } else if win_aspect > target_aspect {
            // Window is wider than target aspect ratio:
            // Pillarbox (vertical black stripes on left & right sides)
            let vp_h = win_h;
            let vp_w = ((win_h as f32 * target_aspect).round() as u32).min(win_w);
            let offset_x = (win_w - vp_w) / 2;
            (vp_w, vp_h, offset_x, 0)
        } else {
            // Window is taller than target aspect ratio:
            // Letterbox (horizontal black stripes on top & bottom sides)
            let vp_w = win_w;
            let vp_h = ((win_w as f32 / target_aspect).round() as u32).min(win_h);
            let offset_y = (win_h - vp_h) / 2;
            (vp_w, vp_h, 0, offset_y)
        };

        let physical_offset = UVec2::new(offset_x, offset_y);
        let physical_size = UVec2::new(vp_w.max(1), vp_h.max(1));
        let logical_offset = Vec2::new(offset_x as f32 / scale, offset_y as f32 / scale);
        let logical_size = Vec2::new(vp_w as f32 / scale, vp_h as f32 / scale);

        Self {
            target_resolution: UVec2::new(target_w, target_h),
            physical_offset,
            physical_size,
            logical_offset,
            logical_size,
        }
    }

    /// Whether black stripes are present (aspect ratio mismatch).
    pub fn has_black_stripes(&self) -> bool {
        self.physical_offset != UVec2::ZERO
    }

    /// True if vertical black stripes (pillarboxing) on left/right sides.
    pub fn is_vertical_stripes(&self) -> bool {
        self.physical_offset.x > 0
    }

    /// True if horizontal black stripes (letterboxing) on top/bottom sides.
    pub fn is_horizontal_stripes(&self) -> bool {
        self.physical_offset.y > 0
    }

    /// Maps a window cursor position in logical window coordinates to UI canvas coordinates.
    ///
    /// The canvas is drawn into the game frame at `target_resolution` and scaled into the
    /// presented area, so the position is scaled from presented points to frame pixels.
    /// Returns `None` if the cursor position falls outside the active viewport region
    /// (e.g. over letterbox/pillarbox black stripes).
    pub fn cursor_to_canvas(&self, cursor_pos: Vec2) -> Option<Vec2> {
        let rel_x = cursor_pos.x - self.logical_offset.x;
        let rel_y = cursor_pos.y - self.logical_offset.y;

        if rel_x >= 0.0
            && rel_x <= self.logical_size.x
            && rel_y >= 0.0
            && rel_y <= self.logical_size.y
        {
            let size = self.logical_size.max(Vec2::ONE);
            Some(Vec2::new(rel_x, rel_y) * self.target_resolution.as_vec2() / size)
        } else {
            None
        }
    }

    /// Resizes the given canvas to the game frame resolution it is drawn into.
    pub fn resize_canvas(&self, canvas: &Canvas) {
        let target = self.target_resolution.as_vec2();
        canvas.resize(target.x, target.y);
    }
}

/// Client size of a windowed-mode window for a configured resolution, in the unit of `bounds`.
///
/// Source sizes a windowed game's client area to the selected video mode. The host passes the
/// monitor size in pixels, so one game-frame pixel maps to one display pixel (also on high-DPI
/// displays). When the request does not fit `bounds`, it is scaled down uniformly so the window
/// stays on screen and keeps the frame's aspect ratio.
pub fn windowed_size(resolution: UVec2, bounds: Option<Vec2>) -> Vec2 {
    let requested = resolution.max(UVec2::ONE).as_vec2();
    let Some(monitor) = bounds.filter(|m| m.x >= 1.0 && m.y >= 1.0) else {
        return requested;
    };
    let fit = (monitor.x / requested.x).min(monitor.y / requested.y);
    if fit >= 1.0 {
        requested
    } else {
        (requested * fit).floor().max(Vec2::ONE)
    }
}

/// Maps a window cursor position in logical window coordinates to UI canvas coordinates,
/// properly offsetting for letterbox/pillarbox black stripes and filtering out positions
/// falling on black stripes outside the active viewport.
pub fn cursor_to_canvas(
    cursor_pos: Option<Vec2>,
    viewport: Option<&ViewportBounds>,
    window_logical_size: Vec2,
) -> Option<Vec2> {
    pointer_to_canvas(cursor_pos, viewport, window_logical_size, Vec2::ZERO)
}

/// Pure function converting pointer coordinates to UI canvas coordinates.
///
/// Ensures consistent hit-testing across window modes (windowed, borderless windowed, fullscreen).
/// Coordinates from `window.cursor_position()` are inner client coordinates where (0, 0) is
/// the top-left of the content area (below any OS title bar). If outer window coordinates are
/// provided along with an outer-vs-inner window origin delta (such as a macOS window title bar
/// height offset), the delta is subtracted so hit testing strictly operates in inner client coordinates.
pub fn pointer_to_canvas(
    cursor_pos: Option<Vec2>,
    viewport: Option<&ViewportBounds>,
    window_logical_size: Vec2,
    outer_to_inner_delta: Vec2,
) -> Option<Vec2> {
    let pos = cursor_pos?;
    let inner_pos = pos - outer_to_inner_delta;
    if let Some(vp) = viewport {
        vp.cursor_to_canvas(inner_pos)
    } else if inner_pos.x >= 0.0
        && inner_pos.x <= window_logical_size.x
        && inner_pos.y >= 0.0
        && inner_pos.y <= window_logical_size.y
    {
        Some(inner_pos)
    } else {
        None
    }
}

/// Resizes a canvas to match the active viewport, or the window logical size if no viewport is active.
pub fn resize_canvas(
    canvas: &Canvas,
    viewport: Option<&ViewportBounds>,
    window_logical_size: Vec2,
) {
    if let Some(vp) = viewport {
        vp.resize_canvas(canvas);
    } else {
        canvas.resize(window_logical_size.x, window_logical_size.y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pillarbox_vertical_stripes_for_narrow_aspect_on_wide_window() {
        // Target: 4:3 (1024x768). Window: 16:9 (1920x1080). Scale: 1.0.
        let vp = ViewportBounds::compute(UVec2::new(1024, 768), UVec2::new(1920, 1080), 1.0);
        assert_eq!(vp.physical_size, UVec2::new(1440, 1080));
        assert_eq!(vp.physical_offset, UVec2::new(240, 0));
        assert_eq!(vp.logical_size, Vec2::new(1440.0, 1080.0));
        assert_eq!(vp.logical_offset, Vec2::new(240.0, 0.0));
        assert!(vp.has_black_stripes());
        assert!(vp.is_vertical_stripes());
        assert!(!vp.is_horizontal_stripes());
    }

    #[test]
    fn letterbox_horizontal_stripes_for_wide_aspect_on_narrow_window() {
        // Target: 16:9 (1280x720). Window: 4:3 (1024x768). Scale: 1.0.
        let vp = ViewportBounds::compute(UVec2::new(1280, 720), UVec2::new(1024, 768), 1.0);
        assert_eq!(vp.physical_size.x, 1024);
        assert_eq!(vp.physical_size.y, 576);
        assert_eq!(vp.physical_offset, UVec2::new(0, 96));
        assert_eq!(vp.logical_size, Vec2::new(1024.0, 576.0));
        assert_eq!(vp.logical_offset, Vec2::new(0.0, 96.0));
        assert!(vp.has_black_stripes());
        assert!(!vp.is_vertical_stripes());
        assert!(vp.is_horizontal_stripes());
    }

    #[test]
    fn exact_aspect_ratio_fills_entire_frame() {
        // Target: 16:9 (1920x1080). Window: 16:9 (1920x1080). Scale: 1.0.
        let vp = ViewportBounds::compute(UVec2::new(1920, 1080), UVec2::new(1920, 1080), 1.0);
        assert_eq!(vp.physical_size, UVec2::new(1920, 1080));
        assert_eq!(vp.physical_offset, UVec2::ZERO);
        assert_eq!(vp.logical_offset, Vec2::ZERO);
        assert!(!vp.has_black_stripes());
    }

    #[test]
    fn viewport_with_scale_factor_calculates_correct_logical_offset() {
        // Target: 4:3 (1024x768). Window: 16:9 Retina (3840x2160). Scale: 2.0.
        let vp = ViewportBounds::compute(UVec2::new(1024, 768), UVec2::new(3840, 2160), 2.0);
        assert_eq!(vp.physical_size, UVec2::new(2880, 2160));
        assert_eq!(vp.physical_offset, UVec2::new(480, 0));
        assert_eq!(vp.logical_size, Vec2::new(1440.0, 1080.0));
        assert_eq!(vp.logical_offset, Vec2::new(240.0, 0.0));
    }

    #[test]
    fn cursor_to_canvas_maps_inside_and_rejects_outside_pillarbox() {
        let vp = ViewportBounds::compute(UVec2::new(1024, 768), UVec2::new(1920, 1080), 1.0);

        // Top-left of the 4:3 area on screen (x=240, y=0) -> frame pixel (0, 0)
        let p = cursor_to_canvas(
            Some(Vec2::new(240.0, 0.0)),
            Some(&vp),
            Vec2::new(1920.0, 1080.0),
        );
        assert_eq!(p, Some(Vec2::new(0.0, 0.0)));

        // Centre of the 4:3 area (x=960, y=540) -> centre of the 1024x768 frame
        let p_center = cursor_to_canvas(
            Some(Vec2::new(960.0, 540.0)),
            Some(&vp),
            Vec2::new(1920.0, 1080.0),
        );
        assert_eq!(p_center, Some(Vec2::new(512.0, 384.0)));

        // Click in black stripe on the left (x=100 < 240) -> None
        let p_left_stripe = cursor_to_canvas(
            Some(Vec2::new(100.0, 500.0)),
            Some(&vp),
            Vec2::new(1920.0, 1080.0),
        );
        assert_eq!(p_left_stripe, None);

        // Click in black stripe on the right (x=1700 > 1680) -> None
        let p_right_stripe = cursor_to_canvas(
            Some(Vec2::new(1700.0, 500.0)),
            Some(&vp),
            Vec2::new(1920.0, 1080.0),
        );
        assert_eq!(p_right_stripe, None);
    }

    #[test]
    fn cursor_to_canvas_maps_inside_and_rejects_outside_letterbox() {
        let vp = ViewportBounds::compute(UVec2::new(1280, 720), UVec2::new(1024, 1024), 1.0);
        // Top letterbox stripe is (1024 - 576)/2 = 224

        // Top stripe (y=100 < 224) -> None
        let p_top = cursor_to_canvas(
            Some(Vec2::new(512.0, 100.0)),
            Some(&vp),
            Vec2::new(1024.0, 1024.0),
        );
        assert_eq!(p_top, None);

        // Bottom stripe (y=850 > 800) -> None
        let p_bottom = cursor_to_canvas(
            Some(Vec2::new(512.0, 850.0)),
            Some(&vp),
            Vec2::new(1024.0, 1024.0),
        );
        assert_eq!(p_bottom, None);

        // Inside active area (x=512, y=324) -> presented (512, 100) -> 1280x720 frame (640, 125)
        let p_inside = cursor_to_canvas(
            Some(Vec2::new(512.0, 324.0)),
            Some(&vp),
            Vec2::new(1024.0, 1024.0),
        );
        assert_eq!(p_inside, Some(Vec2::new(640.0, 125.0)));
    }

    #[test]
    fn cursor_maps_to_frame_pixels_on_high_dpi_full_screen() {
        // 1280x720 frame upscaled on a 3024x1964 Retina panel (1512x982 points): full width,
        // letterboxed. The frame's bottom-right corner maps to (1280, 720).
        let vp = ViewportBounds::compute(UVec2::new(1280, 720), UVec2::new(3024, 1964), 2.0);
        let corner = vp.logical_offset + vp.logical_size;
        let p = vp.cursor_to_canvas(corner).unwrap();
        assert!(
            (p - Vec2::new(1280.0, 720.0)).abs().max_element() < 1e-3,
            "{p}"
        );
    }

    #[test]
    fn resize_canvas_resizes_to_frame_resolution() {
        let canvas = Canvas::default();
        let vp = ViewportBounds::compute(UVec2::new(1024, 768), UVec2::new(1920, 1080), 1.0);
        resize_canvas(&canvas, Some(&vp), Vec2::new(1920.0, 1080.0));
        assert_eq!(canvas.width(), 1024.0);
        assert_eq!(canvas.height(), 768.0);

        // Without viewport, resizes to window size
        resize_canvas(&canvas, None, Vec2::new(800.0, 600.0));
        assert_eq!(canvas.width(), 800.0);
        assert_eq!(canvas.height(), 600.0);
    }

    #[test]
    fn windowed_size_keeps_fitting_resolutions_and_shrinks_others() {
        let monitor = Some(Vec2::new(1512.0, 982.0));
        assert_eq!(
            windowed_size(UVec2::new(1280, 720), monitor),
            Vec2::new(1280.0, 720.0)
        );
        assert_eq!(
            windowed_size(UVec2::new(1920, 1080), None),
            Vec2::new(1920.0, 1080.0)
        );
        let shrunk = windowed_size(UVec2::new(3024, 1964), monitor);
        assert_eq!(shrunk, Vec2::new(1512.0, 982.0));
        let wide = windowed_size(UVec2::new(2560, 1080), monitor);
        assert!(wide.x <= 1512.0 && wide.y <= 982.0);
        assert!((wide.x / wide.y - 2560.0 / 1080.0).abs() < 0.01, "{wide}");
    }
    #[test]
    fn pointer_to_canvas_handles_scale_factors_and_outer_inner_delta() {
        // Test scale factor 1.0 (standard DPI) at 1280x720 window and 1280x720 frame
        let vp_1x = ViewportBounds::compute(UVec2::new(1280, 720), UVec2::new(1280, 720), 1.0);
        let win_size_1x = Vec2::new(1280.0, 720.0);

        // Inner client coordinates directly: point at (100, 200) maps to canvas (100, 200)
        let inner_pos = Vec2::new(100.0, 200.0);
        let p1 = pointer_to_canvas(Some(inner_pos), Some(&vp_1x), win_size_1x, Vec2::ZERO);
        assert_eq!(p1, Some(Vec2::new(100.0, 200.0)));

        // Outer window coordinates with a macOS title-bar height delta of 28pt:
        // Outer point is at (100, 228), delta is (0, 28)
        let outer_pos = Vec2::new(100.0, 228.0);
        let title_bar_delta = Vec2::new(0.0, 28.0);
        let p1_outer =
            pointer_to_canvas(Some(outer_pos), Some(&vp_1x), win_size_1x, title_bar_delta);
        assert_eq!(p1_outer, Some(Vec2::new(100.0, 200.0)));
        assert_eq!(
            p1, p1_outer,
            "outer delta properly subtracted to match inner client position"
        );

        // Test scale factor 2.0 (Retina high-DPI): 2560x1440 physical pixels, 1280x720 logical points
        let vp_2x = ViewportBounds::compute(UVec2::new(1280, 720), UVec2::new(2560, 1440), 2.0);
        let win_size_2x = Vec2::new(1280.0, 720.0); // logical points

        let p2 = pointer_to_canvas(Some(inner_pos), Some(&vp_2x), win_size_2x, Vec2::ZERO);
        assert_eq!(p2, Some(Vec2::new(100.0, 200.0)));

        let p2_outer =
            pointer_to_canvas(Some(outer_pos), Some(&vp_2x), win_size_2x, title_bar_delta);
        assert_eq!(p2_outer, Some(Vec2::new(100.0, 200.0)));
        assert_eq!(p2, p2_outer);

        // Verify corner hit-testing on the active frame (0,0) and (1280, 720)
        let top_left = pointer_to_canvas(Some(Vec2::ZERO), Some(&vp_2x), win_size_2x, Vec2::ZERO);
        assert_eq!(top_left, Some(Vec2::new(0.0, 0.0)));

        let bottom_right = pointer_to_canvas(
            Some(Vec2::new(1280.0, 720.0)),
            Some(&vp_2x),
            win_size_2x,
            Vec2::ZERO,
        );
        assert_eq!(bottom_right, Some(Vec2::new(1280.0, 720.0)));

        // Outer coordinates at top-left with title bar: outer position is (0, 28), delta is (0, 28) -> (0, 0)
        let top_left_outer = pointer_to_canvas(
            Some(Vec2::new(0.0, 28.0)),
            Some(&vp_2x),
            win_size_2x,
            title_bar_delta,
        );
        assert_eq!(top_left_outer, Some(Vec2::new(0.0, 0.0)));

        // Windowed, borderless windowed, and fullscreen all use the exact same conversion
        // In fullscreen / borderless, title bar delta is ZERO (no window decorations)
        let fs_vp = ViewportBounds::compute(UVec2::new(1280, 720), UVec2::new(1920, 1080), 1.0);
        let fs_win_size = Vec2::new(1920.0, 1080.0);
        let fs_p = pointer_to_canvas(
            Some(Vec2::new(960.0, 540.0)),
            Some(&fs_vp),
            fs_win_size,
            Vec2::ZERO,
        );
        assert_eq!(fs_p, Some(Vec2::new(640.0, 360.0)));
    }
}
