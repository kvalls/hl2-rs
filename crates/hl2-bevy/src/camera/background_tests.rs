//! Projection regressions for authored menu artwork and flexible internal frames.
use super::world_fov;
use bevy::prelude::{Handle, UVec2, Window};

const TESTED_ASPECTS: [f32; 11] = [
    9. / 16.,
    1.,
    5. / 4.,
    4. / 3.,
    1.5,
    3024. / 1964.,
    3456. / 2234.,
    16. / 10.,
    16. / 9.,
    21. / 9.,
    32. / 9.,
];

#[test]
fn gameplay_settings_and_zoom_never_change_background_projection() {
    for aspect in TESTED_ASPECTS {
        let expected = world_fov(75., 75., aspect, 0., true);
        for view in [20., 75., 87.3083, 110.] {
            for vertical in [0., 60., 90.] {
                assert_eq!(world_fov(view, 110., aspect, vertical, true), expected);
            }
        }
        assert_ne!(
            world_fov(75., 75., aspect, 0., false),
            world_fov(90., 90., aspect, 0., false),
            "gameplay FOV remains adjustable"
        );
    }
}

#[test]
fn projection_rays_match_authored_artwork_crop() {
    use hl2_ui::gameui::loading::background_source;
    let reference_tan = (crate::video::vertical_fov_for(75., 4. / 3.) * 0.5).tan();
    for aspect in TESTED_ASPECTS {
        let canvas = glam::vec2(aspect * 900., 900.);
        let crop = background_source(canvas, glam::Vec2::ONE);
        let base_aspect = if aspect > 4. / 3. + 1e-3 {
            16. / 9.
        } else {
            4. / 3.
        };
        let tan_v = (world_fov(110., 110., aspect, 90., true) * 0.5).tan();
        // Visible image-space ray extents follow the same crop as the blurred artwork.
        assert!(
            (tan_v - reference_tan * crop.h).abs() < 1e-6,
            "vertical crop at {aspect}"
        );
        assert!(
            (tan_v * aspect - reference_tan * base_aspect * crop.w).abs() < 1e-6,
            "horizontal crop at {aspect}"
        );
    }
}

#[test]
fn fit_display_uses_internal_frame_aspect_through_resize() {
    let mut frame = crate::video::GameFrame {
        image: Handle::default(),
        resolution: UVec2::new(1024, 768),
        configured: UVec2::new(1024, 768),
        flexible: true,
        pending_window: None,
        suspended: 0,
        recenter: false,
    };
    for (w, h) in [(1920, 1080), (1080, 1920), (5120, 1440), (1600, 1200)] {
        let window = Window {
            resolution: bevy::window::WindowResolution::new(w, h),
            ..Default::default()
        };
        frame.resolution = frame.size_for(&window);
        let aspect = crate::video::frame_aspect(Some(&frame));
        let size = frame.resolution;
        assert_eq!(aspect, size.x as f32 / size.y as f32);
        assert_eq!(
            world_fov(20., 110., aspect, 100., true),
            world_fov(75., 75., aspect, 0., true)
        );
    }
}
