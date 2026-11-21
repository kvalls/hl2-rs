//! First-person viewmodel visibility presentation.

use bevy::prelude::{Component, Query, Res, Visibility};

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
