//! What this run's profilers are doing, for on-screen diagnostics, and when they start. Exists
//! only when a profiler runs (`--profile`, or a build that writes a trace), so ordinary play
//! pays nothing. Both start with gameplay, not with the launch: the first frame that is not
//! loading, paused or a menu background level starts the trace, and the `--profile` stage
//! samples restart `STAGE_WARM_UP_FRAMES` gameplay frames later.
use super::{FRAME_TOTAL, Performance};
use bevy::prelude::{App, IntoScheduleConfigs, Res, ResMut, Resource};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Default trace size ceiling (`TRACE_CHROME_MAX_MB` override; `0` disables the ceiling).
pub const DEFAULT_TRACE_MAX_BYTES: u64 = 20 * 1024 * 1024 * 1024;

/// A build that records a trace (selected by the application).
#[derive(Clone, Debug, Default)]
pub struct TraceRecording {
    /// The file being written, when the application knows it.
    pub file: Option<PathBuf>,
    /// Stop tracing cleanly once the output reaches this many bytes (`None` is unlimited).
    pub max_bytes: Option<u64>,
    /// Heap allocations and requested bytes since the start, when the application counts them.
    pub allocations: Option<fn() -> (u64, u64)>,
}

impl TraceRecording {
    /// Parses a trace ceiling in MiB (as from `TRACE_CHROME_MAX_MB`), where `"0"` is unlimited
    /// and omitted or invalid values select [`DEFAULT_TRACE_MAX_BYTES`].
    pub fn max_bytes_from_mb(raw: Option<&str>) -> Option<u64> {
        let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            return Some(DEFAULT_TRACE_MAX_BYTES);
        };
        match raw.parse::<u64>() {
            Ok(0) => None,
            Ok(mb) => Some(mb.saturating_mul(1024 * 1024)),
            Err(_) => Some(DEFAULT_TRACE_MAX_BYTES),
        }
    }
}

/// Starts and finishes the trace file; installed by builds that write one.
#[derive(Resource)]
pub struct TraceSwitch(pub Box<dyn Fn(TraceCommand<'_>) + Send + Sync>);
pub enum TraceCommand<'a> {
    /// Start writing to this file (or the trace writer's default).
    Start(Option<&'a Path>),
    /// Finish the file before the process exits.
    Finish,
}

/// Gameplay frames discarded before `--profile` stage samples start: shader and asset warm-up.
const STAGE_WARM_UP_FRAMES: u64 = 120;
/// The trace file's size is read at most this often: one metadata call, never per frame.
const TRACE_SIZE_INTERVAL: Duration = Duration::from_secs(1);

/// What a recorder is doing now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecorderState {
    /// Waiting for gameplay (and, for stage samples, the warm-up after it).
    Idle,
    Recording,
    /// The trace file has been finalized.
    Finished,
}

/// The `--profile` stage recorder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageRecorder {
    pub state: RecorderState,
    /// Time since measurements started.
    pub elapsed: Duration,
    pub frames: usize,
}

/// The trace recorder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceRecorder {
    pub state: RecorderState,
    /// Time since gameplay started the trace.
    pub elapsed: Duration,
    /// Bytes written so far, `None` before the trace starts or when the file is unknown.
    pub bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProfilingSnapshot {
    pub stages: Option<StageRecorder>,
    pub trace: Option<TraceRecorder>,
}

/// What one frame asks of the recorders.
#[derive(Debug, Default, PartialEq, Eq)]
struct Actions {
    gameplay_started: bool,
    start_trace: bool,
    finish_trace: bool,
    restart_stages: bool,
}

#[derive(Resource)]
pub struct Profiling {
    stages: bool,
    gameplay_started: Option<Instant>,
    gameplay_frames: u64,
    stages_started: Option<Instant>,
    trace: Option<TraceRecording>,
    trace_finished: Option<Instant>,
    trace_bytes: Option<u64>,
    trace_read: Option<Instant>,
    snapshot: ProfilingSnapshot,
}

