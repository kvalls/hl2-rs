//! What lets the player's physics shadow push a body (CBasePlayer's shadow controller).

use super::{
    pose, vector, Physics, NPC_GROUP, PLAYER_PUSH_MASS_LIMIT, PLAYER_PUSH_SPEED_LIMIT,
    PROJECTILE_GROUP, QUERY_GROUPS, RAGDOLL_GROUP, SHADOW_GROUP,
};
use glam::{Quat, Vec3};
use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::prelude::{
    Collider, ColliderHandle, Cuboid, Group, InteractionGroups, QueryFilter, RigidBodyHandle,
};

/// Inputs for the player shadow's ahead-of-contact shape sweep.
pub(super) struct ShadowSweep {
    pub position: Vec3,
    pub skin: Vec3,
    pub skin_offset: Vec3,
    pub velocity: Vec3,
    pub dt: f32,
}

impl Physics {
    /// The player's sweeps for the bodies under the feet and in the way of the wish move
    /// see only dynamic bodies: the first hit decides, so a fixed collider the player
    /// stands inside (d1_trainstation_01's NPC clip box around the turnstile, which the
    /// player passes) hid the turnstile and the player never shoved it.
    pub(super) fn movable_filter() -> QueryFilter<'static> {
        QueryFilter::only_dynamic()
            .groups(QUERY_GROUPS)
            .exclude_sensors()
    }
    /// The body touches a fixed body (world, static prop) or one heavier than the push
    /// mass limit on its far side along `direction` (its contact normal points within
    /// ~45 degrees of it). The player shadow's own contacts do not count. A body held by
    /// a constraint (phys_hinge: d1_trainstation_01's turnstile, whose arm ends touch its
    /// cage) is never blocked: a push turns it about the joint instead of sliding it
    /// into what is behind it.
    pub(super) fn blocked_along(&self, body: RigidBodyHandle, direction: Vec3) -> bool {
        let Some(b) = self.bodies.get(body) else {
            return false;
        };
        if self.impulses.attached_joints(body).next().is_some() {
            return false;
        }
        b.colliders().iter().any(|collider| {
            self.narrow.contact_pairs_with(*collider).any(|pair| {
                if !pair.has_any_active_contact
                    || self.world_sides.shadow.contains(&pair.collider1)
                    || self.world_sides.shadow.contains(&pair.collider2)
                {
                    return false;
                }
                let (sign, other) = if pair.collider1 == *collider {
                    (1., pair.collider2)
                } else {
                    (-1., pair.collider1)
                };
                let immovable = self
                    .colliders
                    .get(other)
                    .and_then(|c| c.parent())
                    .and_then(|h| self.bodies.get(h))
                    .is_none_or(|o| !o.is_dynamic() || o.mass() > PLAYER_PUSH_MASS_LIMIT);
                immovable
                    && pair.manifolds.iter().any(|m| {
                        let n = m.data.normal * sign;
                        !m.data.solver_contacts.is_empty()
                            && Vec3::new(n.x, n.y, n.z).dot(direction) > 0.7
                    })
            })
        })
    }
    /// Applies push-speed limits ahead of contact so a fast-moving shadow does not
    /// penetrate a prop before solver contacts form.
    pub(super) fn clip_shadow_sweep_velocity(
        &self,
        sweep: ShadowSweep,
        excluded: Option<RigidBodyHandle>,
    ) -> Vec3 {
        if sweep.velocity.length_squared() <= 1e-12 {
            return sweep.velocity;
        }
        let shape = Cuboid::new(vector(sweep.skin));
        let skip = |c: ColliderHandle, collider: &Collider| {
            !self.world_sides.shadow.contains(&c)
                && collider.parent().is_some_and(|h| {
                    Some(h) != excluded && self.bodies.get(h).is_some_and(|b| b.is_dynamic())
                })
        };
        let filter = QueryFilter::default()
            .groups(InteractionGroups::new(
                SHADOW_GROUP,
                Group::ALL
                    .difference(PROJECTILE_GROUP)
                    .difference(NPC_GROUP)
                    .difference(SHADOW_GROUP)
                    .difference(RAGDOLL_GROUP),
            ))
            .exclude_sensors()
            .predicate(&skip);
        let delta = sweep.velocity * sweep.dt;
        let mut velocity = sweep.velocity;
        if let Some((collider, hit)) = self.query.cast_shape(
            (&self.bodies, &self.colliders),
            &pose(sweep.position + sweep.skin_offset, Quat::IDENTITY),
            &vector(delta),
            &shape,
            ShapeCastOptions::with_max_time_of_impact(1.),
            filter,
        ) {
            let n = Vec3::new(hit.normal1.x, hit.normal1.y, hit.normal1.z);
            let other = self.colliders.get(collider).and_then(|c| c.parent());
            let pushable = other
                .and_then(|b| self.bodies.get(b))
                .is_some_and(|b| b.mass() <= PLAYER_PUSH_MASS_LIMIT)
                && other.is_some_and(|b| !self.blocked_along(b, n));
            let limit = if pushable {
                PLAYER_PUSH_SPEED_LIMIT
            } else {
                0.
            };
            let into = velocity.dot(n);
            let allowed = (delta * hit.time_of_impact).dot(n) / sweep.dt + limit;
            if into > allowed {
                velocity -= n * (into - allowed.max(0.));
            }
        }
        velocity
    }
    /// Updates the player shadow body only when its target translation or velocity
    /// actually changed, avoiding waking a stationary sleeping shadow every tick.
    pub(super) fn sync_player_shadow_body(
        &mut self,
        handle: RigidBodyHandle,
        position: Vec3,
        velocity: Vec3,
    ) {
        let next_pos = vector(position);
        let next_vel = vector(velocity);
        let unchanged = self.bodies.get(handle).is_some_and(|body| {
            (body.translation() - next_pos).norm_squared() <= 1e-10
                && (body.linvel() - next_vel).norm_squared() <= 1e-10
                && next_vel.norm_squared() <= 1e-10
        });
        if !unchanged {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.set_translation(next_pos, true);
                body.set_linvel(next_vel, true);
            }
        }
    }
}
