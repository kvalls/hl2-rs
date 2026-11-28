//! NPC trace colliders: the studio hitboxes of an NPC, posed from its animated skeleton every
//! tick (DESIGN 14b), or a hull-box NPC's model-shaped colliders when its model has none. Only
//! traces see them (CBaseAnimating::TestHitboxes; the NPC's hull box moves and touches): the
//! broad phase never holds them, so they form no contact pairs and cost nothing when they move.
use super::{angles, mat4_pose, npc_hull, npc_trace, vector, Physics, NPC_GROUP, SCALE};
use crate::npc_probe::CONTENTS_MONSTER;
use glam::{Mat4, Vec3};
use modkit_core::{animation::Rig, ModelInstance, World};
use rapier3d::prelude::{ColliderBuilder, ColliderHandle, ColliderSet, Group, InteractionGroups};

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
    /// An NPC whose model has studio hitboxes traces against them alone: its hull and hitboxes
    /// are added directly, without the render-mesh trimesh that `use_studio_hitboxes` would
    /// remove at once (building it cost a merged-vertex BVH per spawned NPC on the spawn tick).
    /// `false` for every other instance. A model without triangles or a scaled instance stays
    /// without colliders, as before.
    pub(super) fn add_studio_npc(
        &mut self,
        world: &World,
        instance: &ModelInstance,
        asset: &[modkit_core::Surface],
        query_start: Option<usize>,
    ) -> bool {
        let studio = npc_hull(&instance.kind).is_some()
            && world
                .rigs
                .get(&instance.asset_key())
                .is_some_and(|rig| !rig.hitboxes.is_empty());
        if !studio {
            return false;
        }
        if !asset.iter().any(|s| s.indices.len() >= 3) || (instance.scale - 1.).abs() > 0.001 {
            self.skipped += 1;
            return true;
        }
        self.add_npc_model_colliders(world, instance, None, query_start);
        true
    }
    /// Movement and contacts use the unchanged class hull, while traces use model
    /// geometry or studio hitboxes when the model supplies them.
    pub(super) fn add_npc_model_colliders(
        &mut self,
        world: &World,
        instance: &ModelInstance,
        surfaceprop: Option<&str>,
        query_start: Option<usize>,
    ) {
        let (Some(id), Some((mins, maxs))) = (instance.entity, npc_hull(&instance.kind)) else {
            return;
        };
        for c in self.entity_colliders.get(&id).cloned().unwrap_or_default() {
            self.trace_only(c);
        }
        let half = (maxs - mins) * 0.5;
        let hull = self.colliders.insert(
            ColliderBuilder::cuboid(half.x * SCALE, half.y * SCALE, half.z * SCALE)
                .translation(vector(instance.origin + (mins + maxs) * 0.5))
                .friction(0.7)
                .user_data(id as u128 + 1)
                .collision_groups(npc_trace::hull_groups(&instance.kind)),
        );
        self.npc_collider_contents.insert(hull, CONTENTS_MONSTER);
        if let Some(surfaceprop) = surfaceprop {
            self.collider_materials.insert(hull, surfaceprop.to_owned());
        }
        self.npc_hulls.insert(id, (hull, mins, maxs));
        if let Some(rig) = world
            .rigs
            .get(&instance.asset_key())
            .filter(|rig| !rig.hitboxes.is_empty())
        {
            if let Some(start) = query_start {
                // These pending model shapes are replaced before reaching the query tree.
                self.changed_entity_colliders.truncate(start);
            }
            self.use_studio_hitboxes(id, instance, rig, query_start.is_some());
        }
        if query_start.is_some() {
            self.changed_entity_colliders.push(hull);
        }
    }
    /// Replace an NPC's render-mesh trace collider with its studio hitboxes at the bind
    /// pose (the hull box stays for movement and contacts).
    fn use_studio_hitboxes(
        &mut self,
        id: usize,
        instance: &ModelInstance,
        rig: &Rig,
        publish_queries: bool,
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
            if publish_queries {
                self.changed_entity_colliders.push(c);
            }
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
    /// Pose every NPC with studio hitboxes in place, avoiding key/list allocations.
    pub fn pose_studio_hitboxes(
        &mut self,
        mut pose_for: impl FnMut(usize, &str) -> Option<(Mat4, bool, Vec<Mat4>)>,
    ) {
        for (id, set) in &self.studio_hitboxes {
            if let Some((frame, enabled, skin)) = pose_for(*id, &set.asset_key) {
                apply_npc_hitboxes(
                    &set.boxes,
                    &mut self.colliders,
                    &mut self.changed_entity_colliders,
                    frame,
                    &skin,
                    enabled,
                );
            }
        }
    }
    /// Pose an NPC's hitboxes from its skinning matrices (bone pose x inverse bind, model
    /// space) and entity frame; `enabled` follows the entity.
    pub fn set_npc_hitboxes(&mut self, id: usize, frame: Mat4, skin: &[Mat4], enabled: bool) {
        let Some(set) = self.studio_hitboxes.get(&id) else {
            return;
        };
        apply_npc_hitboxes(
            &set.boxes,
            &mut self.colliders,
            &mut self.changed_entity_colliders,
            frame,
            skin,
            enabled,
        );
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

/// Applies animated skinning poses and enabled state to an NPC's studio hitbox colliders.
fn apply_npc_hitboxes(
    boxes: &[(ColliderHandle, usize, Vec3, Mat4)],
    colliders: &mut ColliderSet,
    changed: &mut Vec<ColliderHandle>,
    frame: Mat4,
    skin: &[Mat4],
    enabled: bool,
) {
    for (c, bone, center, bind) in boxes {
        let Some(collider) = colliders.get(*c) else {
            continue;
        };
        let model = || skin.get(*bone).map_or(*bind, |m| *m * *bind);
        let at = enabled
            .then(|| mat4_pose(frame * model() * Mat4::from_translation(*center)))
            .filter(|at| collider.position() != at);
        let toggle = collider.is_enabled() != enabled;
        // A mutable borrow marks the collider modified for the next step: only for a change.
        if at.is_some() || toggle {
            let collider = colliders.get_mut(*c).expect("hitbox collider");
            at.into_iter().for_each(|at| collider.set_position(at));
            collider.set_enabled(enabled);
            changed.push(*c);
        }
    }
}
