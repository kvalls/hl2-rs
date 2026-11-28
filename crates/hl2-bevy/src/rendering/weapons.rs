//! First-person viewmodel visibility presentation.

use bevy::{
    camera::{Camera, visibility::RenderLayers},
    ecs::query::Has,
    prelude::{Commands, Component, Entity, Query, Res, Visibility, With, Without},
};

/// The active viewmodel's pose sample: its clip, sampled time key and a hash of its pose
/// parameters (the gravity gun's "active" changes the pose too).
pub(crate) fn viewmodel_sample(
    game: &crate::gameplay::Gameplay,
    rig: &modkit_core::animation::Rig,
) -> (String, u32, u64) {
    let inventory = &game.inventory;
    let (clip, key) = inventory
        .viewmodel_animation_state(rig, game.scene.time)
        .cache_key(rig);
    let params = inventory.viewmodel_pose_values(rig);
    let hash = params.iter().fold(0u64, |hash, v| {
        (hash ^ u64::from(v.to_bits())).wrapping_mul(0x100000001b3)
    });
    (clip, key, hash)
}

/// Tag identifying which equipped weapon a first-person viewmodel draw belongs to.
#[derive(Component)]
pub struct WeaponMesh(pub String);

/// A viewmodel draw that belongs to one skin only (surfaces whose material differs
/// between skins 0 and 1).
#[derive(Component)]
pub struct WeaponSkin(pub(super) u8);

/// Show only the active, unholstered weapon's viewmodel draws for its current skin.
pub fn present_weapons(
    game: Res<crate::gameplay::Gameplay>,
    mut draws: Query<(&WeaponMesh, Option<&WeaponSkin>, &mut Visibility)>,
) {
    let skin = game.inventory.viewmodel_skin();
    for (weapon, variant, mut visibility) in &mut draws {
        // The +USE carry holsters the weapon (CPlayerPickupController::Init).
        let target = if weapon.0 == game.inventory.active
            && !game.inventory.carrying()
            && variant.is_none_or(|v| v.0 == skin)
        {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != target {
            *visibility = target;
        }
    }
}

type ViewmodelCameraQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut Camera,
        Option<&'static crate::bloom::SourceBloom>,
        Option<&'static crate::bloom::color_correction::SourceColorCorrection>,
        Has<crate::video::SuspendedCamera>,
    ),
    (
        With<crate::camera::ViewmodelCamera>,
        Without<crate::FlyCamera>,
    ),
>;

type WorldCameraQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        Option<&'static crate::bloom::SourceBloom>,
        Option<&'static crate::bloom::color_correction::SourceColorCorrection>,
    ),
    (
        With<crate::FlyCamera>,
        Without<crate::camera::ViewmodelCamera>,
    ),
>;

fn handoff<T: Component + Clone>(
    commands: &mut Commands,
    from: Entity,
    to: Entity,
    source: Option<&T>,
    target: Option<&T>,
) {
    if target.is_none()
        && let Some(component) = source
    {
        commands.entity(to).insert(component.clone());
        commands.entity(from).remove::<T>();
    }
}

type Layer1Effects<'w, 's> = Query<
    'w,
    's,
    (&'static RenderLayers, &'static Visibility),
    (With<crate::effects::QuadDraw>, Without<WeaponMesh>),
>;

