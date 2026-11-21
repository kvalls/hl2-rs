//! First-person viewmodel visibility presentation.

use bevy::prelude::{Component, Query, Res, Visibility};

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
