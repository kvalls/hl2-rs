//! NPC trace colliders: the studio hitboxes of an NPC, posed from its animated skeleton every
//! tick (DESIGN 14b), or a hull-box NPC's model-shaped colliders when its model has none. Only
//! traces see them (CBaseAnimating::TestHitboxes; the NPC's hull box moves and touches): the
//! broad phase never holds them, so they form no contact pairs and cost nothing when they move.
use super::{angles, mat4_pose, Physics, NPC_GROUP, SCALE};
use crate::npc_probe::CONTENTS_MONSTER;
use glam::{Mat4, Vec3};
use rapier3d::prelude::{ColliderBuilder, ColliderHandle, Group, InteractionGroups};

/// An NPC's studio hitboxes (hitbox set 0) as trace colliders.
#[derive(Clone, Debug)]
pub(super) struct StudioHitboxSet {
    /// (collider, bone, box center in bone space, bind-pose bone model matrix)
    pub(super) boxes: Vec<(ColliderHandle, usize, Vec3, Mat4)>,
    pub(super) asset_key: String,
}

impl Physics {
    /// Makes a collider inserted since the last step a trace collider: it never reaches the
    /// broad phase (see `RecordingBroadPhase::trace_only`).
    pub(super) fn trace_only(&mut self, collider: ColliderHandle) {
        self.broad.trace_only.insert(collider);
    }
    pub(super) fn is_trace_only(&self, collider: ColliderHandle) -> bool {
        self.broad.trace_only.contains(&collider)
    }
    /// Replace an NPC's render-mesh trace collider with its studio hitboxes at the bind
    /// pose (the hull box stays for movement and contacts).
    pub(super) fn use_studio_hitboxes(
        &mut self,
        id: usize,
        instance: &modkit_core::ModelInstance,
        rig: &modkit_core::animation::Rig,
    ) {
        for c in self.entity_colliders.remove(&id).unwrap_or_default() {
            self.colliders
                .remove(c, &mut self.islands, &mut self.bodies, false);
            self.broad.trace_only.remove(&c);
            self.npc_collider_contents.remove(&c);
            self.collider_materials.remove(&c);
        }
        let base = Mat4::from_rotation_translation(angles(instance.angles), instance.origin);
        let mut boxes = Vec::new();
        for hitbox in &rig.hitboxes {
            let Some(bone) = rig.bones.get(hitbox.bone) else {
                continue;
            };
            let half = ((hitbox.maxs - hitbox.mins) * 0.5).max(Vec3::splat(0.05));
            let center = (hitbox.maxs + hitbox.mins) * 0.5;
            let bind = bone.inverse_bind.inverse();
            let collider = ColliderBuilder::cuboid(half.x * SCALE, half.y * SCALE, half.z * SCALE)
                .position(mat4_pose(base * bind * Mat4::from_translation(center)))
                .user_data(id as u128 + 1)
                .collision_groups(InteractionGroups::new(NPC_GROUP, Group::ALL));
            let c = self.colliders.insert(collider);
            self.trace_only(c);
            self.npc_collider_contents.insert(c, CONTENTS_MONSTER);
            self.collider_hitgroups.insert(c, hitbox.group);
            self.collider_hitbox_bones.insert(c, hitbox.bone);
            boxes.push((c, hitbox.bone, center, bind));
        }
        self.studio_hitboxes.insert(
            id,
            StudioHitboxSet {
                boxes,
                asset_key: instance.asset_key(),
            },
        );
    }
    /// NPCs with studio hitbox colliders and their model asset keys.
    pub fn studio_hitbox_entities(&self) -> Vec<(usize, String)> {
        self.studio_hitboxes
            .iter()
            .map(|(id, set)| (*id, set.asset_key.clone()))
            .collect()
    }
    /// Pose an NPC's hitboxes from its skinning matrices (bone pose x inverse bind, model
    /// space) and entity frame; `enabled` follows the entity.
    pub fn set_npc_hitboxes(&mut self, id: usize, frame: Mat4, skin: &[Mat4], enabled: bool) {
        let Some(set) = self.studio_hitboxes.get(&id) else {
            return;
        };
        for (c, bone, center, bind) in &set.boxes {
            let Some(collider) = self.colliders.get(*c) else {
                continue;
            };
            let model = || skin.get(*bone).map_or(*bind, |m| *m * *bind);
            let at = enabled
                .then(|| mat4_pose(frame * model() * Mat4::from_translation(*center)))
                .filter(|at| collider.position() != at);
            let toggle = collider.is_enabled() != enabled;
            // A mutable borrow marks the collider modified for the next step: only for a change.
            if at.is_some() || toggle {
                let collider = self.colliders.get_mut(*c).expect("hitbox collider");
                at.into_iter().for_each(|at| collider.set_position(at));
                collider.set_enabled(enabled);
                self.changed_entity_colliders.push(*c);
            }
        }
    }
    /// Enables or disables an NPC's hitboxes with the entity, leaving their pose.
    pub(super) fn enable_hitboxes(&mut self, id: usize, enabled: bool) {
        let Some(set) = self.studio_hitboxes.get(&id) else {
            return;
        };
        for (c, ..) in &set.boxes {
            if (self.colliders.get(*c)).is_some_and(|c| c.is_enabled() != enabled) {
                let collider = self.colliders.get_mut(*c).expect("hitbox collider");
                collider.set_enabled(enabled);
                self.changed_entity_colliders.push(*c);
            }
        }
    }
}