/// Deactivates `ViewmodelCamera` while no viewmodel or layer-1 effect is visible, moving
/// `SourceBloom` and `SourceColorCorrection` to `FlyCamera` so post-processing stays on the
/// last active 3D camera.
pub fn sync_viewmodel_camera(
    mut commands: Commands,
    weapons: Query<&Visibility, With<WeaponMesh>>,
    layer1_effects: Layer1Effects,
    mut viewmodel: ViewmodelCameraQuery,
    world: WorldCameraQuery,
) {
    let Ok((vm_entity, mut vm_camera, vm_bloom, vm_cc, suspended)) = viewmodel.single_mut() else {
        return;
    };
    if suspended {
        return;
    }
    let layer1 = RenderLayers::layer(1);
    let needed = weapons.iter().any(|v| *v != Visibility::Hidden)
        || layer1_effects
            .iter()
            .any(|(layers, v)| *v != Visibility::Hidden && layers.intersects(&layer1));
    if vm_camera.is_active != needed {
        vm_camera.is_active = needed;
    }
    let Ok((world_entity, world_bloom, world_cc)) = world.single() else {
        return;
    };
    if needed {
        handoff(
            &mut commands,
            world_entity,
            vm_entity,
            world_bloom,
            vm_bloom,
        );
        handoff(&mut commands, world_entity, vm_entity, world_cc, vm_cc);
    } else {
        handoff(
            &mut commands,
            vm_entity,
            world_entity,
            vm_bloom,
            world_bloom,
        );
        handoff(&mut commands, vm_entity, world_entity, vm_cc, world_cc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{asset::uuid_handle, ecs::system::RunSystemOnce, image::Image, prelude::App};
    use hl2_simulation::color_correction::Mix;

    #[test]
    fn sync_viewmodel_camera_disables_idle_viewmodel_and_transfers_bloom_and_color_correction() {
        let mut app = App::new();
        let presample: bevy::prelude::Handle<Image> =
            uuid_handle!("8c3f7c4a-1e52-49b0-9a11-7d2f0e6b4a10");
        let cc = crate::bloom::color_correction::SourceColorCorrection {
            lookups: std::sync::Arc::from([]),
            mix: Mix {
                weights: [1., 0., 0., 0.],
                default: 0.,
            },
        };
        let vm = app
            .world_mut()
            .spawn((
                crate::camera::ViewmodelCamera,
                Camera::default(),
                crate::bloom::SourceBloom {
                    amount: 0.75,
                    presample: Some(presample.clone()),
                },
                cc.clone(),
            ))
            .id();
        let world = app
            .world_mut()
            .spawn((crate::FlyCamera, Camera::default()))
            .id();
        let weapon = app
            .world_mut()
            .spawn((WeaponMesh("weapon_crowbar".into()), Visibility::Hidden))
            .id();

        app.world_mut()
            .run_system_once(sync_viewmodel_camera)
            .unwrap();
        assert!(!app.world().get::<Camera>(vm).unwrap().is_active);
        assert!(app.world().get::<crate::bloom::SourceBloom>(vm).is_none());
        assert!(
            app.world()
                .get::<crate::bloom::color_correction::SourceColorCorrection>(vm)
                .is_none()
        );
        let transferred = app.world().get::<crate::bloom::SourceBloom>(world).unwrap();
        assert_eq!(transferred.amount, 0.75);
        assert_eq!(transferred.presample.as_ref(), Some(&presample));
        assert!(
            app.world()
                .get::<crate::bloom::color_correction::SourceColorCorrection>(world)
                .is_some()
        );

        *app.world_mut().get_mut::<Visibility>(weapon).unwrap() = Visibility::Inherited;
        app.world_mut()
            .run_system_once(sync_viewmodel_camera)
            .unwrap();
        assert!(app.world().get::<Camera>(vm).unwrap().is_active);
        assert!(app.world().get::<crate::bloom::SourceBloom>(vm).is_some());
        assert!(
            app.world()
                .get::<crate::bloom::SourceBloom>(world)
                .is_none()
        );
        assert!(
            app.world()
                .get::<crate::bloom::color_correction::SourceColorCorrection>(vm)
                .is_some()
        );

        // Layer-1 effect alone keeps ViewmodelCamera active even when weapons are hidden.
        *app.world_mut().get_mut::<Visibility>(weapon).unwrap() = Visibility::Hidden;
        let effect = app
            .world_mut()
            .spawn((
                crate::effects::QuadDraw,
                RenderLayers::layer(1),
                Visibility::Inherited,
            ))
            .id();
        app.world_mut()
            .run_system_once(sync_viewmodel_camera)
            .unwrap();
        assert!(app.world().get::<Camera>(vm).unwrap().is_active);

        // SuspendedCamera leaves is_active untouched during resize.
        *app.world_mut().get_mut::<Visibility>(effect).unwrap() = Visibility::Hidden;
        app.world_mut().get_mut::<Camera>(vm).unwrap().is_active = false;
        app.world_mut()
            .entity_mut(vm)
            .insert(crate::video::SuspendedCamera(true));
        app.world_mut()
            .run_system_once(sync_viewmodel_camera)
            .unwrap();
        assert!(!app.world().get::<Camera>(vm).unwrap().is_active);
    }
}
