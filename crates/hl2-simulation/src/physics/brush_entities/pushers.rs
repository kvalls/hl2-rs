//! Whether a pusher's new pose overlaps a hull (SDK physics_main.cpp
//! CPushBlockerEnum::IntersectsPushers: a zero-length sweep of the entity's collideable against
//! the pushers alone, with the pushed entity's solid mask). A player the moved pusher overlaps
//! is pushed with it even off the ground: d1_trainstation_01's car carries a jumping player
//! instead of passing through him.
use super::super::{Physics, PLAYER_BRUSH_MASK, SCALE};
use glam::Vec3;
use modkit_core::trace_brushes;

impl Physics {
    /// Whether the player hull `mins`..`maxs` about `origin` starts inside a collider of
    /// `entity` at its current pose: its solid brushes and its player clips (the train car's
    /// seat blocks), not its NPC-only clips.
    pub fn hull_overlaps_entity(
        &self,
        origin: Vec3,
        mins: Vec3,
        maxs: Vec3,
        entity: usize,
    ) -> bool {
        let Some(handles) = self.entity_colliders.get(&entity) else {
            return false;
        };
        let (low, high) = (origin + mins - Vec3::ONE, origin + maxs + Vec3::ONE);
        let mut cache = self
            .player_convex_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        handles.iter().any(|&handle| {
            let Some(collider) = self.colliders.get(handle) else {
                return false;
            };
            let player_solid = self
                .world_brush_contents
                .get(&handle)
                .is_none_or(|contents| contents & PLAYER_BRUSH_MASK != 0);
            if !player_solid || !collider.is_enabled() || collider.is_sensor() {
                return false;
            }
            let aabb = collider.compute_aabb();
            let (lo, hi) = (aabb.mins.coords / SCALE, aabb.maxs.coords / SCALE);
            if lo.x > high.x || lo.y > high.y || lo.z > high.z {
                return false;
            }
            if hi.x < low.x || hi.y < low.y || hi.z < low.z {
                return false;
            }
            cache.brush(handle, collider).is_some_and(|brush| {
                let hit = trace_brushes(std::slice::from_ref(brush), origin, origin, mins, maxs, 1);
                hit.start_solid
            })
        })
    }
}
