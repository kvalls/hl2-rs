//! Bevy camera presentation for the already-simulated player view.

use bevy::prelude::*;

/// The camera that draws the viewmodel layer (its FOV follows the view's FOV offset).
#[derive(Component)]
pub struct ViewmodelCamera;

/// CBasePlayer::CalcView adds the punch angle (degrees; Source pitch is down-positive, ours
/// up-positive) to the player's view angles (radians). Aim and the viewmodel do not include it.
pub fn punched(yaw: f32, pitch: f32, punch: glam::Vec3) -> (f32, f32) {
    (yaw + punch.y.to_radians(), pitch - punch.x.to_radians())
}

/// The vertical field of view (radians) of the world camera.
///
/// A main-menu background level keeps the vertical view of 4:3 on frames up to 16:9 (cropping
/// the sides below 4:3 and expanding horizontally up to 16:9) and keeps the 16:9 horizontal
/// view on ultrawide frames above [`hl2_ui::tuning::widest_design_aspect`] (cutting the top and
/// bottom of the 16:9 frame, like the pre-blurred overlay). Gameplay uses
/// [`crate::video::vertical_fov_with`].
pub fn world_fov(
    view_degrees: f32,
    default_degrees: f32,
    aspect: f32,
    vertical: f32,
    background_level: bool,
) -> f32 {
    if background_level {
        background_fov(aspect, false)
    } else {
        crate::video::vertical_fov_with(view_degrees, default_degrees, aspect, vertical)
    }
}

/// Authored menu projection; forced normal artwork caps the horizontal span at 4:3.
pub fn background_fov(aspect: f32, force_default_blur: bool) -> f32 {
    let widest = if force_default_blur {
        4. / 3.
    } else {
        hl2_ui::tuning::widest_design_aspect()
    };
    let scale = (aspect / widest).max(1.);
    2. * ((75f32.to_radians() / 2.).tan() / ((4. / 3.) * scale)).atan()
}

/// CViewRender::SetUpView: the viewmodel's FOV is `viewmodel_fov - (default_fov - fov)`. At or
/// below zero the view has moved past the viewmodel, which is then not drawn: returns the
/// vertical FOV and near plane for the viewmodel camera given its far plane, aspect-adaptive
/// `fov_offset` (degrees), and the frame's aspect ratio.
pub fn viewmodel_projection(
    viewmodel_degrees: f32,
    fov_offset: f32,
    far: f32,
    aspect: f32,
) -> (f32, f32) {
    if viewmodel_degrees > 0. {
        let degrees = (viewmodel_degrees + fov_offset).clamp(1., 160.);
        (crate::video::vertical_fov_for(degrees, aspect), 0.1)
    } else {
        (crate::video::vertical_fov_for(1., aspect), far * 0.5)
    }
}

type ViewmodelCameraFilter = (With<ViewmodelCamera>, Without<crate::FlyCamera>);

/// The viewmodel camera in viewmodel space: the viewmodel sits at the eye plus its framing offset
/// and the sway (CalcViewModelView), so the camera is that pose's inverse.
fn viewmodel_camera(
    framing: [f32; 3],
    sway: hl2_simulation::viewmodel_sway::Sway,
) -> (Transform, glam::Vec3) {
    let back = sway.rotation.inverse();
    let at = back * -(glam::Vec3::from_array(framing) + sway.offset);
    let axis = |v: glam::Vec3| crate::source_to_bevy(Vec3::from_array(v.to_array()));
    let transform = Transform::from_translation(axis(at))
        .looking_to(axis(back * glam::Vec3::X), axis(back * glam::Vec3::Z));
    (transform, at)
}

