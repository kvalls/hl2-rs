use super::*;
use bevy::{camera::PerspectiveProjection, math::Vec3};

#[test]
fn the_view_of_a_camera_is_its_source_angles_and_fov() {
    let (yaw, pitch) = (35f32.to_radians(), 20f32.to_radians());
    let transform = Transform::from_translation(crate::source_to_bevy(Vec3::new(10., 20., 30.)))
        .looking_to(
            crate::source_to_bevy(crate::source_direction(yaw, pitch)),
            Vec3::Y,
        );
    let fov = 2. * ((90f32.to_radians() / 2.).tan() / (4. / 3.)).atan();
    let projection = Projection::Perspective(PerspectiveProjection {
        fov,
        ..Default::default()
    });
    let view = view_of(&transform, &projection, 16. / 9.);
    assert!((view.origin - glam::Vec3::new(10., 20., 30.)).length() < 1e-4);
    assert!((view.yaw - 35.).abs() < 1e-3, "{}", view.yaw);
    // Source pitch is positive looking down; the camera looks up by 20 degrees.
    assert!((view.pitch + 20.).abs() < 1e-3, "{}", view.pitch);
    assert!((view.fov - 90.).abs() < 1e-3, "{}", view.fov);
    assert_eq!(view.aspect, 16. / 9.);
}
