//! Projection regressions for authored menu artwork and follows_window internal frames.
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
        let crop = background_source(canvas, glam::Vec2::ONE, aspect > 4. / 3. + 1e-3);
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
        follows_window: true,
        pending_window: None,
        suspended: 0,
        recenter: false,
        policy: Default::default(),
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

#[test]
fn forced_normal_projection_matches_its_crop_and_disabled_math_is_unchanged() {
    let reference_tan = (crate::video::vertical_fov_for(75., 4. / 3.) * 0.5).tan();
    for aspect in TESTED_ASPECTS {
        let old_scale = (aspect / hl2_ui::tuning::widest_design_aspect()).max(1.);
        let old = 2. * ((75f32.to_radians() / 2.).tan() / ((4. / 3.) * old_scale)).atan();
        assert_eq!(super::background_fov(aspect, false), old);
        let crop = hl2_ui::gameui::loading::background_source(
            glam::vec2(aspect, 1.),
            glam::Vec2::ONE,
            false,
        );
        let tan_v = (super::background_fov(aspect, true) * 0.5).tan();
        assert!((tan_v - reference_tan * crop.h).abs() < 1e-6);
        assert!((tan_v * aspect - reference_tan * (4. / 3.) * crop.w).abs() < 1e-6);
    }
}

#[test]
fn config_toggle_updates_background_camera_and_preserves_gameplay() {
    let world = modkit_core::World::default();
    let mut sim =
        crate::simulation::Simulation::new(&world, glam::Vec3::ZERO, 0., 0., false, vec![]);
    let mut game = crate::gameplay::Gameplay::synthetic(world);
    let mut ui = crate::ui_host::Console::new(Default::default());
    let aspect = 16. / 9.;
    sim.background_level = true;
    for force in [true, false, true, false] {
        let effects = ui.source.submit(&format!(
            "background_force_default_blur {}",
            u8::from(force)
        ));
        sim.console_effects(&mut ui, &mut game, effects);
        assert_eq!(sim.background_force_default_blur, force);
        let expected = super::background_fov(aspect, force);
        assert_eq!(sim.world_camera_fov(110., aspect), expected);
        sim.background_level = false;
        assert_eq!(
            sim.world_camera_fov(90., aspect),
            world_fov(90., sim.fov, aspect, sim.projection_parameter, false)
        );
        sim.background_level = true;
    }
}