/// Publishes the view after the fixed simulation loop: a point_viewcontrol camera or env_zoom
/// when active, otherwise the player's eye with the weapon punch and SetFOV zoom. Character
/// motion itself remains in `simulation`.
pub fn present_player_view(
    sim: Res<crate::simulation::Simulation>,
    game: Option<Res<crate::gameplay::Gameplay>>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<crate::FlyCamera>>,
    mut viewmodel: Query<(&mut Transform, &mut Projection), ViewmodelCameraFilter>,
    frame: Option<Res<crate::video::GameFrame>>,
    clock: Res<Time>,
    mut effects: Option<ResMut<crate::effects::Effects>>,
) {
    let game = game.as_deref();
    let aspect = crate::video::frame_aspect(frame.as_deref());
    // The player's own FOV, including SetFOV ramps (crossbow zoom).
    let player_fov = game.map_or(sim.fov, |game| game.inventory.view_fov(game.scene.time));
    if let (Some(game), Ok((mut transform, mut projection))) = (game, viewmodel.single_mut()) {
        let framing = sim.viewmodel_framing(&game.inventory.active, aspect);
        let degrees = hl2_simulation::viewmodel_effects::viewmodel_fov(
            sim.viewmodel_fov,
            game.inventory.fov.default_fov,
            player_fov,
        );
        if let Projection::Perspective(p) = &mut *projection {
            let (fov, near) = viewmodel_projection(degrees, framing.fov_offset, p.far, aspect);
            if p.fov != fov || p.near != near {
                p.fov = fov;
                p.near = near;
            }
        }
        let (yaw, pitch) = sim.view_angles();
        let input = hl2_simulation::viewmodel_sway::SwayInput {
            angles: glam::Vec3::new(-pitch.to_degrees(), yaw.to_degrees(), 0.),
            speed: glam::Vec2::new(sim.player.velocity.x, sim.player.velocity.y).length(),
            time: game.scene.time,
            frame_time: clock.delta_secs(),
        };
        let mut sway = effects
            .as_deref()
            .map(|e| e.viewmodel_sway)
            .unwrap_or_default();
        let (camera, at) = viewmodel_camera(framing.offset, sway.place(input));
        if let Some(effects) = effects.as_deref_mut() {
            (effects.viewmodel_sway, effects.viewmodel_camera) = (sway, at);
        }
        if *transform != camera {
            *transform = camera;
        }
    }
    if let Ok((mut camera, mut projection)) = cameras.single_mut() {
        let (yaw, pitch) = sim.view_angles();
        let (yaw, pitch) = match game.filter(|_| sim.player_view()) {
            Some(game) => punched(yaw, pitch, game.inventory.punch.angle),
            None => (yaw, pitch),
        };
        *camera = Transform::from_translation(crate::source_to_bevy(sim.view_origin())).looking_to(
            crate::source_to_bevy(crate::source_direction(yaw, pitch)),
            Vec3::Y,
        );
        let fov = sim.world_camera_fov(sim.view_fov(player_fov), aspect);
        if let Projection::Perspective(perspective) = &mut *projection {
            let active_fov = sim
                .extensions
                .iter()
                .find_map(|extension| extension.projection(&sim, aspect))
                .unwrap_or(fov);
            if perspective.fov != active_fov {
                perspective.fov = active_fov;
            }
            if perspective.aspect_ratio != aspect {
                perspective.aspect_ratio = aspect;
            }
        }
    }
}

