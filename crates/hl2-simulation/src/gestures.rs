//! Scene GESTURE layers following SDK HandleStartGestureSceneEvent/ProcessGestureSceneEvent.
//! Layer cycles come from scene time through Faceposer tag retiming; weights from event
//! intensity. Ended layers fade like RemoveLayer (0.1 s, 0.5 s when canceled).
//! Native layer priorities across scenes, IK and pose-parameter layers are unfinished.
use glam::Mat4;
use modkit_core::animation::{Clip, PoseError, Rig};
use serde::Serialize;
use source_assets::scenes::{find_tag, gesture_original_percentage, ChoreoScene, Tag};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, Serialize)]
pub struct GestureLayer {
    pub scene: usize,
    pub event: usize,
    pub sequence: String,
    /// Event playback fraction: (scene time - start) / duration.
    pub playback: f32,
    /// Event intensity at the scene time; fades after the event ends.
    pub weight: f32,
    /// Posture suppression weight (CSceneEventInfo::m_flWeight); gestures use one.
    pub posture_weight: f32,
    /// Channel index within the actor, the SDK layer priority.
    pub priority: usize,
    pub order: u64,
    /// RemoveLayer kill rate in weight per second, once ended or canceled.
    pub removal: Option<f32>,
    #[serde(skip)]
    data: Arc<ChoreoScene>,
}
impl GestureLayer {
    fn tags(&self) -> &[Vec<Tag>; 2] {
        &self.data.events[self.event].absolute_tags
    }
    /// Layer cycle for this model: ORIGINAL tags are repositioned from the sequence's
    /// Faceposer frames (SDK numframes - 2 divisor) when they differ by more than 0.05,
    /// and the loop tags are linear.
    pub fn cycle(&self, clip: &Clip) -> f32 {
        let [playback, original] = self.tags();
        let mut original = original.clone();
        let mut linear = vec![false; playback.len()];
        if let Some(faceposer) = &clip.layer.faceposer {
            for name in [&faceposer.start_loop, &faceposer.end_loop] {
                if let Some(i) = find_tag(playback, name) {
                    linear[i] = true;
                }
            }
            let max_frame = clip.frames.len() as i32 - 2;
            if max_frame > 0 {
                for (name, frame) in &faceposer.tags {
                    let percentage = *frame as f32 / max_frame as f32;
                    if let Some(i) = find_tag(&original, name) {
                        if (original[i].value - percentage).abs() > 0.05 {
                            original[i].value = percentage;
                        }
                    }
                }
            }
        }
        gesture_original_percentage(playback, &original, &linear, self.playback)
    }
    /// ProcessGestureSceneEvent weight: intensity times spline of the posture weight.
    pub fn layer_weight(&self, clip: &Clip) -> f32 {
        let w = if clip
            .layer
            .faceposer
            .as_ref()
            .is_some_and(|f| f.is_gesture())
        {
            1.
        } else {
            self.posture_weight
        };
        self.weight * (3. * w * w - 2. * w * w * w)
    }
}
#[derive(Default)]
pub struct GestureLayers {
    actors: BTreeMap<usize, Vec<GestureLayer>>,
    order: u64,
}
pub struct GestureUpdate<'a> {
    pub actor: usize,
    pub scene: usize,
    pub event: usize,
    pub data: &'a Arc<ChoreoScene>,
    pub playback: f32,
    pub weight: f32,
    pub priority: usize,
    pub moving: bool,
}
impl GestureLayers {
    /// Start or process one active gesture event for one fixed tick.
    pub fn update(&mut self, update: GestureUpdate) {
        let layers = self.actors.entry(update.actor).or_default();
        let sequence = &update.data.events[update.event].parameters[0];
        if let Some(layer) = layers
            .iter_mut()
            .find(|l| l.scene == update.scene && l.event == update.event && l.removal.is_none())
        {
            layer.playback = update.playback;
            layer.weight = update.weight;
            // Postures fade by 0.2 per scene think while moving; the fixed step is one think.
            layer.posture_weight = if update.moving {
                (layer.posture_weight - 0.2).max(0.)
            } else {
                (layer.posture_weight + 0.2).min(1.)
            };
            return;
        }
        self.order += 1;
        layers.push(GestureLayer {
            scene: update.scene,
            event: update.event,
            sequence: sequence.to_lowercase(),
            playback: update.playback,
            weight: update.weight,
            posture_weight: if update.moving { 0. } else { 1. },
            priority: update.priority,
            order: self.order,
            removal: None,
            data: update.data.clone(),
        });
    }
    /// Event ended (0.1 s) or scene canceled (0.5 s): RemoveLayer kill rates.
    pub fn release(&mut self, scene: usize, event: Option<usize>, canceled: bool) {
        let seconds = if canceled { 0.5 } else { 0.1 };
        for layer in self.actors.values_mut().flatten() {
            if layer.scene == scene
                && event.is_none_or(|e| e == layer.event)
                && layer.removal.is_none()
            {
                layer.removal = Some(layer.weight / seconds);
            }
        }
    }
    pub fn active_events(&self, scene: usize) -> Vec<usize> {
        self.actors
            .values()
            .flatten()
            .filter(|l| l.scene == scene && l.removal.is_none())
            .map(|l| l.event)
            .collect()
    }
    /// Fade released layers and drop finished or dead-actor layers.
    pub fn advance(&mut self, dt: f32, alive: impl Fn(usize) -> bool) {
        self.actors.retain(|actor, layers| {
            layers.retain_mut(|layer| match layer.removal {
                Some(rate) => {
                    layer.weight -= rate * dt;
                    layer.weight > 0.
                }
                None => true,
            });
            alive(*actor) && !layers.is_empty()
        });
    }
    pub fn layers(&self, actor: usize) -> &[GestureLayer] {
        self.actors.get(&actor).map_or(&[], Vec::as_slice)
    }
    pub fn report(&self) -> &BTreeMap<usize, Vec<GestureLayer>> {
        &self.actors
    }
    /// Base clip, gesture layers in priority/start order (client AccumulateLayers), then
    /// autoplay sequences (CalcAutoplaySequences). Unloaded or unsupported layers are
    /// skipped and counted in the returned errors.
    pub fn compose(
        &self,
        rig: &Rig,
        actor: usize,
        base: &str,
        time: f32,
        params: &[f32],
    ) -> (Vec<Mat4>, Vec<PoseError>) {
        self.compose_blended(rig, actor, base, time, params, &[])
    }
    /// `compose` with previous base sequences (sequence, time in seconds, weight) blended
    /// over the base before the layers (C_BaseAnimating::MaintainSequenceTransitions).
    pub fn compose_blended(
        &self,
        rig: &Rig,
        actor: usize,
        base: &str,
        time: f32,
        params: &[f32],
        fades: &[(&str, f32, f32)],
    ) -> (Vec<Mat4>, Vec<PoseError>) {
        let layers = self.layers(actor);
        if layers.is_empty() && rig.autoplay.is_empty() && fades.is_empty() {
            return (rig.matrices(base, time), Vec::new());
        }
        let mut pose = rig.local_pose(base, time);
        let mut errors = Vec::new();
        for (sequence, at, weight) in fades {
            let Some(clip) = rig.clips.get(&sequence.to_lowercase()) else {
                errors.push(PoseError::MissingSequence((*sequence).to_owned()));
                continue;
            };
            // Cycle at the sequence's own rate (local_pose: fps over the last frame).
            let frames = clip.frames.len().saturating_sub(1).max(1) as f32;
            let cycle = at.max(0.) * clip.fps / frames;
            let mut trial = pose.clone();
            match rig.accumulate_pose_with(
                &mut trial,
                &sequence.to_lowercase(),
                cycle,
                *weight,
                params,
            ) {
                Ok(()) => pose = trial,
                Err(e) => errors.push(e),
            }
        }
        let mut ordered = layers.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|l| (l.priority, l.order));
        for layer in ordered {
            let Some(clip) = rig.clips.get(&layer.sequence) else {
                errors.push(PoseError::MissingSequence(layer.sequence.clone()));
                continue;
            };
            let mut trial = pose.clone();
            match rig.accumulate_pose_with(
                &mut trial,
                &layer.sequence,
                layer.cycle(clip),
                layer.layer_weight(clip),
                params,
            ) {
                Ok(()) => pose = trial,
                Err(e) => errors.push(e),
            }
        }
        let mut trial = pose.clone();
        match rig.accumulate_autoplay(&mut trial, time, params) {
            Ok(()) => pose = trial,
            Err(e) => errors.push(e),
        }
        (rig.local_matrices(&pose), errors)
    }
    /// Changes whenever any layer's cycle/weight changes; zero without layers.
    pub fn signature(&self, actor: usize) -> u64 {
        let mut hash = 0u64;
        for layer in self.layers(actor) {
            for value in [
                layer.event as u64,
                layer.playback.to_bits() as u64,
                layer.weight.to_bits() as u64,
                layer.posture_weight.to_bits() as u64,
            ] {
                hash = (hash ^ value).wrapping_mul(0x100000001b3).rotate_left(7);
            }
        }
        hash
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};
    use modkit_core::animation::{sequence_flags, Bone, ClipLayer, Faceposer, Pose};
    use source_assets::scenes::{Event, EventType};
    fn tag(name: &str, value: f32) -> Tag {
        Tag {
            name: name.into(),
            value,
        }
    }
    fn scene() -> Arc<ChoreoScene> {
        Arc::new(ChoreoScene {
            text_crc32: 0,
            actors: Vec::new(),
            events: vec![Event {
                kind: EventType::Gesture,
                name: "g".into(),
                start: 0.,
                end: Some(2.),
                parameters: ["G_Wave".into(), String::new(), String::new()],
                ramp: Vec::new(),
                flags: 8,
                distance: 0.,
                relative_tags: Vec::new(),
                timing_tags: Vec::new(),
                absolute_tags: [
                    vec![tag("apex", 0.25), tag("loop", 0.5), tag("end", 0.8)],
                    vec![tag("apex", 0.2), tag("loop", 0.3), tag("end", 0.6)],
                ],
                gesture_duration: None,
                relative_reference: None,
                flex_tracks: Vec::new(),
                loop_count: None,
                speech: None,
                actor: Some(0),
                channel: Some(0),
                enabled: true,
            }],
            ramp: Vec::new(),
            ignore_phonemes: false,
        })
    }
    fn update(
        data: &Arc<ChoreoScene>,
        playback: f32,
        weight: f32,
        moving: bool,
    ) -> GestureUpdate<'_> {
        GestureUpdate {
            actor: 1,
            scene: 9,
            event: 0,
            data,
            playback,
            weight,
            priority: 0,
            moving,
        }
    }
    fn clip(kind: &str, frames: usize) -> Clip {
        Clip {
            events: Vec::new(),
            fps: 30.,
            looping: false,
            frames: (0..frames)
                .map(|i| {
                    vec![Pose {
                        position: Vec3::ZERO,
                        rotation: Quat::from_rotation_z(i as f32 * 0.01),
                    }]
                })
                .collect(),
            layer: ClipLayer {
                flags: sequence_flags::DELTA,
                faceposer: Some(Faceposer {
                    kind: kind.into(),
                    // 12 of (52 - 2) frames = 0.24 is within 0.05 of apex; loop moves to 0.5.
                    tags: vec![("apex".into(), 12), ("loop".into(), 25)],
                    start_loop: "loop".into(),
                    end_loop: "end".into(),
                }),
                ..Default::default()
            },
        }
    }
    #[test]
    fn layers_start_update_and_fade_with_remove_layer_rates() {
        let data = scene();
        let mut layers = GestureLayers::default();
        layers.update(update(&data, 0.1, 0.8, false));
        layers.update(update(&data, 0.2, 0.9, false));
        let layer = &layers.layers(1)[0];
        assert_eq!(
            (layer.sequence.as_str(), layer.playback, layer.weight),
            ("g_wave", 0.2, 0.9)
        );
        assert_eq!(layers.active_events(9), [0]);
        layers.release(9, Some(0), false);
        assert!(layers.active_events(9).is_empty());
        layers.advance(0.05, |_| true);
        assert!((layers.layers(1)[0].weight - 0.45).abs() < 1e-5);
        layers.advance(0.06, |_| true);
        assert!(layers.layers(1).is_empty());
        // Canceled scenes fade over half a second; dead actors drop immediately.
        layers.update(update(&data, 0.3, 1., false));
        layers.release(9, None, true);
        layers.advance(0.25, |_| true);
        assert!((layers.layers(1)[0].weight - 0.5).abs() < 1e-5);
        layers.advance(0.01, |_| false);
        assert!(layers.report().is_empty());
        assert_eq!(layers.signature(1), 0);
    }
    #[test]
    fn postures_fade_while_moving_and_gestures_ignore_motion() {
        let data = scene();
        let mut layers = GestureLayers::default();
        layers.update(update(&data, 0.1, 1., true));
        layers.update(update(&data, 0.1, 1., false));
        layers.update(update(&data, 0.1, 1., false));
        let layer = &layers.layers(1)[0];
        assert!((layer.posture_weight - 0.4).abs() < 1e-6);
        let spline = 3. * 0.16 - 2. * 0.064;
        assert!((layer.layer_weight(&clip("posture", 52)) - spline).abs() < 1e-6);
        assert_eq!(layer.layer_weight(&clip("gesture", 52)), 1.);
    }
    #[test]
    fn cycle_repositions_original_tags_from_model_frames_and_keeps_loop_linear() {
        let data = scene();
        let mut layers = GestureLayers::default();
        // Playback halfway between loop (0.5) and end (0.8), both linear.
        layers.update(update(&data, 0.65, 1., false));
        let layer = &layers.layers(1)[0];
        // ORIGINAL loop moves 0.3 -> 25/50; end stays 0.6.
        assert!((layer.cycle(&clip("gesture", 52)) - 0.55).abs() < 1e-5);
        let mut untagged = clip("gesture", 52);
        untagged.layer.faceposer = None;
        // Without Faceposer data, no linear tags: Catmull-Rom between authored tags.
        let plain = layer.cycle(&untagged);
        assert!(plain > 0.3 && plain < 0.6);
    }
    #[test]
    fn compose_applies_layers_and_reports_missing_sequences() {
        let data = scene();
        let mut layers = GestureLayers::default();
        let mut rig = Rig {
            bones: vec![Bone {
                name: "root".into(),
                parent: None,
                bind: Pose {
                    position: Vec3::ZERO,
                    rotation: Quat::IDENTITY,
                },
                inverse_bind: Mat4::IDENTITY,
            }],
            ..Default::default()
        };
        assert_eq!(layers.compose(&rig, 1, "", 0., &[]).0, rig.matrices("", 0.));
        layers.update(update(&data, 0.65, 1., false));
        let (_, errors) = layers.compose(&rig, 1, "", 0., &[]);
        assert_eq!(errors, [PoseError::MissingSequence("g_wave".into())]);
        rig.clips.insert("g_wave".into(), clip("gesture", 52));
        let (matrices, errors) = layers.compose(&rig, 1, "", 0., &[]);
        assert!(errors.is_empty());
        let (_, rotation, _) = matrices[0].to_scale_rotation_translation();
        // Cycle 0.55 of 51 intervals: about frame 28 of the rotating test clip.
        assert!((rotation.to_axis_angle().1 - 0.55 * 51. * 0.01).abs() < 1e-3);
        assert_ne!(layers.signature(1), 0);
    }
}
