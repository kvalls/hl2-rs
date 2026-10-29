//! Sample main depth after drawing; avoid the separate DepthPrepass export/copy.
use super::WeaponBlur;
use bevy::{
    prelude::{Camera3d, Commands, Component, Entity, Or, Query, With},
    render::render_resource::TextureUsages,
};

#[derive(Component)]
pub(in crate::bloom) struct Sampling {
    added: u32,
}

type Cameras<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut Camera3d,
        Option<&'static WeaponBlur>,
        Option<&'static Sampling>,
    ),
    Or<(With<WeaponBlur>, With<Sampling>)>,
>;

pub(in crate::bloom) fn update(mut commands: Commands, mut cameras: Cameras) {
    for (entity, mut camera, blur, sampling) in &mut cameras {
        match (blur.is_some(), sampling) {
            (true, None) => {
                let added = TextureUsages::TEXTURE_BINDING.bits() & !camera.depth_texture_usages.0;
                if added != 0 {
                    camera.depth_texture_usages.0 |= added;
                }
                commands.entity(entity).insert(Sampling { added });
            }
            (false, Some(sampling)) => {
                if sampling.added != 0 {
                    camera.depth_texture_usages.0 &= !sampling.added;
                }
                commands.entity(entity).remove::<Sampling>();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::{App, Update};

    #[test]
    fn activation_and_disable_preserve_existing_camera_usage_bits() {
        for already_sampleable in [false, true] {
            let mut app = App::new();
            app.add_systems(Update, update);
            let mut camera = Camera3d::default();
            camera.depth_texture_usages.0 |= TextureUsages::COPY_SRC.bits();
            if already_sampleable {
                camera.depth_texture_usages.0 |= TextureUsages::TEXTURE_BINDING.bits();
            }
            let original = camera.depth_texture_usages.0;
            let entity = app
                .world_mut()
                .spawn((
                    camera,
                    WeaponBlur {
                        max: 0.12,
                        world: true,
                        camera_moving: false,
                    },
                ))
                .id();
            app.update();
            assert_eq!(
                app.world()
                    .get::<Camera3d>(entity)
                    .unwrap()
                    .depth_texture_usages
                    .0,
                original | TextureUsages::TEXTURE_BINDING.bits()
            );
            app.world_mut().entity_mut(entity).remove::<WeaponBlur>();
            app.update();
            assert_eq!(
                app.world()
                    .get::<Camera3d>(entity)
                    .unwrap()
                    .depth_texture_usages
                    .0,
                original
            );
            assert!(app.world().get::<Sampling>(entity).is_none());
        }
    }
}
