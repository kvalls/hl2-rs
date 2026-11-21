//! Shared developer overlay painter. Fonts are supplied by the host.
use glam::{Vec2, vec2};
use source_ui::{
    canvas::{Canvas, Color, WHITE},
    font::FontFace,
};
const DEBUG_HEADER_HEIGHT: f32 = 105.;
/// Height of one profiling row added below the header's four rows.
const PROFILING_ROW: f32 = 20.;
const PROFILING_COLOR: Color = Color::new(0.45, 0.85, 0.95, 1.);
/// Height of one footer row; the footer holds the fifth and later lines, the last at the bottom.
const FOOTER_ROW: f32 = 26.;
pub struct OverlayPainter<'a> {
    pub canvas: &'a Canvas,
    pub font: Option<&'a FontFace>,
}
fn text(
    _canvas: &Canvas,
    value: &str,
    origin: Vec2,
    size: f32,
    scale: f32,
    font: Option<&FontFace>,
    color: Color,
) {
    if let Some(font) = font {
        font.draw_baseline(value, origin, size, scale, color);
    }
}
impl OverlayPainter<'_> {
    /// Four compact header rows and a footer of the remaining lines, the last at the bottom.
    /// Font data is bundled under its own MIT license, independent of installed game content.
    /// Returns the height the overlay occupies at the top of the canvas.
    /// `profiling` rows extend the header below its four rows.
    pub fn draw_debug(&self, lines: &[String], profiling: &[String]) -> f32 {
        let header = DEBUG_HEADER_HEIGHT + PROFILING_ROW * profiling.len() as f32;
        self.canvas.rectangle(
            0.,
            0.,
            self.canvas.width(),
            header,
            Color::new(0.025, 0.045, 0.07, 0.88),
        );
        let rows = [
            (vec2(22., 30.), 27., WHITE),
            (vec2(22., 55.), 19., Color::new(0.784, 0.784, 0.784, 1.)),
            (vec2(22., 78.), 17., Color::new(0.784, 0.784, 0.784, 1.)),
            (vec2(22., 98.), 16., Color::from_rgba(240, 180, 90, 255)),
        ];
        let draw = |line: &str, origin: Vec2, size: f32, color| {
            if let Some(font) = self.font {
                font.draw_baseline(line, origin, size, 1., color);
            } else {
                text(self.canvas, line, origin, size, 1., self.font, color);
            }
        };
        for (line, (origin, size, color)) in lines.iter().take(4).zip(rows) {
            draw(line, origin, size, color);
        }
        for (row, line) in profiling.iter().enumerate() {
            let baseline = DEBUG_HEADER_HEIGHT - 7. + PROFILING_ROW * (row + 1) as f32;
            draw(line, vec2(22., baseline), 16., PROFILING_COLOR);
        }
        let footer = lines.get(4..).unwrap_or_default();
        if !footer.is_empty() {
            let height = 12. + FOOTER_ROW * footer.len() as f32;
            let top = self.canvas.height() - height;
            let colour = Color::new(0.025, 0.045, 0.07, 0.85);
            self.canvas
                .rectangle(0., top, self.canvas.width(), height, colour);
            for (row, line) in footer.iter().enumerate() {
                let baseline = top + FOOTER_ROW * (row + 1) as f32;
                draw(line, vec2(20., baseline), 18., WHITE);
            }
        }
        header
    }

    fn draw_debug_lines_panel(&self, lines: &[String], top: f32, max_lines: usize) -> f32 {
        let count = lines.len().min(max_lines);
        let height = 20. + count as f32 * 22. + 10.;
        self.canvas.rectangle(
            0.,
            top,
            self.canvas.width(),
            height,
            Color::new(0.025, 0.045, 0.07, 0.9),
        );
        let draw = |line: &str, origin: Vec2, color| {
            if let Some(font) = self.font {
                font.draw_baseline(line, origin, 16., 1., color);
            } else {
                text(self.canvas, line, origin, 16., 1., self.font, color);
            }
        };
        let normal = Color::new(0.784, 0.784, 0.784, 1.);
        let last_idx = count.saturating_sub(1);
        for (i, line) in lines.iter().take(count).enumerate() {
            let y = top + 20. + i as f32 * 22.;
            let color = if i == last_idx {
                Color::from_rgba(240, 180, 90, 255)
            } else if i == 0 {
                WHITE
            } else {
                normal
            };
            draw(line, vec2(22., y), color);
        }
        top + height
    }

    /// Draws the live viewmodel tuning values with their top edge at `top`; returns the bottom
    /// edge.
    pub fn draw_viewmodel_debug(&self, lines: &[String], top: f32) -> f32 {
        self.draw_debug_lines_panel(lines, top, lines.len())
    }

    /// Draws the live background visual debug values with their top edge at `top`; returns the bottom
    /// edge.
    pub fn draw_background_debug(&self, lines: &[String], top: f32) -> f32 {
        self.draw_debug_lines_panel(lines, top, lines.len())
    }
}

