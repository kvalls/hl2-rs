//! Actor pose access and cache signatures.
use super::super::{SequenceFade, STUDIO_SNAP};
use super::{BTreeMap, Scene, World};
impl Scene {
    pub fn actor_flex_values(&self, id: usize) -> BTreeMap<String, f32> {
        let mut values = self.flex_controllers.get(&id).cloned().unwrap_or_default();
        for (name, add) in self.lipsync.visemes(id, self.time) {
            *values.entry(name).or_insert(0.) += add;
        }
        values
    }

    pub fn actor_matrices(&self, rig: &modkit_core::animation::Rig, id: usize) -> Vec<glam::Mat4> {
        if let Some(ragdoll) = self.ragdoll_matrices.get(&id) {
            return ragdoll.clone();
        }
        let Some(state) = self.states.get(id) else {
            return rig.matrices("", 0.);
        };
        let params = self.actor_pose_values(rig, id);
        let fades = self.sequence_fades(id);
        self.gestures
            .compose_blended(
                rig,
                id,
                &state.animation,
                self.animation_time(id),
                &params,
                &fades,
            )
            .0
    }
    /// Previous base sequences to blend now, newest first: (sequence, time, weight).
    pub fn sequence_fades(&self, id: usize) -> Vec<(&str, f32, f32)> {
        self.sequence_transitions
            .get(&id)
            .map_or_else(Vec::new, |t| {
                t.fades
                    .iter()
                    .rev()
                    .map(|f| {
                        (
                            f.sequence.as_str(),
                            f.time_at(self.time),
                            f.weight(self.time),
                        )
                    })
                    .filter(|(_, _, w)| *w > 0.)
                    .collect()
            })
    }
    /// C_BaseAnimating::MaintainSequenceTransitions after the tick's AI: a changed or
    /// restarted base sequence of an NPC pushes the previous one, which keeps playing and
    /// fades out (CSequenceTransitioner::CheckForSequenceChange/UpdateCurrent); a
    /// STUDIO_SNAP sequence clears the queue. Killed or ragdolled actors drop theirs.
    pub fn update_sequence_transitions(&mut self, world: &World) {
        let now = self.time;
        for id in 0..self.states.len().min(world.entities.len()) {
            let state = &self.states[id];
            if state.killed
                || self.ragdoll_matrices.contains_key(&id)
                || !self.ent(world, id).class().starts_with("npc_")
            {
                if !self.sequence_transitions.is_empty() {
                    self.sequence_transitions.remove(&id);
                }
                continue;
            }
            let time = self.animation_time(id);
            let sequence = &self.states[id].animation;
            let entry = self.sequence_transitions.entry(id).or_default();
            let changed = entry.seen.is_some()
                && (!entry.sequence.eq_ignore_ascii_case(sequence) || time + 1e-3 < entry.time);
            if changed {
                let rig = world
                    .model_instances
                    .iter()
                    .find(|i| i.entity == Some(id))
                    .and_then(|i| world.rigs.get(&i.asset_key()));
                let clip = |name: &str| rig.and_then(|r| r.clips.get(&name.to_lowercase()));
                match (clip(&entry.sequence), clip(sequence)) {
                    (_, Some(new)) if new.layer.flags & STUDIO_SNAP != 0 => entry.fades.clear(),
                    (Some(old), Some(new)) => {
                        let seen = entry.seen.unwrap_or(now);
                        entry.fades.push(SequenceFade {
                            sequence: entry.sequence.clone(),
                            time: entry.time + (now - seen).max(0.) as f32,
                            changed: now,
                            fade: old.layer.fade_out.min(new.layer.fade_in),
                        });
                    }
                    _ => {}
                }
            }
            entry.fades.retain(|f| f.weight(now) > 0.);
            if entry.sequence != *sequence {
                entry.sequence.clone_from(sequence);
            }
            entry.time = time;
            entry.seen = Some(now);
        }
    }

    /// Normalized pose parameters: model defaults plus head control (head_pitch/yaw/roll)
    /// and the server-side body/head flex controllers (CAI_BaseActor UpdateHeadControl adds
    /// head_rightleft/updown/tilt to the look correction; UpdateBodyControl drives
    /// body_yaw, spine_yaw and neck_trans from body_rightleft, chest_rightleft and
    /// head_forwardback).
    pub fn actor_pose_values(&self, rig: &modkit_core::animation::Rig, id: usize) -> Vec<f32> {
        let mut params = rig.default_pose_values();
        let flexes = self.flex_controllers.get(&id);
        let flex = |name: &str| flexes.and_then(|v| v.get(name)).copied();
        let head = self.heads.get(&id).map(|h| h.goal);
        for (param, goal, controller) in [
            ("head_pitch", head.map(|g| g.x), "head_updown"),
            ("head_yaw", head.map(|g| g.y), "head_rightleft"),
            ("head_roll", head.map(|g| g.z), "head_tilt"),
            ("body_yaw", None, "body_rightleft"),
            ("spine_yaw", None, "chest_rightleft"),
            ("neck_trans", None, "head_forwardback"),
            // MaintainLookTargets: gesture blend position from the gesture flexes.
            ("gesture_height", None, "gesture_updown"),
            ("gesture_width", None, "gesture_rightleft"),
        ] {
            let offset = flex(controller);
            if goal.is_none() && offset.is_none() {
                continue;
            }
            if let Some(i) = rig.pose_parameter(param) {
                let value = goal.unwrap_or(0.) + offset.unwrap_or(0.);
                params[i] = rig.pose_parameters[i].normalize(value);
            }
        }
        let move_yaw = self
            .states
            .get(id)
            .and_then(|s| s.movement_animation.as_ref());
        if let (Some(m), Some(i)) = (move_yaw, rig.pose_parameter("move_yaw")) {
            params[i] = rig.pose_parameters[i].normalize(m.move_yaw);
        }
        params
    }

    /// Changes whenever an actor's composed pose inputs change; zero for plain base clips.
    pub fn pose_signature(&self, id: usize) -> u64 {
        if self.ragdoll_matrices.contains_key(&id) {
            return self.ragdoll_frame | 1 << 63;
        }
        let mut hash = self.gestures.signature(id);
        let mix = |hash: u64, v: f32| {
            (hash ^ v.to_bits() as u64)
                .wrapping_mul(0x100000001b3)
                .rotate_left(9)
        };
        if let Some(head) = self.heads.get(&id) {
            for v in head.goal.to_array() {
                hash = mix(hash, v);
            }
        }
        for (_, time, weight) in self.sequence_fades(id) {
            hash = mix(mix(hash, time), weight);
        }
        if let Some(m) = self
            .states
            .get(id)
            .and_then(|s| s.movement_animation.as_ref())
        {
            hash = mix(hash, m.move_yaw);
        }
        if let Some(values) = self.flex_controllers.get(&id) {
            for name in [
                "head_updown",
                "head_rightleft",
                "head_tilt",
                "body_rightleft",
                "chest_rightleft",
                "head_forwardback",
                "gesture_updown",
                "gesture_rightleft",
            ] {
                if let Some(v) = values.get(name) {
                    hash = mix(hash, *v);
                }
            }
        }
        hash
    }
}
