//! Parenting: parent frames, attachments and children following their parents.

use super::{Parent, Scene};
use glam::Mat4;
use glam::Quat;
use glam::Vec3;
use modkit_core::World;

impl Scene {
    /// Skinning matrices for an actor: current base clip plus scene gesture layers.
    /// Both hosts use this so presentation, eyes and impacts see one pose.
    /// World frame an attached child follows: the parent's origin/rotation, or the
    /// parent's animated attachment (model-scaled). `None` for unknown attachments.
    fn parent_frame(&self, world: &World, parent: usize, attachment: Option<&str>) -> Option<Mat4> {
        let state = self.states.get(parent)?;
        match attachment.filter(|n| !n.is_empty()) {
            None => Some(Mat4::from_rotation_translation(
                state.rotation,
                state.origin,
            )),
            Some(name) => self.attachment_frames(world, parent, &[name])[0],
        }
    }

    /// World frames of named animated attachments (one pose evaluation for all names).
    pub fn attachment_frames(&self, world: &World, id: usize, names: &[&str]) -> Vec<Option<Mat4>> {
        let none = || vec![None; names.len()];
        let Some(state) = self.states.get(id) else {
            return none();
        };
        let Some(instance) = world.model_instances.iter().find(|i| i.entity == Some(id)) else {
            return none();
        };
        let Some(rig) = world.rigs.get(&instance.asset_key()) else {
            return none();
        };
        let found: Vec<_> = names.iter().map(|n| rig.attachment(n)).collect();
        if found.iter().all(Option::is_none) {
            return none();
        }
        let base = Mat4::from_rotation_translation(state.rotation, state.origin);
        let matrices = self.actor_matrices(rig, id);
        found
            .into_iter()
            .map(|a| {
                let local = rig.attachment_matrix(&matrices, a?)?;
                let (_, rotation, translation) = local.to_scale_rotation_translation();
                Some(
                    base * Mat4::from_rotation_translation(
                        rotation.normalize(),
                        translation * instance.scale,
                    ),
                )
            })
            .collect()
    }

