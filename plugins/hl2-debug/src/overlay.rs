//! Shared developer overlay painter. Fonts are supplied by the host.
use glam::{Vec2, vec2};
use source_ui::{
    canvas::{Canvas, Color, WHITE},
    font::FontFace,
};
const DEBUG_HEADER_HEIGHT: f32 = 105.;
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
    pub fn draw_debug(&self, lines: &[String]) -> f32 {
        self.canvas.rectangle(
            0.,
            0.,
            self.canvas.width(),
            DEBUG_HEADER_HEIGHT,
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
        DEBUG_HEADER_HEIGHT
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
    use super::{DeveloperFrame, developer_lines};

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
