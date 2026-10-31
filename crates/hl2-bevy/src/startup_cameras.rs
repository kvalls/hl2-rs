//! Spawns the frame, viewmodel and world cameras of the startup map.
use super::{FlyCamera, Options, bloom, camera, source_direction, source_to_bevy, tonemap, video};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::{
    Camera, Camera2d, Camera3d, ClearColorConfig, Commands, PerspectiveProjection, Projection,
    Transform, Vec3, default,
};
use bevy::render::renderer::RenderAdapterInfo;

pub(super) fn spawn(
    commands: &mut Commands,
    adapter: Option<&RenderAdapterInfo>,
    options: &Options,
    position: Vec3,
    yaw: f32,
) {
    // Every game camera below renders into the game frame (see `video::present_frame`).
    let max_samples = video::adapter_max_msaa_samples(adapter);
    let msaa = video::msaa_for_hardware(options.config.video.advanced.antialiasing, max_samples);
    commands.spawn((
        Camera2d,
        Camera {
            order: 2,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        Tonemapping::None,
        msaa,
        bevy::camera::visibility::RenderLayers::layer(2),
    ));
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        bevy::camera::visibility::RenderLayers::layer(1),
        camera::ViewmodelCamera,
        Tonemapping::None,
        bevy::camera::Exposure::default(),
        tonemap::ToneMapped,
        // The viewmodel camera draws last in the 3D view, so bloom covers the whole scene.
        bloom::SourceBloom {
            amount: 1.,
            presample: None,
        },
        msaa,
        Projection::Perspective(PerspectiveProjection {
            fov: 2.
                * ((hl2_simulation::viewmodel_effects::VIEWMODEL_FOV.to_radians() / 2.).tan()
                    / (4. / 3.))
                    .atan(),
            near: 0.1,
            far: 1000.,
            ..default()
        }),
        Transform::IDENTITY.looking_to(Vec3::X, Vec3::Y),
    ));
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        msaa,
        Projection::Perspective(PerspectiveProjection {
            fov: 2. * ((75f32.to_radians() / 2.).tan() / (4. / 3.)).atan(),
            near: 1.,
            far: 32000.,
            ..default()
        }),
        Transform::from_translation(source_to_bevy(position)).looking_to(
            source_to_bevy(source_direction(yaw, options.pitch)),
            Vec3::Y,
        ),
        FlyCamera,
        bevy::camera::Exposure::default(),
        tonemap::ToneMapped,
        bevy::camera::visibility::RenderLayers::layer(0).with(5),
    ));
}
