//! What this run's profilers are doing, for on-screen diagnostics. Exists only when a profiler
//! runs (`--profile`, or a build that writes a trace), so ordinary play pays nothing.
use super::{FRAME_TOTAL, Performance, SAMPLE_LIMIT};
use bevy::prelude::{App, First, Res, ResMut, Resource};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

/// A build that records a trace for the whole run (selected by the application).
#[derive(Clone, Debug, Default)]
pub struct TraceRecording {
    /// The file being written, when the application knows it.
    pub file: Option<PathBuf>,
}

/// The trace file's size is read at most this often: one metadata call, never per frame.
const TRACE_SIZE_INTERVAL: Duration = Duration::from_secs(1);

/// What a recorder is doing now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecorderState {
    /// Running, but its measurements have not started (the warm-up is discarded).
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
    pub elapsed: Duration,
    /// Bytes written so far, `None` when the file is unknown or unreadable.
    pub bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProfilingSnapshot {
    pub stages: Option<StageRecorder>,
    pub trace: Option<TraceRecorder>,
}

#[derive(Resource)]
pub struct Profiling {
    launched: Instant,
    stages: bool,
    stages_started: Option<Instant>,
    stages_finished: Option<Instant>,
    trace: Option<TraceRecording>,
    trace_bytes: Option<u64>,
    trace_read: Option<Instant>,
    snapshot: ProfilingSnapshot,
}

impl Profiling {
    pub fn snapshot(&self) -> ProfilingSnapshot {
        self.snapshot
    }
    fn update(&mut self, now: Instant, recorded: Option<usize>) {
        let stages = self.stages.then(|| {
            let frames = recorded.unwrap_or(0);
            if frames > 0 && self.stages_started.is_none() {
                self.stages_started = Some(now);
            }
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
        });
        if let Some(file) = self.trace.as_ref().and_then(|t| t.file.as_ref())
            && self
                .trace_read
                .is_none_or(|read| now.duration_since(read) >= TRACE_SIZE_INTERVAL)
        {
            self.trace_read = Some(now);
            self.trace_bytes = std::fs::metadata(file).ok().map(|m| m.len());
        }
        let trace = self.trace.as_ref().map(|_| TraceRecorder {
            elapsed: now.duration_since(self.launched),
            bytes: self.trace_bytes,
        });
        self.snapshot = ProfilingSnapshot { stages, trace };
    }
}

/// Adds the status when a profiler runs; otherwise nothing is added.
pub fn install(app: &mut App, stages: bool, trace: Option<TraceRecording>) {
    if !stages && trace.is_none() {
        return;
    }
    app.insert_resource(Profiling {
        launched: Instant::now(),
        stages,
        stages_started: None,
        stages_finished: None,
        trace,
        trace_bytes: None,
        trace_read: None,
        snapshot: ProfilingSnapshot::default(),
    })
    .add_systems(First, update);
    #[cfg(feature = "trace")]
    crate::state_trace::install(app);
}

fn update(mut profiling: ResMut<Profiling>, performance: Option<Res<Performance>>) {
    let recorded = performance.map(|p| p.recorded(FRAME_TOTAL));
    profiling.update(Instant::now(), recorded);
}

#[cfg(test)]
mod tests {
    use super::{
        Profiling, ProfilingSnapshot, RecorderState, SAMPLE_LIMIT, TraceRecorder, TraceRecording,
    };
    use std::time::{Duration, Instant};

    fn profiling(stages: bool, trace: Option<TraceRecording>, launched: Instant) -> Profiling {
        Profiling {
            launched,
            stages,
            stages_started: None,
            stages_finished: None,
            trace,
            trace_bytes: None,
            trace_read: None,
            snapshot: ProfilingSnapshot::default(),
        }
    }

    #[test]
    fn stages_go_idle_recording_finished_and_freeze_their_time() {
        let t0 = Instant::now();
        let mut p = profiling(true, None, t0);
        p.update(t0, Some(0));
        assert_eq!(p.snapshot().stages.unwrap().state, RecorderState::Idle);
        p.update(t0 + Duration::from_secs(2), Some(1));
        let s = p.snapshot().stages.unwrap();
        assert_eq!(
            (s.state, s.elapsed),
            (RecorderState::Recording, Duration::ZERO)
        );
        p.update(t0 + Duration::from_secs(5), Some(SAMPLE_LIMIT));
        p.update(t0 + Duration::from_secs(9), Some(SAMPLE_LIMIT));
        let s = p.snapshot().stages.unwrap();
        assert_eq!(s.state, RecorderState::Finished);
        assert_eq!(s.elapsed, Duration::from_secs(3));
        assert!(p.snapshot().trace.is_none());
    }

    #[test]
    fn a_trace_without_a_known_file_reports_no_size() {
        let t0 = Instant::now();
        let mut p = profiling(false, Some(TraceRecording::default()), t0);
        p.update(t0 + Duration::from_millis(1500), None);
        let snapshot = p.snapshot();
        assert!(snapshot.stages.is_none());
        assert_eq!(
            snapshot.trace,
            Some(TraceRecorder {
                elapsed: Duration::from_millis(1500),
                bytes: None
            })
        );
    }
}
