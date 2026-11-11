//! Ray queries against the physics world: shots, impacts, the ground and picking.
use super::{
    groups_filter, vector, Collider, ColliderHandle, Physics, Point, RayHit, MASK_SHOT,
    PICK_GROUPS, QUERY_GROUPS, SCALE,
};
use glam::Vec3;
use rapier3d::prelude::{nalgebra, InteractionGroups, QueryFilter, Ray};

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
impl Physics {
    /// `filtered_ray` with a contents mask for world brushes (MASK_BLOCKLOS is 0x4041:
    /// world windows and grates do not block it).
    pub fn masked_ray(
        &self,
        start: Vec3,
        end: Vec3,
        mask: u32,
        keep: &dyn Fn(usize) -> bool,
    ) -> Option<RayHit> {
        let delta = end - start;
        let length = delta.length();
        if !length.is_finite() || length <= 1e-5 {
            return None;
        }
        let direction = delta / length;
        let predicate = |handle: ColliderHandle, collider: &Collider| {
            let entity = (collider.user_data as usize).wrapping_sub(1);
            collider.is_enabled()
                && keep(entity)
                && self
                    .world_brush_contents
                    .get(&handle)
                    .is_none_or(|contents| contents & mask != 0)
        };
        let (handle, hit) = self.query.cast_ray_and_get_normal(
            &self.bodies,
            &self.colliders,
            &Ray::new(
                Point::from(vector(start)),
                vector![direction.x, direction.y, direction.z],
            ),
            length * SCALE,
            true,
            QueryFilter::default()
                .groups(QUERY_GROUPS)
                .exclude_sensors()
                .predicate(&predicate),
        )?;
        Some(RayHit {
            entity: (self.colliders[handle].user_data as usize).wrapping_sub(1),
            position: start + direction * (hit.time_of_impact / SCALE),
            normal: Vec3::new(hit.normal.x, hit.normal.y, hit.normal.z),
            hitgroup: self.hitgroup(handle),
            bone: self.hitbox_bone(handle),
        })
    }
    /// MASK_SOLID_BRUSHONLY: where a trace from `start` to `end` meets the world's solid
    /// brushes, which alone stop it (a point_spotlight's beam).
    pub fn brush_ray(&self, start: Vec3, end: Vec3) -> Option<Vec3> {
        const MASK_SOLID_BRUSHONLY: u32 = 0x400b;
        let delta = end - start;
        let length = delta.length();
        if !length.is_finite() || length <= 1e-5 {
            return None;
        }
        let direction = delta / length;
        let predicate = |handle: ColliderHandle, _: &Collider| {
            self.world_brush_contents
                .get(&handle)
                .is_some_and(|contents| contents & MASK_SOLID_BRUSHONLY != 0)
        };
        let (_, toi) = self.query.cast_ray(
            &self.bodies,
            &self.colliders,
            &Ray::new(
                Point::from(vector(start)),
                nalgebra::vector![direction.x, direction.y, direction.z],
            ),
            length * SCALE,
            true,
            QueryFilter::default()
                .groups(QUERY_GROUPS)
                .exclude_sensors()
                .predicate(&predicate),
        )?;
        Some(start + direction * (toi / SCALE))
    }
    /// The `.phy` surfaceprop of the static prop a trace hit (`World::model_surfaceprops`):
    /// its hull can sit off its drawn mesh (a bench's curved back, a tree's trunk), so the
    /// drawn surface under the hit cannot name it. None for a hit on anything else.
    pub fn static_surfaceprop(&self, hit: &RayHit) -> Option<&str> {
        if hit.entity != usize::MAX {
            return None;
        }
        let point = Point::from(vector(hit.position - hit.normal * 0.25));
        let predicate = |handle: ColliderHandle, collider: &Collider| {
            collider.user_data == 0 && self.collider_materials.contains_key(&handle)
        };
        let (handle, projection) = self.query.project_point(
            &self.bodies,
            &self.colliders,
            &point,
            true,
            QueryFilter::default()
                .exclude_sensors()
                .predicate(&predicate),
        )?;
        let gap = (projection.point - point).norm() / SCALE;
        (projection.is_inside || gap < 0.5).then(|| self.collider_materials[&handle].as_str())
    }
}
