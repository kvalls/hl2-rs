//! Bounded, epoch-aligned profile windows. No ordinary-play or per-frame allocation.
#[derive(Clone, Copy, Default)]
struct Window {
    start: f64,
    end: f64,
    frames: u32,
    maximum: f64,
}
const CAPACITY: usize = 128;
pub(super) struct Windows {
    anchor: Option<(f64, f64)>,
    completed: [Window; CAPACITY],
    total: usize,
    current: Window,
}
impl Default for Windows {
    fn default() -> Self {
        Self {
            anchor: None,
            completed: [Window::default(); CAPACITY],
            total: 0,
            current: Window::default(),
        }
    }
}
impl Windows {
    pub(super) fn observe(&mut self, now: f64, frame_ms: f64) {
        if self.anchor.is_none() {
            self.anchor = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|time| (now, time.as_secs_f64()));
        }
        if self.current.frames == 0 {
            self.current.start = now - frame_ms / 1000.;
        }
        self.current.end = now;
        self.current.frames += 1;
        self.current.maximum = self.current.maximum.max(frame_ms);
        if now - self.current.start >= 1. {
            self.completed[self.total % CAPACITY] = self.current;
            self.total += 1;
            self.current = Window::default();
        }
    }
    pub(super) fn report(&self) -> serde_json::Value {
        let offset = self.anchor.map(|(clock, epoch)| epoch - clock);
        let count = self.total.min(CAPACITY);
        let windows = (self.total - count..self.total)
            .map(|i| self.completed[i % CAPACITY])
            .chain((self.current.frames > 0).then_some(self.current))
            .map(|w| {
                let elapsed = w.end - w.start;
                serde_json::json!({
                    "start_unix_seconds":offset.map(|o| w.start+o),
                    "end_unix_seconds":offset.map(|o| w.end+o),
                    "frames":w.frames,"elapsed_seconds":elapsed,
                    "mean_frame_ms":if elapsed>0. {elapsed*1000./f64::from(w.frames)} else {0.},
                    "max_frame_ms":w.maximum,
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({"windows":windows,"dropped_windows":self.total-count})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_align_to_epoch_and_include_partial_tail_without_overlap() {
        let mut windows = Windows {
            anchor: Some((0., 1000.)),
            ..Default::default()
        };
        windows.observe(0.5, 500.);
        windows.observe(1., 500.);
        windows.observe(1.25, 250.);
        let report = windows.report();
        let rows = report["windows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["start_unix_seconds"], 1000.);
        assert_eq!(rows[0]["end_unix_seconds"], 1001.);
        assert_eq!(rows[0]["frames"], 2);
        assert_eq!(rows[0]["mean_frame_ms"], 500.);
        assert_eq!(rows[1]["start_unix_seconds"], 1001.);
        assert_eq!(rows[1]["end_unix_seconds"], 1001.25);
    }
    #[test]
    fn long_profiles_keep_a_bounded_chronological_tail() {
        let mut windows = Windows {
            anchor: Some((0., 1000.)),
            ..Default::default()
        };
        for second in 1..=140 {
            windows.observe(f64::from(second), 1000.);
        }
        let report = windows.report();
        let rows = report["windows"].as_array().unwrap();
        assert_eq!(rows.len(), CAPACITY);
        assert_eq!(report["dropped_windows"], 12);
        assert_eq!(rows[0]["start_unix_seconds"], 1012.);
        assert_eq!(rows.last().unwrap()["end_unix_seconds"], 1140.);
        assert_eq!(
            Windows::default().report()["windows"],
            serde_json::json!([])
        );
    }
}
