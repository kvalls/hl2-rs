//! Moves entity roots to their simulated poses and shows or hides them.
use super::{NpcWeaponDraw, SourceEntity, draw_hidden, entity_transform};
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
