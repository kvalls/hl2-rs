//! Image-space motion blur, host side. [`hl2_simulation::motion_blur`] turns how the world
//! camera moved since the previous frame into four blur values; this module feeds it each frame
//! and keeps [`SourceMotionBlur`] on the camera while the option is on (the pass itself runs
//! only for frames with a non-zero value). With the option off nothing is computed and no
//! component, texture or buffer exists.
use crate::{FlyCamera, ui_host::Console, video::GameFrame};
use bevy::{
    camera::Projection,
    ecs::{
        component::Component,
        entity::Entity,
        query::With,
        resource::Resource,
        system::{Commands, Query, Res, ResMut},
    },
    math::Vec4,
    render::extract_component::ExtractComponent,
    time::{Real, Time},
    transform::components::Transform,
};
use hl2_simulation::motion_blur::{MotionBlur, Settings, View};

/// The blur of this frame, on the world camera.
#[derive(Component, Clone, Copy, PartialEq, ExtractComponent)]
pub struct SourceMotionBlur {
    pub values: Vec4,
    /// The longest blur vector, as a fraction of the screen (`mat_motion_blur_percent_of_screen_max`).
    pub max: f32,
}

#[derive(Resource, Default)]
pub struct MotionBlurState {
    model: MotionBlur,
}

/// The Source view of the world camera: origin and angles back from the Bevy transform, the 4:3
/// horizontal field of view from the vertical one.
pub(super) fn view_of(transform: &Transform, projection: &Projection, aspect: f32) -> View {
    let forward = crate::bevy_to_source(transform.forward().as_vec3());
    let vertical = match projection {
        Projection::Perspective(p) => p.fov,
        _ => 90f32.to_radians(),
    };
    View {
        origin: glam::Vec3::from_array(crate::bevy_to_source(transform.translation).to_array()),
        pitch: -forward.z.clamp(-1., 1.).asin().to_degrees(),
        yaw: forward.y.atan2(forward.x).to_degrees(),
        fov: 2. * ((vertical / 2.).tan() * 4. / 3.).atan().to_degrees(),
        aspect,
    }
}

/// Updates the camera's [`SourceMotionBlur`] from the view of this frame.
pub fn update(
    mut commands: Commands,
    ui: Option<Res<Console>>,
    time: Res<Time<Real>>,
    frame: Option<Res<GameFrame>>,
    mut state: ResMut<MotionBlurState>,
    mut cameras: Query<
        (
            Entity,
            &Transform,
            &Projection,
            Option<&mut SourceMotionBlur>,
        ),
        With<FlyCamera>,
    >,
) {
    let enabled = ui.is_some_and(|ui| {
        let a = &ui.source.config.video.advanced;
        a.motion_blur_mode().retail_camera()
    });
    let Ok((camera, transform, projection, current)) = cameras.single_mut() else {
        return;
    };
    if !enabled {
        if current.is_some() {
            commands.entity(camera).remove::<SourceMotionBlur>();
        }
        return;
    }
    let settings = Settings::default();
    let aspect = crate::video::frame_aspect(frame.as_deref());
    let values = state.model.update(
        &view_of(transform, projection, aspect),
        time.elapsed_secs(),
        &settings,
    );
    let blur = SourceMotionBlur {
        values: Vec4::from_array(values),
        max: settings.percent_of_screen_max / 100.,
    };
    match current {
        Some(mut current) => {
            if *current != blur {
                *current = blur;
            }
        }
        None => {
            commands.entity(camera).insert(blur);
        }
    }
}

#[cfg(test)]
mod tests;