impl Profiling {
    fn new(stages: bool, trace: Option<TraceRecording>) -> Self {
        Self {
            stages,
            gameplay_started: None,
            gameplay_frames: 0,
            stages_started: None,
            trace,
            trace_finished: None,
            trace_bytes: None,
            trace_read: None,
            snapshot: ProfilingSnapshot::default(),
        }
    }
    pub fn snapshot(&self) -> ProfilingSnapshot {
        self.snapshot
    }
    /// Whether the trace file is actively recording gameplay (neither waiting nor stopped).
    pub fn trace_recording(&self) -> bool {
        matches!(
            self.snapshot.trace,
            Some(trace) if trace.state == RecorderState::Recording
        )
    }
    /// Heap allocations and bytes since the start, when the build counts them.
    pub fn allocations(&self) -> Option<(u64, u64)> {
        self.trace
            .as_ref()
            .and_then(|t| t.allocations)
            .map(|totals| totals())
    }
    /// One frame: `gameplay` when the world runs for the player; `recorded`, the stage samples
    /// kept so far.
    fn update(&mut self, now: Instant, gameplay: bool, recorded: Option<usize>) -> Actions {
        let mut actions = Actions::default();
        if gameplay {
            if self.gameplay_started.is_none() {
                self.gameplay_started = Some(now);
                actions.gameplay_started = true;
                actions.start_trace = self.trace.is_some();
            }
            self.gameplay_frames += 1;
            if self.stages
                && self.stages_started.is_none()
                && self.gameplay_frames >= STAGE_WARM_UP_FRAMES
            {
                self.stages_started = Some(now);
                actions.restart_stages = true;
            }
        }
        let stages = self
            .stages
            .then(|| self.stage_recorder(now, recorded, &actions));
        let trace = self.trace_recorder(now, &mut actions);
        self.snapshot = ProfilingSnapshot { stages, trace };
        actions
    }
    fn stage_recorder(
        &self,
        now: Instant,
        recorded: Option<usize>,
        actions: &Actions,
    ) -> StageRecorder {
        // Samples from before the restart are dropped by it.
        let frames = match self.stages_started {
            Some(_) if !actions.restart_stages => recorded.unwrap_or(0),
            _ => 0,
        };
        let (state, elapsed) = self
            .stages_started
            .map_or((RecorderState::Idle, Duration::ZERO), |start| {
                (RecorderState::Recording, now.duration_since(start))
            });
        StageRecorder {
            state,
            elapsed,
            frames,
        }
    }
    fn trace_recorder(&mut self, now: Instant, actions: &mut Actions) -> Option<TraceRecorder> {
        let trace = self.trace.as_ref()?;
        let Some(started) = self.gameplay_started else {
            return Some(TraceRecorder {
                state: RecorderState::Idle,
                elapsed: Duration::ZERO,
                bytes: None,
            });
        };
        if self.trace_finished.is_none()
            && let Some(file) = trace.file.as_ref()
            && self
                .trace_read
                .is_none_or(|read| now.duration_since(read) >= TRACE_SIZE_INTERVAL)
        {
            self.trace_read = Some(now);
            self.trace_bytes = std::fs::metadata(file).ok().map(|m| m.len());
        }
        if self.trace_finished.is_none()
            && let (Some(limit), Some(bytes)) = (trace.max_bytes, self.trace_bytes)
            && bytes >= limit
        {
            self.trace_finished = Some(now);
            actions.finish_trace = true;
        }
        let state = if self.trace_finished.is_some() {
            RecorderState::Finished
        } else {
            RecorderState::Recording
        };
        let elapsed = self.trace_finished.unwrap_or(now).duration_since(started);
        Some(TraceRecorder {
            state,
            elapsed,
            bytes: self.trace_bytes,
        })
    }
}

/// Adds the status when a profiler runs; otherwise nothing is added.
pub fn install(app: &mut App, stages: bool, trace: Option<TraceRecording>) {
    if !stages && trace.is_none() {
        return;
    }
    app.insert_resource(Profiling::new(stages, trace))
        .add_systems(
            bevy::app::RunFixedMainLoop,
            update
                .in_set(bevy::app::RunFixedMainLoopSystems::BeforeFixedMainLoop)
                .after(crate::ui_system::UiInputSet),
        );
    #[cfg(feature = "trace")]
    crate::state_trace::install(app);
}