/// Snapshot for F1. The host chooses which diagnostics to sample.
pub struct DeveloperFrame<'a> {
    pub map: &'a str,
    pub triangles: usize,
    pub textures: u64,
    pub entities: usize,
    pub flying: bool,
    pub fps: f64,
    pub friendly_fire: bool,
    pub eye: [f32; 3],
    /// View yaw and pitch in degrees, in the convention `--yaw` and `--pitch` take.
    pub angles: [f32; 2],
    pub frame_ms: f64,
    pub materials: usize,
    pub colliders: usize,
    pub bodies: usize,
}
pub fn developer_lines(frame: DeveloperFrame<'_>) -> Vec<String> {
    vec![
            format!("HL2-RS  /  Rust runtime  /  {}", frame.map),
            format!("{} triangles  |  {} textures  |  {} entities  |  {}  |  {:.0} fps", frame.triangles, frame.textures, frame.entities, if frame.flying { "FLY" } else { "WALK" }, frame.fps),
            "WASD / click: mouse / Esc pause / tilde console / F2 fly / Space,Ctrl vertical in fly / G prop impulse".into(),
            "E use / Ctrl crouch / Space jump / R reload / 1-6 / wheel: weapon menu / Q last / F3 dev loadout / F1 overlay / F4 NPC spawn".into(),
            format!("Player hurts friendlies: {} (F4 options, debug)", if frame.friendly_fire { "ON" } else { "off" }),
            location_line(&frame),
        ]
}

/// What a profiler is doing, as F1 names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfilerState {
    /// Waiting for gameplay (and, for stage samples, the warm-up after it).
    Idle,
    Profiling,
    /// Its sample buffer is full; later frames are not measured.
    Finished,
}
impl ProfilerState {
    fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Profiling => "profiling",
            Self::Finished => "finished",
        }
    }
}
/// The `--profile` stage recorder.
pub struct StageProfiler {
    pub state: ProfilerState,
    pub elapsed: std::time::Duration,
    pub frames: usize,
    pub limit: usize,
}
/// A trace, written from the first gameplay frame.
pub struct TraceProfiler {
    pub state: ProfilerState,
    pub elapsed: std::time::Duration,
    /// Bytes written so far; `None` before it starts or when the file is unknown.
    pub bytes: Option<u64>,
}
/// Header rows for the profilers this run has; none when it has none.
pub fn profiling_lines(stages: Option<StageProfiler>, trace: Option<TraceProfiler>) -> Vec<String> {
    let stages = stages.map(|s| {
        format!(
            "Profile (--profile): {}  {}  |  {} / {} frames",
            s.state.label(),
            elapsed_text(s.elapsed),
            s.frames,
            s.limit
        )
    });
    let trace = trace.map(|t| {
        let size = match (t.state, t.bytes) {
            (ProfilerState::Idle, _) => "starts with gameplay".to_owned(),
            (_, Some(bytes)) => size_text(bytes),
            (_, None) => "size unknown (TRACE_CHROME not set)".to_owned(),
        };
        format!(
            "Trace (trace-chrome): {}  {}  |  {size}",
            t.state.label(),
            elapsed_text(t.elapsed)
        )
    });
    stages.into_iter().chain(trace).collect()
}
/// `hh:mm:ss.SSS`.
fn elapsed_text(elapsed: std::time::Duration) -> String {
    let millis = elapsed.as_millis();
    let (hours, minutes) = (millis / 3_600_000, millis / 60_000 % 60);
    let (seconds, millis) = (millis / 1000 % 60, millis % 1000);
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}
/// KB below a megabyte, MB below a gigabyte, then GB (1024-based).
fn size_text(bytes: u64) -> String {
    const KB: f64 = 1024.;
    let bytes = bytes as f64;
    if bytes < KB * KB {
        format!("{:.1} KB", bytes / KB)
    } else if bytes < KB * KB * KB {
        format!("{:.1} MB", bytes / (KB * KB))
    } else {
        format!("{:.2} GB", bytes / (KB * KB * KB))
    }
}

