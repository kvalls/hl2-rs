//! Ray queries against the physics world: shots, impacts, the ground and picking.
use super::{
    groups_filter, vector, Collider, ColliderHandle, Physics, Point, RayHit, MASK_SHOT,
    PICK_GROUPS, QUERY_GROUPS, SCALE,
};
use glam::Vec3;
use rapier3d::prelude::{nalgebra, InteractionGroups, Ray};

impl Physics {
    pub fn ray(&self, origin: Vec3, direction: Vec3, distance: f32) -> Option<(usize, Vec3)> {
        self.impact_ray(origin, direction, distance)
            .map(|h| (h.entity, h.position))
    }
    /// Shot/use trace (MASK_SHOT, MASK_SOLID): player/NPC-only clip brushes do not stop
    /// it.
    pub fn impact_ray(&self, origin: Vec3, direction: Vec3, distance: f32) -> Option<RayHit> {
        self.ray_with(origin, direction, distance, QUERY_GROUPS, false)
    }
    /// `impact_ray` that skips the shooter's own colliders (an NPC's FireBullets
    /// ignores its owner: CTraceFilterSkipTwoEntities).
    pub fn shot_ray(
        &self,
        origin: Vec3,
        direction: Vec3,
        distance: f32,
        excluded: &[usize],
    ) -> Option<RayHit> {
        let predicate = |handle: ColliderHandle, collider: &Collider| {
            collider.is_enabled()
                && !self.world_sides.clip_brushes.contains(&handle)
                && !excluded.contains(&(collider.user_data as usize).wrapping_sub(1))
        };
        self.cast_shot(origin, direction, distance, &predicate)
    }
    /// One ray cast (QUERY_GROUPS) under `predicate`, as a RayHit with its hitgroup and bone.
    pub(super) fn cast_shot(
        &self,
        origin: Vec3,
        direction: Vec3,
        distance: f32,
        predicate: &dyn Fn(ColliderHandle, &Collider) -> bool,
    ) -> Option<RayHit> {
        let ray = Ray::new(
            Point::from(vector(origin)),
            vector![direction.x, direction.y, direction.z],
        );
        let (h, hit) = self.query.cast_ray_and_get_normal(
            &self.bodies,
            &self.colliders,
            &ray,
            distance * SCALE,
            true,
            groups_filter(QUERY_GROUPS).predicate(&predicate),
        )?;
        let id = self.colliders[h].user_data as usize;
        Some(RayHit {
            entity: id.wrapping_sub(1),
            position: origin + direction * (hit.time_of_impact / SCALE),
            normal: Vec3::new(hit.normal.x, hit.normal.y, hit.normal.z),
            hitgroup: self.hitgroup(h),
            bone: self.hitbox_bone(h),
        })
    }
    /// A bullet trace (FireBullets, MASK_SHOT): `shot_ray` that also passes brushes
    /// without MASK_SHOT contents (CONTENTS_GRATE fences).
    pub fn bullet_ray(
        &self,
        origin: Vec3,
        direction: Vec3,
        distance: f32,
        excluded: &[usize],
    ) -> Option<RayHit> {
        let predicate = |handle: ColliderHandle, collider: &Collider| {
            collider.is_enabled()
                && !self.world_sides.clip_brushes.contains(&handle)
                && !excluded.contains(&(collider.user_data as usize).wrapping_sub(1))
                && self
                    .npc_collider_contents
                    .get(&handle)
                    .is_none_or(|contents| contents & MASK_SHOT != 0)
        };
        self.cast_shot(origin, direction, distance, &predicate)
    }
    /// Ground trace under the player (MASK_PLAYERSOLID: clip brushes count).
    pub fn ground_ray(&self, origin: Vec3, direction: Vec3, distance: f32) -> Option<RayHit> {
        self.ray_with(origin, direction, distance, QUERY_GROUPS, true)
    }
    /// `impact_ray` with COLLISION_GROUP_NONE: also hits thrown frags.
    pub fn pick_ray(&self, origin: Vec3, direction: Vec3, distance: f32) -> Option<RayHit> {
        self.ray_with(origin, direction, distance, PICK_GROUPS, false)
    }
    /// `impact_hull` with COLLISION_GROUP_NONE: also hits thrown frags.
    pub fn pick_hull(
        &self,
        origin: Vec3,
        direction: Vec3,
        distance: f32,
        half: Vec3,
    ) -> Option<RayHit> {
        self.hull_with(origin, direction, distance, half, PICK_GROUPS, false)
    }
    pub(super) fn ray_with(
        &self,
        origin: Vec3,
        direction: Vec3,
        distance: f32,
        groups: InteractionGroups,
        clips: bool,
    ) -> Option<RayHit> {
        let predicate = |handle: ColliderHandle, collider: &Collider| {
            collider.is_enabled() && (clips || !self.world_sides.clip_brushes.contains(&handle))
        };
        let ray = Ray::new(
            Point::from(vector(origin)),
            vector![direction.x, direction.y, direction.z],
        );
        let (h, hit) = self.query.cast_ray_and_get_normal(
            &self.bodies,
            &self.colliders,
            &ray,
            distance * SCALE,
            true,
            groups_filter(groups).predicate(&predicate),
        )?;
        let id = self.colliders[h].user_data as usize;
        Some(RayHit {
            entity: id.wrapping_sub(1),
            position: origin + direction * (hit.time_of_impact / SCALE),
            normal: Vec3::new(hit.normal.x, hit.normal.y, hit.normal.z),
            hitgroup: self.hitgroup(h),
            bone: self.hitbox_bone(h),
        })
    }
}

impl super::Physics {
    /// The model key of an NPC whose traces use its studio hitboxes.
    pub fn studio_model(&self, id: usize) -> Option<&str> {
        let set = self.studio_hitboxes.get(&id)?;
        Some(set.asset_key.as_str())
    }
}
