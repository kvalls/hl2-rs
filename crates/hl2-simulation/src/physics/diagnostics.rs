//! Diagnostics of the player hull's contacts (test fixtures and reports).
use super::{groups_filter, pose, vector, Physics, PLAYER_GROUPS, SCALE};
use glam::{Quat, Vec3};
use rapier3d::prelude::Cuboid;

impl Physics {
    /// Diagnostics: colliders (entity id, or None for the world; dynamic body?) that
    /// overlap the player hull at `feet`, shrunk by half the collision epsilon like
    /// trace_hull's overlap test.
    pub fn hull_contacts(&self, feet: Vec3, maxs_z: f32) -> Vec<(Option<usize>, bool, f32, Vec3)> {
        let half = Vec3::new(16., 16., maxs_z * 0.5) - Vec3::splat(0.015625);
        let position = pose(feet + Vec3::Z * maxs_z * 0.5, Quat::IDENTITY);
        let mut found = Vec::new();
        self.query.intersections_with_shape(
            &self.bodies,
            &self.colliders,
            &position,
            &Cuboid::new(vector(half)),
            groups_filter(PLAYER_GROUPS),
            |handle| {
                let c = &self.colliders[handle];
                // Penetration depth (units) and normal from the hull into the collider.
                let (depth, normal) = rapier3d::parry::query::contact(
                    &position,
                    &Cuboid::new(vector(half)),
                    c.position(),
                    c.shape(),
                    0.,
                )
                .ok()
                .flatten()
                .map_or((0., Vec3::ZERO), |k| {
                    (
                        -k.dist / SCALE,
                        Vec3::new(k.normal1.x, k.normal1.y, k.normal1.z),
                    )
                });
                found.push((
                    (c.user_data as usize).checked_sub(1),
                    c.parent()
                        .and_then(|b| self.bodies.get(b))
                        .is_some_and(|b| b.is_dynamic()),
                    depth,
                    normal,
                ));
                true
            },
        );
        found
    }
}
