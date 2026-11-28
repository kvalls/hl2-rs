//! Whole-frame wall time split by main schedule, so scoped stages can be checked against the frame.
//!
//! A one-system marker schedule runs before the first main schedule and after each one. Each
//! marker records the time since the previous marker under the schedule that ran in between;
//! the first marker records the time spent outside the main schedule (extraction, waiting for
//! the render thread, the window event loop). The stages add up to `frame_total`.
use super::Performance;
use bevy::{
    app::MainScheduleOrder,
    ecs::schedule::{ScheduleLabel, SingleThreadedExecutor},
    prelude::{App, Res, ResMut, Resource, Schedule},
};
use std::time::Instant;

/// Stage recorded for the time between the last main schedule and the next frame's first one.
const OUTSIDE: &str = "main_schedule.outside";
/// Stage recorded for the whole frame, first marker to first marker.
const TOTAL: &str = super::FRAME_TOTAL;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
struct Marker(usize);

#[derive(Resource, Default)]
struct Marks {
    previous: Option<Instant>,
    frame: Option<Instant>,
    pending: Vec<(&'static str, f64)>,
}

/// Wraps the main schedules present now; one added later is counted with its predecessor.
pub(super) fn install(app: &mut App) {
    app.init_resource::<Marks>();
    let schedules = app.world().resource::<MainScheduleOrder>().labels.clone();
    let mut wrapped = Vec::with_capacity(schedules.len() * 2 + 1);
    for index in 0..=schedules.len() {
        // Profiling names live for the whole run; there is one per main schedule.
        let name: &'static str = match index.checked_sub(1) {
            Some(before) => {
                Box::leak(format!("main_schedule.{:?}", schedules[before]).into_boxed_str())
            }
            None => OUTSIDE,
        };
        let mut schedule = Schedule::new(Marker(index));
        schedule.set_executor(SingleThreadedExecutor::new());
        schedule.add_systems(
            move |mut marks: ResMut<Marks>, performance: Res<Performance>| {
                let now = Instant::now();
                if let Some(previous) = marks.previous.replace(now) {
                    marks
                        .pending
                        .push((name, (now - previous).as_secs_f64() * 1000.));
                }
                if index == 0
                    && let Some(frame) = marks.frame.replace(now)
                {
                    // Publish only complete frames so warm-up, resets and reports
                    // always see the same sample window for every stage.
                    marks
                        .pending
                        .push((TOTAL, (now - frame).as_secs_f64() * 1000.));
                    performance.record_stages(marks.pending.drain(..));
                }
            },
        );
        app.add_schedule(schedule);
        wrapped.push(Marker(index).intern());
        if let Some(label) = schedules.get(index) {
            wrapped.push(*label);
        }
    }
    app.world_mut().resource_mut::<MainScheduleOrder>().labels = wrapped;
}

#[cfg(test)]
mod tests {
    use super::{Performance, TOTAL, install};
    use bevy::prelude::*;

    #[test]
    fn stages_cover_every_main_schedule_and_add_up_to_the_frame() {
        let mut app = App::new();
        let performance = Performance::default();
        app.insert_resource(performance.clone());
        app.add_systems(Update, || {
            std::thread::sleep(std::time::Duration::from_millis(1))
        });
        install(&mut app);
        for _ in 0..40 {
            app.update();
        }
        let report = performance.report();
        let stages = report.as_object().expect("stages");
        for name in ["First", "PreUpdate", "RunFixedMainLoop", "Update", "Last"] {
            assert!(
                stages.contains_key(&format!("main_schedule.{name}")),
                "{name}"
            );
        }
        let mean = |name: &str| stages[name]["mean_ms"].as_f64().expect("mean");
        assert!(mean("main_schedule.Update") >= 1.);
        let parts: f64 = stages
            .iter()
            .filter(|(name, _)| name.starts_with("main_schedule."))
            .map(|(name, _)| mean(name))
            .sum();
        for (name, stats) in stages {
            assert_eq!(stats["samples"], 9, "{name}");
            assert_eq!(stats["calls"], 39, "{name}");
        }
        assert!(
            (parts - mean(TOTAL)).abs() < 1e-9,
            "{parts} {}",
            mean(TOTAL)
        );
        performance.restart_stages();
        assert!(performance.report().as_object().expect("stages").is_empty());
    }
}