    /// The world frame of an attachment on a bone-merged weapon model (`weapon_key` in
    /// `World::rigs`) carried by NPC `id`: CBoneMergeCache gives the weapon bone its
    /// owner's same-named bone (or keeps its bind offset from the nearest merged
    /// ancestor). `index` is 1-based (GetAttachment) when `name` is None.
    pub fn merged_attachment_frame(
        &self,
        world: &World,
        id: usize,
        weapon_key: &str,
        name: Option<&str>,
        index: usize,
    ) -> Option<Mat4> {
        let state = self.states.get(id)?;
        let instance = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(id))?;
        let rig = world.rigs.get(&instance.asset_key())?;
        let weapon = world.rigs.get(weapon_key)?;
        let attachment = match name {
            Some(n) => weapon.attachment(n)?,
            None => weapon.attachments.get(index.checked_sub(1)?)?,
        };
        let mut bone = Some(attachment.bone);
        let (merged, owner) = loop {
            let b = bone?;
            if let Some(j) = rig
                .bones
                .iter()
                .position(|o| o.name.eq_ignore_ascii_case(&weapon.bones[b].name))
            {
                break (b, j);
            }
            bone = weapon.bones[b].parent;
        };
        let skin = self.actor_matrices(rig, id);
        let owner_bone = *skin.get(owner)? * rig.bones[owner].inverse_bind.inverse();
        let model = owner_bone
            * weapon.bones[merged].inverse_bind
            * weapon.bones[attachment.bone].inverse_bind.inverse()
            * attachment.local;
        let (_, rotation, translation) = model.to_scale_rotation_translation();
        Some(
            Mat4::from_rotation_translation(state.rotation, state.origin)
                * Mat4::from_rotation_translation(
                    rotation.normalize(),
                    translation * instance.scale,
                ),
        )
    }

    /// The world frame of a bone-merged model on `id` (an NPC's weapon): its first bone
    /// that the owner also has (ValveBiped.Weapon_bone on HL2 weapons) sits on the owner's
    /// bone, like Weapon_Drop's Weapon_bone transform times the inverse root bone.
    pub fn merged_model_frame(&self, world: &World, id: usize, weapon_key: &str) -> Option<Mat4> {
        let state = self.states.get(id)?;
        let instance = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(id))?;
        let rig = world.rigs.get(&instance.asset_key())?;
        let weapon = world.rigs.get(weapon_key)?;
        let (merged, owner) = weapon.bones.iter().enumerate().find_map(|(b, bone)| {
            rig.bones
                .iter()
                .position(|o| o.name.eq_ignore_ascii_case(&bone.name))
                .map(|j| (b, j))
        })?;
        let skin = self.actor_matrices(rig, id);
        let owner_bone = *skin.get(owner)? * rig.bones[owner].inverse_bind.inverse();
        let model = owner_bone * weapon.bones[merged].inverse_bind;
        let (_, rotation, translation) = model.to_scale_rotation_translation();
        Some(
            Mat4::from_rotation_translation(state.rotation, state.origin)
                * Mat4::from_rotation_translation(
                    rotation.normalize(),
                    translation * instance.scale,
                ),
        )
    }

    /// SetParent/SetParentAttachment: keep the current world pose as a local offset, or
    /// snap onto the parent frame.
    pub(super) fn set_parent(
        &mut self,
        world: &World,
        id: usize,
        parent: usize,
        attachment: Option<String>,
        keep_offset: bool,
    ) {
        let Some(frame) = self.parent_frame(world, parent, attachment.as_deref()) else {
            self.unsupported_input(self.ent(world, id).class(), "SetParent:missing-attachment");
            return;
        };
        let (origin, rotation) = if keep_offset {
            let state = &self.states[id];
            let local =
                frame.inverse() * Mat4::from_rotation_translation(state.rotation, state.origin);
            let (_, rotation, origin) = local.to_scale_rotation_translation();
            (origin, rotation.normalize())
        } else {
            (Vec3::ZERO, Quat::IDENTITY)
        };
        self.states[id].parent = Some(Parent {
            entity: parent,
            attachment,
            origin,
            rotation,
        });
        self.follow_parent(world, id);
    }

    /// Move a parented child onto its parent frame and local offset.
    pub(super) fn follow_parent(&mut self, world: &World, id: usize) {
        let Some(parent) = self.states[id].parent.clone() else {
            return;
        };
        if self.states[parent.entity].killed {
            return;
        }
        if let Some(frame) = self.parent_frame(world, parent.entity, parent.attachment.as_deref()) {
            let pose = frame * Mat4::from_rotation_translation(parent.rotation, parent.origin);
            let (_, rotation, origin) = pose.to_scale_rotation_translation();
            self.states[id].origin = origin;
            self.states[id].rotation = rotation.normalize();
        }
    }

    /// SetupParentsForSpawnList: `parentname` ("name" or "name,attachment") keeps the
    /// spawn world pose. Parents still held by a point_template do not exist yet.
    pub(super) fn setup_spawn_parents(&mut self, world: &World) {
        for id in 0..world.entities.len() {
            let e = &world.entities[id];
            let Some(spec) = e.get("parentname").filter(|n| !n.is_empty()) else {
                continue;
            };
            if super::self_positioned(e.class())
                || self.template_pending(id)
                || self.makers.held(id)
            {
                continue;
            }
            let (name, attachment) = match spec.split_once(',') {
                Some((n, a)) => (n.trim(), Some(a.trim().to_owned())),
                None => (spec.trim(), None),
            };
            let Some(parent) = world.entities.iter().position(|p| {
                p.get("targetname")
                    .is_some_and(|t| t.eq_ignore_ascii_case(name))
            }) else {
                continue;
            };
            if parent != id && !self.template_pending(parent) && !self.makers.held(parent) {
                self.set_parent(world, id, parent, attachment, true);
            }
        }
    }
}