/// Where the view is, written as the launch arguments that start a run there, and the frame's
/// physics and material counts.
fn location_line(frame: &DeveloperFrame<'_>) -> String {
    let [x, y, z] = frame.eye;
    let [yaw, pitch] = frame.angles;
    format!(
        "--position {x:.0} {y:.0} {z:.0} --yaw {yaw:.0} --pitch {pitch:.0}  |  {:.2} ms  |  {} materials / {} colliders / {} bodies",
        frame.frame_ms, frame.materials, frame.colliders, frame.bodies
    )
}

#[cfg(test)]
mod tests {
    use super::{
        DeveloperFrame, ProfilerState, StageProfiler, TraceProfiler, developer_lines,
        profiling_lines,
    };
    use std::time::Duration;

    #[test]
    fn profiling_rows_show_state_time_and_size_only_for_running_profilers() {
        assert!(profiling_lines(None, None).is_empty());
        let lines = profiling_lines(
            Some(StageProfiler {
                state: ProfilerState::Finished,
                elapsed: Duration::from_millis(3_723_045),
                frames: 4096,
                limit: 4096,
            }),
            Some(TraceProfiler {
                state: ProfilerState::Profiling,
                elapsed: Duration::from_millis(61_500),
                bytes: Some(3 * 1024 * 1024 * 1024 / 2),
            }),
        );
        assert_eq!(
            lines,
            [
                "Profile (--profile): finished  01:02:03.045  |  4096 / 4096 frames",
                "Trace (trace-chrome): profiling  00:01:01.500  |  1.50 GB",
            ]
        );
        let trace = |state, bytes| {
            profiling_lines(
                None,
                Some(TraceProfiler {
                    state,
                    elapsed: Duration::ZERO,
                    bytes,
                }),
            )
        };
        assert_eq!(
            trace(ProfilerState::Profiling, None),
            [
                "Trace (trace-chrome): profiling  00:00:00.000  |  size unknown (TRACE_CHROME not set)"
            ]
        );
        assert_eq!(
            trace(ProfilerState::Idle, None),
            ["Trace (trace-chrome): idle  00:00:00.000  |  starts with gameplay"]
        );
    }

    #[test]
    fn sizes_switch_units_at_each_1024_step() {
        assert_eq!(super::size_text(512), "0.5 KB");
        assert_eq!(super::size_text(5 * 1024 * 1024 + 512 * 1024), "5.5 MB");
        assert_eq!(super::size_text(1024 * 1024 * 1024), "1.00 GB");
    }

    #[test]
    fn the_last_line_names_the_view_as_launch_arguments() {
        let lines = developer_lines(DeveloperFrame {
            map: "d1_canals_01a",
            triangles: 1,
            textures: 2,
            entities: 3,
            flying: false,
            fps: 80.,
            friendly_fire: false,
            eye: [360.4, 6259.6, -33.2],
            angles: [61.3, -4.6],
            frame_ms: 12.5,
            materials: 4,
            colliders: 5,
            bodies: 6,
        });
        assert_eq!(lines.len(), 6);
        assert!(
            lines[5].starts_with("--position 360 6260 -33 --yaw 61 --pitch -5  |  12.50 ms"),
            "{}",
            lines[5]
        );
    }
}

/// Where F1's overlay visibility is saved, so it stays as the player left it across runs.
const SHOWN_SETTING: &str = "developer_overlay";

/// Whether F1's overlay was left shown.
pub fn shown(config: &hl2_ui::config::Config) -> bool {
    config.extensions.bool(SHOWN_SETTING).unwrap_or(false)
}

pub fn set_shown(config: &mut hl2_ui::config::Config, on: bool) {
    config.extensions.set_bool(SHOWN_SETTING, on);
}

/// F1: saves whether the overlay is shown, keeping an open Options dialog from undoing it.
pub fn save_shown(console: &mut hl2_ui::console::Console, on: bool) {
    set_shown(&mut console.config, on);
    console
        .options
        .sync_extension(|config| &mut config.extensions, SHOWN_SETTING, on);
    let _ = console.config.save();
}

#[cfg(test)]
mod shown_tests {
    use super::{set_shown, shown};

    #[test]
    fn the_overlay_setting_survives_a_save_and_load() {
        let mut config = hl2_ui::config::Config::default();
        assert!(!shown(&config));
        set_shown(&mut config, true);
        let text = serde_json::to_string(&config).expect("save");
        let loaded: hl2_ui::config::Config = serde_json::from_str(&text).expect("load");
        assert!(shown(&loaded));
        let plain = serde_json::to_value(hl2_ui::config::Config::default()).expect("save");
        assert!(
            plain.get("extensions").is_none(),
            "nothing extra in files without it"
        );
    }
}
