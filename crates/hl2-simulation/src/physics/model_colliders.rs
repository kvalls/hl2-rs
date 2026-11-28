//! Shared contact parameters for map and runtime model colliders.
use modkit_core::{SurfaceMaterial, World};
use rapier3d::prelude::{CoefficientCombineRule, ColliderBuilder, SharedShape};

pub(super) fn prevent_prop_pickup(world: &World, id: usize) -> bool {
    world
        .entities
        .get(id)
        .and_then(|entity| entity.get("spawnflags"))
        .and_then(|value| value.trim().parse::<u32>().ok())
        .is_some_and(|flags| flags & 0x200 != 0)
}

/// A model instance's spawnflags, whether it is a physical pickup and whether it moves.
/// prop_physics spawnflags: 1 start asleep, 8 motion disabled (static until EnableMotion, which
/// is not implemented). CItem::Spawn and CBaseCombatWeapon::FallInit use bit 1 for a constrained
/// start.
pub(super) fn prop_motion(
    world: &World,
    instance: &modkit_core::ModelInstance,
) -> (u32, bool, bool) {
    let prop_flags = instance
        .entity
        .and_then(|id| world.entities.get(id))
        .and_then(|e| e.get("spawnflags"))
        .and_then(|f| f.parse::<u32>().ok())
        .unwrap_or(0);
    let pickup = source_assets::models::physical_pickup(&instance.kind);
    let dynamic = if pickup {
        prop_flags & 1 == 0
    } else {
        instance.kind.starts_with("prop_physics") && prop_flags & 8 == 0
    };
    (prop_flags, pickup, dynamic)
}

pub(super) fn prop_collider(
    shape: SharedShape,
    material: Option<&SurfaceMaterial>,
    entity: Option<usize>,
) -> ColliderBuilder {
    match material {
        // VPhysics multiplies the materials' coefficients. Rapier restitution
        // applies the adapter's elasticity correction before combining.
        Some(material) => ColliderBuilder::new(shape)
            .friction(material.friction)
            .restitution(super::restitution(material))
            .friction_combine_rule(CoefficientCombineRule::Multiply)
            .restitution_combine_rule(CoefficientCombineRule::Multiply),
        None => ColliderBuilder::new(shape).friction(0.7).restitution(0.05),
    }
    .user_data(entity.map_or(0, |id| id as u128 + 1))
}