/// Keeps [`crate::simulation::Simulation::camera_fov`] on the vertical fields of view the world
/// and viewmodel cameras use.
pub(crate) fn record_camera_fov(
    world: Query<&Projection, (With<crate::FlyCamera>, Without<ViewmodelCamera>)>,
    viewmodel: Query<&Projection, (With<ViewmodelCamera>, Without<crate::FlyCamera>)>,
    mut sim: ResMut<crate::simulation::Simulation>,
) {
    let degrees = |projection: &Projection| match projection {
        Projection::Perspective(p) => Some(p.fov.to_degrees()),
        _ => None,
    };
    let fov = world
        .single()
        .ok()
        .and_then(degrees)
        .zip(viewmodel.single().ok().and_then(degrees))
        .map(|(world, viewmodel)| [world, viewmodel]);
    if sim.camera_fov != fov {
        sim.camera_fov = fov;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn punch_tilts_the_view_in_source_conventions() {
        let (yaw, pitch) = punched(0.5, 0.1, glam::Vec3::new(2., 3., 0.));
        assert!((yaw - (0.5 + 3f32.to_radians())).abs() < 1e-6);
        assert!(
            (pitch - (0.1 - 2f32.to_radians())).abs() < 1e-6,
            "down-positive pitch"
        );
        assert_eq!(punched(0.5, 0.1, glam::Vec3::ZERO), (0.5, 0.1));
    }

    #[test]
    fn viewmodel_is_hidden_past_its_fov() {
        let (fov, near) = viewmodel_projection(54., 0., 1000., 4. / 3.);
        assert_eq!((fov, near), (crate::video::vertical_fov_4_3(54.), 0.1));
        let (_, near) = viewmodel_projection(0., 20., 1000., 9. / 16.);
        assert_eq!(
            near, 500.,
            "near plane beyond any viewmodel even when portrait fov_offset > 0"
        );
    }
    #[test]
    fn menu_backgrounds_crop_sides_when_narrow_and_top_and_bottom_when_ultrawide() {
        use crate::video::vertical_fov_for;
        let horizontal = |vertical: f32, aspect: f32| 2. * ((vertical / 2.).tan() * aspect).atan();
        let v_4_3 = vertical_fov_for(75., 4. / 3.);
        for aspect in [4. / 3., 16. / 10., 16. / 9.] {
            assert!(
                (world_fov(75., 75., aspect, 90., true) - v_4_3).abs() < 1e-6,
                "{aspect}: 4:3..=16:9 menu backgrounds keep the 4:3 vertical view and ignore the projection parameter"
            );
        }
        for aspect in [1., 9. / 16.] {
            let bg = world_fov(75., 75., aspect, 0., true);
            let game = world_fov(75., 75., aspect, 0., false);
            assert_eq!(
                bg, v_4_3,
                "{aspect}: a narrow menu background keeps the 4:3 vertical view (crops sides)"
            );
            assert!(
                game > bg,
                "{aspect}: gameplay shows more above and below on narrow frames"
            );
        }
        let h_16_9 = horizontal(world_fov(75., 75., 16. / 9., 0., true), 16. / 9.);
        for aspect in [3440. / 1440., 64. / 27., 32. / 9.] {
            let bg = world_fov(75., 75., aspect, 0., true);
            let game = world_fov(75., 75., aspect, 0., false);
            assert!(
                bg < v_4_3,
                "{aspect}: ultrawide menu background cuts top and bottom of the 16:9 frame"
            );
            assert!(
                (horizontal(bg, aspect) - h_16_9).abs() < 1e-5,
                "{aspect}: ultrawide menu background keeps the 16:9 horizontal view"
            );
            assert!(
                bg < game && game == v_4_3,
                "{aspect}: without an injected policy gameplay keeps the 4:3 vertical view"
            );
        }
    }

    #[test]
    fn narrow_frames_keep_the_four_by_three_horizontal_view() {
        use crate::video::{vertical_fov_4_3, vertical_fov_for};
        let landscape = vertical_fov_4_3(75.);
        for aspect in [4. / 3., 16. / 9., 16. / 10., 64. / 27., 32. / 9.] {
            assert_eq!(
                vertical_fov_for(75., aspect),
                landscape,
                "{aspect}: unchanged"
            );
        }
        let horizontal = |vertical: f32, aspect: f32| 2. * ((vertical / 2.).tan() * aspect).atan();
        for aspect in [1.25, 1., 3. / 4., 10. / 16., 9. / 16.] {
            let vertical = vertical_fov_for(75., aspect);
            assert!(vertical > landscape, "{aspect}: more above and below");
            assert!(
                (horizontal(vertical, aspect) - horizontal(landscape, 4. / 3.)).abs() < 1e-5,
                "{aspect}: the 4:3 horizontal view"
            );
        }
    }
}

#[cfg(test)]
#[path = "camera/background_tests.rs"]
mod background_tests;
