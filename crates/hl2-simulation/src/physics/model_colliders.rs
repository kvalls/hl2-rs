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
