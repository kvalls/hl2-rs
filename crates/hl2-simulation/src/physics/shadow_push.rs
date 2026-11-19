//! What lets the player's physics shadow push a body (CBasePlayer's shadow controller).

use super::{Physics, PLAYER_PUSH_MASS_LIMIT, QUERY_GROUPS};
use glam::Vec3;
use rapier3d::prelude::{QueryFilter, RigidBodyHandle};

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
}