fn update(
    mut profiling: ResMut<Profiling>,
    performance: Option<Res<Performance>>,
    sim: Option<Res<crate::simulation::Simulation>>,
    ui: Option<Res<crate::ui_host::Console>>,
    switch: Option<Res<TraceSwitch>>,
) {
    if let Some(performance) = performance.as_deref() {
        performance.set_loading(sim.as_deref().is_some_and(|sim| sim.loading()));
    }
    let gameplay = sim.is_some_and(|sim| !sim.paused() && !sim.background_level)
        && !ui.is_some_and(|ui| ui.source.paused());
    let recorded = performance.as_deref().map(|p| p.recorded(FRAME_TOTAL));
    let actions = profiling.update(Instant::now(), gameplay, recorded);
    if actions.gameplay_started {
        bevy::log::info!("profiling: gameplay started");
        if let Some(performance) = performance.as_deref() {
            performance.start_pipeline_log();
        }
    }
    if actions.restart_stages
        && let Some(performance) = performance.as_deref()
    {
        performance.restart_stages();
    }
    if actions.start_trace
        && let Some(switch) = switch.as_deref()
    {
        let file = profiling.trace.as_ref().and_then(|t| t.file.as_deref());
        (switch.0)(TraceCommand::Start(file));
    }
    if actions.finish_trace
        && let Some(switch) = switch.as_deref()
    {
        let bytes = profiling
            .snapshot()
            .trace
            .and_then(|t| t.bytes)
            .unwrap_or(0);
        bevy::log::warn!(
            "trace reached size limit ({:.2} GB); finishing trace cleanly",
            bytes as f64 / (1024. * 1024. * 1024.)
        );
        (switch.0)(TraceCommand::Finish);
    }
}

#[cfg(test)]
mod tests {
    use super::{Actions, Profiling, RecorderState, STAGE_WARM_UP_FRAMES, TraceRecording};
    use std::time::{Duration, Instant};

    #[test]
    fn stages_wait_for_gameplay_and_its_warm_up_then_keep_recording() {
        let t0 = Instant::now();
        let mut p = Profiling::new(true, None);
        // Menu and loading frames: nothing starts.
        for _ in 0..500 {
            assert_eq!(p.update(t0, false, Some(4096)), Actions::default());
        }
        assert_eq!(p.snapshot().stages.unwrap().state, RecorderState::Idle);
        for frame in 1..STAGE_WARM_UP_FRAMES {
            let step = p.update(t0 + Duration::from_secs(1), true, Some(frame as usize));
            assert_eq!(step.gameplay_started, frame == 1);
        }
        assert_eq!(p.snapshot().stages.unwrap().state, RecorderState::Idle);
        let actions = p.update(t0 + Duration::from_secs(2), true, Some(200));
        assert!(actions.restart_stages && !actions.start_trace && !actions.gameplay_started);
        let s = p.snapshot().stages.unwrap();
        assert_eq!((s.state, s.frames), (RecorderState::Recording, 0));
        p.update(t0 + Duration::from_secs(5), false, Some(4096));
        p.update(t0 + Duration::from_secs(9), true, Some(8192));
        let s = p.snapshot().stages.unwrap();
        assert_eq!((s.state, s.frames), (RecorderState::Recording, 8192));
        assert_eq!(s.elapsed, Duration::from_secs(7));
        assert!(p.snapshot().trace.is_none());
    }

    #[test]
    fn the_trace_starts_once_with_gameplay_and_times_from_there() {
        let t0 = Instant::now();
        let mut p = Profiling::new(false, Some(TraceRecording::default()));
        assert_eq!(p.update(t0, false, None), Actions::default());
        let trace = p.snapshot().trace.unwrap();
        assert_eq!((trace.state, trace.bytes), (RecorderState::Idle, None));
        let actions = p.update(t0 + Duration::from_secs(4), true, None);
        assert!(actions.gameplay_started && actions.start_trace && !actions.restart_stages);
        assert!(
            !p.update(t0 + Duration::from_secs(5), false, None)
                .gameplay_started
        );
        assert!(
            !p.update(t0 + Duration::from_secs(6), true, None)
                .start_trace
        );
        let trace = p.snapshot().trace.unwrap();
        assert_eq!(trace.state, RecorderState::Recording);
        assert_eq!(trace.elapsed, Duration::from_secs(2));
        assert!(p.snapshot().stages.is_none());
    }

    #[test]
    fn trace_stays_idle_while_ui_paused_or_window_unfocused_and_starts_on_gameplay() {
        use bevy::prelude::*;
        use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow, WindowResolution};
        use std::sync::{Arc, Mutex};

        let mut app = App::new();
        let events: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
        let recorded = events.clone();

