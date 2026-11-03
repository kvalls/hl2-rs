//! Moves entity roots to their simulated poses and shows or hides them.
use super::{NpcWeaponDraw, SourceEntity, entity_transform};
use bevy::prelude::{Query, Transform, Visibility, Without};

/// The entity roots [`place_entities`] moves.
pub(super) type EntityPlacements<'w, 's> = Query<
    'w,
    's,
    (
        &'static SourceEntity,
        &'static mut Transform,
        &'static mut Visibility,
        Option<&'static NpcWeaponDraw>,
    ),
    Without<crate::gpu_skinning::Joint>,
>;

pub(super) fn place_entities(
    entities: &mut EntityPlacements,
    game: &crate::gameplay::Gameplay,
    sim: &crate::simulation::Simulation,
) {
    for (owner, mut transform, mut visibility, weapon) in entities {
        let Some(state) = game.scene.states.get(owner.0) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let (origin, rotation) = sim
            .physics
            .entity_pose(owner.0)
            .unwrap_or((state.origin, state.rotation));
        let pose = entity_transform(origin, rotation);
        if *transform != pose {
            *transform = pose;
        }
        let visible = if draw_hidden(game, owner.0, weapon) {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *visibility != visible {
            *visibility = visible;
        }
    }
}

/// Whether an entity's draw is hidden this frame: its entity is hidden or killed, or it
/// is an NPC weapon its owner does not hold now.
pub(crate) fn draw_hidden(
    game: &crate::gameplay::Gameplay,
    id: usize,
    weapon: Option<&NpcWeaponDraw>,
) -> bool {
    let Some(s) = game.scene.states.get(id) else {
        return true;
    };
    // A ragdolled NPC stays drawn (its bone-merged weapon does not).
    let ragdoll = game.scene.ragdoll_matrices.contains_key(&id);
    (!s.visible || s.killed) && !(ragdoll && weapon.is_none())
        || ragdoll && weapon.is_some()
        || weapon.is_some_and(|w| {
            game.ai
                .weapon_world_model(id)
                .is_none_or(|m| w.0.strip_suffix("#0") != Some(m.as_str()))
        })
}
