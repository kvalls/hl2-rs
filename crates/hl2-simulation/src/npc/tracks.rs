//! Locomotion root-motion tracks: the forward-track check and a scripted custom move's
//! track (SDK ai_basenpc_schedule.cpp StartScriptMoveToTargetTask: CINE_MOVETO_CUSTOM
//! moves with GetScriptCustomMoveActivity, ACT_SCRIPT_CUSTOM_MOVE when m_iszCustomMove
//! names a sequence, and GetScriptCustomMoveSequence plays that sequence).
use super::{GoalBlockReason, Locomotion, MoveStyle};
use glam::Vec3;
use modkit_core::animation::RootMotion;
use source_assets::models::MotionSequence;
use std::collections::BTreeMap;

/// A named looping sequence's central forward track (with the activity it must have,
/// when one is expected: the translated ACT_WALK/ACT_RUN a walk/run style plays).
pub(super) fn checked_track(
    sequences: &BTreeMap<String, MotionSequence>,
    name: &str,
    activity: Option<&str>,
) -> Result<RootMotion, GoalBlockReason> {
    let sequence = sequences
        .get(&name.to_lowercase())
        .ok_or_else(|| GoalBlockReason::MissingLocomotion(name.to_owned()))?;
    if activity.is_some_and(|a| !sequence.activity_name.eq_ignore_ascii_case(a))
        || sequence.flags & 1 == 0
    {
        return Err(GoalBlockReason::UnsupportedLocomotion(name.to_owned()));
    }
    let track = sequence
        .blends
        .get(sequence.central_blend)
        .ok_or_else(|| GoalBlockReason::UnsupportedLocomotion("central blend index".to_owned()))?;
    validate_forward_track(track)?;
    Ok(track.clone())
}

impl Locomotion {
    /// The custom move's sequence and its central forward track, read at actor setup
    /// (d1_trainstation_01's vort_sweepy: Walk_all_HoldBroom). A missing or unusable
    /// sequence is kept as the failure and the custom move walks.
    pub fn set_custom(&mut self, name: &str, sequences: &BTreeMap<String, MotionSequence>) {
        match checked_track(sequences, name, None) {
            Ok(track) => {
                self.names.insert(MoveStyle::Custom, name.to_lowercase()); // rig clips use lower case
                self.tracks.insert(MoveStyle::Custom, track);
            }
            Err(reason) => {
                self.custom_failure = Some(reason);
            }
        }
    }
    /// The style that moves: a custom move without its own track walks.
    pub(super) fn resolved(&self, style: MoveStyle) -> MoveStyle {
        if style == MoveStyle::Custom && !self.tracks.contains_key(&MoveStyle::Custom) {
            MoveStyle::Walk
        } else {
            style
        }
    }
}

pub(super) fn validate_forward_track(track: &RootMotion) -> Result<(), GoalBlockReason> {
    let error = || {
        GoalBlockReason::UnsupportedLocomotion(
            "only complete forward central tracks are supported".to_owned(),
        )
    };
    track
        .validate()
        .map_err(|e| GoalBlockReason::RootMotion(e.to_string()))?;
    if track
        .duration()
        .map_err(|e| GoalBlockReason::RootMotion(e.to_string()))?
        <= 0.
        || track
            .records
            .last()
            .is_none_or(|r| r.end_frame != track.frame_count as i32 - 1)
    {
        return Err(error());
    }
    let mut previous = 0.;
    for record in &track.records {
        let amount = (record.v0 + record.v1) * 0.5;
        // The classic zombie's walks drift sideways by ~1e-4 of their direction (0.01
        // units a cycle), the torso's crawl by 0.2 degrees (0.13 of 37.4 units) and the
        // burning walk (firewalk) by 0.67 degrees (0.41 of 34.7 units, 0.0024 short of
        // its mean speed along X); the motor moves
        // along the path at the ground speed, so such drift is ignored and the track
        // moves along +X.
        let drift = record.cumulative_position - Vec3::X * (previous + amount);
        if record.direction.distance(Vec3::X) > 0.015
            || record.end_yaw_degrees.abs() > 0.0001
            || record.v0 <= 0.
            || record.v1 <= 0.
            || drift.x.abs() > 0.001f32.max(1e-4 * amount)
            || drift.truncate().length() > 0.05f32.max(0.015 * amount)
            || drift.z.abs() > 0.001
        {
            return Err(error());
        }
        previous = record.cumulative_position.x;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use modkit_core::animation::MovementRecord;

    fn sequence(flags: u32, speed: f32) -> MotionSequence {
        MotionSequence {
            source_model: String::new(),
            sequence_id: 0,
            activity_name: String::new(),
            activity: 0,
            activity_weight: 0,
            flags,
            blend_dimensions: [1, 1],
            animation_indices: vec![0],
            blends: vec![RootMotion {
                fps: 10.,
                frame_count: 11,
                records: vec![MovementRecord {
                    end_frame: 10,
                    motion_flags: 1,
                    v0: speed,
                    v1: speed,
                    end_yaw_degrees: 0.,
                    direction: Vec3::X,
                    cumulative_position: Vec3::X * speed,
                }],
            }],
            central_blend: 0,
        }
    }

    #[test]
    fn a_custom_move_uses_its_sequence_or_walks() {
        let mut l = Locomotion::default();
        l.tracks
            .insert(MoveStyle::Walk, sequence(1, 80.).blends[0].clone());
        assert_eq!(l.resolved(MoveStyle::Custom), MoveStyle::Walk);
        assert_eq!(l.played_name(MoveStyle::Custom, false), "walk_all");
        let sequences = BTreeMap::from([
            ("walk_all_holdbroom".to_owned(), sequence(1, 40.)),
            ("broom_once".to_owned(), sequence(0, 40.)),
        ]);
        // Not looping: kept as the failure; the custom move walks.
        l.set_custom("broom_once", &sequences);
        assert_eq!(l.resolved(MoveStyle::Custom), MoveStyle::Walk);
        assert!(l.custom_failure.is_some());
        l.set_custom("Walk_all_HoldBroom", &sequences);
        assert_eq!(l.resolved(MoveStyle::Custom), MoveStyle::Custom);
        assert_eq!(
            l.played_name(MoveStyle::Custom, false),
            "walk_all_holdbroom"
        );
        assert!(l.required_clips().contains("walk_all_holdbroom"));
        assert_eq!(l.track(MoveStyle::Custom).unwrap().records[0].v0, 40.);
        // The walk and run are untouched.
        assert_eq!(l.resolved(MoveStyle::Walk), MoveStyle::Walk);
    }
}