        app.add_plugins((MinimalPlugins, crate::ui_system::UiPlugin))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_message::<AppExit>()
            .add_message::<bevy::window::WindowEvent>()
            .add_message::<bevy::window::WindowCloseRequested>()
            .add_message::<bevy::input::mouse::MouseWheel>()
            .insert_resource(crate::simulation::Simulation::new(
                &modkit_core::World::default(),
                glam::Vec3::ZERO,
                0.,
                0.,
                false,
                vec![],
            ))
            .insert_resource(crate::gameplay::Gameplay::synthetic(
                modkit_core::World::default(),
            ))
            .insert_resource(crate::ui_host::Console::new(Default::default()))
            .insert_resource(super::TraceSwitch(Box::new(move |cmd| {
                let tag = match cmd {
                    super::TraceCommand::Start(_) => "start",
                    super::TraceCommand::Finish => "finish",
                };
                recorded.lock().unwrap().push(tag);
            })));
        super::install(&mut app, false, Some(TraceRecording::default()));

        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: WindowResolution::new(1024, 768),
                    focused: false,
                    ..default()
                },
                CursorOptions {
                    grab_mode: CursorGrabMode::None,
                    visible: true,
                    ..default()
                },
                PrimaryWindow,
            ))
            .id();

        // Unfocused window: `update_ui` pauses before `status::update` inspects gameplay.
        app.update();
        assert!(events.lock().unwrap().is_empty());
        assert_eq!(
            app.world()
                .resource::<Profiling>()
                .snapshot()
                .trace
                .unwrap()
                .state,
            RecorderState::Idle
        );

        // Focused window with console open: trace still stays idle.
        app.world_mut()
            .entity_mut(window)
            .get_mut::<Window>()
            .unwrap()
            .focused = true;
        app.world_mut()
            .resource_mut::<crate::ui_host::Console>()
            .source
            .set_mode(hl2_ui::console::Mode::Console);
        app.update();
        assert!(events.lock().unwrap().is_empty());
        assert_eq!(
            app.world()
                .resource::<Profiling>()
                .snapshot()
                .trace
                .unwrap()
                .state,
            RecorderState::Idle
        );

        // Console closed into gameplay: trace starts cleanly once.
        app.world_mut()
            .resource_mut::<crate::ui_host::Console>()
            .source
            .set_mode(hl2_ui::console::Mode::Gameplay);
        app.update();
        assert_eq!(*events.lock().unwrap(), vec!["start"]);
        assert_eq!(
            app.world()
                .resource::<Profiling>()
                .snapshot()
                .trace
                .unwrap()
                .state,
            RecorderState::Recording
        );
    }

    #[test]
    fn trace_finishes_cleanly_at_max_bytes_and_freezes_elapsed() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("hl2rs-trace-cap-{nanos}.json"));
        std::fs::write(&path, [b'a'; 32]).unwrap();

        let t0 = Instant::now();
        let mut p = Profiling::new(
            false,
            Some(TraceRecording {
                file: Some(path.clone()),
                max_bytes: Some(16),
                allocations: None,
            }),
        );
        let first = p.update(t0, true, None);
        assert!(first.start_trace && first.finish_trace);
        assert!(!p.trace_recording());
        let snap = p.snapshot().trace.unwrap();
        assert_eq!(snap.state, RecorderState::Finished);
        assert_eq!(snap.bytes, Some(32));
        assert_eq!(snap.elapsed, Duration::ZERO);

        // Second check using a post-start size growth past max_bytes after TRACE_SIZE_INTERVAL.
        let mut p2 = Profiling::new(
            false,
            Some(TraceRecording {
                file: Some(path.clone()),
                max_bytes: Some(64),
                allocations: None,
            }),
        );
        let a0 = p2.update(t0, true, None);
        assert!(a0.start_trace && !a0.finish_trace);
        assert!(p2.trace_recording());

        std::fs::write(&path, [b'b'; 80]).unwrap();
        let a1 = p2.update(t0 + Duration::from_secs(1), true, None);
        assert!(a1.finish_trace);
        assert!(!p2.trace_recording());
        let s1 = p2.snapshot().trace.unwrap();
        assert_eq!(s1.state, RecorderState::Finished);
        assert_eq!(s1.bytes, Some(80));
        assert_eq!(s1.elapsed, Duration::from_secs(1));

        let a2 = p2.update(t0 + Duration::from_secs(5), true, None);
        assert!(!a2.finish_trace);
        assert_eq!(p2.snapshot().trace.unwrap().elapsed, Duration::from_secs(1));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn trace_max_bytes_parses_default_zero_and_explicit_mib() {
        assert_eq!(
            TraceRecording::max_bytes_from_mb(None),
            Some(super::DEFAULT_TRACE_MAX_BYTES)
        );
        assert_eq!(
            TraceRecording::max_bytes_from_mb(Some("")),
            Some(super::DEFAULT_TRACE_MAX_BYTES)
        );
        assert_eq!(
            TraceRecording::max_bytes_from_mb(Some("bad")),
            Some(super::DEFAULT_TRACE_MAX_BYTES)
        );
        assert_eq!(TraceRecording::max_bytes_from_mb(Some("0")), None);
        assert_eq!(
            TraceRecording::max_bytes_from_mb(Some(" 512 ")),
            Some(512 * 1024 * 1024)
        );
    }
}
