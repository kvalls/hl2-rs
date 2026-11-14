//! `script_intro`'s two-view composite, host side (SDK viewrender.cpp
//! `CViewRender::ViewDrawScene_Intro`). While an intro is active, its camera entity's view (the
//! intro shader's "scene") renders into a frame-sized image, and [`IntroComposite`] on the world
//! camera has the intro pass blend it with the ordinary view ("gman"). Nothing exists while no
//! intro is active; the extra view exists only while the intro has a camera entity.
mod pass;

use crate::{FlyCamera, gameplay::Gameplay, source_to_bevy, video::GameFrame};
use bevy::{
    camera::{
        Camera, Camera3d, ClearColorConfig, PerspectiveProjection, Projection, RenderTarget,
        visibility::RenderLayers,
    },
    color::Color,
    core_pipeline::tonemapping::Tonemapping,
    ecs::{
        component::Component,
        entity::Entity,
        query::{With, Without},
        resource::Resource,
        system::{Commands, Query, Res, ResMut},
    },
    image::Image,
    math::{UVec2, Vec3, Vec4},
    prelude::{App, Assets, Handle, Msaa, Plugin, Transform, Update, default},
    render::{extract_component::ExtractComponent, render_resource::TextureFormat},
};

pub struct IntroViewPlugin;
impl Plugin for IntroViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<IntroView>()
            .add_systems(Update, update)
            .add_plugins(pass::IntroPassPlugin);
    }
}

/// The intro passes of this frame on the world camera: (mode, alpha) of up to two passes, and
/// the camera entity's view when there is one (black otherwise, as `m_bDrawPrimary` false).
#[derive(Component, Clone, ExtractComponent)]
pub struct IntroComposite {
    pub passes: Vec4,
    pub scene: Option<Handle<Image>>,
}

#[derive(Component)]
struct IntroCamera;

#[derive(Resource, Default)]
struct IntroView {
    image: Option<(Handle<Image>, UVec2)>,
}

type WorldCamera<'a> = (Entity, &'a Projection, Option<&'a IntroComposite>);
type IntroCameras<'a> = (Entity, &'a mut Transform, &'a mut Projection);

/// Keeps the composite and the camera entity's view in step with the intro of this frame.
fn update(
    mut commands: Commands,
    game: Option<Res<Gameplay>>,
    game_frame: Option<Res<GameFrame>>,
    mut state: ResMut<IntroView>,
    mut images: ResMut<Assets<Image>>,
    world: Query<WorldCamera, (With<FlyCamera>, Without<IntroCamera>)>,
    mut intro: Query<IntroCameras, With<IntroCamera>>,
) {
    let Ok((camera, world_projection, current)) = world.single() else {
        return;
    };
    let composed = game.as_deref().and_then(|game| {
        let frame = game.scene.intro.script.frame(game.scene.time)?;
        let pose = frame
            .camera
            .and_then(|id| game.scene.states.get(id).filter(|s| !s.killed));
        Some((frame, pose.map(|s| (s.origin, s.rotation))))
    });
    let despawn = |commands: &mut Commands, intro: &Query<IntroCameras, With<IntroCamera>>| {
        for (entity, ..) in intro {
            commands.entity(entity).despawn();
        }
    };
    let Some((frame, pose)) = composed else {
        if current.is_some() {
            commands.entity(camera).remove::<IntroComposite>();
        }
        despawn(&mut commands, &intro);
        state.image = None;
        return;
    };
    let pass = |i: usize| frame.passes.get(i).copied().unwrap_or((0, 0.));
    let passes = Vec4::new(pass(0).0 as f32, pass(0).1, pass(1).0 as f32, pass(1).1);
    let Some((origin, rotation)) = pose else {
        despawn(&mut commands, &intro);
        commands.entity(camera).insert(IntroComposite {
            passes,
            scene: None,
        });
        return;
    };
    let size = game_frame
        .as_deref()
        .map_or(UVec2::new(1280, 720), |f| f.resolution)
        .max(UVec2::ONE);
    let resized = state.image.as_ref().is_none_or(|(_, s)| *s != size);
    if resized {
        let image = Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8UnormSrgb, None);
        state.image = Some((images.add(image), size));
        despawn(&mut commands, &intro);
    }
    let image = state.image.as_ref().expect("intro image").0.clone();
    let at = |v: glam::Vec3| source_to_bevy(Vec3::from_array(v.to_array()));
    let transform = Transform::from_translation(at(origin))
        .looking_to(at(rotation * glam::Vec3::X), at(rotation * glam::Vec3::Z));
    // ViewDrawScene_Intro: the intro's FOV (4:3 horizontal) for the camera view, else the view's.
    let projection = match (frame.camera_fov, world_projection) {
        (Some(fov), Projection::Perspective(p)) => Projection::Perspective(PerspectiveProjection {
            fov: 2. * ((fov.to_radians() / 2.).tan() * 0.75).atan(),
            ..p.clone()
        }),
        _ => world_projection.clone(),
    };
    match intro.single_mut() {
        Ok((_, mut t, mut p)) if !resized => {
            *t = transform;
            *p = projection;
        }
        _ => {
            commands.spawn((
                (
                    IntroCamera,
                    crate::video::OwnTarget,
                    crate::campaign::MapOwned,
                ),
                Camera3d::default(),
                Camera {
                    order: -4,
                    clear_color: ClearColorConfig::Custom(Color::BLACK),
                    ..default()
                },
                RenderTarget::Image(image.clone().into()),
                RenderLayers::layer(0).with(5),
                (
                    Tonemapping::None,
                    bevy::camera::Exposure::default(),
                    crate::tonemap::ToneMapped,
                ),
                Msaa::Off,
                projection,
                transform,
            ));
        }
    }
    commands.entity(camera).insert(IntroComposite {
        passes,
        scene: Some(image),
    });
}
