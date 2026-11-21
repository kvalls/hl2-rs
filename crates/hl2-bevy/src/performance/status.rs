//! What this run's profilers are doing, for on-screen diagnostics, and when they start. Exists
//! only when a profiler runs (`--profile`, or a build that writes a trace), so ordinary play
//! pays nothing. Both start with gameplay, not with the launch: the first frame that is not
//! loading, paused or a menu background level starts the trace, and the `--profile` stage
//! samples restart `STAGE_WARM_UP_FRAMES` gameplay frames later.
use super::{FRAME_TOTAL, Performance, SAMPLE_LIMIT};
use bevy::prelude::{App, First, Res, ResMut, Resource};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// A build that records a trace (selected by the application).
#[derive(Clone, Debug, Default)]
pub struct TraceRecording {
    /// The file being written, when the application knows it.
    pub file: Option<PathBuf>,
    /// Heap allocations and requested bytes since the start, when the application counts them.
    pub allocations: Option<fn() -> (u64, u64)>,
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
    /// Its sample buffer is full; later frames are not measured.
    Finished,
}

/// The `--profile` stage recorder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageRecorder {
    pub state: RecorderState,
    /// Time since measurements started, frozen when they finished.
    pub elapsed: Duration,
    pub frames: usize,
    pub limit: usize,
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
    start_trace: bool,
    restart_stages: bool,
}

#[derive(Resource)]
pub struct Profiling {
    stages: bool,
    gameplay_started: Option<Instant>,
    gameplay_frames: u64,
    stages_started: Option<Instant>,
    stages_finished: Option<Instant>,
    trace: Option<TraceRecording>,
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
            stages_finished: None,
            trace,
            trace_bytes: None,
            trace_read: None,
            snapshot: ProfilingSnapshot::default(),
        }
    }
    pub fn snapshot(&self) -> ProfilingSnapshot {
        self.snapshot
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
        let trace = self.trace_recorder(now);
        self.snapshot = ProfilingSnapshot { stages, trace };
        actions
    }
    fn stage_recorder(
        &mut self,
        now: Instant,
        recorded: Option<usize>,
        actions: &Actions,
    ) -> StageRecorder {
        // Samples from before the restart are dropped by it.
        let frames = match self.stages_started {
            Some(_) if !actions.restart_stages => recorded.unwrap_or(0),
            _ => 0,
        };
        if frames >= SAMPLE_LIMIT && self.stages_finished.is_none() {
            self.stages_finished = Some(now);
        }
        let state = match (self.stages_started, self.stages_finished) {
            (_, Some(_)) => RecorderState::Finished,
            (Some(_), None) => RecorderState::Recording,
            (None, None) => RecorderState::Idle,
        };
        let elapsed = self.stages_started.map_or(Duration::ZERO, |start| {
            self.stages_finished.unwrap_or(now).duration_since(start)
        });
        StageRecorder {
            state,
            elapsed,
            frames,
            limit: SAMPLE_LIMIT,
        }
    }
    fn trace_recorder(&mut self, now: Instant) -> Option<TraceRecorder> {
        let trace = self.trace.as_ref()?;
        let Some(started) = self.gameplay_started else {
            return Some(TraceRecorder {
                state: RecorderState::Idle,
                elapsed: Duration::ZERO,
                bytes: None,
            });
        };
        if let Some(file) = trace.file.as_ref()
            && self
                .trace_read
                .is_none_or(|read| now.duration_since(read) >= TRACE_SIZE_INTERVAL)
        {
            self.trace_read = Some(now);
            self.trace_bytes = std::fs::metadata(file).ok().map(|m| m.len());
        }
        Some(TraceRecorder {
            state: RecorderState::Recording,
            elapsed: now.duration_since(started),
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
        .add_systems(First, update);
    #[cfg(feature = "trace")]
    crate::state_trace::install(app);
}

fn update(
    mut profiling: ResMut<Profiling>,
    performance: Option<Res<Performance>>,
    sim: Option<Res<crate::simulation::Simulation>>,
    switch: Option<Res<TraceSwitch>>,
) {
    let gameplay = sim.is_some_and(|sim| !sim.paused() && !sim.background_level);
    let recorded = performance.as_deref().map(|p| p.recorded(FRAME_TOTAL));
    let actions = profiling.update(Instant::now(), gameplay, recorded);
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
}

#[cfg(test)]
mod tests {
    use super::{
        Actions, Profiling, RecorderState, SAMPLE_LIMIT, STAGE_WARM_UP_FRAMES, TraceRecording,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn stages_wait_for_gameplay_and_its_warm_up_then_record_and_freeze_their_time() {
        let t0 = Instant::now();
        let mut p = Profiling::new(true, None);
        // Menu and loading frames: nothing starts.
        for _ in 0..500 {
            assert_eq!(p.update(t0, false, Some(4096)), Actions::default());
        }
        assert_eq!(p.snapshot().stages.unwrap().state, RecorderState::Idle);
        for frame in 1..STAGE_WARM_UP_FRAMES {
            p.update(t0 + Duration::from_secs(1), true, Some(frame as usize));
        }
        assert_eq!(p.snapshot().stages.unwrap().state, RecorderState::Idle);
        let actions = p.update(t0 + Duration::from_secs(2), true, Some(200));
        assert!(actions.restart_stages && !actions.start_trace);
        let s = p.snapshot().stages.unwrap();
        assert_eq!((s.state, s.frames), (RecorderState::Recording, 0));
        p.update(t0 + Duration::from_secs(5), false, Some(SAMPLE_LIMIT));
        p.update(t0 + Duration::from_secs(9), true, Some(SAMPLE_LIMIT));
        let s = p.snapshot().stages.unwrap();
        assert_eq!(s.state, RecorderState::Finished);
        assert_eq!(s.elapsed, Duration::from_secs(3));
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
        assert!(actions.start_trace && !actions.restart_stages);
        assert!(
            !p.update(t0 + Duration::from_secs(5), false, None)
                .start_trace
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
}
