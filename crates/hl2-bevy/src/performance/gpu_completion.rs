//! Bounded GPU submission pacing for offscreen profiling, not interactive gameplay.
use super::Performance;
use bevy::{
    ecs::schedule::IntoScheduleConfigs,
    prelude::{App, Res, ResMut, Resource},
    render::{
        RenderApp,
        render_resource::PollType,
        renderer::{RenderDevice, RenderGraph, RenderGraphSystems, RenderQueue},
    },
};
use std::time::{Duration, Instant};

#[derive(Default, serde::Serialize)]
pub(super) struct Stats {
    limit: usize,
    pub(super) camera_indirect_drawing_disabled: bool,
    submitted_frames: u64,
    completed_frames_observed: u64,
    max_pending_fences: u64,
    wait_calls: u64,
    total_wait_ms: f64,
    max_wait_ms: f64,
    errors: u64,
    raw_world: Reduction,
    raw_weapon: Reduction,
}
#[derive(Default, serde::Serialize)]
struct Reduction {
    passes: u64,
    processed_texels: u64,
    full_texels: u64,
}
impl Stats {
    pub(super) fn reduction(&mut self, world: bool, processed: u64, full: u64) {
        let data = if world {
            &mut self.raw_world
        } else {
            &mut self.raw_weapon
        };
        data.passes += 1;
        data.processed_texels += processed;
        data.full_texels += full;
    }

    fn submit(&mut self) {
        self.submitted_frames += 1;
        self.max_pending_fences = self
            .max_pending_fences
            .max(self.submitted_frames - self.completed_frames_observed);
    }
    fn complete(&mut self, ms: f64) {
        self.completed_frames_observed += 1;
        self.wait_calls += 1;
        self.total_wait_ms += ms;
        self.max_wait_ms = self.max_wait_ms.max(ms);
    }
}
#[derive(Resource)]
struct Fences {
    slots: [Option<PollType>; 2],
    next: usize,
    failed: bool,
}

pub(super) fn prepare(app: &mut App) -> Performance {
    let performance = Performance::default();
    if !app.is_plugin_added::<bevy::winit::WinitPlugin>()
        && let Some(render) = app.get_sub_app_mut(RenderApp)
    {
        *performance.1.lock().expect("GPU profiling state") = Some(Stats {
            limit: 2,
            ..Default::default()
        });
        render
            .insert_resource(Fences {
                slots: [None, None],
                next: 0,
                failed: false,
            })
            .add_systems(RenderGraph, wait.in_set(RenderGraphSystems::Begin))
            .add_systems(RenderGraph, mark.in_set(RenderGraphSystems::Finish));
    }
    performance
}
fn wait(mut fences: ResMut<Fences>, device: Res<RenderDevice>, performance: Res<Performance>) {
    let index = fences.next;
    if let Some(wait) = fences.slots[index].take() {
        let started = Instant::now();
        let result = device.poll(wait);
        let mut data = performance.1.lock().expect("GPU profiling state");
        let stats = data.as_mut().expect("headless profiler");
        if let Err(error) = result {
            stats.errors += 1;
            fences.failed = true;
            fences.slots = [None, None];
            bevy::log::error!("headless GPU completion wait failed: {error}");
        } else {
            stats.complete(started.elapsed().as_secs_f64() * 1000.);
        }
    }
}
fn mark(mut fences: ResMut<Fences>, queue: Res<RenderQueue>, performance: Res<Performance>) {
    if fences.failed {
        return;
    }
    let index = fences.next;
    // Empty submission marks all preceding render work without adding a render pass.
    fences.slots[index] = Some(PollType::Wait {
        submission_index: Some(queue.submit([])),
        timeout: Some(Duration::from_secs(30)),
    });
    fences.next = (index + 1) % 2;
    performance
        .1
        .lock()
        .expect("GPU profiling state")
        .as_mut()
        .expect("headless profiler")
        .submit();
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observed_completion_and_backlog_counters_stay_bounded() {
        let mut stats = Stats {
            limit: 2,
            ..Default::default()
        };
        for frame in 0..100 {
            if frame >= 2 {
                stats.complete(3.);
            }
            stats.submit();
        }
        assert_eq!(stats.submitted_frames, 100);
        assert_eq!(stats.completed_frames_observed, 98);
        assert_eq!(stats.max_pending_fences, 2);
        assert_eq!(stats.wait_calls, 98);
        assert_eq!(stats.total_wait_ms, 294.);
    }
    #[test]
    fn profiler_without_a_renderer_has_no_gpu_fences() {
        let mut app = App::new();
        let performance = prepare(&mut app);
        let mut output = serde_json::json!({});
        performance.write_report(&mut output, None);
        assert!(output["gpu_completion"].is_null());
        assert_eq!(output["cpu_stages"], serde_json::json!({}));
    }
}
