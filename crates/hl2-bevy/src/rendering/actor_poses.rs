//! One composed skeleton per actor, rig and pose. The renderer and the eyes share it instead of
//! composing the same skeleton twice in a frame, or again in a later frame whose pose has not
//! changed (backlog O6).
use bevy::prelude::Resource;
use glam::Mat4;
use modkit_core::animation::Rig;
use std::{collections::HashMap, sync::Arc};

/// What decides an entity's pose: its clip, the sampled animation time and its gesture layers
/// (`Scene::pose_signature`, which also changes with every ragdoll frame). Equal stamps give
/// equal poses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoseStamp {
    pub clip: String,
    pub key: u32,
    pub layers: u64,
}

/// The stamp of entity `id`'s pose on `rig`; `None` when it is not posed: hidden or killed
/// without a ragdoll, or a clip the rig lacks.
pub fn entity_stamp(game: &crate::gameplay::Gameplay, rig: &Rig, id: usize) -> Option<PoseStamp> {
    let state = game.scene.states.get(id)?;
    let ragdoll = game.scene.ragdoll_matrices.contains_key(&id);
    if (!state.visible || state.killed) && !ragdoll {
        return None;
    }
    let clip = state.animation.as_str();
    // A ragdoll's pose comes from its solids (pose_signature changes per ragdoll frame), so a
    // prop_ragdoll without a clip still samples.
    let key = match rig.clips.get(clip) {
        Some(definition) => super::sample_key(definition, game.scene.animation_time(id)),
        None if ragdoll => 0,
        None => return None,
    };
    Some(PoseStamp {
        clip: clip.to_owned(),
        key,
        layers: game.scene.pose_signature(id),
    })
}

/// One actor's composed skeleton on one rig, with the stamp it was composed for.
struct HeldPose {
    rig: String,
    stamp: PoseStamp,
    matrices: Arc<Vec<Mat4>>,
}

/// The latest composed skeleton per actor and rig, cleared when the map changes.
#[derive(Resource, Default)]
pub struct ActorPoses {
    map: String,
    poses: HashMap<usize, Vec<HeldPose>>,
    /// Skeletons composed and reused since the start (report and tests).
    pub composed: u64,
    pub reused: u64,
}

impl ActorPoses {
    /// Entity `id`'s skeleton on the rig named `rig_key` for `stamp`, composed only when no
    /// skeleton with this stamp is held.
    pub fn pose(
        &mut self,
        game: &crate::gameplay::Gameplay,
        (rig_key, rig): (&str, &Rig),
        id: usize,
        stamp: &PoseStamp,
    ) -> Arc<Vec<Mat4>> {
        if self.map != game.world.name {
            self.poses.clear();
            self.map.clone_from(&game.world.name);
        }
        let entries = self.poses.entry(id).or_default();
        if let Some(held) = entries.iter().find(|held| held.rig == rig_key)
            && held.stamp == *stamp
        {
            self.reused += 1;
            return held.matrices.clone();
        }
        self.composed += 1;
        let matrices = Arc::new(game.scene.actor_matrices(rig, id));
        match entries.iter_mut().find(|held| held.rig == rig_key) {
            Some(held) => (held.stamp, held.matrices) = (stamp.clone(), matrices.clone()),
            None => entries.push(HeldPose {
                rig: rig_key.to_owned(),
                stamp: stamp.clone(),
                matrices: matrices.clone(),
            }),
        }
        matrices
    }
}

#[cfg(test)]
mod tests {
    use super::{ActorPoses, PoseStamp};
    use modkit_core::{World, animation::Rig};

    fn stamp(key: u32) -> PoseStamp {
        PoseStamp {
            clip: "idle".into(),
            key,
            layers: 0,
        }
    }

    #[test]
    fn a_pose_is_composed_once_per_stamp_rig_and_map() {
        let world = World {
            name: "d1_trainstation_02".into(),
            ..Default::default()
        };
        let mut game = crate::gameplay::Gameplay::synthetic(world);
        let rig = Rig::default();
        let mut poses = ActorPoses::default();
        let first = poses.pose(&game, ("citizen", &rig), 7, &stamp(1));
        let again = poses.pose(&game, ("citizen", &rig), 7, &stamp(1));
        assert!(std::sync::Arc::ptr_eq(&first, &again));
        assert_eq!((poses.composed, poses.reused), (1, 1));
        poses.pose(&game, ("citizen", &rig), 7, &stamp(2));
        poses.pose(&game, ("citizen_head", &rig), 7, &stamp(2));
        poses.pose(&game, ("citizen", &rig), 8, &stamp(2));
        assert_eq!((poses.composed, poses.reused), (4, 1));
        std::sync::Arc::make_mut(&mut game.world).name = "d1_trainstation_03".into();
        poses.pose(&game, ("citizen", &rig), 7, &stamp(2));
        assert_eq!(
            (poses.composed, poses.reused),
            (5, 1),
            "a map change clears"
        );
    }
}
