//! Scene control events (SDK sceneentity.cpp CSceneEntity::StartEvent, CChoreoEvent::LOOP;
//! choreoscene.cpp LoopToTime): a Loop jumps the scene back to the time its parameter names
//! while it has loops left (a loop count of -1 repeats forever), and the events from there on
//! start again. d1_trainstation_01's cit_pacing.vcd loops the pacer's lines at 9.09 s.
use super::super::{Choreography, Playback, Scene, ScenePause};
use source_assets::scenes::EventType;
use std::collections::BTreeMap;

/// Each Loop event's remaining loops (GetNumLoopsRemaining) and the jump of this tick.
#[derive(Clone, Debug, Default)]
pub(in crate::entities) struct Loops {
    remaining: BTreeMap<usize, i8>,
    jump: Option<f64>,
}

impl Scene {
    /// A Loop event jumps back (once its loops run out it does nothing); SubScene is not
    /// supported and pauses the scene.
    pub(super) fn control_event(
        &mut self,
        play: &mut Playback,
        scene: &Choreography,
        index: usize,
    ) {
        let event = &scene.data.events[index];
        if event.kind == EventType::Loop {
            let back = event.parameters[0].trim().parse::<f32>().ok();
            // A target at or after the loop itself would loop within one tick.
            if let Some(back) = back.filter(|b| b.is_finite() && *b < event.start) {
                let back = back.max(0.);
                let count = event.loop_count.unwrap_or(-1);
                let left = play.loops.remaining.entry(index).or_insert(count);
                if count != -1 {
                    if *left <= 0 {
                        return;
                    }
                    *left -= 1;
                }
                play.loops.jump = Some(f64::from(back));
                play.next = scene
                    .order
                    .iter()
                    .position(|&i| scene.data.events[i].start >= back)
                    .unwrap_or(scene.order.len());
                return;
            }
        }
        self.unsupported_input(
            "logic_choreographed_scene",
            &format!("control:{:?}", event.kind),
        );
        play.pause = Some(ScenePause::UnsupportedControl);
    }
}

/// Whether event dispatch stops for this tick: a pause, or a loop's jump.
pub(super) fn halted(play: &Playback) -> bool {
    play.pause.is_some() || play.loops.jump.is_some()
}

/// The scene time after this tick's events: a loop's target, else `end` unless paused.
pub(super) fn advance(play: &mut Playback, end: f64) -> f64 {
    match play.loops.jump.take() {
        Some(back) => back,
        None if play.pause.is_none() => end,
        None => play.elapsed,
    }
}
